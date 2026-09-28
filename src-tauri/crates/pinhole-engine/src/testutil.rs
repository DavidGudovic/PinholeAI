//! Test doubles (feature `test-util`): tiny HTTP servers on 127.0.0.1 that
//! speak enough of sd-server's `/sdcpp/v1` API and llama-server's chat API for
//! integration and privacy tests.
//!
//! [`MockSdServer`] deliberately behaves like sd-server's *default*
//! (`embed_image_metadata: true`): every PNG it returns carries the received
//! prompt and negative prompt in a `tEXt` "parameters" chunk and an `iTXt`
//! "prompt" chunk — so tests prove Pinhole scrubs them before anything is kept.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use base64::Engine as _;
use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

/// Behaviour knobs for [`MockSdServer`].
#[derive(Debug, Clone)]
pub struct MockOptions {
    /// Number of `GET /jobs/{id}` polls answered before the job completes.
    /// The first poll says `queued`, later ones `generating`.
    pub polls_before_done: u32,
    /// Finish jobs with `status: failed` and this message instead of images.
    pub fail_with: Option<String>,
    /// With `fail_with`: only the first this-many jobs fail (0 = every job).
    pub fail_first: u32,
    /// Like the real sd-server: jobs fail in submission order (job 1 → `[0]`,
    /// job 2 → `[1]` …) with only `generate_image returned no results` as the
    /// job error, while the reason is "printed" (one line per `\n`) to
    /// [`MockOptions::engine_log`] (else appended to the job error). Later
    /// jobs succeed (or follow `fail_with`).
    pub fail_outputs: Vec<String>,
    /// The engine output buffer the mock prints to (e.g. the core's ring
    /// buffer via `pinhole_core::testing::engine_log`): lines go through its
    /// redaction like real engine output.
    pub engine_log: Option<Arc<crate::LogBuffer>>,
    /// Cap for the side length of returned images (keeps tests fast).
    pub max_side: u32,
}

impl Default for MockOptions {
    fn default() -> Self {
        Self { polls_before_done: 2, fail_with: None, fail_first: 0, fail_outputs: Vec::new(), engine_log: None, max_side: 1024 }
    }
}

/// sd-server's job error when `generate_image` fails (examples/server/async_jobs.cpp).
pub const NO_RESULTS: &str = "generate_image returned no results";

#[derive(Default)]
struct SdState {
    requests: Vec<Value>,
    upscale_requests: Vec<Value>,
    cancels: Vec<String>,
    jobs: HashMap<String, MockJob>,
    next_id: u64,
}

struct MockJob {
    body: Value,
    polls: u32,
    cancelled: bool,
    /// 1-based submission order.
    seq: u64,
}

/// A fake sd-server. Dropping it stops the server.
pub struct MockSdServer {
    addr: SocketAddr,
    state: Arc<Mutex<SdState>>,
    stop: CancellationToken,
}

impl Drop for MockSdServer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

impl MockSdServer {
    pub async fn start() -> Self {
        Self::start_with(MockOptions::default()).await
    }

    pub async fn start_with(opts: MockOptions) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind mock sd-server");
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(SdState::default()));
        let stop = CancellationToken::new();
        let (st, stop2, opts) = (state.clone(), stop.clone(), Arc::new(opts));
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop2.cancelled() => break,
                    acc = listener.accept() => {
                        let Ok((sock, _)) = acc else { continue };
                        let (st, opts) = (st.clone(), opts.clone());
                        tokio::spawn(async move {
                            let _ = serve(sock, move |m, p, _h, b| sd_route(&st, &opts, m, p, b)).await;
                        });
                    }
                }
            }
        });
        Self { addr, state, stop }
    }

    /// `http://127.0.0.1:<port>`
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }

    /// Every `img_gen` body received, in order.
    pub fn requests(&self) -> Vec<Value> {
        self.state.lock().requests.clone()
    }

    /// Every `upscale` body received (image field replaced by its length).
    pub fn upscale_requests(&self) -> Vec<Value> {
        self.state.lock().upscale_requests.clone()
    }

    /// Job ids that received a cancel request.
    pub fn cancels(&self) -> Vec<String> {
        self.state.lock().cancels.clone()
    }
}

