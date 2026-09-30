//! Engine-specific glue for `tests/tests/engine_smoke.rs`: install the pinned CPU
//! sd-server, fetch the smoke model, launch, generate, stop.
//!
//! ADAPT: every call into `pinhole-engine` / `pinhole-net` used by the smoke test
//! lives in this file, so an API change there only needs edits here.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use pinhole_engine::sdapi::JobStatus;
use pinhole_engine::{EngineConfig, EngineKind, EngineProcess, ImgGenRequest, LogBuffer, SdClient};
use pinhole_net::download::{self, DownloadSpec};
use pinhole_net::{HttpClient, LocalClient, OfflineFlag};
use tokio_util::sync::CancellationToken;

/// Known-good fallback when `config/models.yaml` has no `test_models.sd15` yet.
/// Verified on a GitHub runner (HF paths-info, 2026-09-28).
pub const FALLBACK_MODEL_URL: &str =
    "https://huggingface.co/Comfy-Org/stable-diffusion-v1-5-archive/resolve/main/v1-5-pruned-emaonly-fp16.safetensors";
pub const FALLBACK_MODEL_SHA256: &str =
    "e9476a13728cd75d8279f6ec8bad753a66a1957ca375a1464dc63b37db6e3916";
pub const FALLBACK_MODEL_BYTES: u64 = 2_132_696_762;

#[derive(Debug, Clone)]
pub struct SmokeModel {
    pub file: String,
    pub url: String,
    pub sha256: Option<String>,
    pub size_mb: Option<u64>,
}

/// `test_models.sd15` from models.yaml (read directly, independent of the registry crate).
pub fn smoke_model(config_dir: &Path) -> SmokeModel {
    let fallback = SmokeModel {
        file: "v1-5-pruned-emaonly-fp16.safetensors".into(),
        url: FALLBACK_MODEL_URL.into(),
        sha256: Some(FALLBACK_MODEL_SHA256.into()),
        size_mb: Some(FALLBACK_MODEL_BYTES / 1_048_576),
    };
    let Ok(text) = std::fs::read_to_string(config_dir.join("models.yaml")) else {
        return fallback;
    };
    let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
        return fallback;
    };
    let spec = &yaml["test_models"]["sd15"];
    let Some(url) = spec["url"].as_str().filter(|u| u.starts_with("https://")) else {
        return fallback;
    };
    let sha = spec["sha256"]
        .as_str()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()));
    let file = spec["file"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| {
            url.rsplit('/')
                .next()
                .unwrap_or("model.safetensors")
                .to_string()
        });
    SmokeModel {
        file,
        url: url.to_string(),
        sha256: sha,
        size_mb: spec["size_mb"].as_u64(),
    }
}

/// The production, allow-listed client (online).
pub fn http_client() -> HttpClient {
    HttpClient::new(OfflineFlag::new(false)).expect("HttpClient::new")
}

pub fn engine_config(config_dir: &Path) -> EngineConfig {
    EngineConfig::load(&config_dir.join("engine.yaml")).expect("config/engine.yaml parses")
}

/// Download + verify + unpack the pinned CPU sd-server for this OS into `<cache>/engine`
/// (skipped when already installed). Returns the executable path.
pub async fn install_cpu_engine(
    http: &HttpClient,
    cfg: &EngineConfig,
    cache: &Path,
) -> Result<PathBuf, String> {
    let root = cache.join("engine");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let cancel = CancellationToken::new();
    // ADAPT: pinhole_engine::install::install_direct(client, engine_root, cfg, kind, os, backend, cancel)
    let installed = pinhole_engine::install::install_direct(
        http,
        &root,
        cfg,
        EngineKind::Sd,
        pinhole_engine::pins::current_os(),
        "cpu",
        &cancel,
    )
    .await
    .map_err(|e| format!("engine install failed: {e}"))?;
    Ok(installed.exe)
}

/// Download the smoke model into `<cache>/models/` unless a verified copy exists.
pub async fn ensure_model(
    http: &HttpClient,
    cache: &Path,
    model: &SmokeModel,
) -> Result<PathBuf, String> {
    let dir = cache.join("models");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join(&model.file);
    if dest.is_file() {
        let path = dest.clone();
        let actual = tokio::task::spawn_blocking(move || download::sha256_file(&path))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        match &model.sha256 {
            Some(want) if want.eq_ignore_ascii_case(&actual) => return Ok(dest),
            None => return Ok(dest),
            Some(_) => {
                eprintln!("smoke: cached model hash mismatch, downloading again");
                let _ = std::fs::remove_file(&dest);
            }
        }
    }
    let spec = DownloadSpec {
        url: model.url.clone(),
        dest: dest.clone(),
        sha256: model.sha256.clone(),
        size_bytes: None,
        label: "smoke model".into(),
        ..Default::default()
    };
    let cancel = CancellationToken::new();
    let last = AtomicU64::new(0);
    let progress = move |done: u64, total: Option<u64>| {
        if let Some(t) = total.filter(|t| *t > 0) {
            let pct = done * 100 / t;
            if pct / 10 > last.load(Ordering::Relaxed) {
                last.store(pct / 10, Ordering::Relaxed);
                eprintln!("smoke: model download {pct}% ({} MB)", done / 1_048_576);
            }
        }
    };
    let file = download::download_file(http, &spec, &cancel, &progress)
        .await
        .map_err(|e| format!("model download failed: {e}"))?;
    if let Some(want) = &model.sha256 {
        if !want.eq_ignore_ascii_case(&file.sha256) {
            return Err(format!(
                "model SHA-256 mismatch: want {want}, got {}",
                file.sha256
            ));
        }
    }
    Ok(file.path)
}

