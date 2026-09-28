//! `describe` commands. OWNER: engine agent. Thin wrappers over `pinhole_core::describe`.

use std::sync::Arc;

use pinhole_core::describe::{self, CaptionerStatus, DescribeStyle};
use pinhole_core::{AppCore, CoreError, InstallStarted};
use tauri::State;

type Core<'a> = State<'a, Arc<AppCore>>;

#[tauri::command]
pub async fn captioner_status(core: Core<'_>) -> Result<CaptionerStatus, CoreError> {
    Ok(describe::captioner_status(&core))
}

#[tauri::command]
pub async fn install_captioner(core: Core<'_>) -> Result<InstallStarted, CoreError> {
    describe::install_captioner(core.inner()).await
}

#[tauri::command]
pub async fn describe_image(core: Core<'_>, image_id: String, style: DescribeStyle) -> Result<String, CoreError> {
    describe::describe_image(core.inner(), &image_id, style).await
}

super::area_commands![captioner_status, install_captioner, describe_image];
