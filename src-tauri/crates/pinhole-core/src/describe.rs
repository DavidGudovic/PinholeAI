//! Describe (img2text) via llama-server. OWNER: engine agent.
use std::sync::Arc;

use crate::{AppCore, CoreResult, InstallStarted};

#[derive(Default)]
pub struct DescribeState {}

pub fn start_idle_watchdog(core: &Arc<AppCore>) {
    let _ = core;
}

pub async fn shutdown(core: &AppCore) {
    let _ = core;
}

/// Queue the default captioner download (used by the Describe tab and by the
/// catalog area for the "describe" recommended role).
pub async fn install_captioner(core: &Arc<AppCore>) -> CoreResult<InstallStarted> {
    let _ = core;
    todo!("engine agent")
}
