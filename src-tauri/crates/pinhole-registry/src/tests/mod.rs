//! In-crate tests. Kept under `src/tests/` so the privacy lint treats them as test code.

mod detect_tests;
mod loading_tests;
mod style_tests;
mod vram_tests;
mod wiring_tests;

use std::path::PathBuf;
use std::sync::OnceLock;

use crate::Registry;

/// `<repo>/config`, the shipped registry.
pub fn config_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../config")
}

pub fn shipped_yaml() -> String {
    std::fs::read_to_string(config_dir().join("models.yaml")).expect("config/models.yaml")
}

/// The real `config/models.yaml`, loaded once.
pub fn shipped() -> &'static Registry {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(|| Registry::load(&config_dir(), None).expect("shipped registry loads"))
}

/// Shipped registry with an overrides document on top.
pub fn with_overrides(overrides: &str) -> Registry {
    Registry::from_yaml(&shipped_yaml(), Some(overrides)).expect("registry with overrides loads")
}

pub fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}