fn sd_route(st: &Mutex<SdState>, opts: &MockOptions, method: &str, path: &str, body: &[u8]) -> (u16, Value) {
    match (method, path) {
        ("GET", "/sdcpp/v1/capabilities") => (200, capabilities()),
        ("POST", "/sdcpp/v1/img_gen") => {
            let Ok(v) = serde_json::from_slice::<Value>(body) else { return (400, json!({"error": "invalid json"})) };
            let mut g = st.lock();
            g.next_id += 1;
            let id = format!("job_mock_{:08}", g.next_id);
            g.requests.push(v.clone());
            let seq = g.next_id;
            g.jobs.insert(id.clone(), MockJob { body: v, polls: 0, cancelled: false, seq });
            (202, json!({"id": id, "kind": "img_gen", "status": "queued", "created": 1, "poll_url": format!("/sdcpp/v1/jobs/{id}")}))
        }
        ("POST", "/sdcpp/v1/upscale") => {
            let Ok(v) = serde_json::from_slice::<Value>(body) else { return (400, json!({"error": "invalid json"})) };
            let img = v.get("image").and_then(|i| i.as_str()).unwrap_or("");
            let raw = base64::engine::general_purpose::STANDARD.decode(img.rsplit(',').next().unwrap_or("")).unwrap_or_default();
            let repeats = v.get("repeats").and_then(|r| r.as_u64()).unwrap_or(1).clamp(1, 4) as u32;
            let mut logged = v.clone();
            logged["image"] = json!(img.len());
            st.lock().upscale_requests.push(logged);
            let Ok(info) = crate::image::sniff(&raw) else { return (400, json!({"error": "image could not be read"})) };
            let f = 4u32.pow(repeats);
            let (w, h) = ((info.width * f).min(opts.max_side * 4), (info.height * f).min(opts.max_side * 4));
            let png = solid_png(w, h, [40, 90, 200, 255]);
            (200, json!({"images": [{"index": 0, "b64_json": b64(&png)}], "upscaler": "RealESRGAN_x4plus", "scale": 4, "repeats": repeats, "width": w, "height": h, "output_format": "png"}))
        }
        ("GET", p) if p.starts_with("/sdcpp/v1/jobs/") => {
            let id = &p["/sdcpp/v1/jobs/".len()..];
            let mut g = st.lock();
            let Some(job) = g.jobs.get_mut(id) else { return (404, json!({"error": "job not found"})) };
            if job.cancelled {
                return (200, json!({"id": id, "status": "cancelled", "queue_position": 0, "result": null, "error": {"code": "cancelled", "message": "job cancelled by client"}}));
            }
            let polls = job.polls;
            job.polls += 1;
            if polls < opts.polls_before_done {
                let status = if polls == 0 { "queued" } else { "generating" };
                return (200, json!({"id": id, "status": status, "queue_position": if polls == 0 { 1 } else { 0 }, "result": null, "error": null}));
            }
            if let Some(output) = usize::try_from(job.seq).ok().and_then(|n| opts.fail_outputs.get(n.wrapping_sub(1))) {
                let msg = match &opts.engine_log {
                    Some(log) => {
                        output.lines().for_each(|l| log.push_line(l));
                        NO_RESULTS.to_string()
                    }
                    None => format!("{NO_RESULTS}\n{output}"),
                };
                return (200, json!({"id": id, "status": "failed", "queue_position": 0, "result": null, "error": {"code": "generation_failed", "message": msg}}));
            }
            let fails = opts.fail_first == 0 || job.seq <= u64::from(opts.fail_first);
            if let Some(msg) = opts.fail_with.as_ref().filter(|_| fails) {
                return (200, json!({"id": id, "status": "failed", "queue_position": 0, "result": null, "error": {"code": "generation_failed", "message": msg}}));
            }
            let images = render(&job.body, opts);
            (200, json!({"id": id, "status": "completed", "queue_position": 0, "result": {"output_format": "png", "images": images}, "error": null}))
        }
        ("POST", p) if p.starts_with("/sdcpp/v1/jobs/") && p.ends_with("/cancel") => {
            let id = p["/sdcpp/v1/jobs/".len()..p.len() - "/cancel".len()].to_string();
            let mut g = st.lock();
            g.cancels.push(id.clone());
            let Some(job) = g.jobs.get_mut(&id) else { return (404, json!({"error": "job not found"})) };
            if job.polls == 0 || job.cancelled {
                job.cancelled = true;
                (200, json!({"id": id, "status": "cancelled", "queue_position": 0, "result": null, "error": {"code": "cancelled", "message": "job cancelled by client"}}))
            } else if job.polls <= opts.polls_before_done {
                (409, json!({"error": "job is currently generating and cannot be interrupted yet"}))
            } else {
                (200, json!({"id": id, "status": "completed", "queue_position": 0}))
            }
        }
        _ => (404, json!({"error": "not found"})),
    }
}

