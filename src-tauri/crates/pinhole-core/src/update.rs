//! "Check for updates" (SPEC §4 rule 6, §8). Runs only when the user presses the
//! button in Settings: Pinhole never checks on its own. Every request goes through
//! the one `HttpClient` (Offline mode, host allow-list); only `api.github.com` and
//! `github.com` release downloads (+ GitHub's release CDN) are contacted.
//!
//! How a copy of Pinhole updates itself depends on how it was installed:
//! * Windows installer (NSIS, `uninstall.exe` next to the exe): download the new
//!   `…-setup.exe`, stop the engines, run it passively (`/P /UPDATE /R` — the same
//!   flags Tauri's own updater passes; `/R` relaunches Pinhole when done) and quit.
//! * Windows portable (`Data/` next to the exe): download the portable zip, unpack
//!   it beside the app, swap the files in (the running exe can be renamed, not
//!   overwritten), relaunch. `Data/` is never touched.
//! * Linux AppImage (`$APPIMAGE`): download the new AppImage next to the old one and
//!   rename it over it (shortcuts keep working), relaunch.
//! * Linux .deb (`/usr/bin/pinhole` owned by the `pinhole` package): download the new
//!   .deb into the Data folder and install it with `pkexec apt-get install`, which asks
//!   for the password; relaunch.
//! * Anything else (dev builds, copies run from elsewhere): the UI opens the release page.
//!
//! Trust: the release's `SHA256SUMS.txt` must carry a valid signature
//! (`SHA256SUMS.txt.sig`, minisign via `tauri signer sign`) from the maintainer's
//! key, whose public half is built into the app (`src-tauri/update-key.pub`). The
//! downloaded file must then match GitHub's size and the signed SHA-256. A release
//! uploaded without that key (by someone who can change releases but can't run the
//! Release workflow from `main` or a `v*` tag) has no valid signature and is never
//! installed. While no key is built in, [`SELF_UPDATE`] is off: every copy
//! is offered the release page and nothing is downloaded or installed in the app.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use pinhole_net::download::{DownloadKind, DownloadSpec};
use serde::{Deserialize, Serialize};

use crate::{AppCore, CoreError, CoreResult};

/// The GitHub repository releases come from.
pub const REPO: &str = "DavidGudovic/PinholeAI";
const RELEASES_API: &str =
    "https://api.github.com/repos/DavidGudovic/PinholeAI/releases?per_page=30";
const SUMS_FILE: &str = "SHA256SUMS.txt";
const MAX_SUMS_BYTES: usize = 64 * 1024;
/// Signature of [`SUMS_FILE`] (base64 of a minisign signature, `tauri signer sign`).
const SIG_FILE: &str = "SHA256SUMS.txt.sig";
const MAX_SIG_BYTES: usize = 4 * 1024;
/// Staging folder name (next to the app for portable / AppImage, in the OS temp
/// dir for the Windows installer). Removed on the next start.
pub const STAGING_DIR: &str = ".pinhole-update";
/// Staging folder for the Windows installer, inside the OS temp dir.
const INSTALLER_STAGING_DIR: &str = "pinhole-update";
const PRODUCT: &str = "Pinhole";
/// Where the .deb installs Pinhole, and dpkg's file list for the `pinhole` package.
const DEB_EXE: &str = "/usr/bin/pinhole";
const DEB_FILE_LIST: &str = "/var/lib/dpkg/info/pinhole.list";
/// Runs the .deb install as root after asking for the password.
const PKEXEC: &str = "/usr/bin/pkexec";

/// The maintainer's update-signing public key: the `.pub` file `tauri signer generate`
/// writes (base64, one line). Empty = no key, so no in-app install.
const UPDATE_PUBLIC_KEY: &str = include_str!("../../../update-key.pub");

/// In-app install ("Update and restart"). On once a public key is built in
/// (RELEASE-SPEC §12 "Signed updates"): the SHA-256 list comes from the same
/// release, so only its signature can catch a release someone else uploaded. While
/// off, "Check for updates" offers the release page for every copy and
/// [`install_update`] refuses.
pub const SELF_UPDATE: bool = !UPDATE_PUBLIC_KEY.trim_ascii().is_empty();

/// One update per app run: set when an install starts, cleared if it fails (then
/// nothing was replaced). The UI can be closed and reopened meanwhile.
static UPDATING: AtomicBool = AtomicBool::new(false);

/// How this copy of Pinhole can update itself. `UpdateInstallMode` in `src/lib/types.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallMode {
    Installer,
    Portable,
    AppImage,
    Deb,
    /// Can't replace itself (dev builds, copies run from elsewhere): open the release page.
    Manual,
}

/// Result of "Check for updates". `UpdateCheck` in `src/lib/types.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub current_version: String,
    /// `None` = already on the newest release.
    pub update: Option<UpdateInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub published_at: Option<String>,
    pub install_mode: InstallMode,
    /// Download size for `install_mode` (None for `manual`).
    pub size_bytes: Option<u64>,
}

/// What the Tauri shell does once the update is downloaded and in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prepared {
    /// Run this NSIS installer with [`INSTALLER_ARGS`], then quit.
    RunInstaller(PathBuf),
    /// Files are already replaced: start this executable, then quit.
    Relaunch(PathBuf),
}

/// Passive install, update mode, restart Pinhole afterwards (Tauri NSIS template).
pub const INSTALLER_ARGS: &[&str] = &["/P", "/UPDATE", "/R"];

/// Where and how this copy is installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Installer {
        staging: PathBuf,
    },
    Portable {
        app_dir: PathBuf,
    },
    AppImage {
        file: PathBuf,
    },
    /// Installed from the .deb; the download is kept in `staging` (inside the Data folder).
    Deb {
        staging: PathBuf,
    },
    Manual,
}

impl Target {
    pub fn mode(&self) -> InstallMode {
        match self {
            Target::Installer { .. } => InstallMode::Installer,
            Target::Portable { .. } => InstallMode::Portable,
            Target::AppImage { .. } => InstallMode::AppImage,
            Target::Deb { .. } => InstallMode::Deb,
            Target::Manual => InstallMode::Manual,
        }
    }

