//! Tauri shell: builds `AppCore`, bridges core events to the WebView, and
//! dispatches commands. Each `commands::<area>` module owns its own command
//! list (see `area_commands!`), so agents never edit this file to add one.

mod commands;

use std::path::PathBuf;
use std::sync::Arc;

use pinhole_core::{AppCore, CoreEvent, EventSink, ShippedPaths};
use tauri::{AppHandle, Emitter, Manager};

struct TauriSink(AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: CoreEvent) {
        let name = event.name();
        let payload = match serde_json::to_value(&event) {
            Ok(v) => v.get("payload").cloned().unwrap_or(serde_json::Value::Null),
            Err(_) => serde_json::Value::Null,
        };
        let _ = self.0.emit(name, payload);
    }
}

/// Shipped config: bundled resources in release, the repo's `config/` in dev.
fn shipped_paths(app: &AppHandle) -> ShippedPaths {
    if let Ok(res) = app.path().resource_dir() {
        let candidate = res.join("config");
        if candidate.join("models.yaml").exists() {
            return ShippedPaths { config_dir: candidate };
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("config");
    ShippedPaths { config_dir: dev }
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let shipped = shipped_paths(&handle);
            let data = pinhole_store::DataDir::resolve(&exe_dir()).map_err(|e| e.to_string())?;
            let core = AppCore::new(shipped, data, Arc::new(TauriSink(handle.clone()))).map_err(|e| e.message)?;
            {
                let core = core.clone();
                tauri::async_runtime::block_on(async move { core.start_background() });
            }
            app.manage(core);
            Ok(())
        })
        .invoke_handler(commands::dispatch)
        .build(tauri::generate_context!())
        .expect("error while building Pinhole")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(core) = app.try_state::<Arc<AppCore>>() {
                    let core = core.inner().clone();
                    tauri::async_runtime::block_on(async move { core.shutdown().await });
                }
            }
        });
}
