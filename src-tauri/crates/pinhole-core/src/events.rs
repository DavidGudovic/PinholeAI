//! Events pushed to the UI. The Tauri layer maps each variant to an event name:
//! `download-progress`, `generation-progress`, `engine-status`, `models-changed`,
//! `hardware-ready`. PRIVACY: no variant may carry prompt text.

use serde::{Deserialize, Serialize};

pub use pinhole_net::download::GroupStatus;

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
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum CoreEvent {
    Download(GroupStatus),
    Generation(GenerationProgress),
    Engine(EngineStatus),
    ModelsChanged,
    HardwareReady,
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
