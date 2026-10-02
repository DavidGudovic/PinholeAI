//! The sd-server process: launch with the model's arguments, reuse while they
//! stay the same, stop when idle, on unload and at exit; progress and failure
//! messages from its output.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pinhole_engine::failure::{classify, Failure};
use pinhole_engine::install::EngineKind;
use pinhole_engine::logbuf::{LogBuffer, ProgressKind};
use pinhole_engine::pins::EngineConfig;
use pinhole_engine::process::{free_port, EngineProcess, ReadyError};
use pinhole_engine::sdapi::{self, SdClient};
use pinhole_hardware::OtherGpuUse;
use pinhole_store::datadir::ModelKind;
use tokio_util::sync::CancellationToken;

use crate::engine_setup;
use crate::events::{GenPhase, GenerationProgress};
use crate::memory::{
    note_offload, others_note, vram_message, MemFallback, OTHERS_PREFIX, RAM_MESSAGE, RETRY_NOTES,
    VRAM_MESSAGE,
};
use crate::{AppCore, CoreError, CoreEvent, CoreResult};

/// How long a model may take to load before we give up (huge models on slow disks).
const LOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// sd-server has no authentication and keeps every finished job (base64 images
/// included) at `GET /sdcpp/v1/jobs/{id}` for 600 s. So once a job ran on a
/// Pinhole-started engine, the engine is stopped this long after the last
/// generate / upscale (and on Reset); the next Generate reloads it.
pub const IDLE_STOP_AFTER: Duration = Duration::from_secs(5 * 60);

/// Another process answered on the port we started an engine on.
pub const PORT_TAKEN_MESSAGE: &str = "Another program is using Pinhole's engine port — try again.";

#[derive(Debug, Clone, Default)]
pub(crate) struct EngineFlags {
    pub installing: bool,
    pub loading: bool,
    pub running: bool,
    pub loaded_model_id: Option<String>,
    /// Last engine failure, shown by the top bar's "Engine problem" popover.
    pub error: Option<CoreError>,
}

#[derive(Default)]
pub(crate) struct EngineSlot {
    pub proc: Option<EngineProcess>,
    pub args: Vec<String>,
    pub model_id: Option<String>,
    /// An img_gen job was submitted to this process, so it may hold finished
    /// results (see [`IDLE_STOP_AFTER`]).
    pub results_cached: bool,
    /// This process's per-launch API key ([`sdapi::API_KEY_ENV`]).
    pub api_key: Option<String>,
}

/// sd-server process + current generation job.
pub struct GenState {
    pub(crate) slot: tokio::sync::Mutex<EngineSlot>,
    /// One generation (or upscale) at a time.
    pub(crate) run_lock: tokio::sync::Mutex<()>,
    pub(crate) install_lock: tokio::sync::Mutex<()>,
    /// Cancels the running generate / model load.
    pub(crate) active: parking_lot::Mutex<Option<CancellationToken>>,
    pub(crate) flags: parking_lot::Mutex<EngineFlags>,
    /// sd-server stdout/stderr (memory only, redacted).
    pub(crate) logs: Arc<LogBuffer>,
    /// Tests: talk to an already-running (mock) server instead of spawning one.
    pub(crate) external: parking_lot::Mutex<Option<String>>,
    pub(crate) config: parking_lot::Mutex<Option<Arc<EngineConfig>>>,
    /// Bumped when a generate / upscale starts; an armed idle stop only fires
    /// when nothing started since.
    pub(crate) activity: AtomicU64,
    /// [`IDLE_STOP_AFTER`] (tests shorten it).
    pub(crate) idle_stop_after: parking_lot::Mutex<Duration>,
    /// Reset happened while a job was running: stop the engine after it.
    pub(crate) clear_pending: AtomicBool,
    /// Automatic memory fallbacks per model id (RAM only, app session).
    pub(crate) mem_fallback: parking_lot::Mutex<HashMap<String, MemFallback>>,
    /// Notes shown with the current job's progress (plain language, no prompt).
    pub(crate) job_note: parking_lot::Mutex<Vec<String>>,
    /// Graphics memory used by other programs when the running engine started (NVIDIA).
    pub(crate) gpu_others: parking_lot::Mutex<Option<OtherGpuUse>>,
    /// Model id + the memory plan its last auto-fit launch printed (where the
    /// weights went; no prompt text). Kept across a retry whose engine doesn't
    /// run auto-fit, and shown in the details of an out-of-memory error.
    pub(crate) memory_plan: parking_lot::Mutex<Option<(String, Vec<String>)>>,
    /// Model id + wiring args of the last launch when it kept the weights in
    /// system memory (`--offload-to-cpu`); `None` after any other launch or an
    /// unload. The same model with the same settings keeps them there, also
    /// after the idle stop, so it doesn't run out of memory on the card again.
    pub(crate) offloaded: parking_lot::Mutex<Option<(String, Vec<String>)>>,
    /// Set at each launch: the note when a GPU-backend engine isn't using the
    /// graphics card (see `pinhole_engine::failure::missed_gpu`).
    pub(crate) not_on_gpu: parking_lot::Mutex<Option<String>>,
    /// Tests: the launch args each run asked an external engine for.
    pub(crate) external_launches: parking_lot::Mutex<Vec<Vec<String>>>,
    /// Model id + launch args while an engine loads (it holds `slot` meanwhile).
    pub(crate) loading: parking_lot::Mutex<Option<(String, Vec<String>)>>,
}