fn capabilities() -> Value {
    json!({
        "model": {"name": "mock.safetensors", "stem": "mock", "path": "/mock/mock.safetensors"},
        "current_mode": "img_gen",
        "supported_modes": ["img_gen"],
        "samplers": ["euler", "euler_a", "dpm++2m"],
        "schedulers": ["discrete", "karras", "simple"],
        "loras": [],
        "upscalers": [{"name": "RealESRGAN_x4plus", "model": true, "image_upscale": true}],
        "upscale": true,
        "limits": {"min_width": 64, "max_width": 4096, "min_height": 64, "max_height": 4096, "max_batch_count": 8}
    })
}

/// One PNG per `batch_count`, each carrying the prompt in text chunks (like the
/// real server with `embed_image_metadata: true`).
fn render(body: &Value, opts: &MockOptions) -> Vec<Value> {
    let w = body.get("width").and_then(|v| v.as_u64()).unwrap_or(512).clamp(8, opts.max_side as u64) as u32;
    let h = body.get("height").and_then(|v| v.as_u64()).unwrap_or(512).clamp(8, opts.max_side as u64) as u32;
    let n = body.get("batch_count").and_then(|v| v.as_u64()).unwrap_or(1).clamp(1, 8);
    let prompt = body.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    let negative = body.get("negative_prompt").and_then(|v| v.as_str()).unwrap_or("");
    let seed = body.get("seed").and_then(|v| v.as_i64()).unwrap_or(0);
    (0..n)
        .map(|i| {
            let png = solid_png(w, h, [(seed as u8).wrapping_add(i as u8), 120, 60, 255]);
            let params = format!("{prompt}\nNegative prompt: {negative}\nSteps: 20, Seed: {}", seed + i as i64);
            let png = crate::png::add_text_chunk(&png, "parameters", &params).expect("png");
            let mut itxt = b"prompt\0\0\0\0\0".to_vec();
            itxt.extend_from_slice(prompt.as_bytes());
            let png = crate::png::insert_chunk(&png, b"iTXt", &itxt).expect("png");
            json!({"index": i, "b64_json": b64(&png)})
        })
        .collect()
}

/// A solid-colour PNG (no metadata).
pub fn solid_png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let px: Vec<u8> = rgba.iter().copied().cycle().take((w * h * 4) as usize).collect();
    crate::image::encode_png_rgba(&px, w, h).expect("encode")
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

// ---------------------------------------------------------------- llama mock

#[derive(Default)]
struct LlamaState {
    requests: Vec<Value>,
    health_polls: u32,
}

/// Model id [`MockLlamaServer`] reports at `GET /v1/models`.
pub const MOCK_LLAMA_MODEL: &str = "/mock/captioner.gguf";

/// A fake llama-server: `/health` (503 for the first `loading_polls`),
/// `/v1/models` and `/v1/chat/completions` answering `reply`. With an API key
/// everything but `/health` needs `Authorization: Bearer <key>` (401 otherwise),
/// like llama-server started with `--api-key` / `LLAMA_API_KEY`.
pub struct MockLlamaServer {
    addr: SocketAddr,
    state: Arc<Mutex<LlamaState>>,
    stop: CancellationToken,
}

impl Drop for MockLlamaServer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

impl MockLlamaServer {
    pub async fn start(reply: &str, loading_polls: u32) -> Self {
        Self::start_with_key(reply, loading_polls, None).await
    }

