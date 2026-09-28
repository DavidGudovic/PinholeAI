//! Engine status / install (first run). OWNER: engine agent.
//!
//! The engine build is picked from `config/engine.yaml` for this OS and the
//! backend from hardware detection (NVIDIA → cuda, AMD/Intel → vulkan, none →
//! cpu) or the Settings override (`engineBackend`). Archives download through
//! the shared [`pinhole_net::download::DownloadManager`] (so the UI sees
//! `download-progress`), are SHA-256 verified, then unpacked into
//! `Data/engine/<sd|llama>/<version>/<backend>/`.

use std::path::PathBuf;
use std::sync::Arc;

use pinhole_engine::install::{self, EngineKind, InstalledEngine};
use pinhole_engine::pins::{self, EngineConfig, SelectedBuild};

use crate::events::EngineStatus;
use crate::{AppCore, CoreError, CoreEvent, CoreResult};

/// Parsed `engine.yaml` (cached after the first read).
pub fn engine_config(core: &AppCore) -> CoreResult<Arc<EngineConfig>> {
    if let Some(cfg) = core.gen.config.lock().clone() {
        return Ok(cfg);
    }
    let path = core.shipped.config_dir.join("engine.yaml");
    let cfg = EngineConfig::load(&path).map_err(|e| {
        CoreError::new("engine_missing", "Pinhole's engine list (engine.yaml) is missing or damaged. Reinstall Pinhole.").with_details(e.to_string())
    })?;
    let cfg = Arc::new(cfg);
    *core.gen.config.lock() = Some(cfg.clone());
    Ok(cfg)
}

/// Backend the user wants: Settings override if set, else hardware detection.
pub fn desired_backend(core: &AppCore) -> String {
    let setting = core.settings.read().engine_backend.clone();
    match setting.as_str() {
        "cuda" | "vulkan" | "cpu" => setting,
        _ => crate::app::hw_context(core).backend,
    }
}

/// Build for this machine (after fallbacks).
pub fn selected_build(core: &AppCore, kind: EngineKind) -> CoreResult<(Arc<EngineConfig>, SelectedBuild)> {
    let cfg = engine_config(core)?;
    let backend = desired_backend(core);
    let sel = cfg.select_build(kind.pin(&cfg), pins::current_os(), &backend).map_err(|_| {
        CoreError::new("engine_missing", "Pinhole's engine isn't available for this system (Windows 10/11 and Ubuntu 24.04+ are supported).")
    })?;
    Ok((cfg, sel))
}

/// `Data/engine/`
pub fn engine_root(core: &AppCore) -> PathBuf {
    core.data.engine()
}

/// The installed engine for the current backend choice; if that backend isn't
/// installed, any installed backend of the pinned version (better than
/// refusing to run — e.g. hardware detection still running, or the Settings
/// backend changed but the new engine isn't downloaded yet).
pub fn installed_engine(core: &AppCore, kind: EngineKind) -> Option<InstalledEngine> {
    let (cfg, sel) = selected_build(core, kind).ok()?;
    let version = &kind.pin(&cfg).version;
    let root = engine_root(core);
    install::find_installed(&root, kind, version, &sel.backend)
        .or_else(|| install::find_any_installed(&root, kind, version, &cfg.backend_candidates(kind.pin(&cfg), &sel.backend)))
}

/// Wait (bounded) for hardware detection so the backend choice is real, unless
/// the user picked a backend in Settings.
async fn settle_backend(core: &AppCore) {
    let explicit = matches!(core.settings.read().engine_backend.as_str(), "cuda" | "vulkan" | "cpu");
    if !explicit && core.hardware.read().is_none() {
        crate::app::wait_for_hardware(core, std::time::Duration::from_secs(30)).await;
    }
}

/// Current status (cheap: reads a marker file).
pub fn engine_status(core: &AppCore) -> EngineStatus {
    let flags = core.gen.flags.lock().clone();
    let external = core.gen.external.lock().is_some();
    let mut st = EngineStatus {
        installed: external,
        installing: flags.installing,
        version: None,
        backend: None,
        running: flags.running || external,
        loading: flags.loading,
        loaded_model_id: flags.loaded_model_id.clone(),
        error: flags.error.clone(),
    };
    match selected_build(core, EngineKind::Sd) {
        Ok((cfg, sel)) => {
            let pin = &cfg.stable_diffusion_cpp;
            st.version = Some(pin.version.clone());
            st.backend = Some(sel.backend.clone());
            if !external {
                let root = engine_root(core);
                let exact = install::find_installed(&root, EngineKind::Sd, &pin.version, &sel.backend);
                st.installed = exact.is_some();
                // Before hardware detection finishes the backend guess is "cpu";
                // report an already-installed GPU engine instead of "missing".
                if exact.is_none() && core.hardware.read().is_none() {
                    if let Some(any) = install::find_any_installed(&root, EngineKind::Sd, &pin.version, &[]) {
                        st.installed = true;
                        st.backend = Some(any.backend);
                    }
                }
            }
        }
        Err(e) if !external => {
            if st.error.is_none() {
                st.error = Some(e.message);
            }
        }
        Err(_) => {}
    }
    st
}

pub(crate) fn emit_status(core: &AppCore) {
    core.emit(CoreEvent::Engine(engine_status(core)));
}

/// Download + verify + unpack the image engine for the current backend.
/// Resolves when done; progress arrives as `download-progress`.
pub async fn install_engine(core: &Arc<AppCore>) -> CoreResult<EngineStatus> {
    let result = install_kind(core, EngineKind::Sd).await;
    match result {
        Ok(_) => {
            core.gen.flags.lock().error = None;
            emit_status(core);
            Ok(engine_status(core))
        }
        Err(e) => {
            if e.code != "cancelled" {
                core.gen.flags.lock().error = Some(e.message.clone());
            }
            emit_status(core);
            Err(e)
        }
    }
}

