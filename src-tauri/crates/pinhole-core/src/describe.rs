//! Describe (img2text) via llama-server. OWNER: engine agent.
//!
//! Captioner files: reuse the edit model's Qwen2.5-VL-7B + mmproj when both are
//! installed (`captioner.prefer_reuse`), else the small default captioner
//! (`captioner.default`, component ids [`DEFAULT_MODEL_ID`] /
//! [`DEFAULT_MMPROJ_ID`], kind `Captioner`). llama-server starts on demand on
//! 127.0.0.1 and is stopped after `captioner.idle_shutdown_seconds` (60 s) idle.
//! The only text sent is the fixed registry instruction (`captioner.prompts`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use pinhole_engine::failure::{classify, Failure};
use pinhole_engine::install::EngineKind;
use pinhole_engine::llama::{self, LlamaClient};
use pinhole_engine::logbuf::LogBuffer;
use pinhole_engine::process::{free_port, EngineProcess, ReadyError};
use pinhole_net::download::DownloadSpec;
use pinhole_store::datadir::ModelKind;
use serde::{Deserialize, Serialize};

use crate::models::{register_download, Registration};
use crate::{engine_setup, AppCore, CoreError, CoreResult, InstallStarted};

/// Component id of the default captioner model file in installed.json.
pub const DEFAULT_MODEL_ID: &str = "captioner_default_model";
/// Component id of the default captioner vision projector (mmproj).
pub const DEFAULT_MMPROJ_ID: &str = "captioner_default_mmproj";
const LOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const CTX_SIZE: u32 = 8192;
/// Images are shrunk to at most this many pixels per side before captioning.
const MAX_SIDE: u32 = 2048;

/// `CaptionerStatus` in src/lib/types.ts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaptionerStatus {
    /// Ready to describe without downloading anything.
    pub available: bool,
    /// `reuse` | `default` | null
    pub source: Option<String>,
    /// Bytes to download to make the captioner available.
    pub download_bytes: u64,
    pub running: bool,
}

/// `DescribeStyle`: `sentence` | `tags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DescribeStyle {
    Sentence,
    Tags,
}

impl DescribeStyle {
    pub fn key(self) -> &'static str {
        match self {
            DescribeStyle::Sentence => "sentence",
            DescribeStyle::Tags => "tags",
        }
    }
}

pub(crate) struct LlamaSlot {
    proc: EngineProcess,
    model: PathBuf,
    mmproj: PathBuf,
    /// Per-launch API key (memory only; handed to llama-server via its environment).
    api_key: String,
}

/// Captioner (llama-server) state.
pub struct DescribeState {
    pub(crate) slot: tokio::sync::Mutex<Option<LlamaSlot>>,
    pub(crate) install_lock: tokio::sync::Mutex<()>,
    pub(crate) last_used: parking_lot::Mutex<Instant>,
    pub(crate) busy: AtomicBool,
    pub(crate) logs: Arc<LogBuffer>,
    /// Last background install failure (unpack), shown on the next describe.
    pub(crate) last_error: parking_lot::Mutex<Option<String>>,
    /// Tests: an already-running (mock) llama-server.
    pub(crate) external: parking_lot::Mutex<Option<String>>,
}

impl Default for DescribeState {
    fn default() -> Self {
        Self {
            slot: tokio::sync::Mutex::new(None),
            install_lock: tokio::sync::Mutex::new(()),
            last_used: parking_lot::Mutex::new(Instant::now()),
            busy: AtomicBool::new(false),
            logs: Arc::new(LogBuffer::default()),
            last_error: parking_lot::Mutex::new(None),
            external: parking_lot::Mutex::new(None),
        }
    }
}

// ---------------------------------------------------------------- files

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Reuse,
    Default,
}

impl Source {
    fn key(self) -> &'static str {
        match self {
            Source::Reuse => "reuse",
            Source::Default => "default",
        }
    }
}