    pub async fn start_with_key(reply: &str, loading_polls: u32, api_key: Option<&str>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind mock llama-server");
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(LlamaState::default()));
        let stop = CancellationToken::new();
        let (st, stop2, reply) = (state.clone(), stop.clone(), Arc::new(reply.to_string()));
        let expected_auth = Arc::new(api_key.map(|k| format!("Bearer {k}")));
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop2.cancelled() => break,
                    acc = listener.accept() => {
                        let Ok((sock, _)) = acc else { continue };
                        let (st, reply, expected_auth) = (st.clone(), reply.clone(), expected_auth.clone());
                        tokio::spawn(async move {
                            let _ = serve(sock, move |m, p, h, b| {
                                let mut g = st.lock();
                                let authorized = match expected_auth.as_deref() {
                                    None => true,
                                    Some(want) => h.iter().any(|(k, v)| k.eq_ignore_ascii_case("authorization") && v == want),
                                };
                                match (m, p) {
                                    ("GET", "/health") => {
                                        g.health_polls += 1;
                                        if g.health_polls <= loading_polls {
                                            (503, json!({"error": {"code": 503, "message": "Loading model"}}))
                                        } else {
                                            (200, json!({"status": "ok"}))
                                        }
                                    }
                                    _ if !authorized => {
                                        (401, json!({"error": {"message": "Invalid API Key", "type": "authentication_error", "code": 401}}))
                                    }
                                    ("GET", "/v1/models") => (200, json!({"object": "list", "data": [{"id": MOCK_LLAMA_MODEL, "object": "model"}]})),
                                    ("POST", "/v1/chat/completions") => {
                                        let v: Value = serde_json::from_slice(b).unwrap_or_default();
                                        g.requests.push(v);
                                        (200, json!({"choices": [{"index": 0, "message": {"role": "assistant", "content": reply.as_str()}}]}))
                                    }
                                    _ => (404, json!({"error": "not found"})),
                                }
                            })
                            .await;
                        });
                    }
                }
            }
        });
        Self { addr, state, stop }
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }

    /// Chat bodies received.
    pub fn requests(&self) -> Vec<Value> {
        self.state.lock().requests.clone()
    }
}

// ---------------------------------------------------------------- tiny HTTP/1.1

