//! Typed client for sd-server's native async API (`/sdcpp/v1/...`, see
//! upstream `examples/server/api.md`), over the loopback-only
//! [`pinhole_net::LocalClient`].
//!
//! PRIVACY: [`ImgGenRequest`] is prompt-bearing. It is `Serialize` only (for
//! the loopback HTTP body), its `Debug` redacts every text/image field, and its
//! constructor ALWAYS sets `embed_image_metadata: false` (the server default is
//! `true`, which would bake the prompt into the PNG). The field is private so no
//! caller can flip it.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::words::CheckedPrompt;

// ---------------------------------------------------------------- request

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Guidance {
    /// Sent as at least 1 ([`MIN_CFG`]). Below 1, sd.cpp leans towards the negative prompt
    /// (at 0 it follows only the negative), and the negative prompt isn't word-checked.
    #[serde(serialize_with = "ser_cfg")]
    pub txt_cfg: f32,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "ser_opt_cfg"
    )]
    pub img_cfg: Option<f32>,
    /// Flux-style distilled guidance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distilled_guidance: Option<f32>,
}

/// The lowest CFG the engine is ever sent. At CFG 1 sd.cpp ignores the negative prompt; above
/// 1 it steers away from it. Enforced where the request is serialized, so no setting, pasted
/// value or IPC call can send less. Matches `pinhole_registry::wiring::MIN_CFG`, which keeps
/// what Fine-tune shows honest (the crates don't depend on each other).
pub const MIN_CFG: f32 = 1.0;

/// `cfg` raised to [`MIN_CFG`]; a non-finite value becomes [`MIN_CFG`].
pub fn safe_cfg(cfg: f32) -> f32 {
    if cfg.is_finite() {
        cfg.max(MIN_CFG)
    } else {
        MIN_CFG
    }
}

fn ser_cfg<S: serde::Serializer>(v: &f32, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f32(safe_cfg(*v))
}

