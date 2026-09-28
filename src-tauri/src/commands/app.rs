//! `app` commands. OWNER: store agent. Thin wrappers over `pinhole_core::app`.

use std::path::Path;
use std::sync::Arc;

use pinhole_core::app::{self, AppInfo, HardwareView};
use pinhole_core::update::{self, Prepared, UpdateCheck};
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

/// The app's own version (tauri.conf.json, which the release workflow checks
/// against the tag).
fn app_version(handle: &AppHandle) -> String {
    handle.package_info().version.to_string()
}

/// Settings → "Check for updates". Only ever runs on that button press.
#[tauri::command]
pub async fn check_for_updates(handle: AppHandle, core: State<'_, Arc<AppCore>>) -> Result<UpdateCheck, CoreError> {
    update::check_for_updates(&core, &app_version(&handle)).await
}

/// Download + verify `version`, put it in place, stop the engines, then run the
/// installer / relaunch and quit. Returns only on failure: before anything was
/// replaced, or (`update_restart`) when the new files are in place but the new
/// version couldn't be started.
#[tauri::command]
pub async fn install_update(handle: AppHandle, core: State<'_, Arc<AppCore>>, version: String) -> Result<(), CoreError> {
    let core = core.inner().clone();
    let prepared = update::install_update(&core, &app_version(&handle), &version).await?;
    core.shutdown().await;
    let started = match &prepared {
        Prepared::RunInstaller(path) => std::process::Command::new(path).args(update::INSTALLER_ARGS).spawn(),
        Prepared::Relaunch(exe) => std::process::Command::new(exe).spawn(),
    };
    if let Err(e) = started {
        let message = match prepared {
            Prepared::RunInstaller(_) => "The update is downloaded but its installer couldn't be started. Close Pinhole, open it again and try again.",
            Prepared::Relaunch(_) => "Pinhole was updated but couldn't restart itself. Close Pinhole and open it again to finish.",
        };
        return Err(CoreError::new("update_restart", message).with_details(e.to_string()));
    }
    handle.exit(0);
    Ok(())
}

/// Open the GitHub release page in the system browser (the WebView never navigates).
#[tauri::command]
pub async fn open_release_page(handle: AppHandle, version: Option<String>) -> Result<(), CoreError> {
    let url = update::release_page_url(version.as_deref())?;
    handle
        .opener()
        .open_url(url.clone(), None::<&str>)
        .map_err(|e| CoreError::new("io", format!("Couldn't open your browser. The page is: {url}")).with_details(e.to_string()))
}

// Declared last so every command (and its generated `__cmd__*` macro) is defined above.
super::area_commands![
    app_info,
    get_settings,
    set_settings,
    get_hardware,
    open_data_folder,
    open_outputs_folder,
    check_for_updates,
    install_update,
    open_release_page
];
