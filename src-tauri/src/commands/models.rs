//! `models` commands. OWNER: catalog agent. Thin wrappers over `pinhole_core::models`.

use std::sync::Arc;

use pinhole_core::models::{
    AddFileResult, DeletePreview, InstalledLora, InstalledModel, PastedResource, RecommendedPick, ResolvedResources,
};
use pinhole_core::{AppCore, CoreError, InstallStarted};
use tauri::State;

#[tauri::command]
pub async fn list_models(core: State<'_, Arc<AppCore>>) -> Result<Vec<InstalledModel>, CoreError> {
    pinhole_core::models::list_models(&core)
}

#[tauri::command]
pub async fn list_loras(core: State<'_, Arc<AppCore>>) -> Result<Vec<InstalledLora>, CoreError> {
    pinhole_core::models::list_loras(&core)
}

#[tauri::command]
pub async fn get_recommended(core: State<'_, Arc<AppCore>>) -> Result<Vec<RecommendedPick>, CoreError> {
    pinhole_core::models::get_recommended(&core)
}

#[tauri::command]
pub async fn install_recommended(core: State<'_, Arc<AppCore>>, role: String) -> Result<InstallStarted, CoreError> {
    pinhole_core::models::install_recommended(core.inner(), &role).await
}

#[tauri::command]
pub async fn add_local_model(core: State<'_, Arc<AppCore>>, path: String) -> Result<AddFileResult, CoreError> {
    pinhole_core::models::add_local_model(core.inner(), &path).await
}

#[tauri::command]
pub async fn confirm_family(core: State<'_, Arc<AppCore>>, token: String, family_id: String) -> Result<AddFileResult, CoreError> {
    pinhole_core::models::confirm_family(&core, &token, &family_id)
}

#[tauri::command]
pub async fn preview_delete(core: State<'_, Arc<AppCore>>, model_id: String) -> Result<DeletePreview, CoreError> {
    pinhole_core::models::preview_delete(&core, &model_id)
}

#[tauri::command]
pub async fn delete_model(core: State<'_, Arc<AppCore>>, model_id: String) -> Result<(), CoreError> {
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

super::area_commands![
    list_models,
    list_loras,
    get_recommended,
    install_recommended,
    add_local_model,
    confirm_family,
    preview_delete,
    delete_model,
    resolve_civitai_resources,
];
