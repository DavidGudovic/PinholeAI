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
//! * Anything else (the .deb, dev builds): the UI opens the release page instead.
//!
//! Integrity: the file must match the size GitHub reports and the SHA-256 listed in
//! the release's `SHA256SUMS.txt`. That catches corrupted or swapped CDN downloads,
//! not a compromised GitHub account; signed updates are RELEASE-SPEC work.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use pinhole_net::download::{DownloadKind, DownloadSpec};
use serde::{Deserialize, Serialize};

use crate::{AppCore, CoreError, CoreResult};

/// The GitHub repository releases come from.
pub const REPO: &str = "DavidGudovic/PinholeAI";
const RELEASES_API: &str = "https://api.github.com/repos/DavidGudovic/PinholeAI/releases?per_page=30";
const SUMS_FILE: &str = "SHA256SUMS.txt";
const MAX_SUMS_BYTES: usize = 64 * 1024;
/// Staging folder name (next to the app for portable / AppImage, in the OS temp
/// dir for the Windows installer). Removed on the next start.
pub const STAGING_DIR: &str = ".pinhole-update";
/// Upper bound for everything unpacked from a portable zip.
const MAX_UNPACKED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const PRODUCT: &str = "Pinhole";

/// How this copy of Pinhole can update itself. `UpdateInstallMode` in `src/lib/types.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallMode {
    Installer,
    Portable,
    AppImage,
    /// Can't replace itself (the .deb, dev builds): open the release page.
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
    Installer { staging: PathBuf },
    Portable { app_dir: PathBuf },
    AppImage { file: PathBuf },
    Manual,
}

impl Target {
    pub fn mode(&self) -> InstallMode {
        match self {
            Target::Installer { .. } => InstallMode::Installer,
            Target::Portable { .. } => InstallMode::Portable,
            Target::AppImage { .. } => InstallMode::AppImage,
            Target::Manual => InstallMode::Manual,
        }
    }

    fn asset_name(&self, version: &str) -> Option<String> {
        match self {
            Target::Installer { .. } => Some(format!("{PRODUCT}-{version}-windows-x64-setup.exe")),
            Target::Portable { .. } => Some(format!("{PRODUCT}-{version}-windows-x64-portable.zip")),
            Target::AppImage { .. } => Some(format!("{PRODUCT}-{version}-linux-x86_64.AppImage")),
            Target::Manual => None,
        }
    }

    fn staging(&self) -> Option<PathBuf> {
        match self {
            Target::Installer { staging } => Some(staging.clone()),
            Target::Portable { app_dir } => Some(app_dir.join(STAGING_DIR)),
            Target::AppImage { file } => file.parent().map(|p| p.join(STAGING_DIR)),
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
        }
    }
}

pub fn detect_target_with(env: &Environment) -> Target {
    if env.debug_build {
        return Target::Manual;
    }
    match env.os {
        "windows" => {
            if env.exe_dir.join("uninstall.exe").is_file() {
                Target::Installer { staging: env.temp_dir.join("pinhole-update") }
            } else if env.portable_data && env.exe_dir.join(format!("{PRODUCT}.exe")).is_file() {
                Target::Portable { app_dir: env.exe_dir.clone() }
            } else {
                Target::Manual
            }
        }
        "linux" => match &env.appimage {
            Some(file) if file.is_file() && file.parent().is_some_and(writable_dir) => Target::AppImage { file: file.clone() },
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

/// Newest published (non-draft) release newer than `current`. Pre-releases count:
/// every build is a pre-release until RELEASE-SPEC is done.
fn newest_release<'a>(releases: &'a [GhRelease], current: &semver::Version) -> Option<(&'a GhRelease, semver::Version)> {
    releases
        .iter()
        .filter(|r| !r.draft)
        .filter_map(|r| tag_version(&r.tag_name).map(|v| (r, v)))
        .filter(|(_, v)| v > current)
        .max_by(|a, b| a.1.cmp(&b.1))
}

/// Download URLs are built from our repo + a tag that parsed as semver + an asset
/// name we chose, never taken from the API response.
fn asset_url(version: &semver::Version, name: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/v{version}/{name}")
}

/// Release page for "What's new" / manual installs. `None` → the releases list.
pub fn release_page_url(version: Option<&str>) -> CoreResult<String> {
    match version {
        None => Ok(format!("https://github.com/{REPO}/releases")),
        Some(v) => {
            let v = semver::Version::parse(v).map_err(|_| CoreError::invalid("That isn't a Pinhole version."))?;
            Ok(format!("https://github.com/{REPO}/releases/tag/v{v}"))
        }
    }
}

/// `SHA256SUMS.txt` (`sha256sum` format, optional `*` binary marker) → hash of `name`.
fn sum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim_start().trim_start_matches('*');
        (file == name && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
    })
}

