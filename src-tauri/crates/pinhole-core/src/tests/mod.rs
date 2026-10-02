//! Core-level round trips against `MockSdServer` / `MockLlamaServer`
//! (no real engine is spawned), grouped by area.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use pinhole_engine::testutil::{MockLlamaServer, MockOptions, MockSdServer};
use pinhole_registry::wiring::{GenMode, Quality, Shape};
use pinhole_store::datadir::ModelKind;
use pinhole_store::{DataDir, InstalledFile};

use crate::events::{CoreEvent, EventSink, GenPhase};
use crate::generate::{self, GenerateRequest};
use crate::testing::*;
use crate::{describe, session, AppCore, ShippedPaths};

const SENTINEL: &str = "PINHOLE_SENTINEL_7f3a";

#[derive(Default)]
struct Recorder(Mutex<Vec<CoreEvent>>);
impl EventSink for Recorder {
    fn emit(&self, event: CoreEvent) {
        self.0.lock().push(event);
    }
}

fn config_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../config")
}

fn new_core() -> (tempfile::TempDir, Arc<AppCore>, Arc<Recorder>) {
    let tmp = tempfile::tempdir().unwrap();
    let rec = Arc::new(Recorder::default());
    let core = AppCore::new(
        ShippedPaths {
            config_dir: config_dir(),
        },
        DataDir::at(tmp.path().join("Data"), false),
        rec.clone(),
    )
    .expect("AppCore::new");
    // No check model files in tests: a stand-in that passes everything.
    use_check(&core, FakeCheck::default());
    (tmp, core, rec)
}

fn scan_for(dir: &std::path::Path, needle: &[u8]) -> Vec<PathBuf> {
    let mut hits = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if std::fs::read(&p)
                .map(|b| b.windows(needle.len()).any(|w| w == needle))
                .unwrap_or(false)
            {
                hits.push(p);
            }
        }
    }
    hits
}

fn has(bytes: &[u8], needle: &str) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
}

/// The field report: Z-Image Turbo on a 16 GB NVIDIA card with ~9 GB taken by
/// another program. The engine answers "generate_image returned no results"
/// and its output says the text encoder ran out of memory.
const TE_OOM: &str = "generate_image returned no results\n\
    [WARN] model_manager.cpp:1919 - model manager cannot make enough memory available on CUDA0: need 518.58 MB device / 6.58 MB budget, available 0.00 MB device / 7044.91 MB budget\n\
    [ERROR] ggml_runner.cpp:899 - qwen3 segment 1/1 (graph) failed during workspace capacity check\n\
    [ERROR] conditioner.hpp:2224 - LLM prompt encoding failed\n\
    [ERROR] image.cpp:448 - failed to encode prompt";

/// A core that plans for a 16 GB CUDA card (no real detection in tests).
fn gpu_core() -> (tempfile::TempDir, Arc<AppCore>, Arc<Recorder>) {
    let (tmp, core, rec) = new_core();
    {
        let mut s = core.settings.write();
        s.engine_backend = "cuda".into();
        s.vram_override_gb = Some(16.0);
    }
    (tmp, core, rec)
}

fn te_on_cpu(args: &[String]) -> bool {
    args.windows(2)
        .any(|w| w[0] == "--backend" && w[1].split(',').any(|p| p == "te=cpu"))
}

/// The `--max-vram` budget a launch asked for.
fn max_vram(args: &[String]) -> Option<&str> {
    args.windows(2)
        .rfind(|w| w[0] == "--max-vram")
        .map(|w| w[1].as_str())
}

/// Pretend the upscaler component is installed.
fn install_fake_upscaler(core: &Arc<AppCore>) {
    let (rel, size) = write_dummy(core, ModelKind::Upscaler, "RealESRGAN_x4plus.pth");
    core.installed.lock().upsert(InstalledFile {
        id: "up".into(),
        rel_path: rel,
        kind: ModelKind::Upscaler,
        sha256: "0".repeat(64),
        size_bytes: size,
        family: None,
        component_id: Some(generate::UPSCALER_COMPONENT.into()),
        friendly_name: "ESRGAN".into(),
        civitai: None,
        added_at: 0,
        last_used: None,
        observed_vram_gb: None,
        dtype: None,
        trigger_words: None,
        lookup: None,
    });
}

