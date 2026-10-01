//! Resumable, SHA-256-verified downloads (CLAUDE.md "Security rules for downloads"):
//! free-disk check first, write `<dest>.part`, resume with `Range`, verify hash,
//! then atomic rename. A [`DownloadManager`] queues groups of files (a model
//! plus its missing components) and broadcasts progress.
//!
//! Nothing here logs. Spec headers (API keys) are never serialized or put in errors.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{FutureExt, StreamExt};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::{broadcast, watch};
use tokio_util::sync::CancellationToken;

use crate::{with_headers, HttpClient, NetError};

/// Free space kept on top of what a download needs.
pub const DISK_MARGIN_BYTES: u64 = 256 * 1024 * 1024;
/// Minimum interval between two progress callbacks (~5/s).
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);
/// Attempts per file for transient network errors (resuming each time).
const MAX_ATTEMPTS: u32 = 4;
/// Finished groups listed by [`DownloadManager::status`].
const STATUS_FINISHED: usize = 20;
/// Finished groups kept for [`DownloadManager::wait`].
const KEEP_FINISHED: usize = 100;
const MB: u64 = 1024 * 1024;
/// Cap for a download whose exact size is unknown (no `size_bytes`, no
/// `Content-Length`): nothing Pinhole fetches is anywhere near this.
pub const MAX_UNKNOWN_SIZE_BYTES: u64 = 64 * 1024 * 1024 * 1024;
/// Free space is re-checked every this many bytes when the size is unknown.
const SPACE_RECHECK_BYTES: u64 = 256 * MB;

/// Extra check run on a finished, hash-verified file (e.g. "parses as a
/// safetensors/GGUF model"). `Err` carries a plain-language message; the file
/// is then deleted and the download fails with it.
pub type ContentCheck = Arc<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;

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
    /// More bytes than the expected size allows (see [`size_limit`]).
    #[error("the download was larger than expected")]
    TooLarge,
    /// The spec's [`ContentCheck`] refused the file (message is plain language).
    #[error("{0}")]
    Rejected(String),
}

impl DownloadError {
    /// Stable machine code, aligned with `CoreError` codes.
    pub fn code(&self) -> &'static str {
        match self {
            DownloadError::Net(NetError::Offline) => "offline",
            DownloadError::Net(NetError::Unauthorized(_)) => "unauthorized",
            DownloadError::Net(_) => "network",
            DownloadError::DiskSpace { .. } => "disk_space",
            DownloadError::HashMismatch { .. } => "hash_mismatch",
            DownloadError::Cancelled => "cancelled",
            DownloadError::Io(_) => "io",
            DownloadError::TooLarge => "too_large",
            DownloadError::Rejected(_) => "invalid",
        }
    }

    /// Plain-language message that says what to do next. `url` (the spec URL)
    /// only picks the wording for 401/403; it is never included.
    pub fn user_message(&self, url: &str) -> String {
        match self {
            DownloadError::Net(e) => net_message(e, url),
            DownloadError::DiskSpace { need_mb, free_mb } => {
                format!("Not enough disk space — free up {} and try again.", human_mb(need_mb.saturating_sub(*free_mb)))
            }
            DownloadError::HashMismatch { .. } => "The download was corrupted (checksum mismatch). Try again.".into(),
            DownloadError::Cancelled => "Cancelled".into(),
            DownloadError::Io(e) => format!("Could not write the download to disk ({e}). Check the Data folder and try again."),
            DownloadError::TooLarge => {
                "The download was larger than expected, so Pinhole stopped it. Try again later or pick another file.".into()
            }
            DownloadError::Rejected(msg) => msg.clone(),
        }
    }
}

