//! Describe (img2text) via llama-server. OWNER: engine agent.
use std::sync::Arc;

use crate::AppCore;

#[derive(Default)]
pub struct DescribeState {}

pub fn start_idle_watchdog(core: &Arc<AppCore>) {
    let _ = core;
}

pub async fn shutdown(core: &AppCore) {
    let _ = core;
}
