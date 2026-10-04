//! Describe (img2text) via llama-server.
//!
//! Captioner files: reuse the edit model's Qwen2.5-VL-7B + mmproj when both are
//! installed (`captioner.prefer_reuse`), else the small default captioner
//! (`captioner.default`, component ids [`DEFAULT_MODEL_ID`] /
//! [`DEFAULT_MMPROJ_ID`], kind `Captioner`). llama-server starts on demand on
//! 127.0.0.1 and is stopped after `captioner.idle_shutdown_seconds` (60 s) idle.
//! The only text sent is the fixed registry instruction (`captioner.prompts`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use pinhole_engine::failure::{classify, Failure};
use pinhole_engine::install::EngineKind;
use pinhole_engine::llama::{self, LlamaClient};
use pinhole_engine::logbuf::LogBuffer;
use pinhole_engine::process::{free_port, EngineProcess, ReadyError};
use pinhole_net::download::DownloadSpec;
use pinhole_registry::{Family, ImproveSpec};
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
    /// Describes in flight (they can overlap): llama-server is only stopped at 0.
    pub(crate) busy: AtomicUsize,
    /// Cancelled by [`shutdown`] so a model load (which holds `slot`) ends
    /// right away; replaced by a fresh token once `shutdown` has the slot.
    pub(crate) stopping: parking_lot::Mutex<tokio_util::sync::CancellationToken>,
    pub(crate) logs: Arc<LogBuffer>,
    /// Last background install failure (unpack), shown on the next describe.
    pub(crate) last_error: parking_lot::Mutex<Option<String>>,
    /// Tests: an already-running (mock) llama-server.
    pub(crate) external: parking_lot::Mutex<Option<String>>,
}

impl DescribeState {
    /// Whether a describe is running right now.
    pub(crate) fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst) > 0
    }
}

/// Counts one describe as in flight while it lives (also when its future is dropped).
struct BusyGuard<'a>(&'a DescribeState);

impl<'a> BusyGuard<'a> {
    fn new(state: &'a DescribeState) -> Self {
        state.busy.fetch_add(1, Ordering::SeqCst);
        Self(state)
    }
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        *self.0.last_used.lock() = Instant::now();
        self.0.busy.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Default for DescribeState {
    fn default() -> Self {
        Self {
            slot: tokio::sync::Mutex::new(None),
            install_lock: tokio::sync::Mutex::new(()),
            last_used: parking_lot::Mutex::new(Instant::now()),
            busy: AtomicUsize::new(0),
            stopping: parking_lot::Mutex::new(tokio_util::sync::CancellationToken::new()),
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

/// Which setting picks the helper model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Describe,
    Improve,
}

impl Purpose {
    /// `describe` | `improve` (anything else reads as Describe).
    pub fn parse(s: &str) -> Self {
        if s == "improve" {
            Purpose::Improve
        } else {
            Purpose::Describe
        }
    }
}

fn component_path(core: &AppCore, component: &str) -> Option<PathBuf> {
    let idx = core.installed.lock();
    let f = idx.find_component(component)?;
    let p = idx.abs_path(&core.data, f);
    p.is_file().then_some(p)
}

/// (model, vision projector) component ids of a helper.
pub(crate) fn helper_components(spec: &pinhole_registry::model::HelperSpec) -> (String, String) {
    if spec.default {
        (DEFAULT_MODEL_ID.into(), DEFAULT_MMPROJ_ID.into())
    } else {
        (
            spec.components.first().cloned().unwrap_or_default(),
            spec.components.get(1).cloned().unwrap_or_default(),
        )
    }
}

/// Helpers that may be offered now (Safe mode hides `needs_safe_off` ones).
pub(crate) fn offered_helpers(core: &AppCore) -> Vec<pinhole_registry::model::HelperSpec> {
    let safe = core.settings.read().content_mode != "all";
    core.registry()
        .captioner()
        .helpers
        .iter()
        .filter(|h| !(safe && h.needs_safe_off))
        .cloned()
        .collect()
}

/// The helper picked in Settings / the pickers for `purpose`, when it is offered and installed;
/// `None` = automatic (a removed or unknown choice reads as automatic too).
fn chosen_helper(core: &AppCore, purpose: Purpose) -> Option<String> {
    let id = {
        let s = core.settings.read();
        match purpose {
            Purpose::Describe => s.describe_model.clone(),
            Purpose::Improve => s.improve_model.clone(),
        }
    };
    if id == "auto" {
        return None;
    }
    let spec = offered_helpers(core).into_iter().find(|h| h.id == id)?;
    let (m, p) = helper_components(&spec);
    (component_path(core, &m).is_some() && component_path(core, &p).is_some()).then_some(id)
}

/// Installed captioner files (model, mmproj): the chosen helper's, else reuse preferred.
fn captioner_files(core: &AppCore, helper: Option<&str>) -> Option<(Source, PathBuf, PathBuf)> {
    if let Some(id) = helper {
        let spec = offered_helpers(core).into_iter().find(|h| h.id == id)?;
        let (m, p) = helper_components(&spec);
        let source = if spec.default {
            Source::Default
        } else {
            Source::Reuse
        };
        return Some((source, component_path(core, &m)?, component_path(core, &p)?));
    }
    // Automatic while Safe mode is Off: an installed Safe-mode-Off helper first (offered_helpers
    // drops them while Safe mode is On).
    for spec in offered_helpers(core)
        .into_iter()
        .filter(|h| h.needs_safe_off)
    {
        let (m, p) = helper_components(&spec);
        if let (Some(m), Some(p)) = (component_path(core, &m), component_path(core, &p)) {
            return Some((Source::Reuse, m, p));
        }
    }
    let reg = core.registry();
    let reuse = &reg.captioner().prefer_reuse;
    if reuse.len() >= 2 {
        if let (Some(m), Some(p)) = (
            component_path(core, &reuse[0]),
            component_path(core, &reuse[1]),
        ) {
            return Some((Source::Reuse, m, p));
        }
    }
    match (
        component_path(core, DEFAULT_MODEL_ID),
        component_path(core, DEFAULT_MMPROJ_ID),
    ) {
        (Some(m), Some(p)) => Some((Source::Default, m, p)),
        _ => None,
    }
}

fn verified(sha: &str) -> Option<String> {
    let s = sha.trim().to_ascii_lowercase();
    (s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())).then_some(s)
}

/// What `install_captioner` would download: (spec, role).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    /// A model file, registered under this component id with this name.
    File {
        component: String,
        name: String,
    },
    Engine,
}

fn missing_parts(core: &AppCore, helper: Option<&str>) -> CoreResult<Vec<(DownloadSpec, Part)>> {
    let mut out = Vec::new();
    let reg = core.registry();
    let spec = match helper {
        Some(id) => Some(
            offered_helpers(core)
                .into_iter()
                .find(|h| h.id == id)
                .ok_or_else(|| {
                    CoreError::not_found(
                        "That helper model isn't in Pinhole's list. Update Pinhole.",
                    )
                })?,
        ),
        None => None,
    };
    // (component id, file, label) of the files this install needs.
    let wanted: Vec<(String, pinhole_registry::model::CaptionerFile, String)> = match &spec {
        Some(h) if !h.default => {
            let mut v = Vec::new();
            for (i, c) in h.components.iter().enumerate() {
                let comp = reg.component(c).ok_or_else(|| {
                    CoreError::not_found(
                        "That helper model isn't in Pinhole's list. Update Pinhole.",
                    )
                })?;
                v.push((
                    c.clone(),
                    pinhole_registry::model::CaptionerFile {
                        file: comp.file.clone(),
                        url: comp.url.clone(),
                        sha256: comp.sha256.clone(),
                        size_mb: comp.size_mb,
                    },
                    if i == 0 {
                        h.title.clone()
                    } else {
                        format!("{} (vision)", h.title)
                    },
                ));
            }
            v
        }
        _ => {
            if spec.is_none() && captioner_files(core, None).is_some() {
                Vec::new()
            } else {
                let def = reg.captioner().default.clone().ok_or_else(|| {
                    CoreError::not_found(
                        "No describe model is listed in Pinhole's model list. Update Pinhole.",
                    )
                })?;
                vec![
                    (
                        DEFAULT_MODEL_ID.to_string(),
                        def.model,
                        "Describe model".to_string(),
                    ),
                    (
                        DEFAULT_MMPROJ_ID.to_string(),
                        def.mmproj,
                        "Describe model (vision)".to_string(),
                    ),
                ]
            }
        }
    };
    if !wanted.is_empty() {
        let dir = core.data.models(ModelKind::Captioner);
        for (comp, file, label) in wanted {
            if component_path(core, &comp).is_some() {
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
                    label: label.clone(),
                    ..Default::default()
                },
                Part::File {
                    component: comp,
                    name: label,
                },
            ));
        }
    }
    if let Some((specs, _, _)) = engine_setup::engine_download_specs(core, EngineKind::Llama)? {
        out.extend(specs.into_iter().map(|s| (s, Part::Engine)));
    }
    Ok(out)
}

// ---------------------------------------------------------------- status / install

