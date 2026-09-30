//! `models` commands. OWNER: catalog agent. Thin wrappers over `pinhole_core::models`.

use std::sync::Arc;

use pinhole_core::models::{
    AddFileResult, DeletePreview, InstalledHelper, InstalledLora, InstalledModel, PastedResource,
    RecommendedPick, ResolvedResources,
};
use pinhole_core::models_folder::{self, ModelsFolderInfo, ModelsFolderPreview};
use pinhole_core::{AppCore, CoreError, InstallStarted};
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn list_models(core: State<'_, Arc<AppCore>>) -> Result<Vec<InstalledModel>, CoreError> {
    pinhole_core::models::list_models(&core)
}

#[tauri::command]
pub async fn list_loras(core: State<'_, Arc<AppCore>>) -> Result<Vec<InstalledLora>, CoreError> {
    pinhole_core::models::list_loras(&core)
}

#[tauri::command]
pub async fn set_lora_trigger_words(
    core: State<'_, Arc<AppCore>>,
    lora_id: String,
    words: Vec<String>,
) -> Result<InstalledLora, CoreError> {
    pinhole_core::models::set_lora_trigger_words(&core, &lora_id, words)
}

#[tauri::command]
pub async fn get_recommended(
    core: State<'_, Arc<AppCore>>,
) -> Result<Vec<RecommendedPick>, CoreError> {
    pinhole_core::models::get_recommended(&core)
}

#[tauri::command]
pub async fn install_recommended(
    core: State<'_, Arc<AppCore>>,
    role: String,
) -> Result<InstallStarted, CoreError> {
    pinhole_core::models::install_recommended(core.inner(), &role).await
}

#[tauri::command]
pub async fn add_local_model(
    core: State<'_, Arc<AppCore>>,
    path: String,
) -> Result<AddFileResult, CoreError> {
    pinhole_core::models::add_local_model(core.inner(), &path).await
}

#[tauri::command]
pub async fn confirm_family(
    core: State<'_, Arc<AppCore>>,
    token: String,
    family_id: String,
) -> Result<AddFileResult, CoreError> {
    pinhole_core::models::confirm_family(&core, &token, &family_id)
}

#[tauri::command]
pub async fn preview_delete(
    core: State<'_, Arc<AppCore>>,
    model_id: String,
) -> Result<DeletePreview, CoreError> {
    pinhole_core::models::preview_delete(&core, &model_id)
}

#[tauri::command]
pub async fn delete_model(
    core: State<'_, Arc<AppCore>>,
    model_id: String,
) -> Result<(), CoreError> {
    pinhole_core::models::delete_model(&core, &model_id).await
}

/// Paste from CivitAI: ids/hashes only — the pasted prompt never reaches Rust.
#[tauri::command]
pub async fn resolve_civitai_resources(
    core: State<'_, Arc<AppCore>>,
    resources: Vec<PastedResource>,
) -> Result<ResolvedResources, CoreError> {
    pinhole_core::models::resolve_civitai_resources(&core, resources).await
}

#[tauri::command]
pub async fn list_helpers(
    core: State<'_, Arc<AppCore>>,
) -> Result<Vec<InstalledHelper>, CoreError> {
    pinhole_core::models::list_helpers(&core)
}

#[tauri::command]
pub async fn delete_helper(
    core: State<'_, Arc<AppCore>>,
    helper_id: String,
) -> Result<(), CoreError> {
    pinhole_core::models::delete_helper(&core, &helper_id).await
}

/// Models → Installed → "Open folder".
#[tauri::command]
pub async fn open_models_folder(
    handle: AppHandle,
    core: State<'_, Arc<AppCore>>,
) -> Result<(), CoreError> {
    let dir = core.data.models_root();
    if !dir.is_dir() {
        return Err(CoreError::new(
            "not_found",
            format!("Your Models folder isn't available: {}. Connect or mount the drive it's on and restart Pinhole.", dir.display()),
        ));
    }
    super::app::open_folder(&handle, &dir)
}

#[tauri::command]
pub async fn models_folder_info(
    core: State<'_, Arc<AppCore>>,
) -> Result<ModelsFolderInfo, CoreError> {
    Ok(models_folder::info(&core))
}

/// What moving to `folder` (`None` = back to the default) would do.
#[tauri::command]
pub async fn preview_models_folder(
    core: State<'_, Arc<AppCore>>,
    folder: Option<String>,
) -> Result<ModelsFolderPreview, CoreError> {
    models_folder::preview(&core, folder.as_deref())
}

/// Move the models to `folder` (progress: `models-move-progress`), then
/// restart Pinhole so every path uses the new folder. Returns only on failure.
#[tauri::command]
pub async fn change_models_folder(
    handle: AppHandle,
    core: State<'_, Arc<AppCore>>,
    folder: Option<String>,
) -> Result<(), CoreError> {
    let core = core.inner().clone();
    models_folder::change(&core, folder).await?;
    core.shutdown().await;
    handle.restart();
}

super::area_commands![
    list_models,
    list_loras,
    set_lora_trigger_words,
    list_helpers,
    delete_helper,
    open_models_folder,
    models_folder_info,
    preview_models_folder,
    change_models_folder,
    get_recommended,
    install_recommended,
    add_local_model,
    confirm_family,
    preview_delete,
    delete_model,
    resolve_civitai_resources,
];
