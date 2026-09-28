//! Presets (SPEC §7): model + style reference + dial and Fine-tune values +
//! LoRAs. NEVER the prompt and NEVER the negative prompt (CLAUDE.md rule 1).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{DataDir, StoreError};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PresetLora {
    /// Installed LoRA id, or CivitAI version id for one-click install.
    pub lora_id: Option<String>,
    pub civitai_version_id: Option<u64>,
    pub name: String,
    pub weight: f32,
}

/// Fine-tune values that may be stored (no negative prompt!).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct PresetFineTune {
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
    pub vae_tiling: Option<bool>,
    pub auto_prompt_prefix: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// Family the preset is for.
    pub family: Option<String>,
    /// Installed model id (if any) and CivitAI version for one-click install.
    pub model_id: Option<String>,
    pub civitai_version_id: Option<u64>,
    pub style_id: Option<String>,
    /// `square` | `portrait` | `landscape` | `wide`
    pub shape: Option<String>,
    /// `fast` | `balanced` | `best`
    pub quality: Option<String>,
    pub stick: Option<f32>,
    pub count: Option<u32>,
    #[serde(default)]
    pub fine_tune: PresetFineTune,
    #[serde(default)]
    pub loras: Vec<PresetLora>,
    #[serde(default)]
    pub builtin: bool,
}

pub fn list(builtin_dir: &Path, dir: &DataDir) -> Result<Vec<Preset>, StoreError> {
    let _ = (builtin_dir, dir);
    todo!("store agent")
}

pub fn save(dir: &DataDir, preset: Preset) -> Result<Preset, StoreError> {
    let _ = (dir, preset);
    todo!("store agent")
}

pub fn delete(dir: &DataDir, id: &str) -> Result<(), StoreError> {
    let _ = (dir, id);
    todo!("store agent")
}