pub fn captioner_status(core: &AppCore, purpose: Purpose) -> CaptionerStatus {
    let external = core.describe.external.lock().is_some();
    let helper = chosen_helper(core, purpose);
    let files = captioner_files(core, helper.as_deref());
    let running = match core.describe.slot.try_lock() {
        Ok(mut g) => g.as_mut().map(|s| s.proc.is_running()).unwrap_or(false),
        Err(_) => true, // busy starting / describing
    } || external;
    if external {
        return CaptionerStatus {
            available: true,
            source: Some("default".into()),
            download_bytes: 0,
            running,
        };
    }
    let download_bytes = missing_parts(core, helper.as_deref())
        .map(|p| p.iter().filter_map(|(s, _)| s.size_hint()).sum())
        .unwrap_or(0);
    let engine_ok = engine_setup::installed_engine(core, EngineKind::Llama).is_some();
    CaptionerStatus {
        available: files.is_some() && engine_ok,
        source: files.map(|f| f.0.key().to_string()),
        download_bytes,
        running,
    }
}

/// Queue the default captioner (+ the llama.cpp engine if missing) as one
/// download group; files are registered / unpacked when it finishes.
/// `helper`: a `captioner.helpers` id, or `None` = the default one.
pub async fn install_captioner(
    core: &Arc<AppCore>,
    helper: Option<&str>,
) -> CoreResult<InstallStarted> {
    let explicit = matches!(
        core.settings.read().engine_backend.as_str(),
        "cuda" | "vulkan" | "cpu"
    );
    if !explicit && core.hardware.read().is_none() {
        crate::app::wait_for_hardware(core, Duration::from_secs(30)).await;
    }
    let parts = missing_parts(core, helper)?;
    // RELEASE-SPEC §6: the helper's licence is accepted once before its files download.
    if parts.iter().any(|(_, p)| matches!(p, Part::File { .. })) {
        let spec = offered_helpers(core)
            .into_iter()
            .find(|h| helper.map_or(h.default, |id| h.id == id));
        if let Some(h) = spec {
            crate::licence::require(core, h.license_accept.as_deref(), h.license_note.as_deref())?;
        }
    }
    let (specs, roles): (Vec<DownloadSpec>, Vec<Part>) = parts.into_iter().unzip();
    crate::models::models_dir_for_write(core, ModelKind::Captioner)?;
    let group_id = core.downloads.enqueue_kind(
        "Describe model".into(),
        pinhole_net::download::DownloadKind::Captioner,
        specs,
    );
    *core.describe.last_error.lock() = None;
    let core2 = core.clone();
    let gid = group_id.clone();
    tokio::spawn(async move {
        let _guard = core2.describe.install_lock.lock().await;
        let Ok(files) = core2.downloads.wait_detailed(&gid).await else {
            return;
        };
        let mut engine_files = Vec::new();
        for (file, role) in files.into_iter().zip(roles) {
            let reg = match role {
                Part::Engine => {
                    engine_files.push(file);
                    continue;
                }
                Part::File { component, name } => Registration {
                    kind: ModelKind::Captioner,
                    friendly_name: name,
                    family: None,
                    component_id: Some(component),
                    civitai: None,
                    dtype: None,
                    lookup: None,
                },
            };
            if let Err(e) = register_download(&core2, &file, reg) {
                core2.downloads.fail_done(&gid, &e.code, &e.message);
                *core2.describe.last_error.lock() = Some(e.message);
            }
        }
        if !engine_files.is_empty() {
            let res = match engine_setup::selected_build(&core2, EngineKind::Llama) {
                Ok((cfg, sel)) => engine_setup::unpack_downloaded(
                    &core2,
                    EngineKind::Llama,
                    &cfg,
                    &sel,
                    engine_files,
                )
                .await
                .map(|_| ()),
                Err(e) => Err(e),
            };
            if let Err(e) = res {
                core2.downloads.fail_done(&gid, &e.code, &e.message);
                *core2.describe.last_error.lock() = Some(e.message);
            }
        }
        core2.emit(crate::CoreEvent::ModelsChanged);
    });
    Ok(InstallStarted { group_id })
}

// ---------------------------------------------------------------- helper models

/// `HelperModel` in src/lib/types.ts: one row of Models → Helpers and the pickers.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HelperModel {
    pub id: String,
    pub title: String,
    pub note: String,
    /// Both files together (installed size, else the registry's rounded estimate).
    pub size_bytes: u64,
    /// Bytes still to download to use it (0 when installed).
    pub download_bytes: u64,
    pub installed: bool,
    /// Pinhole downloaded it as a helper, so Remove is offered (an encoder that came with
    /// Qwen Image Edit belongs to that model).
    pub removable: bool,
    pub fit: Option<pinhole_registry::vram::Fit>,
    pub needs_safe_off: bool,
}

/// Helper models for Models → Helpers and the pickers, in registry order.
pub fn list_helper_models(core: &AppCore) -> Vec<HelperModel> {
    let reg = core.registry();
    let hw = crate::app::hw_context(core);
    let index = core.installed.lock().clone();
    offered_helpers(core)
        .into_iter()
        .map(|h| {
            let (m, p) = helper_components(&h);
            let installed =
                component_path(core, &m).is_some() && component_path(core, &p).is_some();
            let estimate: u64 = if h.default {
                reg.captioner()
                    .default
                    .as_ref()
                    .map(|d| (d.model.size_mb + d.mmproj.size_mb) * 1_000_000)
                    .unwrap_or(0)
            } else {
                h.components
                    .iter()
                    .filter_map(|c| reg.component(c))
                    .map(|c| c.size_mb * 1_000_000)
                    .sum()
            };
            let on_disk: u64 = [&m, &p]
                .iter()
                .filter_map(|c| index.find_component(c))
                .map(|f| f.size_bytes)
                .sum();
            let size_bytes = if installed && on_disk > 0 {
                on_disk
            } else {
                estimate
            };
            // The same rule as Remove itself (a vision file another helper still uses stays).
            let removable = !crate::models::helper_files(core, &index, &h.id).is_empty();
            let download_bytes = if installed {
                0
            } else {
                missing_parts(core, Some(&h.id))
                    .map(|parts| {
                        parts
                            .iter()
                            .filter(|(_, r)| !matches!(r, Part::Engine))
                            .filter_map(|(s, _)| s.size_hint())
                            .sum()
                    })
                    .unwrap_or(estimate)
            };
            HelperModel {
                id: h.id.clone(),
                title: h.title.clone(),
                note: h.note.clone(),
                size_bytes,
                download_bytes,
                installed,
                removable,
                fit: helper_fit(size_bytes, hw.vram_gb, hw.ram_gb),
                needs_safe_off: h.needs_safe_off,
            }
        })
        .collect()
}

/// Fits / Tight (some layers run on the processor, slower) / Too big (more than the RAM).
/// `None` without a graphics card or before hardware is known.
fn helper_fit(size_bytes: u64, vram_gb: f32, ram_gb: f32) -> Option<pinhole_registry::vram::Fit> {
    use pinhole_registry::vram::Fit;
    if vram_gb.is_nan() || vram_gb <= 0.0 {
        return None;
    }
    let need_gb = size_bytes as f32 / 1e9 * 1.15 + 0.5;
    Some(
        if ram_gb > 0.0 && size_bytes as f32 / 1e9 * 1.1 > ram_gb + vram_gb {
            Fit::TooBig
        } else if need_gb <= vram_gb {
            Fit::Fits
        } else {
            Fit::Tight
        },
    )
}

// ---------------------------------------------------------------- describe

/// Describe a session image as a prompt (`sentence`) or booru tags (`tags`).
pub async fn describe_image(
    core: &Arc<AppCore>,
    image_id: &str,
    style: DescribeStyle,
) -> CoreResult<String> {
    // Held for the whole run: the Models folder can't move under the engine.
    let _folder = crate::models::folder_read(core)?;
    let img = core.session.get(image_id).ok_or_else(|| {
        CoreError::not_found("That image isn't in this session anymore. Add it again.")
    })?;
    crate::imagecheck::check_before_describe(core, &img).await?;
    let reg = core.registry();
    let instruction = reg
        .captioner()
        .prompts
        .get(style.key())
        .cloned()
        .ok_or_else(|| {
            CoreError::not_found("This describe style isn't available. Update Pinhole.")
        })?;

    // Shrink very large images (the VLM downsamples anyway).
    let (bytes, mime): (Vec<u8>, &str) = if img.width.max(img.height) > MAX_SIDE {
        let src = img.bytes.clone();
        let png = tokio::task::spawn_blocking(move || -> CoreResult<Vec<u8>> {
            let (mut rgba, mut w, mut h) =
                pinhole_engine::image::decode_rgba(&src).map_err(|e| {
                    CoreError::invalid("This image couldn't be decoded.")
                        .with_details(e.to_string())
                })?;
            while w.max(h) > MAX_SIDE {
                (rgba, w, h) = pinhole_engine::image::downscale_2x_box(&rgba, w, h);
            }
            pinhole_engine::image::encode_png_rgba(&rgba, w, h).map_err(|e| {
                CoreError::internal("Couldn't prepare the image.").with_details(e.to_string())
            })
        })
        .await
        .map_err(|e| {
            CoreError::internal("Couldn't prepare the image.").with_details(e.to_string())
        })??;
        (png, "image/png")
    } else {
        (img.bytes.as_ref().clone(), img.kind.mime())
    };

    let _busy = BusyGuard::new(&core.describe);
    let helper = chosen_helper(core, Purpose::Describe);
    let text = describe_inner(core, &instruction, mime, &bytes, style, helper).await?;
    crate::text_check::check(&text)?;
    Ok(text)
}