fn human_mb(mb: u64) -> String {
    if mb >= 1024 {
        format!("{} GB", mb.div_ceil(1024))
    } else {
        format!("{} MB", mb.max(1))
    }
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

fn net_message(e: &NetError, url: &str) -> String {
    use crate::allow::host_matches;
    match e {
        NetError::Offline => "Offline mode is on. Turn it off in Settings to download.".into(),
        NetError::Unauthorized(code) => {
            let host = host_of(url);
            if host_matches(&host, "civitai.com") {
                "CivitAI needs an API key for this download. Add one in Settings, then try again."
                    .into()
            } else if host_matches(&host, "huggingface.co") || host_matches(&host, "hf.co") {
                "Hugging Face refused this download (the model may need a login or license approval). Pick another model."
                    .into()
            } else {
                format!("The server refused this download (HTTP {code}). Try again later.")
            }
        }
        NetError::Status(404) | NetError::Status(410) => {
            "This file is no longer available on the server. Pick another model or version.".into()
        }
        NetError::Status(429) => {
            "The server is busy (too many requests). Wait a minute and try again.".into()
        }
        NetError::Status(code) if *code >= 500 => {
            format!("The server had a problem (HTTP {code}). Try again later.")
        }
        NetError::Status(code) => format!("The download failed (HTTP {code}). Try again later."),
        NetError::Timeout => {
            "The download timed out — check your internet connection and try again.".into()
        }
        NetError::HostNotAllowed(_) => {
            "The download was redirected to a server Pinhole doesn't trust, so it was stopped."
                .into()
        }
        NetError::BadUrl(_) => "The download link is invalid.".into(),
        NetError::TooLarge => "The server sent more data than expected.".into(),
        NetError::Transport(_) | NetError::Decode(_) => {
            "Network problem — check your internet connection and try again.".into()
        }
    }
}

/// One file to fetch. `headers` may contain an API key: never log them
/// (`Debug` prints header names only; serialization skips them).
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct DownloadSpec {
    pub url: String,
    pub dest: PathBuf,
    /// Lowercase hex. `None` (or registry `TODO`) → hash is computed and returned
    /// but not enforced; callers decide whether that is acceptable.
    pub sha256: Option<String>,
    /// EXACT size in bytes, or `None`. It decides resume / "already downloaded"
    /// and bounds the download, so never put a rounded value here (use
    /// `approx_size_bytes`).
    pub size_bytes: Option<u64>,
    /// Friendly label for the UI ("Z-Image Turbo", "VAE").
    pub label: String,
    #[serde(default, skip_serializing)]
    pub headers: Vec<(String, String)>,
    /// Rounded size (registry `size_mb`, CivitAI `sizeKB`): only for the UI
    /// total and the up-front free-space check.
    #[serde(default)]
    pub approx_size_bytes: Option<u64>,
    /// Run on the finished file before the download counts as done.
    #[serde(skip)]
    pub content_check: Option<ContentCheck>,
}

impl std::fmt::Debug for DownloadSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let header_names: Vec<&str> = self.headers.iter().map(|(k, _)| k.as_str()).collect();
        f.debug_struct("DownloadSpec")
            .field("url", &self.url)
            .field("dest", &self.dest)
            .field("sha256", &self.sha256)
            .field("size_bytes", &self.size_bytes)
            .field("approx_size_bytes", &self.approx_size_bytes)
            .field("label", &self.label)
            .field("headers", &header_names)
            .field("content_check", &self.content_check.is_some())
            .finish()
    }
}

/// Most bytes a download may deliver: the exact size + 1 % + 1 MiB, or
/// [`MAX_UNKNOWN_SIZE_BYTES`] when the exact size isn't known.
pub fn size_limit(size_bytes: Option<u64>) -> u64 {
    match size_bytes {
        Some(s) => s.saturating_add(s / 100).saturating_add(MB),
        None => MAX_UNKNOWN_SIZE_BYTES,
    }
}

impl DownloadSpec {
    /// Best size estimate for totals and disk checks (exact, else approximate).
    pub fn size_hint(&self) -> Option<u64> {
        self.size_bytes.or(self.approx_size_bytes)
    }