    fn asset_name(&self, version: &str) -> Option<String> {
        match self {
            Target::Installer { .. } => Some(format!("{PRODUCT}-{version}-windows-x64-setup.exe")),
            Target::Portable { .. } => {
                Some(format!("{PRODUCT}-{version}-windows-x64-portable.zip"))
            }
            Target::AppImage { .. } => Some(format!("{PRODUCT}-{version}-linux-x86_64.AppImage")),
            Target::Deb { .. } => Some(format!("{PRODUCT}-{version}-linux-amd64.deb")),
            Target::Manual => None,
        }
    }

    fn staging(&self) -> Option<PathBuf> {
        match self {
            Target::Installer { staging } => Some(staging.clone()),
            Target::Portable { app_dir } => Some(app_dir.join(STAGING_DIR)),
            Target::AppImage { file } => file.parent().map(|p| p.join(STAGING_DIR)),
            Target::Deb { staging } => Some(staging.clone()),
            Target::Manual => None,
        }
    }
}

/// Inputs of [`detect_target_with`] (pure, so every case is testable).
#[derive(Debug, Clone)]
pub struct Environment {
    pub os: &'static str,
    pub debug_build: bool,
    pub exe_dir: PathBuf,
    /// `$APPIMAGE` (set by the AppImage runtime to the .AppImage path).
    pub appimage: Option<PathBuf>,
    /// The Data folder is the portable one next to the exe.
    pub portable_data: bool,
    pub temp_dir: PathBuf,
    /// The running exe is the .deb's `/usr/bin/pinhole` and `pkexec` is there.
    pub deb_installed: bool,
    /// The Data folder (the .deb's download goes there: a folder only this user can change).
    pub data_dir: PathBuf,
}

impl Environment {
    pub fn current(core: &AppCore) -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            os: std::env::consts::OS,
            debug_build: cfg!(debug_assertions),
            portable_data: core.data.portable && core.data.root.parent() == Some(exe_dir.as_path()),
            exe_dir,
            appimage: std::env::var_os("APPIMAGE").map(PathBuf::from),
            temp_dir: std::env::temp_dir(),
            deb_installed: std::env::current_exe().is_ok_and(|p| p == Path::new(DEB_EXE))
                && Path::new(DEB_FILE_LIST).is_file()
                && Path::new(PKEXEC).is_file(),
            data_dir: core.data.root.clone(),
        }
    }
}

/// How this copy updates: [`detect_target_with`], or by hand while [`SELF_UPDATE`] is off.
pub fn update_target(env: &Environment) -> Target {
    update_target_with(env, SELF_UPDATE)
}

fn update_target_with(env: &Environment, self_update: bool) -> Target {
    if self_update {
        detect_target_with(env)
    } else {
        Target::Manual
    }
}

pub fn detect_target_with(env: &Environment) -> Target {
    if env.debug_build {
        return Target::Manual;
    }
    match env.os {
        "windows" => {
            if env.exe_dir.join("uninstall.exe").is_file() {
                Target::Installer {
                    staging: env.temp_dir.join(INSTALLER_STAGING_DIR),
                }
            } else if env.portable_data && env.exe_dir.join(format!("{PRODUCT}.exe")).is_file() {
                Target::Portable {
                    app_dir: env.exe_dir.clone(),
                }
            } else {
                Target::Manual
            }
        }
        // The .deb first: a copy started from inside another AppImage inherits its $APPIMAGE.
        "linux" if env.deb_installed => Target::Deb {
            staging: env.data_dir.join(STAGING_DIR),
        },
        "linux" => match &env.appimage {
            Some(file) if file.is_file() && file.parent().is_some_and(writable_dir) => {
                Target::AppImage { file: file.clone() }
            }
            _ => Target::Manual,
        },
        _ => Target::Manual,
    }
}

fn writable_dir(dir: &Path) -> bool {
    let probe = dir.join(format!(".pinhole-write-test-{}", std::process::id()));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

// ------------------------------------------------------------------ GitHub

#[derive(Debug, Clone, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GhAsset {
    name: String,
    size: u64,
}

/// `v1.2.3` → 1.2.3. Anything else (odd tags) is ignored.
fn tag_version(tag: &str) -> Option<semver::Version> {
    semver::Version::parse(tag.strip_prefix('v')?).ok()
}

/// Newest published (non-draft) release newer than `current`. Pre-release versions
/// (`1.1.0-rc.1`) are offered only to a copy that is itself a pre-release.
fn newest_release<'a>(
    releases: &'a [GhRelease],
    current: &semver::Version,
) -> Option<(&'a GhRelease, semver::Version)> {
    newest_matching(releases, current, |_, _| true)
}

fn newest_matching<'a>(
    releases: &'a [GhRelease],
    current: &semver::Version,
    ok: impl Fn(&GhRelease, &semver::Version) -> bool,
) -> Option<(&'a GhRelease, semver::Version)> {
    releases
        .iter()
        .filter(|r| !r.draft)
        .filter_map(|r| tag_version(&r.tag_name).map(|v| (r, v)))
        .filter(|(_, v)| v.pre.is_empty() || !current.pre.is_empty())
        .filter(|(r, v)| v > current && ok(r, v))
        .max_by(|a, b| a.1.cmp(&b.1))
}

/// The update to offer: the newest release that has this copy's file (and a
/// signed checksum list), so a release missing one platform's build doesn't hide an
/// installable one; otherwise the newest release, installed by hand.
fn pick_update(
    releases: &[GhRelease],
    current: &semver::Version,
    target: &Target,
) -> Option<UpdateInfo> {
    let installable = newest_matching(releases, current, |r, v| {
        let has = |name: &str| r.assets.iter().any(|a| a.name == name);
        target.asset_name(&v.to_string()).is_some_and(|n| has(&n))
            && has(SUMS_FILE)
            && has(SIG_FILE)
    });
    if let Some((r, v)) = installable {
        let name = target.asset_name(&v.to_string()).unwrap_or_default();
        return Some(UpdateInfo {
            version: v.to_string(),
            published_at: r.published_at.clone(),
            install_mode: target.mode(),
            size_bytes: r.assets.iter().find(|a| a.name == name).map(|a| a.size),
        });
    }
    newest_release(releases, current).map(|(r, v)| UpdateInfo {
        version: v.to_string(),
        published_at: r.published_at.clone(),
        install_mode: InstallMode::Manual,
        size_bytes: None,
    })
}

