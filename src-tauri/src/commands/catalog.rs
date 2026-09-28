//! `catalog` commands. OWNER: catalog agent. Thin wrappers over `pinhole_core::catalog`.
//! Pattern:
//!   #[tauri::command]
//!   pub async fn my_cmd(core: tauri::State<'_, std::sync::Arc<pinhole_core::AppCore>>, arg: T)
//!       -> Result<U, pinhole_core::CoreError> { ... }

super::area_commands![];