/// Installed captioner files (model, mmproj), reuse preferred.
fn captioner_files(core: &AppCore) -> Option<(Source, PathBuf, PathBuf)> {
    let reg = core.registry();
    let idx = core.installed.lock();
    let path_of = |component: &str| -> Option<PathBuf> {
        let f = idx.find_component(component)?;
        let p = idx.abs_path(&core.data, f);
        p.is_file().then_some(p)
    };
    let reuse = &reg.captioner().prefer_reuse;
    if reuse.len() >= 2 {
        if let (Some(m), Some(p)) = (path_of(&reuse[0]), path_of(&reuse[1])) {
            return Some((Source::Reuse, m, p));
        }
    }
    match (path_of(DEFAULT_MODEL_ID), path_of(DEFAULT_MMPROJ_ID)) {
        (Some(m), Some(p)) => Some((Source::Default, m, p)),
        _ => None,
    }
}

fn verified(sha: &str) -> Option<String> {
    let s = sha.trim().to_ascii_lowercase();
    (s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())).then_some(s)
}

/// What `install_captioner` would download: (spec, role).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Model,
    Mmproj,
    Engine,
}

fn missing_parts(core: &AppCore) -> CoreResult<Vec<(DownloadSpec, Part)>> {
    let mut out = Vec::new();
    if captioner_files(core).is_none() {
        let reg = core.registry();
        let def = reg
            .captioner()
            .default
            .clone()
            .ok_or_else(|| CoreError::not_found("No describe model is listed in Pinhole's model list. Update Pinhole."))?;
        let dir = core.data.models(ModelKind::Captioner);
        let idx = core.installed.lock();
        for (file, part, label) in [(&def.model, Part::Model, "Describe model"), (&def.mmproj, Part::Mmproj, "Describe model (vision)")] {
            let comp = if part == Part::Model { DEFAULT_MODEL_ID } else { DEFAULT_MMPROJ_ID };
            if idx.find_component(comp).map(|f| idx.abs_path(&core.data, f).is_file()).unwrap_or(false) {
                continue;
            }
            out.push((
                DownloadSpec {
                    url: file.url.clone(),
                    dest: dir.join(&file.file),
                    sha256: verified(&file.sha256),
                    // `size_mb` is rounded: an estimate only, never the exact size.
                    size_bytes: None,
                    approx_size_bytes: Some(file.size_mb * 1_000_000),
                    label: label.into(),
                    ..Default::default()
                },
                part,
            ));
        }
    }
    if let Some((specs, _, _)) = engine_setup::engine_download_specs(core, EngineKind::Llama)? {
        out.extend(specs.into_iter().map(|s| (s, Part::Engine)));
    }
    Ok(out)
}

// ---------------------------------------------------------------- status / install

pub fn captioner_status(core: &AppCore) -> CaptionerStatus {
    let external = core.describe.external.lock().is_some();
    let files = captioner_files(core);
    let running = match core.describe.slot.try_lock() {
        Ok(mut g) => g.as_mut().map(|s| s.proc.is_running()).unwrap_or(false),
        Err(_) => true, // busy starting / describing
    } || external;
    if external {
        return CaptionerStatus { available: true, source: Some("default".into()), download_bytes: 0, running };
    }
    let download_bytes = missing_parts(core).map(|p| p.iter().filter_map(|(s, _)| s.size_hint()).sum()).unwrap_or(0);
    let engine_ok = engine_setup::installed_engine(core, EngineKind::Llama).is_some();
    CaptionerStatus { available: files.is_some() && engine_ok, source: files.map(|f| f.0.key().to_string()), download_bytes, running }
}