/// Download URLs are built from our repo + a tag that parsed as semver + an asset
/// name we chose, never taken from the API response.
fn asset_url(version: &semver::Version, name: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/v{version}/{name}")
}

/// One-time cleanup: earlier builds let the user store a GitHub token (updates
/// while the repository was private). Updates are unauthenticated now, so delete
/// any token left in the keychain, once (a marker file in `Data/` records it, so
/// later starts don't touch the keychain). Keychain problems are ignored and the
/// cleanup is tried again next start.
pub async fn remove_legacy_github_token(data_root: std::path::PathBuf) {
    let marker = data_root.join(".github-token-cleared");
    if marker.exists() {
        return;
    }
    if crate::catalog::blocking(pinhole_store::keychain::delete_github_token)
        .await
        .is_ok_and(|r| r.is_ok())
    {
        let _ = std::fs::write(marker, b"");
    }
}

fn api_headers(accept: &str) -> Vec<(String, String)> {
    vec![
        ("accept".to_string(), accept.to_string()),
        ("x-github-api-version".to_string(), "2022-11-28".to_string()),
    ]
}

fn as_refs(h: &[(String, String)]) -> Vec<(&str, &str)> {
    h.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

/// Release page for "What's new" / manual installs. `None` → the releases list.
pub fn release_page_url(version: Option<&str>) -> CoreResult<String> {
    match version {
        None => Ok(format!("https://github.com/{REPO}/releases")),
        Some(v) => {
            let v = semver::Version::parse(v)
                .map_err(|_| CoreError::invalid("That isn't a Pinhole version."))?;
            Ok(format!("https://github.com/{REPO}/releases/tag/v{v}"))
        }
    }
}

/// `SHA256SUMS.txt` (`sha256sum` format, optional `*` binary marker) → hash of `name`.
fn sum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim_start().trim_start_matches('*');
        (file == name && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

/// Check `SHA256SUMS.txt` against its `.sig` (`tauri signer sign` output: base64 of a
/// minisign signature) with `public_key` (base64 of a minisign public key file).
pub fn verify_sums(sums: &[u8], sig: &[u8], public_key: &str) -> CoreResult<()> {
    use base64::Engine as _;
    let b64 = |s: &[u8]| -> Option<String> {
        let raw: Vec<u8> = s
            .iter()
            .copied()
            .filter(|c| !c.is_ascii_whitespace())
            .collect();
        let text = base64::engine::general_purpose::STANDARD.decode(raw).ok()?;
        String::from_utf8(text).ok()
    };
    // A signature that doesn't verify means the release wasn't made with the
    // maintainer's key: don't send people to install the same file by hand.
    let bad = |details: String| {
        CoreError::new("hash_mismatch", "This release isn't signed by Pinhole's maintainer, so Pinhole won't install it. Don't install this version by hand; check the release page again later.")
            .with_details(details)
    };
    let key = b64(public_key.as_bytes())
        .and_then(|t| minisign_verify::PublicKey::decode(&t).ok())
        .ok_or_else(|| {
            CoreError::internal("This copy of Pinhole can't check updates. Download the new version from the release page.")
                .with_details("the built-in update key can't be read")
        })?;
    let signature = b64(sig)
        .and_then(|t| minisign_verify::Signature::decode(&t).ok())
        .ok_or_else(|| bad("the signature file can't be read".into()))?;
    key.verify(sums, &signature, false)
        .map_err(|e| bad(e.to_string()))
}

/// The running app's version. The Tauri shell passes its own (tauri.conf.json,
/// which the release workflow matches against the tag); tests use the crate's.
pub fn parse_current(version: &str) -> CoreResult<semver::Version> {
    semver::Version::parse(version).map_err(|_| {
        CoreError::internal("Pinhole couldn't read its own version.")
            .with_details(version.to_string())
    })
}

/// `CoreError.code` when GitHub doesn't show Pinhole's releases.
pub const UNAVAILABLE: &str = "updates_unavailable";
const UNAVAILABLE_MESSAGE: &str =
    "Pinhole can't see its releases on GitHub right now. You can download new versions from the release page.";

/// The release list (the public GitHub API, no sign-in).
async fn fetch_releases(core: &AppCore) -> CoreResult<Vec<GhRelease>> {
    let headers = api_headers("application/vnd.github+json");
    core.http
        .get_json::<Vec<GhRelease>>(RELEASES_API, &as_refs(&headers))
        .await
        .map_err(releases_error)
}

/// Plain-language error for a failed asset download (checksum list).
fn asset_error(e: pinhole_net::NetError) -> CoreError {
    match e {
        pinhole_net::NetError::Offline => CoreError::new(
            "offline",
            "Offline mode is on. Turn it off in Settings to update.",
        ),
        other => CoreError::from(other),
    }
}

/// Plain-language error for a failed releases request.
fn releases_error(e: pinhole_net::NetError) -> CoreError {
    match e {
        pinhole_net::NetError::Offline => CoreError::new(
            "offline",
            "Offline mode is on. Turn it off in Settings to check for updates.",
        ),
        // GitHub answers 404 when the releases can't be seen (repository not public
        // yet, or moved).
        pinhole_net::NetError::Status(404) => {
            CoreError::new(UNAVAILABLE, UNAVAILABLE_MESSAGE).with_details(e.to_string())
        }
        pinhole_net::NetError::Unauthorized(_) | pinhole_net::NetError::Status(429) => {
            CoreError::new(
                "network",
                "GitHub is limiting update checks right now. Try again in an hour.",
            )
            .with_details(e.to_string())
        }
        other => CoreError::from(other),
    }
}

/// "Check for updates": one request to the GitHub releases API.
pub async fn check_for_updates(core: &AppCore, current_version: &str) -> CoreResult<UpdateCheck> {
    let current = parse_current(current_version)?;
    let releases = fetch_releases(core).await?;
    let target = update_target(&Environment::current(core));
    Ok(UpdateCheck {
        current_version: current.to_string(),
        update: pick_update(&releases, &current, &target),
    })
}

/// Refuse while a picture is being made or other downloads are running: the
/// update ends by restarting Pinhole, which would lose them.
pub fn ensure_idle(core: &AppCore, own_group: Option<&str>) -> CoreResult<()> {
    drop(crate::models::folder_read(core)?);
    if core.gen.run_lock.try_lock().is_err() {
        return Err(CoreError::invalid(
            "Pinhole is making a picture. Wait for it to finish (or cancel it), then update.",
        ));
    }
    let busy = core
        .downloads
        .status()
        .iter()
        .any(|g| !g.state.is_finished() && Some(g.group_id.as_str()) != own_group);
    if busy {
        return Err(CoreError::invalid(
            "Downloads are still running. Wait for them to finish (or cancel them), then update.",
        ));
    }
    Ok(())
}

/// Download `version`, verify it and put it in place. The caller (Tauri shell)
/// then stops the engines and runs / relaunches what [`Prepared`] says. Only one
/// update runs per app run.
pub async fn install_update(
    core: &Arc<AppCore>,
    current_version: &str,
    version: &str,
) -> CoreResult<Prepared> {
    if !SELF_UPDATE {
        return Err(CoreError::invalid(
            "Download the new version from the release page.",
        ));
    }
    let current = parse_current(current_version)?;
    if UPDATING.swap(true, Ordering::SeqCst) {
        return Err(CoreError::invalid("An update is already under way."));
    }
    let result = install_inner(core, &current, version).await;
    if result.is_err() {
        UPDATING.store(false, Ordering::SeqCst);
    }
    result
}

async fn install_inner(
    core: &Arc<AppCore>,
    current: &semver::Version,
    version: &str,
) -> CoreResult<Prepared> {
    let wanted = semver::Version::parse(version)
        .map_err(|_| CoreError::invalid("That isn't a Pinhole version."))?;
    if &wanted <= current {
        return Err(CoreError::invalid("This version is already installed."));
    }
    ensure_idle(core, None)?;
    let target = detect_target_with(&Environment::current(core));
    let (Some(name), Some(staging)) = (target.asset_name(&wanted.to_string()), target.staging())
    else {
        return Err(CoreError::invalid("This copy of Pinhole can't update itself. Open the download page and install the new version from there."));
    };

    let releases = fetch_releases(core).await?;
    let release = releases
        .iter()
        .find(|r| !r.draft && tag_version(&r.tag_name).as_ref() == Some(&wanted))
        .ok_or_else(|| {
            CoreError::not_found("That update isn't available any more. Check for updates again.")
        })?;
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| {
            CoreError::not_found(
                "This release has no download for your system. Open the download page instead.",
            )
        })?;
    if ![SUMS_FILE, SIG_FILE]
        .iter()
        .all(|f| release.assets.iter().any(|a| a.name == *f))
    {
        return Err(CoreError::new(
            "hash_mismatch",
            "This release can't be checked (it isn't signed), so Pinhole won't install it.",
        ));
    }
    let file_url = asset_url(&wanted, &name);
    let sums = core
        .http
        .get_bytes(&asset_url(&wanted, SUMS_FILE), &[], MAX_SUMS_BYTES)
        .await
        .map_err(asset_error)?;
    let sig = core
        .http
        .get_bytes(&asset_url(&wanted, SIG_FILE), &[], MAX_SIG_BYTES)
        .await
        .map_err(asset_error)?;
    // The signed list names files by version, so an older signed list can't vouch
    // for this release's file.
    verify_sums(&sums, &sig, UPDATE_PUBLIC_KEY)?;
    let sha256 = sum_for(&String::from_utf8_lossy(&sums), &name)
        .ok_or_else(|| CoreError::new("hash_mismatch", "This release's checksum list doesn't include your download, so Pinhole won't install it."))?;

    std::fs::create_dir_all(&staging)?;
    let spec = DownloadSpec {
        url: file_url,
        headers: Vec::new(),
        dest: staging.join(&name),
        sha256: Some(sha256.clone()),
        size_bytes: Some(asset.size),
        label: format!("{PRODUCT} {wanted}"),
        ..Default::default()
    };
    let group = core.downloads.enqueue_kind(
        format!("{PRODUCT} {wanted}"),
        DownloadKind::AppUpdate,
        vec![spec],
    );
    let files = crate::downloads::wait(core, &group).await?;
    ensure_idle(core, Some(&group))?;
    let downloaded = files
        .into_iter()
        .next()
        .ok_or_else(|| {
            CoreError::internal("The update download finished without a file. Try again.")
        })?
        .path;

    match target {
        Target::Installer { .. } => Ok(Prepared::RunInstaller(downloaded)),
        Target::Portable { app_dir } => {
            let exe = app_dir.join(format!("{PRODUCT}.exe"));
            tokio::task::spawn_blocking(move || apply_portable(&downloaded, &app_dir))
                .await
                .map_err(|e| {
                    CoreError::internal("The update stopped unexpectedly. Try again.")
                        .with_details(e.to_string())
                })??;
            Ok(Prepared::Relaunch(exe))
        }
        Target::AppImage { file } => {
            let dest = file.clone();
            tokio::task::spawn_blocking(move || apply_appimage(&downloaded, &dest))
                .await
                .map_err(|e| {
                    CoreError::internal("The update stopped unexpectedly. Try again.")
                        .with_details(e.to_string())
                })??;
            Ok(Prepared::Relaunch(file))
        }
        Target::Deb { staging } => {
            let res = tokio::task::spawn_blocking(move || install_deb(&downloaded, &sha256))
                .await
                .map_err(|e| {
                    CoreError::internal("The update stopped unexpectedly. Try again.")
                        .with_details(e.to_string())
                })?;
            let _ = std::fs::remove_dir_all(&staging);
            res?;
            // The password dialog can stay open a while: work started meanwhile isn't cut off.
            if ensure_idle(core, Some(&group)).is_err() {
                return Err(CoreError::new(
                    "update_restart",
                    "The update is installed. Close Pinhole and open it again when your picture or download is done.",
                ));
            }
            Ok(Prepared::Relaunch(PathBuf::from(DEB_EXE)))
        }
        Target::Manual => unreachable!("manual targets return early"),
    }
}

/// `pkexec apt-get install -y <file>`: the system asks for the password, apt installs the
/// package (and anything new it depends on).
pub fn deb_install_command(file: &Path) -> std::process::Command {
    let mut c = std::process::Command::new(PKEXEC);
    c.args(["/usr/bin/apt-get", "install", "-y", "-q"])
        .arg(file);
    c
}

fn install_deb(file: &Path, sha256: &str) -> CoreResult<()> {
    use sha2::Digest;
    // Checked again right before it is handed to apt.
    let bytes = std::fs::read(file).map_err(update_failed)?;
    if !hex::encode(sha2::Sha256::digest(&bytes)).eq_ignore_ascii_case(sha256) {
        return Err(CoreError::new(
            "hash_mismatch",
            "The downloaded package changed after it was checked, so Pinhole won't install it.",
        ));
    }
    let out = deb_install_command(file)
        .output()
        .map_err(|e| deb_failed(None, &e.to_string()))?;
    if out.status.success() {
        return Ok(());
    }
    let tail: String = String::from_utf8_lossy(&out.stderr)
        .lines()
        .rev()
        .take(10)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    Err(deb_failed(out.status.code(), &tail))
}

/// pkexec exits with 126 when the password dialog is closed and 127 when the password is
/// refused or no dialog can be shown; apt exits with 100, saying "lock" when another program
/// is installing software.
fn deb_failed(code: Option<i32>, details: &str) -> CoreError {
    match code {
        Some(126) => CoreError::new(
            "cancelled_auth",
            "The update wasn't installed because the password dialog was closed. Nothing was changed.",
        ),
        Some(127) => CoreError::new(
            "cancelled_auth",
            "The update wasn't installed: the password wasn't accepted, or this system couldn't ask for it. Open the download page to install it by hand.",
        )
        .with_details(details.to_string()),
        Some(100) if details.contains("lock") => CoreError::new(
            "busy",
            "Another program is installing software right now. Try again in a few minutes.",
        )
        .with_details(details.to_string()),
        _ => CoreError::new(
            "io",
            "The new package couldn't be installed. Open the download page and install it from there.",
        )
        .with_details(details.to_string()),
    }
}

// ------------------------------------------------------------------ apply

fn update_failed(e: impl std::fmt::Display) -> CoreError {
    CoreError::new("io", "Pinhole couldn't replace its files, so nothing was changed. Close other copies of Pinhole and try again.")
        .with_details(e.to_string())
}

/// Linux: make the new AppImage executable and rename it over the running one
/// (same folder, so the rename is atomic; the running process keeps its inode).
pub fn apply_appimage(new_file: &Path, target: &Path) -> CoreResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new_file, std::fs::Permissions::from_mode(0o755))
            .map_err(update_failed)?;
    }
    std::fs::rename(new_file, target).map_err(update_failed)
}

