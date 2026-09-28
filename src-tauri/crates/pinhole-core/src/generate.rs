//! Generate / cancel / save / upscale / prompt preview. OWNER: engine agent.
use crate::AppCore;

/// sd-server process + current job. Engine agent: fill in.
#[derive(Default)]
pub struct GenState {}

pub async fn shutdown(core: &AppCore) {
    let _ = core;
}

/// Stop sd-server if it currently has `model_id` loaded (called before deleting a model).
pub async fn unload_model(core: &AppCore, model_id: &str) {
    let _ = (core, model_id);
}