impl Default for GenState {
    fn default() -> Self {
        Self {
            slot: tokio::sync::Mutex::new(EngineSlot::default()),
            run_lock: tokio::sync::Mutex::new(()),
            install_lock: tokio::sync::Mutex::new(()),
            active: parking_lot::Mutex::new(None),
            flags: parking_lot::Mutex::new(EngineFlags::default()),
            logs: Arc::new(LogBuffer::default()),
            external: parking_lot::Mutex::new(None),
            config: parking_lot::Mutex::new(None),
            activity: AtomicU64::new(0),
            idle_stop_after: parking_lot::Mutex::new(IDLE_STOP_AFTER),
            clear_pending: AtomicBool::new(false),
            mem_fallback: parking_lot::Mutex::new(HashMap::new()),
            job_note: parking_lot::Mutex::new(Vec::new()),
            gpu_others: parking_lot::Mutex::new(None),
            memory_plan: parking_lot::Mutex::new(None),
            offloaded: parking_lot::Mutex::new(None),
            not_on_gpu: parking_lot::Mutex::new(None),
            external_launches: parking_lot::Mutex::new(Vec::new()),
            loading: parking_lot::Mutex::new(None),
        }
    }
}

/// Stop the engine (app exit). Cancels any running job first.
pub async fn shutdown(core: &AppCore) {
    if let Some(tok) = core.gen.active.lock().take() {
        tok.cancel();
    }
    let mut slot = core.gen.slot.lock().await;
    if let Some(p) = slot.proc.take() {
        p.stop().await;
    }
    *slot = EngineSlot::default();
    let mut f = core.gen.flags.lock();
    f.running = false;
    f.loading = false;
    f.loaded_model_id = None;
}

/// Stop sd-server before deleting a model's files: when it has `model_id`
/// loaded, has one of the files open (a shared component), or the file is a
/// LoRA or upscaler (the engine reads those from their folders per job), because
/// with `--mmap` it keeps the files it read mapped, and Windows can't delete a
/// mapped file. The next Generate loads the model again.
pub async fn unload_model(core: &AppCore, model_id: &str, files: &[PathBuf]) {
    {
        let mut off = core.gen.offloaded.lock();
        if off.as_ref().is_some_and(|(id, _)| id == model_id) {
            *off = None;
        }
    }
    cancel_load_of(core, model_id, files);
    let mut slot = core.gen.slot.lock().await;
    if unload_stops(core, &slot, model_id, files) {
        if let Some(p) = slot.proc.take() {
            p.stop().await;
        }
        *slot = EngineSlot::default();
        let mut f = core.gen.flags.lock();
        f.running = false;
        f.loaded_model_id = None;
        drop(f);
        engine_setup::emit_status(core);
    }
}

/// Whether [`unload_model`] stops the engine in `slot`: it has `model_id`
/// loaded, has one of the files open (a shared component: Windows can't delete
/// a mapped file, `--mmap`), or a file is in the LoRA / upscaler folders.
fn unload_stops(core: &AppCore, slot: &EngineSlot, model_id: &str, files: &[PathBuf]) -> bool {
    let per_job_dirs = [
        core.data.models(ModelKind::Lora),
        core.data.models(ModelKind::Upscaler),
    ];
    let uses_file = files.iter().any(|f| {
        slot.args.iter().any(|a| Path::new(a) == f.as_path())
            || per_job_dirs.iter().any(|d| f.starts_with(d))
    });
    slot.model_id.as_deref() == Some(model_id) || uses_file
}

/// Whether a job is loading `model_id` (or an engine that opens one of `files`).
fn loading_one_of(core: &AppCore, model_id: &str, files: &[PathBuf]) -> bool {
    core.gen.loading.lock().as_ref().is_some_and(|(id, args)| {
        id == model_id
            || files
                .iter()
                .any(|f| args.iter().any(|a| Path::new(a) == f.as_path()))
    })
}

/// Whether [`unload_model`] would stop the engine under a running job (which
/// would then fail as if the engine had crashed). A load of the model being
/// deleted doesn't count: unload cancels that job instead.
pub(crate) async fn unload_interrupts_job(
    core: &AppCore,
    model_id: &str,
    files: &[PathBuf],
) -> bool {
    if core.gen.active.lock().is_none() || loading_one_of(core, model_id, files) {
        return false;
    }
    let slot = core.gen.slot.lock().await;
    core.gen.active.lock().is_some() && unload_stops(core, &slot, model_id, files)
}