async fn describe_inner(
    core: &Arc<AppCore>,
    instruction: &str,
    mime: &str,
    bytes: &[u8],
    style: DescribeStyle,
    helper: Option<String>,
) -> CoreResult<String> {
    let client = ensure_llama(core, helper.as_deref()).await?;
    let max_tokens = if style == DescribeStyle::Tags {
        200
    } else {
        300
    };
    let text = client.describe(instruction, mime, bytes, max_tokens).await.map_err(|e| {
        let tail = core.describe.logs.tail_text(30);
        match classify(&tail, None) {
            Failure::OutOfMemory => CoreError::new("vram", "Not enough memory to describe right now — close other apps or wait for the image to finish, then try again.").with_details(tail),
            _ => CoreError::new("engine_failed", "Describing the image failed. Try again.").with_details(format!("{e}\n{tail}")),
        }
    })?;
    if text.is_empty() {
        return Err(CoreError::new(
            "engine_failed",
            "The describe model returned nothing. Try again.",
        ));
    }
    Ok(text)
}

/// Longest prompt "Improve my prompt" accepts (characters).
const IMPROVE_MAX_CHARS: usize = 2000;

/// The six lines of the Improve form, in the order the helper writes them: the idea reworded,
/// then what it adds. The names after `prompt` are the keys of `captioner.improve.given`.
const FORM_LINES: [&str; 6] = ["prompt", "details", "place", "shot", "style", "light"];

/// The three lines of the Edit form (`captioner.improve.edit`): the instruction reworded, how the
/// change looks, what stays.
const EDIT_FORM_LINES: [&str; 3] = ["change", "details", "keep"];

/// Longest reworded idea (words) Improve uses; a longer one is a description of its own.
const MAX_REWORDED_WORDS: usize = 40;

/// Number words a reworded idea has to keep.
const NUMBER_WORDS: [&str; 12] = [
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "single", "pair",
];

/// Negation words: when the idea has one, the reworded idea needs one too.
const NEGATIONS: [&str; 8] = [
    "no", "not", "without", "never", "don't", "dont", "isn't", "nothing",
];

/// Most things the Edit form's "Keep … unchanged" names.
const MAX_KEPT: usize = 3;

/// What "Improve my prompt" works on: a Create prompt or an Edit change instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImproveTarget {
    Create,
    Edit,
}

/// Most phrases Improve adds after the user's words in the tag styles.
const MAX_ADDED_TAGS: usize = 12;

/// Longest phrase (words) Improve adds; longer ones are usually sentences about the picture.
const MAX_PHRASE_WORDS: usize = 6;

/// The Improve style for a family: its `improve_style`, else its `style_template`; a style the
/// registry has no form for reads as `natural`.
fn improve_style(spec: &ImproveSpec, family: Option<&Family>) -> String {
    let wanted = family.map(|f| {
        f.improve_style
            .clone()
            .unwrap_or_else(|| f.style_template.clone())
    });
    match wanted {
        Some(w) if spec.styles.contains_key(&w) => w,
        _ => "natural".into(),
    }
}

/// The instruction for "Improve my prompt": the form filled in for the style, the Safe mode
/// rule (`safe` while On, `adult` while Off), and the add-on trigger words not to repeat.
fn improve_instruction(
    spec: &ImproveSpec,
    style: &str,
    safe: bool,
    avoid: &[String],
) -> Option<String> {
    let st = spec.styles.get(style)?;
    if spec.form.trim().is_empty() {
        return None;
    }
    let out = spec
        .form
        .trim()
        .replace("{format}", st.format.trim())
        .replace("{style_examples}", st.style_examples.trim())
        .replace("{example}", st.example.trim());
    Some(with_rules(out, spec, safe, avoid))
}

/// The instruction for Improve in Edit: `captioner.improve.edit.form` with the same Safe mode
/// rule and trigger words as [`improve_instruction`].
fn edit_instruction(spec: &ImproveSpec, safe: bool, avoid: &[String]) -> Option<String> {
    let form = spec.edit.form.trim();
    (!form.is_empty()).then(|| with_rules(form.to_string(), spec, safe, avoid))
}

/// `out` with the Safe mode rule (`safe` while On, `adult` while Off) and the add-on trigger
/// words not to repeat.
fn with_rules(mut out: String, spec: &ImproveSpec, safe: bool, avoid: &[String]) -> String {
    let rule = if safe { &spec.safe } else { &spec.adult };
    if !rule.trim().is_empty() {
        out = format!("{out} {}", rule.trim());
    }
    if !avoid.is_empty() && !spec.avoid.trim().is_empty() {
        out = format!(
            "{out} {}",
            spec.avoid.trim().replace("{words}", &avoid.join(", "))
        );
    }
    out
}

/// The helper's answer as the lines of a form (`names`), in form order. Bullets, numbers and a leading
/// `NAME:` label are taken off, and `-` leaves a line empty. A line labelled with a form line's
/// name goes there; small helpers often drop or rename the labels, so any other line takes the
/// next place. A line that ends in `:` with nothing after it ("Here are the lines:") is skipped.
fn form_lines(answer: &str, names: &[&str]) -> Vec<String> {
    let mut out = vec![String::new(); names.len()];
    let mut next = 0;
    for line in llama::clean_caption(answer).lines() {
        let mut t = line.trim();
        loop {
            let before = t;
            t = t.trim_start_matches(['*', '\u{2022}']).trim_start();
            if let Some(rest) = t.strip_prefix("- ") {
                t = rest.trim_start();
            }
            let digits = t.len() - t.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            if digits > 0 {
                let rest = &t[digits..];
                if let Some(r) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
                    t = r.trim_start();
                }
            }
            if t == before {
                break;
            }
        }
        if t.is_empty() {
            continue;
        }
        let mut slot = None;
        if let Some((label, rest)) = t.split_once(':') {
            let label = label.trim_end_matches('*').trim();
            let rest = rest.trim_start_matches('*').trim();
            let rest = rest.strip_prefix("- ").unwrap_or(rest).trim();
            if rest.is_empty() {
                continue;
            }
            if !label.is_empty()
                && label.len() <= 20
                && label.chars().all(|c| c.is_alphabetic() || c == ' ')
            {
                slot = names.iter().position(|n| n.eq_ignore_ascii_case(label));
                t = rest;
            }
        }
        let i = slot.unwrap_or(next);
        if i >= names.len() {
            break;
        }
        next = i + 1;
        let t = t
            .trim()
            .trim_matches(|c| c == '"' || c == '*')
            .trim()
            .trim_end_matches(['.', ','])
            .trim();
        if !(t == "-" || t.eq_ignore_ascii_case("none") || t.eq_ignore_ascii_case("n/a")) {
            out[i] = t.to_string();
        }
    }
    out
}

/// Lowercase words of three letters or more.
fn content_words(text: &str) -> Vec<String> {
    lower_words(text)
        .into_iter()
        .filter(|w| w.chars().count() >= 3)
        .collect()
}

