//! Generate / cancel / upscale / prompt preview.
//!
//! Flow (docs/ARCHITECTURE.md §4 "Generate"): installed model + family →
//! components → `wiring::launch_args` → (re)start sd-server only when the args
//! differ → combine prompt/style/prefix/negatives in memory → `img_gen` with
//! `embed_image_metadata: false` → poll every 300 ms → decode, scrub every PNG
//! text chunk, keep in the RAM [`Session`](crate::session::Session).
//!
//! The sd-server process lives in [`crate::engine`]; what happens when a job runs out of
//! memory is in [`crate::memory`].
//!
//! PRIVACY: `GenerateRequest`, `FinalPromptPreview` and the engine request body
//! carry prompt text. They are never logged, never written to disk and never
//! put into a `CoreError`. The only disk write here is `last_used` (a number)
//! in `installed.json` (or `linked-folders.json` for a linked model).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use pinhole_engine::detail::{DetailError, DetailPlan};
use pinhole_engine::extend::{Canvas, ExtendError, ExtendPlan};
use pinhole_engine::failure::{classify, memory_failure, Failure, Stage};
use pinhole_engine::install::EngineKind;
use pinhole_engine::logbuf::ProgressKind;
use pinhole_engine::sdapi::{
    ApiError, CancelOutcome, Guidance, HiresRequest, ImgGenRequest, Job, JobStatus, LoraRef,
    SampleParams, SdClient, UpscaleRequest, VaeTilingRequest,
};
use pinhole_engine::words::CheckedPrompt;
use pinhole_registry::style::FinalPrompt;
use pinhole_registry::wiring::{
    self, Dials, FamilyUi, FineTune, GenMode, HwContext, LaunchExtras, ModelFiles, Quality, Shape,
};
use pinhole_registry::{Family, Layout};
use pinhole_store::datadir::ModelKind;
use pinhole_store::InstalledFile;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::engine::{
    after_job, decoding_now, drop_engine, emit_progress, engine_died, engine_failure,
    ensure_engine, exit_details, failure_error, running_engine,
};
use crate::engine_setup;
use crate::events::GenPhase;
use crate::memory::{
    memory_choices, memory_error, next_memory_fallback, offload_fits_ram, remember_memory_choices,
    set_retry_note, with_memory_choices, with_memory_plan, with_remembered_offload, MemFallback,
    TeChoice,
};
use crate::session::SessionImage;
use crate::{AppCore, CoreError, CoreResult};

const POLL_EVERY: Duration = Duration::from_millis(300);
/// Registry component id of the ESRGAN upscaler for photo-style pictures.
pub const UPSCALER_COMPONENT: &str = "realesrgan_x4";
/// Registry component id of the ESRGAN upscaler for drawn pictures.
pub const UPSCALER_DRAWING_COMPONENT: &str = "realesrgan_x4_anime";
/// Registry component id of the ESRGAN photo upscaler that keeps skin texture (Settings only).
pub const UPSCALER_PHOTO_TEXTURE_COMPONENT: &str = "nomos_webphoto_x4";
/// A job that doesn't say why it failed.
pub const UNKNOWN_JOB_MESSAGE: &str = "The engine couldn't make this image. Try again with different settings (e.g. the Fast setting or a smaller size).";
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
    /// Upscaled pictures: which upscaler made it, `photo`, `photo_texture` or `drawing`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upscaler: Option<String>,
    /// Made with "Repeats without seams": its edges join up when repeated.
    #[serde(default)]
    pub seamless: bool,
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

// ================================================================ cancel / family UI

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
    /// Installed file ids of the picked add-ons (for the image check).
    addon_ids: Vec<String>,
}

/// Read-only "Final prompt sent to the model" (combined in memory, never stored).
pub async fn preview_final_prompt(
    core: &Arc<AppCore>,
    req: &GenerateRequest,
) -> CoreResult<FinalPromptPreview> {
    let with_face_prompt = add_detail_prompt(core, req).await?;
    let p = prepare(core, with_face_prompt.as_ref().unwrap_or(req), false).await?;
    Ok(FinalPromptPreview {
        prompt: p.final_prompt.prompt,
        negative: p.final_prompt.negative,
    })
}

