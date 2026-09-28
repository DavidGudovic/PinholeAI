//! Events pushed to the UI. The Tauri layer maps each variant to an event name:
//! `download-progress`, `generation-progress`, `engine-status`, `models-changed`,
//! `hardware-ready`. PRIVACY: no variant may carry prompt text.

use serde::{Deserialize, Serialize};

pub use pinhole_net::download::{DownloadKind, GroupStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GenPhase {
    /// Starting / restarting sd-server with the model ("Loading <model>… ~10–30 s").
    LoadingModel,
    Queued,
    Generating,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationProgress {
    pub phase: GenPhase,
    pub model_label: Option<String>,
    pub queue_position: Option<u32>,
    /// Current / total sampling step when the engine reports it.
    pub step: Option<u32>,
    pub total_steps: Option<u32>,
    pub elapsed_ms: u64,
    /// Plain-language note for this job (e.g. other programs are using the
    /// graphics memory, or an automatic retry with memory-saving settings).
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub installed: bool,
    pub installing: bool,
    pub version: Option<String>,
    /// `cuda` | `vulkan` | `cpu`
    pub backend: Option<String>,
    pub running: bool,
    pub loading: bool,
    pub loaded_model_id: Option<String>,
    /// Plain-language message of the last engine failure (`CoreError.message`,
    /// says what to do next); cleared when the next model load starts.
    pub error: Option<String>,
    /// `CoreError.code` of that failure (`engine_failed`, `model_load`, `vram`…).
    #[serde(default)]
    pub error_code: Option<String>,
    /// Engine output for the "Details" toggle (`CoreError.details`).
    #[serde(default)]
    pub error_details: Option<String>,
    /// Plain-language note about the running engine: an automatic memory
    /// choice (text encoder on the processor) or other programs using a lot of
    /// graphics memory. Never prompt text.
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum CoreEvent {
    Download(GroupStatus),
    Generation(GenerationProgress),
    Engine(EngineStatus),
    ModelsChanged,
    HardwareReady,
    /// Moving the models to another Models folder (Settings → Models folder).
    ModelsMove(ModelsMoveProgress),
}

/// `ModelsMoveProgress` in src/lib/types.ts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsMoveProgress {
    pub done_bytes: u64,
    pub total_bytes: u64,
    /// File being moved (a model file name, never user text).
    pub file_name: String,
}

impl CoreEvent {
    /// Tauri event name.
    pub fn name(&self) -> &'static str {
        match self {
            CoreEvent::Download(_) => "download-progress",
            CoreEvent::Generation(_) => "generation-progress",
            CoreEvent::Engine(_) => "engine-status",
            CoreEvent::ModelsChanged => "models-changed",
            CoreEvent::HardwareReady => "hardware-ready",
            CoreEvent::ModelsMove(_) => "models-move-progress",
        }
    }
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: CoreEvent);
}

/// Discards events (tests).
pub struct NullSink;
impl EventSink for NullSink {
    fn emit(&self, _event: CoreEvent) {}
}

/// Returned by install commands; progress arrives as `download-progress` for `group_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallStarted {
    pub group_id: String,
}
