//! Styles + presets service. OWNER: store agent.
//!
//! Styles are the only user text Pinhole stores, and only through
//! [`save_style`] (the explicit "Save as style" action). Presets never contain
//! a prompt or negative prompt (their types have no such fields).

use pinhole_store::presets::{self, Preset};
use pinhole_store::styles::{self, Style};
use pinhole_store::StoreError;

use crate::{AppCore, CoreError, CoreResult};

/// `StoreError` → `CoreError` keeping the plain-language message and the right
/// code (`invalid` for "Built-in styles can't be deleted.", `not_found`, `io`).
/// Also suitable for keychain errors (`pinhole_store::keychain`).
pub fn store_err(e: StoreError) -> CoreError {
    CoreError::new(e.code(), e.user_message())
}

/// Built-in styles (`builtin:<name>`, read-only) then the user's, each sorted by name.
pub fn list_styles(core: &AppCore) -> CoreResult<Vec<Style>> {
    styles::list(&core.shipped.styles(), &core.data).map_err(store_err)
}

/// Any style, built-in or user.
pub fn get_style(core: &AppCore, id: &str) -> CoreResult<Style> {
    styles::get(&core.shipped.styles(), &core.data, id).map_err(store_err)
}

/// Create (empty `id`) or update a user style in `Data/styles/<slug>.yaml`.
/// Explicit user action only.
pub fn save_style(core: &AppCore, style: Style) -> CoreResult<Style> {
    styles::save(&core.data, style).map_err(store_err)
}

/// Delete a user style; built-ins are refused with a plain message.
pub fn delete_style(core: &AppCore, id: &str) -> CoreResult<()> {
    styles::delete(&core.data, id).map_err(store_err)
}

/// Built-in presets (`builtin:<name>`, read-only) then the user's, each sorted by name.
pub fn list_presets(core: &AppCore) -> CoreResult<Vec<Preset>> {
    presets::list(&core.shipped.presets(), &core.data).map_err(store_err)
}

/// Any preset, built-in or user.
pub fn get_preset(core: &AppCore, id: &str) -> CoreResult<Preset> {
    presets::get(&core.shipped.presets(), &core.data, id).map_err(store_err)
}

/// Create (empty `id`) or update a user preset in `Data/presets/<slug>.yaml`.
pub fn save_preset(core: &AppCore, preset: Preset) -> CoreResult<Preset> {
    presets::save(&core.data, preset).map_err(store_err)
}

/// Delete a user preset; built-ins are refused with a plain message.
pub fn delete_preset(core: &AppCore, id: &str) -> CoreResult<()> {
    presets::delete(&core.data, id).map_err(store_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_mapping_keeps_plain_messages() {
        let e = store_err(StoreError::Invalid("Built-in styles can't be deleted.".into()));
        assert_eq!((e.code.as_str(), e.message.as_str()), ("invalid", "Built-in styles can't be deleted."));
        let e = store_err(StoreError::NotFound("Style".into()));
        assert_eq!(e.code, "not_found");
        let e = store_err(StoreError::Keychain("Your system keychain isn't available".into()));
        assert_eq!(e.message, "Your system keychain isn't available");
    }
}
