//! Privacy test (SPEC §4): generate with a sentinel prompt + negative + a saved
//! style (through the real core service layer and a mock sd-server that bakes the
//! prompt into PNG text chunks, like sd-server's default), save images, save a
//! preset, then scan every byte of the Data folder (incl. decompressed PNG text
//! chunks) and the OS temp dir's Pinhole entries:
//!   * the prompt and negative sentinels must appear nowhere;
//!   * the style sentinel may appear only under `Data/styles/`.
//!
//! Also checks the engine request: `embed_image_metadata: false` and the prompt +
//! style were combined in memory and actually reached the engine.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pinhole_core::{AppCore, NullSink, ShippedPaths};
use pinhole_engine::testutil::MockSdServer;
use pinhole_store::DataDir;
use pinhole_tests::{
    config_dir, describe_hits, pinhole_temp_entries, scan_for, unique_suffix, Hit,
};
use serde_json::{json, Value};

struct Sentinels {
    prompt: String,
    negative: String,
    style: String,
    style_negative: String,
}

impl Sentinels {
    fn new() -> Self {
        let s = unique_suffix();
        Self {
            prompt: format!("PINHOLE_SENTINEL_7f3a_{s}"),
            negative: format!("NEG_SENTINEL_5b1e_{s}"),
            style: format!("STYLE_SENTINEL_91c2_{s}"),
            style_negative: format!("STYLENEG_SENTINEL_d44e_{s}"),
        }
    }
}

// ADAPT: the request mirrors src/lib/types.ts `GenerateRequest` (serde camelCase);
// built from JSON so the test does not depend on the Rust struct layout.
fn generate_request(model_id: &str, style_id: &str, s: &Sentinels, seed: i64) -> Value {
    json!({
        "modelId": model_id,
        "mode": "txt2img",
        "prompt": format!("a lighthouse at dusk, {}", s.prompt),
        "styleId": style_id,
        "dials": { "shape": "square", "quality": "fast", "stick": 0.5, "count": 1 },
        "fineTune": {
            "negativePrompt": format!("lowres, {}", s.negative),
            "seed": seed,
            "steps": 4,
            "width": 256,
            "height": 256
        },
        "loras": [],
        "addTriggerWords": false
    })
}

