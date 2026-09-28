//! The ONE HTTP client wrapper (CLAUDE.md privacy rule 5). Every outbound
//! request in Pinhole goes through [`HttpClient`]. It enforces:
//! * Offline mode — every call fails with [`NetError::Offline`] before a socket opens
//!   (checked in [`HttpClient::check_url`], again in [`HttpClient::send`], on every
//!   redirect hop, in the DNS resolver, and on every received chunk);
//! * an allow-list of hosts (civitai.com, huggingface.co, github.com and their
//!   download CDNs, CDNs only reachable via redirect from their owner, see [`allow`]);
//! * HTTPS only (except [`LocalClient`], which only talks to 127.0.0.1).
//!
//! It never logs anything. Error values never contain URLs (they may carry
//! signed query strings or tokens), headers or bodies.
//!
//! Callers: build requests with [`HttpClient::get`] and send them with
//! [`HttpClient::send`] (re-checks offline, maps 401/403 → `Unauthorized`,
//! other non-2xx → `Status`, timeouts → `Timeout`). Put secrets (CivitAI API
//! key) only in the `Authorization` header — never in the query string, and
//! never in a custom header: reqwest strips `Authorization` on cross-host
//! redirects (tested) but forwards unknown headers.
//!
//! OWNER: net agent.

pub mod allow;
pub mod download;
#[cfg(any(test, feature = "test-util"))]
pub mod testutil;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{HeaderName, HeaderValue};
use serde::de::DeserializeOwned;

use allow::{HostKind, Rules};

/// `User-Agent` sent to every host.
pub const USER_AGENT: &str = concat!("Pinhole/", env!("CARGO_PKG_VERSION"));

/// TCP + TLS connect timeout for internet requests.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Max silence between two reads of a response body (stalled downloads).
pub const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// Whole-request timeout for [`HttpClient::get_json`] / [`HttpClient::get_bytes`].
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Size cap for [`HttpClient::get_json`] responses.
pub const MAX_JSON_BYTES: usize = 32 * 1024 * 1024;
/// Default whole-request timeout of [`LocalClient`] requests (override per request
/// with `RequestBuilder::timeout`).
pub const LOCAL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
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

impl From<reqwest::Error> for NetError {
    /// Maps a reqwest error without ever including its URL.
    fn from(e: reqwest::Error) -> Self {
        let e = e.without_url();
        // Errors raised by our redirect policy / resolver travel in the source chain.
        let mut src: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(&e);
        let mut root: Option<&(dyn std::error::Error + 'static)> = None;
        while let Some(s) = src {
            if let Some(n) = s.downcast_ref::<NetError>() {
                return n.clone();
            }
            if let Some(io) = s.downcast_ref::<std::io::Error>() {
                if io.kind() == std::io::ErrorKind::TimedOut {
                    return NetError::Timeout;
                }
            }
            root = Some(s);
            src = s.source();
        }
        if e.is_timeout() {
            return NetError::Timeout;
        }
        let mut msg = e.to_string();
        if let Some(r) = root {
            msg = format!("{msg}: {r}");
        }
        let msg = scrub_urls(&msg);
        if e.is_decode() {
            NetError::Decode(msg)
        } else {
            NetError::Transport(msg)
        }
    }
}

/// Replace anything that looks like a URL with `<url>` (defence in depth: error
/// text must never carry signed query strings or tokens).
fn scrub_urls(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let next = ["https://", "http://"].iter().filter_map(|p| rest.find(p)).min();
        match next {
            None => {
                out.push_str(rest);
                return out;
            }
            Some(i) => {
                out.push_str(&rest[..i]);
                out.push_str("<url>");
                let tail = &rest[i..];
                let end = tail.find(|c: char| c.is_whitespace() || c == ')' || c == '"' || c == '\'').unwrap_or(tail.len());
                rest = &tail[end..];
            }
        }
    }
}

