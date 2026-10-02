//! `library` commands. Thin wrappers over `pinhole_core::library`.

use std::sync::Arc;

use pinhole_core::{library, AppCore, CoreError};
use pinhole_store::presets::Preset;
use pinhole_store::styles::Style;
use tauri::State;

#[tauri::command]
pub async fn list_styles(core: State<'_, Arc<AppCore>>) -> Result<Vec<Style>, CoreError> {
    library::list_styles(&core)
}

/// Explicit user action only ("Save as style").
#[tauri::command]
pub async fn save_style(core: State<'_, Arc<AppCore>>, style: Style) -> Result<Style, CoreError> {
    library::save_style(&core, style)
}

#[tauri::command]
pub async fn delete_style(core: State<'_, Arc<AppCore>>, id: String) -> Result<(), CoreError> {
    library::delete_style(&core, &id)
}

#[tauri::command]
pub async fn list_presets(core: State<'_, Arc<AppCore>>) -> Result<Vec<Preset>, CoreError> {
    library::list_presets(&core)
}

#[tauri::command]
pub async fn save_preset(
    core: State<'_, Arc<AppCore>>,
    preset: Preset,
) -> Result<Preset, CoreError> {
    library::save_preset(&core, preset)
}

#[tauri::command]
pub async fn delete_preset(core: State<'_, Arc<AppCore>>, id: String) -> Result<(), CoreError> {
    library::delete_preset(&core, &id)
}

// Declared last so every command (and its generated `__cmd__*` macro) is defined above.
super::area_commands![
    list_styles,
    save_style,
    delete_style,
    list_presets,
    save_preset,
    delete_preset
];