/// Windows portable: unpack `Pinhole/…` from the zip (never `Data/`) into
/// `<app_dir>/.pinhole-update/new`, then swap each top-level entry: the current one
/// moves to `.pinhole-update/old` (Windows allows renaming a running exe), the new
/// one moves in. Any failure puts everything back.
pub fn apply_portable(zip_path: &Path, app_dir: &Path) -> CoreResult<()> {
    let stage = app_dir.join(STAGING_DIR);
    let new_dir = stage.join("new");
    let old_dir = stage.join("old");
    for d in [&new_dir, &old_dir] {
        if d.exists() {
            std::fs::remove_dir_all(d).map_err(update_failed)?;
        }
        std::fs::create_dir_all(d).map_err(update_failed)?;
    }
    unpack_portable(zip_path, &new_dir)?;
    if !new_dir.join(format!("{PRODUCT}.exe")).is_file() {
        return Err(CoreError::new(
            "invalid",
            "The downloaded update doesn't contain Pinhole. Nothing was changed.",
        ));
    }

    let mut entries: Vec<std::ffi::OsString> = std::fs::read_dir(&new_dir)
        .map_err(update_failed)?
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();
    entries.sort();
    let mut done: Vec<(std::ffi::OsString, bool)> = Vec::new();
    for name in entries {
        let live = app_dir.join(&name);
        let had_old = live.exists();
        let step = (|| {
            if had_old {
                std::fs::rename(&live, old_dir.join(&name))?;
            }
            if let Err(e) = std::fs::rename(new_dir.join(&name), &live) {
                if had_old {
                    let _ = std::fs::rename(old_dir.join(&name), &live);
                }
                return Err(e);
            }
            Ok::<(), std::io::Error>(())
        })();
        if let Err(e) = step {
            rollback(app_dir, &old_dir, &done);
            return Err(update_failed(e));
        }
        done.push((name, had_old));
    }
    Ok(())
}