/// A load of `model_id` (or of an engine that opens one of `files`) keeps the
/// engine slot for up to [`LOAD_TIMEOUT`]: cancel that job so a delete doesn't
/// wait for it.
fn cancel_load_of(core: &AppCore, model_id: &str, files: &[PathBuf]) {
    if loading_one_of(core, model_id, files) {
        if let Some(tok) = core.gen.active.lock().as_ref() {
            tok.cancel();
        }
    }
}

/// Stop sd-server if it ran a job since it was launched (its finished jobs,
/// images included, stay readable at `GET /sdcpp/v1/jobs/{id}` for 600 s).
/// External (test) engines are never stopped. Returns whether one was stopped.
pub(crate) async fn stop_if_results_cached(core: &AppCore) -> bool {
    let mut slot = core.gen.slot.lock().await;
    if !slot.results_cached || slot.proc.is_none() {
        return false;
    }
    if let Some(p) = slot.proc.take() {
        p.stop().await;
    }
    *slot = EngineSlot::default();
    {
        let mut f = core.gen.flags.lock();
        f.running = false;
        f.loaded_model_id = None;
    }
    drop(slot);
    engine_setup::emit_status(core);
    true
}

/// Reset: stop sd-server so its cached results go with the session.
/// While a job runs, the stop happens right after it instead.
pub(crate) async fn clear_engine_results(core: &AppCore) {
    match core.gen.run_lock.try_lock() {
        Ok(_run) => {
            core.gen.clear_pending.store(false, Ordering::SeqCst);
            stop_if_results_cached(core).await;
        }
        Err(_) => core.gen.clear_pending.store(true, Ordering::SeqCst),
    }
}

/// A job just ended (run lock still held): honour a pending Reset and
/// arm the idle stop.
pub(crate) async fn after_job(core: &Arc<AppCore>, epoch: u64) {
    if core.gen.clear_pending.swap(false, Ordering::SeqCst) {
        stop_if_results_cached(core).await;
    }
    arm_idle_stop(core, epoch);
}

/// After [`IDLE_STOP_AFTER`] without a new generate / upscale, stop sd-server
/// if it holds finished results.
pub(crate) fn arm_idle_stop(core: &Arc<AppCore>, epoch: u64) {
    let Ok(rt) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let after = *core.gen.idle_stop_after.lock();
    let weak = Arc::downgrade(core);
    rt.spawn(async move {
        tokio::time::sleep(after).await;
        let Some(core) = weak.upgrade() else { return };
        if core.gen.activity.load(Ordering::SeqCst) != epoch {
            return;
        }
        // Holding the run lock keeps a new job from starting mid-stop.
        let Ok(_run) = core.gen.run_lock.try_lock() else {
            return;
        };
        if core.gen.activity.load(Ordering::SeqCst) == epoch {
            stop_if_results_cached(&core).await;
        }
    });
}

/// Model path sd-server reports in `/sdcpp/v1/capabilities` for these launch
/// args: `--model`, else `--diffusion-model` (upstream `resolve_display_model_path`).
pub(crate) fn launched_model_path(args: &[String]) -> Option<&str> {
    let value_of = |flags: &[&str]| {
        args.windows(2)
            .find(|w| flags.contains(&w[0].as_str()))
            .map(|w| w[1].as_str())
    };
    value_of(&["--model", "-m"]).or_else(|| value_of(&["--diffusion-model"]))
}

/// Same file? Exact string, canonical paths, or (when the reported path can't
/// be resolved) the same file name: on Windows sd-server's `std::filesystem`
/// round trip can garble non-ASCII folder names (e.g. `C:\Users\José`).
pub(crate) fn same_file_path(reported: &str, expected: &str) -> bool {
    let (reported, expected) = (reported.trim(), expected.trim());
    if reported.is_empty() || expected.is_empty() {
        return false;
    }
    if reported == expected {
        return true;
    }
    let (r, e) = (Path::new(reported), Path::new(expected));
    match (r.canonicalize(), e.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        (Err(_), _) => {
            let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_lowercase());
            name(r).is_some() && name(r) == name(e)
        }
        _ => false,
    }
}

/// After `wait_ready`: the server answering on the port must be the child we
/// started (still running, reporting the model we launched it with).
pub(crate) async fn verify_engine_identity(
    proc: &mut EngineProcess,
    client: &SdClient,
    args: &[String],
) -> CoreResult<()> {
    let taken = || CoreError::new("engine_failed", PORT_TAKEN_MESSAGE);
    if !proc.is_running() {
        return Err(
            taken().with_details("the engine exited while another program answered on its port")
        );
    }
    if let Some(expected) = launched_model_path(args) {
        let caps = client
            .capabilities()
            .await
            .map_err(|e| taken().with_details(format!("capabilities check failed: {e}")))?;
        let reported = caps.model.map(|m| m.path).unwrap_or_default();
        if !same_file_path(&reported, expected) {
            return Err(
                taken().with_details("the server on the engine's port reports a different model")
            );
        }
    }
    if !proc.is_running() {
        return Err(taken().with_details("the engine exited"));
    }
    Ok(())
}

