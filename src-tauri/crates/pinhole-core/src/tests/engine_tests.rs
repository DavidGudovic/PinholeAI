//! The sd-server process: launch arguments, API key, idle stop, unload and failures.

use super::*;

/// engine.yaml's launch defaults are filtered to tuning flags.
#[tokio::test]
async fn sd_args_keep_only_tuning_launch_defaults() {
    let (_tmp, core, _) = new_core();
    let mut cfg = (*crate::engine_setup::engine_config(&core).unwrap()).clone();
    cfg.stable_diffusion_cpp.launch_defaults = [
        "--embd-dir",
        "/e",
        "--photo-maker",
        "/p",
        "-n",
        "words",
        "--mmap",
        "--log-level",
        "warn",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let wiring = vec!["--model".to_string(), "/m.safetensors".to_string()];
    let args = crate::engine::full_sd_args(&core, &wiring, &cfg);
    for bad in ["--embd-dir", "/e", "--photo-maker", "/p", "-n", "words"] {
        assert!(!args.iter().any(|a| a == bad), "{bad}: {args:?}");
    }
    assert!(args.iter().any(|a| a == "--mmap"), "{args:?}");
    assert!(args
        .windows(2)
        .any(|w| w[0] == "--log-level" && w[1] == "warn"));
    assert!(args
        .windows(2)
        .any(|w| w[0] == "--model" && w[1] == "/m.safetensors"));
}

/// Add-ons are applied during each step (engine.yaml), so weights held in system memory
/// stay mapped from the model file; a model's own `--lora-apply-mode` still wins.
#[tokio::test]
async fn sd_args_apply_add_ons_at_runtime() {
    let (_tmp, core, _) = new_core();
    let cfg = crate::engine_setup::engine_config(&core).unwrap();
    let wiring = vec!["--model".to_string(), "/m.safetensors".to_string()];
    let args = crate::engine::full_sd_args(&core, &wiring, &cfg);
    let modes: Vec<&[String]> = args
        .windows(2)
        .filter(|w| w[0] == "--lora-apply-mode")
        .collect();
    assert_eq!(modes.len(), 1, "{args:?}");
    assert_eq!(modes[0][1], "at_runtime", "{args:?}");

    let own: Vec<String> = ["--model", "/m.safetensors", "--lora-apply-mode", "immediately"]
        .map(String::from)
        .to_vec();
    let args = crate::engine::full_sd_args(&core, &own, &cfg);
    let modes: Vec<&String> = args
        .windows(2)
        .filter(|w| w[0] == "--lora-apply-mode")
        .map(|w| &w[1])
        .collect();
    assert_eq!(modes, vec!["immediately"], "{args:?}");
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
    let args = crate::engine::full_sd_args(&core, &wiring, &cfg);
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
    let wiring_loud: Vec<String> = ["--model", "/m.safetensors", "-v", "--log-level", "verbose"]
        .map(String::from)
        .to_vec();
    let args = crate::engine::full_sd_args(&core, &wiring_loud, &loud);
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

#[tokio::test]
async fn api_key_goes_in_the_bearer_header() {
    use pinhole_engine::testutil::{MockOptions, MockSdServer};
    let mock = MockSdServer::start_with(MockOptions {
        api_key: Some("k3y".into()),
        ..MockOptions::default()
    })
    .await;
    let without = pinhole_engine::SdClient::new_plain_for_tests(mock.base_url());
    assert!(!without.is_ready().await);
    assert!(without.capabilities().await.is_err());
    let wrong = pinhole_engine::SdClient::new_plain_for_tests(mock.base_url()).with_api_key("nope");
    assert!(!wrong.is_ready().await);
    let with = pinhole_engine::SdClient::new_plain_for_tests(mock.base_url()).with_api_key("k3y");
    assert!(with.is_ready().await);
    let id = with
        .submit(&pinhole_engine::ImgGenRequest::new(
            pinhole_engine::words::CheckedPrompt::check("a cat").unwrap(),
            64,
            64,
            1,
        ))
        .await
        .unwrap();
    assert!(with.job(&id).await.is_ok());
    assert!(
        !format!("{with:?}").contains("k3y"),
        "Debug leaves out the key"
    );
}

/// A Pinhole-started sd-server gets a fresh key in its environment, never
/// on its command line (a stand-in engine records both, then exits).
#[cfg(unix)]
#[tokio::test]
async fn managed_engine_gets_its_api_key_in_the_environment() {
    use pinhole_engine::install::{self, EngineKind, InstallMarker};
    use std::os::unix::fs::PermissionsExt;
    let (tmp, core, _) = new_core();
    let (cfg, sel) = crate::engine_setup::selected_build(&core, EngineKind::Sd).unwrap();
    let dir = install::install_dir(
        &core.data.engine(),
        EngineKind::Sd,
        &cfg.stable_diffusion_cpp.version,
        &sel.backend,
    );
    std::fs::create_dir_all(&dir).unwrap();
    let seen = tmp.path().join("seen.txt");
    let exe = dir.join("sd-server");
    std::fs::write(
        &exe,
        format!(
            "#!/bin/sh\necho \"key=$SD_API_KEY\" > '{0}'\necho \"args=$*\" >> '{0}'\nexit 1\n",
            seen.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let marker = InstallMarker {
        engine: EngineKind::Sd,
        version: cfg.stable_diffusion_cpp.version.clone(),
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
    let model = register_fake_model(&core, "sd15");
    assert!(
        generate::generate(&core, GenerateRequest::txt2img(model, "a cat"))
            .await
            .is_err()
    );
    let text = std::fs::read_to_string(&seen).unwrap();
    let key = text.lines().find_map(|l| l.strip_prefix("key=")).unwrap();
    assert_eq!(key.len(), 64, "{text}");
    assert!(key.chars().all(|c| c.is_ascii_hexdigit()), "{text}");
    let args = text.lines().find_map(|l| l.strip_prefix("args=")).unwrap();
    assert!(!args.contains(key) && !args.contains("--api-key"), "{args}");
    // The launched engine also gets the lock-down flag and keeps the pinned tuning flags.
    let argv: Vec<&str> = args.split_whitespace().collect();
    assert_eq!(
        argv.contains(&pinhole_engine::sdapi::REJECT_ORIGIN_FLAG),
        crate::engine::ENGINE_LOCKDOWN,
        "{args}"
    );
    assert!(argv.contains(&"--disable-image-metadata"), "{args}");
}

#[tokio::test]
async fn sd_args_keep_the_api_key_off_the_command_line() {
    let (_tmp, core, _) = new_core();
    let mut cfg = (*crate::engine_setup::engine_config(&core).unwrap()).clone();
    cfg.stable_diffusion_cpp
        .launch_defaults
        .extend(["--api-key", "from-yaml", "--reject-origin"].map(String::from));
    let wiring: Vec<String> = ["--model", "/m.safetensors", "--api-key", "from-wiring"]
        .map(String::from)
        .to_vec();
    let args = crate::engine::full_sd_args(&core, &wiring, &cfg);
    assert!(
        !args
            .iter()
            .any(|a| a == "--api-key" || a.starts_with("from-")),
        "{args:?}"
    );
    // The lock-down flag comes from the compiled-in switch only, once.
    assert_eq!(
        args.iter()
            .filter(|a| *a == pinhole_engine::sdapi::REJECT_ORIGIN_FLAG)
            .count(),
        usize::from(crate::engine::ENGINE_LOCKDOWN),
        "{args:?}"
    );
}

/// Drives the REAL sd-server (Linux) through `generate` when
/// `PINHOLE_SD_ARCHIVE` points at the pinned `…-bin-Linux-Ubuntu-24.04-x86_64-cpu.zip`:
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
    let exe = crate::engine_setup::installed_engine(&core, pinhole_engine::install::EngineKind::Sd)
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
    crate::engine::arm_idle_stop(&core, epoch);
    core.gen.activity.fetch_add(1, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        engine_running(&core).await,
        "a newer job started: keep the engine"
    );
    crate::engine::arm_idle_stop(&core, core.gen.activity.load(Ordering::SeqCst));
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
    crate::engine::after_job(&core, core.gen.activity.load(Ordering::SeqCst)).await;
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
    let err = crate::engine::verify_engine_identity(&mut ours, &client, &args)
        .await
        .unwrap_err();
    assert_eq!(
        (err.code.as_str(), err.message.as_str()),
        ("engine_failed", crate::engine::PORT_TAKEN_MESSAGE)
    );
    let args = vec![
        "--diffusion-model".to_string(),
        "/mock/mock.safetensors".to_string(),
    ];
    crate::engine::verify_engine_identity(&mut ours, &client, &args)
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
    let err = crate::engine::verify_engine_identity(&mut dead, &client, &args)
        .await
        .unwrap_err();
    assert_eq!(
        err.message,
        crate::engine::PORT_TAKEN_MESSAGE,
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
    assert_eq!(err.message, crate::engine::PORT_TAKEN_MESSAGE);
    let anon = pinhole_engine::llama::LlamaClient::new(core.local.clone(), llama.base_url());
    assert!(
        describe::verify_llama_identity(&mut ours, &anon, model)
            .await
            .is_err(),
        "no key → 401 → not ours"
    );
    ours.stop().await;
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
    let task =
        tokio::spawn(
            async move { generate::generate(&c2, GenerateRequest::txt2img(m2, "x")).await },
        );
    for _ in 0..500 {
        if core.gen.loading.lock().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(core.gen.loading.lock().is_some(), "the engine is loading");
    tokio::time::timeout(
        Duration::from_secs(10),
        crate::engine::unload_model(&core, &model, &[]),
    )
    .await
    .expect("unload doesn't wait for the load");
    assert_eq!(task.await.unwrap().unwrap_err().code, "cancelled");
    assert!(core.gen.slot.lock().await.proc.is_none());
}

/// Deleting a LoRA (or the loaded model) while a job runs is refused: it
/// would stop the engine under the job, which then fails like a crash.
/// Deleting a model the engine doesn't use still works.
#[cfg(unix)]
#[tokio::test]
async fn delete_during_a_job_does_not_stop_the_engine() {
    let (tmp, core, _rec) = new_core();
    let lora = register_fake_lora(&core, "sd15", &[]);
    let lora_path = {
        let idx = core.installed.lock();
        idx.abs_path(&core.data, idx.get(&lora).unwrap())
    };
    put_engine(&core, fake_engine(tmp.path(), "exec sleep 30"), false).await;
    *core.gen.active.lock() = Some(tokio_util::sync::CancellationToken::new());
    let e = crate::models::delete_model(&core, &lora).await.unwrap_err();
    assert_eq!(e.code, "invalid");
    assert!(e.message.contains("Wait for the current pictures"), "{e:?}");
    assert!(
        engine_running(&core).await,
        "the job's engine keeps running"
    );
    assert!(lora_path.exists());
    assert!(core.installed.lock().get(&lora).is_some());
    let other = register_fake_model(&core, "sd15");
    crate::models::delete_model(&core, &other).await.unwrap();
    assert!(engine_running(&core).await);
    // No job: the LoRA is deleted (and the engine stopped, see unload_model).
    *core.gen.active.lock() = None;
    crate::models::delete_model(&core, &lora).await.unwrap();
    assert!(!lora_path.exists());
    assert!(!engine_running(&core).await);
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
    let task =
        tokio::spawn(
            async move { generate::generate(&c2, GenerateRequest::txt2img(m2, "x")).await },
        );
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
    crate::engine::unload_model(&core, &model, &[]).await;
    assert_eq!(task.await.unwrap().unwrap_err().code, "cancelled");
    assert!(!spawned.exists(), "no engine started after the cancel");
    assert!(core.gen.loading.lock().is_none());
}

/// Deleting a model the loading engine doesn't use goes through without waiting for the
/// load; a LoRA is still refused while the job runs.
#[cfg(unix)]
#[tokio::test]
async fn deleting_another_model_during_a_load_does_not_wait_for_it() {
    let (_tmp, core, _rec) = new_core();
    install_fake_engine(
        &core,
        pinhole_engine::install::EngineKind::Sd,
        "sd-server",
        "exec sleep 30",
    );
    let model = register_fake_model(&core, "sd15");
    let other = register_fake_model(&core, "sd15");
    let lora = register_fake_lora(&core, "sd15", &[]);
    let c2 = core.clone();
    let m2 = model.clone();
    let task =
        tokio::spawn(
            async move { generate::generate(&c2, GenerateRequest::txt2img(m2, "x")).await },
        );
    for _ in 0..500 {
        if core.gen.flags.lock().loading {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(core.gen.flags.lock().loading, "the engine is loading");
    tokio::time::timeout(
        Duration::from_secs(5),
        crate::models::delete_model(&core, &other),
    )
    .await
    .expect("the delete doesn't wait for the load")
    .unwrap();
    assert!(core.installed.lock().get(&other).is_none());
    let e = tokio::time::timeout(
        Duration::from_secs(5),
        crate::models::delete_model(&core, &lora),
    )
    .await
    .expect("the delete doesn't wait for the load")
    .unwrap_err();
    assert!(e.message.contains("Wait for the current pictures"), "{e:?}");
    assert!(core.installed.lock().get(&lora).is_some());
    assert!(core.gen.flags.lock().loading, "the load goes on");
    core.gen.active.lock().as_ref().unwrap().cancel();
    assert_eq!(task.await.unwrap().unwrap_err().code, "cancelled");
}

/// A request stopped by the word check before reaching the engine keeps the loaded
/// engine; a picture the image check drops after the engine made it stops the engine.
#[cfg(unix)]
#[tokio::test]
async fn engine_stays_loaded_when_a_request_stops_before_reaching_it() {
    let (tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let model = register_fake_model(&core, "sdxl");
    // Stands in for a Pinhole-started engine that holds an earlier job's results.
    put_engine(&core, fake_engine(tmp.path(), "exec sleep 30"), true).await;

    // The combined prompt (with the Style) is stopped by the word check.
    let style = crate::library::save_style(
        &core,
        pinhole_store::styles::Style {
            id: String::new(),
            name: "Check".into(),
            positive: "a valid driver's license from Ohio".into(),
            negative: None,
            families: vec![],
            thumbnail: None,
            builtin: false,
        },
    )
    .unwrap();
    let mut req = GenerateRequest::txt2img(model.clone(), "a desk");
    req.style_id = Some(style.id);
    let e = generate::generate(&core, req).await.unwrap_err();
    assert_eq!(e.code, "blocked");
    assert!(mock.requests().is_empty(), "nothing reached the engine");
    assert!(engine_running(&core).await, "the loaded engine is kept");

    // The image check drops a picture the engine made: the engine is stopped.
    let mut readings = intimate_adult();
    readings.tags.as_mut().unwrap().minor = 0.9;
    use_check(
        &core,
        FakeCheck {
            readings,
            ..Default::default()
        },
    );
    let e = generate::generate(&core, GenerateRequest::txt2img(model, "a boat"))
        .await
        .unwrap_err();
    assert_eq!(e.code, "blocked");
    assert_eq!(mock.requests().len(), 1);
    assert!(!engine_running(&core).await, "the engine is stopped");
}
