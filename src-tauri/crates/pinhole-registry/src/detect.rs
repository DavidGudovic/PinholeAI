//! Header sniffing (SPEC §6 step 3). Reads ONLY the safetensors JSON header or
//! GGUF metadata/tensor-info — never tensor data. Mirrors `get_sd_version()` in
//! stable-diffusion.cpp `src/model_loader.cpp` (pinned engine version). The
//! loader prefixes standalone diffusion files with `model.diffusion_model.`, so
//! rules match on tensor-name suffixes as well as prefixes.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Layout, Registry};

/// Hard cap for a safetensors JSON header (CLAUDE.md: defensive parsing).
pub const MAX_HEADER_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum DetectError {
    #[error("could not read file: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a safetensors or GGUF file")]
    UnknownFormat,
    #[error("header too large or malformed: {0}")]
    Malformed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileFormat {
    Safetensors,
    Gguf,
}

/// What we learned from the header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderInfo {
    pub format: FileFormat,
    pub tensor_names: Vec<String>,
    /// Dominant dtype / quant: `f32`, `f16`, `bf16`, `f8_e4m3`, `q8_0`, `q4_k`…
    pub dtype: String,
    pub file_size: u64,
    /// Sum of tensor byte sizes (≈ weights size).
    pub tensor_bytes: u64,
    /// GGUF `general.architecture` etc., when present.
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, String>,
}

/// Result of matching a header against every family's `detect` rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Detection {
    /// Candidate family ids, best first. Empty = unknown. More than one = ask the user
    /// (after known-hash and CivitAI lookups failed).
    pub candidates: Vec<String>,
    pub layout: Layout,
    pub has_vae: bool,
    pub has_text_encoders: bool,
    pub dtype: String,
    /// `true` when the file is a LoRA rather than a base model.
    pub is_lora: bool,
    /// `true` when the file looks like a standalone component (VAE / text encoder).
    pub is_component: Option<String>,
}

/// Read and parse just the header of a `.safetensors` or `.gguf` file.
pub fn read_header(path: &Path) -> Result<HeaderInfo, DetectError> {
    let _ = path;
    todo!("registry agent")
}

/// Parse a header from bytes (tests / already-open files).
pub fn parse_header_bytes(bytes: &[u8], file_size: u64) -> Result<HeaderInfo, DetectError> {
    let _ = (bytes, file_size);
    todo!("registry agent")
}

/// Match a header against the registry's `detect` rules.
pub fn detect(registry: &Registry, header: &HeaderInfo) -> Detection {
    let _ = (registry, header);
    todo!("registry agent")
}