fn rollback(app_dir: &Path, old_dir: &Path, done: &[(std::ffi::OsString, bool)]) {
    for (name, had_old) in done.iter().rev() {
        let live = app_dir.join(name);
        let _ = if live.is_dir() {
            std::fs::remove_dir_all(&live)
        } else {
            std::fs::remove_file(&live)
        };
        if *had_old {
            let _ = std::fs::rename(old_dir.join(name), &live);
        }
    }
}

/// Unpack with the engine installer's hardened zip reader (safe paths, size caps,
/// no symlinks) into `out`, then keep only `Pinhole/…` minus `Pinhole/Data/`.
fn unpack_portable(zip_path: &Path, out: &Path) -> CoreResult<()> {
    let raw = out.with_extension("zip-contents");
    if raw.exists() {
        std::fs::remove_dir_all(&raw).map_err(update_failed)?;
    }
    std::fs::create_dir_all(&raw).map_err(update_failed)?;
    pinhole_engine::install::extract_zip(zip_path, &raw).map_err(|e| {
        CoreError::new(
            "invalid",
            "The downloaded update couldn't be unpacked. Try again.",
        )
        .with_details(e.to_string())
    })?;
    let root = raw.join(PRODUCT);
    if root.is_dir() {
        for entry in std::fs::read_dir(&root).map_err(update_failed)? {
            let entry = entry.map_err(update_failed)?;
            if entry.file_name().eq_ignore_ascii_case("Data") {
                continue;
            }
            std::fs::rename(entry.path(), out.join(entry.file_name())).map_err(update_failed)?;
        }
    }
    let _ = std::fs::remove_dir_all(&raw);
    Ok(())
}