    /// Expected hash when it must be enforced (not `None`, empty or `TODO`).
    pub fn expected_sha256(&self) -> Option<String> {
        let h = self.sha256.as_deref()?.trim();
        if h.is_empty() || h.eq_ignore_ascii_case("todo") {
            None
        } else {
            Some(h.to_ascii_lowercase())
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadedFile {
    pub path: PathBuf,
    pub sha256: String,
    pub size_bytes: u64,
}

/// `<dest>.part`
pub fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Phase reported by [`download_file_with_phase`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Hashing bytes already on disk (resumed `.part`, existing file).
    Verifying,
    Downloading,
}

/// Download one file. `progress(downloaded, total)` is called ~5×/s.
pub async fn download_file(
    client: &HttpClient,
    spec: &DownloadSpec,
    cancel: &CancellationToken,
    progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
) -> Result<DownloadedFile, DownloadError> {
    download_file_with_phase(client, spec, cancel, progress, &|_| {}).await
}

/// [`download_file`] plus a phase callback (for "Verifying…" in the UI).
/// A spec's [`ContentCheck`] runs last; a refused file is deleted.
pub async fn download_file_with_phase(
    client: &HttpClient,
    spec: &DownloadSpec,
    cancel: &CancellationToken,
    progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    phase: &(dyn Fn(Phase) + Send + Sync),
) -> Result<DownloadedFile, DownloadError> {
    let file = fetch_verified(client, spec, cancel, progress, phase).await?;
    if let Some(check) = spec.content_check.clone() {
        phase(Phase::Verifying);
        let path = file.path.clone();
        let verdict = tokio::task::spawn_blocking(move || check(&path))
            .await
            .map_err(|e| DownloadError::Io(std::io::Error::other(e.to_string())))?;
        if let Err(msg) = verdict {
            remove_quietly(&file.path).await;
            return Err(DownloadError::Rejected(msg));
        }
    }
    Ok(file)
}

async fn fetch_verified(
    client: &HttpClient,
    spec: &DownloadSpec,
    cancel: &CancellationToken,
    progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    phase: &(dyn Fn(Phase) + Send + Sync),
) -> Result<DownloadedFile, DownloadError> {
    // Offline / allow-list first: nothing touches disk or network otherwise.
    client.check_url(&spec.url)?;
    if cancel.is_cancelled() {
        return Err(DownloadError::Cancelled);
    }
    let dir = parent_dir(&spec.dest);
    tokio::fs::create_dir_all(&dir).await?;
    let expected = spec.expected_sha256();

    // Already there and verified (e.g. a retried install)? Don't download again.
    if let Some(exp) = &expected {
        if let Ok(meta) = tokio::fs::metadata(&spec.dest).await {
            if meta.is_file() && spec.size_bytes.is_none_or(|s| s == meta.len()) {
                phase(Phase::Verifying);
                let (hasher, len) = hash_prefix(spec.dest.clone(), None, cancel.clone()).await?;
                let actual = hex::encode(hasher.finalize());
                if &actual == exp {
                    progress(len, Some(len));
                    return Ok(DownloadedFile {
                        path: spec.dest.clone(),
                        sha256: actual,
                        size_bytes: len,
                    });
                }
            }
        }
    }

    let part = part_path(&spec.dest);
    // Hash state covering the first N bytes of `.part`, carried across retries.
    let mut carried: Option<(Sha256, u64)> = None;
    // `.part` holds bytes from before this call (e.g. a leftover of an older
    // upload of the file). Cleared once an attempt starts the file over.
    let mut leftover = tokio::fs::metadata(&part)
        .await
        .is_ok_and(|m| m.is_file() && m.len() > 0);
    let mut attempt = 0;
    let mut restarted_after_mismatch = false;
    loop {
        attempt += 1;
        let mut started_over = false;
        let r = try_once(
            client,
            spec,
            &dir,
            &part,
            expected.as_deref(),
            cancel,
            progress,
            phase,
            &mut carried,
            &mut started_over,
        )
        .await;
        leftover &= !started_over;
        match r {
            // The user cancelled while this attempt was failing (e.g. still connecting):
            // report the cancel, not the transport error it raced with.
            Err(_) if cancel.is_cancelled() => return Err(DownloadError::Cancelled),
            // The file was finished from bytes that were on disk before this
            // call: download it again from byte 0, once, with a fresh retry
            // budget, before calling it corrupt.
            Err(DownloadError::HashMismatch { .. }) if leftover && !restarted_after_mismatch => {
                restarted_after_mismatch = true;
                leftover = false;
                carried = None;
                attempt = 0;
                remove_quietly(&part).await;
            }
            Err(e) if attempt < MAX_ATTEMPTS && is_retryable(&e) && !cancel.is_cancelled() => {
                let wait = Duration::from_secs(1 << (attempt - 1));
                tokio::select! {
                    _ = cancel.cancelled() => return Err(DownloadError::Cancelled),
                    _ = tokio::time::sleep(wait) => {}
                }
            }
            other => return other,
        }
    }
}

fn is_retryable(e: &DownloadError) -> bool {
    matches!(
        e,
        DownloadError::Net(NetError::Transport(_))
            | DownloadError::Net(NetError::Timeout)
            | DownloadError::Net(NetError::Status(408 | 500..=599))
    )
}

fn parent_dir(dest: &Path) -> PathBuf {
    match dest.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Parse `Content-Range: bytes START-END/TOTAL` (TOTAL may be `*`).
fn content_range(resp: &reqwest::Response) -> Option<(Option<u64>, Option<u64>)> {
    let v = resp
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    let rest = v.trim().strip_prefix("bytes")?.trim();
    let (range, total) = rest.split_once('/')?;
    let total = total.trim().parse::<u64>().ok();
    let start = if range.trim() == "*" {
        None
    } else {
        range.split('-').next()?.trim().parse::<u64>().ok()
    };
    Some((start, total))
}

#[allow(clippy::too_many_arguments)]
async fn try_once(
    client: &HttpClient,
    spec: &DownloadSpec,
    dir: &Path,
    part: &Path,
    expected: Option<&str>,
    cancel: &CancellationToken,
    progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    phase: &(dyn Fn(Phase) + Send + Sync),
    carried: &mut Option<(Sha256, u64)>,
    // Set when this attempt drops what `.part` held and starts from byte 0.
    started_over: &mut bool,
) -> Result<DownloadedFile, DownloadError> {
    let mut offset = match tokio::fs::metadata(part).await {
        Ok(m) if m.is_file() => m.len(),
        _ => 0,
    };
    if spec.size_bytes.is_some_and(|s| offset > s) {
        // Longer than the file can be: stale or foreign. Start over.
        remove_quietly(part).await;
        *started_over = true;
        offset = 0;
    }
    if let Some(size) = spec.size_hint() {
        check_free_space_async(dir, size.saturating_sub(offset)).await?;
    }

    // Seed the hash with the bytes already on disk.
    let mut hasher = Sha256::new();
    if offset > 0 {
        match carried.take() {
            Some((h, n)) if n == offset => hasher = h,
            _ => {
                phase(Phase::Verifying);
                let (h, n) = hash_prefix(part.to_path_buf(), Some(offset), cancel.clone()).await?;
                hasher = h;
                offset = n;
            }
        }
        if spec.size_bytes == Some(offset) {
            // Everything is already here (e.g. cancelled while finishing).
            progress(offset, Some(offset));
            return finalize(spec, part, hasher, offset, expected, phase).await;
        }
    }

    let mut restarted = false;
    let resp = loop {
        let mut rb = with_headers(client.get(&spec.url)?, &spec.headers)?;
        if offset > 0 {
            rb = rb.header(reqwest::header::RANGE, format!("bytes={offset}-"));
        }
        // Cancel must not wait for a slow connect / an unresponsive server.
        let resp = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(DownloadError::Cancelled),
            r = client.send_unchecked_status(rb) => r?,
        };
        let status = resp.status().as_u16();
        if offset > 0 && status == 416 {
            // Our `.part` is at (or past) the end according to the server.
            if let Some((None, Some(total))) = content_range(&resp) {
                if total == offset {
                    drop(resp);
                    progress(offset, Some(offset));
                    return finalize(spec, part, hasher, offset, expected, phase).await;
                }
            }
        }
        if offset > 0
            && (status == 416
                || (status == 206 && content_range(&resp).and_then(|c| c.0) != Some(offset)))
        {
            // Can't resume from here: start from zero, once.
            drop(resp);
            if restarted {
                return Err(NetError::Status(status).into());
            }
            restarted = true;
            remove_quietly(part).await;
            *started_over = true;
            offset = 0;
            hasher = Sha256::new();
            continue;
        }
        break crate::check_status(resp)?;
    };

    let status = resp.status().as_u16();
    let append = offset > 0 && status == 206;
    let mut downloaded = if append { offset } else { 0 };
    *started_over |= !append;
    if !append {
        // Fresh download, or the server ignored `Range` (200): restart from byte 0.
        hasher = Sha256::new();
    }
    let total = match content_range(&resp) {
        Some((_, Some(t))) if status == 206 => Some(t),
        _ => resp.content_length().map(|l| l + downloaded),
    };
    let limit = size_limit(spec.size_bytes);
    if total.is_some_and(|t| t > limit) {
        drop(resp);
        remove_quietly(part).await;
        return Err(DownloadError::TooLarge);
    }
    if spec.size_bytes.is_none() {
        if let Some(t) = total {
            check_free_space_async(dir, t.saturating_sub(downloaded)).await?;
        }
    }

    let file = if append {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(part)
            .await?
    } else {
        tokio::fs::File::create(part).await?
    };
    let mut out = tokio::io::BufWriter::with_capacity(1024 * 1024, file);
    phase(Phase::Downloading);
    progress(downloaded, total);
    let mut last = Instant::now();
    let mut next_space_check = downloaded.saturating_add(SPACE_RECHECK_BYTES);
    let mut stream = resp.bytes_stream();
    loop {
        let next = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                out.flush().await?;
                *carried = Some((hasher, downloaded));
                return Err(DownloadError::Cancelled);
            }
            n = stream.next() => n,
        };
        match next {
            None => break,
            Some(Err(e)) => {
                out.flush().await?;
                *carried = Some((hasher, downloaded));
                // A body that stops early ("end of file before message length
                // reached") is a dropped connection: retry and resume it.
                return Err(match NetError::from(e) {
                    NetError::Decode(msg) => NetError::Transport(msg),
                    other => other,
                }
                .into());
            }
            Some(Ok(chunk)) => {
                if client.offline_flag().get() {
                    out.flush().await?;
                    *carried = Some((hasher, downloaded));
                    return Err(NetError::Offline.into());
                }
                if downloaded.saturating_add(chunk.len() as u64) > limit {
                    drop(out);
                    remove_quietly(part).await;
                    return Err(DownloadError::TooLarge);
                }
                hasher.update(&chunk);
                out.write_all(&chunk).await?;
                downloaded += chunk.len() as u64;
                // Size unknown up front: make sure the disk doesn't fill up as bytes arrive.
                if total.is_none() && downloaded >= next_space_check {
                    next_space_check = downloaded.saturating_add(SPACE_RECHECK_BYTES);
                    if let Err(e) = check_free_space_async(dir, SPACE_RECHECK_BYTES).await {
                        out.flush().await?;
                        *carried = Some((hasher, downloaded));
                        return Err(e);
                    }
                }
                if last.elapsed() >= PROGRESS_INTERVAL {
                    last = Instant::now();
                    progress(downloaded, total);
                }
            }
        }
    }
    out.flush().await?;
    let file = out.into_inner();
    file.sync_all().await?;
    drop(file);
    if total.is_some_and(|t| downloaded < t) {
        *carried = Some((hasher, downloaded));
        return Err(NetError::Transport("the download ended early".into()).into());
    }
    progress(downloaded, total.or(Some(downloaded)));
    finalize(spec, part, hasher, downloaded, expected, phase).await
}

async fn finalize(
    spec: &DownloadSpec,
    part: &Path,
    hasher: Sha256,
    size: u64,
    expected: Option<&str>,
    phase: &(dyn Fn(Phase) + Send + Sync),
) -> Result<DownloadedFile, DownloadError> {
    phase(Phase::Verifying);
    let actual = hex::encode(hasher.finalize());
    if let Some(exp) = expected {
        if actual != exp {
            remove_quietly(part).await;
            return Err(DownloadError::HashMismatch {
                expected: exp.to_string(),
                actual,
            });
        }
    }
    // A different file already has this name (one the user put in the models folder):
    // it stays, and the download gets the next free name.
    let dest = free_name(&spec.dest);
    tokio::fs::rename(part, &dest).await?;
    Ok(DownloadedFile {
        path: dest,
        sha256: actual,
        size_bytes: size,
    })
}

/// `dest`, or `name-2.ext`, `name-3.ext`… when a file is already there.
fn free_name(dest: &Path) -> PathBuf {
    if !dest.exists() {
        return dest.to_path_buf();
    }
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name.as_str(), ""));
    (2..10_000)
        .map(|i| {
            dest.with_file_name(if ext.is_empty() {
                format!("{stem}-{i}")
            } else {
                format!("{stem}-{i}.{ext}")
            })
        })
        .find(|p| !p.exists() && !part_path(p).exists())
        .unwrap_or_else(|| dest.to_path_buf())
}

