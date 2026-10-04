//! Engine install: download specs for the pinned archives, zip-slip-safe unpack
//! into `Data/engine/<sd|llama>/<version>/<backend>/` (atomic: unpack into a
//! temp folder, then rename), `chmod +x` on Unix, locate the server binary and
//! write a small marker file so "is it installed?" is cheap.
//!
//! The marker holds only version/backend/archive hashes — never prompts.

use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::pins::{exe_name, ArchiveSpec, EngineConfig, EnginePin, SelectedBuild};
use crate::EngineError;

pub const MARKER_FILE: &str = "pinhole-engine.json";
/// Refuse single archive entries above this size (decompression bomb guard).
const MAX_ENTRY_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    /// stable-diffusion.cpp `sd-server`.
    Sd,
    /// llama.cpp `llama-server` (captioner).
    Llama,
}

impl EngineKind {
    pub fn dir_name(self) -> &'static str {
        match self {
            EngineKind::Sd => "sd",
            EngineKind::Llama => "llama",
        }
    }
    pub fn pin(self, cfg: &EngineConfig) -> &EnginePin {
        match self {
            EngineKind::Sd => &cfg.stable_diffusion_cpp,
            EngineKind::Llama => &cfg.llama_cpp,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            EngineKind::Sd => "Image engine",
            EngineKind::Llama => "Describe engine",
        }
    }
}

/// Written next to the binaries after a successful unpack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallMarker {
    pub engine: EngineKind,
    pub version: String,
    pub backend: String,
    pub build: String,
    /// Path of the server binary relative to the install folder (`/`-separated).
    pub binary: String,
    /// `(archive file name, sha256)` actually unpacked.
    pub archives: Vec<(String, String)>,
    /// Unix seconds.
    pub installed_at: i64,
}

/// A ready-to-launch engine.
#[derive(Debug, Clone, PartialEq)]
pub struct InstalledEngine {
    pub kind: EngineKind,
    pub dir: PathBuf,
    pub exe: PathBuf,
    pub version: String,
    pub backend: String,
}

/// `Data/engine/<sd|llama>/<version>/<backend>/`
pub fn install_dir(engine_root: &Path, kind: EngineKind, version: &str, backend: &str) -> PathBuf {
    engine_root
        .join(kind.dir_name())
        .join(sanitize(version))
        .join(sanitize(backend))
}

/// Where archives are downloaded before unpacking (`Data/engine/downloads/`).
pub fn download_dir(engine_root: &Path) -> PathBuf {
    engine_root.join("downloads")
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('.')
        .to_string()
}

/// Read the marker of an installed engine; `None` if missing / other version / binary gone.
pub fn find_installed(
    engine_root: &Path,
    kind: EngineKind,
    version: &str,
    backend: &str,
) -> Option<InstalledEngine> {
    let dir = install_dir(engine_root, kind, version, backend);
    let text = fs::read_to_string(dir.join(MARKER_FILE)).ok()?;
    let marker: InstallMarker = serde_json::from_str(&text).ok()?;
    if marker.engine != kind || marker.version != version || marker.backend != backend {
        return None;
    }
    let rel = safe_relative(&marker.binary)?;
    let exe = dir.join(rel);
    exe.is_file().then(|| InstalledEngine {
        kind,
        dir: dir.clone(),
        exe,
        version: marker.version,
        backend: marker.backend,
    })
}

/// Any installed backend of this engine version (prefers `preferred` order).
pub fn find_any_installed(
    engine_root: &Path,
    kind: EngineKind,
    version: &str,
    preferred: &[String],
) -> Option<InstalledEngine> {
    for b in preferred {
        if let Some(e) = find_installed(engine_root, kind, version, b) {
            return Some(e);
        }
    }
    for b in ["cuda", "vulkan", "cpu"] {
        if let Some(e) = find_installed(engine_root, kind, version, b) {
            return Some(e);
        }
    }
    None
}

/// Download specs for a selected build (archives go to `Data/engine/downloads/`).
pub fn download_specs(
    engine_root: &Path,
    kind: EngineKind,
    pin: &EnginePin,
    sel: &SelectedBuild,
) -> Vec<pinhole_net::download::DownloadSpec> {
    let dir = download_dir(engine_root);
    let archives = sel.build.archives();
    let n = archives.len();
    archives
        .iter()
        .enumerate()
        .map(|(i, a)| pinhole_net::download::DownloadSpec {
            url: a.url.clone(),
            dest: dir.join(format!(
                "{}-{}-{}",
                kind.dir_name(),
                sanitize(&pin.version),
                a.file_name()
            )),
            sha256: a.verified_sha256(),
            // `size_bytes` in engine.yaml is exact; `size_mb` is only an estimate.
            size_bytes: a.size_bytes,
            approx_size_bytes: a.bytes(),
            label: if n > 1 {
                format!("{} ({}/{n})", kind.label(), i + 1)
            } else {
                kind.label().to_string()
            },
            ..Default::default()
        })
        .collect()
}

