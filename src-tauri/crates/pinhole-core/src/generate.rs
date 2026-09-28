//! Generate / cancel / save / upscale / prompt preview. OWNER: engine agent.
use crate::AppCore;

/// sd-server process + current job. Engine agent: fill in.
#[derive(Default)]
pub struct GenState {}

pub async fn shutdown(core: &AppCore) {
    let _ = core;
}