fn current_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("crate version is semver")
}

async fn fetch_releases(core: &AppCore) -> CoreResult<Vec<GhRelease>> {
    let headers = [("accept", "application/vnd.github+json"), ("x-github-api-version", "2022-11-28")];
    core.http.get_json::<Vec<GhRelease>>(RELEASES_API, &headers).await.map_err(|e| match e {
        pinhole_net::NetError::Offline => CoreError::new("offline", "Offline mode is on. Turn it off in Settings to check for updates."),
        pinhole_net::NetError::Unauthorized(_) | pinhole_net::NetError::Status(429) => {
            CoreError::new("network", "GitHub is limiting update checks right now. Try again in an hour.").with_details(e.to_string())
        }
        other => CoreError::from(other),
    })
}

/// "Check for updates": one request to the GitHub releases API.
pub async fn check_for_updates(core: &AppCore) -> CoreResult<UpdateCheck> {
    let current = current_version();
    let releases = fetch_releases(core).await?;
    let target = detect_target_with(&Environment::current(core));
    let update = newest_release(&releases, &current).map(|(r, v)| {
        let version = v.to_string();
        let size_bytes = target
            .asset_name(&version)
            .and_then(|name| r.assets.iter().find(|a| a.name == name).map(|a| a.size));
        // A release without the file this copy needs can only be installed by hand.
        let install_mode = if size_bytes.is_some() { target.mode() } else { InstallMode::Manual };
        UpdateInfo { version, published_at: r.published_at.clone(), install_mode, size_bytes }
    });
    Ok(UpdateCheck { current_version: current.to_string(), update })
}

