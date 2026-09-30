//! Describe (img2text) via llama-server. OWNER: engine agent.
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
                },
            };
            if let Err(e) = register_download(&core2, &file, reg) {
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
            let removable = [&m, &p].iter().any(|c| {
                index
                    .find_component(c)
                    .is_some_and(|f| f.kind == ModelKind::Captioner)
            });
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

/// The instruction for "Improve my prompt": tag or sentence style from the family's
/// `style_template`, the Safe mode rule (`safe` while On, `adult` while Off), and the add-on
/// trigger words not to repeat.
fn improve_instruction(
    improve: &std::collections::BTreeMap<String, String>,
    template: &str,
    safe: bool,
    avoid: &[String],
) -> Option<String> {
    let key = if template == "tags" {
        "tags"
    } else {
        "natural"
    };
    let mut out = improve.get(key)?.trim().to_string();
    if let Some(rule) = improve.get(if safe { "safe" } else { "adult" }) {
        out = format!("{out} {}", rule.trim());
    }
    if !avoid.is_empty() {
        if let Some(rule) = improve.get("avoid") {
            out = format!(
                "{out} {}",
                rule.trim().replace("{words}", &avoid.join(", "))
            );
        }
    }
    Some(out)
}

/// One line of clean text; add-on trigger words the model wrote anyway are taken out
/// (whole words only), because they are added separately at generation time.
fn tidy_improved(text: &str, avoid: &[String]) -> String {
    let mut t = llama::clean_caption(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for w in avoid {
        let w = w.trim();
        // Longest match first is not needed: each phrase is removed on its own.
        while !w.is_empty() && crate::generate::contains_phrase(&t, w) {
            let lower = t.to_lowercase();
            let wl = w.to_lowercase();
            // Byte offsets of the lowercase copy only line up for text that doesn't change
            // length when lowercased; otherwise leave the rest alone.
            if lower.len() != t.len() {
                break;
            }
            let Some(i) = lower.match_indices(&wl).map(|(i, _)| i).find(|&i| {
                let before = lower[..i].chars().next_back();
                let after = lower[i + wl.len()..].chars().next();
                !before.is_some_and(char::is_alphanumeric)
                    && !after.is_some_and(char::is_alphanumeric)
            }) else {
                break;
            };
            t.replace_range(i..i + wl.len(), "");
        }
    }
    // Tidy what the removals left: doubled or dangling commas and spaces.
    let parts: Vec<&str> = t
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    let joined = parts.join(", ");
    joined.split_whitespace().collect::<Vec<_>>().join(" ")
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

/// Drop repeated tags / sentences (case-insensitive). Returns the cleaned text and whether the
/// answer was a repetition loop ("bedroom, bedroom, …") or said nothing beyond the idea.
fn collapse_repeats(text: &str, idea: &str, tags: bool) -> (String, bool) {
    let norm = |p: &str| -> String {
        p.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let pieces: Vec<String> = if tags {
        text.split(',').map(|p| p.trim().to_string()).collect()
    } else {
        // Sentences, each keeping its ending mark.
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            cur.push(c);
            // Only at the end of a sentence: "f/1.8" stays in one piece.
            if matches!(c, '.' | '!' | '?') && chars.peek().is_none_or(|n| n.is_whitespace()) {
                out.push(std::mem::take(&mut cur).trim().to_string());
            }
        }
        // A last sentence cut off by the token limit is dropped (when a whole one came before).
        if !cur.trim().is_empty() && !out.is_empty() {
            cur.clear();
        }
        out.push(cur.trim().to_string());
        out
    };
    let pieces: Vec<String> = pieces.into_iter().filter(|p| !norm(p).is_empty()).collect();
    let total = pieces.len();
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<String> = pieces
        .into_iter()
        .filter(|p| seen.insert(norm(p)))
        .collect();
    let cleaned = unique.join(if tags { ", " } else { " " });
    // The same word four times in a row ("the the the the") is a loop too.
    let words: Vec<String> = norm(&cleaned).split(' ').map(str::to_string).collect();
    let word_loop = words.windows(4).any(|w| w.iter().all(|x| *x == w[0]));
    let mostly_repeats = total >= 4 && (total - unique.len()) * 2 > total;
    let too_thin = if tags {
        unique.len() < 3
    } else {
        cleaned.split_whitespace().count() < 6
    };
    let adds_nothing = norm(&cleaned) == norm(idea);
    (
        cleaned,
        word_loop || mostly_repeats || too_thin || adds_nothing,
    )
}

/// Turn a short idea into a fuller prompt with the local text model (the Describe model).
/// `family_id` picks tags vs sentences; `avoid` = trigger words of the add-ons in use.
/// PRIVACY: the prompt goes only to the loopback llama-server and back; never logged or stored.
pub async fn improve_prompt(
    core: &Arc<AppCore>,
    prompt: &str,
    family_id: Option<&str>,
    avoid: &[String],
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
    let template = family_id
        .and_then(|id| reg.family(id))
        .map(|f| f.style_template.clone())
        .unwrap_or_else(|| "natural".into());
    let safe = core.settings.read().content_mode != "all";
    let instruction = improve_instruction(&reg.captioner().improve, &template, safe, avoid)
        .ok_or_else(|| {
            CoreError::not_found("Improve my prompt isn't available. Update Pinhole.")
        })?;
    let max_tokens = if template == "tags" { 120 } else { 200 };

    let _busy = BusyGuard::new(&core.describe);
    let client = ensure_llama(core, chosen_helper(core, Purpose::Improve).as_deref()).await?;
    let text = client.rewrite(&instruction, idea, max_tokens).await.map_err(|e| {
        let tail = core.describe.logs.tail_text(30);
        match classify(&tail, None) {
            Failure::OutOfMemory => CoreError::new("vram", "Not enough memory to improve the prompt right now — close other apps or wait for the image to finish, then try again.").with_details(tail),
            _ => CoreError::new("engine_failed", "Improving the prompt failed. Try again.").with_details(format!("{e}\n{tail}")),
        }
    })?;
    let (text, degenerate) =
        collapse_repeats(&tidy_improved(&text, avoid), idea, template == "tags");
    // Before the fallbacks: text that would be blocked never comes back, even when short. A
    // refusal that names what it declines is blocked too, on purpose: the check always runs.
    crate::text_check::check(&text)?;
    if is_refusal(&text) {
        return Ok(ImprovedPrompt {
            text: idea.to_string(),
            note: Some("The helper wouldn't rewrite this one, so your prompt is unchanged.".into()),
        });
    }
    if degenerate {
        return Ok(ImprovedPrompt {
            text: idea.to_string(),
            note: Some(
                "The helper couldn't improve this one, so your prompt is unchanged. Try adding a few more words.".into(),
            ),
        });
    }
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
    let taken = || CoreError::new("engine_failed", crate::generate::PORT_TAKEN_MESSAGE);
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
        .any(|id| crate::generate::same_file_path(id, &expected))
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
    if let Some(msg) = core.describe.last_error.lock().clone() {
        return Err(CoreError::new("engine_failed", msg));
    }
    let (_, model, mmproj) = captioner_files(core, helper).ok_or_else(|| {
        CoreError::not_found(
            "The describe model isn't installed yet. Click Get on the Describe tab.",
        )
    })?;
    let engine = engine_setup::installed_engine(core, EngineKind::Llama).ok_or_else(|| {
        CoreError::not_found(
            "The describe engine isn't installed yet. Click Get on the Describe tab.",
        )
    })?;
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
    crate::generate::strip_flag(&mut args, &["--host", "--port"]);
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

    fn improve_map() -> std::collections::BTreeMap<String, String> {
        [
            ("natural", "NAT"),
            ("tags", "TAGS"),
            ("safe", "SAFE."),
            ("adult", "ADULT."),
            ("avoid", "Skip: {words}."),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    #[test]
    fn improve_instruction_follows_style_safe_mode_and_trigger_words() {
        let m = improve_map();
        assert_eq!(
            improve_instruction(&m, "tags", false, &[]).unwrap(),
            "TAGS ADULT."
        );
        assert_eq!(
            improve_instruction(&m, "tags", true, &[]).unwrap(),
            "TAGS SAFE."
        );
        assert_eq!(
            improve_instruction(&m, "natural", true, &["sks style".into(), "ink".into()]).unwrap(),
            "NAT SAFE. Skip: sks style, ink."
        );
        // Unknown templates read as sentences; a registry without the text says so.
        assert!(improve_instruction(&m, "other", false, &[])
            .unwrap()
            .starts_with("NAT"));
        assert!(improve_instruction(&Default::default(), "tags", false, &[]).is_none());
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
    fn improved_text_is_one_clean_line_without_trigger_words() {
        assert_eq!(
            tidy_improved("Prompt: \"a cat,\n  on a mat\"", &[]),
            "a cat, on a mat"
        );
        let avoid = vec!["Sks".to_string(), "ink wash".to_string()];
        assert_eq!(
            tidy_improved("sks, a cat, ink wash, soft light", &avoid),
            "a cat, soft light"
        );
        // Whole words only: "ink" stays inside "pink".
        assert_eq!(
            tidy_improved("a pink cat, ink", &["ink".to_string()]),
            "a pink cat"
        );
        assert_eq!(
            tidy_improved("A cat in the sks style.", &["sks".to_string()]),
            "A cat in the style."
        );
    }

    #[test]
    fn repetition_loops_are_caught_and_repeats_collapsed() {
        let looped = vec!["bedroom"; 60].join(", ");
        assert!(collapse_repeats(&looped, "bedroom", true).1);
        assert!(collapse_repeats("the the the the the", "x", false).1);
        assert!(
            collapse_repeats("bedroom", "bedroom", true).1,
            "adds nothing"
        );
        // A few repeats are just collapsed.
        let (t, bad) = collapse_repeats(
            "bedroom, cozy, Cozy, soft light, bedroom, warm lamp, morning sun",
            "bedroom",
            true,
        );
        assert!(!bad);
        assert_eq!(t, "bedroom, cozy, soft light, warm lamp, morning sun");
        let (t, bad) = collapse_repeats(
            "A cozy bedroom at dawn. Soft light fills the room. A cozy bedroom at dawn.",
            "bedroom",
            false,
        );
        assert!(!bad);
        assert_eq!(t, "A cozy bedroom at dawn. Soft light fills the room.");
    }

    #[test]
    fn sentences_keep_decimals_and_drop_a_cut_off_tail() {
        let (t, bad) = collapse_repeats(
            "A portrait shot at f/1.8 in warm light. The background is soft and calm. The sky is",
            "portrait",
            false,
        );
        assert!(!bad);
        assert_eq!(
            t,
            "A portrait shot at f/1.8 in warm light. The background is soft and calm."
        );
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
