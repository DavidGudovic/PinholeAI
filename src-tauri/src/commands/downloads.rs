//! `downloads` commands. Thin wrappers over `pinhole_core::downloads`.

use std::sync::Arc;

use pinhole_core::{AppCore, CoreError};
use pinhole_net::download::GroupStatus;

/// Active download groups plus the last 20 finished ones.
#[tauri::command]
pub async fn list_downloads(
    core: tauri::State<'_, Arc<AppCore>>,
) -> Result<Vec<GroupStatus>, CoreError> {
    Ok(pinhole_core::downloads::list(&core))
}

/// Cancel a queued or running group (JS arg `groupId`). Partial files are kept for resume.
#[tauri::command]
pub async fn cancel_download(
    core: tauri::State<'_, Arc<AppCore>>,
    group_id: String,
) -> Result<(), CoreError> {
    pinhole_core::downloads::cancel(&core, &group_id);
    Ok(())
}

super::area_commands![list_downloads, cancel_download];