/// Unpack downloaded archives (in build order) into the final install folder,
/// atomically. `downloaded` pairs each archive spec with its local file and the
/// SHA-256 the downloader computed; a known expected hash must match.
pub fn unpack_build(
    engine_root: &Path,
    kind: EngineKind,
    pin: &EnginePin,
    sel: &SelectedBuild,
    downloaded: &[(ArchiveSpec, PathBuf, String)],
) -> Result<InstalledEngine, EngineError> {
    for (spec, _, actual) in downloaded {
        if crate::pins::REQUIRE_PINNED_HASHES && spec.verified_sha256().is_none() {
            return Err(EngineError::Unpinned(spec.file_name()));
        }
        if let Some(expected) = spec.verified_sha256() {
            if !expected.eq_ignore_ascii_case(actual) {
                return Err(EngineError::HashMismatch {
                    file: spec.file_name(),
                    expected,
                    actual: actual.clone(),
                });
            }
        }
    }
    // A working install of this version/backend is kept as it is.
    if let Some(done) = find_installed(engine_root, kind, &pin.version, &sel.backend) {
        return Ok(done);
    }
    let final_dir = install_dir(engine_root, kind, &pin.version, &sel.backend);
    let parent = final_dir
        .parent()
        .ok_or_else(|| EngineError::Config("bad install dir".into()))?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        sanitize(&sel.backend),
        std::process::id()
    ));
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    fs::create_dir_all(&tmp)?;
    let result = (|| -> Result<InstallMarker, EngineError> {
        // Each archive is unpacked into its own staging folder; a single top-level
        // folder (`llama-b11235/`, `cudart-…/`) is flattened away and the content
        // merged into one folder, so runtime libraries (cudart DLLs / .so) always
        // sit next to the server binary.
        for (i, (spec, path, _)) in downloaded.iter().enumerate() {
            let stage = tmp.join(format!(".stage-{i}"));
            fs::create_dir_all(&stage)?;
            extract_archive(path, &stage).map_err(|e| match e {
                EngineError::Archive(m) => {
                    EngineError::Archive(format!("{}: {m}", spec.file_name()))
                }
                other => other,
            })?;
            let src = single_subdir(&stage).unwrap_or_else(|| stage.clone());
            merge_move(&src, &tmp)?;
            let _ = fs::remove_dir_all(&stage);
        }
        let exe = find_binary(&tmp, &pin.binary)
            .ok_or_else(|| EngineError::BinaryMissing(exe_name(&pin.binary)))?;
        make_executable(&exe)?;
        let rel = exe
            .strip_prefix(&tmp)
            .map_err(|_| EngineError::BinaryMissing(pin.binary.clone()))?;
        let rel = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        Ok(InstallMarker {
            engine: kind,
            version: pin.version.clone(),
            backend: sel.backend.clone(),
            build: sel.key.clone(),
            binary: rel,
            archives: downloaded
                .iter()
                .map(|(s, _, h)| (s.file_name(), h.clone()))
                .collect(),
            installed_at: now_secs(),
        })
    })();
    let marker = match result {
        Ok(m) => m,
        Err(e) => {
            let _ = fs::remove_dir_all(&tmp);
            return Err(e);
        }
    };
    let json =
        serde_json::to_vec_pretty(&marker).map_err(|e| EngineError::Archive(e.to_string()))?;
    let placed = (|| -> io::Result<()> {
        fs::write(tmp.join(MARKER_FILE), json)?;
        if final_dir.exists() {
            fs::remove_dir_all(&final_dir)?;
        }
        fs::rename(&tmp, &final_dir)
    })();
    if let Err(e) = placed {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e.into());
    }
    find_installed(engine_root, kind, &pin.version, &sel.backend)
        .ok_or_else(|| EngineError::BinaryMissing(pin.binary.clone()))
}

/// `Some(dir/only_child)` when `dir` contains exactly one entry and it is a real directory.
fn single_subdir(dir: &Path) -> Option<PathBuf> {
    let mut it = fs::read_dir(dir).ok()?.filter_map(|e| e.ok());
    let first = it.next()?;
    if it.next().is_some() {
        return None;
    }
    let ft = first.file_type().ok()?;
    (ft.is_dir() && !ft.is_symlink()).then(|| first.path())
}

/// Move every entry of `src` into `dst` (files replace, directories merge).
fn merge_move(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let ft = entry.file_type()?;
        if ft.is_dir() && !ft.is_symlink() {
            if to.is_dir() {
                merge_move(&from, &to)?;
                continue;
            }
            if fs::symlink_metadata(&to).is_ok() {
                fs::remove_file(&to)?;
            }
        } else if let Ok(meta) = fs::symlink_metadata(&to) {
            if meta.is_dir() {
                fs::remove_dir_all(&to)?;
            } else {
                fs::remove_file(&to)?;
            }
        }
        fs::rename(&from, &to)?;
    }
    Ok(())
}

/// Remove downloaded archives after a successful unpack (best effort).
pub fn cleanup_downloads(paths: &[PathBuf]) {
    for p in paths {
        let _ = fs::remove_file(p);
    }
}

