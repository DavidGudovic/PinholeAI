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
}

/// Either a fixed component id, or a VRAM-dependent choice
/// (`{ vram_gte_16: t5xxl_fp16, else: t5xxl_fp8 }`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ComponentChoice {
    Fixed(String),
    ByVram(BTreeMap<String, String>),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DetectRules {
    #[serde(default)]
    pub any_tensor: Vec<String>,
    #[serde(default)]
    pub all_tensor: Vec<String>,
    #[serde(default)]
    pub none_tensor: Vec<String>,
    #[serde(default)]
    pub ambiguous_with: Vec<String>,
    #[serde(default)]
    pub same_as: Option<String>,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CaptionerSpec {
    #[serde(default)]
    pub prefer_reuse: Vec<String>,
    #[serde(default)]
    pub default: Option<CaptionerDefault>,
    #[serde(default)]
    pub idle_shutdown_seconds: u64,
    /// `sentence` | `tags` → VLM instruction.
    #[serde(default)]
    pub prompts: BTreeMap<String, String>,
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