/// Lowercase words (letters, digits and apostrophes; a curly apostrophe counts as `'`).
fn lower_words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('\u{2019}', "'")
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether `reworded` keeps the intent of `idea`. Never for an idea with an `edit.keep_wording`
/// phrase ("the bottle from image 2"). Otherwise it is at most [`MAX_REWORDED_WORDS`] words and:
/// - keeps at least four in five of the idea's words (`common_words` aside; singular and plural
///   count as one, "1girl" counts as "girl"), every number and `exact_words` word ("more",
///   "her", "left") it has, and every `given` phrase it names (a view, a medium, a light, "keep",
///   "same");
/// - keeps each negation with the word it negates ("not a dog" stays about the dog);
/// - adds no number, no person when the idea has none (`people`), and no `drop` or add-on
///   trigger word;
/// - with `actions` (groups of edit verbs that make the same kind of change), adds no verb from
///   a group the idea has none of ("add a hat" is not "replace the background"); an idea with no
///   verb may get one, but not one of `edit.never_added` ("remove", "replace").
fn keeps_intent(
    idea: &str,
    reworded: &str,
    spec: &ImproveSpec,
    avoid: &[String],
    actions: &[Vec<String>],
) -> bool {
    let r = reworded.trim();
    let in_idea = |p: &String| crate::generate::contains_phrase(idea, p);
    let in_new = |p: &String| crate::generate::contains_phrase(r, p);
    if spec.edit.keep_wording.iter().any(in_idea)
        || r.is_empty()
        || r == "-"
        || r.split_whitespace().count() > MAX_REWORDED_WORDS
    {
        return false;
    }
    let (idea_all, new_all) = (lower_words(idea), lower_words(r));
    // "1girl" → "girl", "2boys" → "boy".
    let base = |w: &str| singular(w.trim_start_matches(|c: char| c.is_ascii_digit()));
    let new_set: std::collections::HashSet<String> = new_all
        .iter()
        .flat_map(|w| [singular(w), base(w)])
        .collect();
    let is_number = |w: &str| w.chars().all(|c| c.is_ascii_digit()) || NUMBER_WORDS.contains(&w);
    let key: Vec<String> = idea_all
        .iter()
        .filter(|w| w.chars().count() >= 3 || is_number(w))
        .filter(|w| !spec.common_words.contains(*w) || spec.exact_words.contains(*w))
        .map(|w| singular(w))
        .collect();
    let missing: Vec<&String> = key.iter().filter(|w| !new_set.contains(*w)).collect();
    // One word in five may go ("cat astronaut" → "a cat in a spacesuit" is still the idea, "a
    // red car in the rain" without the rain is not).
    let must_keep = |w: &str| is_number(w) || spec.exact_words.iter().any(|e| e == w);
    if missing.iter().any(|w| must_keep(w)) || missing.len() * 5 > key.len() {
        return false;
    }
    let idea_numbers: std::collections::HashSet<&String> =
        idea_all.iter().filter(|w| is_number(w)).collect();
    if new_all
        .iter()
        .any(|w| is_number(w) && !idea_numbers.contains(w))
    {
        return false;
    }
    // Each negation and the next word that isn't an article: "not a dog" → "dog".
    let negated = |ws: &[String]| -> Vec<String> {
        ws.iter()
            .enumerate()
            .filter(|(_, w)| NEGATIONS.contains(&w.as_str()))
            .filter_map(|(i, _)| {
                ws[i + 1..]
                    .iter()
                    .find(|w| !matches!(w.as_str(), "a" | "an" | "the" | "any"))
                    .map(|w| singular(w))
            })
            .collect()
    };
    let (idea_negated, new_negated) = (negated(&idea_all), negated(&new_all));
    if idea_negated.iter().any(|w| !new_negated.contains(w))
        || new_negated.iter().any(|w| !idea_negated.contains(w))
    {
        return false;
    }
    let named_but_lost = spec
        .given
        .values()
        .chain(spec.edit.given.values())
        .flatten()
        .any(|p| in_idea(p) && !in_new(p));
    if named_but_lost {
        return false;
    }
    let idea_has_verb = actions.iter().flatten().any(in_idea);
    let new_verb_kind = actions
        .iter()
        .any(|group| group.iter().any(in_new) && !group.iter().any(in_idea));
    let forbidden_verb = !idea_has_verb && spec.edit.never_added.iter().any(in_new);
    if (idea_has_verb && new_verb_kind) || (!actions.is_empty() && forbidden_verb) {
        return false;
    }
    let has_person = names_person(idea, &spec.people);
    let added_person = !has_person
        && (spec.people.iter().any(in_new)
            || new_person_details(idea, has_person, &spec.person_details)
                .into_iter()
                .any(in_new));
    !added_person
        && !spec
            .drop
            .iter()
            .chain(avoid)
            .any(|d| in_new(d) && !in_idea(d))
}

/// Whether `text` names a person ("man", "1girl", "two women"; `people` is the list of such words).
fn names_person(text: &str, people: &[String]) -> bool {
    let words = lower_words(text);
    let one: Vec<String> = words
        .iter()
        .map(|w| singular(w.trim_start_matches(|c: char| c.is_ascii_digit())))
        .collect();
    people.iter().any(|p| {
        let p = p.to_lowercase();
        if p.contains(' ') {
            crate::generate::contains_phrase(text, &p)
        } else {
            words.contains(&p) || one.contains(&singular(&p))
        }
    })
}

/// The `person_details` words ("hair", "dress") that `idea` doesn't have, when it names no person
/// (`has_person`); none when it does.
fn new_person_details<'a>(
    idea: &str,
    has_person: bool,
    person_details: &'a [String],
) -> Vec<&'a String> {
    if has_person {
        return Vec::new();
    }
    person_details
        .iter()
        .filter(|d| !crate::generate::contains_phrase(idea, d))
        .collect()
}

/// The idea Improve builds on: the helper's rewording (`reworded`, trailing full stop off) when it
/// keeps the idea's intent ([`keeps_intent`]), else the idea as typed.
fn base_idea(
    idea: &str,
    reworded: &str,
    spec: &ImproveSpec,
    avoid: &[String],
    actions: &[Vec<String>],
) -> String {
    let r = trimmed(reworded.trim().trim_matches('"'));
    if keeps_intent(idea, r, spec, avoid, actions) {
        r.to_string()
    } else {
        trimmed(idea).to_string()
    }
}

/// The same words, apart from letter case, punctuation and "a", "an", "the": a rewording like
/// that is no improvement on its own.
fn same_words(a: &str, b: &str) -> bool {
    let words = |t: &str| {
        lower_words(t)
            .into_iter()
            .filter(|w| !matches!(w.as_str(), "a" | "an" | "the"))
            .collect::<Vec<_>>()
    };
    words(a) == words(b)
}

/// `text` without a trailing full stop, comma or space.
fn trimmed(text: &str) -> &str {
    text.trim_end_matches(|c: char| c == '.' || c == ',' || c.is_whitespace())
}

/// The DETAILS and PLACE phrases of the form's worked example (lowercase): a helper that copies
/// them describes the example, not the idea.
fn example_phrases(example: &str) -> std::collections::HashSet<String> {
    example
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            l.strip_prefix("DETAILS:")
                .or_else(|| l.strip_prefix("PLACE:"))
        })
        .flat_map(|rest| rest.split(','))
        .map(|p| p.trim().to_lowercase())
        .filter(|p| !p.is_empty() && p != "-")
        .collect()
}

/// The phrases of each form line that add something, by line name. A line the idea already
/// covers (`given`) is left out, and so are phrases with a `skipped` word (mood and quality
/// words, add-on trigger words), phrases naming a person when the idea names none, phrases that say "no …", lines with
/// two or more phrases of the form's example, phrases
/// longer than a few words, phrases made only of the idea's own words, and repeats.
fn added_phrases<'a>(
    idea: &str,
    names: &[&'a str],
    lines: &[String],
    given: &std::collections::BTreeMap<String, Vec<String>>,
    skipped: &[String],
    example: &str,
    people: &[String],
    person_details: &[String],
) -> Vec<(&'a str, Vec<String>)> {
    let copied = example_phrases(example);
    // People the idea doesn't have would change the picture, and so would their hair or clothes.
    let has_person = names_person(idea, people);
    let people: &[String] = if has_person { &[] } else { people };
    let new_details = new_person_details(idea, has_person, person_details);
    let idea_words: std::collections::HashSet<String> = content_words(idea).into_iter().collect();
    let mut seen = std::collections::HashSet::new();
    let mut groups: Vec<(&str, Vec<String>)> = Vec::new();
    for (name, line) in names.iter().zip(lines) {
        let covered = given.get(*name).is_some_and(|phrases| {
            phrases
                .iter()
                .any(|p| crate::generate::contains_phrase(idea, p))
        });
        // A line with two or more of the example's phrases describes the example.
        let from_example = line
            .split(',')
            .filter(|p| copied.contains(&p.trim().trim_end_matches('.').to_lowercase()))
            .count()
            >= 2;
        if covered || from_example {
            continue;
        }
        let items: Vec<String> = line
            .split(',')
            .map(|p| {
                p.trim()
                    .trim_matches(['.', ';', ':', '"'])
                    .trim()
                    .to_string()
            })
            .filter(|p| {
                let words = content_words(p);
                let lower = p.to_lowercase();
                // "no cars" tends to bring cars.
                !p.is_empty()
                    && !lower.starts_with("no ")
                    && !lower.starts_with("without ")
                    && p != "-"
                    && p.split_whitespace().count() <= MAX_PHRASE_WORDS
                    && !names_person(p, people)
                    && !new_details
                        .iter()
                        .any(|d| crate::generate::contains_phrase(p, d))
                    && !skipped
                        .iter()
                        .any(|d| crate::generate::contains_phrase(p, d))
                    && !(!words.is_empty() && words.iter().all(|w| idea_words.contains(w)))
            })
            .filter(|p| seen.insert(p.to_lowercase()))
            .collect();
        if !items.is_empty() {
            groups.push((*name, items));
        }
    }
    groups
}

/// Capital first letter.
fn capitalised(p: &str) -> String {
    let mut c = p.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect::<String>())
        .unwrap_or_default()
}

/// `idea` and the sentences after it, each ending in a full stop. "Wow!" stays "Wow!", not
/// "Wow!.".
fn join_sentences(idea: &str, rest: &[String]) -> String {
    let end = if idea.ends_with(['!', '?']) {
        " "
    } else {
        ". "
    };
    format!("{idea}{end}{}.", rest.join(". "))
}

