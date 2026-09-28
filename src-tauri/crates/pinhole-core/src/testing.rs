//! Test hooks for workspace integration tests (`--features test-util`).
//! OWNER: engine agent (generate hooks) — keep signatures stable, the `tests/`
//! crate (ci agent) depends on them.
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
pub fn use_external_engine(core: &AppCore, base_url: &str) {
    *core.gen.external.lock() = Some(base_url.trim_end_matches('/').to_string());
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
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn write_dummy(core: &AppCore, kind: ModelKind, file_name: &str) -> (String, u64) {
    let dir = core.data.models(kind);
    std::fs::create_dir_all(&dir).expect("create model dir");
    let path = dir.join(file_name);
    let bytes = b"pinhole-test-dummy";
    std::fs::write(&path, bytes).expect("write dummy model");
    let rel = core.data.relative(&path).unwrap_or_else(|| rel_fallback(&core.data.root, &path));
    (rel, bytes.len() as u64)
}

fn rel_fallback(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

/// Register a fake installed model of `family_id` (tiny dummy files on disk,
/// components marked installed) and return its installed-model id.
pub fn register_fake_model(core: &AppCore, family_id: &str) -> String {
    let reg = core.registry();
    let family = reg.family(family_id).unwrap_or_else(|| panic!("unknown family {family_id}")).clone();
    let hw = crate::app::hw_context(core);
    let id = uuid::Uuid::new_v4().to_string();
    let main_kind = match family.layout {
        pinhole_registry::Layout::AllInOne => ModelKind::Checkpoint,
        pinhole_registry::Layout::DiffusionOnly => ModelKind::Diffusion,
    };
    let (rel, size) = write_dummy(core, main_kind, &format!("fake-{family_id}-{}.safetensors", &id[..8]));
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
    }];
    let required = pinhole_registry::wiring::required_components(&reg, &family, &hw);
    {
        let idx = core.installed.lock();
        for rc in required {
            if idx.find_component(&rc.component_id).is_some() {
                continue;
            }
            let file = reg.component(&rc.component_id).map(|c| c.file.clone()).unwrap_or_else(|| format!("{}.safetensors", rc.component_id));
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
    let (rel, size) = write_dummy(core, ModelKind::Lora, &format!("fake-lora-{}.safetensors", &id[..8]));
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
        }),
        added_at: now(),
        last_used: None,
        observed_vram_gb: None,
        dtype: None,
    };
    let mut idx = core.installed.lock();
    idx.upsert(file);
    idx.save(&core.data).expect("save installed.json");
    id
}

#[cfg(test)]
mod tests {
    //! Core-level round trips against `MockSdServer` / `MockLlamaServer`
    //! (no real engine is spawned).
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use parking_lot::Mutex;
    use pinhole_engine::testutil::{MockLlamaServer, MockOptions, MockSdServer};
    use pinhole_registry::wiring::{GenMode, Quality, Shape};
    use pinhole_store::DataDir;

