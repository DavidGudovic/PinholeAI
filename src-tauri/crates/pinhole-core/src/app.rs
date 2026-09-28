//! App info, settings, hardware. OWNER: store agent.
use std::sync::Arc;

use pinhole_registry::wiring::HwContext;

use crate::AppCore;

/// Detect hardware in the background, store it, emit `HardwareReady`.
pub fn start_hardware_detection(core: &Arc<AppCore>) {
    let _ = core;
}

/// Effective hardware for wiring/VRAM decisions: Settings overrides applied
/// (GPU pick / force CPU / VRAM override / engine backend). Used by the engine
/// and catalog areas. Before detection finishes: vram 0, backend "cpu".
pub fn hw_context(core: &AppCore) -> HwContext {
    let _ = core;
    todo!("store agent")
}