/// A running sd-server.
pub struct RunningEngine {
    process: EngineProcess,
    client: SdClient,
    logs: Arc<LogBuffer>,
}

impl RunningEngine {
    pub fn logs(&self) -> &Arc<LogBuffer> {
        &self.logs
    }
    pub fn base_url(&self) -> String {
        self.process.base_url()
    }
}

/// Launch sd-server on 127.0.0.1:<free port> with an all-in-one SD 1.5 checkpoint and
/// wait until `/sdcpp/v1/capabilities` answers.
pub async fn launch(
    cfg: &EngineConfig,
    exe: &Path,
    model: &Path,
    timeout: Duration,
) -> Result<RunningEngine, String> {
    let port = pinhole_engine::process::free_port().map_err(|e| e.to_string())?;
    let logs = Arc::new(LogBuffer::new(400));
    let mut args: Vec<String> = cfg.stable_diffusion_cpp.launch_defaults.clone();
    if !args.iter().any(|a| a == "--listen-ip") {
        args.extend(["--listen-ip".to_string(), "127.0.0.1".to_string()]);
    }
    args.extend([
        "--listen-port".to_string(),
        port.to_string(),
        "-m".to_string(),
        model.display().to_string(),
    ]);
    // ADAPT: EngineProcess::spawn(exe, args, port, logs)
    let mut process = EngineProcess::spawn(exe, &args, port, logs.clone())
        .map_err(|e| format!("spawn {}: {e}", exe.display()))?;
    let client = SdClient::new(
        LocalClient::new().map_err(|e| e.to_string())?,
        process.base_url(),
    );
    let cancel = CancellationToken::new();
    let probe_client = client.clone();
    let started = Instant::now();
    let mut last_report = 0u64;
    let ready = process
        .wait_ready(
            move || {
                let c = probe_client.clone();
                async move { c.is_ready().await }
            },
            timeout,
            &cancel,
            |elapsed| {
                if elapsed.as_secs() / 15 > last_report {
                    last_report = elapsed.as_secs() / 15;
                    eprintln!("smoke: waiting for sd-server… {}s", elapsed.as_secs());
                }
            },
        )
        .await;
    if let Err(e) = ready {
        let tail = logs.tail_text(60);
        process.kill().await;
        return Err(format!(
            "sd-server did not become ready: {e}\n--- engine output ---\n{tail}"
        ));
    }
    eprintln!(
        "smoke: sd-server ready on {} after {:.1}s",
        process.base_url(),
        started.elapsed().as_secs_f32()
    );
    Ok(RunningEngine {
        process,
        client,
        logs,
    })
}

/// One txt2img job through the native async API. Returns PNG bytes per image.
pub async fn txt2img(
    engine: &RunningEngine,
    prompt: &str,
    negative: &str,
    size: (u32, u32),
    steps: u32,
    seed: i64,
    timeout: Duration,
) -> Result<Vec<Vec<u8>>, String> {
    // ADAPT: ImgGenRequest::new(prompt, w, h, seed) + public sample_params / negative_prompt
    let mut req = ImgGenRequest::new(
        pinhole_engine::words::CheckedPrompt::check(prompt)
            .expect("smoke prompt passes the word check"),
        size.0,
        size.1,
        seed,
    );
    req.negative_prompt = negative.to_string();
    req.sample_params.sample_steps = steps;
    if req.embeds_metadata() {
        return Err("ImgGenRequest::new must default embed_image_metadata to false".into());
    }
    engine.logs.set_secrets([prompt, negative]);
    let id = engine
        .client
        .submit(&req)
        .await
        .map_err(|e| format!("submit: {e}"))?;
    let started = Instant::now();
    loop {
        let job = engine
            .client
            .job(&id)
            .await
            .map_err(|e| format!("poll: {e}"))?;
        match job.status {
            JobStatus::Completed => {
                let images = job.result.map(|r| r.images).unwrap_or_default();
                let mut out = Vec::new();
                for img in images {
                    let b64 = img.b64_json.rsplit(',').next().unwrap_or_default();
                    out.push(
                        base64::engine::general_purpose::STANDARD
                            .decode(b64)
                            .map_err(|e| format!("base64: {e}"))?,
                    );
                }
                eprintln!(
                    "smoke: job {id} completed in {:.1}s",
                    started.elapsed().as_secs_f32()
                );
                return Ok(out);
            }
            JobStatus::Failed | JobStatus::Cancelled => {
                let err = job
                    .error
                    .map(|e| format!("{}: {}", e.code, e.message))
                    .unwrap_or_default();
                return Err(format!(
                    "job {:?}: {err}\n--- engine output ---\n{}",
                    job.status,
                    engine.logs.tail_text(60)
                ));
            }
            _ => {}
        }
        if started.elapsed() > timeout {
            let _ = engine.client.cancel(&id).await;
            return Err(format!(
                "job timed out after {}s\n--- engine output ---\n{}",
                timeout.as_secs(),
                engine.logs.tail_text(60)
            ));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

pub async fn stop(engine: RunningEngine) {
    engine.process.stop().await;
}