    use super::*;
    use crate::events::{CoreEvent, EventSink, GenPhase};
    use crate::generate::{self, GenerateRequest};
    use crate::{describe, session, ShippedPaths};

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
        let core = AppCore::new(ShippedPaths { config_dir: config_dir() }, DataDir::at(tmp.path().join("Data"), false), rec.clone()).expect("AppCore::new");
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
                } else if std::fs::read(&p).map(|b| b.windows(needle.len()).any(|w| w == needle)).unwrap_or(false) {
                    hits.push(p);
                }
            }
        }
        hits
    }

    fn has(bytes: &[u8], needle: &str) -> bool {
        bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
    }

    #[tokio::test]
    async fn txt2img_round_trip_scrubs_and_never_writes_the_prompt() {
        let (tmp, core, rec) = new_core();
        let mock = MockSdServer::start().await;
        use_external_engine(&core, &mock.base_url());
        let model = register_fake_model(&core, "sdxl");

        let mut req = GenerateRequest::txt2img(model.clone(), format!("{SENTINEL} a lighthouse"));
        req.dials.count = 2;
        req.dials.shape = Shape::Portrait;
        req.fine_tune.seed = Some(1000);
        req.fine_tune.negative_prompt = Some("NEGSENTINEL_91".into());
        let res = generate::generate(&core, req).await.expect("generate");
        assert_eq!(res.images.len(), 2);
        assert_eq!((res.images[0].seed, res.images[1].seed), (1000, 1001));
        assert_eq!(res.images[0].model_id, model);
        assert_eq!(res.images[0].family_id, "sdxl");

        // What reached the engine.
        let reqs = mock.requests();
        assert_eq!(reqs.len(), 1);
        let body = &reqs[0];
        assert_eq!(body["embed_image_metadata"], serde_json::json!(false));
        assert!(body["prompt"].as_str().unwrap().contains(SENTINEL));
        assert_eq!(body["negative_prompt"], "NEGSENTINEL_91");
        assert_eq!(body["batch_count"], 2);
        assert_eq!(body["seed"], 1000);
        assert!(body["width"].as_u64().unwrap() < body["height"].as_u64().unwrap(), "portrait");

        // Session images are scrubbed PNGs of the right size.
        for r in &res.images {
            let bytes = session::get(&core, &r.id).unwrap();
            assert!(!has(&bytes, SENTINEL) && !has(&bytes, "NEGSENTINEL_91"));
            assert!(pinhole_engine::png::text_chunks(&bytes).is_empty());
            assert_eq!(pinhole_engine::png::dimensions(&bytes), Some((r.width, r.height)));
        }

        // Save (default: no metadata), then with "settings (no prompt)".
        let saved = session::save_image(&core, &res.images[0].id).unwrap();
        let name = std::path::Path::new(&saved.path).file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("pinhole_") && name.ends_with("_1000.png"), "{name}");
        assert!(pinhole_engine::png::text_chunks(&std::fs::read(&saved.path).unwrap()).is_empty());
        core.settings.write().saved_metadata = "settings".into();
        let saved2 = session::save_image(&core, &res.images[0].id).unwrap();
        assert_ne!(saved.path, saved2.path, "unique name on collision");
        let chunks = pinhole_engine::png::text_chunks(&std::fs::read(&saved2.path).unwrap());
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].1.starts_with(b"pinhole\0"));
        assert!(has(&chunks[0].1, "\"seed\":1000"));

        // Nothing under Data/ contains the prompt; last_used was updated (a number).
        assert!(scan_for(&tmp.path().join("Data"), SENTINEL.as_bytes()).is_empty());
        assert!(scan_for(&tmp.path().join("Data"), b"NEGSENTINEL_91").is_empty());
        assert!(core.installed.lock().get(&model).unwrap().last_used.is_some());

        // Progress events: queued → generating → done, none carries prompt text.
        let events = rec.0.lock().clone();
        let phases: Vec<GenPhase> = events.iter().filter_map(|e| if let CoreEvent::Generation(p) = e { Some(p.phase) } else { None }).collect();
        assert!(phases.contains(&GenPhase::Queued) && phases.contains(&GenPhase::Generating));
        assert_eq!(phases.last(), Some(&GenPhase::Done));
        let all = serde_json::to_string(&events).unwrap();
        assert!(!all.contains(SENTINEL));

        // Clear session drops everything.
        session::clear(&core);
        assert!(session::get(&core, &res.images[1].id).is_err());
    }

    #[tokio::test]
    async fn style_loras_trigger_words_and_preview() {
        let (_tmp, core, _rec) = new_core();
        let mock = MockSdServer::start().await;
        use_external_engine(&core, &mock.base_url());
        let model = register_fake_model(&core, "sdxl");
        let lora = register_fake_lora(&core, "sdxl", &["zxc_trigger"]);
        let style = crate::library::save_style(
            &core,
            pinhole_store::styles::Style { id: String::new(), name: "Test film".into(), positive: "grainy 35mm film".into(), negative: Some("cartoon".into()), families: vec![], thumbnail: None, builtin: false },
        )
        .unwrap();

        let mut req = GenerateRequest::txt2img(model, "a red boat");
        req.style_id = Some(style.id.clone());
        req.loras = vec![generate::LoraUse { lora_id: lora, weight: 0.7 }];
        req.add_trigger_words = true;
        let preview = generate::preview_final_prompt(&core, &req).unwrap();
        assert!(preview.prompt.contains("a red boat") && preview.prompt.contains("zxc_trigger") && preview.prompt.contains("grainy 35mm film"), "{}", preview.prompt);
        assert!(preview.negative.as_deref().unwrap_or("").contains("cartoon"));

        generate::generate(&core, req).await.unwrap();
        let body = &mock.requests()[0];
        assert!(body["lora"][0]["path"].as_str().unwrap().starts_with("fake-lora-"));
        assert!((body["lora"][0]["multiplier"].as_f64().unwrap() - 0.7).abs() < 1e-6);
        assert!(body["prompt"].as_str().unwrap().contains("zxc_trigger"));
        assert!(!body["prompt"].as_str().unwrap().contains("<lora:"));
    }

    #[tokio::test]
    async fn restyle_and_instruction_edit_send_images() {
        let (_tmp, core, _rec) = new_core();
        let mock = MockSdServer::start().await;
        use_external_engine(&core, &mock.base_url());
        let model = register_fake_model(&core, "sdxl");
        let src = pinhole_engine::testutil::solid_png(300, 200, [10, 20, 30, 255]);
        let imported = session::import_image(&core, src).unwrap();
        assert_eq!((imported.width, imported.height), (300, 200));

        let mut req = GenerateRequest::txt2img(model, "make it autumn");
        req.mode = GenMode::Img2img;
        req.init_image_id = Some(imported.id.clone());
        req.strength = Some(0.35);
        let res = generate::generate(&core, req).await.unwrap();
        assert_eq!(res.images[0].parent_id.as_deref(), Some(imported.id.as_str()));
        let body = &mock.requests()[0];
        assert!(body["init_image"].as_str().unwrap().len() > 100);
        assert!((body["strength"].as_f64().unwrap() - 0.35).abs() < 1e-6);
        let (w, h) = (body["width"].as_u64().unwrap(), body["height"].as_u64().unwrap());
        assert!(w > h && w % 16 == 0 && h % 16 == 0, "keeps the landscape aspect: {w}x{h}");

        // Instruction edit picks the installed edit model even if a Create model is selected.
        let _edit = register_fake_model(&core, "qwen_image_edit_2511");
        let mut req = GenerateRequest::txt2img(res.images[0].model_id.clone(), "replace the sky with stars");
        req.mode = GenMode::Edit;
        req.ref_image_ids = vec![res.images[0].id.clone()];
        req.dials.quality = Quality::Fast;
        let edited = generate::generate(&core, req).await.unwrap();
        assert_eq!(edited.images[0].family_id, "qwen_image_edit_2511");
        assert_eq!(edited.images[0].parent_id.as_deref(), Some(res.images[0].id.as_str()));
        let body = &mock.requests()[1];
        assert_eq!(body["ref_images"].as_array().unwrap().len(), 1);
        assert!(body.get("init_image").is_none());

        // With an "Only change here" mask the source also goes in as init_image.
        let mask = session::import_image(&core, pinhole_engine::testutil::solid_png(300, 200, [255, 255, 255, 255])).unwrap();
        let mut req = GenerateRequest::txt2img(edited.images[0].model_id.clone(), "make the sign blue");
        req.mode = GenMode::Edit;
        req.ref_image_ids = vec![res.images[0].id.clone()];
        req.mask_image_id = Some(mask.id);
        generate::generate(&core, req).await.unwrap();
        let body = &mock.requests()[2];
        assert!(body["mask_image"].as_str().unwrap().len() > 50);
        assert_eq!(body["init_image"], body["ref_images"][0]);
        assert_eq!(body["strength"], 1.0);
    }

    #[tokio::test]
    async fn cancel_and_failures_map_to_plain_errors() {
        let (_tmp, core, rec) = new_core();
        let mock = MockSdServer::start_with(MockOptions { polls_before_done: 10_000, ..Default::default() }).await;
        use_external_engine(&core, &mock.base_url());
        let model = register_fake_model(&core, "sdxl");
        let c2 = core.clone();
        let task = tokio::spawn(async move { generate::generate(&c2, GenerateRequest::txt2img(model, "x y z")).await });
        for _ in 0..100 {
            if !mock.requests().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        generate::cancel(&core);
        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.code, "cancelled");
        assert_eq!(mock.cancels().len(), 1);
        let last = rec.0.lock().iter().rev().find_map(|e| if let CoreEvent::Generation(p) = e { Some(p.phase) } else { None });
        assert_eq!(last, Some(GenPhase::Cancelled));

        let (_tmp2, core2, _) = new_core();
        let failing = MockSdServer::start_with(MockOptions { polls_before_done: 0, fail_with: Some("CUDA error: out of memory".into()), ..Default::default() }).await;
        use_external_engine(&core2, &failing.base_url());
        let m2 = register_fake_model(&core2, "sdxl");
        let err = generate::generate(&core2, GenerateRequest::txt2img(m2.clone(), "x")).await.unwrap_err();
        assert_eq!(err.code, "vram");
        assert_eq!(err.message, "Not enough VRAM — try the Fast setting or the smaller version of this model");

        // Missing components → plain "download them" message, before any engine work.
        let (_tmp3, core3, _) = new_core();
        use_external_engine(&core3, &failing.base_url());
        let m3 = register_fake_model(&core3, "sdxl");
        {
            let mut idx = core3.installed.lock();
            let comp: Vec<String> = idx.files.iter().filter(|f| f.component_id.is_some()).map(|f| f.id.clone()).collect();
            for id in comp {
                idx.remove(&id);
            }
        }
        let err = generate::generate(&core3, GenerateRequest::txt2img(m3, "x")).await.unwrap_err();
        assert_eq!(err.code, "not_found");
        assert!(err.message.contains("Get"), "{}", err.message);
        let err = generate::generate(&core3, GenerateRequest::txt2img("nope", "x")).await.unwrap_err();
        assert_eq!(err.code, "not_found");
        let err = generate::generate(&core3, GenerateRequest::txt2img(m2, "   ")).await.unwrap_err();
        assert_eq!(err.code, "invalid");
    }

    #[tokio::test]
    async fn upscale_uses_installed_esrgan() {
        let (_tmp, core, _rec) = new_core();
        let mock = MockSdServer::start().await;
        use_external_engine(&core, &mock.base_url());
        let model = register_fake_model(&core, "sdxl");
        // Pretend the upscaler component is installed.
        {
            let (rel, size) = write_dummy(&core, ModelKind::Upscaler, "RealESRGAN_x4plus.pth");
            let mut idx = core.installed.lock();
            idx.upsert(InstalledFile {
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
            });
        }
        let mut req = GenerateRequest::txt2img(model, "a cat");
        req.fine_tune.width = Some(64);
        req.fine_tune.height = Some(48);
        let res = generate::generate(&core, req).await.unwrap();
        let src = &res.images[0];
        let up4 = generate::upscale_image(&core, &src.id, 4).await.unwrap();
        assert_eq!((up4.width, up4.height), (src.width * 4, src.height * 4));
        assert_eq!(up4.parent_id.as_deref(), Some(src.id.as_str()));
        let up2 = generate::upscale_image(&core, &src.id, 2).await.unwrap();
        assert_eq!((up2.width, up2.height), (src.width * 2, src.height * 2));
        assert_eq!(mock.upscale_requests()[0]["upscaler"], "RealESRGAN_x4plus");
        assert!(generate::upscale_image(&core, &src.id, 3).await.is_err());
        let (rgba, w, h) = session::decode_rgba(&core, &up2.id).unwrap();
        assert_eq!(rgba.len() as u32, w * h * 4);
    }

    #[tokio::test]
    async fn describe_through_mock_llama() {
        let (_tmp, core, _rec) = new_core();
        let llama = MockLlamaServer::start("Prompt: a lighthouse at dusk", 0).await;
        use_external_captioner(&core, &llama.base_url());
        let img = session::import_image(&core, pinhole_engine::testutil::solid_png(32, 32, [1, 2, 3, 255])).unwrap();
        let text = describe::describe_image(&core, &img.id, describe::DescribeStyle::Sentence).await.unwrap();
        assert_eq!(text, "a lighthouse at dusk");
        let body = &llama.requests()[0];
        let instruction = body.pointer("/messages/0/content/1/text").and_then(|t| t.as_str()).unwrap();
        assert!(instruction.contains("Describe this image"), "registry instruction is used");
        assert!(describe::captioner_status(&core).available);
    }

    #[tokio::test]
    async fn sd_args_force_loopback_and_privacy_flags() {
        let (_tmp, core, _) = new_core();
        let cfg = crate::engine_setup::engine_config(&core).unwrap();
        let wiring: Vec<String> = ["--model", "/m.safetensors", "--listen-ip", "0.0.0.0", "--listen-port", "80", "--vae-tiling"].iter().map(|s| s.to_string()).collect();
        let args = crate::generate::full_sd_args(&core, &wiring, &cfg);
        let ips: Vec<&String> = args.iter().enumerate().filter(|(i, a)| *a == "--listen-ip" && *i + 1 < args.len()).map(|(i, _)| &args[i + 1]).collect();
        assert_eq!(ips, vec!["127.0.0.1"], "{args:?}");
        assert!(!args.iter().any(|a| a == "--listen-port" || a == "80" || a == "0.0.0.0"), "{args:?}");
        assert_eq!(args.iter().filter(|a| *a == "--disable-image-metadata").count(), 1);
        assert_eq!(args.iter().filter(|a| *a == "--log-level").count(), 1);
        assert!(args.windows(2).any(|w| w[0] == "--lora-model-dir" && w[1].ends_with("loras")));
        assert!(args.windows(2).any(|w| w[0] == "--hires-upscalers-dir" && w[1].ends_with("upscalers")));
        assert!(args.contains(&"--vae-tiling".to_string()));
    }

    /// Drives the REAL sd-server (Linux) through `generate` when
    /// `PINHOLE_SD_ARCHIVE` points at the pinned `…-bin-Linux-Ubuntu-24.04-x86_64.zip`:
    /// install from the archive, launch with a bogus model file, expect the
    /// plain-language "couldn't be loaded" error with the engine output in details.
    #[tokio::test]
    async fn real_engine_bogus_model_gives_plain_error() {
        let Ok(archive) = std::env::var("PINHOLE_SD_ARCHIVE") else { return };
        let (_tmp, core, rec) = new_core();
        let (cfg, sel) = crate::engine_setup::selected_build(&core, pinhole_engine::install::EngineKind::Sd).unwrap();
        assert_eq!(sel.key, "linux_cpu");
        let sha = pinhole_net::download::sha256_file(std::path::Path::new(&archive)).unwrap();
        let installed = pinhole_engine::install::unpack_build(
            &core.data.engine(),
            pinhole_engine::install::EngineKind::Sd,
            &cfg.stable_diffusion_cpp,
            &sel,
            &[(sel.build.archives()[0].clone(), PathBuf::from(&archive), sha)],
        )
        .unwrap();
        assert!(installed.exe.ends_with("sd-server"));
        assert!(crate::engine_setup::engine_status(&core).installed);
        let model = register_fake_model(&core, "sd15");
        let err = generate::generate(&core, GenerateRequest::txt2img(model, "a cat")).await.unwrap_err();
        assert_eq!(err.code, "engine_failed", "{err:?}");
        assert!(err.message.contains("couldn't be loaded"), "{}", err.message);
        assert!(err.details.as_deref().unwrap_or("").contains("new_sd_ctx_t failed"), "{:?}", err.details);
        let phases: Vec<GenPhase> = rec.0.lock().iter().filter_map(|e| if let CoreEvent::Generation(p) = e { Some(p.phase) } else { None }).collect();
        assert_eq!(phases.first(), Some(&GenPhase::LoadingModel));
        assert_eq!(phases.last(), Some(&GenPhase::Failed));
        let st = crate::engine_setup::engine_status(&core);
        assert!(!st.running && st.error.is_some());
    }

    /// `install_engine` through the DownloadManager against the real pinned CPU
    /// release. Network: only when `PINHOLE_NET_INSTALL=1`.
    #[tokio::test]
    async fn install_engine_real_release_if_enabled() {
        if std::env::var("PINHOLE_NET_INSTALL").ok().as_deref() != Some("1") {
            return;
        }
        let (_tmp, core, rec) = new_core();
        core.settings.write().engine_backend = "cpu".into();
        let st = crate::engine_setup::install_engine(&core).await.expect("install_engine");
        assert!(st.installed && !st.installing, "{st:?}");
        assert_eq!(st.backend.as_deref(), Some("cpu"));
        let exe = crate::engine_setup::installed_engine(&core, pinhole_engine::install::EngineKind::Sd).unwrap().exe;
        assert!(exe.is_file());
        let engine_events = rec.0.lock().iter().filter(|e| matches!(e, CoreEvent::Engine(_))).count();
        assert!(engine_events >= 2, "installing → installed events");
        assert!(core.downloads.status().iter().any(|g| g.label.starts_with("Image engine")));

        // The describe engine (llama.cpp .tar.gz with symlinks) installs and runs too.
        let llama = crate::engine_setup::install_kind(&core, pinhole_engine::install::EngineKind::Llama).await.expect("llama install");
        let out = std::process::Command::new(&llama.exe).arg("--version").current_dir(&llama.dir).output().unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(text.contains("11235"), "{text}");
    }

    #[tokio::test]
    async fn layout_follows_installed_kind_for_all_in_one_flux() {
        let (_tmp, core, _) = new_core();
        let reg = core.registry();
        let fam = reg.family("flux1_dev").unwrap().clone();
        let hw = crate::app::hw_context(&core);
        let (rel, size) = write_dummy(&core, ModelKind::Checkpoint, "flux-aio.safetensors");
        let aio = InstalledFile {
            id: "aio".into(),
            rel_path: rel,
            kind: ModelKind::Checkpoint,
            sha256: "1".repeat(64),
            size_bytes: size,
            family: Some("flux1_dev".into()),
            component_id: None,
            friendly_name: "Flux AIO".into(),
            civitai: None,
            added_at: 0,
            last_used: None,
            observed_vram_gb: None,
            dtype: None,
        };
        // No shared components installed: fine for an all-in-one checkpoint…
        let files = crate::generate::model_files(&core, &aio, &fam, &hw).unwrap();
        assert_eq!(files.layout, pinhole_registry::Layout::AllInOne);
        let args = pinhole_registry::wiring::launch_args(&reg, &files, &hw, &Default::default());
        assert!(args.iter().any(|a| a == "--model" || a == "-m"), "{args:?}");
        assert!(!args.iter().any(|a| a == "--diffusion-model"), "{args:?}");
        // …but a diffusion-only file of the same family must have them.
        let mut dif = aio.clone();
        dif.kind = ModelKind::Diffusion;
        let (rel, _) = write_dummy(&core, ModelKind::Diffusion, "flux-dit.safetensors");
        dif.rel_path = rel;
        let err = crate::generate::model_files(&core, &dif, &fam, &hw).unwrap_err();
        assert_eq!(err.code, "not_found");
    }

    #[tokio::test]
    async fn engine_status_and_captioner_status_without_engine() {
        let (_tmp, core, _rec) = new_core();
        let st = crate::engine_setup::engine_status(&core);
        assert!(!st.installed && !st.running);
        assert!(st.version.as_deref().unwrap_or("").starts_with("master-"));
        let cs = describe::captioner_status(&core);
        assert!(!cs.available);
        assert!(cs.download_bytes > 1_000_000_000, "default captioner + engine to download");
    }
}
