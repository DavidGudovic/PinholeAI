//! `Data/config/settings.yaml` — app settings only. No prompts, no history.

use serde::{Deserialize, Serialize};

use crate::{DataDir, StoreError};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub offline: bool,
    /// `auto` | `cpu` | `gpu:<index>`
    pub gpu: String,
    /// Manual VRAM override in GB (None = detected).
    pub vram_override_gb: Option<f32>,
    /// `safe` | `include_18plus` | `only_18plus`
    pub content_mode: String,
    pub show_paid: bool,
    /// `none` | `settings` ("Include generation settings (no prompt)")
    pub saved_metadata: String,
    /// `system` | `light` | `dark`
    pub theme: String,
    /// LoRA trigger words added automatically.
    pub add_trigger_words: bool,
    /// First-run flow finished or skipped.
    pub first_run_done: bool,
    /// Engine backend override: `auto` | `cuda` | `vulkan` | `cpu`
    pub engine_backend: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            offline: false,
            gpu: "auto".into(),
            vram_override_gb: None,
            content_mode: "safe".into(),
            show_paid: false,
            saved_metadata: "none".into(),
            theme: "system".into(),
            add_trigger_words: true,
            first_run_done: false,
            engine_backend: "auto".into(),
        }
    }
}

pub fn load(dir: &DataDir) -> Result<Settings, StoreError> {
    let _ = dir;
    todo!("store agent")
}

pub fn save(dir: &DataDir, settings: &Settings) -> Result<(), StoreError> {
    let _ = (dir, settings);
    todo!("store agent")
}