/// The engine is decoding the finished picture: its last stage line (since the
/// job started) is "decoding N latents", not a later "generating image",
/// "hires … upscale" or "decode_first_stage completed" (stable-diffusion.cpp
/// `src/pipeline/image.cpp`).
pub(crate) fn decoding_now(own: &str) -> bool {
    let mut decoding = false;
    for line in own.lines() {
        if line.contains("decode_first_stage completed")
            || line.contains("generating image")
            || line.contains(" - hires ")
        {
            decoding = false;
        } else if line.contains(" decoding ") && line.contains(" latents") {
            decoding = true;
        }
    }
    decoding
}

pub(crate) fn emit_progress(
    core: &AppCore,
    phase: GenPhase,
    label: &str,
    queue: Option<u32>,
    step: Option<(u32, u32)>,
    t0: Instant,
) {
    let mut notes = core.gen.job_note.lock().clone();
    if let Some(n) = core.gen.not_on_gpu.lock().clone() {
        notes.insert(0, n);
    }
    let note = notes.join(" ");
    core.emit(CoreEvent::Generation(GenerationProgress {
        phase,
        model_label: Some(label.to_string()),
        queue_position: queue,
        step: step.map(|s| s.0),
        total_steps: step.map(|s| s.1),
        elapsed_ms: t0.elapsed().as_millis() as u64,
        note: (!note.is_empty()).then_some(note),
    }));
}

/// Note about other programs' graphics memory, measured at each engine start
/// (replaces the one from an earlier start of this job; shown first).
pub(crate) fn set_others_note(core: &AppCore, note: Option<String>) {
    let mut n = core.gen.job_note.lock();
    n.retain(|x| !x.starts_with(OTHERS_PREFIX));
    if let Some(note) = note {
        n.insert(0, note);
    }
}

/// Show `note` for the automatic retry that is starting (replaces the note of
/// an earlier retry of the same job; other notes stay).
pub(crate) fn set_retry_note(core: &AppCore, note: &str) {
    let mut n = core.gen.job_note.lock();
    n.retain(|x| !RETRY_NOTES.contains(&x.as_str()));
    n.push(note.to_string());
}

/// Remove `flag` (and its value) from an argument list.
pub(crate) fn strip_flag(args: &mut Vec<String>, flags: &[&str]) {
    let mut out = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        if flags.contains(&args[i].as_str()) {
            i += 2;
            continue;
        }
        if flags.iter().any(|f| args[i].starts_with(&format!("{f}="))) {
            i += 1;
            continue;
        }
        out.push(args[i].clone());
        i += 1;
    }
    *args = out;
}

/// Append the pinned launch `defaults` whose flag `args` doesn't set yet. A
/// skipped flag's value is skipped with it (a token that doesn't start with
/// `-`, or a number like `-1`), so no stray value token is left behind.
fn add_defaults(args: &mut Vec<String>, defaults: &[String]) {
    let name = |t: &str| t.split('=').next().unwrap_or(t).to_string();
    let is_value = |t: &str| !t.starts_with('-') || t.parse::<f64>().is_ok();
    let mut i = 0;
    while i < defaults.len() {
        let d = &defaults[i];
        let takes_value = d.starts_with("--")
            && !d.contains('=')
            && defaults.get(i + 1).is_some_and(|v| is_value(v));
        let end = if takes_value { i + 2 } else { i + 1 };
        let set = d.starts_with("--")
            && args
                .iter()
                .any(|a| a.starts_with("--") && name(a) == name(d));
        if !set {
            args.extend_from_slice(&defaults[i..end]);
        }
        i = end;
    }
}

/// The pinned sd-server is locked down (RELEASE-SPEC §12): it refuses requests
/// from web pages ([`sdapi::REJECT_ORIGIN_FLAG`]) and requests without this
/// launch's API key. Compiled in, not a setting: with `true` only an engine
/// build with the patch starts. engine.yaml pins the patched build from
/// Pinhole's fork (`engine/sd-cpp/`).
pub(crate) const ENGINE_LOCKDOWN: bool = true;

