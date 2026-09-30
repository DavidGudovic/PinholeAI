//! Serde types mirroring `config/models.yaml`. Field names follow the YAML.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegistryFile {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub components: BTreeMap<String, Component>,
    #[serde(default)]
    pub style_templates: BTreeMap<String, String>,
    /// Raw family entries (before `inherits` is applied).
    #[serde(default)]
    pub families: BTreeMap<String, serde_yaml::Value>,
    #[serde(default)]
    pub recommended: BTreeMap<String, Vec<RecommendedCandidate>>,
    #[serde(default)]
    pub captioner: CaptionerSpec,
    #[serde(default)]
    pub hardware_profiles: Vec<HardwareProfile>,
    #[serde(default)]
    pub known_files: Vec<KnownFile>,
    #[serde(default)]
    pub test_models: BTreeMap<String, DownloadSpec>,
    /// What the pinned `sd-server` can do that changes wiring (see YAML comments).
    #[serde(default)]
    pub engine_features: EngineFeatures,
}

/// Engine capabilities that influence wiring. Kept in `models.yaml` so a new
/// engine pin can flip them without code changes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EngineFeatures {
    /// `true` only when `sd-server` uses `--taesd` for *previews only*. The
    /// current server builds its context with `tae_preview_only = false`
    /// (examples/server/main.cpp), i.e. `--taesd` would replace the real VAE
    /// for the final image — so Pinhole must not pass it.
    #[serde(default)]
    pub taesd_preview: bool,
}

/// A shared component file (VAE, text encoder, TAESD, upscaler…).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    /// `vae` | `clip_l` | `clip_g` | `t5xxl` | `llm` | `llm_vision` | `taesd` | `upscaler`
    pub kind: String,
    pub file: String,
    pub url: String,
    /// Lowercase hex, or `TODO` until verified.
    pub sha256: String,
    pub size_mb: u64,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    AllInOne,
    DiffusionOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VramGb {
    pub min: f32,
    pub recommended: f32,
}

/// A fully resolved family (inherits applied).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Family {
    #[serde(default)]
    pub id: String,
    pub label: String,
    /// Key into `style_templates` (`natural` | `tags`).
    pub style_template: String,
    #[serde(default)]
    pub vram_gb: Option<VramGb>,
    #[serde(default)]
    pub activation_gb: f32,
    #[serde(default)]
    pub license_note: Option<String>,
    /// e.g. `edit` for instruction-edit families.
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub edit_priority: Option<u32>,
    /// Edits can take a second reference image ("take the bottle from image 2").
    #[serde(default)]
    pub multi_ref: bool,
    /// Small enough to be worth running on the processor when there is no
    /// usable GPU, whatever the file size (SD 1.5). See [`crate::vram::fit_cpu`].
    #[serde(default)]
    pub cpu_friendly: bool,
    /// `txt2img` | `img2img` | `inpaint` | `edit`
    #[serde(default)]
    pub modes: Vec<String>,
    #[serde(default)]
    pub detect: DetectRules,
    #[serde(default)]
    pub civitai_base_models: Vec<String>,
    pub layout: Layout,
    /// kind → component choice. Special key `vae_override` for all-in-one files.
    #[serde(default)]
    pub components: BTreeMap<String, ComponentChoice>,
    #[serde(default)]
    pub taesd: Option<String>,
    #[serde(default)]
    pub uses_negative_prompt: bool,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub defaults: FamilyDefaults,
    #[serde(default)]
    pub dials: DialSpec,
    #[serde(default)]
    pub download: Option<DownloadSpec>,
    /// Parent family this one was derived from (`inherits:`), if any.
    #[serde(default)]
    pub inherits: Option<String>,
}

/// Either a fixed component id, or a VRAM-dependent choice
/// (`{ vram_gte_16: t5xxl_fp16, else: t5xxl_fp8 }`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ComponentChoice {
    Fixed(String),
    ByVram(BTreeMap<String, String>),
}