async fn remove_quietly(path: &Path) {
    let _ = tokio::fs::remove_file(path).await;
}

/// Hash the first `len` bytes of `path` (whole file when `None`) off the async
/// threads. Returns the hasher and the number of bytes hashed.
async fn hash_prefix(
    path: PathBuf,
    len: Option<u64>,
    cancel: CancellationToken,
) -> Result<(Sha256, u64), DownloadError> {
    tokio::task::spawn_blocking(move || -> Result<(Sha256, u64), DownloadError> {
        let file = std::fs::File::open(&path)?;
        let mut reader: Box<dyn Read> = match len {
            Some(n) => Box::new(file.take(n)),
            None => Box::new(file),
        };
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1024 * 1024];
        let mut total = 0u64;
        loop {
            if cancel.is_cancelled() {
                return Err(DownloadError::Cancelled);
            }
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            total += n as u64;
        }
        Ok((hasher, total))
    })
    .await
    .map_err(|e| DownloadError::Io(std::io::Error::other(e.to_string())))?
}

async fn check_free_space_async(dir: &Path, need_bytes: u64) -> Result<(), DownloadError> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || check_free_space(&dir, need_bytes))
        .await
        .map_err(|e| DownloadError::Io(std::io::Error::other(e.to_string())))?
}

