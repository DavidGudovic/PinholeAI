//! Engine smoke test: download the pinned CPU sd-server
//! for this OS and the smallest registered model (`test_models.sd15`, SD 1.5 fp16),
//! launch sd-server on 127.0.0.1, generate one 256×256 image with 4 steps, and
//! check the PNG (magic, size, not blank, no prompt in text chunks).
//!
//! Skipped unless `PINHOLE_SMOKE=1` (needs internet + ~2.2 GB).
//! Cache: `PINHOLE_SMOKE_CACHE` (default `target/smoke-cache`).
//! Optional: `PINHOLE_SMOKE_OUT=<dir>` writes the image there (CI uploads it).
//!
//!     PINHOLE_SMOKE=1 cargo test -p pinhole-tests --test engine_smoke --release -- --nocapture

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pinhole_tests::{
    config_dir, png_dimensions, png_text_chunks, repo_root, smoke, unique_suffix, PNG_SIGNATURE,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn engine_smoke() {
    if std::env::var("PINHOLE_SMOKE").ok().as_deref() != Some("1") {
        eprintln!("engine_smoke: skipped (set PINHOLE_SMOKE=1 to download sd-server + SD 1.5 and generate one image)");
        return;
    }
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    if let Some(v) = pinhole_engine::pins::glibc_version() {
        assert!(
            pinhole_engine::pins::version_at_least(&v, "2.38"),
            "glibc {v} is too old for the upstream Linux sd-server builds (Ubuntu 24.04+ / glibc 2.38+ required)"
        );
    }

    let t0 = Instant::now();
    let cache = std::env::var_os("PINHOLE_SMOKE_CACHE")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("target").join("smoke-cache"));
    std::fs::create_dir_all(&cache).expect("create smoke cache");
    eprintln!("engine_smoke: cache {}", cache.display());

    let cfg = smoke::engine_config(&config_dir());
    let http = smoke::http_client();

    let exe = smoke::install_cpu_engine(&http, &cfg, &cache)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    eprintln!(
        "engine_smoke: sd-server at {} ({:.0}s)",
        exe.display(),
        t0.elapsed().as_secs_f32()
    );

    let model = smoke::smoke_model(&config_dir());
    eprintln!("engine_smoke: model {} ({})", model.file, model.url);
    let model_path = smoke::ensure_model(&http, &cache, &model)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    eprintln!(
        "engine_smoke: model ready ({:.0}s)",
        t0.elapsed().as_secs_f32()
    );

    let engine = smoke::launch(&cfg, &exe, &model_path, Duration::from_secs(900))
        .await
        .unwrap_or_else(|e| panic!("{e}"));

    let sentinel = format!("PINHOLE_SENTINEL_7f3a_{}", unique_suffix());
    let prompt = format!("a red apple on a wooden table, studio photo, {sentinel}");
    let negative = format!("blurry, NEG_SENTINEL_5b1e_{}", unique_suffix());
    let result = smoke::txt2img(
        &engine,
        &prompt,
        &negative,
        (256, 256),
        4,
        42,
        Duration::from_secs(1800),
    )
    .await;
    let log_tail = engine.logs().tail_text(400);
    smoke::stop(engine).await;
    let images = result.unwrap_or_else(|e| panic!("{e}"));

    assert_eq!(images.len(), 1, "expected exactly one image");
    let png = &images[0];
    assert!(
        png.len() > 1000 && png[..8] == PNG_SIGNATURE,
        "engine did not return a PNG ({} bytes)",
        png.len()
    );
    assert_eq!(
        png_dimensions(png),
        Some((256, 256)),
        "unexpected image size"
    );

    // embed_image_metadata:false → no chunk may carry the prompt (or any generation parameters)
    for (kind, keyword, text) in png_text_chunks(png) {
        let text = String::from_utf8_lossy(&text);
        assert!(
            !text.contains(&sentinel) && !keyword.contains(&sentinel),
            "PNG {kind} chunk `{keyword}` contains the prompt — embed_image_metadata must be false"
        );
        eprintln!(
            "engine_smoke: note: PNG has a {kind} chunk `{keyword}` ({} bytes)",
            text.len()
        );
    }
    // The engine output ring buffer must have redacted the prompt.
    assert!(
        !log_tail.contains(&sentinel),
        "engine log buffer contains the prompt (redaction failed)"
    );

    // Not a blank/NaN image: decode and require some pixel variation.
    let decoder = png::Decoder::new(std::io::Cursor::new(png.as_slice()));
    let mut reader = decoder.read_info().expect("PNG decodes");
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("PNG frame decodes");
    let pixels = &buf[..info.buffer_size()];
    let (min, max) = pixels
        .iter()
        .fold((255u8, 0u8), |(lo, hi), &p| (lo.min(p), hi.max(p)));
    assert!(
        max.saturating_sub(min) > 16,
        "image is blank (all pixels within {min}..={max}) — VAE/NaN problem?"
    );

    if let Some(out) = std::env::var_os("PINHOLE_SMOKE_OUT").filter(|v| !v.is_empty()) {
        let out = PathBuf::from(out);
        std::fs::create_dir_all(&out).expect("create PINHOLE_SMOKE_OUT");
        let file = out.join(format!("smoke-{}.png", std::env::consts::OS));
        std::fs::write(&file, png).expect("write smoke image");
        eprintln!("engine_smoke: wrote {}", file.display());
    }
    eprintln!("engine_smoke: OK in {:.0}s", t0.elapsed().as_secs_f32());
}