/// Full sd-server argv (without the port): wiring args + pinned defaults, forced
/// to listen on 127.0.0.1, with LoRA / upscaler folders.
pub(crate) fn full_sd_args(
    core: &AppCore,
    wiring_args: &[String],
    cfg: &EngineConfig,
) -> Vec<String> {
    let mut args = wiring_args.to_vec();
    strip_flag(
        &mut args,
        &[
            "--listen-ip",
            "-l",
            "--listen-port",
            "--log-level",
            "--serve-html-path",
            "--api-key",
        ],
    );
    args.retain(|a| a != sdapi::REJECT_ORIGIN_FLAG);
    // Only tuning flags are taken from engine.yaml's launch defaults.
    let mut defaults = pinhole_registry::wiring::keep_tuning_flags(
        &cfg.stable_diffusion_cpp.launch_defaults,
        &[
            "--listen-ip",
            "-l",
            "--log-level",
            sdapi::REJECT_ORIGIN_FLAG,
        ],
    );
    // The key only travels in the environment (a command line is visible to
    // other programs); the lock-down flag only follows `ENGINE_LOCKDOWN`.
    strip_flag(
        &mut defaults,
        &["--listen-ip", "-l", "--listen-port", "--api-key"],
    );
    defaults.retain(|a| a != sdapi::REJECT_ORIGIN_FLAG);
    // Log level: the pin's last `--log-level` (sd.cpp: the last one wins),
    // never below info — verbose / debug print the request, prompt included.
    let level = defaults
        .windows(2)
        .rev()
        .find(|w| w[0] == "--log-level")
        .map(|w| w[1].to_ascii_lowercase())
        .filter(|l| matches!(l.as_str(), "info" | "warn" | "error"))
        .unwrap_or_else(|| "info".into());
    strip_flag(&mut defaults, &["--log-level"]);
    add_defaults(&mut args, &defaults);
    args.retain(|a| a != "--verbose" && a != "-v");
    args.extend(["--log-level".into(), level]);
    // Always pass --disable-image-metadata (besides `embed_image_metadata: false` per
    // request).
    if !args.iter().any(|a| a == "--disable-image-metadata") {
        args.push("--disable-image-metadata".into());
    }
    if !args.iter().any(|a| a == "--lora-model-dir") {
        args.extend([
            "--lora-model-dir".into(),
            core.data
                .models(ModelKind::Lora)
                .to_string_lossy()
                .into_owned(),
        ]);
    }
    if !args.iter().any(|a| a == "--hires-upscalers-dir") {
        args.extend([
            "--hires-upscalers-dir".into(),
            core.data
                .models(ModelKind::Upscaler)
                .to_string_lossy()
                .into_owned(),
        ]);
    }
    if ENGINE_LOCKDOWN {
        args.push(sdapi::REJECT_ORIGIN_FLAG.into());
    }
    args.extend(["--listen-ip".into(), "127.0.0.1".into()]);
    args
}

/// Why a freshly started engine isn't usable.
enum ReadyFailure {
    Ready(ReadyError),
    /// Another program answered on the port ([`verify_engine_identity`]).
    NotOurs(CoreError),
}

/// Shown while the engine runs when a GPU build ended up without the graphics card.
pub(crate) const NOT_ON_GPU_NOTE: &str = "The engine isn't using your graphics card, so pictures are made much more slowly. Update or reinstall your graphics driver, then restart Pinhole. Settings → Engine → Show engine output shows which devices it found.";

/// Same, for Linux with an NVIDIA card: the engine reaches it through the NVIDIA Vulkan driver.
pub(crate) const NOT_ON_GPU_NOTE_LINUX_NVIDIA: &str = "The engine can't reach your NVIDIA card, so pictures are made much more slowly. On Linux it needs NVIDIA's Vulkan driver: reinstall the NVIDIA driver (on Ubuntu: sudo ubuntu-drivers install), check that vulkaninfo --summary lists your card, then restart Pinhole.";

/// The card was found but was full when the engine started.
pub(crate) const GPU_FULL_NOTE: &str = "Your graphics card had no free memory when the model loaded, so pictures are made on the processor (much slower). Close other programs that use the card, then pick the model again.";

fn not_on_gpu_note(core: &AppCore, gpu_backend: bool, log: &[String]) -> Option<String> {
    if !gpu_backend {
        return None;
    }
    let nvidia = crate::app::effective(core)
        .gpu
        .is_some_and(|g| g.vendor == pinhole_hardware::Vendor::Nvidia);
    let note = match pinhole_engine::failure::missed_gpu(log, nvidia)? {
        pinhole_engine::failure::MissedGpu::NoFreeMemory => GPU_FULL_NOTE,
        pinhole_engine::failure::MissedGpu::NotFound if nvidia && cfg!(target_os = "linux") => {
            NOT_ON_GPU_NOTE_LINUX_NVIDIA
        }
        pinhole_engine::failure::MissedGpu::NotFound => NOT_ON_GPU_NOTE,
    };
    Some(note.to_string())
}

/// Clears `GenState::loading` when [`ensure_engine`] returns, on every path.
struct LoadingMark<'a>(&'a AppCore);

impl Drop for LoadingMark<'_> {
    fn drop(&mut self) {
        *self.0.gen.loading.lock() = None;
    }
}

