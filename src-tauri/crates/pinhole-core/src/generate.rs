//! Generate / cancel / upscale / prompt preview. OWNER: engine agent.
//!
//! Flow (docs/ARCHITECTURE.md §4 "Generate"): installed model + family →
//! components → `wiring::launch_args` → (re)start sd-server only when the args
//! differ → combine prompt/style/prefix/negatives in memory → `img_gen` with
//! `embed_image_metadata: false` → poll every 300 ms → decode, scrub every PNG
//! text chunk, keep in the RAM [`Session`](crate::session::Session).
//!
//! Out of memory (docs/ARCHITECTURE.md §4): before sd-server starts, leftover
//! engines under `Data/engine/` are killed, an idle describe engine is stopped
//! and (NVIDIA) graphics memory used by other programs is measured. A job that
//! runs out of memory is retried with each memory-saving choice at most once:
//! reading the prompt → the text encoder moves to the processor
//! (`--backend te=cpu`, remembered per model for the app session; Settings
//! `textEncoderOnCpu` can force it on or off); decoding / unknown stage →
//! `--vae-tiling` if it isn't on yet; then (right away when denoising runs
//! out) more of the card is kept free (`--max-vram -4`, every GPU launch has
//! `-2`, see [`VRAM_RESERVE_GIB`]); then the weights stay in system memory and
//! are streamed to the card (`--offload-to-cpu`, only while that engine stays
//! loaded). Otherwise the error is `vram` with a
//! message that says what to do next (never the generic "couldn't make this
//! image").
//!
//! PRIVACY: `GenerateRequest`, `FinalPromptPreview` and the engine request body
//! carry prompt text. They are never logged, never written to disk and never
//! put into a `CoreError`. The only disk write here is `last_used` (a number)
//! in `installed.json`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use pinhole_engine::detail::{DetailError, DetailPlan};
use pinhole_engine::extend::{Canvas, ExtendError, ExtendPlan};
use pinhole_engine::failure::{classify, memory_failure, Failure, Stage};
use pinhole_engine::install::EngineKind;
use pinhole_engine::logbuf::{LogBuffer, ProgressKind};
use pinhole_engine::pins::EngineConfig;
use pinhole_engine::process::{free_port, EngineProcess, ReadyError};
use pinhole_engine::sdapi::{
    self, ApiError, CancelOutcome, Guidance, HiresRequest, ImgGenRequest, Job, JobStatus, LoraRef,
    SampleParams, SdClient, UpscaleRequest, VaeTilingRequest,
};
use pinhole_engine::words::CheckedPrompt;
use pinhole_hardware::OtherGpuUse;
use pinhole_registry::style::FinalPrompt;
use pinhole_registry::wiring::{
    self, Dials, FamilyUi, FineTune, GenMode, HwContext, LaunchExtras, ModelFiles, Quality, Shape,
};
use pinhole_registry::{Family, Layout};
use pinhole_store::datadir::ModelKind;
use pinhole_store::InstalledFile;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::engine_setup;
use crate::events::{GenPhase, GenerationProgress};
use crate::session::SessionImage;
use crate::{AppCore, CoreError, CoreEvent, CoreResult};

/// How long a model may take to load before we give up (huge models on slow disks).
const LOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const POLL_EVERY: Duration = Duration::from_millis(300);
/// Registry component id of the ESRGAN upscaler.
pub const UPSCALER_COMPONENT: &str = "realesrgan_x4";
/// sd-server has no authentication and keeps every finished job (base64 images
/// included) at `GET /sdcpp/v1/jobs/{id}` for 600 s. So once a job ran on a
/// Pinhole-started engine, the engine is stopped this long after the last
/// generate / upscale (and on Reset); the next Generate reloads it.
pub const IDLE_STOP_AFTER: Duration = Duration::from_secs(5 * 60);
/// Another process answered on the port we started an engine on.
pub const PORT_TAKEN_MESSAGE: &str = "Another program is using Pinhole's engine port — try again.";
/// The graphics card ran out of memory (no numbers about other programs).
pub const VRAM_MESSAGE: &str = "Your graphics card ran out of memory. Close other programs that use the graphics card (games, other AI apps) and try again, or pick the smaller version of this model in Models.";
/// System memory ran out (CPU engine, or the text encoder already on the processor).
pub const RAM_MESSAGE: &str = "Your computer ran out of memory. Close other programs and try again, or pick the smaller version of this model in Models.";
/// Reading the prompt ran out of graphics memory while Settings keeps the text encoder on the card.
pub const TE_ON_GPU_MESSAGE: &str = "Your graphics card ran out of memory while reading your prompt. In Settings → Engine, set “Read the prompt on the processor” to Automatic or On, or close other programs that use the graphics card and try again.";
/// A job that doesn't say why it failed.
pub const UNKNOWN_JOB_MESSAGE: &str = "The engine couldn't make this image. Try again with different settings (e.g. the Fast setting or a smaller size).";
pub(crate) const TE_RETRY_NOTE: &str = "Your graphics card ran out of memory while reading your prompt — trying again with that step on the processor (a bit slower).";
pub(crate) const TILING_RETRY_NOTE: &str =
    "Your graphics card ran out of memory — trying once more with memory-saving settings.";
pub(crate) const OFFLOAD_RETRY_NOTE: &str = "Your graphics card ran out of memory — trying again with the model kept in system memory and sent to the card as needed (slower).";
pub(crate) const MORE_ROOM_RETRY_NOTE: &str = "Your graphics card ran out of memory — trying again with more of the card kept free and the model sent to it in parts (slower).";
const RETRY_NOTES: &[&str] = &[
    TE_RETRY_NOTE,
    TILING_RETRY_NOTE,
    MORE_ROOM_RETRY_NOTE,
    OFFLOAD_RETRY_NOTE,
];
const MORE_ROOM_NOTE: &str = "This model keeps more of the graphics card free and is sent to it in parts, because the card ran out of memory earlier. Pictures take a bit longer.";
/// Graphics memory (GiB) a GPU launch keeps free on top of the engine's own
/// estimate (`--max-vram -2` on a 12 GB+ card, see [`vram_reserves`]). sd.cpp budgets weights + working memory + 0.5 GiB
/// (backend_fit.cpp, model_manager.cpp `check_capacity`) and runs the whole
/// model in one piece when that fits the free memory it measured. On Windows /
/// CUDA the real use was ~0.85 GiB higher (a Krea 2 edit on a 16 GB card: 12.5 GB
/// of weights staged, then 1.6 GB free for a 1.9 GB workspace), and the job
/// failed with every weight held for that one piece, so nothing could be
/// evicted. With a budget below free memory, a model that would only just fit
/// runs in parts instead (ggml_runner.cpp segmented execution): the card caches
/// what fits and the rest streams in, like Forge's reserved inference memory.
pub(crate) const VRAM_RESERVE_GIB: u8 = 2;
/// The reserve after running out of memory anyway on a 16 GB+ card
/// ([`MORE_ROOM_RETRY_NOTE`]).
pub(crate) const MORE_ROOM_RESERVE_GIB: u8 = 4;

/// (reserve at launch, reserve for the "more room" retry) in GiB for a card
/// with `vram_gb` (0 = unknown). Smaller cards keep less free: the reserve
/// comes out of what auto-fit may keep on the card (free − reserve instead of
/// free − 0.5 GiB). The retry stays at a quarter of the card at most, because
/// sd.cpp treats a reserve at or above the free memory as no limit at all
/// (ggml_graph_cut.cpp `resolve_auto_max_vram_bytes`). Below 8 GB the launch
/// keeps the engine's own default.
pub(crate) fn vram_reserves(vram_gb: f32) -> (u8, u8) {
    if !(vram_gb.is_finite() && vram_gb > 0.0) {
        (1, 2)
    } else if vram_gb >= 16.0 {
        (VRAM_RESERVE_GIB, MORE_ROOM_RESERVE_GIB)
    } else if vram_gb >= 12.0 {
        (VRAM_RESERVE_GIB, 3)
    } else if vram_gb >= 8.0 {
        (1, 2)
    } else if vram_gb >= 4.0 {
        (0, 1)
    } else {
        (0, 0)
    }
}
const OFFLOAD_NOTE: &str = "This model is kept in system memory and sent to the graphics card as needed, because the card ran out of memory. Pictures take longer until the model is next loaded.";
/// System memory kept free when deciding whether a model fits there (OS, other apps).
const OFFLOAD_SPARE_RAM_GB: f64 = 2.0;
const TE_ON_CPU_NOTE: &str = "Your prompt is read on the processor for this model because the graphics card ran out of memory earlier. You can change this in Settings → Engine (“Read the prompt on the processor”).";
const TILING_ON_NOTE: &str = "This model finishes pictures in smaller pieces to save memory, because it ran out of memory earlier (a bit slower). You can turn this off in Fine-tune.";
/// Reading the prompt failed without any sign of running out of memory.
/// (Memory lines can be missing from the output, so closing other programs comes first.)
pub const ENCODER_FAILED_MESSAGE: &str = "The engine couldn't read your prompt. Close other programs that use the graphics card and try again. If it keeps happening, one of this model's files may be damaged or the wrong version: reinstall it in Models.";

// ================================================================ IPC types (mirror src/lib/types.ts)

/// `LoraUse`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoraUse {
    pub lora_id: String,
    pub weight: f32,
    /// Trigger words picked on the add-on's chip; `None` = all of them. Only
    /// words the add-on actually lists are used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<String>>,
}

/// `GenerateRequest` — prompt-bearing: Deserialize only, redacting Debug.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateRequest {
    pub model_id: String,
    pub mode: GenMode,
    pub prompt: String,
    #[serde(default)]
    pub style_id: Option<String>,
    pub dials: Dials,
    #[serde(default)]
    pub fine_tune: FineTune,
    #[serde(default)]
    pub loras: Vec<LoraUse>,
    #[serde(default)]
    pub add_trigger_words: bool,
    #[serde(default)]
    pub init_image_id: Option<String>,
    #[serde(default)]
    pub strength: Option<f32>,
    #[serde(default)]
    pub ref_image_ids: Vec<String>,
    #[serde(default)]
    pub mask_image_id: Option<String>,
    /// Edit "Fix details" (img2img + mask): redraw only a padded box around the
    /// mask at the model's native size, then blend it back into the source.
    #[serde(default)]
    pub fix_details: bool,
    /// Edit "Extend" (img2img): put the source on this bigger canvas and draw
    /// the new space (plus a seam over the old edge), then paste the source back.
    #[serde(default)]
    pub extend: Option<ExtendCanvas>,
}

/// `ExtendCanvas`: the new canvas in source pixels; `left`/`top` = where the
/// source's top-left corner goes.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtendCanvas {
    pub width: u32,
    pub height: u32,
    pub left: u32,
    pub top: u32,
}

impl GenerateRequest {
    /// Plain txt2img with default dials (square, fast, middle "stick", 1 image).
    pub fn txt2img(model_id: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
            mode: GenMode::Txt2img,
            prompt: prompt.into(),
            style_id: None,
            dials: Dials {
                shape: Shape::Square,
                quality: Quality::Fast,
                stick: 0.5,
                count: 1,
            },
            fine_tune: FineTune::default(),
            loras: Vec::new(),
            add_trigger_words: true,
            init_image_id: None,
            strength: None,
            ref_image_ids: Vec::new(),
            mask_image_id: None,
            fix_details: false,
            extend: None,
        }
    }
}

impl std::fmt::Debug for GenerateRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenerateRequest")
            .field("model_id", &self.model_id)
            .field("mode", &self.mode)
            .field("prompt", &"[redacted]")
            .field("style_id", &self.style_id)
            .field("loras", &self.loras.len())
            .finish()
    }
}

/// `ResultKind`: how a session image was made.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResultKind {
    /// txt2img / img2img / edit output.
    #[default]
    Generated,
    /// Output of `upscale_image`. Model / seed / sampling fields are copied from
    /// the source image (empty model id and seed 0 for an imported source).
    Upscaled,
}

/// Where a session image came from (RELEASE-SPEC §3.1). Anything made from an
/// `Imported` image stays `Imported`, through every mode; a marker inside a file
/// never makes an import `Generated`. Unknown (e.g. deserialized without the
/// field) counts as `Imported`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    /// The chain starts with a Create run without a brought-in reference picture.
    Generated,
    /// Brought in (file, paste, drag, CivitAI example, a reopened save), or made from such an image.
    #[default]
    Imported,
}

impl Origin {
    /// Origin of a result made from `inputs`: `Imported` if any input is.
    pub fn of_result<'a>(inputs: impl IntoIterator<Item = &'a Origin>) -> Origin {
        if inputs.into_iter().any(|o| *o == Origin::Imported) {
            Origin::Imported
        } else {
            Origin::Generated
        }
    }
}

/// `ResultImage`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResultImage {
    pub id: String,
    #[serde(default)]
    pub kind: ResultKind,
    pub width: u32,
    pub height: u32,
    pub seed: i64,
    pub model_id: String,
    pub model_label: String,
    pub family_id: String,
    pub steps: u32,
    pub cfg: f32,
    pub guidance: Option<f32>,
    pub sampler: Option<String>,
    pub scheduler: Option<String>,
    pub parent_id: Option<String>,
    #[serde(default)]
    pub origin: Origin,
    /// The size the picture was made at, before hires fix or an upscale enlarged it: what
    /// "settings (no prompt)" records, so reusing them makes the same picture again.
    #[serde(skip)]
    pub base_size: Option<(u32, u32)>,
}