/// Queue the default captioner (+ the llama.cpp engine if missing) as one
/// download group; files are registered / unpacked when it finishes.
pub async fn install_captioner(core: &Arc<AppCore>) -> CoreResult<InstallStarted> {
    let explicit = matches!(core.settings.read().engine_backend.as_str(), "cuda" | "vulkan" | "cpu");
    if !explicit && core.hardware.read().is_none() {
        crate::app::wait_for_hardware(core, Duration::from_secs(30)).await;
    }
    let parts = missing_parts(core)?;
    let (specs, roles): (Vec<DownloadSpec>, Vec<Part>) = parts.into_iter().unzip();
    std::fs::create_dir_all(core.data.models(ModelKind::Captioner))?;
    let group_id = core.downloads.enqueue_kind("Describe model".into(), pinhole_net::download::DownloadKind::Captioner, specs);
    *core.describe.last_error.lock() = None;
    let core2 = core.clone();
    let gid = group_id.clone();
    tokio::spawn(async move {
        let _guard = core2.describe.install_lock.lock().await;
        let Ok(files) = core2.downloads.wait_detailed(&gid).await else { return };
        let mut engine_files = Vec::new();
        for (file, role) in files.into_iter().zip(roles) {
            let reg = match role {
                Part::Engine => {
                    engine_files.push(file);
                    continue;
                }
                Part::Model => Registration {
                    kind: ModelKind::Captioner,
                    friendly_name: "Describe model".into(),
                    family: None,
                    component_id: Some(DEFAULT_MODEL_ID.into()),
                    civitai: None,
                    dtype: None,
                },
                Part::Mmproj => Registration {
                    kind: ModelKind::Captioner,
                    friendly_name: "Describe model (vision)".into(),
                    family: None,
                    component_id: Some(DEFAULT_MMPROJ_ID.into()),
                    civitai: None,
                    dtype: None,
                },
            };
            if let Err(e) = register_download(&core2, &file, reg) {
                *core2.describe.last_error.lock() = Some(e.message);
            }
        }
        if !engine_files.is_empty() {
            let res = match engine_setup::selected_build(&core2, EngineKind::Llama) {
                Ok((cfg, sel)) => engine_setup::unpack_downloaded(&core2, EngineKind::Llama, &cfg, &sel, engine_files).await.map(|_| ()),
                Err(e) => Err(e),
            };
            if let Err(e) = res {
                *core2.describe.last_error.lock() = Some(e.message);
            }
        }
        core2.emit(crate::CoreEvent::ModelsChanged);
    });
    Ok(InstallStarted { group_id })
}

// ---------------------------------------------------------------- describe

/// Describe a session image as a prompt (`sentence`) or booru tags (`tags`).
pub async fn describe_image(core: &Arc<AppCore>, image_id: &str, style: DescribeStyle) -> CoreResult<String> {
    let img = core.session.get(image_id).ok_or_else(|| CoreError::not_found("That image isn't in this session anymore. Add it again."))?;
    let reg = core.registry();
    let instruction = reg
        .captioner()
        .prompts
        .get(style.key())
        .cloned()
        .ok_or_else(|| CoreError::not_found("This describe style isn't available. Update Pinhole."))?;

    // Shrink very large images (the VLM downsamples anyway).
    let (bytes, mime): (Vec<u8>, &str) = if img.width.max(img.height) > MAX_SIDE {
        let src = img.bytes.clone();
        let png = tokio::task::spawn_blocking(move || -> CoreResult<Vec<u8>> {
            let (mut rgba, mut w, mut h) = pinhole_engine::image::decode_rgba(&src).map_err(|e| CoreError::invalid("This image couldn't be decoded.").with_details(e.to_string()))?;
            while w.max(h) > MAX_SIDE {
                (rgba, w, h) = pinhole_engine::image::downscale_2x_box(&rgba, w, h);
            }
            pinhole_engine::image::encode_png_rgba(&rgba, w, h).map_err(|e| CoreError::internal("Couldn't prepare the image.").with_details(e.to_string()))
        })
        .await
        .map_err(|e| CoreError::internal("Couldn't prepare the image.").with_details(e.to_string()))??;
        (png, "image/png")
    } else {
        (img.bytes.as_ref().clone(), img.kind.mime())
    };

    core.describe.busy.store(true, Ordering::SeqCst);
    let res = describe_inner(core, &instruction, mime, &bytes, style).await;
    *core.describe.last_used.lock() = Instant::now();
    core.describe.busy.store(false, Ordering::SeqCst);
    res
}

