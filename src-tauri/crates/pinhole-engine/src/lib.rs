//! Inference engines (SPEC §2): pinned `sd-server` / `llama-server` builds
//! (`config/engine.yaml`), download + SHA-256 verify + unpack into
//! `Data/engine/<sd|llama>/<version>/<backend>/`, process management (127.0.0.1 only,
//! random free port, stdout/stderr into an in-memory ring buffer — never to
//! disk), and typed clients for the native sd-server API
//! (`/sdcpp/v1/img_gen`, `/jobs/{id}`, `/jobs/{id}/cancel`, `/capabilities`,
//! `/upscale`) and llama-server's chat API for captioning.
//!
//! PRIVACY: nothing in this crate writes prompt text anywhere. Prompt-bearing
//! types ([`sdapi::ImgGenRequest`]) are `Serialize` only for the loopback HTTP
//! body, have a redacting `Debug`, and the engine output ring buffer
//! ([`logbuf::LogBuffer`]) redacts prompt text before storing a line.

pub mod detail;
pub mod extend;
pub mod failure;
pub mod image;
pub mod install;
pub mod llama;
pub mod logbuf;
pub mod orphans;
pub mod pins;
pub mod png;
pub mod process;
pub mod provenance;
pub mod sdapi;
#[cfg(feature = "test-util")]
pub mod testutil;
pub mod watermark;
pub mod words;

pub use failure::{classify, failed_stage, memory_failure, Failure, Stage};
pub use install::{EngineKind, InstalledEngine};
pub use logbuf::{LogBuffer, ProgressKind, StepProgress};
pub use pins::{EngineConfig, EnginePin};
pub use process::EngineProcess;
pub use sdapi::{ImgGenRequest, SdClient};

/// Errors from this crate. Messages are technical; `pinhole-core` maps them to
/// plain-language `CoreError`s. Never contains prompt text.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("engine config: {0}")]
    Config(String),
    #[error("no engine build for {os}/{backend}")]
    NoBuild { os: String, backend: String },
    #[error("unsafe path in archive: {0}")]
    UnsafeArchive(String),
    #[error("archive: {0}")]
    Archive(String),
    #[error("`{0}` not found in the unpacked engine")]
    BinaryMissing(String),
    #[error("SHA-256 mismatch for {file}")]
    HashMismatch {
        file: String,
        expected: String,
        actual: String,
    },
    /// Release builds: an archive in engine.yaml has no pinned SHA-256 (`TODO`).
    #[error("no pinned SHA-256 for {0}")]
    Unpinned(String),
    #[error("download: {0}")]
    Download(String),
    #[error("disk error: {0}")]
    Io(#[from] std::io::Error),
}
