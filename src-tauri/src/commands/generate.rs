//! `generate` commands. OWNER: engine agent. Thin wrappers over
//! `pinhole_core::{engine_setup, generate, session}` (names/args = src/lib/api.ts).
//!
//! PRIVACY: `req` (GenerateRequest) carries prompt text — never log it.

use std::sync::Arc;

use pinhole_core::events::EngineStatus;
use pinhole_core::generate::{
    self as gen, FinalPromptPreview, GenerateRequest, GenerateResult, ImportedImage, ResultImage,
    SavedImage,
};
use pinhole_core::{engine_setup, session, AppCore, CoreError};
use pinhole_registry::wiring::FamilyUi;
use tauri::ipc::{InvokeBody, Request, Response};
use tauri::State;

type Core<'a> = State<'a, Arc<AppCore>>;

fn join_err(e: impl std::fmt::Display) -> CoreError {
    CoreError::internal("Something went wrong in the background. Try again.")
        .with_details(e.to_string())
}

#[tauri::command]
pub async fn engine_status(core: Core<'_>) -> Result<EngineStatus, CoreError> {
    Ok(engine_setup::engine_status(&core))
}

#[tauri::command]
pub async fn engine_output(core: Core<'_>) -> Result<String, CoreError> {
    Ok(engine_setup::engine_output(&core))
}

#[tauri::command]
pub async fn install_engine(core: Core<'_>) -> Result<EngineStatus, CoreError> {
    engine_setup::install_engine(core.inner()).await
}

#[tauri::command]
pub async fn family_ui(core: Core<'_>, family_id: String) -> Result<FamilyUi, CoreError> {
    gen::family_ui(&core, &family_id)
}

#[tauri::command]
pub async fn generate(core: Core<'_>, req: GenerateRequest) -> Result<GenerateResult, CoreError> {
    gen::generate(core.inner(), req).await
}

#[tauri::command]
pub async fn cancel_generation(core: Core<'_>) -> Result<(), CoreError> {
    gen::cancel(&core);
    Ok(())
}

#[tauri::command]
pub async fn preview_final_prompt(
    core: Core<'_>,
    req: GenerateRequest,
) -> Result<FinalPromptPreview, CoreError> {
    gen::preview_final_prompt(&core, &req)
}

/// Raw binary body (PNG/JPEG/WebP bytes).
#[tauri::command]
pub async fn import_image(
    core: Core<'_>,
    request: Request<'_>,
) -> Result<ImportedImage, CoreError> {
    let bytes = match request.body() {
        InvokeBody::Raw(b) => b.clone(),
        InvokeBody::Json(serde_json::Value::Array(items)) => items
            .iter()
            .map(|v| v.as_u64().filter(|n| *n <= 255).map(|n| n as u8))
            .collect::<Option<Vec<u8>>>()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    if bytes.is_empty() {
        return Err(CoreError::invalid("No image data was received."));
    }
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || session::import_image(&core, bytes))
        .await
        .map_err(join_err)?
}

/// Image bytes as an ArrayBuffer (PNG for generated images).
#[tauri::command]
pub async fn get_image(core: Core<'_>, id: String) -> Result<Response, CoreError> {
    let bytes = session::get(&core, &id)?;
    Ok(Response::new(bytes.as_ref().clone()))
}

#[tauri::command]
pub async fn save_image(core: Core<'_>, id: String) -> Result<SavedImage, CoreError> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || session::save_image(&core, &id))
        .await
        .map_err(join_err)?
}

#[tauri::command]
pub async fn save_image_as(
    core: Core<'_>,
    id: String,
    path: String,
) -> Result<SavedImage, CoreError> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || session::save_image_as(&core, &id, &path))
        .await
        .map_err(join_err)?
}

#[tauri::command]
pub async fn copy_image(
    app: tauri::AppHandle,
    core: Core<'_>,
    id: String,
) -> Result<(), CoreError> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let core = core.inner().clone();
    let (rgba, w, h) =
        tauri::async_runtime::spawn_blocking(move || session::decode_rgba(&core, &id))
            .await
            .map_err(join_err)??;
    app.clipboard()
        .write_image(&tauri::image::Image::new_owned(rgba, w, h))
        .map_err(|e| {
            CoreError::internal("Couldn't copy the image to the clipboard.")
                .with_details(e.to_string())
        })
}

#[tauri::command]
pub async fn discard_image(core: Core<'_>, id: String) -> Result<(), CoreError> {
    session::discard(&core, &id);
    Ok(())
}

/// Drops every in-memory image immediately (and stops sd-server if it holds results).
#[tauri::command]
pub async fn clear_session(core: Core<'_>) -> Result<(), CoreError> {
    session::clear(&core).await;
    Ok(())
}

#[tauri::command]
pub async fn upscale_image(
    core: Core<'_>,
    id: String,
    factor: u32,
) -> Result<ResultImage, CoreError> {
    gen::upscale_image(core.inner(), &id, factor).await
}

super::area_commands![
    engine_status,
    engine_output,
    install_engine,
    family_ui,
    generate,
    cancel_generation,
    preview_final_prompt,
    import_image,
    get_image,
    save_image,
    save_image_as,
    copy_image,
    discard_image,
    clear_session,
    upscale_image,
];