/// Download `version`, verify it and put it in place. The caller (Tauri shell)
/// then stops the engines and runs / relaunches what [`Prepared`] says.
pub async fn install_update(core: &Arc<AppCore>, version: &str) -> CoreResult<Prepared> {
    let wanted = semver::Version::parse(version).map_err(|_| CoreError::invalid("That isn't a Pinhole version."))?;
    if wanted <= current_version() {
        return Err(CoreError::invalid("This version is already installed."));
    }
    let target = detect_target_with(&Environment::current(core));
    let (Some(name), Some(staging)) = (target.asset_name(&wanted.to_string()), target.staging()) else {
        return Err(CoreError::invalid("This copy of Pinhole can't update itself. Open the download page and install the new version from there."));
    };

    let releases = fetch_releases(core).await?;
    let release = releases
        .iter()
        .find(|r| !r.draft && tag_version(&r.tag_name).as_ref() == Some(&wanted))
        .ok_or_else(|| CoreError::not_found("That update isn't available any more. Check for updates again."))?;
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| CoreError::not_found("This release has no download for your system. Open the download page instead."))?;
    if !release.assets.iter().any(|a| a.name == SUMS_FILE) {
        return Err(CoreError::new("hash_mismatch", "This release can't be checked (it has no SHA256SUMS.txt), so Pinhole won't install it."));
    }
    let sums = core.http.get_bytes(&asset_url(&wanted, SUMS_FILE), &[], MAX_SUMS_BYTES).await?;
    let sha256 = sum_for(&String::from_utf8_lossy(&sums), &name)
        .ok_or_else(|| CoreError::new("hash_mismatch", "This release's checksum list doesn't include your download, so Pinhole won't install it."))?;

    std::fs::create_dir_all(&staging)?;
    let spec = DownloadSpec {
        url: asset_url(&wanted, &name),
        dest: staging.join(&name),
        sha256: Some(sha256),
        size_bytes: Some(asset.size),
        label: format!("{PRODUCT} {wanted}"),
        ..Default::default()
    };
    let group = core.downloads.enqueue_kind(format!("{PRODUCT} {wanted}"), DownloadKind::AppUpdate, vec![spec]);
    let files = core.downloads.wait_detailed(&group).await.map_err(|e| CoreError::new(&e.code, e.message))?;
    let downloaded = files
        .into_iter()
        .next()
        .ok_or_else(|| CoreError::internal("The update download finished without a file. Try again."))?
        .path;

    match target {
        Target::Installer { .. } => Ok(Prepared::RunInstaller(downloaded)),
        Target::Portable { app_dir } => {
            let exe = app_dir.join(format!("{PRODUCT}.exe"));
            tokio::task::spawn_blocking(move || apply_portable(&downloaded, &app_dir))
                .await
                .map_err(|e| CoreError::internal("The update stopped unexpectedly. Try again.").with_details(e.to_string()))??;
            Ok(Prepared::Relaunch(exe))
        }
        Target::AppImage { file } => {
            let dest = file.clone();
            tokio::task::spawn_blocking(move || apply_appimage(&downloaded, &dest))
                .await
                .map_err(|e| CoreError::internal("The update stopped unexpectedly. Try again.").with_details(e.to_string()))??;
            Ok(Prepared::Relaunch(file))
        }
        Target::Manual => unreachable!("manual targets return early"),
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
        std::fs::set_permissions(new_file, std::fs::Permissions::from_mode(0o755)).map_err(update_failed)?;
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
        return Err(CoreError::new("invalid", "The downloaded update doesn't contain Pinhole. Nothing was changed."));
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
        let _ = if live.is_dir() { std::fs::remove_dir_all(&live) } else { std::fs::remove_file(&live) };
        if *had_old {
            let _ = std::fs::rename(old_dir.join(name), &live);
        }
    }
}

fn unpack_portable(zip_path: &Path, out: &Path) -> CoreResult<()> {
    let bad = |e: &dyn std::fmt::Display| CoreError::new("invalid", "The downloaded update couldn't be unpacked. Try again.").with_details(e.to_string());
    let file = std::fs::File::open(zip_path).map_err(update_failed)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| bad(&e))?;
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| bad(&e))?;
        let Some(path) = entry.enclosed_name() else {
            return Err(bad(&"unsafe path in zip"));
        };
        let Some(rel) = portable_relative(&path) else { continue };
        let dest = out.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest).map_err(update_failed)?;
            continue;
        }
        total = total.saturating_add(entry.size());
        if total > MAX_UNPACKED_BYTES {
            return Err(bad(&"zip too large"));
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(update_failed)?;
        }
        let mut f = std::fs::File::create(&dest).map_err(update_failed)?;
        std::io::copy(&mut entry, &mut f).map_err(update_failed)?;
    }
    Ok(())
}

/// `Pinhole/config/models.yaml` → `config/models.yaml`. Entries outside `Pinhole/`
/// and anything under `Pinhole/Data/` (the user's data) → `None`.
fn portable_relative(path: &Path) -> Option<PathBuf> {
    let mut comps = path.components();
    match comps.next() {
        Some(Component::Normal(first)) if first == PRODUCT => {}
        _ => return None,
    }
    let rel: PathBuf = comps.collect();
    let top = rel.components().next()?;
    if matches!(top, Component::Normal(t) if t.eq_ignore_ascii_case("Data")) {
        return None;
    }
    Some(rel)
}

