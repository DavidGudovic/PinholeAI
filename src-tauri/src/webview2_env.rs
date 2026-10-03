//! Windows only: the WebView2 environment for the main window.
//!
//! Built like wry's default environment (same browser arguments, UI language
//! and profile folder) plus `IsCustomCrashReportingEnabled`, so WebView2 crash
//! dumps stay on the computer instead of being sent to Microsoft. If creating
//! it fails, the caller builds the window with wry's default environment, so
//! the app still opens.

use std::path::Path;
use std::sync::mpsc;

use webview2_com::Microsoft::Web::WebView2::Win32::{
    CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2Environment,
    ICoreWebView2EnvironmentOptions,
};
use webview2_com::{CoreWebView2EnvironmentOptions, CreateCoreWebView2EnvironmentCompletedHandler};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{E_POINTER, E_UNEXPECTED};
use windows::Win32::Globalization::{
    GetUserDefaultUILanguage, LCIDToLocaleName, LOCALE_ALLOW_NEUTRAL_NAMES, MAX_LOCALE_NAME,
};

/// wry's default browser arguments, which it drops once an environment is
/// supplied: no mini menu, no PDF mini menu, no SmartScreen URL checks.
pub const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

/// Creates the environment with its profile in `data_dir` (the folder Tauri
/// would otherwise pass to wry). Must run on the main thread, like wry does.
pub fn create(data_dir: Option<&Path>) -> webview2_com::Result<ICoreWebView2Environment> {
    if let Some(dir) = data_dir {
        let _ = std::fs::create_dir_all(dir);
    }
    let data_dir = data_dir
        .map(|d| HSTRING::from(d.as_os_str()))
        .unwrap_or_default();
    let options = CoreWebView2EnvironmentOptions::default();
    let (tx, rx) = mpsc::channel();
    unsafe {
        options.set_additional_browser_arguments(BROWSER_ARGS.to_string());
        options.set_is_custom_crash_reporting_enabled(true);
        // The user's Windows display language, as wry sets it.
        let mut lang = [0u16; MAX_LOCALE_NAME as usize];
        let len = LCIDToLocaleName(
            GetUserDefaultUILanguage() as u32,
            Some(&mut lang),
            LOCALE_ALLOW_NEUTRAL_NAMES,
        );
        if len > 1 {
            options.set_language(String::from_utf16_lossy(&lang[..len as usize - 1]));
        }
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            &data_dir,
            &ICoreWebView2EnvironmentOptions::from(options),
            &CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
                move |error_code, environment| {
                    let result = error_code.and_then(|()| {
                        environment.ok_or_else(|| windows::core::Error::from(E_POINTER))
                    });
                    tx.send(result)
                        .map_err(|_| windows::core::Error::from(E_UNEXPECTED))
                },
            )),
        )
        .map_err(webview2_com::Error::WindowsError)?;
    }
    webview2_com::wait_with_pump(rx)?.map_err(webview2_com::Error::WindowsError)
}

#[cfg(test)]
mod tests {
    #[test]
    fn keeps_wrys_default_browser_args() {
        assert!(super::BROWSER_ARGS.contains("msSmartScreenProtection"));
        assert!(super::BROWSER_ARGS.starts_with("--disable-features="));
    }
}
