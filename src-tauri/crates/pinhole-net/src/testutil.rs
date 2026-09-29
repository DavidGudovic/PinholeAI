//! Tiny HTTP/1.1 mock server on 127.0.0.1 for tests (feature `test-util`).
//! One request per connection (`Connection: close`). Records every request and
//! counts accepted TCP connections, so tests can prove that nothing connected.
//!
//! ```ignore
//! let srv = MockServer::start(|req| MockResponse::ok(b"hi".to_vec())).await;
//! let client = HttpClient::new_for_tests(OfflineFlag::new(false), true)?;
//! let body = client.get_bytes(&srv.url("/x"), &[], 1024).await?;
//! ```

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// A request as seen by the mock server. Header names are lowercase.
#[derive(Debug, Clone)]
pub struct MockRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl MockRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Start offset of a `Range: bytes=N-` header.
    pub fn range_start(&self) -> Option<u64> {
        self.header("range")?
            .strip_prefix("bytes=")?
            .split('-')
            .next()?
            .parse()
            .ok()
    }
}

#[derive(Debug, Clone)]
pub struct MockResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Send only this many body bytes, then keep the connection open (stall).
    pub stall_after: Option<usize>,
    /// Wait this long before sending anything (timeouts).
    pub delay: Option<Duration>,
    /// Omit `Content-Length` (body ends when the connection closes).
    pub no_content_length: bool,
}

impl MockResponse {
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
            stall_after: None,
            delay: None,
            no_content_length: false,
        }
    }
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self::new(200, body)
    }
    pub fn status(status: u16) -> Self {
        Self::new(status, Vec::new())
    }
    /// 302 with `Location`.
    pub fn redirect(location: &str) -> Self {
        Self::new(302, Vec::new()).header("location", location)
    }
    pub fn json(value: &serde_json::Value) -> Self {
        Self::ok(serde_json::to_vec(value).unwrap_or_default())
            .header("content-type", "application/json")
    }
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
    pub fn stall_after(mut self, bytes: usize) -> Self {
        self.stall_after = Some(bytes);
        self
    }
    pub fn delay(mut self, d: Duration) -> Self {
        self.delay = Some(d);
        self
    }
    pub fn without_content_length(mut self) -> Self {
        self.no_content_length = true;
        self
    }

    /// Serve `data` honouring `Range: bytes=N-` (206 + `Content-Range`), 416 when
    /// N is past the end, 200 otherwise.
    pub fn ranged(req: &MockRequest, data: &[u8]) -> Self {
        match req.range_start() {
            Some(start) if start as usize >= data.len() => {
                Self::status(416).header("content-range", &format!("bytes */{}", data.len()))
            }
            Some(start) => {
                let s = start as usize;
                Self::new(206, data[s..].to_vec())
                    .header(
                        "content-range",
                        &format!("bytes {}-{}/{}", s, data.len() - 1, data.len()),
                    )
                    .header("accept-ranges", "bytes")
            }
            None => Self::ok(data.to_vec()).header("accept-ranges", "bytes"),
        }
    }
}

type Handler = Arc<dyn Fn(&MockRequest) -> MockResponse + Send + Sync>;

struct Shared {
    connections: AtomicUsize,
    requests: Mutex<Vec<MockRequest>>,
}

pub struct MockServer {
    addr: SocketAddr,
    shared: Arc<Shared>,
    task: JoinHandle<()>,
}

impl MockServer {
    /// Bind 127.0.0.1 on a free port and serve `handler`.
    pub async fn start(
        handler: impl Fn(&MockRequest) -> MockResponse + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind mock server");
        let addr = listener.local_addr().expect("mock server addr");
        let shared = Arc::new(Shared {
            connections: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        });
        let handler: Handler = Arc::new(handler);
        let task_shared = shared.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                task_shared.connections.fetch_add(1, Ordering::SeqCst);
                let shared = task_shared.clone();
                let handler = handler.clone();
                tokio::spawn(async move {
                    let _ = serve(stream, shared, handler).await;
                });
            }
        });
        Self { addr, shared, task }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
    pub fn port(&self) -> u16 {
        self.addr.port()
    }
    /// `http://127.0.0.1:<port><path>`
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.addr.port(), path)
    }
    /// `http://localhost:<port><path>` — the test client's redirect-only "CDN".
    pub fn cdn_url(&self, path: &str) -> String {
        format!("http://localhost:{}{}", self.addr.port(), path)
    }
    /// Accepted TCP connections so far.
    pub fn connections(&self) -> usize {
        self.shared.connections.load(Ordering::SeqCst)
    }
    pub fn requests(&self) -> Vec<MockRequest> {
        self.shared.requests.lock().clone()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(
    mut stream: TcpStream,
    shared: Arc<Shared>,
    handler: Handler,
) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(4096);
    let head_end = loop {
        let mut tmp = [0u8; 4096];
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > 64 * 1024 {
            return Ok(());
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split(' ');
    let method = first.next().unwrap_or_default().to_string();
    let path = first.next().unwrap_or_default().to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let content_length = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < content_length {
        let mut tmp = [0u8; 4096];
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    let req = MockRequest {
        method,
        path,
        headers,
        body,
    };
    shared.requests.lock().push(req.clone());
    let resp = handler(&req);

    if let Some(d) = resp.delay {
        tokio::time::sleep(d).await;
    }
    let mut out = format!("HTTP/1.1 {} {}\r\n", resp.status, reason(resp.status));
    for (k, v) in &resp.headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    if !resp.no_content_length {
        out.push_str(&format!("content-length: {}\r\n", resp.body.len()));
    }
    out.push_str("connection: close\r\n\r\n");
    stream.write_all(out.as_bytes()).await?;
    match resp.stall_after {
        Some(n) => {
            stream
                .write_all(&resp.body[..n.min(resp.body.len())])
                .await?;
            stream.flush().await?;
            // Hold the connection open until the client gives up.
            tokio::time::sleep(Duration::from_secs(120)).await;
        }
        None => {
            stream.write_all(&resp.body).await?;
            stream.flush().await?;
        }
    }
    let _ = stream.shutdown().await;
    Ok(())
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        307 => "Temporary Redirect",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        416 => "Range Not Satisfiable",
        500 => "Internal Server Error",
        _ => "Status",
    }
}