/// Maps a response status: 401/403 → `Unauthorized`, other non-2xx → `Status`.
pub fn check_status(resp: reqwest::Response) -> Result<reqwest::Response, NetError> {
    let status = resp.status();
    let code = status.as_u16();
    if code == 401 || code == 403 {
        Err(NetError::Unauthorized(code))
    } else if !status.is_success() {
        Err(NetError::Status(code))
    } else {
        Ok(resp)
    }
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

/// DNS resolver that refuses to resolve anything while Offline mode is on, so
/// even a request built before Offline was switched on cannot open a socket.
struct GuardedResolver {
    offline: OfflineFlag,
}

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let offline = self.offline.get();
        let host = name.as_str().to_owned();
        Box::pin(async move {
            if offline {
                return Err(Box::new(NetError::Offline) as Box<dyn std::error::Error + Send + Sync>);
            }
            let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Internet client (allow-listed, offline-aware).
#[derive(Clone)]
pub struct HttpClient {
    pub(crate) offline: OfflineFlag,
    inner: reqwest::Client,
    rules: Rules,
}

impl HttpClient {
    pub fn new(offline: OfflineFlag) -> Result<Self, NetError> {
        Self::build(offline, Rules::default())
    }

    /// Test client: additionally allows `http://127.0.0.1:<port>` as a primary
    /// host (local mock servers) and `http://localhost:<port>` as a
    /// redirect-only "CDN" owned by 127.0.0.1 (resolved to 127.0.0.1). Ignores
    /// system proxies. Everything else behaves exactly like [`HttpClient::new`].
    #[cfg(any(test, feature = "test-util"))]
    pub fn new_for_tests(offline: OfflineFlag, allow_loopback_http: bool) -> Result<Self, NetError> {
        Self::build(offline, Rules { loopback_http: allow_loopback_http })
    }

    fn build(offline: OfflineFlag, rules: Rules) -> Result<Self, NetError> {
        let policy_offline = offline.clone();
        let redirect = reqwest::redirect::Policy::custom(move |attempt| {
            match allow::check_redirect(attempt.url(), attempt.previous(), policy_offline.get(), rules) {
                Ok(()) => attempt.follow(),
                Err(e) => attempt.error(e),
            }
        });
        let mut builder = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .redirect(redirect)
            .referer(false)
            .https_only(!rules.loopback_http)
            .dns_resolver(Arc::new(GuardedResolver { offline: offline.clone() }));
        if rules.loopback_http {
            builder = builder
                .no_proxy()
                .resolve("localhost", std::net::SocketAddr::from(([127, 0, 0, 1], 0)));
        }
        let inner = builder
            .build()
            .map_err(|e| NetError::Transport(scrub_urls(&format!("could not start the HTTP client: {e}"))))?;
        Ok(Self { offline, inner, rules })
    }

    pub fn offline_flag(&self) -> &OfflineFlag {
        &self.offline
    }

    /// Fails with `Offline` / `HostNotAllowed` / `BadUrl` without touching the network.
    /// Offline is checked first. Only primary hosts may be requested directly.
    pub fn check_url(&self, url: &str) -> Result<url::Url, NetError> {
        if self.offline.get() {
            return Err(NetError::Offline);
        }
        let parsed = url::Url::parse(url).map_err(|e| NetError::BadUrl(e.to_string()))?;
        self.check_parsed(&parsed)?;
        Ok(parsed)
    }

    fn check_parsed(&self, url: &url::Url) -> Result<(), NetError> {
        if self.offline.get() {
            return Err(NetError::Offline);
        }
        match allow::classify(url, self.rules)? {
            HostKind::Primary => Ok(()),
            HostKind::Cdn => Err(NetError::HostNotAllowed(url.host_str().unwrap_or_default().to_string())),
        }
    }

    /// GET + JSON decode (capped at [`MAX_JSON_BYTES`], [`REQUEST_TIMEOUT`]).
    /// `headers` values are never logged (may hold an API key).
    pub async fn get_json<T: DeserializeOwned>(&self, url: &str, headers: &[(&str, &str)]) -> Result<T, NetError> {
        let mut all: Vec<(&str, &str)> = Vec::with_capacity(headers.len() + 1);
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("accept")) {
            all.push(("accept", "application/json"));
        }
        all.extend_from_slice(headers);
        let bytes = self.get_bytes(url, &all, MAX_JSON_BYTES).await?;
        serde_json::from_slice(&bytes).map_err(|e| NetError::Decode(e.to_string()))
    }

    /// GET raw bytes, capped at `max_bytes` while streaming (preview images etc.).
    pub async fn get_bytes(&self, url: &str, headers: &[(&str, &str)], max_bytes: usize) -> Result<Vec<u8>, NetError> {
        let rb = with_headers(self.get(url)?, headers)?.timeout(REQUEST_TIMEOUT);
        let resp = self.send(rb).await?;
        if let Some(len) = resp.content_length() {
            if len > max_bytes as u64 {
                return Err(NetError::TooLarge);
            }
        }
        let cap = resp.content_length().map(|l| l as usize).unwrap_or(64 * 1024).min(max_bytes);
        let mut out = Vec::with_capacity(cap);
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if self.offline.get() {
                return Err(NetError::Offline);
            }
            if out.len() + chunk.len() > max_bytes {
                return Err(NetError::TooLarge);
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    /// Checked request builder for streaming downloads. `check_url` runs first,
    /// and the client's redirect policy re-checks every hop. Send it with
    /// [`HttpClient::send`].
    pub fn get(&self, url: &str) -> Result<reqwest::RequestBuilder, NetError> {
        let parsed = self.check_url(url)?;
        Ok(self.inner.get(parsed))
    }

    /// Send a request built by [`HttpClient::get`]: re-checks Offline and the
    /// allow-list, maps transport errors (never including the URL) and statuses
    /// (401/403 → `Unauthorized`, other non-2xx → `Status`).
    pub async fn send(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::Response, NetError> {
        check_status(self.send_unchecked_status(rb).await?)
    }

    /// Like [`HttpClient::send`] but returns any HTTP status (e.g. 206/416 for
    /// resumable downloads).
    pub(crate) async fn send_unchecked_status(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::Response, NetError> {
        let (_, req) = rb.build_split();
        let req = req.map_err(NetError::from)?;
        self.check_parsed(req.url())?;
        self.inner.execute(req).await.map_err(NetError::from)
    }
}

/// Attach caller headers, marking every value sensitive (hidden from `Debug`).
pub(crate) fn with_headers(
    mut rb: reqwest::RequestBuilder,
    headers: &[(impl AsRef<str>, impl AsRef<str>)],
) -> Result<reqwest::RequestBuilder, NetError> {
    for (k, v) in headers {
        let name = HeaderName::from_bytes(k.as_ref().as_bytes())
            .map_err(|_| NetError::Transport("invalid request header name".into()))?;
        let mut value =
            HeaderValue::from_str(v.as_ref()).map_err(|_| NetError::Transport("invalid request header value".into()))?;
        value.set_sensitive(true);
        rb = rb.header(name, value);
    }
    Ok(rb)
}

/// Loopback-only client for talking to our own engines on 127.0.0.1.
/// Works in Offline mode (no internet traffic), refuses any non-loopback host,
/// never follows redirects and ignores system proxies.
///
/// Default whole-request timeout is [`LOCAL_TIMEOUT`]; long calls (e.g. a
/// synchronous upscale) override it per request with
/// `local.post_json(..)?.timeout(Duration::from_secs(900))`.
#[derive(Clone)]
pub struct LocalClient {
    pub(crate) inner: reqwest::Client,
}

impl LocalClient {
    pub fn new() -> Result<Self, NetError> {
        let inner = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .connect_timeout(Duration::from_secs(5))
            .timeout(LOCAL_TIMEOUT)
            .build()
            .map_err(|e| NetError::Transport(scrub_urls(&format!("could not start the local HTTP client: {e}"))))?;
        Ok(Self { inner })
    }

    /// Joins `base` + `path` and checks the result is `http://127.0.0.1:<port>/…`.
    pub fn url(&self, base: &str, path: &str) -> Result<url::Url, NetError> {
        let base = url::Url::parse(base).map_err(|e| NetError::BadUrl(e.to_string()))?;
        check_loopback(&base)?;
        let joined = base.join(path).map_err(|e| NetError::BadUrl(e.to_string()))?;
        check_loopback(&joined)?;
        Ok(joined)
    }

    /// `base` must be `http://127.0.0.1:<port>`.
    pub fn post_json<B: serde::Serialize + ?Sized>(&self, base: &str, path: &str, body: &B) -> Result<reqwest::RequestBuilder, NetError> {
        let url = self.url(base, path)?;
        Ok(self.inner.post(url).json(body))
    }

    pub fn get(&self, base: &str, path: &str) -> Result<reqwest::RequestBuilder, NetError> {
        let url = self.url(base, path)?;
        Ok(self.inner.get(url))
    }

    pub fn inner(&self) -> &reqwest::Client {
        &self.inner
    }
}

fn check_loopback(url: &url::Url) -> Result<(), NetError> {
    let loopback = url.scheme() == "http"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_some()
        && matches!(url.host(), Some(url::Host::Ipv4(ip)) if ip == std::net::Ipv4Addr::LOCALHOST);
    if loopback {
        Ok(())
    } else {
        Err(NetError::HostNotAllowed(url.host_str().unwrap_or_default().to_string()))
    }
}
