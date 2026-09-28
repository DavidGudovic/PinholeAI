//! Test hooks for workspace integration tests (`--features test-util`).
//! OWNER: engine agent (generate hooks) — keep signatures stable, the `tests/`
//! crate (ci agent) depends on them.

use crate::AppCore;

/// Make `generate` talk to an already-running (mock) sd-server at `base_url`
/// instead of installing/spawning the real engine.
pub fn use_external_engine(core: &AppCore, base_url: &str) {
    let _ = (core, base_url);
    todo!("engine agent")
}

/// Register a fake installed model of `family_id` (tiny dummy files on disk,
/// components marked installed) and return its installed-model id.
pub fn register_fake_model(core: &AppCore, family_id: &str) -> String {
    let _ = (core, family_id);
    todo!("engine agent")
}