/// Header-sniffing rules (SPEC §6 step 3), mirroring `get_sd_version()`.
///
/// **Pattern syntax.** A pattern is matched against every tensor name, both as
/// stored and with a leading `model.diffusion_model.` removed (the loader adds
/// that prefix to standalone diffusion files). By default it is a substring
/// match (like upstream's `name.find(...)`); a leading `^` anchors it at the
/// start, a trailing `$` at the end (`^…$` = exact name).
///
/// A family matches when it has at least one positive rule (`any_tensor`,
/// `all_tensor`, `all_of_any`) and every rule holds.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DetectRules {
    /// At least one pattern matches some tensor.
    #[serde(default)]
    pub any_tensor: Vec<String>,
    /// Every pattern matches some tensor.
    #[serde(default)]
    pub all_tensor: Vec<String>,
    /// No pattern matches any tensor.
    #[serde(default)]
    pub none_tensor: Vec<String>,
    /// AND of ORs: every inner list needs at least one matching pattern.
    #[serde(default)]
    pub all_of_any: Vec<Vec<String>>,
    /// `pattern → n`: every tensor matching `pattern` must have `ne[0] == n`
    /// (ggml order, i.e. the *last* PyTorch dimension). Mirrors the shape
    /// checks in `get_sd_version()`. Holds trivially when nothing matches.
    #[serde(default)]
    pub tensor_ne0: BTreeMap<String, u64>,
    /// If one of these matches, this family wins over the other candidates
    /// (e.g. the `__index_timestep_zero__` marker of Qwen-Image-Edit-2511).
    #[serde(default)]
    pub decisive_tensor: Vec<String>,
    /// Families with the same tensors (informational; filled symmetrically).
    #[serde(default)]
    pub ambiguous_with: Vec<String>,
    /// Copy another family's rules (kept as a separate family).
    #[serde(default)]
    pub same_as: Option<String>,
    /// Files of this family are complete checkpoints whose own tensor names
    /// already include the text encoder (and which may decode in pixel
    /// space, with no VAE), e.g. HiDream-O1 (`model.language_model.*`). A
    /// matching file is always all-in-one (`--model`): `--diffusion-model`
    /// would prefix every name with `model.diffusion_model.` and the engine
    /// would no longer recognise it.
    #[serde(default)]
    pub whole_checkpoint: bool,
}