/// `materialize`: make linked add-ons readable by the engine (a link or copy into Pinhole's
/// add-on folder). Off for the read-only prompt preview, which writes nothing.
async fn prepare(
    core: &Arc<AppCore>,
    req: &GenerateRequest,
    materialize: bool,
) -> CoreResult<Prepared> {
    let (model, family) = resolve_model(core, req)?;
    let reg = core.registry();
    // Add-ons were picked for the chosen model; an edit that fell back to
    // another model (the chosen one can't edit) doesn't get them.
    let picked = if model.id == req.model_id {
        &req.loras[..]
    } else {
        &[]
    };
    crate::lookup::refuse_if_flagged(&model)?;
    let engine_paths = if materialize {
        link_add_ons(core, picked).await?
    } else {
        HashMap::new()
    };

    // LoRAs → structured list (+ trigger words, in memory).
    let lora_dir = core.data.models(ModelKind::Lora);
    let mut loras = Vec::new();
    let mut triggers: Vec<String> = Vec::new();
    // Every picked add-on's name and trigger words, for the word check below (whether or
    // not the words are added to the prompt: the add-on steers the image either way).
    let mut addon_words: Vec<String> = Vec::new();
    let mut addon_ids = Vec::new();
    {
        let idx = core.installed.lock();
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
                    let Some(p) = engine_paths.get(&f.id) else {
                        return Err(CoreError::not_found(format!(
                            "The add-on “{}” isn't in the other app's folder any more. Check that its drive is connected, or remove it in Fine-tune.",
                            f.friendly_name
                        )));
                    };
                    abs = p.clone();
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
            addon_ids.push(f.id.clone());
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

    // Word-checked: the whole positive prompt (idea + style + trigger words) and the add-ons.
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
        addon_ids,
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

/// Engine paths (by file id) of the picked add-ons from linked folders: links, or copies,
/// in Pinhole's add-on folder ([`crate::linked::lora_path_for_engine`]). Made on a blocking
/// thread without holding the index lock. Add-ons whose file isn't there are left out.
/// A flagged add-on refuses the request before anything is linked or copied.
async fn link_add_ons(
    core: &Arc<AppCore>,
    picked: &[LoraUse],
) -> CoreResult<HashMap<String, PathBuf>> {
    let todo: Vec<(InstalledFile, PathBuf)> = {
        let idx = core.installed.lock();
        let addons: Vec<&InstalledFile> = picked
            .iter()
            .filter_map(|l| idx.get(&l.lora_id))
            .filter(|f| f.kind == ModelKind::Lora)
            .collect();
        for f in &addons {
            crate::lookup::refuse_if_flagged(f)?;
        }
        addons
            .into_iter()
            .filter(|f| f.is_linked())
            .map(|f| (f.clone(), idx.abs_path(&core.data, f)))
            .collect()
    };
    if todo.is_empty() {
        return Ok(HashMap::new());
    }
    let core = core.clone();
    tokio::task::spawn_blocking(move || {
        let mut out = HashMap::new();
        for (f, abs) in todo {
            if abs.is_file() {
                let p = crate::linked::lora_path_for_engine(&core, &f, &abs)?;
                out.insert(f.id, p);
            }
        }
        Ok(out)
    })
    .await
    .map_err(|_| CoreError::internal("Getting an add-on ready stopped unexpectedly."))?
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

/// Add detail's prompt when nothing is typed, for a photo-style picture ("a detailed face"
/// alone made photo faces look carved).
pub(crate) const PHOTO_FACE_PROMPT: &str = "photo of a face, natural skin texture, sharp focus";
/// Add detail's prompt when nothing is typed, for a drawn picture.
pub(crate) const DRAWN_FACE_PROMPT: &str = "a detailed face";

/// The picture Add detail (Fix details, nothing painted) works on, when nothing is typed.
fn add_detail_source(core: &AppCore, req: &GenerateRequest) -> Option<SessionImage> {
    let plain = req.fix_details
        && req.mode == GenMode::Img2img
        && req.mask_image_id.is_none()
        && req.prompt.trim().is_empty();
    plain
        .then_some(req.init_image_id.as_deref())
        .flatten()
        .and_then(|id| session_image(core, id).ok())
}

/// Add detail with nothing typed: the request with a prompt that says what the faces are,
/// by picture style. `None` for every other request.
async fn add_detail_prompt(
    core: &Arc<AppCore>,
    req: &GenerateRequest,
) -> CoreResult<Option<GenerateRequest>> {
    let Some(src) = add_detail_source(core, req) else {
        return Ok(None);
    };
    let mut r = req.clone();
    // A picked Style says what the picture looks like: no photo words against it.
    let styled = req.style_id.as_deref().is_some_and(|s| !s.is_empty());
    r.prompt = if !styled && crate::imagecheck::is_photo_style(core, &src).await? {
        PHOTO_FACE_PROMPT
    } else {
        DRAWN_FACE_PROMPT
    }
    .to_string();
    Ok(Some(r))
}

/// Add detail redraws at most this many faces (the largest), one engine pass each.
const MAX_FACES: usize = 6;

/// Masks (PNG, white = redraw) for the faces on `src` that Add detail redraws, largest
/// first: each face box found by the image check's face finder, grown a little so hair
/// line, chin and ears blend in. Only faces whose redraw box (the face and the room
/// around it, about twice its size) is smaller than the model's `area`: a larger face
/// would be drawn smaller than it is and come back softer.
async fn faces_to_redraw(
    core: &Arc<AppCore>,
    src: &SessionImage,
    area: u64,
) -> CoreResult<std::collections::VecDeque<Vec<u8>>> {
    let boxes = crate::imagecheck::face_boxes(core, src).await?;
    let (w, h) = (src.width, src.height);
    let model_side = (area as f32).sqrt();
    tokio::task::spawn_blocking(move || {
        boxes
            .iter()
            .filter(|b| b[2].max(b[3]) * 2.0 < model_side)
            .take(MAX_FACES)
            .map(|b| pinhole_engine::detail::box_mask(w, h, *b, 0.15))
            .collect::<Result<_, _>>()
    })
    .await
    .map_err(|_| CoreError::internal("Finding faces stopped unexpectedly."))?
    .map_err(|e| CoreError::invalid(e.to_string()))
}

/// A "Fix details" plan for `mask` on `src`, drawn at about `area` (off the async workers).
async fn detail_plan(
    src: Arc<Vec<u8>>,
    mask: Arc<Vec<u8>>,
    area: u64,
    multiple: u32,
) -> CoreResult<DetailPlan> {
    tokio::task::spawn_blocking(move || {
        DetailPlan::new(&src, &mask, |w, h| size_like(w, h, area, multiple))
    })
    .await
    .map_err(|_| CoreError::internal("Fixing details stopped unexpectedly."))?
    .map_err(|e| match e {
        DetailError::NothingPainted => CoreError::invalid("Paint over the spot to fix first."),
        DetailError::Image(e) => CoreError::invalid(e.to_string()),
    })
}

/// The pictures a job sends to the engine, read from the session only through here: every
/// one taken is an input of the result for the image check ([`crate::imagecheck::MadeBy`]),
/// and a mask goes as its shape only.
#[derive(Default)]
struct Inputs(Vec<SessionImage>);

impl Inputs {
    /// A session picture to send (base64), recorded as an input.
    fn take(&mut self, core: &AppCore, id: &str) -> CoreResult<(String, SessionImage)> {
        let img = session_image(core, id)?;
        self.0.push(img.clone());
        Ok((
            base64::engine::general_purpose::STANDARD.encode(img.bytes.as_slice()),
            img,
        ))
    }

    /// A mask: only its shape (black and white) is sent, so it isn't an input.
    async fn mask(core: &AppCore, id: &str) -> CoreResult<(String, SessionImage)> {
        let img = session_image(core, id)?;
        let bytes = img.bytes.clone();
        // Decode and encode: off the async workers.
        let shape = tokio::task::spawn_blocking(move || pinhole_engine::detail::mask_shape(&bytes))
            .await
            .map_err(|_| CoreError::internal("Reading the painted area stopped unexpectedly."))?
            .map_err(|e| CoreError::invalid(e.to_string()))?;
        Ok((base64::engine::general_purpose::STANDARD.encode(shape), img))
    }

    fn pictures(&self) -> &[SessionImage] {
        &self.0
    }
}

fn session_image(core: &AppCore, id: &str) -> CoreResult<SessionImage> {
    core.session.get(id).ok_or_else(|| {
        CoreError::not_found("That image isn't in this session anymore. Add it again.")
    })
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
    *core.gen.part_note.lock() = None;
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
        Err(_) => emit_progress(core, GenPhase::Failed, &label, None, None, t0),
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
    let with_face_prompt = add_detail_prompt(core, req).await?;
    let req = with_face_prompt.as_ref().unwrap_or(req);
    let prep = prepare(core, req, true).await?;
    let reg = core.registry();
    let hw = crate::app::hw_context(core);
    let label = prep.model.friendly_name.clone();
    final_label.clone_from(&label);

    // Source images (checked before any engine work).
    let mut init_image = None;
    let mut ref_images = Vec::new();
    let mut mask_image = None;
    let mut source: Option<SessionImage> = None;
    // Every picture the result is made from (not the mask: that is only a shape).
    let mut inputs = Inputs::default();
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
                let (b64, _) = inputs.take(core, id)?;
                ref_images.push(b64);
            }
        }
        GenMode::Img2img => {
            let id = req
                .init_image_id
                .as_deref()
                .ok_or_else(|| CoreError::invalid("Add an image to restyle first."))?;
            let (b64, img) = inputs.take(core, id)?;
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
                let (b64, img) = inputs.take(core, id)?;
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
            let (b64, img) = Inputs::mask(core, mid).await?;
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
    // "Fix details" with nothing painted redraws the whole picture.
    let fix_source = if req.fix_details {
        match (req.mode, &source) {
            (GenMode::Img2img, Some(src)) => Some((src, mask_src)),
            _ => return Err(CoreError::invalid("Add an image to fix first.")),
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
    // "Repeats without seams" is a launch flag in sd-server: a job with it on (or the
    // first job after it) restarts the engine, with the usual loading progress.
    let seamless =
        wiring::seamless_launch(&prep.family, &req.fine_tune, req.mode) && ref_images.is_empty();
    let extras = LaunchExtras {
        lora_dir: Some(core.data.models(ModelKind::Lora)),
        upscalers_dir: Some(core.data.models(ModelKind::Upscaler)),
        vae_tiling: None,
        seamless,
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
    // Add detail (nothing painted): each face the image check's face finder sees is redrawn
    // the same way, one after another (see `face_masks` below).
    let mut face_masks: std::collections::VecDeque<Vec<u8>> = Default::default();
    // Which pass runs, of how many: one per face for Add detail, else one.
    let mut part = (0u32, 1u32);
    let fix = match fix_source {
        Some((src, mask)) => {
            let area = u64::from(params.width) * u64::from(params.height);
            let multiple = wiring::size_multiple(&prep.family);
            let mask_bytes = match mask {
                Some(m) => m.bytes.clone(),
                None => {
                    face_masks = faces_to_redraw(core, src, area).await?;
                    part.1 = u32::try_from(face_masks.len()).unwrap_or(1).max(1);
                    Arc::new(face_masks.pop_front().ok_or_else(|| {
                        CoreError::invalid(
                            "No face found that needs more detail. Paint over the part to fix instead.",
                        )
                    })?)
                }
            };
            // Decode, resize, blur and encode: off the async workers.
            let src_bytes = src.bytes.clone();
            let plan = detail_plan(src_bytes, mask_bytes, area, multiple).await?;
            (width, height) = plan.work;
            init_image = Some(base64::engine::general_purpose::STANDARD.encode(&plan.init_png));
            mask_image = Some(base64::engine::general_purpose::STANDARD.encode(&plan.mask_png));
            Some(Redraw::Detail(Arc::new(plan)))
        }
        None => None,
    };
    // "Extend": the engine draws the whole bigger canvas at about the dial's area.
    let mut fix = match (fix, req.extend, &source) {
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
    let parent_id = source.as_ref().map(|s| s.id.clone());
    let mut pngs = Vec::new();
    let mut also_check = Vec::new();
    // One pass, or one per face for Add detail: each face is redrawn on the picture the
    // previous pass made.
    loop {
        if part.1 > 1 {
            *core.gen.part_note.lock() = Some(format!("Face {} of {}.", part.0 + 1, part.1));
        }
        // Memory-saving choices this pass's retries made; remembered once a run succeeds.
        let mut learned = MemFallback::default();
        let job = loop {
            let args = with_memory_choices(&wiring_args, fb);
            // An idle describe engine holds graphics memory this job needs,
            // also when the loaded image engine is reused.
            if gpu_backend {
                crate::describe::stop_if_idle(core).await;
            }
            let client = ensure_engine(core, &args, &prep.model.id, &label, cancel, t0).await?;
            match run_job(
                core,
                &client,
                &body,
                &prep.secrets,
                &label,
                (steps, batches),
                part,
                cancel,
                t0,
            )
            .await
            {
                Ok(job) => {
                    remember_memory_choices(core, &prep.model.id, learned);
                    break job;
                }
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
                    // The automatic choice, remembered for this model once a retry succeeds.
                    learned.te_on_cpu |= next_fb.te_on_cpu && !fb.te_on_cpu;
                    learned.vae_tiling |= next_fb.vae_tiling;
                    if next_fb.vram_reserve_gib > fb.vram_reserve_gib {
                        learned.vram_reserve_gib =
                            learned.vram_reserve_gib.max(next_fb.vram_reserve_gib);
                    }
                    fb = next_fb;
                    set_retry_note(core, note);
                    emit_progress(core, GenPhase::LoadingModel, &label, None, None, t0);
                }
            }
        };
        if cancel.is_cancelled() {
            return Err(CoreError::new("cancelled", "Cancelled."));
        }

        // Decode + scrub + keep in RAM.
        let mut images = job.result.map(|r| r.images).unwrap_or_default();
        images.sort_by_key(|i| i.index);
        if images.is_empty() {
            return Err(CoreError::new(
                "engine_failed",
                "The engine returned no image. Try again.",
            )
            .with_details(core.gen.logs.tail_text(20)));
        }
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
        let Some(next_mask) = face_masks.pop_front() else {
            break;
        };
        part.0 += 1;
        // Next face: plan it on the picture this pass made.
        let done = pngs.pop().expect("one image per Fix details pass");
        let area = u64::from(params.width) * u64::from(params.height);
        let plan = detail_plan(
            Arc::new(done),
            Arc::new(next_mask),
            area,
            wiring::size_multiple(&prep.family),
        )
        .await?;
        (body.width, body.height) = plan.work;
        body.init_image = Some(base64::engine::general_purpose::STANDARD.encode(&plan.init_png));
        body.mask_image = Some(base64::engine::general_purpose::STANDARD.encode(&plan.mask_png));
        fix = Some(Redraw::Detail(Arc::new(plan)));
    }
    *core.gen.part_note.lock() = None;
    drop(body);
    // Only whether this was a redraw is needed from here on; the plan holds the full source.
    let redrawn = fix.is_some();
    drop(fix);
    // Result intake: every picture passes the image check first; if one is blocked,
    // none is kept. A redrawn box is also checked on its own.
    let checked = crate::imagecheck::check_results(
        core,
        pngs,
        also_check,
        crate::imagecheck::MadeBy::Model {
            model_id: &prep.model.id,
            addon_ids: &prep.addon_ids,
            inputs: inputs.pictures(),
        },
    )
    .await
    // The engine keeps finished jobs readable on its port: a picture the check dropped
    // must go with it (the engine is stopped after this job).
    .inspect_err(|_| core.gen.clear_pending.store(true, Ordering::SeqCst))?;
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
            origin: Origin::of_result(inputs.pictures().iter().map(|i| &i.origin)),
            upscaler: None,
            seamless,
            // Fix details / Extend work on a crop or a canvas: the picture's own size stands.
            base_size: (!redrawn).then_some((width, height)),
        };
        let photo = png.photo_style();
        if !core
            .session
            .insert_generated(session_epoch, png, meta.clone())
        {
            // Reset while the job ran: its images go with the session.
            return Err(CoreError::new("cancelled", "Cancelled."));
        }
        core.check.note_photo_style(&core.session, &meta.id, photo);
        out.push(meta);
    }
    touch_last_used(core, &prep.model.id).await;
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
    (part_index, parts): (u32, u32),
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
    // From now on the engine may hold this job's images (IDLE_STOP_AFTER).
    core.gen.slot.lock().await.results_cached = true;
    let job_id = match client.submit(body).await {
        Ok(id) => id,
        Err(e) => {
            // The engine may have queued the job anyway: stop it so the job
            // doesn't keep running or hold up the next one.
            let queued = submit_may_have_queued(&e);
            let err = api_failure(core, e, secrets);
            if queued && core.gen.external.lock().is_none() {
                drop_engine(core).await;
            }
            return Err(job_failure(core, err, mark));
        }
    };

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
                                    // Several passes (Add detail, one per face): one bar for all.
                                    (part_index * p.total + p.step, p.total * parts)
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
                    let err = api_failure(core, e, secrets);
                    // The job may still be queued or running on the engine.
                    cancel_job(core, client, &job_id).await;
                    return Err(job_failure(core, err, mark));
                }
            }
        }
    }
}