/// Best effort: remove what no pinned engine can use any more from `engine_root`:
/// `<sd|llama>/<version>/` folders of other versions, unpack staging folders
/// (`.<backend>.tmp-<pid>`) whose process is no longer running, and files in
/// `downloads/` that are not an archive (or its `.part`) of a pinned build.
/// Errors are ignored per entry. Run it when no engine of another version is
/// running (e.g. after the orphan sweep).
pub fn sweep_stale(engine_root: &Path, cfg: &EngineConfig) {
    let mut keep_files = std::collections::HashSet::new();
    for kind in [EngineKind::Sd, EngineKind::Llama] {
        let pin = kind.pin(cfg);
        let pinned = sanitize(&pin.version);
        for build in pin.builds.values() {
            for a in build.archives() {
                let name = format!("{}-{pinned}-{}", kind.dir_name(), a.file_name());
                keep_files.insert(format!("{name}.part"));
                keep_files.insert(name);
            }
        }
        for (name, path) in real_entries(&engine_root.join(kind.dir_name()), true) {
            if name != pinned {
                let _ = fs::remove_dir_all(&path);
                continue;
            }
            for (name, path) in real_entries(&path, true) {
                if !name.starts_with('.') {
                    continue;
                }
                let Some((_, pid)) = name.rsplit_once(".tmp-") else {
                    continue;
                };
                // Another Pinhole running on the same Data folder may be unpacking.
                if pid.parse::<u32>().is_ok_and(crate::orphans::pid_running) {
                    continue;
                }
                let _ = fs::remove_dir_all(&path);
            }
        }
    }
    for (name, path) in real_entries(&download_dir(engine_root), false) {
        if !keep_files.contains(&name) {
            let _ = fs::remove_file(&path);
        }
    }
}

/// `(name, path)` of the directories (`dirs`) or files (`!dirs`) directly in
/// `dir`; symlinks and unreadable entries are skipped.
fn real_entries(dir: &Path, dirs: bool) -> Vec<(String, PathBuf)> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type()
                .is_ok_and(|t| !t.is_symlink() && if dirs { t.is_dir() } else { t.is_file() })
        })
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .collect()
}

/// Download (via the one allow-listed [`pinhole_net::HttpClient`]) + verify +
/// unpack without the DownloadManager. Used by the CI engine smoke test.
pub async fn install_direct(
    client: &pinhole_net::HttpClient,
    engine_root: &Path,
    cfg: &EngineConfig,
    kind: EngineKind,
    os: &str,
    backend: &str,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<InstalledEngine, EngineError> {
    let pin = kind.pin(cfg);
    let sel = cfg.select_build(pin, os, backend)?;
    if let Some(done) = find_installed(engine_root, kind, &pin.version, &sel.backend) {
        return Ok(done);
    }
    fs::create_dir_all(download_dir(engine_root))?;
    let specs = download_specs(engine_root, kind, pin, &sel);
    let mut downloaded = Vec::new();
    for (spec, archive) in specs.iter().zip(sel.build.archives()) {
        let noop = |_: u64, _: Option<u64>| {};
        let file = pinhole_net::download::download_file(client, spec, cancel, &noop)
            .await
            .map_err(|e| EngineError::Download(e.to_string()))?;
        downloaded.push((archive, file.path, file.sha256));
    }
    let root = engine_root.to_path_buf();
    let pin2 = pin.clone();
    let sel2 = sel.clone();
    let dl = downloaded.clone();
    let installed =
        tokio::task::spawn_blocking(move || unpack_build(&root, kind, &pin2, &sel2, &dl))
            .await
            .map_err(|e| EngineError::Archive(e.to_string()))??;
    cleanup_downloads(&downloaded.iter().map(|d| d.1.clone()).collect::<Vec<_>>());
    Ok(installed)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ------------------------------------------------------------------ Windows runtime

/// MSVC runtime DLLs (VC++ 2015–2022 x64 redistributable) the upstream Windows
/// builds import. Not included in the release zips; present on most PCs but not
/// on a fresh Windows install.
pub fn msvc_runtime_dlls(kind: EngineKind) -> &'static [&'static str] {
    match kind {
        // ggml-base / ggml-cpu use MSVC OpenMP (vcomp140); stable-diffusion.dll uses codecvt ids.
        EngineKind::Sd => &[
            "msvcp140.dll",
            "vcruntime140.dll",
            "vcruntime140_1.dll",
            "vcomp140.dll",
            "msvcp140_codecvt_ids.dll",
        ],
        // llama.cpp ships its own libomp.dll.
        EngineKind::Llama => &["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"],
    }
}

/// Windows: make sure the MSVC runtime DLLs are loadable by the engine in
/// `exe_dir` (found there or in System32; otherwise copied from `bundled`, e.g.
/// the app's `vcrt/` resource folder). Returns the DLLs still missing. Always
/// empty on other systems.
pub fn provide_runtime_dlls(
    exe_dir: &Path,
    kind: EngineKind,
    bundled: &[PathBuf],
) -> Vec<&'static str> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let system = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32"));
    runtime_dlls_missing_after_copy(exe_dir, msvc_runtime_dlls(kind), system.as_deref(), bundled)
}

