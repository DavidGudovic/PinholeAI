//! Data folder resolution: portable (`Data/` next to the exe, if writable) or
//! installed (`%LOCALAPPDATA%\Pinhole\Data`, `~/.local/share/pinhole/Data`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::StoreError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataDir {
    pub root: PathBuf,
    pub portable: bool,
}

/// Model sub-folders under `Data/models/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Checkpoint,
    Diffusion,
    TextEncoder,
    Vae,
    Lora,
    Upscaler,
    Captioner,
    Taesd,
}

impl DataDir {
    /// Portable if `<exe_dir>/Data` exists and is writable; else the per-user dir.
    pub fn resolve(exe_dir: &Path) -> Result<Self, StoreError> {
        let _ = exe_dir;
        todo!("store agent")
    }

    /// Use an explicit root (tests).
    pub fn at(root: PathBuf, portable: bool) -> Self {
        Self { root, portable }
    }

    /// Create the folder layout from SPEC §3 if missing.
    pub fn ensure_layout(&self) -> Result<(), StoreError> {
        todo!("store agent")
    }

    pub fn models(&self, kind: ModelKind) -> PathBuf {
        let _ = kind;
        todo!("store agent")
    }
    pub fn outputs(&self) -> PathBuf { self.root.join("outputs") }
    pub fn presets(&self) -> PathBuf { self.root.join("presets") }
    pub fn styles(&self) -> PathBuf { self.root.join("styles") }
    pub fn config(&self) -> PathBuf { self.root.join("config") }
    pub fn settings_file(&self) -> PathBuf { self.config().join("settings.yaml") }
    pub fn overrides_file(&self) -> PathBuf { self.config().join("overrides.yaml") }
    pub fn installed_file(&self) -> PathBuf { self.root.join("catalog").join("installed.json") }
    pub fn engine(&self) -> PathBuf { self.root.join("engine") }
}
