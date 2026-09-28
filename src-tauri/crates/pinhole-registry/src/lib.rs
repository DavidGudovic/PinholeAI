//! Model registry (SPEC §6). Loads `config/models.yaml`, deep-merges
//! `Data/config/overrides.yaml`, resolves `inherits`, and answers every
//! "what is this model / how do I run it" question. Model knowledge lives in
//! YAML; this crate only interprets it.
//!
//! OWNER: registry agent. Public signatures below are the cross-crate contract
//! (see docs/ARCHITECTURE.md). Extend freely; don't break them.

pub mod detect;
pub mod model;
pub mod style;
pub mod vram;
pub mod wiring;

use std::collections::BTreeMap;
use std::path::Path;

pub use model::*;

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("could not read {path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("invalid registry YAML: {0}")]
    Yaml(String),
    #[error("family `{0}` inherits from unknown family `{1}`")]
    BadInherit(String, String),
    #[error("{0}")]
    Invalid(String),
}

/// The loaded, fully resolved registry. Cheap to clone behind an `Arc`.
#[derive(Debug, Clone)]
pub struct Registry {
    pub(crate) file: RegistryFile,
    /// Families with `inherits` / `same_as` already applied.
    pub(crate) families: BTreeMap<String, Family>,
}

impl Registry {
    /// Load `<config_dir>/models.yaml` and deep-merge `overrides` (user wins).
    pub fn load(config_dir: &Path, overrides: Option<&Path>) -> Result<Self, RegistryError> {
        let _ = (config_dir, overrides);
        todo!("registry agent")
    }

    /// Same as [`Registry::load`] but from strings (tests).
    pub fn from_yaml(models_yaml: &str, overrides_yaml: Option<&str>) -> Result<Self, RegistryError> {
        let _ = (models_yaml, overrides_yaml);
        todo!("registry agent")
    }

    pub fn family(&self, id: &str) -> Option<&Family> {
        self.families.get(id)
    }

    pub fn families(&self) -> impl Iterator<Item = &Family> {
        self.families.values()
    }

    pub fn component(&self, id: &str) -> Option<&Component> {
        self.file.components.get(id)
    }

    pub fn components(&self) -> &BTreeMap<String, Component> {
        &self.file.components
    }

    /// Families whose `civitai_base_models` contains `base_model` (exact, case-insensitive).
    pub fn families_for_base_model(&self, base_model: &str) -> Vec<&Family> {
        let _ = base_model;
        todo!("registry agent")
    }

    /// Every CivitAI baseModel string Pinhole can run ("Compatibility" filter).
    pub fn all_civitai_base_models(&self) -> Vec<String> {
        todo!("registry agent")
    }

    pub fn known_file(&self, sha256: &str) -> Option<&KnownFile> {
        let _ = sha256;
        todo!("registry agent")
    }

    /// Hardware tier for this much VRAM (first profile with `max_vram_gb >= vram_gb`).
    pub fn hardware_profile(&self, vram_gb: f32) -> &HardwareProfile {
        let _ = vram_gb;
        todo!("registry agent")
    }

    /// Ranked candidates per role: `realistic`, `anime`, `edit`, `describe`.
    pub fn recommended(&self) -> &BTreeMap<String, Vec<RecommendedCandidate>> {
        &self.file.recommended
    }

    pub fn captioner(&self) -> &CaptionerSpec {
        &self.file.captioner
    }

    /// `natural` → "{prompt}. Style: {style}", `tags` → "{prompt}, {style}".
    pub fn style_template(&self, name: &str) -> Option<&str> {
        self.file.style_templates.get(name).map(String::as_str)
    }

    /// Optional test-only models (engine smoke test), from `test_models:`.
    pub fn test_models(&self) -> &BTreeMap<String, DownloadSpec> {
        &self.file.test_models
    }
}