/// Error if `dir`'s filesystem has less than `need_bytes` (+ [`DISK_MARGIN_BYTES`]) free.
/// `dir` may not exist yet: its nearest existing ancestor is checked.
pub fn check_free_space(dir: &Path, need_bytes: u64) -> Result<(), DownloadError> {
    if need_bytes == 0 {
        return Ok(());
    }
    let mut probe = dir;
    while !probe.exists() {
        match probe.parent() {
            Some(p) if !p.as_os_str().is_empty() => probe = p,
            _ => {
                probe = Path::new(".");
                break;
            }
        }
    }
    let free = fs2::available_space(probe)?;
    let need = need_bytes.saturating_add(DISK_MARGIN_BYTES);
    if free < need {
        return Err(DownloadError::DiskSpace {
            need_mb: need.div_ceil(MB),
            free_mb: free / MB,
        });
    }
    Ok(())
}

/// Streaming SHA-256 of a file (lowercase hex). Blocking: call from `spawn_blocking`.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

// ============================================================== manager

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

impl DownloadState {
    pub fn is_finished(self) -> bool {
        matches!(
            self,
            DownloadState::Done | DownloadState::Failed | DownloadState::Cancelled
        )
    }
}

/// What a group downloads, so the UI can find e.g. the engine group without
/// matching on its label. `DownloadKind` in `src/lib/types.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadKind {
    /// The image engine (sd-server).
    Engine,
    /// A model / LoRA plus the components it needs.
    Model,
    /// The Describe model (and the llama.cpp engine it runs on).
    Captioner,
    /// The Real-ESRGAN upscaler.
    Upscaler,
    /// A new version of Pinhole itself (Settings → Check for updates).
    AppUpdate,
    /// The image check's model files.
    SafetyCheck,
}

/// Progress of a group (what the UI shows as one row).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupStatus {
    pub group_id: String,
    pub label: String,
    /// `None` for groups queued with plain [`DownloadManager::enqueue`].
    #[serde(default)]
    pub kind: Option<DownloadKind>,
    pub state: DownloadState,
    /// Label of the file currently downloading.
    pub current_file: Option<String>,
    /// 0-based index of the current file (UI shows `fileIndex + 1` of `fileCount`).
    pub file_index: usize,
    pub file_count: usize,
    pub downloaded_bytes: u64,
    /// Sum of known sizes; grows when a file without a size reports Content-Length.
    pub total_bytes: u64,
    /// Plain-language error ("Not enough disk space — free up 12 GB and try again.").
    /// `None` unless `state == Failed`.
    pub error: Option<String>,
}