fn runtime_dlls_missing_after_copy(
    exe_dir: &Path,
    dlls: &[&'static str],
    system: Option<&Path>,
    bundled: &[PathBuf],
) -> Vec<&'static str> {
    let mut missing = Vec::new();
    for dll in dlls {
        if exe_dir.join(dll).is_file() || system.map(|s| s.join(dll).is_file()).unwrap_or(false) {
            continue;
        }
        let copied = bundled
            .iter()
            .map(|b| b.join(dll))
            .find(|p| p.is_file())
            .map(|src| fs::copy(&src, exe_dir.join(dll)).is_ok())
            .unwrap_or(false);
        if !copied {
            missing.push(*dll);
        }
    }
    missing
}

// ------------------------------------------------------------------ extraction

/// A `/`- or `\`-separated archive path → safe relative path, or `None` if it
/// is absolute, has a drive/UNC prefix, or contains `..`.
pub fn safe_relative(name: &str) -> Option<PathBuf> {
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains('\0') {
        return None;
    }
    let mut out = PathBuf::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            p if p.contains(':') => return None, // `C:` drive prefixes, NTFS streams
            p => out.push(p),
        }
    }
    // Belt and braces: only normal components.
    if out.as_os_str().is_empty() || out.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(out)
}

/// Join an archive entry name under `dest`, refusing zip-slip.
pub fn safe_join(dest: &Path, name: &str) -> Result<PathBuf, EngineError> {
    safe_relative(name)
        .map(|rel| dest.join(rel))
        .ok_or_else(|| EngineError::UnsafeArchive(name.chars().take(200).collect()))
}

/// Extract `.zip` or `.tar.gz`/`.tgz` into `dest` (must exist).
pub fn extract_archive(archive: &Path, dest: &Path) -> Result<(), EngineError> {
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mut head = [0u8; 4];
    {
        let mut f = fs::File::open(archive)?;
        let n = f.read(&mut head)?;
        if n < 2 {
            return Err(EngineError::Archive("empty archive".into()));
        }
    }
    if head == *b"PK\x03\x04" || name.ends_with(".zip") {
        extract_zip(archive, dest)
    } else if head[..2] == [0x1f, 0x8b] || name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        extract_tar_gz(archive, dest)
    } else {
        Err(EngineError::Archive("unknown archive format".into()))
    }
}

pub fn extract_zip(archive: &Path, dest: &Path) -> Result<(), EngineError> {
    let file = fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(file))
        .map_err(|e| EngineError::Archive(e.to_string()))?;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| EngineError::Archive(e.to_string()))?;
        let raw_name = entry.name().to_string();
        let out = safe_join(dest, &raw_name)?;
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if entry.is_symlink() {
            // Not needed by any pinned archive; never follow archive-provided links.
            continue;
        }
        if entry.size() > MAX_ENTRY_BYTES {
            return Err(EngineError::Archive(format!("entry too large: {raw_name}")));
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = fs::File::create(&out)?;
        let copied = io::copy(&mut (&mut entry).take(MAX_ENTRY_BYTES + 1), &mut f)?;
        if copied > MAX_ENTRY_BYTES {
            return Err(EngineError::Archive(format!("entry too large: {raw_name}")));
        }
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o755))?;
        }
    }
    Ok(())
}

pub fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), EngineError> {
    let file = fs::File::open(archive)?;
    let gz = flate2::read::GzDecoder::new(io::BufReader::new(file));
    let mut tar = tar::Archive::new(gz);
    let entries = tar
        .entries()
        .map_err(|e| EngineError::Archive(e.to_string()))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| EngineError::Archive(e.to_string()))?;
        let raw_name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            fs::create_dir_all(safe_join(dest, &raw_name)?)?;
            continue;
        }
        if kind.is_symlink() {
            let out = safe_join(dest, &raw_name)?;
            let target = entry
                .link_name()
                .map_err(|e| EngineError::Archive(e.to_string()))?
                .map(|t| t.to_string_lossy().into_owned())
                .unwrap_or_default();
            // Only sibling links like `libggml.so -> libggml.so.0` are allowed.
            if target.is_empty() || target.contains(['/', '\\']) || target == ".." || target == "."
            {
                return Err(EngineError::UnsafeArchive(format!(
                    "{raw_name} -> {target}"
                )));
            }
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            #[cfg(unix)]
            {
                let _ = fs::remove_file(&out);
                std::os::unix::fs::symlink(&target, &out)?;
            }
            continue;
        }
        if kind.is_hard_link() {
            return Err(EngineError::UnsafeArchive(raw_name));
        }
        if !kind.is_file() {
            continue; // fifos, devices, pax headers…
        }
        let out = safe_join(dest, &raw_name)?;
        if entry.size() > MAX_ENTRY_BYTES {
            return Err(EngineError::Archive(format!("entry too large: {raw_name}")));
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        let mode = entry.header().mode().ok();
        let mut f = fs::File::create(&out)?;
        io::copy(&mut (&mut entry).take(MAX_ENTRY_BYTES), &mut f)?;
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o755))?;
        }
    }
    Ok(())
}