fn is_under(path: &Path, dir: &Path) -> bool {
    path.starts_with(dir)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sentinel_prompt_never_reaches_disk() {
    let s = Sentinels::new();

    // Temp Data folder whose name contains "pinhole", so the temp-dir scan covers it too.
    let tmp = tempfile::Builder::new()
        .prefix("pinhole-privacy-")
        .tempdir()
        .expect("tempdir");
    let data_root = tmp.path().join("Data");
    // Save writes outside Data: `Pictures/Pinhole`, like an installed copy.
    let pictures = tmp.path().join("Pictures").join("Pinhole");
    std::env::set_var("PINHOLE_DATA_DIR", &data_root);

    let core: Arc<AppCore> = AppCore::new(
        ShippedPaths {
            config_dir: config_dir(),
        },
        DataDir::at(data_root.clone(), false).with_pictures_home(Some(pictures.clone())),
        Arc::new(NullSink),
    )
    .unwrap_or_else(|e| panic!("AppCore::new failed: {} ({:?})", e.message, e.details));

    // ---- engine: mock sd-server that embeds the prompt into tEXt/iTXt chunks
    let mock = MockSdServer::start().await;
    pinhole_core::testing::use_external_engine(&core, &mock.base_url());
    let model_id = pinhole_core::testing::register_fake_model(&core, "sdxl");

    // ---- explicit "Save as style" (the only user text allowed on disk)
    let style = pinhole_core::library::save_style(
        &core,
        serde_json::from_value(json!({
            "id": "",
            "name": "Privacy test style",
            "positive": format!("35mm film, soft light, {}", s.style),
            "negative": format!("plastic skin, {}", s.style_negative),
            "families": [],
            "thumbnail": null,
            "builtin": false
        }))
        .expect("Style JSON"),
    )
    .unwrap_or_else(|e| panic!("save_style failed: {}", e.message));
    let style_json = serde_json::to_value(&style).expect("Style serializes");
    let style_id = style_json["id"].as_str().expect("style id").to_string();
    assert!(!style_id.is_empty(), "saved style must get an id");

    // ---- generate #1 (saved-image metadata: none)
    let result = pinhole_core::generate::generate(
        &core,
        serde_json::from_value(generate_request(&model_id, &style_id, &s, 1234))
            .expect("GenerateRequest JSON"),
    )
    .await
    .unwrap_or_else(|e| panic!("generate failed: {} ({:?})", e.message, e.details));
    let result = serde_json::to_value(&result).expect("GenerateResult serializes");
    let image_id = result["images"][0]["id"]
        .as_str()
        .expect("result image id")
        .to_string();

    // The engine got exactly what we expect.
    let bodies = mock.requests();
    assert!(
        !bodies.is_empty(),
        "the mock sd-server received no img_gen request"
    );
    for body in &bodies {
        assert_eq!(
            body.get("embed_image_metadata"),
            Some(&json!(false)),
            "img_gen must send \"embed_image_metadata\": false"
        );
        assert!(
            body.get("lora").is_none_or(|l| l.is_array()),
            "LoRAs go in the structured `lora` array"
        );
    }
    let prompt_sent = bodies[0]["prompt"].as_str().unwrap_or_default();
    assert!(
        prompt_sent.contains(&s.prompt),
        "the prompt did not reach the engine"
    );
    assert!(
        prompt_sent.contains(&s.style),
        "the style was not combined into the prompt sent to the engine"
    );
    let negative_sent = bodies[0]["negative_prompt"].as_str().unwrap_or_default();
    assert!(
        negative_sent.contains(&s.negative),
        "the negative prompt did not reach the engine (sdxl uses negatives)"
    );
    assert!(
        negative_sent.contains(&s.style_negative),
        "the style's negative was not appended to the negative prompt"
    );
    assert!(
        !prompt_sent.contains("<lora:"),
        "<lora:…> prompt tags are not supported by sd-server"
    );

    // ---- Save (writes Pictures/Pinhole/pinhole_<date>_<time>_<seed>.png)
    let saved = pinhole_core::session::save_image(&core, &image_id)
        .unwrap_or_else(|e| panic!("save_image failed: {}", e.message));
    let saved_path = PathBuf::from(
        serde_json::to_value(&saved).expect("SavedImage serializes")["path"]
            .as_str()
            .expect("saved path")
            .to_string(),
    );
    check_saved_file(&saved_path, &pictures);

    // ---- generate #2 + save with "Include generation settings (no prompt)"
    core.settings.write().saved_metadata = "settings".to_string();
    let result2 = pinhole_core::generate::generate(
        &core,
        serde_json::from_value(generate_request(&model_id, &style_id, &s, 5678))
            .expect("GenerateRequest JSON"),
    )
    .await
    .unwrap_or_else(|e| panic!("generate #2 failed: {}", e.message));
    let result2 = serde_json::to_value(&result2).expect("GenerateResult serializes");
    let image_id2 = result2["images"][0]["id"]
        .as_str()
        .expect("result image id")
        .to_string();
    let saved2 = pinhole_core::session::save_image(&core, &image_id2)
        .unwrap_or_else(|e| panic!("save_image #2 failed: {}", e.message));
    let saved_path2 = PathBuf::from(
        serde_json::to_value(&saved2).expect("SavedImage serializes")["path"]
            .as_str()
            .expect("saved path")
            .to_string(),
    );
    check_saved_file(&saved_path2, &pictures);

    // ---- Save as preset (model + style reference + dials; never the prompt)
    let _preset = pinhole_core::library::save_preset(
        &core,
        serde_json::from_value(json!({
            "id": "",
            "name": "Privacy test preset",
            "family": "sdxl",
            "modelId": model_id,
            "civitaiVersionId": null,
            "styleId": style_id,
            "shape": "square",
            "quality": "fast",
            "stick": 0.5,
            "count": 1,
            "fineTune": { "seed": 1234, "steps": 4, "width": 256, "height": 256 },
            "loras": [],
            "builtin": false
        }))
        .expect("Preset JSON"),
    )
    .unwrap_or_else(|e| panic!("save_preset failed: {}", e.message));

    // Stop engines / flush state, then scan.
    core.shutdown().await;
    drop(core);
    drop(mock);

    let forbidden = [s.prompt.as_str(), s.negative.as_str()];
    let styles_dir = data_root.join("styles");

    // 1) Data folder and the Save folder
    assert!(
        pictures.is_dir(),
        "Pictures/Pinhole should exist after saving"
    );
    let saved_hits = scan_for(
        &pictures,
        &[
            s.prompt.as_str(),
            s.negative.as_str(),
            s.style.as_str(),
            s.style_negative.as_str(),
        ],
    );
    assert!(
        saved_hits.is_empty(),
        "privacy violation in the Save folder:\n{}",
        describe_hits(&saved_hits)
    );
    let hits = scan_for(
        &data_root,
        &[
            s.prompt.as_str(),
            s.negative.as_str(),
            s.style.as_str(),
            s.style_negative.as_str(),
        ],
    );
    let bad: Vec<Hit> = hits
        .into_iter()
        .filter(|h| {
            forbidden.contains(&h.needle.as_str())
                || h.needle.is_empty()
                || !is_under(&h.path, &styles_dir)
        })
        .collect();
    assert!(
        bad.is_empty(),
        "privacy violation in the Data folder:\n{}",
        describe_hits(&bad)
    );

    // The style must actually be stored (sanity check that the scanner works).
    let style_hits = scan_for(&styles_dir, &[s.style.as_str()]);
    assert!(
        !style_hits.is_empty(),
        "the saved style was not found under Data/styles (scanner or save_style broken)"
    );

    // 2) OS temp dir: every entry named *pinhole* (includes this test's own folder)
    let mut bad = Vec::new();
    for entry in pinhole_temp_entries() {
        for h in scan_for(
            &entry,
            &[
                s.prompt.as_str(),
                s.negative.as_str(),
                s.style.as_str(),
                s.style_negative.as_str(),
            ],
        ) {
            let allowed_style = !forbidden.contains(&h.needle.as_str())
                && !h.needle.is_empty()
                && is_under(&h.path, &styles_dir);
            if !allowed_style {
                bad.push(h);
            }
        }
    }
    assert!(
        bad.is_empty(),
        "privacy violation in the OS temp dir:\n{}",
        describe_hits(&bad)
    );
}

fn check_saved_file(path: &Path, save_folder: &Path) {
    assert!(path.is_file(), "saved image missing: {}", path.display());
    assert!(
        path.starts_with(save_folder),
        "saved image must be in the Save folder, got {}",
        path.display()
    );
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    assert!(
        name.starts_with("pinhole_") && name.ends_with(".png"),
        "saved file name must be pinhole_YYYYMMDD_HHMMSS_<seed>.png, got {name}"
    );
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(
        &bytes[..8],
        &pinhole_tests::PNG_SIGNATURE,
        "saved file is not a PNG"
    );
    let mut marker = false;
    for (kind, keyword, text) in pinhole_tests::png_text_chunks(&bytes) {
        if kind == "iTXt" && keyword == "XML:com.adobe.xmp" {
            // The AI-generated marker (RELEASE-SPEC §2): always there, app name only.
            let text = String::from_utf8_lossy(&text);
            assert!(text.contains("DigitalSourceType"), "{text}");
            marker = true;
            continue;
        }
        assert!(
            kind == "tEXt" && keyword == "pinhole",
            "saved PNG may only carry the AI marker and the `pinhole` settings chunk, found {kind} `{keyword}`"
        );
    }
    assert!(marker, "saved PNG must carry the AI-generated marker");
}