/// Remove leftovers of a finished (or abandoned) update. Best effort: the old exe
/// of a portable update may still be exiting; the next start tries again.
pub fn cleanup_after_update(exe_dir: &Path) {
    let mut dirs = vec![exe_dir.join(STAGING_DIR), std::env::temp_dir().join("pinhole-update")];
    if let Some(parent) = std::env::var_os("APPIMAGE").as_ref().and_then(|p| Path::new(p).parent().map(Path::to_path_buf)) {
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
            assets: assets.iter().map(|(n, s)| GhAsset { name: (*n).into(), size: *s }).collect(),
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
            rel("v0.10.0-rc.1", false, &[]),
        ];
        let (r, v) = newest_release(&list, &current).unwrap();
        assert_eq!((r.tag_name.as_str(), v.to_string().as_str()), ("v0.10.0-rc.1", "0.10.0-rc.1"));
        assert!(newest_release(&list[4..5], &current).is_none(), "same version is not an update");
        assert!(newest_release(&list[..1], &current).is_none(), "drafts are ignored");
    }

    #[test]
    fn checksum_lookup() {
        let h = "a".repeat(64);
        let sums = format!("{h}  Pinhole-0.2.0-windows-x64-setup.exe\n{}  *Pinhole-0.2.0-linux-x86_64.AppImage\n", "B".repeat(64));
        assert_eq!(sum_for(&sums, "Pinhole-0.2.0-windows-x64-setup.exe"), Some(h));
        assert_eq!(sum_for(&sums, "Pinhole-0.2.0-linux-x86_64.AppImage"), Some("b".repeat(64)));
        assert_eq!(sum_for(&sums, "Pinhole-0.2.0-windows-x64-portable.zip"), None);
        assert_eq!(sum_for("xyz  Pinhole.exe", "Pinhole.exe"), None);
    }

    #[test]
    fn urls_are_built_not_taken_from_the_api() {
        let v = semver::Version::parse("0.2.0").unwrap();
        assert_eq!(
            asset_url(&v, "SHA256SUMS.txt"),
            "https://github.com/DavidGudovic/PinholeAI/releases/download/v0.2.0/SHA256SUMS.txt"
        );
        assert_eq!(release_page_url(Some("0.2.0")).unwrap(), "https://github.com/DavidGudovic/PinholeAI/releases/tag/v0.2.0");
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
        assert_eq!(detect_target_with(&e), Target::Portable { app_dir: dir.to_path_buf() });
        // Installer wins when uninstall.exe is there.
        std::fs::write(dir.join("uninstall.exe"), b"").unwrap();
        assert_eq!(detect_target_with(&e), Target::Installer { staging: dir.join("tmp").join("pinhole-update") });
        // Dev builds never replace themselves.
        e.debug_build = true;
        assert_eq!(detect_target_with(&e), Target::Manual);
        // Linux: only an AppImage can update itself.
        assert_eq!(detect_target_with(&env("linux", dir)), Target::Manual);
        let img = dir.join("Pinhole.AppImage");
        std::fs::write(&img, b"").unwrap();
        let mut l = env("linux", dir);
        l.appimage = Some(img.clone());
        assert_eq!(detect_target_with(&l), Target::AppImage { file: img });
        assert_eq!(detect_target_with(&env("macos", dir)), Target::Manual);
    }

    fn write_zip(path: &Path, files: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (name, data) in files {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
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
        std::fs::write(app.join("Data").join("outputs").join("mine.png"), b"keep me").unwrap();

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
        assert_eq!(std::fs::read(app.join("config").join("models.yaml")).unwrap(), b"new");
        assert!(!app.join("config").join("removed.yaml").exists(), "config/ is replaced as a whole");
        assert_eq!(std::fs::read(app.join("Data").join("outputs").join("mine.png")).unwrap(), b"keep me");
        assert!(!app.join("Data").join("README.txt").exists());
        assert!(!app.join("evil.txt").exists());
        assert_eq!(std::fs::read(app.join(STAGING_DIR).join("old").join("Pinhole.exe")).unwrap(), b"old exe");

        cleanup_after_update(&app);
        assert!(!app.join(STAGING_DIR).exists());
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
        assert_eq!(std::fs::metadata(&live).unwrap().permissions().mode() & 0o777, 0o755);
    }
}
