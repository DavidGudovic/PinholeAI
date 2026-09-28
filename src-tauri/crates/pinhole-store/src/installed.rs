//! `Data/catalog/installed.json` — index of installed files: path, sha256,
//! family, CivitAI ids, observed VRAM. Never contains prompts.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::datadir::ModelKind;
use crate::{DataDir, StoreError};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CivitaiRef {
    pub model_id: u64,
    pub version_id: u64,
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub version_name: Option<String>,
    #[serde(default)]
    pub base_model: Option<String>,
    /// LoRA trigger words (`trainedWords`).
    #[serde(default)]
    pub trained_words: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledFile {
    /// Stable id (uuid).
    pub id: String,
    /// Path relative to the Data root, `/`-separated.
    pub rel_path: String,
    pub kind: ModelKind,
    pub sha256: String,
    pub size_bytes: u64,
    /// Registry family id (main models and LoRAs).
    #[serde(default)]
    pub family: Option<String>,
    /// Registry component id when this file is a shared component.
    #[serde(default)]
    pub component_id: Option<String>,
    pub friendly_name: String,
    #[serde(default)]
    pub civitai: Option<CivitaiRef>,
    /// Unix seconds.
    pub added_at: i64,
    #[serde(default)]
    pub last_used: Option<i64>,
    /// Observed peak VRAM (GB) after a real run — a number only.
    #[serde(default)]
    pub observed_vram_gb: Option<f32>,
    /// Dominant dtype/quant from the header (`bf16`, `q4_k`…).
    #[serde(default)]
    pub dtype: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledIndex {
    pub schema_version: u32,
    pub files: Vec<InstalledFile>,
}

impl InstalledIndex {
    pub fn load(dir: &DataDir) -> Result<Self, StoreError> {
        let _ = dir;
        todo!("store agent")
    }
    pub fn save(&self, dir: &DataDir) -> Result<(), StoreError> {
        let _ = dir;
        todo!("store agent")
    }
    pub fn get(&self, id: &str) -> Option<&InstalledFile> {
        self.files.iter().find(|f| f.id == id)
    }
    pub fn find_by_sha(&self, sha256: &str) -> Option<&InstalledFile> {
        self.files.iter().find(|f| f.sha256.eq_ignore_ascii_case(sha256))
    }
    pub fn find_component(&self, component_id: &str) -> Option<&InstalledFile> {
        self.files.iter().find(|f| f.component_id.as_deref() == Some(component_id))
    }
    /// Insert or replace (by id).
    pub fn upsert(&mut self, file: InstalledFile) {
        let _ = file;
        todo!("store agent")
    }
    pub fn remove(&mut self, id: &str) -> Option<InstalledFile> {
        let _ = id;
        todo!("store agent")
    }
    pub fn abs_path(&self, dir: &DataDir, file: &InstalledFile) -> PathBuf {
        let _ = (dir, file);
        todo!("store agent")
    }
    /// Main models: checkpoints + diffusion files.
    pub fn models(&self) -> impl Iterator<Item = &InstalledFile> {
        self.files.iter().filter(|f| matches!(f.kind, ModelKind::Checkpoint | ModelKind::Diffusion))
    }
    pub fn loras(&self) -> impl Iterator<Item = &InstalledFile> {
        self.files.iter().filter(|f| f.kind == ModelKind::Lora)
    }
}
