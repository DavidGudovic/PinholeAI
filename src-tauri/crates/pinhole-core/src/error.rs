//! User-facing error type returned by every core call / Tauri command.
//! `message` is plain language and says what to do next (CLAUDE.md UX rules);
//! `details` holds technical output (engine ring buffer tail) for the
//! "Details" toggle. PRIVACY: never put prompt text in either field.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct CoreError {
    /// Stable machine code: `offline`, `not_found`, `disk_space`, `vram`,
    /// `engine_missing`, `engine_failed`, `model_load`, `cancelled`, `unauthorized`,
    /// `hash_mismatch`, `invalid`, `network`, `io`, `internal`.
    /// `engine_missing` means exactly "the image engine isn't installed for the
    /// current backend": the UI offers `install_engine` as the fix.
    /// `model_load` = the engine couldn't load the model file (UI: "Open Models").
    pub code: String,
    pub message: String,
    pub details: Option<String>,
}

pub type CoreResult<T> = Result<T, CoreError>;

impl CoreError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), details: None }
    }
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }
    pub fn not_found(what: impl Into<String>) -> Self {
        Self::new("not_found", what)
    }
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::new("invalid", msg)
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new("internal", msg)
    }
}

// Every `From` keeps raw library / OS text out of `message` (plain words,
// says what to do next) and puts it in `details` for the "Details" toggle.

impl From<pinhole_store::StoreError> for CoreError {
    fn from(e: pinhole_store::StoreError) -> Self {
        let details = e.details();
        let err = Self::new(e.code(), e.user_message());
        match details {
            Some(d) => err.with_details(d),
            None => err,
        }
    }
}

impl From<pinhole_registry::RegistryError> for CoreError {
    fn from(e: pinhole_registry::RegistryError) -> Self {
        Self::new(
            "invalid",
            "Pinhole's list of models couldn't be loaded. If you edited Data/config/overrides.yaml, undo that change; otherwise reinstall Pinhole.",
        )
        .with_details(e.to_string())
    }
}

impl From<pinhole_net::NetError> for CoreError {
    fn from(e: pinhole_net::NetError) -> Self {
        use pinhole_net::NetError as N;
        match e {
            N::Offline => Self::new("offline", "Offline mode is on. Turn it off in Settings to browse or download."),
            N::Unauthorized(_) => Self::new("unauthorized", "CivitAI needs an API key for this download. Add one in Settings."),
            // The transport text ("error sending request: tunnel error…") is for the Details toggle.
            other => Self::new("network", "Network problem — check your connection and try again.").with_details(other.to_string()),
        }
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        Self::new("io", pinhole_store::IO_MESSAGE).with_details(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_store_and_registry_errors_keep_raw_text_in_details() {
        let os = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Access is denied. (os error 5)");
        let e = CoreError::from(os);
        assert_eq!(e.code, "io");
        assert_eq!(e.message, "Pinhole couldn't read or write a file. Check that the disk isn't full and that Pinhole's Data folder is writable.");
        assert!(!e.message.contains("os error"));
        assert_eq!(e.details.as_deref(), Some("Access is denied. (os error 5)"));

        let e = CoreError::from(pinhole_store::StoreError::Io(std::io::Error::other("No space left on device (os error 28)")));
        assert_eq!((e.code.as_str(), e.message.as_str()), ("io", pinhole_store::IO_MESSAGE));
        assert!(e.details.unwrap().contains("os error 28"));

        let e = CoreError::from(pinhole_store::StoreError::Parse { path: "/d/Data/config/settings.yaml".into(), msg: "invalid type: map".into() });
        assert!(e.message.contains("(settings.yaml)"), "{}", e.message);
        assert!(!e.message.contains("invalid type"), "{}", e.message);
        assert_eq!(e.details.as_deref(), Some("/d/Data/config/settings.yaml: invalid type: map"));
        // User-facing store messages carry no details.
        let e = CoreError::from(pinhole_store::StoreError::Invalid("Built-in styles can't be deleted.".into()));
        assert_eq!((e.code.as_str(), e.details), ("invalid", None));

        let e = CoreError::from(pinhole_registry::Registry::from_yaml("families: [", None).unwrap_err());
        assert_eq!(e.code, "invalid");
        assert!(e.message.starts_with("Pinhole's list of models couldn't be loaded"), "{}", e.message);
        assert!(e.details.unwrap().contains("models.yaml"));
    }

    #[test]
    fn network_error_keeps_transport_text_in_details() {
        let e = CoreError::from(pinhole_net::NetError::Transport("error sending request: tunnel error: unsuccessful".into()));
        assert_eq!(e.code, "network");
        assert!(!e.message.contains("tunnel"), "{}", e.message);
        assert!(e.details.as_deref().unwrap_or_default().contains("tunnel error"));
    }
}