pub(crate) async fn ensure_engine(
    core: &Arc<AppCore>,
    wiring_args: &[String],
    model_id: &str,
    label: &str,
    cancel: &CancellationToken,
    t0: Instant,
) -> CoreResult<SdClient> {
    let external = core.gen.external.lock().clone();
    if let Some(url) = external {
        core.gen.external_launches.lock().push(wiring_args.to_vec());
        let mut slot = core.gen.slot.lock().await;
        slot.args = wiring_args.to_vec();
        slot.model_id = Some(model_id.to_string());
        note_offload(core, model_id, wiring_args);
        return Ok(SdClient::new(core.local.clone(), url));
    }
    let cfg = engine_setup::engine_config(core)?;
    let args = full_sd_args(core, wiring_args, &cfg);
    let installed = engine_setup::installed_engine(core, EngineKind::Sd);
    let mut slot = core.gen.slot.lock().await;
    // From here until return this job holds the slot: a delete of this model
    // cancels it (`cancel_load_of`) instead of waiting for the slot.
    *core.gen.loading.lock() = Some((model_id.to_string(), args.clone()));
    let _loading = LoadingMark(core);
    {
        let s = &mut *slot;
        if let Some(p) = s.proc.as_mut() {
            // Same args AND the same engine build (the backend may have changed in Settings).
            let same_build = installed.as_ref().is_some_and(|i| i.exe == p.exe());
            if p.is_running() && s.args == args && same_build {
                return Ok(slot_client(core, p, s.api_key.as_deref()));
            }
        }
    }
    // The old engine must be gone (its graphics memory freed) before the next
    // one starts: `stop` waits for the exit; one that outlives the wait is
    // killed by the leftover sweep below.
    if let Some(old) = slot.proc.take() {
        old.stop().await;
    }
    *slot = EngineSlot::default();
    *core.gen.not_on_gpu.lock() = None;
    {
        let mut f = core.gen.flags.lock();
        f.running = false;
        f.loaded_model_id = None;
    }
    // The old engine is gone even if this load is cancelled or fails before the spawn.
    engine_setup::emit_status(core);

    let installed = installed.ok_or_else(|| {
        CoreError::new("engine_missing", "The image engine isn't set up yet. Click “Get the engine” (Settings → Engine) to download it, then try again.")
    })?;

    engine_setup::ensure_runtime(core, &installed)?;
    emit_progress(core, GenPhase::LoadingModel, label, None, None, t0);

    let gpu_backend = installed.backend != "cpu";
    if gpu_backend {
        crate::describe::stop_if_idle(core).await;
    }
    engine_setup::sweep_orphans(core).await;
    let others = if gpu_backend {
        engine_setup::gpu_others(core).await
    } else {
        None
    };
    set_others_note(
        core,
        others
            .as_ref()
            .filter(|o| o.is_significant())
            .map(others_note),
    );
    *core.gen.gpu_others.lock() = others;

    if cancel.is_cancelled() {
        return Err(CoreError::new("cancelled", "Cancelled."));
    }
    let port = free_port().map_err(|e| {
        CoreError::internal("Couldn't find a free local port for the engine.")
            .with_details(e.to_string())
    })?;
    let mut argv = args.clone();
    argv.extend(["--listen-port".into(), port.to_string()]);
    core.gen.logs.clear();
    let api_key = crate::describe::new_api_key();
    let mut proc = EngineProcess::spawn_with_env(
        &installed.exe,
        &argv,
        &[(sdapi::API_KEY_ENV, api_key.as_str())],
        port,
        core.gen.logs.clone(),
    )
    .map_err(|e| {
        CoreError::new("engine_failed", "The image engine couldn't be started. Your antivirus may have blocked it — try setting up the engine again.").with_details(e.to_string())
    })?;
    {
        let mut f = core.gen.flags.lock();
        f.loading = true;
        f.error = None;
    }
    engine_setup::emit_status(core);
    emit_progress(core, GenPhase::LoadingModel, label, None, None, t0);

    let client = SdClient::new(core.local.clone(), proc.base_url()).with_api_key(api_key.clone());
    let logs = core.gen.logs.clone();
    let mut last_emit = Instant::now();
    let ready = proc
        .wait_ready(
            || client.is_ready(),
            LOAD_TIMEOUT,
            cancel,
            |_| {
                if last_emit.elapsed() >= Duration::from_millis(500) {
                    last_emit = Instant::now();
                    let step = logs
                        .progress()
                        .filter(|p| p.kind == ProgressKind::Loading)
                        .map(|p| (p.step, p.total));
                    emit_progress(core, GenPhase::LoadingModel, label, None, step, t0);
                }
            },
        )
        .await;
    core.gen.flags.lock().loading = false;
    // Port squatting: whoever answered must be our child with our model.
    let ready = match ready {
        Ok(()) => verify_engine_identity(&mut proc, &client, &args)
            .await
            .map_err(ReadyFailure::NotOurs),
        Err(e) => Err(ReadyFailure::Ready(e)),
    };
    match ready {
        Ok(()) => {
            let log = core.gen.logs.tail(usize::MAX);
            let plan = pinhole_engine::failure::memory_plan(&log);
            if !plan.is_empty() {
                *core.gen.memory_plan.lock() = Some((model_id.to_string(), plan));
            }
            *core.gen.not_on_gpu.lock() = not_on_gpu_note(core, gpu_backend, &log);
            note_offload(core, model_id, wiring_args);
            slot.proc = Some(proc);
            slot.args = args;
            slot.model_id = Some(model_id.to_string());
            slot.api_key = Some(api_key);
            {
                let mut f = core.gen.flags.lock();
                f.running = true;
                f.loaded_model_id = Some(model_id.to_string());
            }
            engine_setup::emit_status(core);
            Ok(client)
        }
        Err(e) => {
            let code = proc.exit_code();
            let err = match e {
                ReadyFailure::NotOurs(err) => {
                    proc.kill().await;
                    err
                }
                ReadyFailure::Ready(e) => {
                    proc.stop().await;
                    match e {
                        ReadyError::Cancelled => CoreError::new("cancelled", "Cancelled."),
                        ReadyError::Timeout => CoreError::new("engine_failed", "The model took too long to load. Try again, or try a smaller version of this model.")
                            .with_details(core.gen.logs.tail_text(40)),
                        ReadyError::Exited { .. } => {
                            let err = engine_failure(&core.gen.logs, code);
                            match err.code.as_str() {
                                "vram" if gpu_backend => CoreError { message: vram_message(core), ..err },
                                "vram" => CoreError { message: RAM_MESSAGE.into(), ..err },
                                _ => err,
                            }
                        }
                    }
                }
            };
            if err.code != "cancelled" {
                core.gen.flags.lock().error = Some(err.clone());
            }
            engine_setup::emit_status(core);
            Err(err)
        }
    }
}