/// The improved prompt: the idea (reworded when that keeps its intent, see [`base_idea`]), then
/// the form lines that add something (see [`added_phrases`]). `None` when it would be the idea
/// unchanged.
fn assemble_improved(
    idea: &str,
    lines: &[String],
    spec: &ImproveSpec,
    style: &str,
    avoid: &[String],
) -> Option<String> {
    let base = base_idea(
        idea,
        lines.first().map_or("", String::as_str),
        spec,
        avoid,
        &[],
    );
    let both = format!("{idea}\n{base}");
    let groups = added_phrases(
        &both,
        &FORM_LINES[1..],
        lines.get(1..).unwrap_or_default(),
        &spec.given,
        &[spec.drop.as_slice(), avoid].concat(),
        spec.styles.get(style).map_or("", |st| st.example.as_str()),
        &spec.people,
        &spec.person_details,
    );
    if groups.is_empty() {
        return (!same_words(&base, idea)).then_some(base);
    }
    let idea = base.as_str();
    if style == "natural" {
        // Details and place as their own sentences, then shot, style and light together.
        let mut parts = vec![idea.to_string()];
        let mut tail = Vec::new();
        for (name, items) in groups {
            if matches!(name, "details" | "place") {
                parts.push(items.join(", "));
            } else {
                tail.extend(items);
            }
        }
        if !tail.is_empty() {
            parts.push(tail.join(", "));
        }
        let rest: Vec<String> = parts[1..].iter().map(|p| capitalised(p)).collect();
        Some(join_sentences(&parts[0], &rest))
    } else {
        let tags: Vec<String> = groups
            .into_iter()
            .flat_map(|(_, items)| items)
            .map(|t| t.to_lowercase())
            .take(MAX_ADDED_TAGS)
            .collect();
        Some(format!("{idea}, {}", tags.join(", ")))
    }
}

/// A lowercase word in the singular, roughly: "boxes" → "box", "glasses" → "glass",
/// "cars" → "car", "glass" stays.
fn singular(w: &str) -> String {
    let w = w.trim_end_matches("'s");
    if ["sses", "xes", "ches", "shes"]
        .iter()
        .any(|e| w.ends_with(e))
    {
        w[..w.len() - 2].to_string()
    } else if w.ends_with('s') && !w.ends_with("ss") {
        w[..w.len() - 1].to_string()
    } else {
        w.to_string()
    }
}

/// "a", "a and b", "a, b and c".
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The improved edit instruction: the instruction (reworded when that keeps its intent, see
/// [`base_idea`]), then the details of the change as a sentence and "Keep … unchanged." `None`
/// when it would be the instruction unchanged. Besides the rules of [`added_phrases`], a keep
/// phrase that names something the instruction mentions is left out (it is what changes), and
/// so are details that say to keep something or say "no …". At most three things to keep.
fn assemble_edit(
    instruction: &str,
    lines: &[String],
    spec: &ImproveSpec,
    avoid: &[String],
) -> Option<String> {
    let base = base_idea(
        instruction,
        lines.first().map_or("", String::as_str),
        spec,
        avoid,
        &spec.edit.actions,
    );
    let both = format!("{instruction}\n{base}");
    let mut groups = added_phrases(
        &both,
        &EDIT_FORM_LINES[1..],
        lines.get(1..).unwrap_or_default(),
        &spec.edit.given,
        &[spec.drop.as_slice(), avoid].concat(),
        &spec.edit.form,
        &spec.people,
        &spec.person_details,
    );
    let common = &spec.common_words;
    let mentioned: std::collections::HashSet<String> = content_words(&both)
        .iter()
        .filter(|w| !common.contains(*w))
        .map(|w| singular(w))
        .collect();
    // What the instruction keeps: "same bottle", "keep the bottle the same" → "bottle".
    let words = lower_words(instruction);
    let kept_same: std::collections::HashSet<String> = words
        .iter()
        .enumerate()
        .filter(|(_, w)| matches!(w.as_str(), "same" | "keep"))
        .filter_map(|(i, _)| {
            words[i + 1..]
                .iter()
                .find(|w| !matches!(w.as_str(), "a" | "an" | "the" | "same"))
        })
        .map(|w| singular(w))
        .collect();
    for (name, items) in &mut groups {
        if *name == "details" {
            // "same lighting" is not a detail of the change.
            items.retain(|p| {
                let lower = p.to_ascii_lowercase();
                !["same ", "keep ", "maintain ", "preserve "]
                    .iter()
                    .any(|s| lower.starts_with(s))
            });
            // "same bottle, different table": details are about the table, never the bottle.
            if !kept_same.is_empty() {
                items.retain(|p| {
                    content_words(p).iter().map(|w| singular(w)).any(|w| {
                        mentioned.contains(&w)
                            && !kept_same.contains(&w)
                            && !matches!(w.as_str(), "same" | "different" | "keep")
                    })
                });
            }
        } else if *name == "keep" {
            // "keep the table the same" → "the table".
            for p in items.iter_mut() {
                // "the sign (if any)" → "the sign".
                if let (Some(open), Some(close)) = (p.find('('), p.rfind(')')) {
                    if open < close {
                        p.replace_range(open..=close, "");
                        *p = p.split_whitespace().collect::<Vec<_>>().join(" ");
                    }
                }
                // "the same lighting" → "the lighting".
                if p.to_ascii_lowercase().starts_with("the same ") {
                    *p = format!("the {}", &p["the same ".len()..]);
                }
                let lower = p.to_ascii_lowercase();
                let start = ["keep ", "unchanged "]
                    .iter()
                    .find(|s| lower.starts_with(*s))
                    .map_or(0, |s| s.len());
                let end = [
                    " unchanged",
                    " stays the same",
                    " stay the same",
                    " the same",
                    " as it is",
                    " as is",
                    " remains",
                    " remain",
                    " stays",
                    " stay",
                ]
                .iter()
                .find(|e| lower.ends_with(*e))
                .map_or(p.len(), |e| p.len() - e.len());
                *p = p
                    .get(start..end.max(start))
                    .unwrap_or_default()
                    .trim()
                    .to_string();
            }
            // "the man's face" stays for "make the man's shirt blue": the owner word doesn't
            // count.
            items.retain(|p| {
                !p.is_empty()
                    && !content_words(p)
                        .iter()
                        .filter(|w| !w.ends_with("'s"))
                        .any(|w| mentioned.contains(&singular(w)))
            });
            items.truncate(MAX_KEPT);
        }
    }
    groups.retain(|(_, items)| !items.is_empty());
    if groups.is_empty() {
        return (!same_words(&base, instruction)).then_some(base);
    }
    let rest: Vec<String> = groups
        .into_iter()
        .map(|(name, items)| {
            if name == "keep" {
                format!("Keep {} unchanged", and_list(&items))
            } else {
                capitalised(&items.join(", "))
            }
        })
        .collect();
    Some(join_sentences(&base, &rest))
}

/// The helper declined instead of rewriting ("I'm sorry, but I can't…"). Only the start of the
/// answer counts, so a prompt that merely contains "cannot" is kept.
fn is_refusal(text: &str) -> bool {
    let t = text
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
        .replace('\u{2019}', "'");
    [
        "i'm sorry",
        "i am sorry",
        "sorry",
        "my apologies",
        "i apologize",
        "i apologise",
        "i can't",
        "i cannot",
        "i can not",
        "i won't",
        "i will not",
        "i'm unable",
        "i am unable",
        "i'm not able",
        "i am not able",
        "i'm afraid",
        "i must decline",
        "i'm not comfortable",
        "as an ai",
        "unfortunately",
    ]
    .iter()
    // Whole words only: "As an aircraft…" and "sorrowful…" are prompts.
    .any(|p| {
        t.strip_prefix(p)
            .is_some_and(|rest| !rest.starts_with(char::is_alphanumeric))
    })
}

/// `ImprovedPrompt` in src/lib/types.ts. `note` is set when the model's answer was unusable:
/// `text` is then the user's own words, unchanged.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImprovedPrompt {
    pub text: String,
    pub note: Option<String>,
}

