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
    CoreError::from(e)
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
    use crate::app::tests::test_core;
    use crate::NullSink;
    use std::sync::Arc;

    const SENTINEL: &str = "PINHOLE_SENTINEL_7f3a";

    #[tokio::test]
    async fn styles_and_presets_through_core() {
        let (_t, core) = test_core(Arc::new(NullSink));

        let styles = list_styles(&core).unwrap();
        assert!(styles
            .iter()
            .any(|s| s.id == "builtin:film-photo" && s.builtin));
        let saved = save_style(
            &core,
            Style {
                id: String::new(),
                name: "Mine".into(),
                positive: "soft window light".into(),
                negative: Some("plastic skin".into()),
                families: vec![],
                thumbnail: None,
                builtin: false,
            },
        )
        .unwrap();
        assert_eq!(saved.id, "mine");
        assert!(core.data.styles().join("mine.yaml").is_file());
        assert_eq!(get_style(&core, &saved.id).unwrap(), saved);
        assert_eq!(
            get_style(&core, "builtin:film-photo").unwrap().name,
            "Film photo"
        );
        let e = delete_style(&core, "builtin:film-photo").unwrap_err();
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("invalid", "Built-in styles can't be deleted.")
        );
        delete_style(&core, &saved.id).unwrap();
        assert_eq!(
            delete_style(&core, &saved.id).unwrap_err().code,
            "not_found"
        );

        let presets = list_presets(&core).unwrap();
        assert!(presets.iter().filter(|p| p.builtin).count() >= 3);
        // A preset sent by the UI with prompt fields: they never reach disk.
        let json = serde_json::json!({
            "id": "", "name": "Copy", "prompt": SENTINEL, "family": "sdxl", "modelId": null,
            "civitaiVersionId": null, "styleId": "builtin:film-photo", "shape": "square",
            "quality": "fast", "stick": 0.5, "count": 2,
            "fineTune": { "negativePrompt": SENTINEL, "steps": 12 }, "loras": [], "builtin": false
        });
        let copy = save_preset(&core, serde_json::from_value(json).unwrap()).unwrap();
        assert!(!copy.builtin);
        assert_eq!(get_preset(&core, &copy.id).unwrap(), copy);
        let text =
            std::fs::read_to_string(core.data.presets().join(format!("{}.yaml", copy.id))).unwrap();
        assert!(!text.contains(SENTINEL));
        assert_eq!(
            delete_preset(&core, "builtin:photo-portrait")
                .unwrap_err()
                .code,
            "invalid"
        );
        delete_preset(&core, &copy.id).unwrap();
    }

    #[test]
    fn error_mapping_keeps_plain_messages() {
        let e = store_err(StoreError::Invalid(
            "Built-in styles can't be deleted.".into(),
        ));
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("invalid", "Built-in styles can't be deleted.")
        );
        let e = store_err(StoreError::NotFound("Style".into()));
        assert_eq!(e.code, "not_found");
        let e = store_err(StoreError::Keychain(
            "Your system keychain isn't available".into(),
        ));
        assert_eq!(e.message, "Your system keychain isn't available");
    }
}