#[cfg(windows)]
const MISSING_LIBRARY_MESSAGE: &str = "The image engine is missing a system component. Install the Microsoft Visual C++ Redistributable (x64) and update your graphics driver, or switch the engine to CPU in Settings.";

#[cfg(not(windows))]
const MISSING_LIBRARY_MESSAGE: &str = "The image engine is missing a system library (usually the Vulkan driver: install mesa-vulkan-drivers or your GPU's driver). Or switch the engine to CPU in Settings.";

/// Plain-language error for an engine failure (ring-buffer tail in `details`).
pub(crate) fn engine_failure(logs: &LogBuffer, exit_code: Option<i32>) -> CoreError {
    let tail = logs.tail_text(40);
    failure_error(classify(&tail, exit_code)).with_details(exit_details(tail, exit_code))
}

/// Engine output plus the exit code, for the Details toggle.
pub(crate) fn exit_details(tail: String, exit_code: Option<i32>) -> String {
    match exit_code {
        Some(c) if !tail.is_empty() => format!("{tail}\n(exit code {c})"),
        Some(c) => format!("exit code {c}"),
        None => tail,
    }
}

/// Code + message that say what to do next (no details).
pub(crate) fn failure_error(failure: Failure) -> CoreError {
    let (code, msg) = match failure {
        Failure::OutOfMemory => ("vram", VRAM_MESSAGE),
        Failure::DriverTooOld => (
            "engine_failed",
            "Your NVIDIA driver is too old for the image engine. Update it (version 570 or newer), or switch the engine to Vulkan in Settings.",
        ),
        Failure::NoGpu => ("engine_failed", "The image engine couldn't use your graphics card. Update your graphics driver, or switch the engine to CPU in Settings."),
        Failure::GlibcTooOld => ("engine_failed", "The image engine needs Ubuntu 24.04 or newer. Please update your system."),
        Failure::MissingLibrary => ("engine_failed", MISSING_LIBRARY_MESSAGE),
        Failure::ModelLoad => (
            "model_load",
            "This model couldn't be loaded — the file may be damaged or not supported. Delete it in Models → Installed and download it again.",
        ),
        Failure::Unknown => ("engine_failed", "The engine stopped unexpectedly. Try again; if it keeps happening, try the Fast setting or restart Pinhole."),
    };
    CoreError::new(code, msg)
}

/// Kill the engine (cancel while generating, or after it died).
pub(crate) async fn drop_engine(core: &AppCore) {
    let mut slot = core.gen.slot.lock().await;
    if let Some(p) = slot.proc.take() {
        p.kill().await;
    }
    *slot = EngineSlot::default();
    let mut f = core.gen.flags.lock();
    f.running = false;
    f.loaded_model_id = None;
    drop(f);
    engine_setup::emit_status(core);
}

/// `Some(exit code)` if the managed engine process has died.
pub(crate) async fn engine_died(core: &AppCore) -> Option<Option<i32>> {
    if core.gen.external.lock().is_some() {
        return None;
    }
    let mut slot = core.gen.slot.lock().await;
    let p = slot.proc.as_mut()?;
    if p.is_running() {
        None
    } else {
        Some(p.exit_code())
    }
}

/// A client for the running sd-server (with its API key), if one runs.
pub(crate) async fn running_engine(core: &AppCore) -> Option<SdClient> {
    if let Some(u) = core.gen.external.lock().clone() {
        return Some(SdClient::new(core.local.clone(), u));
    }
    let mut slot = core.gen.slot.lock().await;
    let s = &mut *slot;
    let p = s.proc.as_mut()?;
    p.is_running()
        .then(|| slot_client(core, p, s.api_key.as_deref()))
}