/// `GenerateResult`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GenerateResult {
    pub images: Vec<ResultImage>,
}

/// `FinalPromptPreview` — prompt-bearing (returned to the UI only).
#[derive(Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FinalPromptPreview {
    pub prompt: String,
    pub negative: Option<String>,
}

impl std::fmt::Debug for FinalPromptPreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FinalPromptPreview([redacted])")
    }
}

/// `ImportedImage`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedImage {
    pub id: String,
    pub width: u32,
    pub height: u32,
}

/// `SavedImage`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedImage {
    pub path: String,
}

/// One file written by "Save all".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedEntry {
    pub id: String,
    pub path: String,
}

/// `SavedBatch`: what "Save all" wrote; `failed` counts images that couldn't be saved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedBatch {
    pub saved: Vec<SavedEntry>,
    pub failed: usize,
}

// ================================================================ state

/// Memory-saving launch choices made automatically for one model (RAM only,
/// kept for the rest of the app session).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MemFallback {
    /// `--backend te=cpu`: the text encoder runs on the processor.
    pub te_on_cpu: bool,
    /// `--vae-tiling`
    pub vae_tiling: bool,
    /// `--offload-to-cpu`: every weight lives in system memory (mapped from the
    /// file, `--mmap`) and the card only caches what fits, so the engine can
    /// make room for its working memory. Never remembered for the session: it
    /// only sticks while the engine that needed it stays loaded.
    pub offload: bool,
    /// `--max-vram -N`: graphics memory (GiB) the engine keeps free beyond its
    /// own estimate. 0 = don't pass it (CPU engine, small card). In the
    /// remembered choices: only set when a retry raised it.
    pub vram_reserve_gib: u8,
    /// The reserve a "more room" retry raises it to; 0 = no such retry.
    /// See [`vram_reserves`].
    pub more_room_gib: u8,
}

/// Settings `textEncoderOnCpu`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TeChoice {
    Auto,
    On,
    Off,
}

impl TeChoice {
    pub(crate) fn current(core: &AppCore) -> Self {
        match core.settings.read().text_encoder_on_cpu.as_str() {
            "on" => TeChoice::On,
            "off" => TeChoice::Off,
            _ => TeChoice::Auto,
        }
    }
}

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

/// Cancel the running generation (or model load). No-op when idle.
pub fn cancel(core: &AppCore) {
    if let Some(tok) = core.gen.active.lock().as_ref() {
        tok.cancel();
    }
}

/// Dials + Fine-tune defaults for a family.
pub fn family_ui(core: &AppCore, family_id: &str) -> CoreResult<FamilyUi> {
    let reg = core.registry();
    let fam = reg.family(family_id).ok_or_else(|| {
        CoreError::not_found("Pinhole doesn't know this model family. Pick another model.")
    })?;
    Ok(wiring::family_ui(&reg, fam))
}

// ================================================================ prompt

struct Prepared {
    model: InstalledFile,
    family: Family,
    final_prompt: FinalPrompt,
    /// `final_prompt.prompt` after the word check: the only prompt the engine request takes.
    prompt: CheckedPrompt,
    /// User texts for log redaction (memory only).
    secrets: Vec<String>,
    loras: Vec<LoraRef>,
    /// The model or a picked LoRA is marked "safe images only" on CivitAI.
    safe_images_only: bool,
}

/// Read-only "Final prompt sent to the model" (combined in memory, never stored).
pub fn preview_final_prompt(
    core: &AppCore,
    req: &GenerateRequest,
) -> CoreResult<FinalPromptPreview> {
    let p = prepare(core, req, false)?;
    Ok(FinalPromptPreview {
        prompt: p.final_prompt.prompt,
        negative: p.final_prompt.negative,
    })
}

/// `materialize`: make linked add-ons readable by the engine (a link or copy into Pinhole's
/// add-on folder). Off for the read-only prompt preview, which writes nothing.
fn prepare(core: &AppCore, req: &GenerateRequest, materialize: bool) -> CoreResult<Prepared> {
    let (model, family) = resolve_model(core, req)?;
    let reg = core.registry();

    // LoRAs → structured list (+ trigger words, in memory).
    let lora_dir = core.data.models(ModelKind::Lora);
    let mut loras = Vec::new();
    let mut triggers: Vec<String> = Vec::new();
    // Every picked add-on's name and trigger words, for the word check below (whether or
    // not the words are added to the prompt: the add-on steers the image either way).
    let mut addon_words: Vec<String> = Vec::new();
    crate::lookup::refuse_if_flagged(&model)?;
    let mut safe_images_only = model.safe_images_only();
    {
        let idx = core.installed.lock();
        // Add-ons were picked for the chosen model; an edit that fell back to
        // another model (the chosen one can't edit) doesn't get them.
        let picked = if model.id == req.model_id {
            &req.loras[..]
        } else {
            &[]
        };
        for l in picked {
            let Some(f) = idx.get(&l.lora_id).filter(|f| f.kind == ModelKind::Lora) else {
                return Err(CoreError::not_found("A style add-on (LoRA) you picked isn't installed anymore. Remove it in Fine-tune."));
            };
            let mut abs = idx.abs_path(&core.data, f);
            if f.is_linked() {
                if !abs.is_file() {
                    return Err(CoreError::not_found(format!(
                        "The add-on “{}” isn't in the other app's folder any more. Check that its drive is connected, or remove it in Fine-tune.",
                        f.friendly_name
                    )));
                }
                // sd-server only loads add-ons from Pinhole's add-on folder.
                if materialize {
                    abs = crate::linked::lora_path_for_engine(core, f, &abs)?;
                } else {
                    abs = lora_dir.join(f.id.as_str());
                }
            }
            let rel = abs
                .strip_prefix(&lora_dir)
                .ok()
                .map(|r| r.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/"))
                .filter(|r| !r.is_empty())
                .ok_or_else(|| CoreError::invalid("A style add-on (LoRA) is stored outside Data/models/loras. Reinstall it from Models."))?;
            if (materialize || !f.is_linked()) && !abs.is_file() {
                return Err(CoreError::not_found(format!(
                    "The add-on “{}” is missing from the Data folder. Reinstall it from Models.",
                    f.friendly_name
                )));
            }
            loras.push(LoraRef {
                path: rel,
                multiplier: if l.weight.is_finite() {
                    l.weight.clamp(-4.0, 4.0)
                } else {
                    1.0
                },
            });
            crate::lookup::refuse_if_flagged(f)?;
            safe_images_only |= f.safe_images_only();
            addon_words.push(f.friendly_name.clone());
            addon_words.extend(f.trigger_words().iter().map(|w| w.to_string()));
            // CivitAI's own name and trained words too: editing the trigger words changes
            // what goes into the prompt, not what the add-on was trained on.
            if let Some(c) = &f.civitai {
                addon_words.extend(c.model_name.iter().cloned());
                addon_words.extend(c.version_name.iter().cloned());
                addon_words.extend(c.trained_words.iter().cloned());
            }
            if req.add_trigger_words {
                for w in f.trigger_words() {
                    let w = w.trim();
                    let picked = l
                        .words
                        .as_ref()
                        .is_none_or(|p| p.iter().any(|p| p.trim().eq_ignore_ascii_case(w)));
                    if picked
                        && !w.is_empty()
                        && !triggers.iter().any(|t| t.eq_ignore_ascii_case(w))
                    {
                        triggers.push(w.to_string());
                    }
                }
            }
        }
    }

    let mut prompt = req.prompt.trim().to_string();
    let missing: Vec<&String> = triggers
        .iter()
        .filter(|t| !contains_phrase(&prompt, t))
        .collect();
    if !missing.is_empty() {
        let joined = missing
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        prompt = if prompt.is_empty() {
            joined
        } else {
            format!("{prompt}, {joined}")
        };
    }

    let style = match req.style_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => Some(crate::library::get_style(core, id).map_err(|_| {
            CoreError::not_found("That style doesn't exist anymore. Pick another style.")
        })?),
        None => None,
    };
    let apply_prefix =
        req.fine_tune.auto_prompt_prefix.unwrap_or(true) && req.mode != GenMode::Edit;
    let final_prompt = pinhole_registry::style::combine(
        &reg,
        &family,
        &prompt,
        style.as_ref().map(|s| s.positive.as_str()),
        style.as_ref().and_then(|s| s.negative.as_deref()),
        req.fine_tune.negative_prompt.as_deref(),
        apply_prefix,
    );

    // The whole positive prompt (idea + style + trigger words) and the add-ons; the negative
    // prompt is where people list what to keep out, so it isn't checked.
    let prompt = crate::text_check::checked_with(final_prompt.prompt.clone(), &addon_words)?;

    let mut secrets = vec![req.prompt.clone(), final_prompt.prompt.clone()];
    if let Some(n) = &final_prompt.negative {
        secrets.push(n.clone());
    }
    if let Some(n) = &req.fine_tune.negative_prompt {
        secrets.push(n.clone());
    }
    if let Some(s) = &style {
        secrets.push(s.positive.clone());
    }
    Ok(Prepared {
        model,
        family,
        final_prompt,
        prompt,
        secrets,
        loras,
        safe_images_only,
    })
}

/// The installed model + its family. Edit mode uses the best installed edit model
/// when the chosen one isn't an edit model.
/// Is `phrase` already in `text` as whole words (case-insensitive)? "art" is
/// not in "heart", so a trigger word isn't skipped by a longer word.
pub(crate) fn contains_phrase(text: &str, phrase: &str) -> bool {
    let (text, phrase) = (text.to_lowercase(), phrase.trim().to_lowercase());
    if phrase.is_empty() {
        return true;
    }
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    text.match_indices(&phrase).any(|(i, m)| {
        !word(text[..i].chars().next_back()) && !word(text[i + m.len()..].chars().next())
    })
}

fn resolve_model(core: &AppCore, req: &GenerateRequest) -> CoreResult<(InstalledFile, Family)> {
    let reg = core.registry();
    let idx = core.installed.lock();
    let chosen = idx
        .get(&req.model_id)
        .filter(|f| matches!(f.kind, ModelKind::Checkpoint | ModelKind::Diffusion))
        .cloned();
    // Two images ("take the bottle from image 2") need a family that combines them.
    let two_images = req.mode == GenMode::Edit && req.ref_image_ids.len() > 1;
    let is_edit = |f: &InstalledFile| {
        f.family
            .as_deref()
            .and_then(|id| reg.family(id))
            .is_some_and(|fam| {
                pinhole_registry::wiring::can_edit(fam) && (!two_images || fam.multi_ref)
            })
    };

    let model = if req.mode == GenMode::Edit {
        match chosen.filter(|f| is_edit(f)) {
            Some(m) => m,
            None => idx
                .models()
                .filter(|f| is_edit(f))
                .min_by_key(|f| {
                    f.family
                        .as_deref()
                        .and_then(|id| reg.family(id))
                        .and_then(|fam| fam.edit_priority)
                        .unwrap_or(u32::MAX)
                })
                .cloned()
                .ok_or_else(|| {
                    CoreError::not_found(if two_images {
                        "None of your models can combine two images. Remove the second image, or get Qwen-Image 2.1 from the Edit tab."
                    } else {
                        "No edit model is installed yet. Get one from the Edit tab."
                    })
                })?,
        }
    } else {
        chosen.ok_or_else(|| {
            CoreError::not_found("That model isn't installed anymore. Pick another model.")
        })?
    };
    let fid = model
        .family
        .clone()
        .ok_or_else(|| CoreError::invalid("Pinhole doesn't know what kind of model this is yet. Pick its model family in Models → Installed."))?;
    let family = reg.family(&fid).cloned().ok_or_else(|| CoreError::invalid("This model's family isn't in Pinhole's model list anymore. Update Pinhole or pick another model."))?;
    let mode_name = match req.mode {
        GenMode::Txt2img => "txt2img",
        GenMode::Img2img => "img2img",
        GenMode::Edit => "edit",
    };
    if !family.modes.is_empty() && !family.modes.iter().any(|m| m == mode_name) {
        let msg = match req.mode {
            GenMode::Edit => {
                "This model can't do instruction edits. Use Restyle, or get an edit model."
            }
            _ if family.modes.iter().all(|m| m == "edit") => {
                "This is an edit model — use it in the Edit tab."
            }
            _ => "This model doesn't support that mode. Pick another model.",
        };
        return Err(CoreError::invalid(msg));
    }
    Ok((model, family))
}

fn component_label(kind: &str) -> &'static str {
    match kind {
        "vae" => "its VAE",
        "clip_l" => "the CLIP-L text encoder",
        "clip_g" => "the CLIP-G text encoder",
        "t5xxl" => "the T5 text encoder",
        "llm" => "its text encoder",
        "llm_vision" => "its vision encoder",
        "taesd" => "the preview decoder",
        _ => "a required component",
    }
}

