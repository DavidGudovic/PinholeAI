//! `catalog` commands. OWNER: catalog agent. Thin wrappers over `pinhole_core::catalog`.

use std::sync::Arc;

use pinhole_core::catalog::{BrowsePage, BrowseQuery, CatalogFilterOptions, ContentMode, InstallPlan};
use pinhole_core::gallery::ModelGallery;
use pinhole_core::{AppCore, CoreError, InstallStarted};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub async fn catalog_filters(core: State<'_, Arc<AppCore>>) -> Result<CatalogFilterOptions, CoreError> {
    pinhole_core::catalog::catalog_filters(&core)
}

#[tauri::command]
pub async fn browse_catalog(core: State<'_, Arc<AppCore>>, query: BrowseQuery) -> Result<BrowsePage, CoreError> {
    pinhole_core::catalog::browse(&core, query).await
}

/// Preview image bytes → `ArrayBuffer` in JS (the WebView makes no network calls).
#[tauri::command]
pub async fn fetch_preview(core: State<'_, Arc<AppCore>>, url: String) -> Result<tauri::ipc::Response, CoreError> {
    let bytes = pinhole_core::catalog::fetch_preview(&core, &url).await?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Model details page: the version's preview images + generation data (memory only).
#[tauri::command]
pub async fn model_gallery(
    core: State<'_, Arc<AppCore>>,
    version_id: u64,
    content: ContentMode,
    model_nsfw: bool,
) -> Result<ModelGallery, CoreError> {
    pinhole_core::gallery::model_gallery(&core, version_id, content, model_nsfw).await
}

/// Open the model's CivitAI page in the system browser (the URL is built in Rust).
#[tauri::command]
pub async fn open_civitai_page(
    handle: AppHandle,
    model_id: u64,
    version_id: Option<u64>,
    nsfw: bool,
) -> Result<(), CoreError> {
    let url = pinhole_core::gallery::civitai_page(model_id, version_id, nsfw)?;
    handle
        .opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|e| CoreError::new("io", format!("Couldn't open your browser. The page is {url}")).with_details(e.to_string()))
}

#[tauri::command]
pub async fn plan_civitai_install(core: State<'_, Arc<AppCore>>, version_id: u64) -> Result<InstallPlan, CoreError> {
    pinhole_core::catalog::plan_civitai_install(&core, version_id).await
}

#[tauri::command]
pub async fn install_civitai(
    core: State<'_, Arc<AppCore>>,
    version_id: u64,
    family_id: Option<String>,
) -> Result<InstallStarted, CoreError> {
    pinhole_core::catalog::install_civitai(core.inner(), version_id, family_id).await
}

#[tauri::command]
pub async fn has_civitai_key(core: State<'_, Arc<AppCore>>) -> Result<bool, CoreError> {
    pinhole_core::catalog::has_civitai_key(&core).await
}

/// The key goes straight to the OS keychain; it is never logged or written to Data/.
#[tauri::command]
pub async fn set_civitai_key(core: State<'_, Arc<AppCore>>, key: String) -> Result<(), CoreError> {
    pinhole_core::catalog::set_civitai_key(&core, key).await
}

#[tauri::command]
pub async fn clear_civitai_key(core: State<'_, Arc<AppCore>>) -> Result<(), CoreError> {
    pinhole_core::catalog::clear_civitai_key(&core).await
}

super::area_commands![
    catalog_filters,
    browse_catalog,
    fetch_preview,
    model_gallery,
    open_civitai_page,
    plan_civitai_install,
    install_civitai,
    has_civitai_key,
    set_civitai_key,
    clear_civitai_key,
];
