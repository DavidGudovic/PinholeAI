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
            return ShippedPaths {
                config_dir: candidate,
            };
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config");
    ShippedPaths { config_dir: dev }
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// WebDriver e2e runs (`tests/e2e`, tauri-driver sets `TAURI_WEBVIEW_AUTOMATION=true`).
/// On Linux wry gives an incognito webview its own ephemeral WebKit context with
/// automation switched off, so WebKitWebDriver could never attach. Debug builds
/// only: a release build is always incognito.
fn under_webdriver() -> bool {
    cfg!(debug_assertions) && std::env::var("TAURI_WEBVIEW_AUTOMATION").as_deref() == Ok("true")
}

/// The main window is built here rather than in tauri.conf.json so it can be
/// private: incognito means the WebView keeps no cookies, cache or storage on
/// disk, and in portable mode its profile folder lives inside `Data/` instead
/// of the user's profile.
fn create_main_window(app: &AppHandle, webview_dir: Option<PathBuf>) -> tauri::Result<()> {
    let mut builder = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
        .title("Pinhole")
        .inner_size(1280.0, 860.0)
        .min_inner_size(900.0, 640.0)
        .disable_drag_drop_handler()
        .incognito(!under_webdriver());
    if let Some(dir) = webview_dir {
        builder = builder.data_directory(dir);
    }
    builder.build()?;
    Ok(())
}

/// WebKitGTK's DMA-BUF renderer is slow and glitchy with NVIDIA's proprietary driver
/// (laggy scrolling, gray boxes over images). The widely used fix is to switch it off
/// before GTK starts. Only for that driver, and never over the user's own setting.
fn webkit_env_fixes(nvidia_driver: bool, user_set: bool) -> Option<(&'static str, &'static str)> {
    (cfg!(target_os = "linux") && nvidia_driver && !user_set)
        .then_some(("WEBKIT_DISABLE_DMABUF_RENDERER", "1"))
}

fn apply_webkit_env_fixes() {
    let nvidia = std::path::Path::new("/proc/driver/nvidia/version").exists();
    let user_set = std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_some();
    if let Some((key, value)) = webkit_env_fixes(nvidia, user_set) {
        // Runs first thing in `run`, before any other thread exists.
        std::env::set_var(key, value);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    apply_webkit_env_fixes();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let handle = app.handle().clone();
            pinhole_core::update::cleanup_after_update(&exe_dir());
            let shipped = shipped_paths(&handle);
            let data = pinhole_store::DataDir::resolve(&exe_dir()).map_err(|e| e.to_string())?;
            // Under WebDriver the (non-incognito) profile stays inside the test's Data/.
            let webview_dir =
                (data.portable || under_webdriver()).then(|| data.root.join("webview"));
            let core = AppCore::new(shipped, data, Arc::new(TauriSink(handle.clone())))
                .map_err(|e| e.message)?;
            create_main_window(&handle, webview_dir)?;
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

#[cfg(test)]
mod tests {
    use super::webkit_env_fixes;

    #[test]
    fn dmabuf_renderer_off_only_for_nvidia_on_linux_unless_user_set() {
        let fix = Some(("WEBKIT_DISABLE_DMABUF_RENDERER", "1"));
        assert_eq!(
            webkit_env_fixes(true, false),
            if cfg!(target_os = "linux") { fix } else { None }
        );
        assert_eq!(webkit_env_fixes(true, true), None);
        assert_eq!(webkit_env_fixes(false, false), None);
    }
}
