//! Running out of memory: retries with memory-saving launch choices and their messages.

use super::*;

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
    assert_eq!(err.message, crate::memory::TE_ON_GPU_MESSAGE);
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
    assert_eq!(err.message, crate::memory::RAM_MESSAGE);
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
        details.starts_with("Memory plan of the earlier attempt") && details.contains(&plan_line),
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
        .position(|n| n == crate::memory::TE_RETRY_NOTE)
        .expect("text encoder retry note");
    let room_note = loading_notes
        .iter()
        .position(|n| n == crate::memory::MORE_ROOM_RETRY_NOTE)
        .expect("more room retry note");
    let offload_note = loading_notes
        .iter()
        .position(|n| n == crate::memory::OFFLOAD_RETRY_NOTE)
        .expect("offload retry note");
    assert!(
        te_note < room_note && room_note < offload_note,
        "{loading_notes:?}"
    );
    // The retry succeeded: its choices are remembered for the model.
    let remembered = core.gen.mem_fallback.lock().get(&model).copied();
    assert!(
        remembered.is_some_and(|fb| fb.te_on_cpu && fb.vram_reserve_gib == 4),
        "{remembered:?}"
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
    crate::engine::unload_model(
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
    crate::engine::unload_model(&core, "other", &[shared.into()]).await;
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
    crate::engine::unload_model(&core, "a-lora", &[lora]).await;
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
    let off = crate::memory::MemFallback {
        offload: true,
        ..Default::default()
    };
    *core.gen.offloaded.lock() = Some((
        model.clone(),
        crate::memory::with_memory_choices(&wiring, off),
    ));
    assert!(
        crate::memory::with_remembered_offload(&core, &model, &wiring, Default::default(), true)
            .offload
    );
    assert!(
        !crate::memory::with_remembered_offload(&core, "other", &wiring, Default::default(), true)
            .offload
    );
    assert!(
        !crate::memory::with_remembered_offload(&core, &model, &wiring, Default::default(), false)
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
    crate::engine::unload_model(&core, &model, &[]).await;
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
    crate::memory::set_others_note(&core, Some(crate::memory::others_note(&others(9216))));
    crate::memory::set_retry_note(&core, crate::memory::TE_RETRY_NOTE);
    crate::memory::set_others_note(&core, Some(crate::memory::others_note(&others(9300))));
    assert_eq!(notes().len(), 2, "{:?}", notes());
    assert!(
        notes()[0].starts_with("Other programs are using 9.1 GB of your graphics memory."),
        "{:?}",
        notes()
    );
    assert_eq!(notes()[1], crate::memory::TE_RETRY_NOTE);
    crate::memory::set_retry_note(&core, crate::memory::TILING_RETRY_NOTE);
    crate::memory::set_others_note(&core, None);
    assert_eq!(
        notes(),
        vec![crate::memory::TILING_RETRY_NOTE.to_string()],
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
    let err = generate::generate(&core, GenerateRequest::txt2img(model.clone(), SENTINEL))
        .await
        .unwrap_err();
    assert_eq!(err.code, "vram");
    assert_eq!(err.message, crate::memory::VRAM_MESSAGE);
    assert!(
        core.gen.mem_fallback.lock().get(&model).is_none(),
        "no retry succeeded: nothing is remembered for the model"
    );
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
    assert_eq!(err.message, crate::memory::RAM_MESSAGE);
    let details = err.details.unwrap_or_default();
    assert!(
        details.contains("failed to allocate") && details.contains("exit code 1"),
        "{details}"
    );
    let _ = leftover.kill();
    let _ = leftover.wait();
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

/// With a GPU engine build, an idle describe engine is stopped before each
/// job and upscale, also when the image engine is already loaded.
#[cfg(unix)]
#[tokio::test]
async fn idle_describe_engine_is_stopped_before_each_job_and_upscale() {
    let (tmp, core, _) = gpu_core();
    let mock = MockSdServer::start_with(MockOptions {
        polls_before_done: 0,
        ..Default::default()
    })
    .await;
    use_external_engine(&core, &mock.base_url());
    let model = register_fake_model(&core, "sdxl");
    install_fake_upscaler(&core);
    let put_llama = || async {
        let proc = fake_engine(tmp.path(), "exec sleep 30");
        *core.describe.slot.lock().await = Some(describe::LlamaSlot::for_tests(proc));
    };

    put_llama().await;
    let res = generate::generate(&core, GenerateRequest::txt2img(model.clone(), "a boat"))
        .await
        .unwrap();
    assert!(core.describe.slot.lock().await.is_none(), "first job");

    put_llama().await;
    generate::generate(&core, GenerateRequest::txt2img(model, "a boat"))
        .await
        .unwrap();
    assert!(
        core.describe.slot.lock().await.is_none(),
        "the loaded image engine was reused"
    );

    put_llama().await;
    generate::upscale_image(&core, &res.images[0].id, 4)
        .await
        .unwrap();
    assert!(core.describe.slot.lock().await.is_none(), "upscale");
}
