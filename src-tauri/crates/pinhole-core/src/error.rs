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
    /// `engine_missing`, `engine_failed`, `cancelled`, `unauthorized`,
    /// `hash_mismatch`, `invalid`, `network`, `io`, `internal`.
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

impl From<pinhole_store::StoreError> for CoreError {
    fn from(e: pinhole_store::StoreError) -> Self {
        Self::new(e.code(), e.user_message())
    }
}

impl From<pinhole_registry::RegistryError> for CoreError {
    fn from(e: pinhole_registry::RegistryError) -> Self {
        Self::new("invalid", format!("The model registry could not be loaded: {e}"))
    }
}

impl From<pinhole_net::NetError> for CoreError {
    fn from(e: pinhole_net::NetError) -> Self {
        use pinhole_net::NetError as N;
        match e {
            N::Offline => Self::new("offline", "Offline mode is on. Turn it off in Settings to browse or download."),
            N::Unauthorized(_) => Self::new("unauthorized", "CivitAI needs an API key for this download. Add one in Settings."),
            other => Self::new("network", format!("Network problem — check your connection and try again. ({other})")),
        }
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        Self::new("io", format!("Disk error: {e}"))
    }
}
