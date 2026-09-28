//! The ONE HTTP client wrapper (CLAUDE.md privacy rule 5). Every outbound
//! request in Pinhole goes through [`HttpClient`]. It enforces:
//! * Offline mode — every call fails with [`NetError::Offline`] before a socket opens;
//! * an allow-list of hosts (civitai.com, huggingface.co, github.com and their
//!   download CDNs, CDNs only reachable via redirect from an allowed origin);
//! * HTTPS only (except [`LocalClient`], which only talks to 127.0.0.1).
//! It never logs URLs with query strings, headers or bodies.
//!
//! OWNER: net agent.

pub mod allow;
pub mod download;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("Offline mode is on")]
    Offline,
    #[error("host not allowed: {0}")]
    HostNotAllowed(String),
    #[error("invalid URL: {0}")]
    BadUrl(String),
    #[error("unauthorized (HTTP {0})")]
    Unauthorized(u16),
    #[error("HTTP {0}")]
    Status(u16),
    #[error("timed out")]
    Timeout,
    #[error("response too large")]
    TooLarge,
    #[error("network error: {0}")]
    Transport(String),
    #[error("could not decode response: {0}")]
    Decode(String),
}

/// Shared offline switch. Cloned into every client.
#[derive(Debug, Clone, Default)]
pub struct OfflineFlag(Arc<AtomicBool>);

impl OfflineFlag {
    pub fn new(offline: bool) -> Self {
        Self(Arc::new(AtomicBool::new(offline)))
    }
    pub fn set(&self, offline: bool) {
        self.0.store(offline, Ordering::SeqCst)
    }
    pub fn get(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Internet client (allow-listed, offline-aware).
#[derive(Clone)]
pub struct HttpClient {
    #[allow(dead_code)]
    pub(crate) offline: OfflineFlag,
}

impl HttpClient {
    pub fn new(offline: OfflineFlag) -> Result<Self, NetError> {
        let _ = offline;
        todo!("net agent")
    }

    pub fn offline_flag(&self) -> &OfflineFlag {
        &self.offline
    }

    /// Fails with `Offline` / `HostNotAllowed` / `BadUrl` without touching the network.
    pub fn check_url(&self, url: &str) -> Result<url::Url, NetError> {
        let _ = url;
        todo!("net agent")
    }

    /// GET + JSON decode. `headers` values are never logged (may hold an API key).
    pub async fn get_json<T: DeserializeOwned>(&self, url: &str, headers: &[(&str, &str)]) -> Result<T, NetError> {
        let _ = (url, headers);
        todo!("net agent")
    }

    /// GET raw bytes, capped at `max_bytes` (preview images etc.).
    pub async fn get_bytes(&self, url: &str, headers: &[(&str, &str)], max_bytes: usize) -> Result<Vec<u8>, NetError> {
        let _ = (url, headers, max_bytes);
        todo!("net agent")
    }

    /// Checked request builder for streaming downloads. `check_url` runs first,
    /// and the client's redirect policy re-checks every hop.
    pub fn get(&self, url: &str) -> Result<reqwest::RequestBuilder, NetError> {
        let _ = url;
        todo!("net agent")
    }
}

/// Loopback-only client for talking to our own engines on 127.0.0.1.
/// Works in Offline mode (no internet traffic), refuses any non-loopback host.
#[derive(Clone)]
pub struct LocalClient {
    pub(crate) inner: reqwest::Client,
}

impl LocalClient {
    pub fn new() -> Result<Self, NetError> {
        todo!("net agent")
    }

    /// `base` must be `http://127.0.0.1:<port>`.
    pub fn post_json<B: serde::Serialize + ?Sized>(&self, base: &str, path: &str, body: &B) -> Result<reqwest::RequestBuilder, NetError> {
        let _ = (base, path, body);
        todo!("net agent")
    }

    pub fn get(&self, base: &str, path: &str) -> Result<reqwest::RequestBuilder, NetError> {
        let _ = (base, path);
        todo!("net agent")
    }

    pub fn inner(&self) -> &reqwest::Client {
        &self.inner
    }
}