/// Why a group did not finish. `code` matches `CoreError` codes
/// (`cancelled`, `offline`, `unauthorized`, `disk_space`, `hash_mismatch`,
/// `network`, `io`, `not_found`, `internal`); `message` is plain language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct GroupError {
    pub code: String,
    pub message: String,
}

impl GroupError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    fn cancelled() -> Self {
        Self::new("cancelled", "Cancelled")
    }
}

type Outcome = Result<Vec<DownloadedFile>, GroupError>;

struct Group {
    status: GroupStatus,
    files: Vec<DownloadSpec>,
    cancel: CancellationToken,
    done: watch::Sender<Option<Outcome>>,
}

#[derive(Default)]
struct State {
    groups: HashMap<String, Group>,
    /// Every known group id in enqueue order.
    order: Vec<String>,
    queue: VecDeque<String>,
    finished: VecDeque<String>,
    worker_running: bool,
}

struct Shared {
    state: Mutex<State>,
    events: broadcast::Sender<GroupStatus>,
}

/// Queue of download groups; groups run one at a time, files sequentially.
///
/// `new` may be called outside a Tokio runtime; the worker is spawned lazily on
/// `enqueue` (on the current runtime, or on a private thread if there is none)
/// and exits when the queue is empty.
#[derive(Clone)]
pub struct DownloadManager {
    pub(crate) client: HttpClient,
    shared: Arc<Shared>,
}

