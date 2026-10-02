//! `describe` commands. Thin wrappers over `pinhole_core::describe`.

use std::sync::Arc;

use pinhole_core::describe::{self, CaptionerStatus, DescribeStyle};
use pinhole_core::{AppCore, CoreError, InstallStarted};
use tauri::State;

type Core<'a> = State<'a, Arc<AppCore>>;

#[tauri::command]
pub async fn captioner_status(
    core: Core<'_>,
    purpose: Option<String>,
) -> Result<CaptionerStatus, CoreError> {
    let purpose = describe::Purpose::parse(purpose.as_deref().unwrap_or("describe"));
    Ok(describe::captioner_status(&core, purpose))
}

#[tauri::command]
pub async fn install_captioner(
    core: Core<'_>,
    helper_id: Option<String>,
) -> Result<InstallStarted, CoreError> {
    describe::install_captioner(core.inner(), helper_id.as_deref()).await
}

#[tauri::command]
pub async fn list_helper_models(core: Core<'_>) -> Result<Vec<describe::HelperModel>, CoreError> {
    Ok(describe::list_helper_models(&core))
}

#[tauri::command]
pub async fn describe_image(
    core: Core<'_>,
    image_id: String,
    style: DescribeStyle,
) -> Result<String, CoreError> {
    describe::describe_image(core.inner(), &image_id, style).await
}

#[tauri::command]
pub async fn improve_prompt(
    core: Core<'_>,
    prompt: String,
    family_id: Option<String>,
    avoid: Vec<String>,
) -> Result<describe::ImprovedPrompt, CoreError> {
    describe::improve_prompt(core.inner(), &prompt, family_id.as_deref(), &avoid).await
}

super::area_commands![
    captioner_status,
    install_captioner,
    list_helper_models,
    describe_image,
    improve_prompt
];