/// Find `binary` (or `binary.exe`) anywhere under `dir` (depth ≤ 4), shallowest first.
pub fn find_binary(dir: &Path, binary: &str) -> Option<PathBuf> {
    let wanted = exe_name(binary);
    let mut level = vec![dir.to_path_buf()];
    for _ in 0..5 {
        let mut next = Vec::new();
        for d in &level {
            let Ok(rd) = fs::read_dir(d) else { continue };
            let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            entries.sort();
            for p in entries {
                let is_link = fs::symlink_metadata(&p)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(true);
                if is_link {
                    continue;
                }
                if p.is_file()
                    && p.file_name()
                        .map(|n| n.to_string_lossy().eq_ignore_ascii_case(&wanted))
                        .unwrap_or(false)
                {
                    return Some(p);
                }
                if p.is_dir() {
                    next.push(p);
                }
            }
        }
        level = next;
    }
    None
}

#[cfg(unix)]
fn make_executable(exe: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = fs::metadata(exe)?.permissions();
    perm.set_mode(perm.mode() | 0o755);
    fs::set_permissions(exe, perm)
}

#[cfg(not(unix))]
fn make_executable(_exe: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::pins::BuildSpec;

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let f = fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (name, data) in entries {
            let opts = zip::write::SimpleFileOptions::default().unix_permissions(0o644);
            z.start_file(*name, opts).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    fn make_tgz(path: &Path, files: &[(&str, &[u8])], links: &[(&str, &str)]) {
        let f = fs::File::create(path).unwrap();
        let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
        let mut t = tar::Builder::new(gz);
        for (name, data) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o755);
            h.set_entry_type(tar::EntryType::Regular);
            // Write the raw name bytes so tests can inject `..` (the builder would refuse).
            let gnu = h.as_gnu_mut().unwrap();
            gnu.name[..name.len()].copy_from_slice(name.as_bytes());
            h.set_cksum();
            t.append(&h, *data).unwrap();
        }
        for (name, target) in links {
            let mut h = tar::Header::new_gnu();
            h.set_size(0);
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_link_name(target).unwrap();
            let gnu = h.as_gnu_mut().unwrap();
            gnu.name[..name.len()].copy_from_slice(name.as_bytes());
            h.set_cksum();
            t.append(&h, io::empty()).unwrap();
        }
        t.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn runtime_dlls_are_found_or_copied_from_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let (exe_dir, system, bundle) = (
            tmp.path().join("engine"),
            tmp.path().join("System32"),
            tmp.path().join("vcrt"),
        );
        for d in [&exe_dir, &system, &bundle] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(system.join("msvcp140.dll"), b"x").unwrap();
        fs::write(bundle.join("vcomp140.dll"), b"x").unwrap();
        let dlls = msvc_runtime_dlls(EngineKind::Sd);
        let missing = runtime_dlls_missing_after_copy(
            &exe_dir,
            dlls,
            Some(&system),
            std::slice::from_ref(&bundle),
        );
        assert!(
            exe_dir.join("vcomp140.dll").is_file(),
            "copied next to the engine"
        );
        assert!(!missing.contains(&"msvcp140.dll") && !missing.contains(&"vcomp140.dll"));
        assert!(missing.contains(&"vcruntime140.dll"));
        if !cfg!(windows) {
            assert!(provide_runtime_dlls(&exe_dir, EngineKind::Sd, &[]).is_empty());
        }
    }

    #[test]
    fn safe_relative_rejects_escapes() {
        assert_eq!(
            safe_relative("bin/sd-server"),
            Some(PathBuf::from("bin").join("sd-server"))
        );
        assert_eq!(
            safe_relative("./a\\b.dll"),
            Some(PathBuf::from("a").join("b.dll"))
        );
        for bad in [
            "../evil",
            "a/../../evil",
            "/etc/passwd",
            "\\\\server\\share\\x",
            "C:\\Windows\\x.dll",
            "C:evil",
            "",
            "a\0b",
        ] {
            assert_eq!(safe_relative(bad), None, "{bad}");
        }
    }

    #[test]
    fn zip_slip_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("evil.zip");
        make_zip(
            &zip_path,
            &[("ok.txt", b"ok"), ("../../escaped.txt", b"pwned")],
        );
        let dest = tmp.path().join("out");
        fs::create_dir_all(&dest).unwrap();
        let err = extract_zip(&zip_path, &dest).unwrap_err();
        assert!(matches!(err, EngineError::UnsafeArchive(_)), "{err}");
        assert!(!tmp.path().join("escaped.txt").exists());
        assert!(!tmp.path().parent().unwrap().join("escaped.txt").exists());
    }

    #[test]
    fn tar_slip_and_bad_symlinks_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("out");
        fs::create_dir_all(&dest).unwrap();

        let evil = tmp.path().join("evil.tar.gz");
        make_tgz(&evil, &[("../escaped", b"x")], &[]);
        assert!(matches!(
            extract_tar_gz(&evil, &dest),
            Err(EngineError::UnsafeArchive(_))
        ));
        assert!(!tmp.path().join("escaped").exists());

        let evil_link = tmp.path().join("link.tar.gz");
        make_tgz(&evil_link, &[], &[("llama-b1/libx.so", "/etc/passwd")]);
        assert!(matches!(
            extract_tar_gz(&evil_link, &dest),
            Err(EngineError::UnsafeArchive(_))
        ));
    }

    #[test]
    fn unpack_build_finds_binary_in_subfolder_and_writes_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("engine");
        let exe = exe_name("llama-server");
        let tgz = tmp.path().join("llama-b1-bin-ubuntu-x64.tar.gz");
        let exe_path = format!("llama-b1/{exe}");
        make_tgz(
            &tgz,
            &[
                (exe_path.as_str(), b"#!/bin/sh\n"),
                ("llama-b1/libllama.so.0.5.0", b"lib"),
            ],
            &[("llama-b1/libllama.so", "libllama.so.0.5.0")],
        );
        let pin = EnginePin {
            repo: "r".into(),
            version: "b1".into(),
            commit: None,
            binary: "llama-server".into(),
            launch_defaults: vec![],
            backend_override: Default::default(),
            builds: Default::default(),
        };
        let build = BuildSpec {
            url: "https://github.com/x/releases/download/b1/a.tar.gz".into(),
            sha256: "TODO".into(),
            size_bytes: None,
            size_mb: None,
            extra: vec![],
            note: None,
            min_glibc: None,
        };
        let sel = SelectedBuild {
            key: "linux_cpu".into(),
            backend: "cpu".into(),
            build: build.clone(),
        };
        let spec = build.archives()[0].clone();
        let got = unpack_build(
            &root,
            EngineKind::Llama,
            &pin,
            &sel,
            &[(spec.clone(), tgz.clone(), "abc".into())],
        )
        .unwrap();
        // The single top-level `llama-b1/` folder is flattened away.
        assert_eq!(got.exe, got.dir.join(&exe));
        assert_eq!(got.dir, install_dir(&root, EngineKind::Llama, "b1", "cpu"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(fs::metadata(&got.exe).unwrap().permissions().mode() & 0o111 != 0);
            assert!(fs::symlink_metadata(got.dir.join("libllama.so"))
                .unwrap()
                .file_type()
                .is_symlink());
        }
        assert_eq!(
            find_installed(&root, EngineKind::Llama, "b1", "cpu"),
            Some(got.clone())
        );
        assert_eq!(find_installed(&root, EngineKind::Llama, "b2", "cpu"), None);
        assert_eq!(
            find_any_installed(&root, EngineKind::Llama, "b1", &["vulkan".into()]),
            Some(got)
        );
        // No temp folders left behind.
        let leftovers: Vec<_> = fs::read_dir(root.join("llama").join("b1"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "{leftovers:?}");
    }

    #[test]
    fn unpack_build_checks_hash_and_merges_extras() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("engine");
        let exe = exe_name("sd-server");
        let main = tmp.path().join("sd-bin.zip");
        make_zip(&main, &[(exe.as_str(), b"bin"), ("ggml.dll", b"dll")]);
        let extra = tmp.path().join("cudart.zip");
        make_zip(&extra, &[("cudart64_12.dll", b"rt")]);
        let pin = EnginePin {
            repo: "r".into(),
            version: "master-1-abc".into(),
            commit: None,
            binary: "sd-server".into(),
            launch_defaults: vec![],
            backend_override: Default::default(),
            builds: Default::default(),
        };
        let good = "a".repeat(64);
        let build = BuildSpec {
            url: "https://github.com/x/releases/download/v/sd-bin.zip".into(),
            sha256: good.clone(),
            size_bytes: Some(3),
            size_mb: None,
            extra: vec![ArchiveSpec {
                url: "https://github.com/x/releases/download/v/cudart.zip".into(),
                sha256: "TODO".into(),
                size_bytes: None,
                size_mb: None,
            }],
            note: None,
            min_glibc: None,
        };
        let sel = SelectedBuild {
            key: "windows_cuda".into(),
            backend: "cuda".into(),
            build: build.clone(),
        };
        let a = build.archives();
        // Wrong hash → refused, nothing installed.
        let err = unpack_build(
            &root,
            EngineKind::Sd,
            &pin,
            &sel,
            &[
                (a[0].clone(), main.clone(), "b".repeat(64)),
                (a[1].clone(), extra.clone(), "c".repeat(64)),
            ],
        )
        .unwrap_err();
        assert!(matches!(err, EngineError::HashMismatch { .. }));
        assert!(find_installed(&root, EngineKind::Sd, "master-1-abc", "cuda").is_none());
        // Right hash → both archives land in one folder.
        let got = unpack_build(
            &root,
            EngineKind::Sd,
            &pin,
            &sel,
            &[
                (a[0].clone(), main, good),
                (a[1].clone(), extra, "c".repeat(64)),
            ],
        )
        .unwrap();
        assert!(got.dir.join("cudart64_12.dll").is_file());
        assert!(got.dir.join("ggml.dll").is_file());
        let marker: InstallMarker =
            serde_json::from_str(&fs::read_to_string(got.dir.join(MARKER_FILE)).unwrap()).unwrap();
        assert_eq!(marker.build, "windows_cuda");
        assert_eq!(marker.archives.len(), 2);
    }

    #[test]
    fn tar_extras_with_their_own_top_folder_land_next_to_the_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("engine");
        let exe = exe_name("llama-server");
        let main = tmp.path().join("llama-b9-bin-ubuntu-cuda-12.8-x64.tar.gz");
        let exe_path = format!("llama-b9/{exe}");
        make_tgz(
            &main,
            &[(exe_path.as_str(), b"bin"), ("llama-b9/LICENSE", b"mit")],
            &[],
        );
        let rt = tmp
            .path()
            .join("cudart-llama-b9-bin-ubuntu-cuda-12.8-x64.tar.gz");
        make_tgz(
            &rt,
            &[
                ("cudart-llama-b9/libcudart.so.12", b"rt"),
                ("cudart-llama-b9/libcublas.so.12", b"blas"),
            ],
            &[],
        );
        let pin = EnginePin {
            repo: "r".into(),
            version: "b9".into(),
            commit: None,
            binary: "llama-server".into(),
            launch_defaults: vec![],
            backend_override: Default::default(),
            builds: Default::default(),
        };
        let build = BuildSpec {
            url: "https://github.com/x/releases/download/b9/main.tar.gz".into(),
            sha256: "TODO".into(),
            size_bytes: None,
            size_mb: None,
            extra: vec![ArchiveSpec {
                url: "https://github.com/x/releases/download/b9/rt.tar.gz".into(),
                sha256: "TODO".into(),
                size_bytes: None,
                size_mb: None,
            }],
            note: None,
            min_glibc: None,
        };
        let sel = SelectedBuild {
            key: "linux_cuda".into(),
            backend: "cuda".into(),
            build: build.clone(),
        };
        let a = build.archives();
        let got = unpack_build(
            &root,
            EngineKind::Llama,
            &pin,
            &sel,
            &[
                (a[0].clone(), main, "x".into()),
                (a[1].clone(), rt, "y".into()),
            ],
        )
        .unwrap();
        assert_eq!(got.exe, got.dir.join(&exe));
        assert!(got.dir.join("libcudart.so.12").is_file());
        assert!(got.dir.join("libcublas.so.12").is_file());
        assert!(got.dir.join("LICENSE").is_file());
    }

    /// A small llama CPU build: (pin, selection, archive spec, archive path).
    fn llama_cpu_build(dir: &Path) -> (EnginePin, SelectedBuild, ArchiveSpec, PathBuf) {
        let exe_path = format!("llama-b1/{}", exe_name("llama-server"));
        let tgz = dir.join("llama-b1-bin-ubuntu-x64.tar.gz");
        make_tgz(&tgz, &[(exe_path.as_str(), b"#!/bin/sh\n")], &[]);
        let pin = EnginePin {
            repo: "r".into(),
            version: "b1".into(),
            commit: None,
            binary: "llama-server".into(),
            launch_defaults: vec![],
            backend_override: Default::default(),
            builds: Default::default(),
        };
        let build = BuildSpec {
            url: "https://github.com/x/releases/download/b1/a.tar.gz".into(),
            sha256: "TODO".into(),
            size_bytes: None,
            size_mb: None,
            extra: vec![],
            note: None,
            min_glibc: None,
        };
        let sel = SelectedBuild {
            key: "linux_cpu".into(),
            backend: "cpu".into(),
            build: build.clone(),
        };
        (pin, sel, build.archives()[0].clone(), tgz)
    }

    #[test]
    fn unpack_build_keeps_an_existing_install() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("engine");
        let (pin, sel, spec, tgz) = llama_cpu_build(tmp.path());
        let first = unpack_build(
            &root,
            EngineKind::Llama,
            &pin,
            &sel,
            &[(spec.clone(), tgz.clone(), "abc".into())],
        )
        .unwrap();
        fs::write(first.dir.join("kept.txt"), b"x").unwrap();
        // The archive is gone by now (removed after the first unpack).
        fs::remove_file(&tgz).unwrap();
        let again = unpack_build(
            &root,
            EngineKind::Llama,
            &pin,
            &sel,
            &[(spec, tgz, "abc".into())],
        )
        .unwrap();
        assert_eq!(again, first);
        assert!(first.dir.join("kept.txt").is_file());
        assert!(first.dir.join(MARKER_FILE).is_file());
    }

    #[test]
    fn unpack_build_removes_staging_when_the_final_move_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("engine");
        let (pin, sel, spec, tgz) = llama_cpu_build(tmp.path());
        // A file where the install folder goes: it can't be replaced by a folder.
        let final_dir = install_dir(&root, EngineKind::Llama, "b1", "cpu");
        fs::create_dir_all(final_dir.parent().unwrap()).unwrap();
        fs::write(&final_dir, b"x").unwrap();
        assert!(unpack_build(
            &root,
            EngineKind::Llama,
            &pin,
            &sel,
            &[(spec, tgz, "abc".into())],
        )
        .is_err());
        let names: Vec<_> = fs::read_dir(final_dir.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["cpu".to_string()]);
    }

    #[test]
    fn sweep_stale_removes_other_versions_staging_and_archives() {
        let cfg = EngineConfig::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../config/engine.yaml"),
        )
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("engine");
        let sd_pin = sanitize(&cfg.stable_diffusion_cpp.version);
        let llama_pin = sanitize(&cfg.llama_cpp.version);
        let write = |rel: &str| {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"x").unwrap();
            p
        };
        let own = std::process::id();
        // The pid of a process that has exited.
        let exited = {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("--list")
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let pid = child.id();
            child.wait().unwrap();
            pid
        };
        let first = |pin: &EnginePin| pin.builds.values().next().unwrap().archives()[0].file_name();
        let sd_archive = first(&cfg.stable_diffusion_cpp);
        let llama_archive = first(&cfg.llama_cpp);
        let removed = [
            write(&format!("sd/old-ver/cpu/{MARKER_FILE}")),
            write("llama/b1/cuda/llama-server"),
            write(&format!("sd/{sd_pin}/.cpu.tmp-{exited}/x")),
            write("downloads/sd-old-ver-a.zip"),
            write("downloads/llama-b1-a.tar.gz.part"),
            write(&format!("downloads/llama-{llama_pin}-foo-{llama_archive}")),
            write(&format!("downloads/sd-{sd_pin}-other.zip")),
        ];
        let mut kept = vec![
            write(&format!("sd/{sd_pin}/cpu/{MARKER_FILE}")),
            write(&format!("sd/{sd_pin}/.cuda.tmp-{own}/x")),
            write(&format!("llama/{llama_pin}/vulkan/{MARKER_FILE}")),
            write(&format!("downloads/sd-{sd_pin}-{sd_archive}")),
            write(&format!("downloads/llama-{llama_pin}-{llama_archive}.part")),
            write("notes.txt"),
        ];
        // Staging of another running process (here: the one that started the tests).
        #[cfg(unix)]
        kept.push(write(&format!(
            "llama/{llama_pin}/.cpu.tmp-{}/x",
            std::os::unix::process::parent_id()
        )));
        sweep_stale(&root, &cfg);
        for p in &removed {
            assert!(!p.exists(), "{}", p.display());
        }
        for p in &kept {
            assert!(p.exists(), "{}", p.display());
        }
        assert!(!root.join("sd").join("old-ver").exists());
        assert!(!root.join("llama").join("b1").exists());
        // A missing engine folder is fine.
        sweep_stale(&tmp.path().join("nothing"), &cfg);
    }

    #[test]
    fn download_specs_point_into_engine_downloads() {
        let cfg = EngineConfig::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../config/engine.yaml"),
        )
        .unwrap();
        let sel = cfg
            .select_build(&cfg.stable_diffusion_cpp, "windows", "cuda")
            .unwrap();
        let root = Path::new("/data/engine");
        let specs = download_specs(root, EngineKind::Sd, &cfg.stable_diffusion_cpp, &sel);
        assert_eq!(specs.len(), 2);
        assert!(specs.iter().all(|s| s.dest.starts_with(download_dir(root))));
        assert!(specs
            .iter()
            .all(|s| s.sha256.as_deref().map(|h| h.len() == 64).unwrap_or(false)));
        assert!(specs[1]
            .dest
            .to_string_lossy()
            .ends_with("cudart-sd-bin-win-cu12-x64.zip"));
        assert_eq!(specs[0].size_bytes, sel.build.size_bytes);
        assert!(specs[0].size_bytes.unwrap_or(0) > 100_000_000);
    }

    /// Full download → SHA-256 verify → unpack of the pinned CPU build through the
    /// real allow-listed client, then `sd-server --version`. Network: only when
    /// `PINHOLE_NET_INSTALL=1`.
    #[tokio::test]
    async fn install_direct_real_release_if_enabled() {
        if std::env::var("PINHOLE_NET_INSTALL").ok().as_deref() != Some("1") {
            return;
        }
        let cfg = EngineConfig::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../config/engine.yaml"),
        )
        .unwrap();
        let client = pinhole_net::HttpClient::new(pinhole_net::OfflineFlag::new(false)).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let got = install_direct(
            &client,
            tmp.path(),
            &cfg,
            EngineKind::Sd,
            crate::pins::current_os(),
            "cpu",
            &cancel,
        )
        .await
        .unwrap();
        assert!(got.exe.is_file());
        assert!(
            fs::read_dir(download_dir(tmp.path()))
                .unwrap()
                .next()
                .is_none(),
            "archives cleaned up"
        );
        let out = std::process::Command::new(&got.exe)
            .arg("--version")
            .current_dir(&got.dir)
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains(
            cfg.stable_diffusion_cpp
                .commit
                .as_deref()
                .unwrap_or("x")
                .get(..7)
                .unwrap()
        ));
        // Second call is a no-op (marker found).
        let again = install_direct(
            &client,
            tmp.path(),
            &cfg,
            EngineKind::Sd,
            crate::pins::current_os(),
            "cpu",
            &cancel,
        )
        .await
        .unwrap();
        assert_eq!(again, got);
    }

    /// Unpack a real upstream archive when `PINHOLE_ENGINE_ARCHIVE` points at one
    /// (e.g. sd-master-…-bin-Linux-Ubuntu-24.04-x86_64.zip); skipped otherwise.
    #[test]
    fn unpack_real_archive_if_provided() {
        let Ok(path) = std::env::var("PINHOLE_ENGINE_ARCHIVE") else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        extract_archive(Path::new(&path), tmp.path()).unwrap();
        assert!(
            find_binary(tmp.path(), "sd-server").is_some()
                || find_binary(tmp.path(), "llama-server").is_some()
        );
    }
}
