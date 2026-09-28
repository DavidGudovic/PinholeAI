//! Everything Pinhole writes to disk lives here (SPEC §3). Nothing in this crate
//! may ever accept or store prompt / negative-prompt text — the only user text
//! stored is a Style the user explicitly saves (`Data/styles/`).
//!
//! OWNER: store agent.

pub mod datadir;
pub mod installed;
pub mod keychain;
pub mod presets;
pub mod settings;
pub mod styles;

pub use datadir::DataDir;
pub use installed::{InstalledFile, InstalledIndex};
pub use settings::Settings;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("disk error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid file {path}: {msg}")]
    Parse { path: String, msg: String },
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("keychain: {0}")]
    Keychain(String),
}

/// Write `bytes` to `path` atomically (temp file in the same dir + rename).
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), StoreError> {
    let _ = (path, bytes);
    todo!("store agent")
}

/// File-name-safe slug ("Film photo" → "film-photo"); never derived from prompts.
pub fn slugify(name: &str) -> String {
    let _ = name;
    todo!("store agent")
}