/// Main file + installed components for `family` on this hardware. The layout
/// comes from how the file is installed (Checkpoint → all-in-one `--model`,
/// Diffusion → `--diffusion-model`): CivitAI all-in-one Flux files are
/// checkpoints of a diffusion-only family and carry their own VAE/encoders, so
/// for those, missing shared components are not an error.
/// `edit`: the run edits a reference image. Only then is a missing vision
/// encoder (`llm_vision`) an error: a generator that can also edit (Qwen-Image
/// 2.1) still creates without it.
pub(crate) fn model_files(
    core: &AppCore,
    model: &InstalledFile,
    family: &Family,
    hw: &HwContext,
    edit: bool,
) -> CoreResult<ModelFiles> {
    let layout = match model.kind {
        ModelKind::Checkpoint => Layout::AllInOne,
        ModelKind::Diffusion => Layout::DiffusionOnly,
        _ => family.layout,
    };
    let components_optional = layout == Layout::AllInOne && family.layout == Layout::DiffusionOnly;
    let reg = core.registry();
    let idx = core.installed.lock();
    // Another installed option of a VRAM-dependent choice (e.g. the bf16 text
    // encoder on a 16 GB card) is used rather than asking for a download.
    let installed = |id: &str| {
        idx.find_component(id)
            .is_some_and(|f| idx.abs_path(&core.data, f).is_file())
    };
    let required = wiring::required_components_with(&reg, family, hw, &installed);
    let main = idx.abs_path(&core.data, model);
    if !main.is_file() && model.is_linked() {
        return Err(CoreError::not_found(format!(
            "The file for “{}” isn't in the other app's folder any more. Check that its drive is connected, then try again.",
            model.friendly_name
        )));
    }
    if !main.is_file() {
        return Err(CoreError::not_found(format!(
            "The file for “{}” is missing from the Data folder. Reinstall it from Models.",
            model.friendly_name
        )));
    }
    let mut components = BTreeMap::new();
    let mut missing = Vec::new();
    for rc in required {
        match idx.find_component(&rc.component_id) {
            Some(f) if idx.abs_path(&core.data, f).is_file() => {
                components.insert(rc.kind.clone(), idx.abs_path(&core.data, f));
            }
            _ if components_optional => {}
            _ if rc.kind == "llm_vision" && !edit => {}
            _ => {
                let file = reg
                    .component(&rc.component_id)
                    .map(|c| c.file.clone())
                    .unwrap_or_else(|| rc.component_id.clone());
                missing.push(format!("{} ({file})", component_label(&rc.kind)));
            }
        }
    }
    if !missing.is_empty() {
        return Err(CoreError::not_found(format!(
            "This model needs {} before it can run. Open Models → Installed and click Get to download {}.",
            missing.join(" and "),
            if missing.len() == 1 { "it" } else { "them" }
        )));
    }
    Ok(ModelFiles {
        family_id: family.id.clone(),
        main,
        layout,
        components,
    })
}

// ================================================================ engine lifecycle

/// The engine is decoding the finished picture: its last stage line (since the
/// job started) is "decoding N latents", not a later "generating image",
/// "hires … upscale" or "decode_first_stage completed" (stable-diffusion.cpp
/// `src/pipeline/image.cpp`).
fn decoding_now(own: &str) -> bool {
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

fn emit_progress(
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

// ================================================================ memory

/// sd.cpp module names for the text encoder (`parse_backend_module`,
/// src/core/ggml_extend_backend.cpp; case-insensitive, `-`/`_` ignored).
const TE_MODULES: &[&str] = &[
    "te",
    "clip",
    "text",
    "textencoder",
    "textencoders",
    "conditioner",
    "cond",
    "llm",
    "t5",
    "t5xxl",
];

fn is_te_module(key: &str) -> bool {
    let k: String = key
        .trim()
        .chars()
        .filter(|c| *c != '-' && *c != '_')
        .collect::<String>()
        .to_ascii_lowercase();
    TE_MODULES.contains(&k.as_str())
}

/// Does sd-server run the text encoder on the CPU with these args?
/// (`--backend` entries in order, a later one wins; `--clip-on-cpu` is
/// prepended by sd.cpp so any explicit entry overrides it.)
pub(crate) fn text_encoder_on_cpu(args: &[String]) -> bool {
    let (mut default, mut te) = (None::<String>, None::<String>);
    for w in args.windows(2).filter(|w| w[0] == "--backend") {
        for part in w[1].split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part.split_once('=') {
                None => default = Some(part.to_string()),
                Some((k, v))
                    if matches!(
                        k.trim().to_ascii_lowercase().as_str(),
                        "all" | "default" | "*"
                    ) =>
                {
                    default = Some(v.trim().to_string())
                }
                Some((k, v)) if is_te_module(k) => te = Some(v.trim().to_string()),
                Some(_) => {}
            }
        }
    }
    match te {
        Some(b) => b.eq_ignore_ascii_case("cpu"),
        None => {
            args.iter().any(|a| a == "--clip-on-cpu")
                || default.is_some_and(|b| b.eq_ignore_ascii_case("cpu"))
        }
    }
}

/// Run the text encoder on the processor: every `--backend` list is merged
/// into one, other text-encoder entries dropped and `te=cpu` added last (sd.cpp
/// joins repeated `--backend` values with `,`; a later module entry wins).
pub(crate) fn with_text_encoder_on_cpu(args: &mut Vec<String>) {
    let mut parts: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(args.len() + 2);
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--backend" && i + 1 < args.len() {
            let keep = args[i + 1].split(',').map(str::trim).filter(|p| {
                !p.is_empty() && !p.split_once('=').is_some_and(|(k, _)| is_te_module(k))
            });
            parts.extend(keep.map(String::from));
            i += 2;
        } else {
            out.push(args[i].clone());
            i += 1;
        }
    }
    parts.push("te=cpu".into());
    out.extend(["--backend".into(), parts.join(",")]);
    *args = out;
}

/// This model's memory choices: Settings `textEncoderOnCpu` on / off, or (auto)
/// what an out-of-memory retry chose earlier this session. GPU backends only.
fn memory_choices(core: &AppCore, model_id: &str, gpu_backend: bool, vram_gb: f32) -> MemFallback {
    let mut fb = core
        .gen
        .mem_fallback
        .lock()
        .get(model_id)
        .copied()
        .unwrap_or_default();
    fb.te_on_cpu = gpu_backend
        && match TeChoice::current(core) {
            TeChoice::On => true,
            TeChoice::Off => false,
            TeChoice::Auto => fb.te_on_cpu,
        };
    let (reserve, more_room) = if gpu_backend {
        vram_reserves(vram_gb)
    } else {
        (0, 0)
    };
    fb.vram_reserve_gib = if gpu_backend {
        fb.vram_reserve_gib.max(reserve)
    } else {
        0
    };
    fb.more_room_gib = more_room;
    fb
}

/// Weights in system memory stick while this model runs with the same
/// settings (see `GenState::offloaded`); anything else tries the card again.
pub(crate) fn with_remembered_offload(
    core: &AppCore,
    model_id: &str,
    wiring_args: &[String],
    fb: MemFallback,
    gpu_backend: bool,
) -> MemFallback {
    let offload = gpu_backend && {
        let with_offload = with_memory_choices(
            wiring_args,
            MemFallback {
                offload: true,
                ..fb
            },
        );
        core.gen
            .offloaded
            .lock()
            .as_ref()
            .is_some_and(|(id, a)| id == model_id && *a == with_offload)
    };
    MemFallback { offload, ..fb }
}

/// Wiring args with the memory choices applied.
pub(crate) fn with_memory_choices(wiring_args: &[String], fb: MemFallback) -> Vec<String> {
    let mut args = wiring_args.to_vec();
    if fb.te_on_cpu && !text_encoder_on_cpu(&args) {
        with_text_encoder_on_cpu(&mut args);
    }
    if fb.vae_tiling && !args.iter().any(|a| a == "--vae-tiling") {
        args.push("--vae-tiling".into());
    }
    if fb.offload && !args.iter().any(|a| a == "--offload-to-cpu") {
        args.push("--offload-to-cpu".into());
    }
    // A --max-vram from the registry (family / hardware profile) wins.
    if fb.vram_reserve_gib > 0 && !has_max_vram(&args) {
        args.extend(["--max-vram".into(), format!("-{}", fb.vram_reserve_gib)]);
    }
    args
}

fn has_max_vram(args: &[String]) -> bool {
    args.iter()
        .any(|a| a == "--max-vram" || a.starts_with("--max-vram="))
}

/// The value of the last `flag value` / `flag=value` in `args`.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let eq = format!("{flag}=");
    let mut found = None;
    for (i, a) in args.iter().enumerate() {
        if a == flag {
            found = args.get(i + 1).cloned();
        } else if let Some(v) = a.strip_prefix(&eq) {
            found = Some(v.to_string());
        }
    }
    found
}