/// Turn a short idea into a fuller prompt with the local text model (the Describe model). The
/// helper rewords the idea and fills the form; its wording is used only when it keeps the idea's
/// intent ([`keeps_intent`]), and the form lines are added after it. `family_id` picks the style
/// (sentences, tags or Danbooru tags); `avoid` = trigger words of the add-ons in use. For
/// [`ImproveTarget::Edit`] the words are a change instruction: the helper fills the Edit form
/// and the result is always sentences.
/// PRIVACY: the prompt goes only to the loopback llama-server and back; never logged or stored.
pub async fn improve_prompt(
    core: &Arc<AppCore>,
    prompt: &str,
    family_id: Option<&str>,
    avoid: &[String],
    target: ImproveTarget,
) -> CoreResult<ImprovedPrompt> {
    let idea = prompt.trim();
    if idea.is_empty() {
        return Err(CoreError::invalid(
            "Type a few words about your picture first.",
        ));
    }
    if idea.chars().count() > IMPROVE_MAX_CHARS {
        return Err(CoreError::invalid(
            "That prompt is already long. Improve works on shorter ideas.",
        ));
    }
    crate::text_check::check(idea)?;
    let _folder = crate::models::folder_read(core)?;
    let reg = core.registry();
    let spec = &reg.captioner().improve;
    // Edit is always sentences: every edit model reads natural language.
    let style = match target {
        ImproveTarget::Create => improve_style(spec, family_id.and_then(|id| reg.family(id))),
        ImproveTarget::Edit => "natural".into(),
    };
    let safe = core.settings.read().content_mode != "all";
    let instruction = match target {
        ImproveTarget::Create => improve_instruction(spec, &style, safe, avoid),
        ImproveTarget::Edit => edit_instruction(spec, safe, avoid),
    }
    .ok_or_else(|| CoreError::not_found("Improve my prompt isn't available. Update Pinhole."))?;
    // Six short lines (three for Edit), with the first label written for the helper so it
    // keeps to the form.
    // The first line restates the idea: room for all of it on top of the other lines (a long
    // idea's restatement is not used, but the helper writes it before the other lines).
    let restated = u32::try_from(idea.split_whitespace().count() * 2).unwrap_or(u32::MAX);
    let (max_tokens, answer_start) = match target {
        ImproveTarget::Create => (200 + restated, "PROMPT:"),
        ImproveTarget::Edit => (120 + restated, "CHANGE:"),
    };

    let _busy = BusyGuard::new(&core.describe);
    let client = ensure_llama(core, chosen_helper(core, Purpose::Improve).as_deref()).await?;
    let text = client.rewrite(&instruction, idea, max_tokens, Some(answer_start)).await.map_err(|e| {
        let tail = core.describe.logs.tail_text(30);
        match classify(&tail, None) {
            Failure::OutOfMemory => CoreError::new("vram", "Not enough memory to improve the prompt right now — close other apps or wait for the image to finish, then try again.").with_details(tail),
            _ => CoreError::new("engine_failed", "Improving the prompt failed. Try again.").with_details(format!("{e}\n{tail}")),
        }
    })?;
    // Before the fallbacks: the word check runs on the whole answer, including the parts left
    // out below and refusals, and on the result.
    let answer = text.split_whitespace().collect::<Vec<_>>().join(" ");
    crate::text_check::check(&answer)?;
    // The started label isn't the helper's: "PROMPT: I'm sorry…" is a refusal too.
    let own = answer.strip_prefix(answer_start).unwrap_or(&answer);
    if is_refusal(own) {
        return Ok(ImprovedPrompt {
            text: idea.to_string(),
            note: Some("The helper wouldn't rewrite this one, so your prompt is unchanged.".into()),
        });
    }
    let improved = match target {
        ImproveTarget::Create => {
            assemble_improved(idea, &form_lines(&text, &FORM_LINES), spec, &style, avoid)
        }
        ImproveTarget::Edit => {
            assemble_edit(idea, &form_lines(&text, &EDIT_FORM_LINES), spec, avoid)
        }
    };
    let Some(text) = improved else {
        return Ok(ImprovedPrompt {
            text: idea.to_string(),
            note: Some(
                "The helper couldn't improve this one, so your prompt is unchanged. Try adding a few more words.".into(),
            ),
        });
    };
    crate::text_check::check(&text)?;
    Ok(ImprovedPrompt { text, note: None })
}

