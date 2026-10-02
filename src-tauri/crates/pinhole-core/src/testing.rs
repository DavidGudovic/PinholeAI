//! Test hooks for workspace integration tests (`--features test-util`).
//! The workspace `tests/` crate uses these hooks, so keep their signatures stable.
//!
//! Typical privacy test:
//! ```ignore
//! let mock = pinhole_engine::testutil::MockSdServer::start().await;
//! let core = AppCore::new(shipped, DataDir::at(tmp, false), Arc::new(NullSink))?;
//! pinhole_core::testing::use_external_engine(&core, &mock.base_url());
//! let model_id = pinhole_core::testing::register_fake_model(&core, "sdxl");
//! let res = pinhole_core::generate::generate(&core, GenerateRequest::txt2img(model_id, "PINHOLE_SENTINEL_7f3a")).await?;
//! pinhole_core::session::save_image(&core, &res.images[0].id)?;
//! ```

use std::path::Path;

use pinhole_store::datadir::ModelKind;
use pinhole_store::InstalledFile;

use crate::AppCore;

/// Make `generate` talk to an already-running (mock) sd-server at `base_url`
/// instead of installing/spawning the real engine.
/// Also puts a stand-in image check that passes every picture in place (see
/// [`use_check`]), so tests don't need the check's model files.
pub fn use_external_engine(core: &AppCore, base_url: &str) {
    *core.gen.external.lock() = Some(base_url.trim_end_matches('/').to_string());
    use_check(core, FakeCheck::default());
}

pub use crate::imagecheck::FakeCheck;

/// Replace the image check with `fake`.
pub fn use_check(core: &AppCore, fake: FakeCheck) {
    crate::imagecheck::use_fake(core, fake);
}

/// sd-server's output buffer (memory only, redacted), so a mock engine can
/// "print" to it like the real one (`MockOptions::engine_log`).
pub fn engine_log(core: &AppCore) -> std::sync::Arc<pinhole_engine::LogBuffer> {
    core.gen.logs.clone()
}

/// Make `describe_image` talk to an already-running (mock) llama-server.
pub fn use_external_captioner(core: &AppCore, base_url: &str) {
    *core.describe.external.lock() = Some(base_url.trim_end_matches('/').to_string());
}

/// Component kind (`vae`, `t5xxl`, …) → `Data/models/` folder kind.
fn component_model_kind(kind: &str) -> ModelKind {
    match kind {
        "vae" => ModelKind::Vae,
        "taesd" => ModelKind::Taesd,
        "upscaler" => ModelKind::Upscaler,
        _ => ModelKind::TextEncoder,
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn write_dummy(core: &AppCore, kind: ModelKind, file_name: &str) -> (String, u64) {
    let dir = core.data.models(kind);
    std::fs::create_dir_all(&dir).expect("create model dir");
    let path = dir.join(file_name);
    let bytes = b"pinhole-test-dummy";
    std::fs::write(&path, bytes).expect("write dummy model");
    let rel = core
        .data
        .relative(&path)
        .unwrap_or_else(|| rel_fallback(&core.data.root, &path));
    (rel, bytes.len() as u64)
}

fn rel_fallback(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Register a fake installed model of `family_id` (tiny dummy files on disk,
/// components marked installed) and return its installed-model id.
pub fn register_fake_model(core: &AppCore, family_id: &str) -> String {
    let reg = core.registry();
    let family = reg
        .family(family_id)
        .unwrap_or_else(|| panic!("unknown family {family_id}"))
        .clone();
    let hw = crate::app::hw_context(core);
    let id = uuid::Uuid::new_v4().to_string();
    let main_kind = match family.layout {
        pinhole_registry::Layout::AllInOne => ModelKind::Checkpoint,
        pinhole_registry::Layout::DiffusionOnly => ModelKind::Diffusion,
    };
    let (rel, size) = write_dummy(
        core,
        main_kind,
        &format!("fake-{family_id}-{}.safetensors", &id[..8]),
    );
    let mut files = vec![InstalledFile {
        id: id.clone(),
        rel_path: rel,
        kind: main_kind,
        sha256: format!("{:0>64}", id.replace('-', "")),
        size_bytes: size,
        family: Some(family_id.to_string()),
        component_id: None,
        friendly_name: format!("Test {}", family.label),
        civitai: None,
        added_at: now(),
        last_used: None,
        observed_vram_gb: None,
        dtype: Some("f16".into()),
        trigger_words: None,
        lookup: None,
    }];
    let required = pinhole_registry::wiring::required_components(&reg, &family, &hw);
    {
        let idx = core.installed.lock();
        for rc in required {
            if idx.find_component(&rc.component_id).is_some() {
                continue;
            }
            let file = reg
                .component(&rc.component_id)
                .map(|c| c.file.clone())
                .unwrap_or_else(|| format!("{}.safetensors", rc.component_id));
            let kind = component_model_kind(&rc.kind);
            let (rel, size) = write_dummy(core, kind, &file);
            files.push(InstalledFile {
                id: uuid::Uuid::new_v4().to_string(),
                rel_path: rel,
                kind,
                sha256: format!("{:0>64}", rc.component_id.len()),
                size_bytes: size,
                family: None,
                component_id: Some(rc.component_id.clone()),
                friendly_name: format!("Test component {}", rc.component_id),
                civitai: None,
                added_at: now(),
                last_used: None,
                observed_vram_gb: None,
                dtype: None,
                trigger_words: None,
                lookup: None,
            });
        }
    }
    let mut idx = core.installed.lock();
    for f in files {
        idx.upsert(f);
    }
    idx.save(&core.data).expect("save installed.json");
    id
}

/// Register a fake LoRA (for trigger-word / structured `lora` tests).
pub fn register_fake_lora(core: &AppCore, family_id: &str, trained_words: &[&str]) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let (rel, size) = write_dummy(
        core,
        ModelKind::Lora,
        &format!("fake-lora-{}.safetensors", &id[..8]),
    );
    let file = InstalledFile {
        id: id.clone(),
        rel_path: rel,
        kind: ModelKind::Lora,
        sha256: format!("{:0>64}", id.replace('-', "")),
        size_bytes: size,
        family: Some(family_id.to_string()),
        component_id: None,
        friendly_name: "Test LoRA".into(),
        civitai: Some(pinhole_store::installed::CivitaiRef {
            model_id: 1,
            version_id: 2,
            model_name: None,
            version_name: None,
            base_model: None,
            trained_words: trained_words.iter().map(|s| s.to_string()).collect(),
            license: None,
            creator_notes: None,
            sfw_only: false,
        }),
        added_at: now(),
        last_used: None,
        observed_vram_gb: None,
        dtype: None,
        trigger_words: None,
        lookup: None,
    };
    let mut idx = core.installed.lock();
    idx.upsert(file);
    idx.save(&core.data).expect("save installed.json");
    id
}
