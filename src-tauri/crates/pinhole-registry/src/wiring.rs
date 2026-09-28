//! Wiring (SPEC §6): family + installed files + hardware → `sd-server` launch
//! arguments and concrete request parameters. Pure functions, no I/O.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Family, Layout, Registry};

/// Files that make up one runnable model set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelFiles {
    pub family_id: String,
    /// Checkpoint (all-in-one) or diffusion-only file.
    pub main: PathBuf,
    pub layout: Layout,
    /// component kind (`vae`, `clip_l`, `clip_g`, `t5xxl`, `llm`, `llm_vision`, `taesd`) → path.
    pub components: BTreeMap<String, PathBuf>,
}

/// Hardware facts wiring needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HwContext {
    /// Effective VRAM (detected or user override). 0 for CPU-only.
    pub vram_gb: f32,
    /// `cuda` | `vulkan` | `cpu`
    pub backend: String,
}

/// Extra directories passed at launch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LaunchExtras {
    pub lora_dir: Option<PathBuf>,
    pub upscalers_dir: Option<PathBuf>,
    /// Force VAE tiling on (Fine-tune override).
    pub vae_tiling: Option<bool>,
    /// Enable TAESD preview decoding if the family has one installed.
    pub use_taesd: bool,
}

/// A component the family needs: `(kind, component_id)`, VRAM choice resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredComponent {
    pub kind: String,
    pub component_id: String,
}

/// Components required to run `family` on this hardware (VRAM-dependent choices
/// resolved; `vae_override` included for all-in-one families; TAESD excluded —
/// see [`optional_components`]).
pub fn required_components(registry: &Registry, family: &Family, hw: &HwContext) -> Vec<RequiredComponent> {
    let _ = (registry, family, hw);
    todo!("registry agent")
}

/// Nice-to-have components (TAESD live preview).
pub fn optional_components(registry: &Registry, family: &Family) -> Vec<RequiredComponent> {
    let _ = (registry, family);
    todo!("registry agent")
}

/// Build `sd-server` arguments for a model set (without `--listen-ip/--listen-port`
/// and `--log-level`, which the engine manager adds). Includes `--diffusion-model`
/// or `--model`, component flags (`--vae`, `--clip_l`, `--t5xxl`, `--llm`,
/// `--llm_vision`…), family `flags`, hardware-profile flags and extras.
pub fn launch_args(registry: &Registry, files: &ModelFiles, hw: &HwContext, extras: &LaunchExtras) -> Vec<String> {
    let _ = (registry, files, hw, extras);
    todo!("registry agent")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Shape {
    Square,
    Portrait,
    Landscape,
    Wide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    Fast,
    Balanced,
    Best,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GenMode {
    Txt2img,
    Img2img,
    Edit,
}

/// Fine-tune drawer overrides. `None` = use the registry default.
/// PRIVACY: `negative_prompt` is user text — never persist or log this struct.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FineTune {
    pub sampler: Option<String>,
    pub scheduler: Option<String>,
    pub steps: Option<u32>,
    pub cfg: Option<f32>,
    pub guidance: Option<f32>,
    pub seed: Option<i64>,
    pub flow_shift: Option<f32>,
    pub clip_skip: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub hires: Option<bool>,
    pub hires_scale: Option<f32>,
    pub hires_denoise: Option<f32>,
    pub vae_tiling: Option<bool>,
    pub negative_prompt: Option<String>,
    /// Add the family's `auto_prompt_prefix` (e.g. Pony score tags). Default true.
    pub auto_prompt_prefix: Option<bool>,
}

/// Dial positions from the simple UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dials {
    pub shape: Shape,
    pub quality: Quality,
    /// "Stick to prompt" / "Stay close to original", 0.0 (loose) ..= 1.0 (strict).
    pub stick: f32,
    /// 1 | 2 | 4
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HiresParams {
    pub scale: f32,
    pub denoising_strength: f32,
    pub steps: u32,
}

/// Concrete parameters for one `img_gen` request (prompt text excluded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedParams {
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg: f32,
    pub guidance: Option<f32>,
    pub sampler: Option<String>,
    pub scheduler: Option<String>,
    pub clip_skip: Option<i32>,
    pub flow_shift: Option<f32>,
    pub hires: Option<HiresParams>,
    pub vae_tiling: bool,
    pub batch_count: u32,
}

/// Map dials + fine-tune overrides to concrete parameters for `family`.
pub fn resolve_params(registry: &Registry, family: &Family, dials: &Dials, fine: &FineTune, mode: GenMode, hw: &HwContext) -> ResolvedParams {
    let _ = (registry, family, dials, fine, mode, hw);
    todo!("registry agent")
}

/// Everything the UI needs to render dials + Fine-tune defaults for a family.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyUi {
    pub family_id: String,
    pub label: String,
    pub modes: Vec<String>,
    /// shape name → [w, h]
    pub shapes: BTreeMap<String, [u32; 2]>,
    pub quality_steps: [u32; 3],
    /// Show the "Stick to prompt" dial.
    pub show_stick: bool,
    /// What the dial maps to: `cfg` | `guidance`.
    pub stick_maps_to: String,
    pub stick_range: [f32; 2],
    pub stick_default: f32,
    pub uses_negative_prompt: bool,
    pub default_negative_prompt: Option<String>,
    pub default_sampler: Option<String>,
    pub default_scheduler: Option<String>,
    pub default_clip_skip: Option<i32>,
    pub default_flow_shift: Option<f32>,
    pub default_cfg: f32,
    pub default_guidance: Option<f32>,
    pub auto_prompt_prefix: Option<String>,
    pub hires_at_best: bool,
    pub license_note: Option<String>,
    pub is_edit_family: bool,
}

pub fn family_ui(registry: &Registry, family: &Family) -> FamilyUi {
    let _ = (registry, family);
    todo!("registry agent")
}
