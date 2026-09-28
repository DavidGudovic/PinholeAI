//! `app` commands. OWNER: store agent. Thin wrappers over `pinhole_core::app`.

use std::path::Path;
use std::sync::Arc;

use pinhole_core::app::{self, AppInfo, HardwareView};
use pinhole_core::{AppCore, CoreError};
use pinhole_store::Settings;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub async fn app_info(core: State<'_, Arc<AppCore>>) -> Result<AppInfo, CoreError> {
    Ok(app::app_info(&core))
}

#[tauri::command]
pub async fn get_settings(core: State<'_, Arc<AppCore>>) -> Result<Settings, CoreError> {
    Ok(app::get_settings(&core))
}

#[tauri::command]
pub async fn set_settings(core: State<'_, Arc<AppCore>>, settings: Settings) -> Result<Settings, CoreError> {
    app::set_settings(&core, settings)
}

#[tauri::command]
pub async fn get_hardware(core: State<'_, Arc<AppCore>>) -> Result<HardwareView, CoreError> {
    Ok(app::hardware_view(&core))
}

#[tauri::command]
pub async fn open_data_folder(handle: AppHandle, core: State<'_, Arc<AppCore>>) -> Result<(), CoreError> {
    let dir = app::data_folder(&core)?;
    open_folder(&handle, &dir)
}

#[tauri::command]
pub async fn open_outputs_folder(handle: AppHandle, core: State<'_, Arc<AppCore>>) -> Result<(), CoreError> {
    let dir = app::outputs_folder(&core)?;
    open_folder(&handle, &dir)
}

/// Open a folder in the system file manager (no network, no WebView navigation).
fn open_folder(handle: &AppHandle, dir: &Path) -> Result<(), CoreError> {
    handle.opener().open_path(dir.to_string_lossy(), None::<&str>).map_err(|e| {
        CoreError::new(
            "io",
            format!("Couldn't open the folder. You can find it here: {}", dir.display()),
        )
        .with_details(e.to_string())
    })
}

// Declared last so every command (and its generated `__cmd__*` macro) is defined above.
super::area_commands![app_info, get_settings, set_settings, get_hardware, open_data_folder, open_outputs_folder];
