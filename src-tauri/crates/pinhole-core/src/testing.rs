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

fn write_dummy(core: &AppCore, kind: ModelKind, file_name: &str) -> (String, u64) {
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
        let core = AppCore::new(
            ShippedPaths {
                config_dir: config_dir(),
            },
            DataDir::at(tmp.path().join("Data"), false),
            rec.clone(),
        )
        .expect("AppCore::new");
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
        assert!(
            body["width"].as_u64().unwrap() < body["height"].as_u64().unwrap(),
            "portrait"
        );

        // Session images are scrubbed PNGs of the right size.
        for r in &res.images {
            let bytes = session::get(&core, &r.id).unwrap();
            assert!(!has(&bytes, SENTINEL) && !has(&bytes, "NEGSENTINEL_91"));
            assert!(pinhole_engine::png::text_chunks(&bytes).is_empty());
            assert_eq!(
                pinhole_engine::png::dimensions(&bytes),
                Some((r.width, r.height))
            );
        }

        // Save (default: no metadata), then with "settings (no prompt)".
        let saved = session::save_image(&core, &res.images[0].id).unwrap();
        let name = std::path::Path::new(&saved.path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(
            name.starts_with("pinhole_") && name.ends_with("_1000.png"),
            "{name}"
        );
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
        assert!(core
            .installed
            .lock()
            .get(&model)
            .unwrap()
            .last_used
            .is_some());

        // Progress events: queued → generating → done, none carries prompt text.
        let events = rec.0.lock().clone();
        let phases: Vec<GenPhase> = events
            .iter()
            .filter_map(|e| {
                if let CoreEvent::Generation(p) = e {
                    Some(p.phase)
                } else {
                    None
                }
            })
            .collect();
        assert!(phases.contains(&GenPhase::Queued) && phases.contains(&GenPhase::Generating));
        assert_eq!(phases.last(), Some(&GenPhase::Done));
        let all = serde_json::to_string(&events).unwrap();
        assert!(!all.contains(SENTINEL));

        // Reset drops everything.
        session::clear(&core).await;
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
            pinhole_store::styles::Style {
                id: String::new(),
                name: "Test film".into(),
                positive: "grainy 35mm film".into(),
                negative: Some("cartoon".into()),
                families: vec![],
                thumbnail: None,
                builtin: false,
            },
        )
        .unwrap();

        let mut req = GenerateRequest::txt2img(model, "a red boat");
        req.style_id = Some(style.id.clone());
        req.loras = vec![generate::LoraUse {
            lora_id: lora,
            weight: 0.7,
        }];
        req.add_trigger_words = true;
        let preview = generate::preview_final_prompt(&core, &req).unwrap();
        assert!(
            preview.prompt.contains("a red boat")
                && preview.prompt.contains("zxc_trigger")
                && preview.prompt.contains("grainy 35mm film"),
            "{}",
            preview.prompt
        );
        assert!(preview
            .negative
            .as_deref()
            .unwrap_or("")
            .contains("cartoon"));

        generate::generate(&core, req).await.unwrap();
        let body = &mock.requests()[0];
        assert!(body["lora"][0]["path"]
            .as_str()
            .unwrap()
            .starts_with("fake-lora-"));
        assert!((body["lora"][0]["multiplier"].as_f64().unwrap() - 0.7).abs() < 1e-6);
        assert!(body["prompt"].as_str().unwrap().contains("zxc_trigger"));
        assert!(!body["prompt"].as_str().unwrap().contains("<lora:"));
    }

    #[tokio::test]
    async fn restyle_and_instruction_edit_send_images() {
        let (_tmp, core, rec) = new_core();
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
        assert_eq!(
            res.images[0].parent_id.as_deref(),
            Some(imported.id.as_str())
        );
        let body = &mock.requests()[0];
        assert!(body["init_image"].as_str().unwrap().len() > 100);
        assert!((body["strength"].as_f64().unwrap() - 0.35).abs() < 1e-6);
        let (w, h) = (
            body["width"].as_u64().unwrap(),
            body["height"].as_u64().unwrap(),
        );
        assert!(
            w > h && w % 16 == 0 && h % 16 == 0,
            "keeps the landscape aspect: {w}x{h}"
        );

        // Instruction edit picks the installed edit model even if a Create model is selected.
        let _edit = register_fake_model(&core, "qwen_image_edit_2511");
        let mut req =
            GenerateRequest::txt2img(res.images[0].model_id.clone(), "replace the sky with stars");
        req.mode = GenMode::Edit;
        req.ref_image_ids = vec![res.images[0].id.clone()];
        req.dials.quality = Quality::Fast;
        let edited = generate::generate(&core, req).await.unwrap();
        assert_eq!(edited.images[0].family_id, "qwen_image_edit_2511");
        let last = last_generation_event(&rec).unwrap();
        assert_eq!(
            (last.phase, last.model_label.as_deref()),
            (GenPhase::Done, Some(edited.images[0].model_label.as_str())),
            "the final event names the edit model that ran"
        );
        assert_eq!(
            edited.images[0].parent_id.as_deref(),
            Some(res.images[0].id.as_str())
        );
        let body = &mock.requests()[1];
        assert_eq!(body["ref_images"].as_array().unwrap().len(), 1);
        assert!(body.get("init_image").is_none());

        // With an "Only change here" mask the source also goes in as init_image.
        let mask = session::import_image(
            &core,
            pinhole_engine::testutil::solid_png(300, 200, [255, 255, 255, 255]),
        )
        .unwrap();
        let mut req =
            GenerateRequest::txt2img(edited.images[0].model_id.clone(), "make the sign blue");
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
        let mock = MockSdServer::start_with(MockOptions {
            polls_before_done: 10_000,
            ..Default::default()
        })
        .await;
        use_external_engine(&core, &mock.base_url());
        let model = register_fake_model(&core, "sdxl");
        let c2 = core.clone();
        let task = tokio::spawn(async move {
            generate::generate(&c2, GenerateRequest::txt2img(model, "x y z")).await
        });
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
        let last = rec.0.lock().iter().rev().find_map(|e| {
            if let CoreEvent::Generation(p) = e {
                Some(p.phase)
            } else {
                None
            }
        });
        assert_eq!(last, Some(GenPhase::Cancelled));

        let (_tmp2, core2, _) = new_core();
        let failing = MockSdServer::start_with(MockOptions {
            polls_before_done: 0,
            fail_with: Some("CUDA error: out of memory".into()),
            ..Default::default()
        })
        .await;
        use_external_engine(&core2, &failing.base_url());
        let m2 = register_fake_model(&core2, "sdxl");
        let err = generate::generate(&core2, GenerateRequest::txt2img(m2.clone(), "x"))
            .await
            .unwrap_err();
        assert_eq!(err.code, "vram");
        // No hardware detected in tests → CPU engine → it's the computer's memory.
        assert_eq!(err.message, generate::RAM_MESSAGE);

        // Missing components → plain "download them" message, before any engine work.
        let (_tmp3, core3, _) = new_core();
        use_external_engine(&core3, &failing.base_url());
        let m3 = register_fake_model(&core3, "sdxl");
        {
            let mut idx = core3.installed.lock();
            let comp: Vec<String> = idx
                .files
                .iter()
                .filter(|f| f.component_id.is_some())
                .map(|f| f.id.clone())
                .collect();
            for id in comp {
                idx.remove(&id);
            }
        }
        let err = generate::generate(&core3, GenerateRequest::txt2img(m3, "x"))
            .await
            .unwrap_err();
        assert_eq!(err.code, "not_found");
        assert!(err.message.contains("Get"), "{}", err.message);
        let err = generate::generate(&core3, GenerateRequest::txt2img("nope", "x"))
            .await
            .unwrap_err();
        assert_eq!(err.code, "not_found");
        let err = generate::generate(&core3, GenerateRequest::txt2img(m2, "   "))
            .await
            .unwrap_err();
        assert_eq!(err.code, "invalid");
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

    #[tokio::test]
    async fn text_encoder_out_of_memory_retries_on_the_processor_and_is_remembered() {
        let (_tmp, core, rec) = gpu_core();
        let mock = MockSdServer::start_with(MockOptions {
            polls_before_done: 0,
            fail_with: Some(TE_OOM.into()),
            fail_first: 1,
            ..Default::default()
        })
        .await;
        use_external_engine(&core, &mock.base_url());
        let model = register_fake_model(&core, "z_image_turbo");

        let res = generate::generate(&core, GenerateRequest::txt2img(model.clone(), SENTINEL))
            .await
            .expect("retried on the processor");
        assert_eq!(res.images.len(), 1);
        let launches = core.gen.external_launches.lock().clone();
        assert_eq!(launches.len(), 2, "{launches:?}");
        assert!(!te_on_cpu(&launches[0]), "{:?}", launches[0]);
        assert!(te_on_cpu(&launches[1]), "{:?}", launches[1]);
        assert_eq!(mock.requests().len(), 2, "the same request was sent again");
        // The UI saw a "loading" step with a plain note, never the prompt.
        let notes: Vec<(GenPhase, String)> = rec
            .0
            .lock()
            .iter()
            .filter_map(|e| {
                if let CoreEvent::Generation(p) = e {
                    p.note.clone().map(|n| (p.phase, n))
                } else {
                    None
                }
            })
            .collect();
        assert!(
            notes
                .iter()
                .any(|(ph, n)| *ph == GenPhase::LoadingModel && n.contains("on the processor")),
            "{notes:?}"
        );
        assert!(notes.iter().all(|(_, n)| !n.contains(SENTINEL)));

        // Remembered for this model: the next run starts on the processor, no retry.
        rec.0.lock().clear();
        generate::generate(&core, GenerateRequest::txt2img(model.clone(), "x"))
            .await
            .unwrap();
        let launches = core.gen.external_launches.lock().clone();
        assert_eq!(launches.len(), 3);
        assert!(te_on_cpu(&launches[2]));
        assert!(rec
            .0
            .lock()
            .iter()
            .all(|e| !matches!(e, CoreEvent::Generation(p) if p.note.is_some())));
        // …and visible on the engine status while that model is loaded.
        {
            let mut f = core.gen.flags.lock();
            f.running = true;
            f.loaded_model_id = Some(model.clone());
        }
        let note = crate::engine_setup::engine_status(&core)
            .note
            .unwrap_or_default();
        assert!(
            note.contains("processor") && note.contains("Settings"),
            "{note}"
        );

        // Settings "off" overrides the remembered choice (and the note goes away).
        core.settings.write().text_encoder_on_cpu = "off".into();
        assert!(crate::engine_setup::engine_status(&core).note.is_none());
        generate::generate(&core, GenerateRequest::txt2img(model, "x"))
            .await
            .unwrap();
        assert!(!te_on_cpu(
            core.gen.external_launches.lock().last().unwrap()
        ));
    }

    #[tokio::test]
    async fn text_encoder_setting_on_and_off() {
        let (_tmp, core, _) = gpu_core();
        let always = MockSdServer::start_with(MockOptions {
            polls_before_done: 0,
            fail_with: Some(TE_OOM.into()),
            ..Default::default()
        })
        .await;
        use_external_engine(&core, &always.base_url());
        let model = register_fake_model(&core, "z_image_turbo");

        // Off: the text encoder stays on the card; the retries keep more of
        // the card free, then the weights in system memory, then the message
        // says where to change it.
        core.settings.write().text_encoder_on_cpu = "off".into();
        let err = generate::generate(&core, GenerateRequest::txt2img(model.clone(), SENTINEL))
            .await
            .unwrap_err();
        assert_eq!(err.code, "vram");
        assert_eq!(err.message, generate::TE_ON_GPU_MESSAGE);
        let launches = core.gen.external_launches.lock().clone();
        assert_eq!(launches.len(), 3, "{launches:?}");
        assert!(
            launches.iter().all(|a| !te_on_cpu(a))
                && max_vram(&launches[0]) == Some("-2")
                && max_vram(&launches[1]) == Some("-4")
                && !launches[1].iter().any(|a| a == "--offload-to-cpu")
                && launches[2].iter().any(|a| a == "--offload-to-cpu"),
            "{launches:?}"
        );
        let details = err.details.unwrap_or_default();
        assert!(
            details.contains("failed to encode prompt") && !details.contains(SENTINEL),
            "{details}"
        );

        // On: the first launch already has the text encoder on the processor;
        // running out of memory there means the computer's memory.
        core.settings.write().text_encoder_on_cpu = "on".into();
        let err = generate::generate(&core, GenerateRequest::txt2img(model, "x"))
            .await
            .unwrap_err();
        assert!(te_on_cpu(core.gen.external_launches.lock().last().unwrap()));
        assert_eq!(err.code, "vram");
        assert_eq!(err.message, generate::RAM_MESSAGE);
        assert_eq!(
            core.gen.external_launches.lock().len(),
            4,
            "one launch, no retry"
        );
    }

    #[tokio::test]
    async fn out_of_memory_is_never_the_generic_message() {
        let (_tmp, core, _) = gpu_core();
        let msg = "[WARN] model_manager.cpp:1919 - model manager cannot make enough memory available on CUDA0: need 900.00 MB device\n\
                   [ERROR] image.cpp:904 - sampling for image 1/1 failed after 3.20s";
        let mock = MockSdServer::start_with(MockOptions {
            polls_before_done: 0,
            fail_with: Some(msg.into()),
            ..Default::default()
        })
        .await;
        use_external_engine(&core, &mock.base_url());
        let model = register_fake_model(&core, "z_image_turbo");
        // Other programs held 9 GB when the engine started (nvidia-smi).
        *core.gen.gpu_others.lock() = Some(pinhole_hardware::OtherGpuUse {
            gpu_index: 0,
            total_mib: 16275,
            others_mib: 9216,
            processes: vec![pinhole_hardware::GpuProcess {
                pid: 4242,
                name: "python.exe".into(),
                used_mib: Some(9114),
            }],
        });
        // The memory plan this model's engine printed at start leads the details.
        let plan_line = "[INFO ] backend_fit.cpp:346  -     DiT          params  11740 MiB -> compute CUDA0, params CUDA0".to_string();
        *core.gen.memory_plan.lock() = Some((model.clone(), vec![plan_line.clone()]));
        let err = generate::generate(&core, GenerateRequest::txt2img(model, "x"))
            .await
            .unwrap_err();
        let details = err.details.clone().unwrap_or_default();
        // The last attempt kept the weights in system memory (no auto-fit): the plan is the earlier one's.
        assert!(
            details.starts_with("Memory plan of the earlier attempt")
                && details.contains(&plan_line),
            "{details}"
        );
        assert!(
            details.contains("sampling for image 1/1 failed"),
            "{details}"
        );
        assert_eq!(err.code, "vram");
        assert_eq!(
            err.message,
            "Your graphics card ran out of memory. Other programs are using 9 GB of your graphics memory: python.exe (8.9 GB). Close them and try again, or pick the smaller version of this model in Models."
        );
        assert_ne!(err.message, generate::UNKNOWN_JOB_MESSAGE);
        // Denoising ran out: one retry with more of the card kept free, one
        // with the weights in system memory (VAE tiling wouldn't help that step).
        let launches = core.gen.external_launches.lock().clone();
        assert_eq!(launches.len(), 3);
        let has = |a: &[String], f: &str| a.iter().any(|x| x == f);
        assert_eq!(
            launches.iter().map(|a| max_vram(a)).collect::<Vec<_>>(),
            [Some("-2"), Some("-4"), Some("-4")]
        );
        assert!(
            !has(&launches[0], "--offload-to-cpu")
                && !has(&launches[1], "--offload-to-cpu")
                && has(&launches[2], "--offload-to-cpu"),
            "{launches:?}"
        );
        assert!(
            launches.iter().all(|a| !has(a, "--vae-tiling")),
            "{launches:?}"
        );

        // A failure that isn't about memory keeps the generic message.
        let (_tmp2, core2, _) = gpu_core();
        let odd = MockSdServer::start_with(MockOptions {
            polls_before_done: 0,
            fail_with: Some("generate_image returned no results".into()),
            ..Default::default()
        })
        .await;
        use_external_engine(&core2, &odd.base_url());
        let m2 = register_fake_model(&core2, "z_image_turbo");
        let err = generate::generate(&core2, GenerateRequest::txt2img(m2, "x"))
            .await
            .unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.message.as_str()),
            ("engine_failed", generate::UNKNOWN_JOB_MESSAGE)
        );
        assert_eq!(core2.gen.external_launches.lock().len(), 1);
    }

    /// Like the real sd-server: the job only says "generate_image returned no
    /// results" and the reason is in the engine output (stdout/stderr → ring
    /// buffer). The field report (prompt encoding out of memory), then the
    /// diffusion model out of memory on the retry: each memory-saving choice is
    /// tried once, and the third run makes the picture.
    #[tokio::test]
    async fn field_report_prompt_then_diffusion_out_of_memory_recovers() {
        let (_tmp, core, rec) = gpu_core();
        let te_oom = format!(
            "ggml_cuda_init: found 1 CUDA devices (Total VRAM: 16275 MiB):\n\
             [WARN   ] model_manager.cpp:1753 - model manager memory on CUDA0: reported free 0.00 MB / total 16275.44 MB, tracked weights 7480.09 MB / other runtime 0.00 MB / current runtime 0.00 MB\n\
             [WARN   ] model_manager.cpp:1919 - model manager cannot make enough memory available on CUDA0: need 518.58 MB device / 6.58 MB budget, available 0.00 MB device / 7044.91 MB budget\n\
             [ERROR  ] ggml_runner.cpp:899  - qwen3 segment 1/1 (graph) failed during workspace capacity check\n\
             [ERROR  ] conditioner.hpp:2224 - LLM prompt encoding failed\n\
             [ERROR  ] image.cpp:448  - failed to encode prompt\n\
             [DEBUG  ] echo {SENTINEL} a lighthouse"
        );
        let diffusion_oom = "[WARN   ] model_manager.cpp:1919 - model manager cannot make enough memory available on CUDA0: need 1200.00 MB device / 300.00 MB budget, available 0.00 MB device / 7044.91 MB budget\n\
             [ERROR  ] ggml_runner.cpp:899  - z_image segment 4/9 (blocks) failed during workspace capacity check\n\
             [ERROR  ] image.cpp:904  - sampling for image 1/1 failed after 2.10s";
        let mock = MockSdServer::start_with(MockOptions {
            polls_before_done: 0,
            fail_outputs: vec![te_oom, diffusion_oom.into(), diffusion_oom.into()],
            engine_log: Some(engine_log(&core)),
            ..Default::default()
        })
        .await;
        use_external_engine(&core, &mock.base_url());
        let model = register_fake_model(&core, "z_image_turbo");

        let res = generate::generate(
            &core,
            GenerateRequest::txt2img(model.clone(), format!("{SENTINEL} a lighthouse")),
        )
        .await
        .expect("recovered");
        assert_eq!(res.images.len(), 1);
        assert_eq!(mock.requests().len(), 4);
        let launches = core.gen.external_launches.lock().clone();
        assert_eq!(launches.len(), 4, "{launches:?}");
        let offloaded = |a: &[String]| a.iter().any(|x| x == "--offload-to-cpu");
        assert!(!te_on_cpu(&launches[0]) && !offloaded(&launches[0]));
        assert!(te_on_cpu(&launches[1]) && !offloaded(&launches[1]));
        assert!(te_on_cpu(&launches[2]) && !offloaded(&launches[2]));
        assert_eq!(max_vram(&launches[1]), Some("-2"));
        assert_eq!(
            max_vram(&launches[2]),
            Some("-4"),
            "more of the card kept free"
        );
        assert!(te_on_cpu(&launches[3]) && offloaded(&launches[3]));
        assert_eq!(max_vram(&launches[3]), Some("-4"));

        // Each retry shows its own note while the engine reloads (the later one replaces the earlier).
        let loading_notes: Vec<String> = rec
            .0
            .lock()
            .iter()
            .filter_map(|e| match e {
                CoreEvent::Generation(p) if p.phase == GenPhase::LoadingModel => p.note.clone(),
                _ => None,
            })
            .collect();
        let te_note = loading_notes
            .iter()
            .position(|n| n == generate::TE_RETRY_NOTE)
            .expect("text encoder retry note");
        let room_note = loading_notes
            .iter()
            .position(|n| n == generate::MORE_ROOM_RETRY_NOTE)
            .expect("more room retry note");
        let offload_note = loading_notes
            .iter()
            .position(|n| n == generate::OFFLOAD_RETRY_NOTE)
            .expect("offload retry note");
        assert!(
            te_note < room_note && room_note < offload_note,
            "{loading_notes:?}"
        );
        // The text encoder choice is kept for the session; system memory only
        // while that engine stays loaded (and the engine status says so).
        let next = generate::generate(&core, GenerateRequest::txt2img(model.clone(), "x"))
            .await
            .unwrap();
        assert_eq!(next.images.len(), 1);
        let last = core.gen.external_launches.lock().last().cloned().unwrap();
        assert!(te_on_cpu(&last) && offloaded(&last), "{last:?}");
        {
            let mut f = core.gen.flags.lock();
            (f.running, f.loaded_model_id) = (true, Some(model.clone()));
        }
        let note = crate::engine_setup::engine_status(&core)
            .note
            .unwrap_or_default();
        assert!(note.contains("system memory"), "{note}");
        // Deleting another model leaves this engine alone, unless it has one of the files open.
        let running = core.gen.slot.lock().await.args.clone();
        generate::unload_model(
            &core,
            "other",
            &[std::path::PathBuf::from("/not/used.gguf")],
        )
        .await;
        assert_eq!(
            core.gen.slot.lock().await.model_id.as_deref(),
            Some(model.as_str())
        );
        let shared = running
            .iter()
            .find(|a| a.ends_with(".gguf") || a.ends_with(".safetensors"))
            .cloned()
            .expect("a weight file");
        generate::unload_model(&core, "other", &[shared.into()]).await;
        assert!(
            core.gen.slot.lock().await.model_id.is_none(),
            "the engine that had the file open stopped"
        );
        assert!(
            core.gen.offloaded.lock().is_some(),
            "same model, same settings: still system memory"
        );
        generate::generate(&core, GenerateRequest::txt2img(model.clone(), "x"))
            .await
            .unwrap();
        assert!(offloaded(
            &core.gen.external_launches.lock().last().cloned().unwrap()
        ));
        // A LoRA isn't in the launch args (the engine reads it per job), but it may be mapped: stop.
        let lora = core.data.models(ModelKind::Lora).join("style.safetensors");
        generate::unload_model(&core, "a-lora", &[lora]).await;
        assert!(
            core.gen.slot.lock().await.model_id.is_none(),
            "deleting a LoRA stops the engine"
        );
        assert!(
            core.gen.offloaded.lock().is_some(),
            "another model's delete keeps the system-memory choice"
        );
        generate::generate(&core, GenerateRequest::txt2img(model.clone(), "x"))
            .await
            .unwrap();
        assert!(offloaded(
            &core.gen.external_launches.lock().last().cloned().unwrap()
        ));
        // Upscale starts the engine with the same choices (it must not forget system memory).
        let saved = core.gen.offloaded.lock().clone();
        let wiring: Vec<String> = ["--diffusion-model", "/m.gguf"].map(String::from).to_vec();
        let off = generate::MemFallback {
            offload: true,
            ..Default::default()
        };
        *core.gen.offloaded.lock() =
            Some((model.clone(), generate::with_memory_choices(&wiring, off)));
        assert!(
            generate::with_remembered_offload(&core, &model, &wiring, Default::default(), true)
                .offload
        );
        assert!(
            !generate::with_remembered_offload(&core, "other", &wiring, Default::default(), true)
                .offload
        );
        assert!(
            !generate::with_remembered_offload(&core, &model, &wiring, Default::default(), false)
                .offload,
            "CPU engine"
        );
        *core.gen.offloaded.lock() = saved;
        // Other settings (other launch args): the card again.
        core.gen
            .offloaded
            .lock()
            .as_mut()
            .unwrap()
            .1
            .push("--other-setting".into());
        generate::generate(&core, GenerateRequest::txt2img(model.clone(), "x"))
            .await
            .unwrap();
        assert!(!offloaded(
            &core.gen.external_launches.lock().last().cloned().unwrap()
        ));
        assert!(core.gen.offloaded.lock().is_none());
        // Deleting this model forgets it.
        *core.gen.offloaded.lock() = Some((
            model.clone(),
            core.gen.external_launches.lock().last().cloned().unwrap(),
        ));
        generate::unload_model(&core, &model, &[]).await;
        generate::generate(&core, GenerateRequest::txt2img(model, "x"))
            .await
            .unwrap();
        let last = core.gen.external_launches.lock().last().cloned().unwrap();
        assert!(
            te_on_cpu(&last) && !offloaded(&last),
            "the next load tries the card again: {last:?}"
        );
        // The prompt never reached the engine output buffer (redacted like real output).
        let kept = engine_log(&core).tail_text(200);
        assert!(
            kept.contains("failed to encode prompt") && !kept.contains(SENTINEL),
            "{kept}"
        );
    }

    /// Decoding ran out of memory: one retry with tiling, remembered for the model and
    /// shown in the engine note (plain words); Fine-tune "VAE tiling: Off" still wins per
    /// request without restarting the engine.
    #[tokio::test]
    async fn remembered_tiling_is_shown_and_fine_tune_off_wins() {
        let (_tmp, core, _) = gpu_core();
        let vae_oom = "[WARN   ] model_manager.cpp:1919 - model manager cannot make enough memory available on CUDA0: need 1200.00 MB device\n\
                       [ERROR  ] ggml_runner.cpp:899  - vae segment 1/1 (graph) failed during workspace capacity check\n\
                       [ERROR  ] image.cpp:614  - decode_first_stage failed for latent 1";
        let mock = MockSdServer::start_with(MockOptions {
            polls_before_done: 0,
            fail_outputs: vec![vae_oom.into()],
            engine_log: Some(engine_log(&core)),
            ..Default::default()
        })
        .await;
        use_external_engine(&core, &mock.base_url());
        let model = register_fake_model(&core, "z_image_turbo");
        generate::generate(&core, GenerateRequest::txt2img(model.clone(), "x"))
            .await
            .expect("retried with tiling");
        let tiled = |a: &[String]| a.iter().any(|x| x == "--vae-tiling");
        let last = core.gen.external_launches.lock().last().cloned().unwrap();
        assert!(tiled(&last), "{last:?}");
        {
            let mut f = core.gen.flags.lock();
            (f.running, f.loaded_model_id) = (true, Some(model.clone()));
        }
        let note = crate::engine_setup::engine_status(&core)
            .note
            .unwrap_or_default();
        assert!(
            note.contains("smaller pieces") && note.contains("Fine-tune"),
            "{note}"
        );
        assert!(
            !note.contains("VAE"),
            "plain words outside Fine-tune: {note}"
        );
        let launches = core.gen.external_launches.lock().len();
        let mut req = GenerateRequest::txt2img(model, "x");
        req.fine_tune.vae_tiling = Some(false);
        generate::generate(&core, req).await.unwrap();
        let body = mock.requests().last().cloned().unwrap();
        assert_eq!(
            body["vae_tiling_params"]["enabled"],
            serde_json::json!(false),
            "{body}"
        );
        assert!(
            tiled(core.gen.external_launches.lock().last().unwrap()),
            "same launch args, no restart"
        );
        assert_eq!(core.gen.external_launches.lock().len(), launches + 1);
    }

    /// "failed to encode prompt" with no memory line (a broken or mismatched text
    /// encoder): no memory retry, a plain message that points to Models.
    #[tokio::test]
    async fn encoder_failure_without_memory_lines_is_not_out_of_memory() {
        let (_tmp, core, _) = gpu_core();
        let out = "generate_image returned no results\n[ERROR  ] conditioner.hpp:2224 - LLM prompt encoding failed\n[ERROR  ] image.cpp:448  - failed to encode prompt";
        let mock = MockSdServer::start_with(MockOptions {
            polls_before_done: 0,
            fail_with: Some(out.into()),
            ..Default::default()
        })
        .await;
        use_external_engine(&core, &mock.base_url());
        let model = register_fake_model(&core, "z_image_turbo");
        let err = generate::generate(&core, GenerateRequest::txt2img(model.clone(), SENTINEL))
            .await
            .unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.message.as_str()),
            ("model_load", generate::ENCODER_FAILED_MESSAGE)
        );
        assert_eq!(core.gen.external_launches.lock().len(), 1, "no retry");
        assert!(
            core.gen.mem_fallback.lock().get(&model).is_none(),
            "nothing remembered"
        );
        assert!(!err.details.unwrap_or_default().contains(SENTINEL));
    }

    /// Progress notes: the other-programs note is measured again at every
    /// engine start and replaces the older one (shown first); a later retry
    /// note replaces an earlier one.
    #[tokio::test]
    async fn job_notes_replace_older_ones() {
        let (_tmp, core, _) = gpu_core();
        let others = |mib| pinhole_hardware::OtherGpuUse {
            gpu_index: 0,
            total_mib: 16275,
            others_mib: mib,
            processes: vec![],
        };
        let notes = || core.gen.job_note.lock().clone();
        generate::set_others_note(&core, Some(generate::others_note(&others(9216))));
        generate::set_retry_note(&core, generate::TE_RETRY_NOTE);
        generate::set_others_note(&core, Some(generate::others_note(&others(9300))));
        assert_eq!(notes().len(), 2, "{:?}", notes());
        assert!(
            notes()[0].starts_with("Other programs are using 9.1 GB of your graphics memory."),
            "{:?}",
            notes()
        );
        assert_eq!(notes()[1], generate::TE_RETRY_NOTE);
        generate::set_retry_note(&core, generate::TILING_RETRY_NOTE);
        generate::set_others_note(&core, None);
        assert_eq!(
            notes(),
            vec![generate::TILING_RETRY_NOTE.to_string()],
            "closed the other program: no note"
        );
    }

    /// Out of memory for good (every run fails): a `vram` error that says what
    /// to do, with the engine output (never the prompt) behind Details.
    #[tokio::test]
    async fn field_report_without_recovery_is_a_plain_vram_error() {
        let (_tmp, core, _) = gpu_core();
        let te_oom = TE_OOM
            .lines()
            .skip(1)
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("\n");
        let diffusion_oom = "[ERROR  ] ggml_runner.cpp:899  - z_image segment 1/9 (blocks) failed during workspace allocation\n\
                             [ERROR  ] image.cpp:904  - sampling for image 1/1 failed after 0.40s";
        let mock = MockSdServer::start_with(MockOptions {
            polls_before_done: 0,
            fail_outputs: vec![
                te_oom,
                diffusion_oom.into(),
                diffusion_oom.into(),
                diffusion_oom.into(),
            ],
            engine_log: Some(engine_log(&core)),
            ..Default::default()
        })
        .await;
        use_external_engine(&core, &mock.base_url());
        let model = register_fake_model(&core, "z_image_turbo");
        let err = generate::generate(&core, GenerateRequest::txt2img(model, SENTINEL))
            .await
            .unwrap_err();
        assert_eq!(err.code, "vram");
        assert_eq!(err.message, generate::VRAM_MESSAGE);
        assert!(
            err.message
                .starts_with("Your graphics card ran out of memory. Close other programs"),
            "{}",
            err.message
        );
        assert_eq!(
            mock.requests().len(),
            4,
            "text encoder, more room and system memory retries, then stop"
        );
        let details = err.details.unwrap_or_default();
        assert!(
            details.contains("generate_image returned no results")
                && details.contains("sampling for image 1/1 failed"),
            "{details}"
        );
        assert!(!details.contains(SENTINEL));
        let st = crate::engine_setup::engine_status(&core);
        assert!(st.error.is_none(), "a job error is not an engine problem");
    }

    /// A launch first kills leftover engines under `Data/engine/` (they hold
    /// graphics memory); an engine that runs out of memory while loading gets
    /// a plain `vram` error (the computer's memory on the CPU build).
    #[cfg(unix)]
    #[tokio::test]
    async fn launch_kills_leftover_engines_and_load_oom_is_plain() {
        use pinhole_engine::install::{self, EngineKind, InstallMarker};
        use std::os::unix::fs::PermissionsExt;
        let Some(sleep) = ["/usr/bin/sleep", "/bin/sleep"]
            .iter()
            .map(std::path::Path::new)
            .find(|p| p.is_file())
        else {
            return;
        };
        let (_tmp, core, _) = new_core();
        core.settings.write().engine_backend = "cpu".into();
        let (cfg, sel) = crate::engine_setup::selected_build(&core, EngineKind::Sd).unwrap();
        let root = core.data.engine();
        let version = cfg.stable_diffusion_cpp.version.clone();
        let exec = |p: &std::path::Path| {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap()
        };

        // The installed "engine" runs out of memory while loading.
        let dir = install::install_dir(&root, EngineKind::Sd, &version, &sel.backend);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("sd-server");
        std::fs::write(&exe, "#!/bin/sh\necho 'ggml_backend_cpu_buffer_type_alloc_buffer: failed to allocate buffer of size 9663676416' >&2\necho '[ERROR  ] main.cpp:91   - new_sd_ctx_t failed' >&2\nexit 1\n").unwrap();
        exec(&exe);
        let marker = InstallMarker {
            engine: EngineKind::Sd,
            version: version.clone(),
            backend: sel.backend.clone(),
            build: sel.key.clone(),
            binary: "sd-server".into(),
            archives: vec![],
            installed_at: 0,
        };
        std::fs::write(
            dir.join(install::MARKER_FILE),
            serde_json::to_string(&marker).unwrap(),
        )
        .unwrap();

        // A leftover engine from an earlier run (same binary name, inside Data/engine).
        let old = root.join("sd").join("master-1-0000000").join("cpu");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::copy(sleep, old.join("sd-server")).unwrap();
        exec(&old.join("sd-server"));
        let mut leftover = std::process::Command::new(old.join("sd-server"))
            .arg("30")
            .spawn()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        if leftover.try_wait().unwrap().is_some() {
            return; // `sleep` is a multi-call binary here (busybox): can't run under another name.
        }

        let model = register_fake_model(&core, "sd15");
        let err = generate::generate(&core, GenerateRequest::txt2img(model, "x"))
            .await
            .unwrap_err();
        assert!(
            leftover.try_wait().unwrap().is_some(),
            "the leftover engine was killed before the launch"
        );
        assert_eq!(err.code, "vram", "{err:?}");
        assert_eq!(err.message, generate::RAM_MESSAGE);
        let details = err.details.unwrap_or_default();
        assert!(
            details.contains("failed to allocate") && details.contains("exit code 1"),
            "{details}"
        );
        let _ = leftover.kill();
        let _ = leftover.wait();
    }

    #[tokio::test]
    async fn upscale_uses_installed_esrgan() {
        let (_tmp, core, rec) = new_core();
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
        let before = rec.0.lock().len();
        let up4 = generate::upscale_image(&core, &src.id, 4).await.unwrap();
        let phases: Vec<GenPhase> = rec.0.lock()[before..]
            .iter()
            .filter_map(|e| {
                if let CoreEvent::Generation(p) = e {
                    Some(p.phase)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            phases,
            vec![GenPhase::Generating, GenPhase::Done],
            "one Done, after the upscale"
        );
        assert_eq!(
            last_generation_event(&rec).unwrap().model_label.as_deref(),
            Some(src.model_label.as_str())
        );
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
        let img = session::import_image(
            &core,
            pinhole_engine::testutil::solid_png(32, 32, [1, 2, 3, 255]),
        )
        .unwrap();
        let text = describe::describe_image(&core, &img.id, describe::DescribeStyle::Sentence)
            .await
            .unwrap();
        assert_eq!(text, "a lighthouse at dusk");
        let body = &llama.requests()[0];
        let instruction = body
            .pointer("/messages/0/content/1/text")
            .and_then(|t| t.as_str())
            .unwrap();
        assert!(
            instruction.contains("Describe this image"),
            "registry instruction is used"
        );
        assert!(describe::captioner_status(&core).available);
    }

    #[tokio::test]
    async fn sd_args_force_loopback_and_privacy_flags() {
        let (_tmp, core, _) = new_core();
        let cfg = crate::engine_setup::engine_config(&core).unwrap();
        let wiring: Vec<String> = [
            "--model",
            "/m.safetensors",
            "--listen-ip",
            "0.0.0.0",
            "--listen-port",
            "80",
            "--vae-tiling",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let args = crate::generate::full_sd_args(&core, &wiring, &cfg);
        let ips: Vec<&String> = args
            .iter()
            .enumerate()
            .filter(|(i, a)| *a == "--listen-ip" && *i + 1 < args.len())
            .map(|(i, _)| &args[i + 1])
            .collect();
        assert_eq!(ips, vec!["127.0.0.1"], "{args:?}");
        assert!(
            !args
                .iter()
                .any(|a| a == "--listen-port" || a == "80" || a == "0.0.0.0"),
            "{args:?}"
        );
        assert_eq!(
            args.iter()
                .filter(|a| *a == "--disable-image-metadata")
                .count(),
            1
        );
        assert_eq!(args.iter().filter(|a| *a == "--log-level").count(), 1);
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--lora-model-dir" && w[1].ends_with("loras")));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--hires-upscalers-dir" && w[1].ends_with("upscalers")));
        assert!(args.contains(&"--vae-tiling".to_string()));
        // Info level (memory plan in Details) and memory-mapped weights by default.
        assert!(
            args.windows(2).any(|w| w == ["--log-level", "info"]),
            "{args:?}"
        );
        assert!(args.contains(&"--mmap".to_string()), "{args:?}");
        // Verbose / debug print the request (prompt included): never enabled.
        let mut loud = (*cfg).clone();
        loud.stable_diffusion_cpp
            .launch_defaults
            .extend(["--verbose", "--log-level", "debug"].map(String::from));
        let wiring_loud: Vec<String> =
            ["--model", "/m.safetensors", "-v", "--log-level", "verbose"]
                .map(String::from)
                .to_vec();
        let args = crate::generate::full_sd_args(&core, &wiring_loud, &loud);
        assert!(
            !args
                .iter()
                .any(|a| a == "--verbose" || a == "-v" || a == "debug" || a == "verbose"),
            "{args:?}"
        );
        assert_eq!(
            args.iter().filter(|a| *a == "--log-level").count(),
            1,
            "{args:?}"
        );
        assert!(
            args.windows(2).any(|w| w == ["--log-level", "info"]),
            "{args:?}"
        );
    }

    /// Drives the REAL sd-server (Linux) through `generate` when
    /// `PINHOLE_SD_ARCHIVE` points at the pinned `…-bin-Linux-Ubuntu-24.04-x86_64.zip`:
    /// install from the archive, launch with a bogus model file, expect the
    /// plain-language "couldn't be loaded" error with the engine output in details.
    #[tokio::test]
    async fn real_engine_bogus_model_gives_plain_error() {
        let Ok(archive) = std::env::var("PINHOLE_SD_ARCHIVE") else {
            return;
        };
        let (_tmp, core, rec) = new_core();
        let (cfg, sel) =
            crate::engine_setup::selected_build(&core, pinhole_engine::install::EngineKind::Sd)
                .unwrap();
        assert_eq!(sel.key, "linux_cpu");
        let sha = pinhole_net::download::sha256_file(std::path::Path::new(&archive)).unwrap();
        let installed = pinhole_engine::install::unpack_build(
            &core.data.engine(),
            pinhole_engine::install::EngineKind::Sd,
            &cfg.stable_diffusion_cpp,
            &sel,
            &[(
                sel.build.archives()[0].clone(),
                PathBuf::from(&archive),
                sha,
            )],
        )
        .unwrap();
        assert!(installed.exe.ends_with("sd-server"));
        assert!(crate::engine_setup::engine_status(&core).installed);
        let model = register_fake_model(&core, "sd15");
        let err = generate::generate(&core, GenerateRequest::txt2img(model, "a cat"))
            .await
            .unwrap_err();
        assert_eq!(err.code, "model_load", "{err:?}");
        assert!(
            err.message.contains("couldn't be loaded"),
            "{}",
            err.message
        );
        assert!(
            err.details
                .as_deref()
                .unwrap_or("")
                .contains("new_sd_ctx_t failed"),
            "{:?}",
            err.details
        );
        let phases: Vec<GenPhase> = rec
            .0
            .lock()
            .iter()
            .filter_map(|e| {
                if let CoreEvent::Generation(p) = e {
                    Some(p.phase)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(phases.first(), Some(&GenPhase::LoadingModel));
        assert_eq!(phases.last(), Some(&GenPhase::Failed));
        // The top bar shows the same message, code and engine output.
        let st = crate::engine_setup::engine_status(&core);
        assert!(!st.running);
        assert_eq!(st.error.as_deref(), Some(err.message.as_str()));
        assert_eq!(st.error_code.as_deref(), Some("model_load"));
        assert_eq!(st.error_details, err.details);
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
        let st = crate::engine_setup::install_engine(&core)
            .await
            .expect("install_engine");
        assert!(st.installed && !st.installing, "{st:?}");
        assert_eq!(st.backend.as_deref(), Some("cpu"));
        let exe =
            crate::engine_setup::installed_engine(&core, pinhole_engine::install::EngineKind::Sd)
                .unwrap()
                .exe;
        assert!(exe.is_file());
        let engine_events = rec
            .0
            .lock()
            .iter()
            .filter(|e| matches!(e, CoreEvent::Engine(_)))
            .count();
        assert!(engine_events >= 2, "installing → installed events");
        assert!(core
            .downloads
            .status()
            .iter()
            .any(|g| g.label.starts_with("Image engine")));

        // The describe engine (llama.cpp .tar.gz with symlinks) installs and runs too.
        let llama =
            crate::engine_setup::install_kind(&core, pinhole_engine::install::EngineKind::Llama)
                .await
                .expect("llama install");
        let out = std::process::Command::new(&llama.exe)
            .arg("--version")
            .current_dir(&llama.dir)
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
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
    async fn installed_other_text_encoder_option_is_used() {
        // Regression: bf16 Qwen3-4B installed before the 16 GB tier moved to Q8.
        let (_tmp, core, _) = new_core();
        let reg = core.registry();
        let fam = reg.family("z_image_turbo").unwrap().clone();
        let hw = pinhole_registry::wiring::HwContext {
            vram_gb: 16.0,
            backend: "cuda".into(),
            ram_gb: 32.0,
        };
        let (rel, size) = write_dummy(
            &core,
            ModelKind::Diffusion,
            "z_image_turbo_bf16.safetensors",
        );
        let zit = InstalledFile {
            id: "zit".into(),
            rel_path: rel,
            kind: ModelKind::Diffusion,
            sha256: "2".repeat(64),
            size_bytes: size,
            family: Some("z_image_turbo".into()),
            component_id: None,
            friendly_name: "Z-Image Turbo".into(),
            civitai: None,
            added_at: 0,
            last_used: None,
            observed_vram_gb: None,
            dtype: None,
        };
        let err = crate::generate::model_files(&core, &zit, &fam, &hw).unwrap_err();
        assert!(
            err.message.contains("Qwen3-4B-Q8_0.gguf"),
            "{}",
            err.message
        );
        {
            let mut idx = core.installed.lock();
            for (id, kind) in [
                ("flux_ae", ModelKind::Vae),
                ("qwen3_4b", ModelKind::TextEncoder),
            ] {
                let (rel, size) = write_dummy(&core, kind, &reg.component(id).unwrap().file);
                idx.upsert(InstalledFile {
                    id: id.into(),
                    rel_path: rel,
                    kind,
                    sha256: format!("{:0>64}", id.len()),
                    size_bytes: size,
                    family: None,
                    component_id: Some(id.into()),
                    friendly_name: id.into(),
                    civitai: None,
                    added_at: 0,
                    last_used: None,
                    observed_vram_gb: None,
                    dtype: None,
                });
            }
        }
        let files = crate::generate::model_files(&core, &zit, &fam, &hw).unwrap();
        let llm = files.components.get("llm").unwrap();
        assert!(
            llm.ends_with(&reg.component("qwen3_4b").unwrap().file),
            "{llm:?}"
        );
    }

    /// A long-running child standing in for a Pinhole-started engine.
    #[cfg(unix)]
    fn fake_engine(dir: &std::path::Path, body: &str) -> pinhole_engine::EngineProcess {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(format!("fake-engine-{}.sh", uuid::Uuid::new_v4()));
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        pinhole_engine::EngineProcess::spawn(
            &p,
            &[],
            1,
            Arc::new(pinhole_engine::LogBuffer::default()),
        )
        .unwrap()
    }

    #[cfg(unix)]
    async fn put_engine(core: &AppCore, proc: pinhole_engine::EngineProcess, results_cached: bool) {
        *core.gen.slot.lock().await = generate::EngineSlot {
            proc: Some(proc),
            args: vec![],
            model_id: Some("m".into()),
            results_cached,
        };
    }

    #[cfg(unix)]
    async fn engine_running(core: &AppCore) -> bool {
        core.gen.slot.lock().await.proc.is_some()
    }

    /// sd-server keeps finished results (unauthenticated) for 600 s: Reset
    /// and the idle timer stop a Pinhole-started engine once it ran a job.
    #[cfg(unix)]
    #[tokio::test]
    async fn clear_session_and_idle_stop_engines_that_hold_results() {
        use std::sync::atomic::Ordering;
        let (tmp, core, _rec) = new_core();

        // No job ran yet: Reset leaves the loaded model alone.
        put_engine(&core, fake_engine(tmp.path(), "exec sleep 30"), false).await;
        session::clear(&core).await;
        assert!(engine_running(&core).await);
        // After a job: stopped.
        core.gen.slot.lock().await.results_cached = true;
        session::clear(&core).await;
        assert!(!engine_running(&core).await);
        assert!(!crate::engine_setup::engine_status(&core).running);

        // Idle stop: not while a newer job started, yes once nothing did.
        *core.gen.idle_stop_after.lock() = Duration::from_millis(100);
        put_engine(&core, fake_engine(tmp.path(), "exec sleep 30"), true).await;
        let epoch = core.gen.activity.fetch_add(1, Ordering::SeqCst) + 1;
        generate::arm_idle_stop(&core, epoch);
        core.gen.activity.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            engine_running(&core).await,
            "a newer job started: keep the engine"
        );
        generate::arm_idle_stop(&core, core.gen.activity.load(Ordering::SeqCst));
        for _ in 0..100 {
            if !engine_running(&core).await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            !engine_running(&core).await,
            "idle engine with results is stopped"
        );

        // Reset while a job runs: the engine stops right after the job.
        put_engine(&core, fake_engine(tmp.path(), "exec sleep 30"), true).await;
        let run = core.gen.run_lock.lock().await;
        session::clear(&core).await;
        assert!(engine_running(&core).await && core.gen.clear_pending.load(Ordering::SeqCst));
        drop(run);
        generate::after_job(&core, core.gen.activity.load(Ordering::SeqCst)).await;
        assert!(!engine_running(&core).await);
    }

    #[tokio::test]
    async fn external_engines_are_never_stopped() {
        let (_tmp, core, _rec) = new_core();
        let mock = MockSdServer::start().await;
        use_external_engine(&core, &mock.base_url());
        *core.gen.idle_stop_after.lock() = Duration::from_millis(50);
        let model = register_fake_model(&core, "sdxl");
        generate::generate(&core, GenerateRequest::txt2img(model.clone(), "a boat"))
            .await
            .unwrap();
        assert!(
            core.gen.slot.lock().await.results_cached,
            "a submitted job marks the engine"
        );
        session::clear(&core).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            core.gen.external.lock().as_deref(),
            Some(mock.base_url().as_str())
        );
        generate::generate(&core, GenerateRequest::txt2img(model, "a boat"))
            .await
            .unwrap();
        assert_eq!(mock.requests().len(), 2);
    }

    /// Port squatting: after `wait_ready`, the server on the port must be our
    /// live child and report the model we launched.
    #[cfg(unix)]
    #[tokio::test]
    async fn engine_port_answered_by_someone_else_is_refused() {
        let (tmp, core, _rec) = new_core();
        let squatter = MockSdServer::start().await; // reports /mock/mock.safetensors
        let client = pinhole_engine::SdClient::new(core.local.clone(), squatter.base_url());
        let mut ours = fake_engine(tmp.path(), "exec sleep 30");
        let real = tmp.path().join("model.safetensors");
        std::fs::write(&real, b"x").unwrap();
        let args = vec!["--model".to_string(), real.to_string_lossy().into_owned()];
        let err = generate::verify_engine_identity(&mut ours, &client, &args)
            .await
            .unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.message.as_str()),
            ("engine_failed", generate::PORT_TAKEN_MESSAGE)
        );
        let args = vec![
            "--diffusion-model".to_string(),
            "/mock/mock.safetensors".to_string(),
        ];
        generate::verify_engine_identity(&mut ours, &client, &args)
            .await
            .unwrap();
        ours.stop().await;

        let mut dead = fake_engine(tmp.path(), "exit 0");
        for _ in 0..100 {
            if !dead.is_running() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let err = generate::verify_engine_identity(&mut dead, &client, &args)
            .await
            .unwrap_err();
        assert_eq!(
            err.message,
            generate::PORT_TAKEN_MESSAGE,
            "our child died: whoever answered isn't us"
        );

        // llama-server: /v1/models (needs our API key) must report our model.
        let llama = MockLlamaServer::start_with_key("x", 0, Some("k-1")).await;
        let keyed = pinhole_engine::llama::LlamaClient::new(core.local.clone(), llama.base_url())
            .with_api_key("k-1");
        let mut ours = fake_engine(tmp.path(), "exec sleep 30");
        let model = std::path::Path::new(pinhole_engine::testutil::MOCK_LLAMA_MODEL);
        describe::verify_llama_identity(&mut ours, &keyed, model)
            .await
            .unwrap();
        let err = describe::verify_llama_identity(
            &mut ours,
            &keyed,
            std::path::Path::new("/other/another-model.gguf"),
        )
        .await
        .unwrap_err();
        assert_eq!(err.message, generate::PORT_TAKEN_MESSAGE);
        let anon = pinhole_engine::llama::LlamaClient::new(core.local.clone(), llama.base_url());
        assert!(
            describe::verify_llama_identity(&mut ours, &anon, model)
                .await
                .is_err(),
            "no key → 401 → not ours"
        );
        ours.stop().await;
    }

    /// Imported JPEG/WebP are re-encoded as PNG: EXIF / XMP (GPS, camera) never
    /// reach the session, the engines or a saved file.
    #[tokio::test]
    async fn imported_jpeg_loses_its_exif() {
        let (_tmp, core, _rec) = new_core();
        let jpeg = pinhole_engine::image::jpeg_with_exif(8, 4, "GPS 52.52N PINHOLE_EXIF_SECRET");
        assert!(has(&jpeg, "PINHOLE_EXIF_SECRET"));
        let img = session::import_image(&core, jpeg).unwrap();
        assert_eq!(
            (img.width, img.height),
            (4, 8),
            "EXIF orientation applied before it is dropped"
        );
        let bytes = session::get(&core, &img.id).unwrap();
        assert!(pinhole_engine::png::is_png(&bytes));
        assert!(!has(&bytes, "PINHOLE_EXIF_SECRET"));
        assert_eq!(pinhole_engine::png::dimensions(&bytes), Some((4, 8)));
        let saved = session::save_image(&core, &img.id).unwrap();
        assert!(saved.path.ends_with("_import.png"), "{}", saved.path);
        assert!(!has(
            &std::fs::read(&saved.path).unwrap(),
            "PINHOLE_EXIF_SECRET"
        ));
        assert_eq!(
            session::import_image(&core, b"not an image".to_vec())
                .unwrap_err()
                .code,
            "invalid"
        );
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

    /// Reset while a job runs: the job's images must not land in the session afterwards.
    #[tokio::test]
    async fn reset_during_a_job_drops_its_images() {
        let (_tmp, core, _rec) = new_core();
        let mock = MockSdServer::start_with(MockOptions {
            polls_before_done: 4,
            ..Default::default()
        })
        .await;
        use_external_engine(&core, &mock.base_url());
        let model = register_fake_model(&core, "sdxl");
        let c2 = core.clone();
        let task = tokio::spawn(async move {
            generate::generate(&c2, GenerateRequest::txt2img(model, "a lighthouse")).await
        });
        for _ in 0..200 {
            if !mock.requests().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!mock.requests().is_empty());
        session::clear(&core).await;
        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.code, "cancelled");
        assert!(
            core.session.is_empty(),
            "Reset drops every in-memory image, also one that finished later"
        );
    }

    /// An engine answer that echoes the request must not put prompt text in the error details.
    #[tokio::test]
    async fn engine_400_details_are_redacted() {
        let (_tmp, core, _rec) = new_core();
        let secrets = vec![format!("{SENTINEL} a red boat")];
        let e = pinhole_engine::sdapi::ApiError::Status {
            code: 400,
            error: format!("bad field near \"{SENTINEL} a red boat\""),
        };
        let err = generate::api_failure(&core, e, &secrets);
        assert_eq!(err.code, "invalid");
        assert!(!err.details.unwrap_or_default().contains(SENTINEL));
        let e = pinhole_engine::sdapi::ApiError::Status {
            code: 500,
            error: format!("{SENTINEL} a red boat"),
        };
        assert!(!generate::api_failure(&core, e, &secrets)
            .details
            .unwrap_or_default()
            .contains(SENTINEL));
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

    /// Deleting the model that is loading cancels the load instead of waiting
    /// up to the load timeout for the engine slot.
    #[cfg(unix)]
    #[tokio::test]
    async fn deleting_the_loading_model_cancels_the_load() {
        let (_tmp, core, _rec) = new_core();
        install_fake_engine(
            &core,
            pinhole_engine::install::EngineKind::Sd,
            "sd-server",
            "exec sleep 30",
        );
        let model = register_fake_model(&core, "sd15");
        let c2 = core.clone();
        let m2 = model.clone();
        let task = tokio::spawn(async move {
            generate::generate(&c2, GenerateRequest::txt2img(m2, "x")).await
        });
        for _ in 0..500 {
            if core.gen.loading.lock().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(core.gen.loading.lock().is_some(), "the engine is loading");
        tokio::time::timeout(
            Duration::from_secs(10),
            generate::unload_model(&core, &model, &[]),
        )
        .await
        .expect("unload doesn't wait for the load");
        assert_eq!(task.await.unwrap().unwrap_err().code, "cancelled");
        assert!(core.gen.slot.lock().await.proc.is_none());
    }

    /// A delete while the job still stops the previous engine (before the new
    /// one is spawned) cancels the job too: no new engine starts.
    #[cfg(unix)]
    #[tokio::test]
    async fn deleting_the_model_while_the_old_engine_stops_cancels_the_load() {
        let (tmp, core, _rec) = new_core();
        let spawned = tmp.path().join("spawned");
        install_fake_engine(
            &core,
            pinhole_engine::install::EngineKind::Sd,
            "sd-server",
            &format!("touch '{}'\nexec sleep 30", spawned.display()),
        );
        // The running engine ignores SIGTERM, so stopping it takes a few seconds.
        let old = tmp.path().join("old-sd.sh");
        std::fs::write(&old, "#!/bin/sh\ntrap '' TERM\nwhile :; do sleep 1; done\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let proc = pinhole_engine::EngineProcess::spawn(
            &old,
            &[],
            1,
            Arc::new(pinhole_engine::LogBuffer::default()),
        )
        .unwrap();
        put_engine(&core, proc, false).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let model = register_fake_model(&core, "sd15");
        let c2 = core.clone();
        let m2 = model.clone();
        let task = tokio::spawn(async move {
            generate::generate(&c2, GenerateRequest::txt2img(m2, "x")).await
        });
        for _ in 0..200 {
            if core.gen.loading.lock().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            core.gen.loading.lock().is_some(),
            "marked as loading while the old engine stops"
        );
        generate::unload_model(&core, &model, &[]).await;
        assert_eq!(task.await.unwrap().unwrap_err().code, "cancelled");
        assert!(!spawned.exists(), "no engine started after the cancel");
        assert!(core.gen.loading.lock().is_none());
    }

    /// Reset while a generate waits for the previous job: its images are dropped too.
    #[tokio::test]
    async fn reset_while_a_job_waits_drops_its_images() {
        let (_tmp, core, _rec) = new_core();
        let mock = MockSdServer::start().await;
        use_external_engine(&core, &mock.base_url());
        let model = register_fake_model(&core, "sdxl");
        let busy = core.gen.run_lock.lock().await;
        let c2 = core.clone();
        let task = tokio::spawn(async move {
            generate::generate(&c2, GenerateRequest::txt2img(model, "a lighthouse")).await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        core.session.clear();
        drop(busy);
        assert_eq!(task.await.unwrap().unwrap_err().code, "cancelled");
        assert!(core.session.is_empty());
    }

    /// Cancelling an upscale stops a Pinhole-started engine: sd-server upscales
    /// synchronously and would keep working otherwise.
    #[cfg(unix)]
    #[tokio::test]
    async fn cancelled_upscale_stops_the_engine() {
        let (tmp, core, rec) = new_core();
        install_component(
            &core,
            ModelKind::Upscaler,
            "RealESRGAN_x4plus.pth",
            generate::UPSCALER_COMPONENT,
        );
        // An "engine" whose port accepts the request but never answers.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let script = tmp.path().join("fake-sd.sh");
        std::fs::write(&script, "#!/bin/sh\nexec sleep 30\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let proc = pinhole_engine::EngineProcess::spawn(
            &script,
            &[],
            port,
            Arc::new(pinhole_engine::LogBuffer::default()),
        )
        .unwrap();
        put_engine(&core, proc, false).await;
        let img = session::import_image(
            &core,
            pinhole_engine::testutil::solid_png(8, 8, [1, 2, 3, 255]),
        )
        .unwrap();
        let c2 = core.clone();
        let task = tokio::spawn(async move { generate::upscale_image(&c2, &img.id, 4).await });
        let (_conn, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
            .await
            .expect("upscale request sent")
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        generate::cancel(&core);
        let err = tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("cancel ends the upscale")
            .unwrap()
            .unwrap_err();
        assert_eq!(err.code, "cancelled");
        assert!(!engine_running(&core).await, "the busy engine was stopped");
        assert_eq!(
            last_generation_event(&rec).map(|p| p.phase),
            Some(GenPhase::Cancelled)
        );
        assert_eq!(core.session.len(), 1, "only the source image");
    }

    /// App exit / update while the describe engine loads: `shutdown` doesn't
    /// wait for the load timeout, and describe works again afterwards.
    #[cfg(unix)]
    #[tokio::test]
    async fn describe_shutdown_cancels_a_loading_engine() {
        let (_tmp, core, _rec) = new_core();
        // Its log reads like an out-of-memory failure: a cancelled load is still "cancelled".
        install_fake_engine(
            &core,
            pinhole_engine::install::EngineKind::Llama,
            "llama-server",
            "echo 'ggml: out of memory'\nexec sleep 30",
        );
        install_component(
            &core,
            ModelKind::Captioner,
            "cap.gguf",
            describe::DEFAULT_MODEL_ID,
        );
        install_component(
            &core,
            ModelKind::Captioner,
            "cap-mmproj.gguf",
            describe::DEFAULT_MMPROJ_ID,
        );
        let img = session::import_image(
            &core,
            pinhole_engine::testutil::solid_png(8, 8, [1, 2, 3, 255]),
        )
        .unwrap();
        let c2 = core.clone();
        let id = img.id.clone();
        let task = tokio::spawn(async move {
            describe::describe_image(&c2, &id, describe::DescribeStyle::Sentence).await
        });
        for _ in 0..500 {
            if core.describe.slot.try_lock().is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(core.describe.is_busy());
        tokio::time::timeout(Duration::from_secs(10), describe::shutdown(&core))
            .await
            .expect("shutdown doesn't wait for the load");
        assert_eq!(task.await.unwrap().unwrap_err().code, "cancelled");
        assert!(!core.describe.is_busy());
        assert!(
            !core.describe.stopping.lock().is_cancelled(),
            "a fresh token for the next describe"
        );
    }

    /// A describe that gets the engine slot while a shutdown is pending
    /// doesn't start llama-server at all.
    #[cfg(unix)]
    #[tokio::test]
    async fn describe_during_shutdown_starts_no_engine() {
        let (tmp, core, _rec) = new_core();
        let spawned = tmp.path().join("spawned");
        install_fake_engine(
            &core,
            pinhole_engine::install::EngineKind::Llama,
            "llama-server",
            &format!("touch '{}'\nexec sleep 30", spawned.display()),
        );
        install_component(
            &core,
            ModelKind::Captioner,
            "cap.gguf",
            describe::DEFAULT_MODEL_ID,
        );
        install_component(
            &core,
            ModelKind::Captioner,
            "cap-mmproj.gguf",
            describe::DEFAULT_MMPROJ_ID,
        );
        let img = session::import_image(
            &core,
            pinhole_engine::testutil::solid_png(8, 8, [1, 2, 3, 255]),
        )
        .unwrap();
        // A launch clears the log buffer: this line stays only when nothing launched.
        core.describe.logs.push_bytes(b"before\n");
        // What `shutdown` does first, before it gets the slot.
        core.describe.stopping.lock().cancel();
        let err = describe::describe_image(&core, &img.id, describe::DescribeStyle::Sentence)
            .await
            .unwrap_err();
        assert_eq!(err.code, "cancelled");
        assert!(
            core.describe.logs.tail_text(5).contains("before"),
            "llama-server wasn't started"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!spawned.exists(), "llama-server wasn't started");
    }

    #[tokio::test]
    async fn engine_status_and_captioner_status_without_engine() {
        let (_tmp, core, _rec) = new_core();
        let st = crate::engine_setup::engine_status(&core);
        assert!(!st.installed && !st.running);
        assert!(st.version.as_deref().unwrap_or("").starts_with("master-"));
        let cs = describe::captioner_status(&core);
        assert!(!cs.available);
        assert!(
            cs.download_bytes > 1_000_000_000,
            "default captioner + engine to download"
        );
    }

    #[test]
    fn engine_output_keeps_the_memory_plan_in_view() {
        let (_tmp, core, _rec) = new_core();
        assert_eq!(crate::engine_setup::engine_output(&core), "");
        let plan = "[INFO ] backend_fit.cpp:326  -     CUDA0        NVIDIA GeForce RTX 5070 Ti       free  15010 MiB, budget  14498 MiB";
        let log = engine_log(&core);
        log.push_line(plan);
        log.push_line("[INFO ] main.cpp:149  - listening on: http://127.0.0.1:5000");
        *core.gen.memory_plan.lock() = Some(("m".into(), vec![plan.to_string()]));
        // Still in the ring: shown once, in place.
        let out = crate::engine_setup::engine_output(&core);
        assert_eq!(out.matches("RTX 5070 Ti").count(), 1, "{out}");
        // Scrolled out of the ring: shown first.
        core.gen.logs.clear();
        log.push_line("[INFO ] image.cpp:899  - sampling completed, taking 8.00s");
        let out = crate::engine_setup::engine_output(&core);
        assert!(
            out.starts_with("Memory plan when the engine started:"),
            "{out}"
        );
        assert!(
            out.contains("RTX 5070 Ti") && out.contains("sampling completed"),
            "{out}"
        );
    }
}