async fn serve<F>(mut sock: TcpStream, route: F) -> std::io::Result<()>
where
    F: Fn(&str, &str, &[(String, String)], &[u8]) -> (u16, Value),
{
    let mut buf = Vec::with_capacity(8192);
    let header_end = loop {
        let mut chunk = [0u8; 8192];
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > 1 << 20 {
            return Ok(());
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    let len: usize = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < len {
        let mut chunk = vec![0u8; (len - body.len()).min(1 << 16)];
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    let (code, value) = route(&method, &path, &headers, &body);
    let payload = value.to_string();
    let reason = match code {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        409 => "Conflict",
        503 => "Service Unavailable",
        _ => "Status",
    };
    let resp = format!("HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", payload.len());
    sock.write_all(resp.as_bytes()).await?;
    sock.write_all(payload.as_bytes()).await?;
    sock.shutdown().await
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdapi::{CancelOutcome, ImgGenRequest, JobStatus, SdClient, UpscaleRequest};

    #[tokio::test]
    async fn job_lifecycle_against_mock() {
        let mock = MockSdServer::start().await;
        let c = SdClient::new_plain_for_tests(mock.base_url());
        assert!(c.is_ready().await);
        let caps = c.capabilities().await.unwrap();
        assert!(caps.upscale);
        let mut req = ImgGenRequest::new("PINHOLE_SENTINEL_7f3a", 64, 48, 7);
        req.batch_count = 2;
        let id = c.submit(&req).await.unwrap();
        let first = c.job(&id).await.unwrap();
        assert_eq!((first.status, first.queue_position), (JobStatus::Queued, 1));
        assert_eq!(c.job(&id).await.unwrap().status, JobStatus::Generating);
        let done = c.job(&id).await.unwrap();
        assert_eq!(done.status, JobStatus::Completed);
        let imgs = done.result.unwrap().images;
        assert_eq!(imgs.len(), 2);
        // The mock embeds the prompt like the real server's default…
        let png = base64::engine::general_purpose::STANDARD.decode(&imgs[0].b64_json).unwrap();
        assert!(png.windows(21).any(|w| w == b"PINHOLE_SENTINEL_7f3a"));
        // …and scrubbing removes it.
        let clean = crate::png::scrub(&png).unwrap();
        assert!(!clean.windows(21).any(|w| w == b"PINHOLE_SENTINEL_7f3a"));
        assert_eq!(crate::png::dimensions(&clean), Some((64, 48)));
        // The request that reached the "engine" had metadata embedding off.
        let reqs = mock.requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0]["embed_image_metadata"], json!(false));
        assert_eq!(c.job("job_missing").await.unwrap_err(), crate::sdapi::ApiError::NotFound);
    }

    #[tokio::test]
    async fn cancel_queued_vs_generating() {
        let mock = MockSdServer::start_with(MockOptions { polls_before_done: 5, ..Default::default() }).await;
        let c = SdClient::new_plain_for_tests(mock.base_url());
        let id = c.submit(&ImgGenRequest::new("x", 64, 64, 1)).await.unwrap();
        assert_eq!(c.cancel(&id).await.unwrap(), CancelOutcome::Cancelled);
        assert_eq!(c.job(&id).await.unwrap().status, JobStatus::Cancelled);

        let id2 = c.submit(&ImgGenRequest::new("y", 64, 64, 1)).await.unwrap();
        c.job(&id2).await.unwrap();
        c.job(&id2).await.unwrap();
        assert_eq!(c.cancel(&id2).await.unwrap(), CancelOutcome::Running);
        assert_eq!(c.cancel("job_nope").await.unwrap(), CancelOutcome::Gone);
        assert_eq!(mock.cancels().len(), 3);
    }

    #[tokio::test]
    async fn failed_jobs_and_upscale() {
        let mock = MockSdServer::start_with(MockOptions { polls_before_done: 0, fail_with: Some("generate_image returned no results".into()), ..Default::default() }).await;
        let c = SdClient::new_plain_for_tests(mock.base_url());
        let id = c.submit(&ImgGenRequest::new("x", 64, 64, 1)).await.unwrap();
        let j = c.job(&id).await.unwrap();
        assert_eq!(j.status, JobStatus::Failed);
        assert_eq!(j.error.unwrap().code, "generation_failed");

        let src = solid_png(16, 8, [1, 2, 3, 255]);
        let up = c.upscale(&UpscaleRequest::new(b64(&src), None, 1)).await.unwrap();
        assert_eq!((up.width, up.height, up.scale), (64, 32, 4));
        assert_eq!(mock.upscale_requests()[0]["repeats"], 1);
    }

    #[tokio::test]
    async fn llama_mock_describes() {
        let mock = MockLlamaServer::start("Prompt: a red bicycle", 1).await;
        let c = crate::llama::LlamaClient::new_plain_for_tests(mock.base_url());
        assert!(!c.is_ready().await, "503 while loading");
        assert!(c.is_ready().await);
        let text = c.describe("Describe this image.", "image/png", &solid_png(4, 4, [0, 0, 0, 255]), 64).await.unwrap();
        assert_eq!(text, "a red bicycle");
        let reqs = mock.requests();
        let url = reqs[0].pointer("/messages/0/content/0/image_url/url").and_then(|u| u.as_str()).unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
    }

    #[tokio::test]
    async fn llama_api_key_is_required_and_sent() {
        let mock = MockLlamaServer::start_with_key("a cat", 0, Some("k-123")).await;
        let anon = crate::llama::LlamaClient::new_plain_for_tests(mock.base_url());
        assert!(anon.is_ready().await, "/health stays public");
        let err = anon.describe("Describe.", "image/png", &solid_png(2, 2, [0, 0, 0, 255]), 8).await.unwrap_err();
        assert!(matches!(err, crate::sdapi::ApiError::Status { code: 401, .. }), "{err:?}");
        assert!(matches!(anon.model_ids().await, Err(crate::sdapi::ApiError::Status { code: 401, .. })));
        let wrong = anon.clone().with_api_key("nope");
        assert!(wrong.model_ids().await.is_err());
        let c = anon.with_api_key("k-123");
        assert_eq!(c.model_ids().await.unwrap(), vec![MOCK_LLAMA_MODEL.to_string()]);
        assert_eq!(c.describe("Describe.", "image/png", &solid_png(2, 2, [0, 0, 0, 255]), 8).await.unwrap(), "a cat");
        assert_eq!(mock.requests().len(), 1, "only the authorized chat request got through");
    }
}