/// Weight files these launch args load (main model + components), in GiB.
fn weights_gb(args: &[String]) -> f64 {
    let bytes: u64 = args
        .windows(2)
        .filter(|w| pinhole_registry::wiring::WEIGHT_FILE_FLAGS.contains(&w[0].as_str()))
        .filter_map(|w| std::fs::metadata(&w[1]).ok())
        .map(|m| m.len())
        .sum();
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

/// Every weight fits in system memory with room to spare (unknown RAM: assume so).
fn offload_fits_ram(args: &[String], ram_gb: f32) -> bool {
    !(ram_gb.is_finite() && ram_gb > 0.0)
        || weights_gb(args) + OFFLOAD_SPARE_RAM_GB <= f64::from(ram_gb)
}

/// The next retry after running out of memory at `stage`, if any. Each
/// choice is made at most once, so a job is retried at most four times:
/// * reading the prompt → text encoder on the processor (Settings Automatic,
///   GPU backend, not there yet); with Settings Off → more room, then weights
///   to system memory;
/// * decoding (VAE) or unknown → VAE tiling (not on yet and allowed), then
///   more room, then weights to system memory;
/// * denoising → more room, then weights to system memory (tiling only as a
///   last resort: it only helps the VAE).
///
/// "More room" raises the graphics memory the engine keeps free (`--max-vram`,
/// e.g. from 2 to 4 GiB on a 16 GB card, see [`vram_reserves`]), so a model
/// that almost fits runs in parts and its weights give way to working memory.
/// Weights in system memory alone don't help: sd.cpp still runs the model in
/// one piece when its estimate fits, holding every weight on the card
/// (src/core/ggml_runner.cpp), which is why the `--max-vram` budget stays on.
///
/// Weights go to system memory only on a GPU backend and when they fit there
/// (`offload_ok`, see [`offload_fits_ram`]). Returns the new choices and the note.
fn next_memory_fallback(
    fb: MemFallback,
    stage: Stage,
    te: TeChoice,
    gpu_backend: bool,
    args: &[String],
    tiling_allowed: bool,
    offload_ok: bool,
) -> Option<(MemFallback, &'static str)> {
    let tiling = (tiling_allowed && !args.iter().any(|a| a == "--vae-tiling")).then_some((
        MemFallback {
            vae_tiling: true,
            ..fb
        },
        TILING_RETRY_NOTE,
    ));
    let offload = (gpu_backend && offload_ok && !args.iter().any(|a| a == "--offload-to-cpu"))
        .then_some((
            MemFallback {
                offload: true,
                ..fb
            },
            OFFLOAD_RETRY_NOTE,
        ));
    // Only when the budget in `args` is Pinhole's own (not a registry --max-vram).
    let own_reserve = flag_value(args, "--max-vram")
        == (fb.vram_reserve_gib > 0).then(|| format!("-{}", fb.vram_reserve_gib));
    let more_room = (gpu_backend && fb.vram_reserve_gib < fb.more_room_gib && own_reserve)
        .then_some((
            MemFallback {
                vram_reserve_gib: fb.more_room_gib,
                ..fb
            },
            MORE_ROOM_RETRY_NOTE,
        ))
        .or(offload);
    match stage {
        // No GPU, or the text encoder already on the processor: it's system memory.
        Stage::TextEncoder if !gpu_backend || text_encoder_on_cpu(args) => None,
        Stage::TextEncoder if te == TeChoice::Auto => Some((
            MemFallback {
                te_on_cpu: true,
                ..fb
            },
            TE_RETRY_NOTE,
        )),
        Stage::TextEncoder => more_room,
        Stage::Diffusion if gpu_backend && offload_ok => more_room,
        // Tiling as a last resort: the stage is read from the engine output.
        Stage::Diffusion => more_room.or(tiling),
        Stage::Vae | Stage::Unknown => tiling.or(more_room),
    }
}

/// `9 GB`, `8.9 GB` (MiB in, GiB out like the rest of the UI).
fn gb_text(mib: u64) -> String {
    let gb = (mib as f64 / 1024.0 * 10.0).round() / 10.0;
    if gb.fract() == 0.0 {
        format!("{gb:.0} GB")
    } else {
        format!("{gb:.1} GB")
    }
}

/// "Other programs are using 9 GB of your graphics memory: python.exe (8.9 GB)."
pub(crate) fn others_sentence(o: &OtherGpuUse) -> String {
    let mut names: Vec<(String, Option<u64>)> = Vec::new();
    for p in &o.processes {
        match names
            .iter_mut()
            .find(|(n, _)| n.eq_ignore_ascii_case(&p.name))
        {
            Some((_, used)) => {
                *used = match (*used, p.used_mib) {
                    (Some(a), Some(b)) => Some(a + b),
                    (a, b) => a.or(b),
                }
            }
            None => names.push((p.name.clone(), p.used_mib)),
        }
    }
    let listed: Vec<String> = names
        .iter()
        .take(3)
        .map(|(n, used)| match used {
            Some(m) if *m >= 100 => format!("{n} ({})", gb_text(*m)),
            _ => n.clone(),
        })
        .collect();
    let amount = gb_text(o.others_mib);
    if listed.is_empty() {
        format!("Other programs are using {amount} of your graphics memory.")
    } else {
        format!(
            "Other programs are using {amount} of your graphics memory: {}.",
            listed.join(", ")
        )
    }
}

/// How [`others_sentence`] starts (to replace an older note).
const OTHERS_PREFIX: &str = "Other programs are using ";

/// Shown while loading when other programs hold a lot of graphics memory.
pub(crate) fn others_note(o: &OtherGpuUse) -> String {
    format!(
        "{} If pictures fail, close them and try again.",
        others_sentence(o)
    )
}

/// Out of graphics memory: names other programs when the engine start saw them.
pub(crate) fn vram_message(core: &AppCore) -> String {
    match core.gen.gpu_others.lock().clone().filter(OtherGpuUse::is_significant) {
        Some(o) => format!(
            "Your graphics card ran out of memory. {} Close them and try again, or pick the smaller version of this model in Models.",
            others_sentence(&o)
        ),
        None => VRAM_MESSAGE.to_string(),
    }
}

/// Engine output of an out-of-memory job, after the memory plan this model's
/// last auto-fit launch printed (when it isn't in the output already).
fn with_memory_plan(core: &AppCore, model_id: &str, args: &[String], details: String) -> String {
    let plan = match core.gen.memory_plan.lock().as_ref() {
        Some((id, plan)) if id == model_id => plan.clone(),
        _ => return details,
    };
    if plan.iter().all(|l| details.contains(l.as_str())) {
        return details;
    }
    // An offloaded engine doesn't run auto-fit: the plan is the earlier attempt's.
    let title = if args.iter().any(|a| a == "--offload-to-cpu") {
        "Memory plan of the earlier attempt (before the weights moved to system memory):"
    } else {
        "Memory plan when the engine started:"
    };
    format!("{title}\n{}\n\n{details}", plan.join("\n"))
}

/// The final out-of-memory error for a job (after any retry).
fn memory_error(core: &AppCore, stage: Stage, args: &[String], gpu_backend: bool) -> CoreError {
    let msg = if !gpu_backend || (stage == Stage::TextEncoder && text_encoder_on_cpu(args)) {
        RAM_MESSAGE.to_string()
    } else if stage == Stage::TextEncoder && TeChoice::current(core) == TeChoice::Off {
        TE_ON_GPU_MESSAGE.to_string()
    } else {
        vram_message(core)
    };
    CoreError::new("vram", msg)
}

/// `EngineStatus.note` for the running engine.
pub(crate) fn engine_note(core: &AppCore, flags: &EngineFlags) -> Option<String> {
    if !flags.running {
        return None;
    }
    let mut notes: Vec<String> = core.gen.not_on_gpu.lock().iter().cloned().collect();
    let fb = flags
        .loaded_model_id
        .as_ref()
        .and_then(|id| core.gen.mem_fallback.lock().get(id).copied())
        .unwrap_or_default();
    if fb.te_on_cpu && TeChoice::current(core) == TeChoice::Auto {
        notes.push(TE_ON_CPU_NOTE.to_string());
    }
    if fb.vae_tiling {
        notes.push(TILING_ON_NOTE.to_string());
    }
    if fb.vram_reserve_gib > 0 && core.gen.offloaded.lock().is_none() {
        notes.push(MORE_ROOM_NOTE.to_string());
    }
    if core.gen.offloaded.lock().is_some() {
        notes.push(OFFLOAD_NOTE.to_string());
    }
    if let Some(o) = core
        .gen
        .gpu_others
        .lock()
        .clone()
        .filter(OtherGpuUse::is_significant)
    {
        notes.push(others_sentence(&o));
    }
    (!notes.is_empty()).then(|| notes.join(" "))
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
/// launch's API key. Compiled in, not a setting: with `true` an engine build
/// without the patch can't start, so the checks can't be skipped by pointing
/// engine.yaml at an upstream build. engine.yaml pins the patched build from
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
    // Only tuning flags from engine.yaml (an editable file in some installs): nothing that
    // loads content past the checks.
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
    // Privacy safeguard #2 (besides `embed_image_metadata: false` per request):
    // server-wide default off, even if engine.yaml is edited.
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

/// Make sure sd-server runs with exactly `wiring_args` (restart otherwise) and
/// is ready. Returns its base URL.
///
/// Before a launch: the old engine has fully exited (`stop` waits), leftover
/// engines under `Data/engine/` are killed, an idle describe engine is
/// stopped (GPU backends) and graphics memory used by other programs is
/// measured (NVIDIA) — a lot of it becomes a progress note.
/// Remember a launch with the weights in system memory (see `GenState::offloaded`).
fn note_offload(core: &AppCore, model_id: &str, wiring_args: &[String]) {
    *core.gen.offloaded.lock() = wiring_args
        .iter()
        .any(|a| a == "--offload-to-cpu")
        .then(|| (model_id.to_string(), wiring_args.to_vec()));
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

async fn ensure_engine(
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
fn exit_details(tail: String, exit_code: Option<i32>) -> String {
    match exit_code {
        Some(c) if !tail.is_empty() => format!("{tail}\n(exit code {c})"),
        Some(c) => format!("exit code {c}"),
        None => tail,
    }
}

/// Code + message that say what to do next (no details).
fn failure_error(failure: Failure) -> CoreError {
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
async fn drop_engine(core: &AppCore) {
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
async fn engine_died(core: &AppCore) -> Option<Option<i32>> {
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

// ================================================================ generate

/// A job that draws part of a picture and blends it back ("Fix details", "Extend").
#[derive(Clone)]
enum Redraw {
    Detail(Arc<DetailPlan>),
    Extend(Arc<ExtendPlan>),
}

impl Redraw {
    /// The finished whole picture as PNG, or `None` when the redraw can't be read.
    fn blend(&self, redraw: &[u8]) -> Option<Vec<u8>> {
        match self {
            Redraw::Detail(p) => p.blend(redraw).ok(),
            Redraw::Extend(p) => p.blend(redraw).ok(),
        }
    }
}

/// Size for img2img / edit: keep the source aspect ratio at about the dial's
/// area, rounded to the family's size multiple (SD1.5/SDXL 64, others 16).
fn size_like(src_w: u32, src_h: u32, target_area: u64, multiple: u32) -> (u32, u32) {
    let aspect = src_w.max(1) as f64 / src_h.max(1) as f64;
    let area = (target_area.max(256 * 256)) as f64;
    let h = (area / aspect).sqrt();
    let w = h * aspect;
    let r = |v: f64| wiring::round_to_multiple((v.round() as u32).clamp(256, 2048), multiple);
    (r(w), r(h))
}

fn b64_image(core: &AppCore, id: &str) -> CoreResult<(String, SessionImage)> {
    let img = core.session.get(id).ok_or_else(|| {
        CoreError::not_found("That image isn't in this session anymore. Add it again.")
    })?;
    Ok((
        base64::engine::general_purpose::STANDARD.encode(img.bytes.as_slice()),
        img,
    ))
}

/// Run one generation. See module docs.
pub async fn generate(core: &Arc<AppCore>, req: GenerateRequest) -> CoreResult<GenerateResult> {
    // Read before waiting for another job: results of a job that outlives a
    // Reset are dropped (see `Session::insert_generated`).
    let session_epoch = core.session.epoch();
    // Early word check so a blocked prompt doesn't wait behind a running job; `prepare`
    // checks the combined prompt again.
    crate::text_check::check(&req.prompt)?;
    // Held for the whole run: the Models folder can't move under the engine.
    let _folder = crate::models::folder_read(core)?;
    // Without every image check file nothing is made (fail closed).
    crate::imagecheck::ensure_ready(core)?;
    let _run = core.gen.run_lock.lock().await;
    let epoch = core.gen.activity.fetch_add(1, Ordering::SeqCst) + 1;
    core.gen.job_note.lock().clear();
    let cancel = CancellationToken::new();
    *core.gen.active.lock() = Some(cancel.clone());
    let t0 = Instant::now();
    // The model that actually runs (Edit may pick another one than `req.model_id`).
    let mut label = core
        .installed
        .lock()
        .get(&req.model_id)
        .map(|m| m.friendly_name.clone())
        .unwrap_or_default();
    let result = generate_inner(core, &req, &cancel, t0, session_epoch, &mut label).await;
    *core.gen.active.lock() = None;
    core.gen.logs.clear_secrets();
    match &result {
        Ok(_) => emit_progress(core, GenPhase::Done, &label, None, None, t0),
        Err(e) if e.code == "cancelled" => {
            emit_progress(core, GenPhase::Cancelled, &label, None, None, t0)
        }
        Err(e) => {
            if e.code == "blocked" || e.code.starts_with("check_") {
                // The engine keeps finished jobs readable on its port: a picture the check
                // dropped must go with it.
                core.gen.clear_pending.store(true, Ordering::SeqCst);
            }
            emit_progress(core, GenPhase::Failed, &label, None, None, t0)
        }
    }
    after_job(core, epoch).await;
    result
}

async fn generate_inner(
    core: &Arc<AppCore>,
    req: &GenerateRequest,
    cancel: &CancellationToken,
    t0: Instant,
    session_epoch: u64,
    final_label: &mut String,
) -> CoreResult<GenerateResult> {
    if req.mode == GenMode::Txt2img && req.prompt.trim().is_empty() {
        return Err(CoreError::invalid("Type what you want to see first."));
    }
    let prep = prepare(core, req, true)?;
    let reg = core.registry();
    let hw = crate::app::hw_context(core);
    let label = prep.model.friendly_name.clone();
    final_label.clone_from(&label);

    // Source images (checked before any engine work).
    let mut init_image = None;
    let mut ref_images = Vec::new();
    let mut mask_image = None;
    let mut source: Option<SessionImage> = None;
    // Origins of every picture the result is made from (not the mask: that is only a shape).
    let mut input_origins: Vec<Origin> = Vec::new();
    // The brought-in pictures behind them, for the image check.
    let mut sources: Vec<crate::session::Source> = Vec::new();
    // Made pictures fed in from a brought-in chain: a face can show up in them (enlarged,
    // straightened, sharpened) that the brought-in picture didn't show clearly.
    let mut inputs: Vec<crate::session::Source> = Vec::new();
    let mut add_sources = |img: &SessionImage| {
        inputs.extend(img.fed_in_source());
        for s in img.sources() {
            if !sources.iter().any(|k| k.id == s.id) {
                sources.push(s);
            }
        }
    };
    match req.mode {
        // Create's reference picture ("in the style of this picture"): sent like an edit's
        // image, but the size comes from the dials, not from the picture.
        GenMode::Txt2img => {
            if let Some(id) = req.ref_image_ids.first() {
                if !wiring::can_edit(&prep.family) {
                    return Err(CoreError::invalid(
                        "This model can't use a reference picture. Pick a FLUX.2 model, or remove the picture.",
                    ));
                }
                let (b64, img) = b64_image(core, id)?;
                input_origins.push(img.origin);
                add_sources(&img);
                ref_images.push(b64);
            }
        }
        GenMode::Img2img => {
            let id = req
                .init_image_id
                .as_deref()
                .ok_or_else(|| CoreError::invalid("Add an image to restyle first."))?;
            let (b64, img) = b64_image(core, id)?;
            input_origins.push(img.origin);
            add_sources(&img);
            init_image = Some(b64);
            source = Some(img);
        }
        GenMode::Edit => {
            let mut ids: Vec<String> = req.ref_image_ids.clone();
            if ids.is_empty() {
                ids.extend(req.init_image_id.clone());
            }
            if ids.is_empty() {
                return Err(CoreError::invalid("Add an image to edit first."));
            }
            // The Edit tab sends the image being edited plus at most one more.
            for id in ids.iter().take(2) {
                let (b64, img) = b64_image(core, id)?;
                input_origins.push(img.origin);
                add_sources(&img);
                if source.is_none() {
                    source = Some(img);
                }
                ref_images.push(b64);
            }
        }
    }
    let mut mask_src = None;
    if req.mode != GenMode::Txt2img {
        if let Some(mid) = req.mask_image_id.as_deref() {
            let (b64, img) = b64_image(core, mid)?;
            mask_image = Some(b64);
            mask_src = Some(img);
        }
    }
    if req.extend.is_some()
        && (req.fix_details || req.mode != GenMode::Img2img || mask_src.is_some())
    {
        return Err(CoreError::invalid(
            "Extend works on its own: switch off the brush and pick Extend again.",
        ));
    }
    let fix_source = if req.fix_details {
        match (req.mode, &source, mask_src) {
            (GenMode::Img2img, Some(src), Some(mask)) => Some((src, mask)),
            _ => return Err(CoreError::invalid("Paint over the spot to fix first.")),
        }
    } else {
        None
    };

    // A Create reference picture is read like an edit's image: it needs the vision encoder.
    let files = model_files(
        core,
        &prep.model,
        &prep.family,
        &hw,
        req.mode == GenMode::Edit || !ref_images.is_empty(),
    )?;
    let extras = LaunchExtras {
        lora_dir: Some(core.data.models(ModelKind::Lora)),
        upscalers_dir: Some(core.data.models(ModelKind::Upscaler)),
        vae_tiling: None,
        use_taesd: false,
    };
    let wiring_args = wiring::launch_args(&reg, &files, &hw, &extras);
    let params = wiring::resolve_params(
        &reg,
        &prep.family,
        &req.dials,
        &req.fine_tune,
        req.mode,
        &hw,
    );

    let (mut width, mut height) = (params.width, params.height);
    if let Some(src) = &source {
        if req.fine_tune.width.is_none() && req.fine_tune.height.is_none() {
            let multiple = wiring::size_multiple(&prep.family);
            (width, height) = size_like(
                src.width,
                src.height,
                u64::from(params.width) * u64::from(params.height),
                multiple,
            );
        }
    }
    // "Fix details": the engine only draws the box around the mask, scaled to
    // about the dial's area (so a small face is redrawn at the model's size).
    let fix = match fix_source {
        Some((src, mask)) => {
            let area = u64::from(params.width) * u64::from(params.height);
            let multiple = wiring::size_multiple(&prep.family);
            // Decode, resize, blur and encode: off the async workers.
            let (src_bytes, mask_bytes) = (src.bytes.clone(), mask.bytes.clone());
            let plan = tokio::task::spawn_blocking(move || {
                DetailPlan::new(&src_bytes, &mask_bytes, |w, h| {
                    size_like(w, h, area, multiple)
                })
            })
            .await
            .map_err(|_| CoreError::internal("Fixing details stopped unexpectedly."))?
            .map_err(|e| match e {
                DetailError::NothingPainted => {
                    CoreError::invalid("Paint over the spot to fix first.")
                }
                DetailError::Image(e) => CoreError::invalid(e.to_string()),
            })?;
            (width, height) = plan.work;
            init_image = Some(base64::engine::general_purpose::STANDARD.encode(&plan.init_png));
            mask_image = Some(base64::engine::general_purpose::STANDARD.encode(&plan.mask_png));
            Some(Redraw::Detail(Arc::new(plan)))
        }
        None => None,
    };
    // "Extend": the engine draws the whole bigger canvas at about the dial's area.
    let fix = match (fix, req.extend, &source) {
        (None, Some(c), Some(src)) => {
            let area = u64::from(params.width) * u64::from(params.height);
            let multiple = wiring::size_multiple(&prep.family);
            let src_bytes = src.bytes.clone();
            let canvas = Canvas {
                width: c.width,
                height: c.height,
                left: c.left,
                top: c.top,
            };
            let plan = tokio::task::spawn_blocking(move || {
                ExtendPlan::new(&src_bytes, canvas, |w, h| size_like(w, h, area, multiple))
            })
            .await
            .map_err(|_| CoreError::internal("Extending the picture stopped unexpectedly."))?
            .map_err(|e| match e {
                ExtendError::NothingToAdd => CoreError::invalid(
                    "The picture is already this shape. Pick another shape to extend it.",
                ),
                ExtendError::TooBig => CoreError::invalid(format!(
                    "The extended picture would be too big (over {} pixels on a side). Extend a smaller step, or pick a shape closer to this one.",
                    pinhole_engine::extend::MAX_SIDE
                )),
                ExtendError::Image(e) => CoreError::invalid(e.to_string()),
            })?;
            (width, height) = plan.work;
            init_image = Some(base64::engine::general_purpose::STANDARD.encode(&plan.init_png));
            mask_image = Some(base64::engine::general_purpose::STANDARD.encode(&plan.mask_png));
            Some(Redraw::Extend(Arc::new(plan)))
        }
        (fix, _, _) => fix,
    };
    let seed: i64 = match req.fine_tune.seed {
        Some(s) if s >= 0 => s,
        _ => i64::from(rand::random::<u32>() >> 1),
    };

    let mut body = ImgGenRequest::new(prep.prompt.clone(), width, height, seed);
    body.negative_prompt = prep.final_prompt.negative.clone().unwrap_or_default();
    body.clip_skip = params.clip_skip.unwrap_or(-1);
    body.batch_count = if fix.is_some() {
        1
    } else {
        params.batch_count.clamp(1, 8)
    };
    body.sample_params = SampleParams {
        sample_method: params.sampler.clone(),
        scheduler: params.scheduler.clone(),
        sample_steps: params.steps.max(1),
        flow_shift: params.flow_shift,
        guidance: Guidance {
            txt_cfg: params.cfg,
            img_cfg: None,
            distilled_guidance: params.guidance,
        },
    };
    // Hires fix would redraw the crop again; "Fix details" and "Extend" already work at the model's size.
    let hires = params.hires.as_ref().filter(|_| fix.is_none());
    body.hires = hires.map(|h| HiresRequest::image_space(h.scale, h.steps, h.denoising_strength));
    body.vae_tiling_params = if params.vae_tiling {
        Some(VaeTilingRequest { enabled: true })
    } else if req.fine_tune.vae_tiling == Some(false) {
        Some(VaeTilingRequest { enabled: false })
    } else {
        None
    };
    body.lora = prep.loras.clone();
    body.init_image = init_image;
    body.ref_images = ref_images;
    body.mask_image = mask_image;
    if req.mode == GenMode::Img2img {
        body.strength = Some(req.strength.unwrap_or(0.55).clamp(0.05, 1.0));
    }
    // The new space starts from noise: only the mask decides what is kept.
    if matches!(fix, Some(Redraw::Extend(_))) {
        body.strength = Some(1.0);
    }
    // "Only change here": sd.cpp blends the denoise mask against the init latent,
    // so an edit with a mask also sends the source as init_image at full strength
    // (unmasked areas are kept, masked areas are regenerated).
    if req.mode == GenMode::Edit && body.mask_image.is_some() && body.init_image.is_none() {
        body.init_image = body.ref_images.first().cloned();
        body.strength = Some(1.0);
    }

    // Engine (restart only when the launch args differ) + job. When the graphics
    // card runs out of memory, each memory-saving choice is tried once (text
    // encoder on the processor, VAE tiling, more of the card kept free, weights
    // in system memory), so there are at most four retries (see `next_memory_fallback`).
    // A GPU engine build (a CPU build may stand in while the GPU one isn't downloaded).
    let gpu_backend = engine_setup::installed_engine(core, EngineKind::Sd)
        .map_or(hw.backend != "cpu", |e| e.backend != "cpu");
    let tiling_allowed = !params.vae_tiling && req.fine_tune.vae_tiling != Some(false);
    // (Fine-tune "VAE tiling: Off" still wins over a remembered tiling choice:
    // the request body turns tiling off per job, without an engine restart.)
    let fb = memory_choices(core, &prep.model.id, gpu_backend, hw.vram_gb);
    let mut fb = with_remembered_offload(core, &prep.model.id, &wiring_args, fb, gpu_backend);
    let steps = params.steps.max(1);
    let batches = u32::from(hires.is_none()) * body.batch_count;
    let job = loop {
        let args = with_memory_choices(&wiring_args, fb);
        let client = ensure_engine(core, &args, &prep.model.id, &label, cancel, t0).await?;
        match run_job(
            core,
            &client,
            &body,
            &prep.secrets,
            &label,
            (steps, batches),
            cancel,
            t0,
        )
        .await
        {
            Ok(job) => break job,
            Err(RunError::Failed(e)) => return Err(e),
            Err(RunError::OutOfMemory { stage, details }) => {
                let offload_ok = offload_fits_ram(&args, hw.ram_gb);
                let Some((next_fb, note)) = next_memory_fallback(
                    fb,
                    stage,
                    TeChoice::current(core),
                    gpu_backend,
                    &args,
                    tiling_allowed,
                    offload_ok,
                ) else {
                    return Err(memory_error(core, stage, &args, gpu_backend)
                        .with_details(with_memory_plan(core, &prep.model.id, &args, details)));
                };
                // Remember the automatic choice for this model (RAM only, app session).
                {
                    let mut remembered = core.gen.mem_fallback.lock();
                    let entry = remembered.entry(prep.model.id.clone()).or_default();
                    entry.te_on_cpu |= next_fb.te_on_cpu && !fb.te_on_cpu;
                    entry.vae_tiling |= next_fb.vae_tiling;
                    if next_fb.vram_reserve_gib > fb.vram_reserve_gib {
                        entry.vram_reserve_gib =
                            entry.vram_reserve_gib.max(next_fb.vram_reserve_gib);
                    }
                }
                fb = next_fb;
                set_retry_note(core, note);
                emit_progress(core, GenPhase::LoadingModel, &label, None, None, t0);
            }
        }
    };
    drop(body);
    if cancel.is_cancelled() {
        return Err(CoreError::new("cancelled", "Cancelled."));
    }

    // Decode + scrub + keep in RAM.
    let mut images = job.result.map(|r| r.images).unwrap_or_default();
    images.sort_by_key(|i| i.index);
    if images.is_empty() {
        return Err(
            CoreError::new("engine_failed", "The engine returned no image. Try again.")
                .with_details(core.gen.logs.tail_text(20)),
        );
    }
    let parent_id = source.as_ref().map(|s| s.id.clone());
    let mut pngs = Vec::with_capacity(images.len());
    let mut also_check = Vec::new();
    for img in images {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(img.b64_json.as_bytes())
            .map_err(|_| {
                CoreError::new(
                    "engine_failed",
                    "The engine returned a damaged image. Try again.",
                )
            })?;
        let mut png = pinhole_engine::png::scrub(&raw).map_err(|_| {
            CoreError::new(
                "engine_failed",
                "The engine returned a damaged image. Try again.",
            )
        })?;
        if let Some(plan) = &fix {
            // Paste the redrawn box back into the whole image (or the source into the canvas).
            // Fix details: the redraw is also checked on its own, at the size the engine drew
            // it. Shrunk into a large picture it's too small to judge.
            if matches!(plan, Redraw::Detail(_)) {
                also_check.push(png.clone());
            }
            let plan = plan.clone();
            png = tokio::task::spawn_blocking(move || plan.blend(&png))
                .await
                .ok()
                .flatten()
                .ok_or_else(|| {
                    CoreError::new(
                        "engine_failed",
                        "The engine returned a damaged image. Try again.",
                    )
                })?;
        }
        pngs.push(png);
    }
    // Result intake: every picture passes the image check first; if one is blocked,
    // none is kept. A redrawn box is also checked on its own.
    let checked = crate::imagecheck::check_results(
        core,
        pngs,
        also_check,
        sources,
        inputs,
        prep.safe_images_only,
    )
    .await?;
    // Cancel pressed during the check: nothing is kept (as for an upscale).
    if cancel.is_cancelled() {
        return Err(CoreError::new("cancelled", "Cancelled."));
    }
    let mut out = Vec::new();
    for (i, png) in checked.into_iter().enumerate() {
        let (w, h) = pinhole_engine::png::dimensions(png.png()).unwrap_or((width, height));
        let meta = ResultImage {
            id: uuid::Uuid::new_v4().to_string(),
            kind: ResultKind::Generated,
            width: w,
            height: h,
            seed: seed + i as i64,
            model_id: prep.model.id.clone(),
            model_label: label.clone(),
            family_id: prep.family.id.clone(),
            steps: params.steps,
            cfg: params.cfg,
            guidance: params.guidance,
            sampler: params.sampler.clone(),
            scheduler: params.scheduler.clone(),
            parent_id: parent_id.clone(),
            origin: Origin::of_result(&input_origins),
            // Fix details / Extend work on a crop or a canvas: the picture's own size stands.
            base_size: fix.is_none().then_some((width, height)),
        };
        if !core
            .session
            .insert_generated(session_epoch, png, meta.clone())
        {
            // Reset while the job ran: its images go with the session.
            return Err(CoreError::new("cancelled", "Cancelled."));
        }
        out.push(meta);
    }
    touch_last_used(core, &prep.model.id);
    Ok(GenerateResult { images: out })
}

fn lowered(secrets: &[String]) -> Vec<String> {
    pinhole_engine::logbuf::expand_secrets(secrets)
}

/// Why a job produced no images.
enum RunError {
    /// Cancelled, an API problem, or a failure that isn't about memory.
    Failed(CoreError),
    /// Ran out of memory at `stage`; `details` = engine output (redacted).
    OutOfMemory { stage: Stage, details: String },
}

/// Submit `body` and poll every 300 ms until the job ends. Failures are
/// classified from the engine output printed during this job only.
#[allow(clippy::too_many_arguments)]
async fn run_job(
    core: &Arc<AppCore>,
    client: &SdClient,
    body: &ImgGenRequest,
    secrets: &[String],
    label: &str,
    (steps, batches): (u32, u32),
    cancel: &CancellationToken,
    t0: Instant,
) -> Result<Job, RunError> {
    core.gen.logs.set_secrets(secrets);
    core.gen.logs.reset_progress();
    if cancel.is_cancelled() {
        return Err(RunError::Failed(CoreError::new("cancelled", "Cancelled.")));
    }
    let mark = core.gen.logs.mark();
    emit_progress(core, GenPhase::Queued, label, None, None, t0);
    let job_id = match client.submit(body).await {
        Ok(id) => id,
        Err(e) => return Err(job_failure(core, api_failure(core, e, secrets), mark)),
    };
    // From now on the engine may hold this job's images (IDLE_STOP_AFTER).
    core.gen.slot.lock().await.results_cached = true;

    let (mut passes, mut last_step) = (0u32, 0u32);
    let mut errors = 0;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(POLL_EVERY) => {}
            _ = cancel.cancelled() => {
                return Err(RunError::Failed(cancel_job(core, client, &job_id).await));
            }
        }
        if let Some(code) = engine_died(core).await {
            // Killing also waits for the output readers, so every line is in.
            drop_engine(core).await;
            let own = core.gen.logs.since_text(mark);
            let details = exit_details(core.gen.logs.tail_text(40), code);
            return Err(match memory_failure(&own) {
                Some(stage) => RunError::OutOfMemory { stage, details },
                None => RunError::Failed(failure_error(classify(&own, code)).with_details(details)),
            });
        }
        match client.job(&job_id).await {
            Ok(job) => {
                errors = 0;
                match job.status {
                    JobStatus::Queued => emit_progress(
                        core,
                        GenPhase::Queued,
                        label,
                        Some(job.queue_position),
                        None,
                        t0,
                    ),
                    JobStatus::Generating => {
                        // Decoding in tiles draws the same bar as sampling (one mark per
                        // tile): don't show the tiles as extra steps.
                        let decoding = decoding_now(&core.gen.logs.since_text(mark));
                        let step = core
                            .gen
                            .logs
                            .progress()
                            .filter(|p| p.kind == ProgressKind::Sampling && !decoding)
                            .map(|p| {
                                if batches > 1 && p.total == steps {
                                    if p.step < last_step {
                                        passes += 1;
                                    }
                                    last_step = p.step;
                                    ((passes.min(batches - 1)) * steps + p.step, steps * batches)
                                } else {
                                    (p.step, p.total)
                                }
                            });
                        emit_progress(core, GenPhase::Generating, label, None, step, t0);
                    }
                    JobStatus::Completed => return Ok(job),
                    JobStatus::Failed => {
                        // sd-server only says "generate_image returned no results": the
                        // reason is in the engine output. Let the readers catch up with it.
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        let msg = job.error.map(|e| e.message).unwrap_or_default();
                        let msg = redact_text(&msg, secrets);
                        let own = format!("{msg}\n{}", core.gen.logs.since_text(mark));
                        let details = format!("{msg}\n{}", core.gen.logs.tail_text(40))
                            .trim()
                            .to_string();
                        if let Some(stage) = memory_failure(&own) {
                            return Err(RunError::OutOfMemory { stage, details });
                        }
                        let err = match classify(&own, None) {
                            // "failed to encode prompt" with no memory line: likely a broken or mismatched text encoder.
                            Failure::Unknown
                                if pinhole_engine::failure::failed_stage(&own)
                                    == Stage::TextEncoder =>
                            {
                                CoreError::new("model_load", ENCODER_FAILED_MESSAGE)
                            }
                            Failure::Unknown => {
                                CoreError::new("engine_failed", UNKNOWN_JOB_MESSAGE)
                            }
                            other => failure_error(other),
                        };
                        return Err(RunError::Failed(err.with_details(details)));
                    }
                    JobStatus::Cancelled => {
                        return Err(RunError::Failed(CoreError::new("cancelled", "Cancelled.")))
                    }
                    JobStatus::Unknown => {}
                }
            }
            Err(ApiError::NotFound) => {
                return Err(RunError::Failed(CoreError::new(
                    "engine_failed",
                    "The engine lost track of this job. Try again.",
                )))
            }
            Err(e) => {
                errors += 1;
                if errors >= 5 {
                    return Err(job_failure(core, api_failure(core, e, secrets), mark));
                }
            }
        }
    }
}

/// A job error that may be about memory: `vram` takes the memory path, with
/// the stage from the output printed since `mark`.
fn job_failure(core: &AppCore, err: CoreError, mark: u64) -> RunError {
    if err.code == "vram" {
        let stage = pinhole_engine::failure::failed_stage(&core.gen.logs.since_text(mark));
        RunError::OutOfMemory {
            stage,
            details: err.details.unwrap_or_default(),
        }
    } else {
        RunError::Failed(err)
    }
}

/// Cancel the job; sd-server can't interrupt a running job, so kill the engine then.
async fn cancel_job(core: &AppCore, client: &SdClient, job_id: &str) -> CoreError {
    match client.cancel(job_id).await {
        Ok(CancelOutcome::Cancelled) | Ok(CancelOutcome::Gone) | Ok(CancelOutcome::Finished) => {}
        Ok(CancelOutcome::Running) | Err(_) => {
            if core.gen.external.lock().is_none() {
                drop_engine(core).await;
            }
        }
    }
    CoreError::new("cancelled", "Cancelled.")
}

/// `text` with every line that holds prompt text (`secrets`) redacted.
fn redact_text(text: &str, secrets: &[String]) -> String {
    let hidden = lowered(secrets);
    text.lines()
        .map(|l| pinhole_engine::logbuf::redact_line(l, &hidden))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Plain error for an API failure. The engine's answer goes into the details
/// with prompt text (`secrets`) redacted: it may echo the request.
pub(crate) fn api_failure(core: &AppCore, e: ApiError, secrets: &[String]) -> CoreError {
    let mut err = match e {
        ApiError::QueueFull => CoreError::new(
            "engine_failed",
            "The engine is busy. Wait for the current images to finish and try again.",
        ),
        ApiError::Connect | ApiError::Timeout => engine_failure(&core.gen.logs, None),
        ApiError::Status { code: 400, error } => CoreError::new(
            "invalid",
            "The engine didn't accept these settings. Try resetting Fine-tune to the defaults.",
        )
        .with_details(format!("HTTP 400: {error}")),
        other => CoreError::new(
            "engine_failed",
            "The engine stopped unexpectedly. Try again.",
        )
        .with_details(other.to_string()),
    };
    if let Some(d) = err.details.take() {
        err.details = Some(redact_text(&d, secrets));
    }
    err
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `last_used` in installed.json — a timestamp only.
fn touch_last_used(core: &AppCore, model_id: &str) {
    let mut idx = core.installed.lock();
    if let Some(f) = idx.get_mut(model_id) {
        f.last_used = Some(now_secs());
        let _ = idx.save(&core.data);
    }
}

// ================================================================ upscale

/// Refusal for sources the upscaler can't take: 2× also runs at 4× first.
pub(crate) const UPSCALE_TOO_LARGE: &str = "This image is too large to upscale: the upscaler works at 4× first, up to 8192 pixels per side. Try a smaller image.";

/// Upscale a session image with Real-ESRGAN (4×; 2× = 4× then halve). The
/// upscaler is downloaded on first use. Needs the engine running (any model).
pub async fn upscale_image(core: &Arc<AppCore>, id: &str, factor: u32) -> CoreResult<ResultImage> {
    let _folder = crate::models::folder_read(core)?;
    if factor != 2 && factor != 4 {
        return Err(CoreError::invalid("Upscale works at 2× or 4×."));
    }
    crate::imagecheck::ensure_ready(core)?;
    // The result is dropped when Reset happens meanwhile (see `Session::insert_generated`).
    let session_epoch = core.session.epoch();
    let src = core
        .session
        .get(id)
        .ok_or_else(|| CoreError::not_found("That image isn't in this session anymore."))?;
    if u64::from(src.width) * 4 > 8192 || u64::from(src.height) * 4 > 8192 {
        return Err(CoreError::invalid(UPSCALE_TOO_LARGE));
    }

    let _run = core.gen.run_lock.lock().await;
    let epoch = core.gen.activity.fetch_add(1, Ordering::SeqCst) + 1;
    core.gen.job_note.lock().clear();
    // Set before the first-use upscaler download, so Cancel works during it too.
    let cancel = CancellationToken::new();
    *core.gen.active.lock() = Some(cancel.clone());
    // Cancelled just as the download finished: it stays installed, no upscale.
    let ready = ensure_upscaler(core, &cancel).await.and_then(|u| {
        if cancel.is_cancelled() {
            Err(CoreError::new("cancelled", "Cancelled."))
        } else {
            Ok(u)
        }
    });
    let upscaler = match ready {
        Ok(u) => u,
        Err(e) => {
            *core.gen.active.lock() = None;
            after_job(core, epoch).await;
            return Err(e);
        }
    };
    let t0 = Instant::now();
    let mut label = String::new();
    let res = upscale_inner(
        core,
        &src,
        &upscaler,
        factor,
        &cancel,
        t0,
        session_epoch,
        &mut label,
    )
    .await;
    *core.gen.active.lock() = None;
    // The final event comes after the upscale itself (not after the model load).
    match &res {
        Ok(_) => emit_progress(core, GenPhase::Done, &label, None, None, t0),
        Err(e) if e.code == "cancelled" => {
            emit_progress(core, GenPhase::Cancelled, &label, None, None, t0)
        }
        Err(_) => emit_progress(core, GenPhase::Failed, &label, None, None, t0),
    }
    after_job(core, epoch).await;
    res
}

/// Friendly name of the model the engine has loaded ("" when none).
async fn loaded_model_label(core: &AppCore) -> String {
    let id = core.gen.slot.lock().await.model_id.clone();
    id.and_then(|id| {
        core.installed
            .lock()
            .get(&id)
            .map(|m| m.friendly_name.clone())
    })
    .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
async fn upscale_inner(
    core: &Arc<AppCore>,
    src: &SessionImage,
    upscaler_stem: &str,
    factor: u32,
    cancel: &CancellationToken,
    t0: Instant,
    session_epoch: u64,
    label: &mut String,
) -> CoreResult<ResultImage> {
    let client = match running_engine(core).await {
        Some(u) => {
            *label = loaded_model_label(core).await;
            u
        }
        None => {
            // sd-server needs a model loaded to run at all: start it with the image's
            // model, or the most recently used one.
            let model_id = pick_model_for_upscale(core, src).ok_or_else(|| {
                CoreError::not_found(
                    "Install a model first — the upscaler runs inside the image engine.",
                )
            })?;
            let prep_model = model_and_family(core, &model_id)?;
            label.clone_from(&prep_model.0.friendly_name);
            let hw = crate::app::hw_context(core);
            let files = model_files(core, &prep_model.0, &prep_model.1, &hw, false)?;
            let extras = LaunchExtras {
                lora_dir: Some(core.data.models(ModelKind::Lora)),
                upscalers_dir: Some(core.data.models(ModelKind::Upscaler)),
                vae_tiling: None,
                use_taesd: false,
            };
            let args = wiring::launch_args(&core.registry(), &files, &hw, &extras);
            // The same memory choices as Generate, so this launch doesn't forget
            // that the model's weights had to go to system memory.
            let gpu_backend = engine_setup::installed_engine(core, EngineKind::Sd)
                .map_or(hw.backend != "cpu", |e| e.backend != "cpu");
            let fb = with_remembered_offload(
                core,
                &model_id,
                &args,
                memory_choices(core, &model_id, gpu_backend, hw.vram_gb),
                gpu_backend,
            );
            let args = with_memory_choices(&args, fb);
            let r = ensure_engine(
                core,
                &args,
                &model_id,
                &prep_model.0.friendly_name,
                cancel,
                t0,
            )
            .await;
            r?
        }
    };
    // A Cancel before the request goes out leaves the (maybe just loaded) engine alone.
    if cancel.is_cancelled() {
        return Err(CoreError::new("cancelled", "Cancelled."));
    }
    emit_progress(core, GenPhase::Generating, label, None, None, t0);
    let b64 = base64::engine::general_purpose::STANDARD.encode(src.bytes.as_slice());
    let req = UpscaleRequest::new(b64, Some(upscaler_stem.to_string()), 1);
    let resp = tokio::select! {
        r = client.upscale(&req) => r,
        _ = cancel.cancelled() => {
            // sd-server upscales synchronously and can't be interrupted: without
            // a stop it keeps working (up to the request timeout) and blocks the next job.
            if core.gen.external.lock().is_none() {
                drop_engine(core).await;
            }
            return Err(CoreError::new("cancelled", "Cancelled."));
        }
    };
    let resp = resp.map_err(|e| match e {
        // The upscale request carries no prompt; `redact_text` still cuts prompt-like fields.
        ApiError::Status { code: 400, error } => {
            CoreError::new("invalid", "The upscaler couldn't process this image.")
                .with_details(redact_text(&error, &[]))
        }
        other => api_failure(core, other, &[]),
    })?;
    let img = resp.images.into_iter().next().ok_or_else(|| {
        CoreError::new(
            "engine_failed",
            "The upscaler returned no image. Try again.",
        )
    })?;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(img.b64_json.as_bytes())
        .map_err(|_| CoreError::new("engine_failed", "The upscaler returned a damaged image."))?;
    let png = tokio::task::spawn_blocking(move || -> CoreResult<Vec<u8>> {
        let clean = pinhole_engine::png::scrub(&raw).map_err(|_| {
            CoreError::new("engine_failed", "The upscaler returned a damaged image.")
        })?;
        if factor == 4 {
            return Ok(clean);
        }
        let (rgba, w, h) = pinhole_engine::image::decode_rgba(&clean).map_err(|e| {
            CoreError::new("engine_failed", "The upscaler returned a damaged image.")
                .with_details(e.to_string())
        })?;
        let (small, sw, sh) = pinhole_engine::image::downscale_2x_box(&rgba, w, h);
        pinhole_engine::image::encode_png_rgba(&small, sw, sh).map_err(|e| {
            CoreError::internal("Couldn't finish the 2× upscale.").with_details(e.to_string())
        })
    })
    .await
    .map_err(|e| {
        CoreError::internal("The upscale was interrupted.").with_details(e.to_string())
    })??;
    let (w, h) =
        pinhole_engine::png::dimensions(&png).unwrap_or((src.width * factor, src.height * factor));
    let mut meta = src.meta.clone().unwrap_or(ResultImage {
        id: String::new(),
        kind: ResultKind::Upscaled,
        width: 0,
        height: 0,
        seed: 0,
        model_id: String::new(),
        model_label: "Upscaled image".into(),
        family_id: String::new(),
        steps: 0,
        cfg: 0.0,
        guidance: None,
        sampler: None,
        scheduler: None,
        parent_id: None,
        origin: src.origin,
        base_size: None,
    });
    meta.base_size = meta.base_size.or(Some((src.width, src.height)));
    meta.id = uuid::Uuid::new_v4().to_string();
    meta.kind = ResultKind::Upscaled;
    meta.origin = src.origin;
    meta.width = w;
    meta.height = h;
    meta.parent_id = Some(src.id.clone());
    // Checked like every made picture (one way in), and it keeps the source's brought-in
    // pictures for later edits.
    let from = src.sources();
    let input = src.fed_in_source().into_iter().collect();
    let checked = crate::imagecheck::check_results(core, vec![png], Vec::new(), from, input, false)
        .await?
        .pop()
        .ok_or_else(|| CoreError::internal("The upscale returned no image."))?;
    if cancel.is_cancelled()
        || !core
            .session
            .insert_generated(session_epoch, checked, meta.clone())
    {
        return Err(CoreError::new("cancelled", "Cancelled."));
    }
    Ok(meta)
}

fn model_and_family(core: &AppCore, model_id: &str) -> CoreResult<(InstalledFile, Family)> {
    let reg = core.registry();
    let idx = core.installed.lock();
    let m = idx
        .get(model_id)
        .cloned()
        .ok_or_else(|| CoreError::not_found("That model isn't installed anymore."))?;
    let fam = m
        .family
        .as_deref()
        .and_then(|f| reg.family(f))
        .cloned()
        .ok_or_else(|| CoreError::invalid("Pinhole doesn't know how to run this model."))?;
    Ok((m, fam))
}

/// A client for the running sd-server (with its API key), if one runs.
async fn running_engine(core: &AppCore) -> Option<SdClient> {
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

fn pick_model_for_upscale(core: &AppCore, src: &SessionImage) -> Option<String> {
    let idx = core.installed.lock();
    let reg = core.registry();
    let usable = |f: &&InstalledFile| f.family.as_deref().and_then(|id| reg.family(id)).is_some();
    if let Some(m) = src
        .meta
        .as_ref()
        .and_then(|m| idx.get(&m.model_id))
        .filter(|f| usable(f))
    {
        return Some(m.id.clone());
    }
    idx.models()
        .filter(usable)
        .max_by_key(|f| (f.last_used.unwrap_or(0), f.added_at))
        .map(|f| f.id.clone())
}

/// Wait for a download group; `cancel` cancels the group (its `.part` stays for
/// a later resume) and returns `cancelled`. Biased to the wait: a group that
/// finished just as Cancel came still returns its files, so they get registered.
async fn wait_download_or_cancel(
    downloads: &pinhole_net::download::DownloadManager,
    group: &str,
    cancel: &CancellationToken,
) -> CoreResult<Vec<pinhole_net::download::DownloadedFile>> {
    tokio::select! {
        biased;
        r = downloads.wait_detailed(group) => r.map_err(|e| CoreError::new(&e.code, e.message)),
        _ = cancel.cancelled() => {
            downloads.cancel(group);
            Err(CoreError::new("cancelled", "Cancelled."))
        }
    }
}

/// Make sure the Real-ESRGAN component is installed; returns its file stem (the
/// sd-server upscaler name).
async fn ensure_upscaler(core: &Arc<AppCore>, cancel: &CancellationToken) -> CoreResult<String> {
    let stem_of = |f: &InstalledFile| {
        std::path::Path::new(&f.rel_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
    };
    {
        let idx = core.installed.lock();
        if let Some(f) = idx.find_component(UPSCALER_COMPONENT) {
            if idx.abs_path(&core.data, f).is_file() {
                return stem_of(f).ok_or_else(|| CoreError::internal("Bad upscaler file name."));
            }
        }
    }
    let reg = core.registry();
    let comp = reg.component(UPSCALER_COMPONENT).cloned().ok_or_else(|| {
        CoreError::not_found("The upscaler isn't in Pinhole's model list. Update Pinhole.")
    })?;
    let dest = core.data.models(ModelKind::Upscaler).join(&comp.file);
    crate::models::models_dir_for_write(core, ModelKind::Upscaler)?;
    let sha = comp.sha256.trim().to_ascii_lowercase();
    let spec = pinhole_net::download::DownloadSpec {
        url: comp.url.clone(),
        dest,
        sha256: (sha.len() == 64 && sha.chars().all(|c| c.is_ascii_hexdigit())).then_some(sha),
        // `size_mb` is rounded: an estimate only, never the exact size.
        size_bytes: None,
        approx_size_bytes: Some(comp.size_mb * 1_000_000),
        label: "Upscaler (Real-ESRGAN 4×)".into(),
        ..Default::default()
    };
    let group = core.downloads.enqueue_kind(
        "Upscaler (Real-ESRGAN 4×)".into(),
        pinhole_net::download::DownloadKind::Upscaler,
        vec![spec],
    );
    let files = wait_download_or_cancel(&core.downloads, &group, cancel).await?;
    let file = files
        .into_iter()
        .next()
        .ok_or_else(|| CoreError::internal("The upscaler download is incomplete. Try again."))?;
    let reg_file = crate::models::register_download(
        core,
        &file,
        crate::models::Registration {
            kind: ModelKind::Upscaler,
            friendly_name: "Real-ESRGAN 4× upscaler".into(),
            family: None,
            component_id: Some(UPSCALER_COMPONENT.into()),
            civitai: None,
            dtype: None,
            lookup: None,
        },
    )?;
    stem_of(&reg_file).ok_or_else(|| CoreError::internal("Bad upscaler file name."))
}

// ================================================================ tests

#[cfg(test)]
mod tests {
    use super::*;

    /// Cancel while the first-use upscaler download runs ends the wait at once
    /// and cancels the download group.
    #[tokio::test]
    async fn cancel_ends_the_upscaler_download_wait() {
        use pinhole_net::download::{DownloadManager, DownloadSpec, DownloadState};
        use pinhole_net::testutil::{MockResponse, MockServer};
        let srv =
            MockServer::start(|_| MockResponse::ok(vec![0u8; 16]).delay(Duration::from_secs(30)))
                .await;
        let dir = tempfile::tempdir().unwrap();
        let m = DownloadManager::new(
            pinhole_net::HttpClient::new_for_tests(pinhole_net::OfflineFlag::new(false), true)
                .unwrap(),
        );
        let group = m.enqueue(
            "Upscaler".into(),
            vec![DownloadSpec {
                url: srv.url("/up.pth"),
                dest: dir.path().join("up.pth"),
                label: "Upscaler".into(),
                ..Default::default()
            }],
        );
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            c2.cancel();
        });
        let err = tokio::time::timeout(
            Duration::from_secs(10),
            wait_download_or_cancel(&m, &group, &cancel),
        )
        .await
        .expect("cancel ends the wait")
        .unwrap_err();
        assert_eq!(err.code, "cancelled");
        let mut state = None;
        for _ in 0..200 {
            state = m
                .status()
                .into_iter()
                .find(|s| s.group_id == group)
                .map(|s| s.state);
            if state == Some(DownloadState::Cancelled) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(state, Some(DownloadState::Cancelled));
    }

    /// Cancel that comes as the upscaler download finishes still hands back the
    /// finished files (so they are registered, not downloaded again).
    #[tokio::test]
    async fn finished_upscaler_download_wins_over_cancel() {
        use pinhole_net::download::{DownloadManager, DownloadSpec, DownloadState};
        use pinhole_net::testutil::{MockResponse, MockServer};
        let srv = MockServer::start(|_| MockResponse::ok(vec![7u8; 16])).await;
        let dir = tempfile::tempdir().unwrap();
        let m = DownloadManager::new(
            pinhole_net::HttpClient::new_for_tests(pinhole_net::OfflineFlag::new(false), true)
                .unwrap(),
        );
        let group = m.enqueue(
            "Upscaler".into(),
            vec![DownloadSpec {
                url: srv.url("/up.pth"),
                dest: dir.path().join("up.pth"),
                label: "Upscaler".into(),
                ..Default::default()
            }],
        );
        let mut state = None;
        for _ in 0..500 {
            state = m
                .status()
                .into_iter()
                .find(|s| s.group_id == group)
                .map(|s| s.state);
            if state == Some(DownloadState::Done) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(state, Some(DownloadState::Done));
        let cancel = CancellationToken::new();
        cancel.cancel();
        // Both branches are ready; repeat so an unbiased pick would show up.
        for _ in 0..32 {
            let files = wait_download_or_cancel(&m, &group, &cancel)
                .await
                .expect("finished download is kept");
            assert_eq!(files.len(), 1);
        }
    }

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
    fn size_like_keeps_aspect_in_family_multiples() {
        let (w, h) = size_like(1920, 1080, 1024 * 1024, 16);
        assert_eq!((w % 16, h % 16), (0, 0));
        let ratio = w as f64 / h as f64;
        assert!((ratio - 16.0 / 9.0).abs() < 0.05, "{w}x{h}");
        assert!((w as u64 * h as u64) as f64 / (1024.0 * 1024.0) > 0.9);
        let (w, h) = size_like(1920, 1080, 1024 * 1024, 64);
        assert_eq!((w % 64, h % 64), (0, 0));
        assert_eq!(size_like(10, 10, 1, 64), (256, 256));
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
    fn generate_request_debug_is_redacted_and_camel_case_parses() {
        let json = serde_json::json!({
            "modelId": "m1", "mode": "img2img", "prompt": "PINHOLE_SENTINEL_7f3a", "styleId": null,
            "dials": {"shape": "portrait", "quality": "best", "stick": 0.7, "count": 2},
            "fineTune": {"negativePrompt": "NEG", "seed": 5}, "loras": [{"loraId": "l1", "weight": 0.8}],
            "addTriggerWords": true, "initImageId": "img1", "strength": 0.35, "refImageIds": [], "maskImageId": null
        });
        let req: GenerateRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.mode, GenMode::Img2img);
        assert_eq!(req.fine_tune.seed, Some(5));
        assert_eq!(req.loras[0].lora_id, "l1");
        let dbg = format!("{req:?}");
        assert!(!dbg.contains("SENTINEL") && !dbg.contains("NEG"), "{dbg}");
        let p = FinalPromptPreview {
            prompt: "PINHOLE_SENTINEL_7f3a".into(),
            negative: None,
        };
        assert!(!format!("{p:?}").contains("SENTINEL"));
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

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn text_encoder_moves_to_the_processor_in_backend_lists() {
        let mut a = v(&["--diffusion-model", "/d", "--diffusion-fa"]);
        assert!(!text_encoder_on_cpu(&a));
        with_text_encoder_on_cpu(&mut a);
        assert_eq!(
            a,
            v(&[
                "--diffusion-model",
                "/d",
                "--diffusion-fa",
                "--backend",
                "te=cpu"
            ])
        );
        assert!(text_encoder_on_cpu(&a));

        // Existing lists are merged; other text-encoder entries (any alias) dropped.
        let mut a = v(&[
            "--backend",
            "diffusion=cuda0,CLIP=cuda0,vae=cpu",
            "--vae",
            "/v",
            "--backend",
            "t5_xxl=cuda0",
        ]);
        with_text_encoder_on_cpu(&mut a);
        assert_eq!(
            a,
            v(&["--vae", "/v", "--backend", "diffusion=cuda0,vae=cpu,te=cpu"])
        );
        let mut a = v(&["--backend", "cuda0"]);
        with_text_encoder_on_cpu(&mut a);
        assert_eq!(a, v(&["--backend", "cuda0,te=cpu"]));

        // sd.cpp semantics: a later entry wins; --clip-on-cpu is prepended.
        assert!(text_encoder_on_cpu(&v(&["--backend", "cpu"])));
        assert!(text_encoder_on_cpu(&v(&[
            "--backend",
            "all=cpu,diffusion=cuda0"
        ])));
        assert!(!text_encoder_on_cpu(&v(&["--backend", "cpu,te=cuda0"])));
        assert!(text_encoder_on_cpu(&v(&["--clip-on-cpu"])));
        assert!(!text_encoder_on_cpu(&v(&[
            "--clip-on-cpu",
            "--backend",
            "llm=cuda0"
        ])));
        assert!(!text_encoder_on_cpu(&v(&["--backend", "vae=cpu"])));
    }

    #[test]
    fn one_memory_retry_per_stage() {
        let gpu = v(&["--diffusion-model", "/d"]);
        let none = MemFallback::default();
        let next = |fb, stage, te, args: &[String]| {
            next_memory_fallback(fb, stage, te, true, args, true, true)
        };
        // Reading the prompt: text encoder to the processor, and nothing after that.
        let (fb, note) = next(none, Stage::TextEncoder, TeChoice::Auto, &gpu).unwrap();
        assert_eq!(
            (fb, note),
            (
                MemFallback {
                    te_on_cpu: true,
                    ..none
                },
                TE_RETRY_NOTE
            )
        );
        let args = with_memory_choices(&gpu, fb);
        assert!(text_encoder_on_cpu(&args), "{args:?}");
        assert!(next(fb, Stage::TextEncoder, TeChoice::Auto, &args).is_none());
        // Settings keeps it on the card: the weights go to system memory instead.
        let (fb, note) = next(none, Stage::TextEncoder, TeChoice::Off, &gpu).unwrap();
        assert_eq!(
            (fb, note),
            (
                MemFallback {
                    offload: true,
                    ..none
                },
                OFFLOAD_RETRY_NOTE
            )
        );
        // Denoising: straight to system memory (tiling only helps the VAE), once.
        let (fb, note) = next(none, Stage::Diffusion, TeChoice::Auto, &gpu).unwrap();
        assert_eq!(
            (fb, note),
            (
                MemFallback {
                    offload: true,
                    ..none
                },
                OFFLOAD_RETRY_NOTE
            )
        );
        let args = with_memory_choices(&gpu, fb);
        assert_eq!(args.iter().filter(|a| *a == "--offload-to-cpu").count(), 1);
        assert!(next(fb, Stage::Diffusion, TeChoice::Auto, &args).is_none());
        // Decoding / unknown: VAE tiling once, then system memory once.
        for stage in [Stage::Vae, Stage::Unknown] {
            let (fb, note) = next(none, stage, TeChoice::Auto, &gpu).unwrap();
            assert_eq!(
                (fb.vae_tiling, fb.offload, note),
                (true, false, TILING_RETRY_NOTE)
            );
            let args = with_memory_choices(&gpu, fb);
            assert_eq!(args.iter().filter(|a| *a == "--vae-tiling").count(), 1);
            let (fb, note) = next(fb, stage, TeChoice::Auto, &args).unwrap();
            assert_eq!(
                (fb.vae_tiling, fb.offload, note),
                (true, true, OFFLOAD_RETRY_NOTE)
            );
            assert!(next(fb, stage, TeChoice::Auto, &with_memory_choices(&gpu, fb)).is_none());
            // Fine-tune turned tiling off: system memory right away.
            let (fb, _) =
                next_memory_fallback(none, stage, TeChoice::Auto, true, &gpu, false, true).unwrap();
            assert_eq!((fb.vae_tiling, fb.offload), (false, true));
        }
        // Not enough system memory for every weight: no offload, tiling as a last resort.
        let (fb, _) = next_memory_fallback(
            none,
            Stage::Diffusion,
            TeChoice::Auto,
            true,
            &gpu,
            true,
            false,
        )
        .unwrap();
        assert_eq!((fb.vae_tiling, fb.offload), (true, false));
        assert!(next_memory_fallback(
            fb,
            Stage::Diffusion,
            TeChoice::Auto,
            true,
            &with_memory_choices(&gpu, fb),
            true,
            false
        )
        .is_none());
        // No GPU: VAE tiling only, never for the prompt.
        assert!(next_memory_fallback(
            none,
            Stage::TextEncoder,
            TeChoice::Auto,
            false,
            &gpu,
            true,
            true
        )
        .is_none());
        let (fb, _) = next_memory_fallback(
            none,
            Stage::Diffusion,
            TeChoice::Auto,
            false,
            &gpu,
            true,
            true,
        )
        .unwrap();
        assert_eq!((fb.vae_tiling, fb.offload), (true, false));
        let (fb, _) =
            next_memory_fallback(none, Stage::Vae, TeChoice::Auto, false, &gpu, true, true)
                .unwrap();
        assert_eq!((fb.vae_tiling, fb.offload), (true, false));
    }

    /// Field report (Windows, RTX 5070 Ti 16 GB, Krea 2 edit at 864×1536): the
    /// weights were already in system memory, yet sd.cpp staged all 12.5 GB of
    /// them for a one-piece run and had 1.6 GB left for a 1.9 GB workspace. Every
    /// GPU launch keeps 2 GB free (`--max-vram -2`), and the retry keeps 4 GB
    /// free so the model runs in parts, instead of repeating the same launch.
    #[test]
    fn krea2_edit_workspace_failure_retries_with_more_room() {
        let log = "[WARN   ] model_manager.cpp:1753 - model manager memory on CUDA0: reported free 1644.26 MB / total 16275.44 MB, tracked weights 12535.73 MB / other runtime 0.00 MB / current runtime 0.00 MB\n\
                   [WARN   ] model_manager.cpp:1919 - model manager cannot make enough memory available on CUDA0: need 1861.73 MB device / 1349.73 MB budget, available 1644.26 MB device / unlimited budget\n\
                   [ERROR  ] ggml_runner.cpp:899  - krea2 segment 1/1 (graph) failed during workspace capacity check\n\
                   [ERROR  ] diffusion_engine.cpp:2594 - diffusion model compute failed\n\
                   [ERROR  ] diffusion_engine.cpp:2733 - Diffusion model sampling failed\n\
                   [ERROR  ] image.cpp:907  - sampling for image 1/1 failed after 3.73s";
        let stage = memory_failure(log).expect("out of memory");
        assert_eq!(stage, Stage::Diffusion);
        let wiring = v(&["--diffusion-model", "/krea2.safetensors"]);
        let (reserve, more_room) = vram_reserves(16.0);
        let fb = MemFallback {
            vram_reserve_gib: reserve,
            more_room_gib: more_room,
            ..Default::default()
        };
        let args = with_memory_choices(&wiring, fb);
        assert_eq!(flag_value(&args, "--max-vram").as_deref(), Some("-2"));
        let next = |fb, args: &[String]| {
            next_memory_fallback(fb, stage, TeChoice::Auto, true, args, true, true)
        };
        let (fb, note) = next(fb, &args).unwrap();
        assert_eq!(
            (fb.vram_reserve_gib, fb.offload, note),
            (MORE_ROOM_RESERVE_GIB, false, MORE_ROOM_RETRY_NOTE)
        );
        let args = with_memory_choices(&wiring, fb);
        assert_eq!(flag_value(&args, "--max-vram").as_deref(), Some("-4"));
        assert_eq!(args.iter().filter(|a| *a == "--max-vram").count(), 1);
        // Then system memory, keeping the 4 GB budget; then nothing is left.
        let (fb, note) = next(fb, &args).unwrap();
        assert_eq!(
            (fb.vram_reserve_gib, fb.offload, note),
            (MORE_ROOM_RESERVE_GIB, true, OFFLOAD_RETRY_NOTE)
        );
        let args = with_memory_choices(&wiring, fb);
        assert_eq!(flag_value(&args, "--max-vram").as_deref(), Some("-4"));
        assert!(next(fb, &args).is_none());

        // A --max-vram from the registry wins and isn't raised.
        let pinned = v(&["--diffusion-model", "/d", "--max-vram", "6"]);
        let (reserve, more_room) = vram_reserves(16.0);
        let fb = MemFallback {
            vram_reserve_gib: reserve,
            more_room_gib: more_room,
            ..Default::default()
        };
        let args = with_memory_choices(&pinned, fb);
        assert_eq!(flag_value(&args, "--max-vram").as_deref(), Some("6"));
        let (fb, note) = next(fb, &args).unwrap();
        assert_eq!((fb.offload, note), (true, OFFLOAD_RETRY_NOTE));
        // CPU engine: no budget at all.
        assert!(!with_memory_choices(&wiring, MemFallback::default())
            .iter()
            .any(|a| a == "--max-vram"));
    }

    /// The reserve scales with the card and the retry stays at a quarter of it
    /// (sd.cpp treats a reserve at or above the free memory as no limit).
    #[test]
    fn vram_reserve_scales_with_the_card() {
        assert_eq!(vram_reserves(16.0), (2, 4));
        assert_eq!(vram_reserves(12.0), (2, 3));
        assert_eq!(vram_reserves(8.0), (1, 2));
        assert_eq!(vram_reserves(6.0), (0, 1));
        assert_eq!(vram_reserves(4.0), (0, 1));
        assert_eq!(vram_reserves(2.0), (0, 0));
        assert_eq!(vram_reserves(0.0), (1, 2), "unknown card");
        for gb in [2.0_f32, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0] {
            let (r, m) = vram_reserves(gb);
            assert!(r <= m && f32::from(m) <= gb / 4.0, "{gb}");
        }
        // A 4 GB card launches without a budget; the retry adds one.
        let wiring = v(&["--diffusion-model", "/d"]);
        let fb = MemFallback {
            more_room_gib: 1,
            ..Default::default()
        };
        let args = with_memory_choices(&wiring, fb);
        assert!(!args.iter().any(|a| a == "--max-vram"));
        let (fb, note) = next_memory_fallback(
            fb,
            Stage::Diffusion,
            TeChoice::Auto,
            true,
            &args,
            true,
            true,
        )
        .unwrap();
        assert_eq!((fb.vram_reserve_gib, note), (1, MORE_ROOM_RETRY_NOTE));
        let args = with_memory_choices(&wiring, fb);
        assert_eq!(flag_value(&args, "--max-vram").as_deref(), Some("-1"));
    }

    #[test]
    fn offload_needs_room_in_system_memory() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("m.gguf");
        let t = dir.path().join("t.safetensors");
        std::fs::write(&f, vec![0u8; 3 << 20]).unwrap();
        std::fs::write(&t, vec![0u8; 1 << 20]).unwrap();
        let args = v(&[
            "--diffusion-model",
            f.to_str().unwrap(),
            "--t5xxl",
            t.to_str().unwrap(),
            "--listen-ip",
            "127.0.0.1",
        ]);
        assert!((weights_gb(&args) - 4.0 / 1024.0).abs() < 1e-9);
        assert!(offload_fits_ram(&args, 2.01));
        assert!(
            !offload_fits_ram(&args, 2.0),
            "4 MB of weights + 2 GB spare > 2 GB"
        );
        assert!(offload_fits_ram(&args, 0.0), "unknown RAM");
    }

    #[test]
    fn other_programs_are_named_with_their_memory() {
        let p = |pid, name: &str, mib| pinhole_hardware::GpuProcess {
            pid,
            name: name.into(),
            used_mib: mib,
        };
        let o = OtherGpuUse {
            gpu_index: 0,
            total_mib: 16275,
            others_mib: 9216,
            processes: vec![p(7, "python.exe", Some(9114))],
        };
        assert_eq!(
            others_sentence(&o),
            "Other programs are using 9 GB of your graphics memory: python.exe (8.9 GB)."
        );
        // WDDM: no per-process numbers; repeated names are listed once; at most three.
        let o = OtherGpuUse {
            others_mib: 10854,
            processes: vec![
                p(1, "python.exe", None),
                p(2, "python.exe", None),
                p(3, "obs64.exe", None),
                p(4, "a.exe", None),
                p(5, "b.exe", None),
            ],
            ..o
        };
        assert_eq!(others_sentence(&o), "Other programs are using 10.6 GB of your graphics memory: python.exe, obs64.exe, a.exe.");
        let o = OtherGpuUse {
            processes: vec![],
            ..o
        };
        assert_eq!(
            others_sentence(&o),
            "Other programs are using 10.6 GB of your graphics memory."
        );
        assert!(others_note(&o).ends_with("If pictures fail, close them and try again."));
    }

    /// The UI shows "Open Models" on a `vram` error only when its message points there
    /// (src/components/ErrorWithFix.tsx matches "in Models").
    #[test]
    fn vram_messages_point_to_models_except_the_settings_one() {
        for m in [VRAM_MESSAGE, RAM_MESSAGE] {
            assert!(m.contains("in Models"), "{m}");
        }
        assert!(!TE_ON_GPU_MESSAGE.contains("in Models"));
        // `vram_message` with other programs named: see testing.rs `out_of_memory_is_never_the_generic_message`.
    }

    #[test]
    fn result_image_serializes_camel_case() {
        let r = ResultImage {
            id: "a".into(),
            kind: ResultKind::Upscaled,
            width: 1,
            height: 2,
            seed: 3,
            model_id: "m".into(),
            model_label: "M".into(),
            family_id: "f".into(),
            steps: 4,
            cfg: 1.0,
            guidance: None,
            sampler: None,
            scheduler: None,
            parent_id: None,
            origin: Origin::Generated,
            base_size: Some((1, 2)),
        };
        let v = serde_json::to_value(&r).unwrap();
        for k in [
            "id",
            "kind",
            "width",
            "height",
            "seed",
            "modelId",
            "modelLabel",
            "familyId",
            "steps",
            "cfg",
            "guidance",
            "sampler",
            "scheduler",
            "parentId",
        ] {
            assert!(v.get(k).is_some(), "{k}");
        }
        assert_eq!(v["kind"], "upscaled");
        assert_eq!(
            serde_json::to_value(ResultKind::Generated).unwrap(),
            "generated"
        );
    }
}