impl DetectRules {
    /// `true` when the rules can match anything at all.
    pub fn has_positive_rule(&self) -> bool {
        !self.any_tensor.is_empty()
            || !self.all_tensor.is_empty()
            || self.all_of_any.iter().any(|g| !g.is_empty())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FamilyDefaults {
    #[serde(default)]
    pub sampler: Option<String>,
    #[serde(default)]
    pub scheduler: Option<String>,
    #[serde(default)]
    pub clip_skip: Option<i32>,
    #[serde(default)]
    pub negative_prompt: Option<String>,
    #[serde(default)]
    pub guidance: Option<f32>,
    #[serde(default)]
    pub flow_shift: Option<f32>,
    #[serde(default)]
    pub auto_prompt_prefix: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QualitySteps {
    pub fast: u32,
    pub balanced: u32,
    pub best: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HiresAtBest {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub scale: Option<f32>,
    #[serde(default)]
    pub denoising_strength: Option<f32>,
    /// Second-pass steps; `0`/absent = reuse the main step count (server default).
    #[serde(default)]
    pub steps: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DialSpec {
    /// `square` | `portrait` | `landscape` | `wide` → [w, h]
    #[serde(default)]
    pub shape: BTreeMap<String, [u32; 2]>,
    #[serde(default)]
    pub quality: Option<QualitySteps>,
    #[serde(default)]
    pub cfg_range: Option<[f32; 2]>,
    #[serde(default)]
    pub cfg_default: Option<f32>,
    /// When set, CFG is fixed and the "Stick to prompt" dial is hidden
    /// unless `stick_to_prompt_maps_to: guidance`.
    #[serde(default)]
    pub cfg_fixed: Option<f32>,
    #[serde(default)]
    pub stick_to_prompt_maps_to: Option<String>,
    #[serde(default)]
    pub guidance_range: Option<[f32; 2]>,
    /// Edit families: what "Stay close to original" maps to (`cfg` | `guidance`).
    #[serde(default)]
    pub stay_close_maps_to: Option<String>,
    #[serde(default)]
    pub hires_at_best: Option<HiresAtBest>,
    /// Width/height are rounded to a multiple of this (UNet: 64, DiT: 16).
    /// Defaults to 64, which is valid for every family.
    #[serde(default)]
    pub size_multiple: Option<u32>,
}

/// Downloadable file (family `download:`, captioner files, test models).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadSpec {
    pub file: String,
    pub url: String,
    pub sha256: String,
    pub size_mb: u64,
    #[serde(default)]
    pub vram_gb: Option<VramGb>,
    /// quant name (`q8_0`, `q4_k`) → alternative file.
    #[serde(default)]
    pub alt_quants: BTreeMap<String, QuantSpec>,
    /// Family to wire this file as (test models only).
    #[serde(default)]
    pub family: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuantSpec {
    #[serde(default)]
    pub file: Option<String>,
    pub url: String,
    pub sha256: String,
    pub size_mb: u64,
    #[serde(default)]
    pub vram_gb: Option<VramGb>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendedCandidate {
    #[serde(default)]
    pub family: Option<String>,
    /// `registry` | `civitai`
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub civitai_model_id: Option<serde_yaml::Value>,
    #[serde(default)]
    pub civitai_version_id: Option<serde_yaml::Value>,
    /// Filled in once verified: size and VRAM of the CivitAI file.
    #[serde(default)]
    pub size_mb: Option<u64>,
    #[serde(default)]
    pub vram_gb: Option<VramGb>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// Describe role: `reuse` | `default`
    #[serde(default)]
    pub captioner: Option<String>,
    /// Registry candidates: picked only when a version at least this good
    /// (`q6_k`, `q8_0`…) Fits; otherwise the next candidate.
    #[serde(default)]
    pub min_quant: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CaptionerFile {
    pub file: String,
    pub url: String,
    pub sha256: String,
    pub size_mb: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CaptionerDefault {
    pub model: CaptionerFile,
    pub mmproj: CaptionerFile,
}

/// A helper model choice (Models → Helpers, the Describe / Improve pickers).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HelperSpec {
    /// Stable id stored in settings (`describe_model` / `improve_model`).
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub note: String,
    /// Uses the files of `captioner.default`.
    #[serde(default)]
    pub default: bool,
    /// Ids in `components:` (model first, then its vision projector).
    #[serde(default)]
    pub components: Vec<String>,
    /// Only offered while Safe mode is Off.
    #[serde(default)]
    pub needs_safe_off: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CaptionerSpec {
    #[serde(default)]
    pub helpers: Vec<HelperSpec>,
    #[serde(default)]
    pub prefer_reuse: Vec<String>,
    #[serde(default)]
    pub default: Option<CaptionerDefault>,
    #[serde(default)]
    pub idle_shutdown_seconds: u64,
    /// `sentence` | `tags` → VLM instruction.
    #[serde(default)]
    pub prompts: BTreeMap<String, String>,
    /// "Improve my prompt": `natural` | `tags` instructions, `safe` (Safe mode on) and
    /// `avoid` (`{words}` = add-on trigger words) rules appended to them.
    #[serde(default)]
    pub improve: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareProfile {
    pub name: String,
    pub max_vram_gb: f32,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub prefer_quant: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownFile {
    pub sha256: String,
    pub family: String,
    #[serde(default)]
    pub friendly_name: Option<String>,
}