async fn describe_inner(core: &Arc<AppCore>, instruction: &str, mime: &str, bytes: &[u8], style: DescribeStyle) -> CoreResult<String> {
    let client = ensure_llama(core).await?;
    let max_tokens = if style == DescribeStyle::Tags { 200 } else { 300 };
    let text = client.describe(instruction, mime, bytes, max_tokens).await.map_err(|e| {
        let tail = core.describe.logs.tail_text(30);
        match classify(&tail, None) {
            Failure::OutOfMemory => CoreError::new("vram", "Not enough memory to describe right now — close other apps or wait for the image to finish, then try again.").with_details(tail),
            _ => CoreError::new("engine_failed", "Describing the image failed. Try again.").with_details(format!("{e}\n{tail}")),
        }
    })?;
    if text.is_empty() {
        return Err(CoreError::new("engine_failed", "The describe model returned nothing. Try again."));
    }
    Ok(text)
}

/// 32 random bytes as hex: llama-server's API key for one launch.
fn new_api_key() -> String {
    rand::random::<[u8; 32]>().iter().map(|b| format!("{b:02x}")).collect()
}

/// After `wait_ready`: our child must still be running and the server on the
/// port must report the model we launched it with (`GET /v1/models`, which also
/// needs our API key) — otherwise another program holds the port.
pub(crate) async fn verify_llama_identity(proc: &mut EngineProcess, client: &LlamaClient, model: &std::path::Path) -> CoreResult<()> {
    let taken = || CoreError::new("engine_failed", crate::generate::PORT_TAKEN_MESSAGE);
    if !proc.is_running() {
        return Err(taken().with_details("the describe engine exited while another program answered on its port"));
    }
    let ids = client.model_ids().await.map_err(|e| taken().with_details(format!("model check failed: {e}")))?;
    let expected = model.to_string_lossy();
    if !ids.iter().any(|id| crate::generate::same_file_path(id, &expected)) {
        return Err(taken().with_details("the server on the describe engine's port reports a different model"));
    }
    if !proc.is_running() {
        return Err(taken().with_details("the describe engine exited"));
    }
    Ok(())
}