/// 32 random bytes as hex: an engine's API key for one launch (llama-server
/// and sd-server).
pub(crate) fn new_api_key() -> String {
    rand::random::<[u8; 32]>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// After `wait_ready`: our child must still be running and the server on the
/// port must report the model we launched it with (`GET /v1/models`, which also
/// needs our API key) — otherwise another program holds the port.
pub(crate) async fn verify_llama_identity(
    proc: &mut EngineProcess,
    client: &LlamaClient,
    model: &std::path::Path,
) -> CoreResult<()> {
    let taken = || CoreError::new("engine_failed", crate::engine::PORT_TAKEN_MESSAGE);
    if !proc.is_running() {
        return Err(taken().with_details(
            "the describe engine exited while another program answered on its port",
        ));
    }
    let ids = client
        .model_ids()
        .await
        .map_err(|e| taken().with_details(format!("model check failed: {e}")))?;
    let expected = model.to_string_lossy();
    if !ids
        .iter()
        .any(|id| crate::engine::same_file_path(id, &expected))
    {
        return Err(taken()
            .with_details("the server on the describe engine's port reports a different model"));
    }
    if !proc.is_running() {
        return Err(taken().with_details("the describe engine exited"));
    }
    Ok(())
}

/// Start llama-server if needed; returns a client for it (with its API key).
async fn ensure_llama(core: &Arc<AppCore>, helper: Option<&str>) -> CoreResult<LlamaClient> {
    if let Some(url) = core.describe.external.lock().clone() {
        return Ok(LlamaClient::new(core.local.clone(), url));
    }
    // A failed install's message explains why its files are missing; once the files and the
    // engine are there (another helper, a retried install) it no longer applies.
    let last_error = || {
        core.describe
            .last_error
            .lock()
            .clone()
            .map(|msg| CoreError::new("engine_failed", msg))
    };
    let (_, model, mmproj) = captioner_files(core, helper).ok_or_else(|| {
        last_error().unwrap_or_else(|| {
            CoreError::not_found(
                "The describe model isn't installed yet. Click Get on the Describe tab.",
            )
        })
    })?;
    let engine = engine_setup::installed_engine(core, EngineKind::Llama).ok_or_else(|| {
        last_error().unwrap_or_else(|| {
            CoreError::not_found(
                "The describe engine isn't installed yet. Click Get on the Describe tab.",
            )
        })
    })?;
    *core.describe.last_error.lock() = None;
    let mut slot = core.describe.slot.lock().await;
    if let Some(s) = slot.as_mut() {
        // Same files AND the same engine build (the backend may have changed in Settings).
        if s.proc.is_running()
            && s.model == model
            && s.mmproj == mmproj
            && s.proc.exe() == engine.exe
        {
            return Ok(LlamaClient::new(core.local.clone(), s.proc.base_url())
                .with_api_key(s.api_key.clone()));
        }
    }
    // Describe and Improve can use different helpers: never stop one that is still answering
    // the other (the caller's own guard counts as one).
    if slot.as_mut().is_some_and(|s| s.proc.is_running())
        && core.describe.busy.load(Ordering::SeqCst) > 1
    {
        return Err(CoreError::invalid(
            "Wait for the other description or prompt to finish, then try again.",
        ));
    }
    // Taken while holding `slot`: a `shutdown` waiting for the slot has cancelled this one.
    let cancel = core.describe.stopping.lock().clone();
    let cancelled = || CoreError::new("cancelled", "Cancelled.");
    if let Some(old) = slot.take() {
        old.proc.stop().await;
    }
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    engine_setup::ensure_runtime(core, &engine)?;
    engine_setup::sweep_orphans(core).await;
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    let cfg = engine_setup::engine_config(core)?;
    let port = free_port().map_err(|e| {
        CoreError::internal("Couldn't find a free local port.").with_details(e.to_string())
    })?;
    // `launch_args` sets the host and port (loopback only).
    let mut args = cfg.llama_cpp.launch_defaults.clone();
    // engine.yaml is editable in some installs: nothing that writes text to disk, prints
    // prompts or loads extra weights.
    crate::engine::strip_flag(
        &mut args,
        &[
            "--host",
            "--port",
            "--log-file",
            "--lora",
            "--lora-scaled",
            "--control-vector",
            "--control-vector-scaled",
            "--slot-save-path",
            "--model",
            "-m",
            "--mmproj",
            "--path",
        ],
    );
    args.retain(|a| {
        !matches!(
            a.as_str(),
            "-v" | "--verbose" | "--log-verbose" | "--verbose-prompt" | "--log-prompts"
        )
    });
    args.extend(llama::launch_args(
        &model,
        &mmproj,
        port,
        &engine.backend,
        CTX_SIZE,
    ));
    core.describe.logs.clear();
    let api_key = new_api_key();
    let mut proc = EngineProcess::spawn_with_env(
        &engine.exe,
        &args,
        &[(llama::API_KEY_ENV, api_key.as_str())],
        port,
        core.describe.logs.clone(),
    )
    .map_err(|e| {
        CoreError::new("engine_failed", "The describe engine couldn't be started.")
            .with_details(e.to_string())
    })?;
    let client =
        LlamaClient::new(core.local.clone(), proc.base_url()).with_api_key(api_key.clone());
    match proc
        .wait_ready(|| client.is_ready(), LOAD_TIMEOUT, &cancel, |_| {})
        .await
    {
        Ok(()) => {
            if let Err(e) = verify_llama_identity(&mut proc, &client, &model).await {
                proc.kill().await;
                return Err(e);
            }
            *slot = Some(LlamaSlot {
                proc,
                model,
                mmproj,
                api_key,
            });
            *core.describe.last_used.lock() = Instant::now();
            Ok(client)
        }
        Err(e) => {
            let code = proc.exit_code();
            proc.stop().await;
            let tail = core.describe.logs.tail_text(30);
            // A cancelled load (its log tail may look like a failure) is just cancelled.
            Err(match (e, classify(&tail, code)) {
                (ReadyError::Cancelled, _) => cancelled(),
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
            if core.describe.is_busy()
                || core.describe.last_used.lock().elapsed() < Duration::from_secs(idle)
            {
                continue;
            }
            let Ok(mut slot) = core.describe.slot.try_lock() else {
                continue;
            };
            if let Some(s) = slot.take() {
                s.proc.stop().await;
            }
        }
    });
}

/// Stop llama-server unless it is describing right now (it holds graphics
/// memory the image engine is about to need). Returns whether it was stopped.
pub(crate) async fn stop_if_idle(core: &AppCore) -> bool {
    if core.describe.is_busy() {
        return false;
    }
    let Ok(mut slot) = core.describe.slot.try_lock() else {
        return false;
    };
    match slot.take() {
        Some(s) => {
            s.proc.stop().await;
            true
        }
        None => false,
    }
}

/// Stop llama-server. A model load in progress is cancelled first (it holds
/// the slot for up to [`LOAD_TIMEOUT`]), so app exit or an update never waits for it.
pub async fn shutdown(core: &AppCore) {
    core.describe.stopping.lock().cancel();
    let mut slot = core.describe.slot.lock().await;
    if let Some(s) = slot.take() {
        s.proc.stop().await;
    }
    // Describe works again afterwards (Models folder change, helper delete).
    *core.describe.stopping.lock() = tokio_util::sync::CancellationToken::new();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_describes_keep_the_engine_busy() {
        let st = DescribeState::default();
        let a = BusyGuard::new(&st);
        let b = BusyGuard::new(&st);
        drop(a);
        assert!(st.is_busy(), "the other describe still runs");
        drop(b);
        assert!(!st.is_busy());
    }

    fn improve_spec() -> ImproveSpec {
        let style = |name: &str| pinhole_registry::ImproveStyle {
            format: format!("{name} format."),
            style_examples: "ink".into(),
            example: "EX".into(),
        };
        let list = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        ImproveSpec {
            form: "FORM {format} STYLE {style_examples} {example}".into(),
            styles: [
                ("natural", style("nat")),
                ("tags", style("tags")),
                ("booru", style("booru")),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
            safe: "SAFE.".into(),
            adult: "ADULT.".into(),
            avoid: "Skip: {words}.".into(),
            given: [
                ("shot", list(&["full body", "side view"])),
                ("style", list(&["cartoon", "photo"])),
                ("light", list(&["sunset"])),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
            drop: list(&["atmosphere", "best quality"]),
            common_words: list(&["the", "with", "make", "add", "replace", "give", "her"]),
            people: list(&["1boy", "1girl", "girl", "man"]),
            person_details: list(&["hair", "suit", "suits", "dress"]),
            exact_words: list(&["her", "more", "less"]),
            edit: Default::default(),
        }
    }

    #[test]
    fn improve_instruction_follows_style_safe_mode_and_trigger_words() {
        let m = improve_spec();
        assert_eq!(
            improve_instruction(&m, "tags", false, &[]).unwrap(),
            "FORM tags format. STYLE ink EX ADULT."
        );
        assert_eq!(
            improve_instruction(&m, "booru", true, &[]).unwrap(),
            "FORM booru format. STYLE ink EX SAFE."
        );
        assert_eq!(
            improve_instruction(&m, "natural", true, &["sks style".into(), "ink".into()]).unwrap(),
            "FORM nat format. STYLE ink EX SAFE. Skip: sks style, ink."
        );
        assert!(improve_instruction(&Default::default(), "tags", false, &[]).is_none());
    }

    #[test]
    fn improve_style_comes_from_the_family() {
        let reg = pinhole_registry::Registry::from_yaml(
            include_str!("../../../../config/models.yaml"),
            None,
        )
        .unwrap();
        let spec = &reg.captioner().improve;
        assert_eq!(improve_style(spec, reg.family("sd15")), "tags");
        assert_eq!(improve_style(spec, reg.family("sdxl_illustrious")), "booru");
        assert_eq!(improve_style(spec, reg.family("z_image_turbo")), "natural");
        assert_eq!(improve_style(spec, None), "natural");
        for st in spec.styles.values() {
            assert!(!st.example.is_empty() && !st.format.is_empty());
        }
    }

    #[test]
    fn form_lines_follow_labels_then_position() {
        let six = |v: [&str; 6]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            form_lines(
                "Here are the six lines:\nPROMPT: a fox in a cafe\n**DETAILS:** red fur, round glasses\n- PLACE: small cafe\n\nSHOT: -\n3d render, clay\nlight: soft daylight.\nEXTRA: ignored",
                &FORM_LINES,
            ),
            six(["a fox in a cafe", "red fur, round glasses", "small cafe", "", "3d render, clay", "soft daylight"])
        );
        // Unknown labels and plain lines take the next place; known labels go to their own.
        assert_eq!(
            form_lines(
                "castle: stone walls\n1. floating island\nLIGHT: warm sun",
                &FORM_LINES
            ),
            six(["stone walls", "floating island", "", "", "", "warm sun"])
        );
        assert_eq!(form_lines("Sure:\nred fur", &FORM_LINES)[0], "red fur");
    }

    #[test]
    fn improved_prompt_keeps_the_idea_and_adds_the_rest() {
        let m = improve_spec();
        let lines: Vec<String> = [
            "-",
            "red fur, round glasses, red fur, cosy atmosphere",
            "small cafe",
            "close-up",
            "photo, 50mm lens",
            "warm window light",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        // The idea names a style and a view: those lines are left out; repeats, `drop` words and
        // the idea's own words go too.
        let idea = "A fox reading in a cafe, cartoon, side view";
        assert_eq!(
            assemble_improved(idea, &lines, &m, "tags", &[]).unwrap(),
            "A fox reading in a cafe, cartoon, side view, red fur, round glasses, small cafe, warm window light"
        );
        assert_eq!(
            assemble_improved("a fox. ", &lines, &m, "natural", &["glasses".into()]).unwrap(),
            "a fox. Red fur. Small cafe. Close-up, photo, 50mm lens, warm window light."
        );
        // Long sentences are left out.
        let long = vec![
            "-".to_string(),
            "The fox looks like it is enjoying a quiet moment with its paper".to_string(),
        ];
        assert!(assemble_improved("a fox", &long, &m, "tags", &[]).is_none());
        assert!(assemble_improved("a fox", &[], &m, "natural", &[]).is_none());
        assert_eq!(
            assemble_improved("Wow!", &lines[..2], &m, "natural", &[]).unwrap(),
            "Wow! Red fur, round glasses."
        );
        // A person the idea doesn't have is left out; one it has stays.
        let person = vec![
            "-".to_string(),
            "rain on the glass, girl looking out the window, neon signs".to_string(),
        ];
        assert_eq!(
            assemble_improved("a rainy street", &person, &m, "tags", &[]).unwrap(),
            "a rainy street, rain on the glass, neon signs"
        );
        assert_eq!(
            assemble_improved("1girl, rainy street", &person, &m, "tags", &[]).unwrap(),
            "1girl, rainy street, rain on the glass, girl looking out the window, neon signs"
        );
    }

    #[test]
    fn improve_rewords_the_idea_when_that_keeps_its_intent() {
        let m = improve_spec();
        let with = |prompt: &str, details: &str| vec![prompt.to_string(), details.to_string()];
        // Reworded: the idea's words are there, details follow.
        assert_eq!(
            assemble_improved(
                "fox reading cafe cartoon",
                &with("a cartoon fox reading a book in a cafe.", "red fur"),
                &m,
                "natural",
                &[]
            )
            .unwrap(),
            "a cartoon fox reading a book in a cafe. Red fur."
        );
        // A rewording alone is enough to change the prompt.
        assert_eq!(
            assemble_improved(
                "cat astronaut",
                &with("a cat astronaut in a spacesuit", "-"),
                &m,
                "natural",
                &[]
            )
            .unwrap(),
            "a cat astronaut in a spacesuit"
        );
        // Lost words, a lost style, number or negation, an added negation or mood words: the idea as typed.
        for (idea, reworded) in [
            (
                "same robot sitting in a garden",
                "an old lighthouse keeper, watercolor",
            ),
            ("a fox, cartoon", "a fox in a forest, photo"),
            ("two dogs on a sofa", "dogs on a sofa"),
            ("a man with no hat", "a man with a hat"),
            ("a red car", "a car that is not red"),
            ("a cabin in snow", "a cabin in snow, cosy atmosphere"),
        ] {
            assert_eq!(
                assemble_improved(idea, &with(reworded, "red door"), &m, "natural", &[]).unwrap(),
                format!("{idea}. Red door."),
                "{reworded}"
            );
        }
        assert!(!keeps_intent("two dogs", "2 dogs", &m, &[], &[]));
        assert!(keeps_intent(
            "same bottle, different table",
            "the same bottle on a different table",
            &m,
            &[],
            &[]
        ));
        assert!(!keeps_intent(
            "a fox",
            "a fox, sks",
            &m,
            &["sks".into()],
            &[]
        ));
        assert!(!keeps_intent(
            "a snowy cabin",
            "1boy, snowy cabin",
            &m,
            &[],
            &[]
        ));
        let actions = vec![
            vec!["add".to_string(), "put".to_string()],
            vec!["replace".to_string()],
            vec!["make".to_string(), "change".to_string()],
        ];
        assert!(!keeps_intent(
            "add a red umbrella",
            "replace the background with a red umbrella",
            &m,
            &[],
            &actions
        ));
        assert!(keeps_intent(
            "make the sky a sunset",
            "change the sky to a sunset",
            &m,
            &[],
            &actions
        ));
        assert!(keeps_intent(
            "same bottle, different table",
            "put the same bottle on a different table",
            &m,
            &[],
            &actions
        ));
        let mut refs = improve_spec();
        refs.edit.keep_wording = vec!["image 2".into()];
        assert!(!keeps_intent(
            "put the bottle from image 2 on the shelf",
            "place the bottle on the shelf like in image 2",
            &refs,
            &[],
            &[]
        ));
        assert!(!keeps_intent(
            "a red car in the rain",
            "a red sports car",
            &m,
            &[],
            &[]
        ));
        // Swapped meaning with the same words, added numbers or people, a verb that undoes the
        // instruction: not used.
        let mut verbs = improve_spec();
        verbs.edit.never_added = vec!["remove".into()];
        for (idea, reworded) in [
            ("make the shirt less red", "make the shirt more red"),
            ("a cat, not a dog", "a dog, not a cat"),
            ("a dog on a sofa", "three dogs on a sofa"),
            ("a hat on the cat", "remove the hat from the cat"),
        ] {
            assert!(
                !keeps_intent(idea, reworded, &verbs, &[], &actions),
                "{reworded}"
            );
        }
        assert!(keeps_intent(
            "a girl with red hair",
            "1girl, red hair",
            &m,
            &[],
            &[]
        ));
        assert!(keeps_intent(
            "a hat on the cat",
            "put a hat on the cat",
            &verbs,
            &[],
            &actions
        ));
        // Only letter case, punctuation or an article changed: no improvement.
        assert!(assemble_improved("A fox!", &with("a fox", "-"), &m, "natural", &[]).is_none());
        // Phrases copied from the form's example are left out.
        let mut ex = improve_spec();
        ex.styles.get_mut("natural").unwrap().example =
            "PROMPT: x\nDETAILS: grey beard, flat cap\nPLACE: rocky shore".into();
        assert_eq!(
            assemble_improved(
                "old man",
                &[
                    "-".into(),
                    "grey beard, flat cap, red scarf".into(),
                    "rocky shore".into(),
                    "red scarf, no hat".into()
                ],
                &ex,
                "natural",
                &[]
            )
            .unwrap(),
            "old man. Rocky shore. Red scarf."
        );
    }

    #[test]
    fn edit_improve_adds_details_and_what_to_keep() {
        let mut m = improve_spec();
        m.edit.given = [
            ("details", vec!["remove".to_string()]),
            ("keep", vec!["keep".to_string(), "unchanged".to_string()]),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let lines = |d: &str, k: &str| vec!["-".to_string(), d.to_string(), k.to_string()];
        assert_eq!(
            assemble_edit(
                "make it evening.",
                &lines("dark blue sky, warm street lights", "Keep the buildings the same, the cars"),
                &m,
                &[],
            )
            .unwrap(),
            "make it evening. Dark blue sky, warm street lights. Keep the buildings and the cars unchanged."
        );
        // Keep phrases naming what changes are left out, singular or plural; so is the whole
        // line when the instruction already says what to keep.
        assert_eq!(
            assemble_edit(
                "replace the cars with bikes next to the man",
                &lines("-", "the car, the street, the man's face"),
                &m,
                &[]
            )
            .unwrap(),
            "replace the cars with bikes next to the man. Keep the street and the man's face unchanged."
        );
        assert_eq!(
            assemble_edit(
                "paint the door red, keep the rest",
                &lines("glossy paint", "the walls"),
                &m,
                &[]
            )
            .unwrap(),
            "paint the door red, keep the rest. Glossy paint."
        );
        // Details that say to keep something or say "no …" are left out; at most three things
        // to keep.
        assert_eq!(
            assemble_edit(
                "add a car",
                &lines(
                    "same road, no people, maintain the colours, black sedan",
                    "the trees, the sky, the road, the houses"
                ),
                &m,
                &[]
            )
            .unwrap(),
            "add a car. Black sedan. Keep the trees, the sky and the road unchanged."
        );
        // "-es" plurals; pronouns don't count as what changes; other wordings of "keep".
        assert_eq!(
            assemble_edit(
                "remove the boxes",
                &lines("-", "the box, the shelf stays the same"),
                &m,
                &[]
            )
            .unwrap(),
            "remove the boxes. Keep the shelf unchanged."
        );
        assert_eq!(
            assemble_edit(
                "give her a red hat",
                &lines("wide brim", "her face, the same lighting (if any)"),
                &m,
                &[]
            )
            .unwrap(),
            "give her a red hat. Wide brim. Keep her face and the lighting unchanged."
        );
        assert_eq!(singular("glasses"), "glass");
        assert_eq!(singular("glass"), "glass");
        // The helper can't see what is behind a removed thing: only what to keep is added.
        assert_eq!(
            assemble_edit("remove the car", &lines("empty road", "the trees"), &m, &[]).unwrap(),
            "remove the car. Keep the trees unchanged."
        );
        // The reworded instruction leads when it keeps the intent; details of what stays the
        // same are left out.
        assert_eq!(
            assemble_edit(
                "same bottle, different table",
                &[
                    "put the same bottle on a different table".into(),
                    "round glass vase, the same bottle label, dark oak table".into(),
                    "the bottle, the lighting".into()
                ],
                &m,
                &[]
            )
            .unwrap(),
            "put the same bottle on a different table. Dark oak table. Keep the lighting unchanged."
        );
        // Mood words and trigger words go as in Create; nothing left = no change.
        assert!(assemble_edit(
            "add a hat",
            &lines("cosy atmosphere, sks hat", "the hat"),
            &m,
            &["sks".into()]
        )
        .is_none());
        assert_eq!(
            form_lines("KEEP: the face\nDETAILS: - red wool", &EDIT_FORM_LINES),
            vec![
                String::new(),
                "red wool".to_string(),
                "the face".to_string()
            ]
        );
        assert_eq!(
            and_list(&["a".into(), "b".into(), "c".into()]),
            "a, b and c"
        );
    }

    #[test]
    fn edit_instruction_has_the_safe_mode_rule_and_trigger_words() {
        let mut m = improve_spec();
        assert!(edit_instruction(&m, true, &[]).is_none());
        m.edit.form = " EDIT FORM ".into();
        assert_eq!(edit_instruction(&m, true, &[]).unwrap(), "EDIT FORM SAFE.");
        assert_eq!(
            edit_instruction(&m, false, &["sks".into()]).unwrap(),
            "EDIT FORM ADULT. Skip: sks."
        );
    }

    #[test]
    fn tag_styles_add_at_most_twelve_tags() {
        let m = improve_spec();
        let many: Vec<String> = (0..5)
            .map(|i| {
                (0..5)
                    .map(|j| format!("t{i}{j}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .collect();
        let out = assemble_improved(
            "idea",
            &[&["-".to_string()], &many[..]].concat(),
            &m,
            "booru",
            &[],
        )
        .unwrap();
        assert_eq!(out.split(", ").count(), 1 + MAX_ADDED_TAGS);
    }

    #[test]
    fn hair_and_clothes_are_added_only_when_the_idea_has_a_person() {
        let m = improve_spec();
        let lines = |reworded: &str, details: &str| {
            let mut l = vec![reworded.to_string(), details.to_string()];
            l.resize(6, "-".to_string());
            l
        };
        // No person: hair and outfits would put one in the picture.
        assert_eq!(
            assemble_improved(
                "castle, sunset",
                &lines("-", "stone walls, long_hair, golden hair, sleek suits"),
                &m,
                "booru",
                &[]
            )
            .unwrap(),
            "castle, sunset, stone walls"
        );
        assert!(!keeps_intent(
            "castle at sunset",
            "castle with long golden hair at sunset",
            &m,
            &[],
            &[]
        ));
        // A person, or the thing itself in the idea: kept.
        assert_eq!(
            assemble_improved(
                "1girl, castle",
                &lines("-", "long_hair, stone walls"),
                &m,
                "booru",
                &[]
            )
            .unwrap(),
            "1girl, castle, long_hair, stone walls"
        );
        assert_eq!(
            assemble_improved(
                "a dress on a hanger",
                &lines("-", "red silk dress"),
                &m,
                "natural",
                &[]
            )
            .unwrap(),
            "a dress on a hanger. Red silk dress."
        );
    }

    #[test]
    fn refusals_are_recognised_only_at_the_start() {
        assert!(is_refusal("I'm sorry, but I can't help with that."));
        assert!(is_refusal("\"I cannot create that content.\""));
        assert!(is_refusal("I\u{2019}m unable to assist with this request."));
        assert!(is_refusal("As an AI, I won't write that."));
        assert!(!is_refusal(
            "A knight who cannot sleep, pacing a moonlit hall."
        ));
        assert!(!is_refusal("sorrowful widow, rain, candlelight"));
        assert!(!is_refusal("icy mountain, i can see for miles"));
        assert!(is_refusal("Sorry but I can't do that."));
        assert!(is_refusal("Unfortunately I cannot write this."));
        assert!(is_refusal("I apologise, that isn't something I can do."));
        assert!(is_refusal("I'm afraid I can't help with that."));
        assert!(!is_refusal(
            "As an aircraft banks over the bay, gulls scatter."
        ));
    }

    #[test]
    fn helper_fit_reads_size_against_memory() {
        use pinhole_registry::vram::Fit;
        assert_eq!(helper_fit(2_800_000_000, 16.0, 32.0), Some(Fit::Fits));
        assert_eq!(helper_fit(8_900_000_000, 8.0, 32.0), Some(Fit::Tight));
        assert_eq!(helper_fit(8_900_000_000, 4.0, 4.0), Some(Fit::TooBig));
        assert_eq!(helper_fit(2_800_000_000, 0.0, 16.0), None);
    }

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
        let st = CaptionerStatus {
            available: false,
            source: None,
            download_bytes: 5,
            running: false,
        };
        let v = serde_json::to_value(&st).unwrap();
        assert_eq!(v["downloadBytes"], 5);
        assert!(v["source"].is_null());
    }
}