impl DownloadManager {
    pub fn new(client: HttpClient) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            client,
            shared: Arc::new(Shared {
                state: Mutex::new(State::default()),
                events,
            }),
        }
    }

    /// Queue a group. Returns its id immediately; completion is observable via
    /// [`DownloadManager::subscribe`] or [`DownloadManager::wait`].
    pub fn enqueue(&self, label: String, files: Vec<DownloadSpec>) -> String {
        self.enqueue_group(label, None, files)
    }

    /// [`DownloadManager::enqueue`] with a [`DownloadKind`] the UI can match on.
    pub fn enqueue_kind(
        &self,
        label: String,
        kind: DownloadKind,
        files: Vec<DownloadSpec>,
    ) -> String {
        self.enqueue_group(label, Some(kind), files)
    }

    fn enqueue_group(
        &self,
        label: String,
        kind: Option<DownloadKind>,
        files: Vec<DownloadSpec>,
    ) -> String {
        let group_id = uuid::Uuid::new_v4().to_string();
        let status = GroupStatus {
            group_id: group_id.clone(),
            label,
            kind,
            state: DownloadState::Queued,
            current_file: None,
            file_index: 0,
            file_count: files.len(),
            downloaded_bytes: 0,
            total_bytes: files.iter().filter_map(|f| f.size_hint()).sum(),
            error: None,
        };
        let spawn = {
            let mut st = self.shared.state.lock();
            let (done, _) = watch::channel(None);
            st.groups.insert(
                group_id.clone(),
                Group {
                    status: status.clone(),
                    files,
                    cancel: CancellationToken::new(),
                    done,
                },
            );
            st.order.push(group_id.clone());
            st.queue.push_back(group_id.clone());
            let spawn = !st.worker_running;
            st.worker_running = true;
            spawn
        };
        let _ = self.shared.events.send(status);
        if spawn {
            self.spawn_worker();
        }
        group_id
    }

    /// Wait for a group to finish. Ok = every file downloaded + verified.
    /// Err is a plain-language message. Works for groups that already finished.
    pub async fn wait(&self, group_id: &str) -> Result<Vec<DownloadedFile>, String> {
        self.wait_detailed(group_id).await.map_err(|e| e.message)
    }

    /// Like [`DownloadManager::wait`], with a machine-readable error code.
    pub async fn wait_detailed(&self, group_id: &str) -> Result<Vec<DownloadedFile>, GroupError> {
        let mut rx = {
            let st = self.shared.state.lock();
            match st.groups.get(group_id) {
                Some(g) => g.done.subscribe(),
                None => {
                    return Err(GroupError::new(
                        "not_found",
                        "This download is no longer listed.",
                    ))
                }
            }
        };
        let outcome = match rx.wait_for(|v| v.is_some()).await {
            Ok(v) => v.clone(),
            Err(_) => None,
        };
        outcome.unwrap_or_else(|| {
            Err(GroupError::new(
                "internal",
                "The download stopped unexpectedly. Try again.",
            ))
        })
    }

    /// Cancel a queued or running group. Running files keep their `.part` for a
    /// later resume. No-op for finished or unknown groups.
    pub fn cancel(&self, group_id: &str) {
        let emitted = {
            let mut st = self.shared.state.lock();
            let state = match st.groups.get(group_id) {
                Some(g) => g.status.state,
                None => return,
            };
            match state {
                DownloadState::Queued => {
                    st.queue.retain(|id| id != group_id);
                    finish_locked(
                        &mut st,
                        group_id,
                        DownloadState::Cancelled,
                        Err(GroupError::cancelled()),
                    )
                }
                DownloadState::Downloading | DownloadState::Verifying => {
                    if let Some(g) = st.groups.get(group_id) {
                        g.cancel.cancel();
                    }
                    None
                }
                _ => None,
            }
        };
        if let Some(s) = emitted {
            let _ = self.shared.events.send(s);
        }
    }

    /// Mark a group that finished downloading as failed after all, e.g. when
    /// its files couldn't be registered. `wait` then returns this error too.
    /// No-op unless the group is `Done`.
    pub fn fail_done(&self, group_id: &str, code: &str, message: &str) {
        let emitted = {
            let mut st = self.shared.state.lock();
            let Some(g) = st.groups.get_mut(group_id) else {
                return;
            };
            if g.status.state != DownloadState::Done {
                return;
            }
            g.status.state = DownloadState::Failed;
            g.status.error = Some(message.to_string());
            g.done
                .send_replace(Some(Err(GroupError::new(code, message))));
            g.status.clone()
        };
        let _ = self.shared.events.send(emitted);
    }

    /// Active groups (running + queued) and the last 20 finished, in enqueue order.
    pub fn status(&self) -> Vec<GroupStatus> {
        let st = self.shared.state.lock();
        let recent: HashSet<&String> = st.finished.iter().rev().take(STATUS_FINISHED).collect();
        st.order
            .iter()
            .filter_map(|id| st.groups.get(id))
            .filter(|g| !g.status.state.is_finished() || recent.contains(&g.status.group_id))
            .map(|g| g.status.clone())
            .collect()
    }

    /// Every state change and progress tick (~5/s while downloading).
    pub fn subscribe(&self) -> broadcast::Receiver<GroupStatus> {
        self.shared.events.subscribe()
    }

    // ------------------------------------------------------------ worker

    fn spawn_worker(&self) {
        let this = self.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move { this.run_worker().await });
            }
            Err(_) => {
                let spawned = std::thread::Builder::new()
                    .name("pinhole-downloads".into())
                    .spawn(move || {
                        match tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                        {
                            Ok(rt) => rt.block_on(this.run_worker()),
                            Err(e) => this.fail_all(&format!("Could not start downloads ({e}).")),
                        }
                    });
                if spawned.is_err() {
                    self.fail_all("Could not start downloads.");
                }
            }
        }
    }

    /// Fail every queued group (worker could not start).
    fn fail_all(&self, message: &str) {
        let emitted: Vec<GroupStatus> = {
            let mut st = self.shared.state.lock();
            st.worker_running = false;
            let ids: Vec<String> = st.queue.drain(..).collect();
            ids.iter()
                .filter_map(|id| {
                    finish_locked(
                        &mut st,
                        id,
                        DownloadState::Failed,
                        Err(GroupError::new("internal", message)),
                    )
                })
                .collect()
        };
        for s in emitted {
            let _ = self.shared.events.send(s);
        }
    }

    async fn run_worker(self) {
        let mut guard = WorkerGuard {
            shared: self.shared.clone(),
            current: None,
            armed: true,
        };
        loop {
            let next = {
                let mut st = self.shared.state.lock();
                loop {
                    let Some(id) = st.queue.pop_front() else {
                        st.worker_running = false;
                        guard.armed = false;
                        break None;
                    };
                    if let Some(g) = st.groups.get_mut(&id) {
                        if g.status.state == DownloadState::Queued {
                            g.status.state = DownloadState::Downloading;
                            break Some((id, g.files.clone(), g.cancel.clone(), g.status.clone()));
                        }
                    }
                }
            };
            let Some((id, files, cancel, status)) = next else {
                return;
            };
            guard.current = Some(id.clone());
            let _ = self.shared.events.send(status);
            let outcome = std::panic::AssertUnwindSafe(self.run_group(&id, &files, &cancel))
                .catch_unwind()
                .await;
            let (state, outcome) = match outcome {
                Ok(Ok(done)) => (DownloadState::Done, Ok(done)),
                Ok(Err(e)) if e.code == "cancelled" => (DownloadState::Cancelled, Err(e)),
                Ok(Err(e)) => (DownloadState::Failed, Err(e)),
                Err(_) => (
                    DownloadState::Failed,
                    Err(GroupError::new(
                        "internal",
                        "Something went wrong while downloading. Try again.",
                    )),
                ),
            };
            let emitted = finish_locked(&mut self.shared.state.lock(), &id, state, outcome);
            guard.current = None;
            if let Some(s) = emitted {
                let _ = self.shared.events.send(s);
            }
        }
    }

    async fn run_group(
        &self,
        id: &str,
        files: &[DownloadSpec],
        cancel: &CancellationToken,
    ) -> Outcome {
        // Whole-group disk check first, so the message covers everything missing.
        let need = remaining_bytes(files).await;
        let mut dirs: Vec<PathBuf> = files.iter().map(|f| parent_dir(&f.dest)).collect();
        dirs.sort();
        dirs.dedup();
        for dir in &dirs {
            if let Err(e) = check_free_space_async(dir, need).await {
                return Err(GroupError::new(e.code(), e.user_message("")));
            }
        }

        let mut done = Vec::with_capacity(files.len());
        let mut completed: u64 = 0;
        // Sum of sizes of all files: known sizes, corrected as real sizes arrive.
        let mut base_total: u64 = files.iter().filter_map(|f| f.size_hint()).sum();
        for (i, spec) in files.iter().enumerate() {
            if cancel.is_cancelled() {
                return Err(GroupError::cancelled());
            }
            self.update(id, |s| {
                s.state = DownloadState::Downloading;
                s.file_index = i;
                s.current_file = Some(spec.label.clone());
                s.downloaded_bytes = completed;
                s.total_bytes = base_total.max(completed);
            });
            let known = spec.size_hint();
            let base = base_total;
            let before = completed;
            let progress = |d: u64, t: Option<u64>| {
                self.update(id, |s| {
                    s.downloaded_bytes = before + d;
                    if let (None, Some(t)) = (known, t) {
                        s.total_bytes = base + t;
                    }
                    s.total_bytes = s.total_bytes.max(s.downloaded_bytes);
                });
            };
            let phase = |p: Phase| {
                self.update(id, |s| {
                    s.state = match p {
                        Phase::Verifying => DownloadState::Verifying,
                        Phase::Downloading => DownloadState::Downloading,
                    };
                });
            };
            match download_file_with_phase(&self.client, spec, cancel, &progress, &phase).await {
                Ok(f) => {
                    completed += f.size_bytes;
                    base_total = base_total - known.unwrap_or(0) + f.size_bytes;
                    done.push(f);
                }
                Err(DownloadError::Cancelled) => return Err(GroupError::cancelled()),
                Err(e) => return Err(GroupError::new(e.code(), e.user_message(&spec.url))),
            }
        }
        self.update(id, |s| {
            s.downloaded_bytes = completed;
            s.total_bytes = completed;
        });
        Ok(done)
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut GroupStatus)) {
        let status = {
            let mut st = self.shared.state.lock();
            let Some(g) = st.groups.get_mut(id) else {
                return;
            };
            if g.status.state.is_finished() {
                return;
            }
            f(&mut g.status);
            g.status.clone()
        };
        let _ = self.shared.events.send(status);
    }
}