/// Start llama-server if needed; returns a client for it (with its API key).
async fn ensure_llama(core: &Arc<AppCore>) -> CoreResult<LlamaClient> {
    if let Some(url) = core.describe.external.lock().clone() {
        return Ok(LlamaClient::new(core.local.clone(), url));
    }
    if let Some(msg) = core.describe.last_error.lock().clone() {
        return Err(CoreError::new("engine_failed", msg));
    }
    let (_, model, mmproj) = captioner_files(core).ok_or_else(|| CoreError::not_found("The describe model isn't installed yet. Click Get on the Describe tab."))?;
    let engine = engine_setup::installed_engine(core, EngineKind::Llama)
        .ok_or_else(|| CoreError::not_found("The describe engine isn't installed yet. Click Get on the Describe tab."))?;
    let mut slot = core.describe.slot.lock().await;
    if let Some(s) = slot.as_mut() {
        // Same files AND the same engine build (the backend may have changed in Settings).
        if s.proc.is_running() && s.model == model && s.mmproj == mmproj && s.proc.exe() == engine.exe {
            return Ok(LlamaClient::new(core.local.clone(), s.proc.base_url()).with_api_key(s.api_key.clone()));
        }
    }
    if let Some(old) = slot.take() {
        old.proc.stop().await;
    }
    engine_setup::ensure_runtime(core, &engine)?;
    engine_setup::sweep_orphans(core).await;
    let cfg = engine_setup::engine_config(core)?;
    let port = free_port().map_err(|e| CoreError::internal("Couldn't find a free local port.").with_details(e.to_string()))?;
    let mut args: Vec<String> = cfg.llama_cpp.launch_defaults.iter().filter(|a| *a != "--host" && *a != "127.0.0.1").cloned().collect();
    args.extend(llama::launch_args(&model, &mmproj, port, &engine.backend, CTX_SIZE));
    core.describe.logs.clear();
    let api_key = new_api_key();
    let mut proc = EngineProcess::spawn_with_env(&engine.exe, &args, &[(llama::API_KEY_ENV, api_key.as_str())], port, core.describe.logs.clone())
        .map_err(|e| CoreError::new("engine_failed", "The describe engine couldn't be started.").with_details(e.to_string()))?;
    let client = LlamaClient::new(core.local.clone(), proc.base_url()).with_api_key(api_key.clone());
    let cancel = tokio_util::sync::CancellationToken::new();
    match proc.wait_ready(|| client.is_ready(), LOAD_TIMEOUT, &cancel, |_| {}).await {
        Ok(()) => {
            if let Err(e) = verify_llama_identity(&mut proc, &client, &model).await {
                proc.kill().await;
                return Err(e);
            }
            *slot = Some(LlamaSlot { proc, model, mmproj, api_key });
            *core.describe.last_used.lock() = Instant::now();
            Ok(client)
        }
        Err(e) => {
            let code = proc.exit_code();
            proc.stop().await;
            let tail = core.describe.logs.tail_text(30);
            Err(match (e, classify(&tail, code)) {
                (_, Failure::OutOfMemory) => CoreError::new("vram", "Not enough memory to load the describe model. Close other apps and try again.").with_details(tail),
                (_, Failure::ModelLoad) => CoreError::new("engine_failed", "The describe model couldn't be loaded — the file may be damaged. Download it again.").with_details(tail),
                (ReadyError::Timeout, _) => CoreError::new("engine_failed", "The describe model took too long to load. Try again.").with_details(tail),
                _ => CoreError::new("engine_failed", "The describe engine stopped unexpectedly. Try again.").with_details(tail),
            })
        }
    }
}

// ---------------------------------------------------------------- lifecycle

/// Stop llama-server after `captioner.idle_shutdown_seconds` without use.
pub fn start_idle_watchdog(core: &Arc<AppCore>) {
    let weak: Weak<AppCore> = Arc::downgrade(core);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let Some(core) = weak.upgrade() else { break };
            let idle = match core.registry().captioner().idle_shutdown_seconds {
                0 => 60,
                s => s,
            };
            if core.describe.busy.load(Ordering::SeqCst) || core.describe.last_used.lock().elapsed() < Duration::from_secs(idle) {
                continue;
            }
            let Ok(mut slot) = core.describe.slot.try_lock() else { continue };
            if let Some(s) = slot.take() {
                s.proc.stop().await;
            }
        }
    });
}

/// Stop llama-server unless it is describing right now (it holds graphics
/// memory the image engine is about to need). Returns whether it was stopped.
pub(crate) async fn stop_if_idle(core: &AppCore) -> bool {
    if core.describe.busy.load(Ordering::SeqCst) {
        return false;
    }
    let Ok(mut slot) = core.describe.slot.try_lock() else { return false };
    match slot.take() {
        Some(s) => {
            s.proc.stop().await;
            true
        }
        None => false,
    }
}

pub async fn shutdown(core: &AppCore) {
    let mut slot = core.describe.slot.lock().await;
    if let Some(s) = slot.take() {
        s.proc.stop().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_keys_are_random_per_launch() {
        let (a, b) = (new_api_key(), new_api_key());
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn describe_style_parses_like_types_ts() {
        let s: DescribeStyle = serde_json::from_str("\"tags\"").unwrap();
        assert_eq!(s, DescribeStyle::Tags);
        assert_eq!(DescribeStyle::Sentence.key(), "sentence");
        let st = CaptionerStatus { available: false, source: None, download_bytes: 5, running: false };
        let v = serde_json::to_value(&st).unwrap();
        assert_eq!(v["downloadBytes"], 5);
        assert!(v["source"].is_null());
    }
}
