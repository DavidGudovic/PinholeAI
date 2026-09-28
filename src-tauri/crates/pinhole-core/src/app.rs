//! App info, settings, hardware. OWNER: store agent.
use std::sync::Arc;

use crate::AppCore;

/// Detect hardware in the background, store it, emit `HardwareReady`.
pub fn start_hardware_detection(core: &Arc<AppCore>) {
    let _ = core;
}
