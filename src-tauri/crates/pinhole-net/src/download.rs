//! Resumable, SHA-256-verified downloads (CLAUDE.md "Security rules for downloads"):
//! free-disk check first, write `<dest>.part`, resume with `Range`, verify hash,
//! then atomic rename. A [`DownloadManager`] queues groups of files (a model
//! plus its missing components) and broadcasts progress.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::{HttpClient, NetError};

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error(transparent)]
    Net(#[from] NetError),
    #[error("not enough free disk space: need {need_mb} MB, have {free_mb} MB")]
    DiskSpace { need_mb: u64, free_mb: u64 },
    #[error("file is corrupt (SHA-256 mismatch)")]
    HashMismatch { expected: String, actual: String },
    #[error("cancelled")]
    Cancelled,
    #[error("disk error: {0}")]
    Io(#[from] std::io::Error),
}

/// One file to fetch. `headers` may contain an API key: never log them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadSpec {
    pub url: String,
    pub dest: PathBuf,
    /// Lowercase hex. `None` (or registry `TODO`) → hash is computed and returned
    /// but not enforced; callers decide whether that is acceptable.
    pub sha256: Option<String>,
    pub size_bytes: Option<u64>,
    /// Friendly label for the UI ("Z-Image Turbo", "VAE").
    pub label: String,
    #[serde(skip_serializing)]
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadedFile {
    pub path: PathBuf,
    pub sha256: String,
    pub size_bytes: u64,
}

/// Download one file. `progress(downloaded, total)` is called ~5×/s.
pub async fn download_file(
    client: &HttpClient,
    spec: &DownloadSpec,
    cancel: &CancellationToken,
    progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
) -> Result<DownloadedFile, DownloadError> {
    let _ = (client, spec, cancel, progress);
    todo!("net agent")
}

/// Error if `dir`'s filesystem has less than `need_bytes` (+ small margin) free.
pub fn check_free_space(dir: &Path, need_bytes: u64) -> Result<(), DownloadError> {
    let _ = (dir, need_bytes);
    todo!("net agent")
}

/// Streaming SHA-256 of a file (lowercase hex). Blocking: call from `spawn_blocking`.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let _ = path;
    todo!("net agent")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadState {
    Queued,
    Downloading,
    Verifying,
    Done,
    Failed,
    Cancelled,
}

/// Progress of a group (what the UI shows as one row).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupStatus {
    pub group_id: String,
    pub label: String,
    pub state: DownloadState,
    /// Label of the file currently downloading.
    pub current_file: Option<String>,
    pub file_index: usize,
    pub file_count: usize,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    /// Plain-language error ("Not enough disk space — free up 12 GB and retry").
    pub error: Option<String>,
}

/// Queue of download groups; groups run one at a time, files sequentially.
#[derive(Clone)]
pub struct DownloadManager {
    #[allow(dead_code)]
    pub(crate) client: HttpClient,
}

impl DownloadManager {
    pub fn new(client: HttpClient) -> Self {
        let _ = client;
        todo!("net agent")
    }

    /// Queue a group. Returns its id immediately; completion is observable via
    /// [`DownloadManager::subscribe`] or [`DownloadManager::wait`].
    pub fn enqueue(&self, label: String, files: Vec<DownloadSpec>) -> String {
        let _ = (label, files);
        todo!("net agent")
    }

    /// Wait for a group to finish. Ok = every file downloaded + verified.
    pub async fn wait(&self, group_id: &str) -> Result<Vec<DownloadedFile>, String> {
        let _ = group_id;
        todo!("net agent")
    }

    pub fn cancel(&self, group_id: &str) {
        let _ = group_id;
        todo!("net agent")
    }

    pub fn status(&self) -> Vec<GroupStatus> {
        todo!("net agent")
    }

    pub fn subscribe(&self) -> broadcast::Receiver<GroupStatus> {
        todo!("net agent")
    }
}
