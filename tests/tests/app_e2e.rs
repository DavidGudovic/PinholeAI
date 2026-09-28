//! End-to-end app test without the UI: the exact path a user takes after the
//! first run, driven through the core service layer the Tauri commands call.
//!
//!   1. install the engine through `engine_setup::install_engine` (CPU build,
//!      DownloadManager, SHA-256 check, unpack into `Data/engine/`);
//!   2. "Add a file I already have" with the SD 1.5 test model
//!      (`models::add_local_model`: header detection / known hash → family, copy
//!      into `Data/models/`, register in installed.json);
//!   3. `generate::generate` with the simple dials (registry wiring → sd-server
//!      launch args → `/sdcpp/v1/img_gen` → poll → scrub → in-memory session);
//!   4. `session::save_image` → `Data/outputs/pinhole_*.png`;
//!   5. scan `Data/` for the prompt sentinel.
//!
//! Skipped unless `PINHOLE_SMOKE=1` (needs internet; the model comes from the
//! same cache as `engine_smoke`).
//!
//!     PINHOLE_SMOKE=1 cargo test -p pinhole-tests --test app_e2e --release -- --nocapture

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use pinhole_core::generate::GenerateRequest;
use pinhole_core::{AppCore, NullSink, ShippedPaths};
use pinhole_store::DataDir;
use pinhole_tests::{config_dir, describe_hits, png_dimensions, png_text_chunks, repo_root, scan_for, smoke, unique_suffix, PNG_SIGNATURE};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn app_e2e() {
    if std::env::var("PINHOLE_SMOKE").ok().as_deref() != Some("1") {
        eprintln!("app_e2e: skipped (set PINHOLE_SMOKE=1)");
        return;
    }
    let t0 = Instant::now();
    let cache = std::env::var_os("PINHOLE_SMOKE_CACHE")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("target").join("smoke-cache"));
    std::fs::create_dir_all(&cache).expect("create smoke cache");

    // The model file the user "already has".
    let http = smoke::http_client();
    let model = smoke::smoke_model(&config_dir());
    let model_path = smoke::ensure_model(&http, &cache, &model).await.unwrap_or_else(|e| panic!("{e}"));
    eprintln!("app_e2e: model file ready ({:.0}s)", t0.elapsed().as_secs_f32());

    let tmp = tempfile::tempdir().expect("tempdir");
    let data = DataDir::at(tmp.path().join("Data"), true);
    let core: Arc<AppCore> =
        AppCore::new(ShippedPaths { config_dir: config_dir() }, data, Arc::new(NullSink)).expect("AppCore::new");
    core.start_background();

    // CPU engine (CI runners have no GPU).
    let mut settings = pinhole_core::app::get_settings(&core);
    settings.engine_backend = "cpu".into();
    pinhole_core::app::set_settings(&core, settings).expect("set_settings");

    // 1. Engine install.
    let status = pinhole_core::engine_setup::install_engine(&core).await.unwrap_or_else(|e| panic!("install_engine: {} ({:?})", e.message, e.details));
    assert!(status.installed, "engine not installed after install_engine");
    eprintln!("app_e2e: engine {:?} installed ({:.0}s)", status.version, t0.elapsed().as_secs_f32());

    // 2. Add the model file.
    let added = pinhole_core::models::add_local_model(&core, &model_path.to_string_lossy())
        .await
        .unwrap_or_else(|e| panic!("add_local_model: {}", e.message));
    let installed = match (added.model, added.needs_choice) {
        (Some(m), _) => m,
        (None, Some(choice)) => {
            eprintln!("app_e2e: family is ambiguous ({:?}); picking sd15", choice.candidates);
            pinhole_core::models::confirm_family(&core, &choice.token, "sd15")
                .unwrap_or_else(|e| panic!("confirm_family: {}", e.message))
                .model
                .expect("model after confirm_family")
        }
        (None, None) => panic!("add_local_model returned neither a model nor a choice"),
    };
    assert_eq!(installed.family_id.as_deref(), Some("sd15"), "SD 1.5 file detected as the wrong family");
    assert!(installed.missing_components.is_empty(), "SD 1.5 needs no extra components: {:?}", installed.missing_components);
    eprintln!("app_e2e: model registered as {} ({})", installed.id, installed.friendly_name);

    // 3. Generate through the real wiring + sd-server.
    let sentinel = format!("PINHOLE_SENTINEL_7f3a_{}", unique_suffix());
    let mut req = GenerateRequest::txt2img(installed.id.clone(), format!("a red apple on a wooden table, {sentinel}"));
    req.fine_tune.width = Some(256);
    req.fine_tune.height = Some(256);
    req.fine_tune.steps = Some(4);
    req.fine_tune.seed = Some(42);
    let result = pinhole_core::generate::generate(&core, req)
        .await
        .unwrap_or_else(|e| panic!("generate: {}\n--- details ---\n{}", e.message, e.details.unwrap_or_default()));
    assert_eq!(result.images.len(), 1);
    let img = &result.images[0];
    assert_eq!((img.width, img.height), (256, 256));
    assert_eq!(img.seed, 42);
    let bytes = pinhole_core::session::get(&core, &img.id).expect("image in session");
    assert!(bytes.starts_with(&PNG_SIGNATURE), "result is not a PNG");
    assert_eq!(png_dimensions(&bytes), Some((256, 256)));
    assert!(png_text_chunks(&bytes).is_empty(), "generated PNG still has text chunks");
    eprintln!("app_e2e: generated in {:.0}s total", t0.elapsed().as_secs_f32());

    // Nothing on disk before Save.
    let outputs = core.data.outputs();
    assert!(std::fs::read_dir(&outputs).map(|d| d.count()).unwrap_or(0) == 0, "outputs written before Save");

    // 4. Save.
    let saved = pinhole_core::session::save_image(&core, &img.id).unwrap_or_else(|e| panic!("save_image: {}", e.message));
    let saved_path = PathBuf::from(&saved.path);
    let name = saved_path.file_name().unwrap().to_string_lossy().to_string();
    assert!(name.starts_with("pinhole_") && name.ends_with("_42.png"), "unexpected file name {name}");
    let on_disk = std::fs::read(&saved_path).expect("read saved image");
    assert!(on_disk.starts_with(&PNG_SIGNATURE));

    // 5. The prompt must be nowhere in Data/.
    core.shutdown().await;
    let hits = scan_for(tmp.path(), &[&sentinel]);
    assert!(hits.is_empty(), "prompt sentinel found on disk:\n{}", describe_hits(&hits));

    if let Some(out) = std::env::var_os("PINHOLE_SMOKE_OUT").filter(|v| !v.is_empty()) {
        let out = PathBuf::from(out);
        let _ = std::fs::create_dir_all(&out);
        let _ = std::fs::write(out.join(format!("app-e2e-{}.png", std::env::consts::OS)), &on_disk);
    }
    eprintln!("app_e2e: OK in {:.0}s", t0.elapsed().as_secs_f32());
}