fn ser_opt_cfg<S: serde::Serializer>(v: &Option<f32>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(v) => s.serialize_some(&safe_cfg(*v)),
        None => s.serialize_none(),
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct SampleParams {
    /// sd.cpp sampler name (`euler`, `euler_a`, `dpm++2m`…). `None` = server default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduler: Option<String>,
    pub sample_steps: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow_shift: Option<f32>,
    pub guidance: Guidance,
}

/// Structured LoRA (`<lora:…>` prompt tags are NOT supported by the server).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LoraRef {
    /// Path relative to `--lora-model-dir`, `/`-separated.
    pub path: String,
    pub multiplier: f32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HiresRequest {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upscaler: Option<String>,
    pub scale: f32,
    pub steps: u32,
    pub denoising_strength: f32,
}

/// Hires upscaler sent when no upscaler model is chosen. sd-server's default is
/// `Latent`, which blurs and garbles the image unless the second pass denoises
/// heavily (about 0.5+); copied CivitAI settings often use 0.3–0.4, meant for an
/// image upscaler. `Lanczos` upscales the decoded image instead, so low
/// strengths only refine it.
pub const HIRES_IMAGE_UPSCALER: &str = "Lanczos";

impl HiresRequest {
    /// Hires fix with an image-space upscale ([`HIRES_IMAGE_UPSCALER`]).
    /// `steps` 0 = reuse the first pass's steps.
    pub fn image_space(scale: f32, steps: u32, denoising_strength: f32) -> Self {
        Self {
            enabled: true,
            upscaler: Some(HIRES_IMAGE_UPSCALER.into()),
            scale,
            steps,
            denoising_strength,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VaeTilingRequest {
    pub enabled: bool,
}

/// Body of `POST /sdcpp/v1/img_gen`. Build with [`ImgGenRequest::new`].
#[derive(Clone, Serialize)]
pub struct ImgGenRequest {
    /// Private: set only by [`ImgGenRequest::new`] from a word-checked prompt.
    prompt: String,
    pub negative_prompt: String,
    /// -1 = model default.
    pub clip_skip: i32,
    pub width: u32,
    pub height: u32,
    /// img2img strength (0..1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<f32>,
    pub seed: i64,
    pub batch_count: u32,
    /// Always `false` — see module docs. Private on purpose.
    embed_image_metadata: bool,
    /// Base64 (or data URL) images.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub init_image: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ref_images: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask_image: Option<String>,
    pub sample_params: SampleParams,
    pub lora: Vec<LoraRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hires: Option<HiresRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vae_tiling_params: Option<VaeTilingRequest>,
    output_format: &'static str,
    output_compression: u8,
}

impl ImgGenRequest {
    /// New request: PNG output, metadata embedding OFF, one image, 20 steps.
    /// The prompt must have passed the word check ([`CheckedPrompt`]).
    pub fn new(prompt: CheckedPrompt, width: u32, height: u32, seed: i64) -> Self {
        Self {
            prompt: prompt.into_string(),
            negative_prompt: String::new(),
            clip_skip: -1,
            width,
            height,
            strength: None,
            seed,
            batch_count: 1,
            embed_image_metadata: false,
            init_image: None,
            ref_images: Vec::new(),
            mask_image: None,
            sample_params: SampleParams {
                sample_steps: 20,
                guidance: Guidance {
                    txt_cfg: 7.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            lora: Vec::new(),
            hires: None,
            vae_tiling_params: None,
            output_format: "png",
            output_compression: 100,
        }
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn embeds_metadata(&self) -> bool {
        self.embed_image_metadata
    }
}

impl std::fmt::Debug for ImgGenRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImgGenRequest")
            .field("prompt", &"[redacted]")
            .field("negative_prompt", &"[redacted]")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("seed", &self.seed)
            .field("batch_count", &self.batch_count)
            .field("steps", &self.sample_params.sample_steps)
            .field("init_image", &self.init_image.is_some())
            .field("ref_images", &self.ref_images.len())
            .field("mask_image", &self.mask_image.is_some())
            .field("lora", &self.lora.len())
            .finish()
    }
}

/// Body of `POST /sdcpp/v1/upscale`.
#[derive(Clone, Serialize)]
pub struct UpscaleRequest {
    pub image: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upscaler: Option<String>,
    pub repeats: u32,
    output_format: &'static str,
}

impl UpscaleRequest {
    pub fn new(image_b64: String, upscaler: Option<String>, repeats: u32) -> Self {
        Self {
            image: image_b64,
            upscaler,
            repeats: repeats.clamp(1, 4),
            output_format: "png",
        }
    }
}

impl std::fmt::Debug for UpscaleRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpscaleRequest")
            .field("upscaler", &self.upscaler)
            .field("repeats", &self.repeats)
            .finish()
    }
}

// ---------------------------------------------------------------- responses

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Generating,
    Completed,
    Failed,
    Cancelled,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Deserialize)]
pub struct JobImage {
    #[serde(default)]
    pub index: u32,
    pub b64_json: String,
}

impl std::fmt::Debug for JobImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobImage")
            .field("index", &self.index)
            .field("b64_len", &self.b64_json.len())
            .finish()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct JobResult {
    #[serde(default)]
    pub output_format: Option<String>,
    #[serde(default)]
    pub images: Vec<JobImage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct JobError {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Job {
    pub id: String,
    pub status: JobStatus,
    #[serde(default)]
    pub queue_position: u32,
    #[serde(default)]
    pub result: Option<JobResult>,
    #[serde(default)]
    pub error: Option<JobError>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CapModel {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub stem: String,
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CapUpscaler {
    pub name: String,
    #[serde(default)]
    pub model: bool,
    #[serde(default)]
    pub image_upscale: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CapLora {
    pub name: String,
    pub path: String,
}

/// Subset of `GET /sdcpp/v1/capabilities` Pinhole uses.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub model: Option<CapModel>,
    #[serde(default)]
    pub supported_modes: Vec<String>,
    #[serde(default)]
    pub samplers: Vec<String>,
    #[serde(default)]
    pub schedulers: Vec<String>,
    #[serde(default)]
    pub loras: Vec<CapLora>,
    #[serde(default)]
    pub upscalers: Vec<CapUpscaler>,
    /// A compatible RGB ESRGAN model is available for `/upscale`.
    #[serde(default)]
    pub upscale: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpscaleResponse {
    #[serde(default)]
    pub images: Vec<JobImage>,
    #[serde(default)]
    pub upscaler: String,
    #[serde(default)]
    pub scale: u32,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// Was queued, now cancelled.
    Cancelled,
    /// sd-server can't interrupt a job that is already generating (HTTP 409).
    Running,
    /// Already finished (completed/failed) — nothing to cancel.
    Finished,
    /// Unknown or expired job.
    Gone,
}

/// Transport/protocol errors. Never contains request bodies.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    #[error("engine not reachable")]
    Connect,
    #[error("engine request timed out")]
    Timeout,
    #[error("engine queue is full")]
    QueueFull,
    #[error("job not found")]
    NotFound,
    #[error("engine answered HTTP {code}: {error}")]
    Status { code: u16, error: String },
    #[error("unexpected engine response: {0}")]
    Decode(String),
    #[error("{0}")]
    Net(String),
}

// ---------------------------------------------------------------- client

#[derive(Clone)]
enum Http {
    Local(pinhole_net::LocalClient),
    #[cfg(any(test, feature = "test-util"))]
    Plain(reqwest::Client),
}

/// Environment variable the patched sd-server reads its per-launch API key from
/// (`--api-key`, env form so the key doesn't show up in the process list).
/// Upstream sd-server at the current pin ignores it (and the bearer header).
pub const API_KEY_ENV: &str = "SD_API_KEY";

/// Patched sd-server flag: refuse every request that carries an `Origin` header
/// (web pages) and send no CORS headers. Upstream sd-server at the current pin
/// doesn't know it and would refuse to start.
pub const REJECT_ORIGIN_FLAG: &str = "--reject-origin";

/// Client for one sd-server instance. `Debug` leaves out the API key.
#[derive(Clone)]
pub struct SdClient {
    http: Http,
    base: String,
    api_key: Option<String>,
}

impl std::fmt::Debug for SdClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SdClient")
            .field("base", &self.base)
            .finish()
    }
}

const T_SHORT: Duration = Duration::from_secs(5);
const T_SUBMIT: Duration = Duration::from_secs(120);
const T_POLL: Duration = Duration::from_secs(60);
const T_UPSCALE: Duration = Duration::from_secs(30 * 60);

impl SdClient {
    /// `base` = `http://127.0.0.1:<port>` (LocalClient refuses anything else).
    pub fn new(local: pinhole_net::LocalClient, base: impl Into<String>) -> Self {
        Self {
            http: Http::Local(local),
            base: base.into().trim_end_matches('/').to_string(),
            api_key: None,
        }
    }

    /// Send `Authorization: Bearer <key>` with every request.
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into()).filter(|k| !k.is_empty());
        self
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.api_key {
            Some(k) => rb.bearer_auth(k),
            None => rb,
        }
    }

    /// Test-only client without the net crate (still loopback-only, no proxy).
    #[cfg(any(test, feature = "test-util"))]
    pub fn new_plain_for_tests(base: impl Into<String>) -> Self {
        let base = base.into().trim_end_matches('/').to_string();
        assert!(
            base.starts_with("http://127.0.0.1:"),
            "test client is loopback-only"
        );
        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("reqwest client");
        Self {
            http: Http::Plain(client),
            base,
            api_key: None,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    fn get(&self, path: &str) -> Result<reqwest::RequestBuilder, ApiError> {
        let rb = match &self.http {
            Http::Local(c) => c
                .get(&self.base, path)
                .map_err(|e| ApiError::Net(e.to_string()))?,
            #[cfg(any(test, feature = "test-util"))]
            Http::Plain(c) => c.get(format!("{}{}", self.base, path)),
        };
        Ok(self.auth(rb))
    }

    fn post<B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<reqwest::RequestBuilder, ApiError> {
        let rb = match &self.http {
            Http::Local(c) => c
                .post_json(&self.base, path, body)
                .map_err(|e| ApiError::Net(e.to_string()))?,
            #[cfg(any(test, feature = "test-util"))]
            Http::Plain(c) => c.post(format!("{}{}", self.base, path)).json(body),
        };
        Ok(self.auth(rb))
    }

    /// `true` once `GET /sdcpp/v1/capabilities` answers 200 (model loaded).
    pub async fn is_ready(&self) -> bool {
        let Ok(rb) = self.get("/sdcpp/v1/capabilities") else {
            return false;
        };
        matches!(rb.timeout(T_SHORT).send().await, Ok(r) if r.status().is_success())
    }

    pub async fn capabilities(&self) -> Result<Capabilities, ApiError> {
        let resp = send(
            self.get("/sdcpp/v1/capabilities")?
                .timeout(Duration::from_secs(15)),
        )
        .await?;
        decode(resp).await
    }

    /// Submit an image job; returns the job id.
    pub async fn submit(&self, req: &ImgGenRequest) -> Result<String, ApiError> {
        debug_assert!(!req.embeds_metadata());
        let resp = send(self.post("/sdcpp/v1/img_gen", req)?.timeout(T_SUBMIT)).await?;
        #[derive(Deserialize)]
        struct Submitted {
            id: String,
        }
        let s: Submitted = decode(resp).await?;
        if s.id.is_empty()
            || !s
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(ApiError::Decode("bad job id".into()));
        }
        Ok(s.id)
    }

    pub async fn job(&self, id: &str) -> Result<Job, ApiError> {
        let resp = send(self.get(&format!("/sdcpp/v1/jobs/{id}"))?.timeout(T_POLL)).await?;
        decode(resp).await
    }

    pub async fn cancel(&self, id: &str) -> Result<CancelOutcome, ApiError> {
        let rb = self.post(
            &format!("/sdcpp/v1/jobs/{id}/cancel"),
            &serde_json::json!({}),
        )?;
        let resp = match rb.timeout(Duration::from_secs(10)).send().await {
            Ok(r) => r,
            Err(e) => return Err(map_reqwest(e)),
        };
        match resp.status().as_u16() {
            409 => Ok(CancelOutcome::Running),
            404 | 410 => Ok(CancelOutcome::Gone),
            s if (200..300).contains(&s) => {
                let job: Job = decode(resp).await?;
                Ok(if job.status == JobStatus::Cancelled {
                    CancelOutcome::Cancelled
                } else {
                    CancelOutcome::Finished
                })
            }
            code => Err(ApiError::Status {
                code,
                error: error_field(resp).await,
            }),
        }
    }

    /// ESRGAN upscale (synchronous on the server; can take minutes on CPU).
    pub async fn upscale(&self, req: &UpscaleRequest) -> Result<UpscaleResponse, ApiError> {
        let resp = send(self.post("/sdcpp/v1/upscale", req)?.timeout(T_UPSCALE)).await?;
        decode(resp).await
    }
}

fn map_reqwest(e: reqwest::Error) -> ApiError {
    if e.is_timeout() {
        ApiError::Timeout
    } else if e.is_connect() || e.is_request() {
        ApiError::Connect
    } else {
        ApiError::Net(e.without_url().to_string())
    }
}

async fn send(rb: reqwest::RequestBuilder) -> Result<reqwest::Response, ApiError> {
    let resp = rb.send().await.map_err(map_reqwest)?;
    let status = resp.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(resp);
    }
    Err(match status {
        404 | 410 => ApiError::NotFound,
        429 => ApiError::QueueFull,
        code => ApiError::Status {
            code,
            error: error_field(resp).await,
        },
    })
}

/// The server's `{"error": "..."}` string only (never `message`, which can echo
/// parser input), capped.
async fn error_field(resp: reqwest::Response) -> String {
    let bytes = resp.bytes().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    v.get("error")
        .and_then(|e| e.as_str())
        .unwrap_or("")
        .chars()
        .take(200)
        .collect()
}

async fn decode<T: serde::de::DeserializeOwned>(resp: reqwest::Response) -> Result<T, ApiError> {
    let bytes = resp.bytes().await.map_err(map_reqwest)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| ApiError::Decode(e.to_string().chars().take(200).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn img_gen_body_never_embeds_metadata() {
        let mut req = ImgGenRequest::new(
            crate::words::CheckedPrompt::check("a cat").unwrap(),
            512,
            768,
            42,
        );
        req.negative_prompt = "blurry".into();
        req.lora.push(LoraRef {
            path: "styles/film.safetensors".into(),
            multiplier: 0.8,
        });
        req.sample_params = SampleParams {
            sample_method: Some("euler".into()),
            scheduler: Some("simple".into()),
            sample_steps: 8,
            flow_shift: None,
            guidance: Guidance {
                txt_cfg: 1.0,
                img_cfg: None,
                distilled_guidance: Some(3.5),
            },
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["embed_image_metadata"], serde_json::Value::Bool(false));
        assert_eq!(v["output_format"], "png");
        assert_eq!(v["width"], 512);
        assert_eq!(v["height"], 768);
        assert_eq!(v["seed"], 42);
        assert_eq!(v["clip_skip"], -1);
        assert_eq!(v["lora"][0]["path"], "styles/film.safetensors");
        assert!((v["lora"][0]["multiplier"].as_f64().unwrap() - 0.8).abs() < 1e-6);
        assert_eq!(v["sample_params"]["sample_method"], "euler");
        assert_eq!(v["sample_params"]["guidance"]["distilled_guidance"], 3.5);
        assert!(
            v.get("init_image").is_none()
                && v.get("ref_images").is_none()
                && v.get("hires").is_none()
        );
        assert!(!req.embeds_metadata());
    }

    /// Regression: below CFG 1 the engine follows the negative prompt, which isn't
    /// word-checked, so the body never carries less than 1, whatever the caller set.
    #[test]
    fn cfg_below_one_is_sent_as_one() {
        for cfg in [0.0, 0.5, -3.0, f32::NAN, f32::NEG_INFINITY, f32::INFINITY] {
            let mut req = ImgGenRequest::new(
                crate::words::CheckedPrompt::check("a cat").unwrap(),
                512,
                512,
                1,
            );
            req.sample_params.guidance.txt_cfg = cfg;
            req.sample_params.guidance.img_cfg = Some(cfg);
            let v = serde_json::to_value(&req).unwrap();
            assert_eq!(v["sample_params"]["guidance"]["txt_cfg"], 1.0, "{cfg}");
            assert_eq!(v["sample_params"]["guidance"]["img_cfg"], 1.0, "{cfg}");
        }
        let mut req = ImgGenRequest::new(
            crate::words::CheckedPrompt::check("a cat").unwrap(),
            512,
            512,
            1,
        );
        req.sample_params.guidance.txt_cfg = 6.5;
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["sample_params"]["guidance"]["txt_cfg"], 6.5);
    }

    /// Regression: with no `upscaler`, sd-server upscales in latent space, which
    /// garbled copied CivitAI settings (hires ×2 at strength 0.35).
    #[test]
    fn hires_uses_an_image_space_upscaler() {
        let mut req = ImgGenRequest::new(
            crate::words::CheckedPrompt::check("a cat").unwrap(),
            1024,
            1024,
            1,
        );
        req.hires = Some(HiresRequest::image_space(2.0, 0, 0.35));
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["hires"]["enabled"], true);
        assert_eq!(v["hires"]["upscaler"], "Lanczos");
        assert_eq!(v["hires"]["scale"], 2.0);
        assert_eq!(v["hires"]["steps"], 0);
        assert!((v["hires"]["denoising_strength"].as_f64().unwrap() - 0.35).abs() < 1e-6);
    }

    #[test]
    fn debug_never_prints_prompt() {
        let mut req = ImgGenRequest::new(
            crate::words::CheckedPrompt::check("PINHOLE_SENTINEL_7f3a").unwrap(),
            64,
            64,
            1,
        );
        req.negative_prompt = "NEG_SENTINEL".into();
        req.init_image = Some("aGVsbG8=".into());
        let dbg = format!("{req:?}");
        assert!(
            !dbg.contains("SENTINEL") && !dbg.contains("aGVsbG8="),
            "{dbg}"
        );
        let up = UpscaleRequest::new("aGVsbG8=".into(), None, 9);
        assert_eq!(up.repeats, 4);
        assert!(!format!("{up:?}").contains("aGVsbG8="));
    }

    #[test]
    fn parses_job_shapes_from_api_md() {
        let done: Job = serde_json::from_str(r#"{"id":"job_1","kind":"img_gen","status":"completed","created":1,"started":2,"completed":3,"queue_position":0,"result":{"output_format":"png","images":[{"index":0,"b64_json":"iVBORw0KGgo="}]},"error":null}"#).unwrap();
        assert_eq!(done.status, JobStatus::Completed);
        assert_eq!(done.result.unwrap().images.len(), 1);
        let failed: Job = serde_json::from_str(r#"{"id":"job_1","kind":"img_gen","status":"failed","created":1,"started":null,"completed":3,"queue_position":0,"result":null,"error":{"code":"generation_failed","message":"generate_image returned empty results"}}"#).unwrap();
        assert_eq!(failed.status, JobStatus::Failed);
        assert_eq!(failed.error.unwrap().code, "generation_failed");
        let queued: Job =
            serde_json::from_str(r#"{"id":"j","status":"queued","queue_position":2}"#).unwrap();
        assert_eq!(
            (queued.status, queued.queue_position),
            (JobStatus::Queued, 2)
        );
        let odd: Job = serde_json::from_str(r#"{"id":"j","status":"paused"}"#).unwrap();
        assert_eq!(odd.status, JobStatus::Unknown);
    }
}