/// Install (if needed) the engine of `kind` for the current backend.
pub(crate) async fn install_kind(core: &Arc<AppCore>, kind: EngineKind) -> CoreResult<InstalledEngine> {
    settle_backend(core).await;
    let lock = match kind {
        EngineKind::Sd => &core.gen.install_lock,
        EngineKind::Llama => &core.describe.install_lock,
    };
    let _guard = lock.lock().await;
    let (cfg, sel) = selected_build(core, kind)?;
    let pin = kind.pin(&cfg).clone();
    let root = engine_root(core);
    if let Some(done) = install::find_installed(&root, kind, &pin.version, &sel.backend) {
        return Ok(done);
    }
    check_glibc(&sel)?;
    let set_installing = |v: bool| {
        if kind == EngineKind::Sd {
            core.gen.flags.lock().installing = v;
            emit_status(core);
        }
    };
    set_installing(true);
    let res = download_and_unpack(core, kind, &cfg, &sel).await;
    set_installing(false);
    res
}

/// Engine archives still to download, with the config + build they belong to.
pub(crate) type EngineDownload = (Vec<pinhole_net::download::DownloadSpec>, Arc<EngineConfig>, SelectedBuild);

pub(crate) fn engine_download_specs(core: &AppCore, kind: EngineKind) -> CoreResult<Option<EngineDownload>> {
    let (cfg, sel) = selected_build(core, kind)?;
    let pin = kind.pin(&cfg);
    let root = engine_root(core);
    if install::find_installed(&root, kind, &pin.version, &sel.backend).is_some() {
        return Ok(None);
    }
    check_glibc(&sel)?;
    std::fs::create_dir_all(install::download_dir(&root))?;
    let specs = install::download_specs(&root, kind, pin, &sel);
    Ok(Some((specs, cfg, sel)))
}

/// Unpack already-downloaded engine archives (paths in build order).
pub(crate) async fn unpack_downloaded(
    core: &AppCore,
    kind: EngineKind,
    cfg: &EngineConfig,
    sel: &SelectedBuild,
    files: Vec<pinhole_net::download::DownloadedFile>,
) -> CoreResult<InstalledEngine> {
    let root = engine_root(core);
    let archives = sel.build.archives();
    if files.len() != archives.len() {
        return Err(CoreError::internal("The engine download is incomplete. Try again."));
    }
    // Unpacked size is roughly 1.5× the archives; check before writing.
    let need = files.iter().map(|f| f.size_bytes).sum::<u64>().saturating_mul(3) / 2;
    std::fs::create_dir_all(&root)?;
    if let Err(e) = pinhole_net::download::check_free_space(&root, need) {
        return Err(CoreError::new(e.code(), e.user_message("")));
    }
    let downloaded: Vec<_> = archives.into_iter().zip(files.iter()).map(|(a, f)| (a, f.path.clone(), f.sha256.clone())).collect();
    let paths: Vec<PathBuf> = files.iter().map(|f| f.path.clone()).collect();
    let pin = kind.pin(cfg).clone();
    let sel2 = sel.clone();
    let root2 = root.clone();
    let res = tokio::task::spawn_blocking(move || install::unpack_build(&root2, kind, &pin, &sel2, &downloaded))
        .await
        .map_err(|e| CoreError::internal("Unpacking the engine failed.").with_details(e.to_string()))?;
    let installed = res.map_err(|e| match e {
        pinhole_engine::EngineError::HashMismatch { .. } => CoreError::new("hash_mismatch", "The engine download was corrupted (checksum mismatch). Try again.").with_details(e.to_string()),
        pinhole_engine::EngineError::Io(io) => CoreError::new("io", "Couldn't unpack the engine into the Data folder. Check free disk space and try again.").with_details(io.to_string()),
        other => CoreError::new("engine_failed", "The engine download couldn't be unpacked. Try again.").with_details(other.to_string()),
    })?;
    install::cleanup_downloads(&paths);
    Ok(installed)
}

async fn download_and_unpack(core: &Arc<AppCore>, kind: EngineKind, cfg: &Arc<EngineConfig>, sel: &SelectedBuild) -> CoreResult<InstalledEngine> {
    let Some((specs, _, _)) = engine_download_specs(core, kind)? else {
        let pin = kind.pin(cfg);
        return install::find_installed(&engine_root(core), kind, &pin.version, &sel.backend)
            .ok_or_else(|| CoreError::internal("The engine install vanished. Try again."));
    };
    let label = match kind {
        EngineKind::Sd => format!("Image engine ({})", backend_label(&sel.backend)),
        EngineKind::Llama => format!("Describe engine ({})", backend_label(&sel.backend)),
    };
    let group = core.downloads.enqueue(label, specs);
    let files = core.downloads.wait_detailed(&group).await.map_err(|e| CoreError::new(&e.code, e.message))?;
    unpack_downloaded(core, kind, cfg, sel, files).await
}

pub(crate) fn backend_label(backend: &str) -> &'static str {
    match backend {
        "cuda" => "NVIDIA CUDA",
        "vulkan" => "Vulkan",
        _ => "CPU",
    }
}

/// Linux: refuse early when the engine needs a newer glibc than this system has.
fn check_glibc(sel: &SelectedBuild) -> CoreResult<()> {
    if let (Some(need), Some(have)) = (sel.build.min_glibc.as_deref(), pins::glibc_version()) {
        if !pins::version_at_least(&have, need) {
            return Err(CoreError::new(
                "engine_failed",
                format!("Pinhole's image engine needs Ubuntu 24.04 or newer (glibc {need}; this system has {have}). Please update your system."),
            ));
        }
    }
    Ok(())
}