/// Whether a failed submit may still have left the job queued on the engine
/// (no answer, or an answer that isn't a clear refusal).
fn submit_may_have_queued(e: &ApiError) -> bool {
    match e {
        ApiError::Connect | ApiError::QueueFull | ApiError::NotFound => false,
        ApiError::Status { code, .. } => *code >= 500,
        ApiError::Timeout | ApiError::Decode(_) | ApiError::Net(_) => true,
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

/// `last_used` of the model — a timestamp only. Saved on a blocking thread, to the file
/// that holds the entry: linked-folders.json for a linked model, else installed.json.
async fn touch_last_used(core: &Arc<AppCore>, model_id: &str) {
    let core = core.clone();
    let id = model_id.to_string();
    let _ = tokio::task::spawn_blocking(move || {
        let mut idx = core.installed.lock();
        let Some(f) = idx.get_mut(&id) else {
            return;
        };
        f.last_used = Some(now_secs());
        let _ = if f.is_linked() {
            idx.save_linked(&core.data)
        } else {
            idx.save_to(&core.data, &core.data.installed_file())
                .and_then(|()| idx.save_seals(&core.data))
        };
    })
    .await;
}

// ================================================================ upscale

/// Refusal for sources the upscaler can't take: 2× also runs at 4× first.
pub(crate) const UPSCALE_TOO_LARGE: &str = "This image is too large to upscale: the upscaler works at 4× first, up to 8192 pixels per side. Try a smaller image.";

/// The upscaler for `src`: the Settings choice, or with `auto` the photo upscaler for a
/// photo-style picture and the drawing upscaler otherwise. Returns (component id, `photo` |
/// `photo_texture` | `drawing`).
async fn pick_upscaler(
    core: &Arc<AppCore>,
    src: &SessionImage,
) -> CoreResult<(&'static str, &'static str)> {
    let choice = core.settings.read().upscaler.clone();
    if choice == "photo_texture" {
        return Ok((UPSCALER_PHOTO_TEXTURE_COMPONENT, "photo_texture"));
    }
    let photo = match choice.as_str() {
        "photo" => true,
        "drawing" => false,
        _ => crate::imagecheck::is_photo_style(core, src).await?,
    };
    Ok(if photo {
        (UPSCALER_COMPONENT, "photo")
    } else {
        (UPSCALER_DRAWING_COMPONENT, "drawing")
    })
}

/// Upscale a session image with Real-ESRGAN (4×; 2× = 4× then halve), using the photo or
/// drawing upscaler ([`pick_upscaler`]). The upscaler is downloaded on first use. Needs the
/// engine running (any model).
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
    *core.gen.part_note.lock() = None;
    // Set before the picture-style reading and the first-use upscaler download, so Cancel
    // works during both.
    let cancel = CancellationToken::new();
    *core.gen.active.lock() = Some(cancel.clone());
    let cancelled = || {
        if cancel.is_cancelled() {
            Err(CoreError::new("cancelled", "Cancelled."))
        } else {
            Ok(())
        }
    };
    let ready = async {
        let (component, style) = pick_upscaler(core, &src).await?;
        cancelled()?;
        let u = ensure_upscaler(core, component, &cancel).await?;
        // Cancelled just as the download finished: it stays installed, no upscale.
        cancelled()?;
        Ok::<_, CoreError>((u, style))
    }
    .await;
    let (upscaler, style) = match ready {
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
        style,
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
    style: &str,
    factor: u32,
    cancel: &CancellationToken,
    t0: Instant,
    session_epoch: u64,
    label: &mut String,
) -> CoreResult<ResultImage> {
    let hw = crate::app::hw_context(core);
    // A GPU engine build (a CPU build may stand in while the GPU one isn't downloaded).
    let gpu_backend = engine_setup::installed_engine(core, EngineKind::Sd)
        .map_or(hw.backend != "cpu", |e| e.backend != "cpu");
    // An idle describe engine holds graphics memory the upscale needs.
    if gpu_backend {
        crate::describe::stop_if_idle(core).await;
    }
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
            let files = model_files(core, &prep_model.0, &prep_model.1, &hw, false)?;
            let extras = LaunchExtras {
                lora_dir: Some(core.data.models(ModelKind::Lora)),
                upscalers_dir: Some(core.data.models(ModelKind::Upscaler)),
                vae_tiling: None,
                seamless: false,
                use_taesd: false,
            };
            let args = wiring::launch_args(&core.registry(), &files, &hw, &extras);
            // The same memory choices as Generate, so this launch doesn't forget
            // that the model's weights had to go to system memory.
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
    // The engine has run a job: the idle stop and Reset now stop it.
    core.gen.slot.lock().await.results_cached = true;
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
    // The request holds the source as base64: not needed once the engine answered.
    drop(req);
    let resp = resp.map_err(|e| match e {
        // The upscale request carries no prompt; `redact_text` still cuts prompt-like fields.
        ApiError::Status { code: 400, error } => {
            CoreError::new("invalid", "The upscaler couldn't process this image.")
                .with_details(redact_text(&error, &[]))
        }
        other => api_failure(core, other, &[]),
    })?;
    // The base64 answer goes as soon as it is decoded.
    let raw = {
        let img = resp.images.into_iter().next().ok_or_else(|| {
            CoreError::new(
                "engine_failed",
                "The upscaler returned no image. Try again.",
            )
        })?;
        base64::engine::general_purpose::STANDARD
            .decode(img.b64_json.as_bytes())
            .map_err(|_| {
                CoreError::new("engine_failed", "The upscaler returned a damaged image.")
            })?
    };
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
        upscaler: None,
        seamless: false,
        base_size: None,
    });
    meta.base_size = meta.base_size.or(Some((src.width, src.height)));
    meta.upscaler = Some(style.to_string());
    // The upscaler doesn't wrap around the edges, so they may no longer join exactly.
    meta.seamless = false;
    meta.id = uuid::Uuid::new_v4().to_string();
    meta.kind = ResultKind::Upscaled;
    meta.origin = src.origin;
    meta.width = w;
    meta.height = h;
    meta.parent_id = Some(src.id.clone());
    // Checked like every made picture (one way in), and it keeps the source's brought-in
    // pictures for later edits.
    let checked = crate::imagecheck::check_results(
        core,
        vec![png],
        Vec::new(),
        crate::imagecheck::MadeBy::Upscale(src),
    )
    .await?
    .pop()
    .ok_or_else(|| CoreError::internal("The upscale returned no image."))?;
    let photo = checked.photo_style();
    if cancel.is_cancelled()
        || !core
            .session
            .insert_generated(session_epoch, checked, meta.clone())
    {
        return Err(CoreError::new("cancelled", "Cancelled."));
    }
    core.check.note_photo_style(&core.session, &meta.id, photo);
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

pub(crate) fn pick_model_for_upscale(core: &AppCore, src: &SessionImage) -> Option<String> {
    let idx = core.installed.lock();
    let reg = core.registry();
    let known = |f: &&InstalledFile| f.family.as_deref().and_then(|id| reg.family(id)).is_some();
    // Prefer a model whose file is present (a linked drive may be disconnected).
    // If none is, still pick one so starting it reports which file is missing.
    let pick = |need_file: bool| {
        let usable =
            |f: &&InstalledFile| known(f) && (!need_file || idx.abs_path(&core.data, f).is_file());
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
    };
    pick(true).or_else(|| pick(false))
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

/// Display name of an upscaler component (downloads list, Models → Helpers).
fn upscaler_name(component: &str) -> &'static str {
    if component == UPSCALER_DRAWING_COMPONENT {
        "Upscaler for drawings (Real-ESRGAN anime 4×)"
    } else if component == UPSCALER_PHOTO_TEXTURE_COMPONENT {
        "Upscaler for photos, skin texture (4xNomosWebPhoto)"
    } else {
        "Upscaler for photos (Real-ESRGAN 4×)"
    }
}

/// Make sure a Real-ESRGAN component is installed; returns its file stem (the
/// sd-server upscaler name).
async fn ensure_upscaler(
    core: &Arc<AppCore>,
    component: &str,
    cancel: &CancellationToken,
) -> CoreResult<String> {
    let stem_of = |f: &InstalledFile| {
        std::path::Path::new(&f.rel_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
    };
    {
        let idx = core.installed.lock();
        if let Some(f) = idx.find_component(component) {
            if idx.abs_path(&core.data, f).is_file() {
                return stem_of(f).ok_or_else(|| CoreError::internal("Bad upscaler file name."));
            }
        }
    }
    let reg = core.registry();
    let comp = reg.component(component).cloned().ok_or_else(|| {
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
        label: upscaler_name(component).into(),
        ..Default::default()
    };
    let group = core.downloads.enqueue_kind(
        upscaler_name(component).into(),
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
            friendly_name: upscaler_name(component).into(),
            family: None,
            component_id: Some(component.into()),
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

    /// A core with a linked folder holding a checkpoint and a style add-on.
    fn core_with_linked_folder(tmp: &std::path::Path) -> (Arc<AppCore>, String, String) {
        use crate::linked::{fixtures, tests::new_core, tests::wait_scans};
        let comfy = tmp.join("Comfy");
        fixtures::sdxl(&comfy.join("checkpoints/m.safetensors"));
        fixtures::sdxl_lora(&comfy.join("loras/w.safetensors"));
        let core = new_core(&tmp.join("Data"));
        crate::linked::add(&core, &comfy.display().to_string()).unwrap();
        wait_scans(&core);
        let model = crate::models::list_models(&core).unwrap()[0].id.clone();
        let lora = core.installed.lock().loras().next().unwrap().id.clone();
        (core, model, lora)
    }

    /// Linked add-ons are made ready for the engine without holding the index lock.
    #[tokio::test]
    async fn linked_add_ons_are_made_ready_outside_the_index_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, model, lora) = core_with_linked_folder(tmp.path());
        let mut req = GenerateRequest::txt2img(model, "a red boat");
        req.loras = vec![LoraUse {
            lora_id: lora,
            weight: 1.0,
            words: None,
        }];
        let prep = prepare(&core, &req, true).await.unwrap();
        assert_eq!(prep.loras.len(), 1);
        let lora_dir = core.data.models(ModelKind::Lora);
        assert!(lora_dir.join(&prep.loras[0].path).is_file());
        assert_eq!(*core.linked.index_locked_while_linking.lock(), vec![false]);
    }

    /// A flagged model or add-on is refused before any linked add-on is linked or copied.
    #[tokio::test]
    async fn a_flagged_request_links_no_add_ons() {
        use pinhole_store::installed::Lookup;
        for flag_model in [true, false] {
            let tmp = tempfile::tempdir().unwrap();
            let (core, model, lora) = core_with_linked_folder(tmp.path());
            let flagged = if flag_model { &model } else { &lora };
            core.installed.lock().get_mut(flagged).unwrap().lookup = Some(Lookup::Refused);
            let mut req = GenerateRequest::txt2img(model, "a red boat");
            req.loras = vec![LoraUse {
                lora_id: lora,
                weight: 1.0,
                words: None,
            }];
            assert!(prepare(&core, &req, true).await.is_err());
            assert!(core.linked.index_locked_while_linking.lock().is_empty());
            let linked = core.data.models(ModelKind::Lora).join(".pinhole-linked");
            let n = std::fs::read_dir(&linked).map(|d| d.count()).unwrap_or(0);
            assert_eq!(n, 0);
        }
    }

    /// Using a linked model writes only linked-folders.json.
    #[tokio::test]
    async fn last_used_of_a_linked_model_is_saved_with_the_linked_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, model, _) = core_with_linked_folder(tmp.path());
        let installed = core.data.installed_file();
        assert!(!installed.exists());
        touch_last_used(&core, &model).await;
        assert!(core
            .installed
            .lock()
            .get(&model)
            .unwrap()
            .last_used
            .is_some());
        assert!(!installed.exists());
        let back = pinhole_store::installed::InstalledIndex::load(&core.data).unwrap();
        assert!(back.get(&model).unwrap().last_used.is_some());
    }

    /// Only a clear refusal means the engine didn't queue the job.
    #[test]
    fn submit_errors_that_may_have_queued_the_job() {
        assert!(submit_may_have_queued(&ApiError::Timeout));
        assert!(submit_may_have_queued(&ApiError::Decode("x".into())));
        assert!(submit_may_have_queued(&ApiError::Net("x".into())));
        assert!(submit_may_have_queued(&ApiError::Status {
            code: 500,
            error: String::new()
        }));
        assert!(!submit_may_have_queued(&ApiError::Connect));
        assert!(!submit_may_have_queued(&ApiError::QueueFull));
        assert!(!submit_may_have_queued(&ApiError::Status {
            code: 400,
            error: String::new()
        }));
    }

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
            upscaler: None,
            seamless: false,
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