/// Readings of an intimate photo of an adult (made-up scores).
fn intimate_adult() -> pinhole_check::Readings {
    pinhole_check::Readings {
        nudity: 0.95,
        tags: Some(pinhole_check::Tags {
            questionable: 0.6,
            explicit: 0.3,
            realistic: 0.8,
            nude: 0.9,
            ..Default::default()
        }),
        faces: Some(vec![pinhole_check::Face {
            score: 0.9,
            side: 100.0,
            child_face: Some(0.02),
            under_20_face: Some(0.05),
            age: Some(35.0),
        }]),
    }
}

/// A long-running child standing in for a Pinhole-started engine.
#[cfg(unix)]
fn fake_engine(dir: &std::path::Path, body: &str) -> pinhole_engine::EngineProcess {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join(format!("fake-engine-{}.sh", uuid::Uuid::new_v4()));
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    pinhole_engine::EngineProcess::spawn(&p, &[], 1, Arc::new(pinhole_engine::LogBuffer::default()))
        .unwrap()
}

#[cfg(unix)]
async fn put_engine(core: &AppCore, proc: pinhole_engine::EngineProcess, results_cached: bool) {
    *core.gen.slot.lock().await = crate::engine::EngineSlot {
        proc: Some(proc),
        args: vec![],
        model_id: Some("m".into()),
        results_cached,
        api_key: None,
    };
}

#[cfg(unix)]
async fn engine_running(core: &AppCore) -> bool {
    core.gen.slot.lock().await.proc.is_some()
}

fn last_generation_event(rec: &Recorder) -> Option<crate::events::GenerationProgress> {
    rec.0.lock().iter().rev().find_map(|e| {
        if let CoreEvent::Generation(p) = e {
            Some(p.clone())
        } else {
            None
        }
    })
}

#[cfg(unix)]
fn install_component(core: &AppCore, kind: ModelKind, file: &str, component_id: &str) {
    let (rel, size) = write_dummy(core, kind, file);
    let mut idx = core.installed.lock();
    idx.upsert(InstalledFile {
        id: uuid::Uuid::new_v4().to_string(),
        rel_path: rel,
        kind,
        sha256: "0".repeat(64),
        size_bytes: size,
        family: None,
        component_id: Some(component_id.into()),
        friendly_name: component_id.into(),
        civitai: None,
        added_at: 0,
        last_used: None,
        observed_vram_gb: None,
        dtype: None,
        trigger_words: None,
        lookup: None,
    });
}

/// Install a shell script as the CPU build of `kind` (it never answers HTTP).
#[cfg(unix)]
fn install_fake_engine(
    core: &AppCore,
    kind: pinhole_engine::install::EngineKind,
    binary: &str,
    script: &str,
) {
    use pinhole_engine::install::{self, InstallMarker};
    use std::os::unix::fs::PermissionsExt;
    core.settings.write().engine_backend = "cpu".into();
    let (cfg, sel) = crate::engine_setup::selected_build(core, kind).unwrap();
    let version = kind.pin(&cfg).version.clone();
    let dir = install::install_dir(&core.data.engine(), kind, &version, &sel.backend);
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join(binary);
    std::fs::write(&exe, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let marker = InstallMarker {
        engine: kind,
        version,
        backend: sel.backend.clone(),
        build: sel.key.clone(),
        binary: binary.into(),
        archives: vec![],
        installed_at: 0,
    };
    std::fs::write(
        dir.join(install::MARKER_FILE),
        serde_json::to_string(&marker).unwrap(),
    )
    .unwrap();
}

mod check_tests;
mod describe_tests;
mod engine_tests;
mod generate_tests;
mod memory_tests;
mod session_tests;
mod upscale_tests;