/// Remove leftovers of a finished (or abandoned) update. Best effort: the old exe
/// of a portable update may still be exiting; the next start tries again.
pub fn cleanup_after_update(exe_dir: &Path, data_dir: &Path) {
    let mut dirs = vec![exe_dir.join(STAGING_DIR), data_dir.join(STAGING_DIR)];
    if cfg!(windows) {
        dirs.push(std::env::temp_dir().join(INSTALLER_STAGING_DIR));
    }
    if let Some(parent) = std::env::var_os("APPIMAGE")
        .as_ref()
        .and_then(|p| Path::new(p).parent().map(Path::to_path_buf))
    {
        dirs.push(parent.join(STAGING_DIR));
    }
    for d in dirs {
        if d.is_dir() {
            let _ = std::fs::remove_dir_all(&d);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn rel(tag: &str, draft: bool, assets: &[(&str, u64)]) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
            draft,
            published_at: None,
            assets: assets
                .iter()
                .map(|(n, s)| GhAsset {
                    name: (*n).into(),
                    size: *s,
                })
                .collect(),
        }
    }

    #[test]
    fn picks_newest_published_release_above_current() {
        let current = semver::Version::parse("0.1.0").unwrap();
        let list = vec![
            rel("v0.3.0", true, &[]),
            rel("v0.2.0", false, &[]),
            rel("nightly", false, &[]),
            rel("v0.1.1", false, &[]),
            rel("v0.1.0", false, &[]),
            rel("v0.10.0", false, &[]),
        ];
        let (r, v) = newest_release(&list, &current).unwrap();
        assert_eq!(
            (r.tag_name.as_str(), v.to_string().as_str()),
            ("v0.10.0", "0.10.0")
        );
        assert!(
            newest_release(&list[4..5], &current).is_none(),
            "same version is not an update"
        );
        assert!(
            newest_release(&list[..1], &current).is_none(),
            "drafts are ignored"
        );
    }

    #[test]
    fn offers_the_newest_release_this_copy_can_install() {
        let current = semver::Version::parse("0.2.0").unwrap();
        let setup = |v: &str| format!("Pinhole-{v}-windows-x64-setup.exe");
        let (s3, s4) = (setup("0.3.0"), setup("0.4.0"));
        let list = vec![
            rel(
                "v0.4.0",
                false,
                &[
                    ("Pinhole-0.4.0-linux-x86_64.AppImage", 1),
                    ("SHA256SUMS.txt", 1),
                ],
            ),
            rel(
                "v0.3.0",
                false,
                &[(s3.as_str(), 7), (SUMS_FILE, 1), (SIG_FILE, 1)],
            ),
        ];
        let win = Target::Installer {
            staging: PathBuf::from("t"),
        };
        let u = pick_update(&list, &current, &win).unwrap();
        assert_eq!(
            (u.version.as_str(), u.install_mode, u.size_bytes),
            ("0.3.0", InstallMode::Installer, Some(7))
        );
        // No checksum list, or one without a signature → not installable, offered by hand.
        for assets in [
            vec![(s4.as_str(), 9)],
            vec![(s4.as_str(), 9), (SUMS_FILE, 1)],
        ] {
            let bare = vec![rel("v0.4.0", false, &assets)];
            let u = pick_update(&bare, &current, &win).unwrap();
            assert_eq!(
                (u.version.as_str(), u.install_mode, u.size_bytes),
                ("0.4.0", InstallMode::Manual, None)
            );
        }
        assert_eq!(
            pick_update(&list, &current, &Target::Manual)
                .unwrap()
                .version,
            "0.4.0"
        );
        assert!(pick_update(&list, &semver::Version::parse("0.4.0").unwrap(), &win).is_none());
    }

    #[test]
    fn unseen_releases_get_a_plain_message() {
        let e = releases_error(pinhole_net::NetError::Status(404));
        assert_eq!(e.code, UNAVAILABLE);
        assert!(e.message.contains("release page"), "{}", e.message);
        assert!(!e.message.to_lowercase().contains("token"), "{}", e.message);
        assert_eq!(e.details.as_deref(), Some("HTTP 404"));
        // A missing checksum file is not "releases can't be seen".
        assert_eq!(
            asset_error(pinhole_net::NetError::Status(404)).code,
            "network"
        );
        assert_eq!(
            releases_error(pinhole_net::NetError::Unauthorized(403)).code,
            "network"
        );
        assert_eq!(
            releases_error(pinhole_net::NetError::Offline).code,
            "offline"
        );
        assert_eq!(
            releases_error(pinhole_net::NetError::Status(500)).code,
            "network"
        );
        // Requests carry no credentials.
        assert!(api_headers("a").iter().all(|(k, _)| k != "authorization"));
    }

    #[test]
    fn checksum_lookup() {
        let h = "a".repeat(64);
        let sums = format!(
            "{h}  Pinhole-0.2.0-windows-x64-setup.exe\n{}  *Pinhole-0.2.0-linux-x86_64.AppImage\n",
            "B".repeat(64)
        );
        assert_eq!(
            sum_for(&sums, "Pinhole-0.2.0-windows-x64-setup.exe"),
            Some(h)
        );
        assert_eq!(
            sum_for(&sums, "Pinhole-0.2.0-linux-x86_64.AppImage"),
            Some("b".repeat(64))
        );
        assert_eq!(
            sum_for(&sums, "Pinhole-0.2.0-windows-x64-portable.zip"),
            None
        );
        assert_eq!(sum_for("xyz  Pinhole.exe", "Pinhole.exe"), None);
    }

    #[test]
    fn urls_are_built_not_taken_from_the_api() {
        let v = semver::Version::parse("0.2.0").unwrap();
        assert_eq!(
            asset_url(&v, "SHA256SUMS.txt"),
            "https://github.com/DavidGudovic/PinholeAI/releases/download/v0.2.0/SHA256SUMS.txt"
        );
        assert_eq!(
            release_page_url(Some("0.2.0")).unwrap(),
            "https://github.com/DavidGudovic/PinholeAI/releases/tag/v0.2.0"
        );
        assert!(release_page_url(Some("../../evil")).is_err());
    }

    fn env(os: &'static str, dir: &Path) -> Environment {
        Environment {
            os,
            debug_build: false,
            exe_dir: dir.to_path_buf(),
            appimage: None,
            portable_data: false,
            temp_dir: dir.join("tmp"),
            deb_installed: false,
            data_dir: dir.join("Data"),
        }
    }

    #[test]
    fn detects_how_pinhole_was_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // Nothing recognisable → manual.
        assert_eq!(detect_target_with(&env("windows", dir)), Target::Manual);
        // Portable: Data/ next to Pinhole.exe.
        std::fs::write(dir.join("Pinhole.exe"), b"").unwrap();
        let mut e = env("windows", dir);
        e.portable_data = true;
        assert_eq!(
            detect_target_with(&e),
            Target::Portable {
                app_dir: dir.to_path_buf()
            }
        );
        // Installer wins when uninstall.exe is there.
        std::fs::write(dir.join("uninstall.exe"), b"").unwrap();
        assert_eq!(
            detect_target_with(&e),
            Target::Installer {
                staging: dir.join("tmp").join("pinhole-update")
            }
        );
        // Dev builds never replace themselves.
        e.debug_build = true;
        assert_eq!(detect_target_with(&e), Target::Manual);
        // Linux: an AppImage or the .deb update themselves.
        assert_eq!(detect_target_with(&env("linux", dir)), Target::Manual);
        let mut deb = env("linux", dir);
        deb.deb_installed = true;
        assert_eq!(
            detect_target_with(&deb),
            Target::Deb {
                staging: dir.join("Data").join(STAGING_DIR)
            }
        );
        // An $APPIMAGE inherited from another app doesn't make the .deb an AppImage.
        let other = dir.join("Terminal.AppImage");
        std::fs::write(&other, b"").unwrap();
        deb.appimage = Some(other);
        assert!(matches!(detect_target_with(&deb), Target::Deb { .. }));
        assert_eq!(
            Target::Deb {
                staging: dir.into()
            }
            .asset_name("1.0.3")
            .as_deref(),
            Some("Pinhole-1.0.3-linux-amd64.deb")
        );
        let img = dir.join("Pinhole.AppImage");
        std::fs::write(&img, b"").unwrap();
        let mut l = env("linux", dir);
        l.appimage = Some(img.clone());
        assert_eq!(detect_target_with(&l), Target::AppImage { file: img });
        assert_eq!(detect_target_with(&env("macos", dir)), Target::Manual);
    }

    #[test]
    fn the_deb_is_installed_with_pkexec_and_apt() {
        let c = deb_install_command(Path::new(
            "/home/u/Data/.pinhole-update/Pinhole-1.0.4-linux-amd64.deb",
        ));
        assert_eq!(c.get_program(), "/usr/bin/pkexec");
        let args: Vec<_> = c
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "/usr/bin/apt-get",
                "install",
                "-y",
                "-q",
                "/home/u/Data/.pinhole-update/Pinhole-1.0.4-linux-amd64.deb"
            ]
        );
        assert_eq!(deb_failed(Some(126), "").code, "cancelled_auth");
        assert_eq!(deb_failed(Some(127), "").code, "cancelled_auth");
        assert_eq!(
            deb_failed(
                Some(100),
                "E: Could not get lock /var/lib/dpkg/lock-frontend"
            )
            .code,
            "busy"
        );
        let e = deb_failed(Some(100), "E: broken");
        assert_eq!(e.code, "io");
        assert!(e.message.contains("download page"));
    }

    #[test]
    fn without_a_built_in_key_every_copy_updates_from_the_release_page() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("uninstall.exe"), b"").unwrap();
        let e = env("windows", dir);
        assert_ne!(detect_target_with(&e), Target::Manual);
        assert_eq!(update_target_with(&e, false), Target::Manual);
        assert_eq!(update_target_with(&e, true), detect_target_with(&e));
        assert_eq!(update_target(&e), update_target_with(&e, SELF_UPDATE));
        let releases = vec![rel(
            "v9.0.0",
            false,
            &[
                ("Pinhole-9.0.0-windows-x64-setup.exe", 10),
                (SUMS_FILE, 1),
                (SIG_FILE, 1),
            ],
        )];
        let current = semver::Version::new(1, 0, 0);
        let info = pick_update(&releases, &current, &update_target_with(&e, false)).unwrap();
        assert_eq!(info.install_mode, InstallMode::Manual);
        assert_eq!(info.size_bytes, None);
        let info = pick_update(&releases, &current, &update_target_with(&e, true)).unwrap();
        assert_eq!(info.install_mode, InstallMode::Installer);
        assert_eq!(info.size_bytes, Some(10));
    }

    // A throwaway key (`tauri signer generate`) and its signature of SIGNED_SUMS
    // (`tauri signer sign`). Not the release key.
    const TEST_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDgyRkU2NkU4Mzc1N0Y2MEQKUldRTjlsYzM2R2IrZ2lmdFZZY3AzYnh1TUNwTVlMaWgwTmlHd2ZVUXRudW5tUnplT3pUSXJPMlkK";
    const OTHER_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEM3MDc4MDUxREVCNDA2MjAKUldRZ0JyVGVVWUFIeDNkWEpLZC8yME1kZzFmTzMrZUppYWJzenUxT1hMRzhhejJFMm1yaWlva08K";
    const SIGNED_SUMS: &str = "0f343b0931126a20f133d67c2b018a3b5ed1c2f3c2d4f6d9a8f4e1e9b7c5a3d1  Pinhole-9.0.0-windows-x64-setup.exe\n";
    const TEST_SIG: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVRTjlsYzM2R2IrZ3E5UnFnVkZYdWRoNnNwR0h3UVZQei9lOVVRRWIxVm9JWG84cGswRmJWd0crWWhLOUd4dFRzdHNLanVnaEZnVnpVZHhUZ0tYWHF6Q3Y0bW1FNmtUY3dzPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkwODc2OTcwCWZpbGU6U0hBMjU2U1VNUy50eHQKWDVWWXFXUE5XNlRxWC96dFBCWjliMklYdzRYSGl0aWg0K0RJM2JGenlhenVXUGJDODJhQjE2Y1R2Y052ZWVWL1ZmNFZjemJCTW5ZUFhnVEhZdGQvQ1E9PQo=";

    #[test]
    fn signed_checksum_list_is_accepted() {
        verify_sums(SIGNED_SUMS.as_bytes(), TEST_SIG.as_bytes(), TEST_KEY).unwrap();
        // Trailing newlines in the .sig / .pub files are fine.
        let sig = format!("{TEST_SIG}\n");
        verify_sums(
            SIGNED_SUMS.as_bytes(),
            sig.as_bytes(),
            &format!("{TEST_KEY}\n"),
        )
        .unwrap();
    }

    #[test]
    fn changed_list_other_key_or_garbage_is_refused() {
        let swapped = SIGNED_SUMS.replace("0f34", "aaaa");
        let cases: [(&[u8], &[u8], &str); 4] = [
            (swapped.as_bytes(), TEST_SIG.as_bytes(), TEST_KEY),
            (SIGNED_SUMS.as_bytes(), TEST_SIG.as_bytes(), OTHER_KEY),
            (SIGNED_SUMS.as_bytes(), b"not a signature", TEST_KEY),
            (SIGNED_SUMS.as_bytes(), b"", TEST_KEY),
        ];
        for (sums, sig, key) in cases {
            let err = verify_sums(sums, sig, key).unwrap_err();
            assert_eq!(err.code, "hash_mismatch");
        }
        // A broken built-in key is this copy's problem, not the release's.
        let err = verify_sums(SIGNED_SUMS.as_bytes(), TEST_SIG.as_bytes(), "").unwrap_err();
        assert_eq!(err.code, "internal");
    }

    #[test]
    fn built_in_update_key_is_empty_or_valid() {
        if SELF_UPDATE {
            use base64::Engine as _;
            let text = base64::engine::general_purpose::STANDARD
                .decode(UPDATE_PUBLIC_KEY.trim())
                .expect("update-key.pub must be the base64 .pub file from `tauri signer generate`");
            minisign_verify::PublicKey::decode(&String::from_utf8(text).unwrap())
                .expect("update-key.pub is not a minisign public key");
        } else {
            assert!(UPDATE_PUBLIC_KEY.trim().is_empty());
        }
    }

    #[test]
    fn stable_copies_are_not_offered_pre_releases() {
        let releases = vec![rel("v1.1.0-rc.1", false, &[]), rel("v1.0.1", false, &[])];
        let stable = semver::Version::new(1, 0, 0);
        let info = pick_update(&releases, &stable, &Target::Manual).unwrap();
        assert_eq!(info.version, "1.0.1");
        let only_rc = vec![rel("v1.1.0-rc.1", false, &[])];
        assert!(pick_update(&only_rc, &stable, &Target::Manual).is_none());
        let rc = semver::Version::parse("1.1.0-rc.0").unwrap();
        let info = pick_update(&releases, &rc, &Target::Manual).unwrap();
        assert_eq!(info.version, "1.1.0-rc.1");
    }

    fn write_zip(path: &Path, files: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (name, data) in files {
            z.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn portable_update_replaces_app_files_and_keeps_data() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("Pinhole");
        std::fs::create_dir_all(app.join("config")).unwrap();
        std::fs::create_dir_all(app.join("Data").join("outputs")).unwrap();
        std::fs::write(app.join("Pinhole.exe"), b"old exe").unwrap();
        std::fs::write(app.join("config").join("models.yaml"), b"old").unwrap();
        std::fs::write(app.join("config").join("removed.yaml"), b"stale").unwrap();
        std::fs::write(
            app.join("Data").join("outputs").join("mine.png"),
            b"keep me",
        )
        .unwrap();

        let zip = tmp.path().join("u.zip");
        write_zip(
            &zip,
            &[
                ("Pinhole/Pinhole.exe", b"new exe"),
                ("Pinhole/config/models.yaml", b"new"),
                ("Pinhole/Data/README.txt", b"must not land"),
                ("Other/evil.txt", b"ignored"),
            ],
        );
        apply_portable(&zip, &app).unwrap();

        assert_eq!(std::fs::read(app.join("Pinhole.exe")).unwrap(), b"new exe");
        assert_eq!(
            std::fs::read(app.join("config").join("models.yaml")).unwrap(),
            b"new"
        );
        assert!(
            !app.join("config").join("removed.yaml").exists(),
            "config/ is replaced as a whole"
        );
        assert_eq!(
            std::fs::read(app.join("Data").join("outputs").join("mine.png")).unwrap(),
            b"keep me"
        );
        assert!(!app.join("Data").join("README.txt").exists());
        assert!(!app.join("evil.txt").exists());
        assert_eq!(
            std::fs::read(app.join(STAGING_DIR).join("old").join("Pinhole.exe")).unwrap(),
            b"old exe"
        );

        let data = tmp.path().join("Data");
        std::fs::create_dir_all(data.join(STAGING_DIR)).unwrap();
        cleanup_after_update(&app, &data);
        assert!(!app.join(STAGING_DIR).exists());
        assert!(
            !data.join(STAGING_DIR).exists(),
            "a .deb download left in Data"
        );
    }

    #[test]
    fn portable_update_without_exe_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("Pinhole");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("Pinhole.exe"), b"old exe").unwrap();
        let zip = tmp.path().join("u.zip");
        write_zip(&zip, &[("Pinhole/config/models.yaml", b"new")]);
        assert!(apply_portable(&zip, &app).is_err());
        assert_eq!(std::fs::read(app.join("Pinhole.exe")).unwrap(), b"old exe");
        assert!(!app.join("config").exists());
        // Zip-slip entries are refused by the shared zip reader.
        write_zip(
            &zip,
            &[("Pinhole/Pinhole.exe", b"new exe"), ("../escape.txt", b"x")],
        );
        assert!(apply_portable(&zip, &app).is_err());
        assert_eq!(std::fs::read(app.join("Pinhole.exe")).unwrap(), b"old exe");
        assert!(!tmp.path().join("escape.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn appimage_is_replaced_in_place_and_executable() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let live = tmp.path().join("Pinhole.AppImage");
        std::fs::write(&live, b"old").unwrap();
        let new = tmp.path().join(STAGING_DIR);
        std::fs::create_dir_all(&new).unwrap();
        let new = new.join("next.AppImage");
        std::fs::write(&new, b"new").unwrap();
        apply_appimage(&new, &live).unwrap();
        assert_eq!(std::fs::read(&live).unwrap(), b"new");
        assert_eq!(
            std::fs::metadata(&live).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }
}