/// If the worker task is dropped mid-way (its runtime shut down), fail the
/// current group and let the next `enqueue` start a fresh worker, so waiters
/// never hang and the queue never wedges.
struct WorkerGuard {
    shared: Arc<Shared>,
    current: Option<String>,
    armed: bool,
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let emitted = {
            let mut st = self.shared.state.lock();
            st.worker_running = false;
            self.current.take().and_then(|id| {
                let err = GroupError::new("internal", "The download was interrupted. Try again.");
                finish_locked(&mut st, &id, DownloadState::Failed, Err(err))
            })
        };
        if let Some(s) = emitted {
            let _ = self.shared.events.send(s);
        }
    }
}

/// Bytes still to download for a group (known sizes minus `.part` bytes;
/// files already at `dest` count as zero).
async fn remaining_bytes(files: &[DownloadSpec]) -> u64 {
    let mut need = 0u64;
    for f in files {
        let Some(size) = f.size_hint() else { continue };
        // A file already there counts as done unless its size shows it's a different file
        // (that one is kept and the download gets a new name).
        if tokio::fs::metadata(&f.dest)
            .await
            .is_ok_and(|m| m.is_file() && f.size_bytes.is_none_or(|s| s == m.len()))
        {
            continue;
        }
        let have = tokio::fs::metadata(part_path(&f.dest))
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        need = need.saturating_add(size.saturating_sub(have));
    }
    need
}

/// Mark a group finished, publish its outcome, prune old history. Returns the
/// status to broadcast (caller sends it after releasing the lock).
fn finish_locked(
    st: &mut State,
    id: &str,
    state: DownloadState,
    outcome: Outcome,
) -> Option<GroupStatus> {
    let g = st.groups.get_mut(id)?;
    if g.status.state.is_finished() {
        return None;
    }
    g.status.state = state;
    g.status.error = match (&outcome, state) {
        (Err(e), DownloadState::Failed) => Some(e.message.clone()),
        _ => None,
    };
    if state == DownloadState::Done {
        g.status.current_file = None;
    }
    g.files.clear();
    g.done.send_replace(Some(outcome));
    let status = g.status.clone();
    st.finished.push_back(id.to_string());
    while st.finished.len() > KEEP_FINISHED {
        if let Some(old) = st.finished.pop_front() {
            st.groups.remove(&old);
            st.order.retain(|x| *x != old);
        }
    }
    Some(status)
}