fn slot_client(core: &AppCore, p: &EngineProcess, api_key: Option<&str>) -> SdClient {
    let client = SdClient::new(core.local.clone(), p.base_url());
    match api_key {
        Some(k) => client.with_api_key(k),
        None => client,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_while_decoding_are_not_steps() {
        let sampling = "[INFO ] image.cpp:866  - generating image: 1/1 - seed 42\n";
        assert!(!decoding_now(sampling));
        let decoding = format!("{sampling}[INFO ] image.cpp:899  - sampling completed, taking 3.10s\n[INFO ] image.cpp:554  - decoding 1 latents\n");
        assert!(decoding_now(&decoding));
        assert!(decoding_now(&format!(
            "{decoding}[INFO ] image.cpp:552  - decoding 1/2 latents\n"
        )));
        assert!(!decoding_now(&format!(
            "{decoding}[INFO ] image.cpp:624  - decode_first_stage completed, taking 1.20s\n"
        )));
        assert!(!decoding_now(&format!(
            "{decoding}[INFO ] image.cpp:866  - generating image: 2/2 - seed 43\n"
        )));
    }

    #[test]
    fn launched_model_path_mirrors_the_engine() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            launched_model_path(&a(&["--diffusion-model", "/d", "--model", "/m"])),
            Some("/m")
        );
        assert_eq!(
            launched_model_path(&a(&["--vae", "/v", "--diffusion-model", "/d"])),
            Some("/d")
        );
        assert_eq!(launched_model_path(&a(&["--vae", "/v"])), None);
    }

    #[test]
    fn same_file_path_compares_strings_canonical_paths_and_names() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("model.safetensors");
        let g = tmp.path().join("other.safetensors");
        std::fs::write(&f, b"x").unwrap();
        std::fs::write(&g, b"y").unwrap();
        let fs = f.to_string_lossy().into_owned();
        assert!(same_file_path(&fs, &fs));
        let dotted = tmp.path().join(".").join("model.safetensors");
        assert!(same_file_path(&dotted.to_string_lossy(), &fs));
        assert!(
            !same_file_path(&g.to_string_lossy(), &fs),
            "a different existing file"
        );
        assert!(
            same_file_path("/unresolvable/elsewhere/MODEL.safetensors", &fs),
            "re-encoded path: same name"
        );
        assert!(!same_file_path("/mock/mock.safetensors", &fs));
        assert!(!same_file_path("", &fs) && !same_file_path(&fs, ""));
    }

    #[test]
    fn launch_defaults_skip_a_set_flag_with_its_value() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<String>>();
        let mut args = v(&["--model", "m", "--threads", "4", "--mmap"]);
        add_defaults(
            &mut args,
            &v(&[
                "--threads",
                "8",
                "--mmap",
                "--seed",
                "-1",
                "--rng=cuda",
                "--disable-image-metadata",
            ]),
        );
        assert_eq!(
            args,
            v(&[
                "--model",
                "m",
                "--threads",
                "4",
                "--mmap",
                "--seed",
                "-1",
                "--rng=cuda",
                "--disable-image-metadata"
            ])
        );
        let mut args = v(&["--rng", "cpu"]);
        add_defaults(
            &mut args,
            &v(&["--rng=cuda", "--offload-to-cpu", "--vae-tiling"]),
        );
        assert_eq!(
            args,
            v(&["--rng", "cpu", "--offload-to-cpu", "--vae-tiling"])
        );
    }

    #[test]
    fn strip_flag_removes_values_and_equals_forms() {
        let mut a: Vec<String> = [
            "--model",
            "m",
            "--listen-ip",
            "0.0.0.0",
            "--listen-port=9",
            "--vae",
            "v",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        strip_flag(&mut a, &["--listen-ip", "--listen-port"]);
        assert_eq!(a, vec!["--model", "m", "--vae", "v"]);
    }

    #[test]
    fn engine_failure_messages_say_what_to_do() {
        let logs = LogBuffer::default();
        logs.push_line("ggml_backend_cuda_buffer_type_alloc_buffer: allocating 9000 MiB on device 0: cudaMalloc failed: out of memory");
        let e = engine_failure(&logs, Some(1));
        assert_eq!(e.code, "vram");
        assert_eq!(e.message, VRAM_MESSAGE);
        assert!(e.details.unwrap().contains("exit code 1"));
        let e = engine_failure(&LogBuffer::default(), None);
        assert!(e.message.starts_with("The engine stopped unexpectedly"));
        // A model that fails to load has its own code, so the UI can offer "Open Models".
        let logs = LogBuffer::default();
        logs.push_line("[ERROR] main.cpp:91  - new_sd_ctx_t failed");
        let e = engine_failure(&logs, Some(1));
        assert_eq!(e.code, "model_load");
        assert!(
            e.message.starts_with("This model couldn't be loaded"),
            "{}",
            e.message
        );
        assert!(e.details.unwrap().contains("new_sd_ctx_t failed"));
    }
}
