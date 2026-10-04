//! Describe and "Improve my prompt" against `MockLlamaServer`.

use super::*;

/// A second download of the describe engine that finishes after the first one
/// installed it: the installed engine is kept and the extra archives are removed.
#[cfg(unix)]
#[tokio::test]
async fn unpacking_an_engine_that_is_already_installed_keeps_it() {
    use pinhole_engine::install::{self, EngineKind};
    let (_tmp, core, _rec) = new_core();
    install_fake_engine(&core, EngineKind::Llama, "llama-server", "exit 0");
    let (cfg, sel) = crate::engine_setup::selected_build(&core, EngineKind::Llama).unwrap();
    let before = install::find_installed(
        &core.data.engine(),
        EngineKind::Llama,
        &cfg.llama_cpp.version,
        &sel.backend,
    )
    .unwrap();
    let dl = install::download_dir(&core.data.engine());
    std::fs::create_dir_all(&dl).unwrap();
    let files: Vec<_> = (0..sel.build.archives().len())
        .map(|i| {
            let path = dl.join(format!("archive-{i}.zip"));
            std::fs::write(&path, b"not an archive").unwrap();
            pinhole_net::download::DownloadedFile {
                path,
                sha256: "0".repeat(64),
                size_bytes: 14,
            }
        })
        .collect();
    let paths: Vec<_> = files.iter().map(|f| f.path.clone()).collect();
    let got = crate::engine_setup::unpack_downloaded(&core, EngineKind::Llama, &cfg, &sel, files)
        .await
        .unwrap();
    assert_eq!(got, before);
    assert!(got.dir.join(install::MARKER_FILE).is_file());
    assert!(paths.iter().all(|p| !p.exists()));
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
    assert!(describe::captioner_status(&core, describe::Purpose::Describe).available);
}

#[tokio::test]
async fn improve_prompt_through_mock_llama() {
    let (_tmp, core, _rec) = new_core();
    let llama = MockLlamaServer::start(
        "PROMPT: a red fox standing in the snow\nDETAILS: thick red fur, sks\nPLACE: in snow\nSHOT: -\nSTYLE: photo\nLIGHT: soft light",
        0,
    )
    .await;
    use_external_captioner(&core, &llama.base_url());
    let text = describe::improve_prompt(
        &core,
        "  a red fox  ",
        None,
        &["sks".into()],
        describe::ImproveTarget::Create,
    )
    .await
    .unwrap();
    assert_eq!(
        text.text, "a red fox standing in the snow. Thick red fur. Photo, soft light.",
        "the reworded idea keeps the intent; the form lines follow it"
    );
    assert!(text.note.is_none());
    let body = &llama.requests()[0];
    let system = body
        .pointer("/messages/0/content")
        .and_then(|t| t.as_str())
        .unwrap();
    assert!(system.contains("six lines"), "{system}");
    assert_eq!(
        body.pointer("/messages/2/content").and_then(|t| t.as_str()),
        Some("PROMPT:"),
        "the answer is started with the first label"
    );
    assert!(
        system.contains("safe for work"),
        "Safe mode is on by default"
    );
    assert!(system.contains("sks"), "trigger words are not repeated");
    assert_eq!(
        body.pointer("/messages/1/content").and_then(|t| t.as_str()),
        Some("a red fox"),
        "the idea is the chat message, not part of the instruction"
    );
    let e = describe::improve_prompt(&core, "   ", None, &[], describe::ImproveTarget::Create)
        .await
        .unwrap_err();
    assert_eq!(e.code, "invalid");
}

#[tokio::test]
async fn improve_edit_instruction_through_mock_llama() {
    let (_tmp, core, _rec) = new_core();
    let llama = MockLlamaServer::start(
        // Without the started "CHANGE:", as other servers answer.
        "swap the mug for a glass of juice\n\nDETAILS: frosted glass, apple juice, same spot\n\nKEEP: the table, the mug, the lighting",
        0,
    )
    .await;
    use_external_captioner(&core, &llama.base_url());
    let out = describe::improve_prompt(
        &core,
        "replace the mug with a glass of juice",
        Some("qwen_image_edit_2511"),
        &[],
        describe::ImproveTarget::Edit,
    )
    .await
    .unwrap();
    assert_eq!(
        out.text,
        "swap the mug for a glass of juice. Frosted glass, apple juice. Keep the table and the lighting unchanged.",
        "the reworded instruction leads; what it changes is not kept"
    );
    let body = &llama.requests()[0];
    let system = body
        .pointer("/messages/0/content")
        .and_then(|t| t.as_str())
        .unwrap();
    assert!(system.contains("three lines"), "{system}");
    assert_eq!(
        body.pointer("/messages/2").unwrap(),
        &serde_json::json!({ "role": "assistant", "content": "CHANGE:" }),
        "the answer is started with the first label"
    );
    assert_eq!(
        body["stop"],
        serde_json::json!(["\nCHANGE:"]),
        "the form may have blank lines between its two lines"
    );
    assert!(system.contains("safe for work"), "{system}");
    assert!(body["max_tokens"].as_u64().unwrap() <= 160);
}

#[tokio::test]
async fn improve_prompt_with_safe_mode_off_allows_adult_and_catches_refusals() {
    let (_tmp, core, _rec) = new_core();
    core.settings.write().content_mode = "all".into();
    let llama = MockLlamaServer::start(
        "I'm sorry, but I can't help with that request. Please ask something else.",
        0,
    )
    .await;
    use_external_captioner(&core, &llama.base_url());
    let out = describe::improve_prompt(
        &core,
        "a nude figure study",
        None,
        &[],
        describe::ImproveTarget::Create,
    )
    .await
    .unwrap();
    assert_eq!(
        out.text, "a nude figure study",
        "a refusal never replaces the prompt"
    );
    assert!(out.note.is_some());
    let system = llama.requests()[0]
        .pointer("/messages/0/content")
        .and_then(|t| t.as_str())
        .unwrap()
        .to_string();
    assert!(system.contains("Adult themes are allowed"), "{system}");
    assert!(system.contains("anyone under 18"), "{system}");
    assert!(!system.contains("safe for work"), "{system}");
}

#[tokio::test]
async fn improve_edit_catches_a_refusal_after_the_started_label() {
    let (_tmp, core, _rec) = new_core();
    let llama = MockLlamaServer::start("CHANGE: I'm sorry, but I can't help with that.", 0).await;
    use_external_captioner(&core, &llama.base_url());
    let out = describe::improve_prompt(
        &core,
        "make the sky green",
        None,
        &[],
        describe::ImproveTarget::Edit,
    )
    .await
    .unwrap();
    assert_eq!(out.text, "make the sky green");
    assert!(out.note.is_some());
}

#[tokio::test]
async fn improve_prompt_falls_back_when_the_model_loops() {
    let (_tmp, core, _rec) = new_core();
    let looped = vec!["bedroom"; 60].join(", ");
    let llama = MockLlamaServer::start(&looped, 0).await;
    use_external_captioner(&core, &llama.base_url());
    let out = describe::improve_prompt(
        &core,
        "bedroom",
        Some("sdxl"),
        &[],
        describe::ImproveTarget::Create,
    )
    .await
    .unwrap();
    assert_eq!(out.text, "bedroom", "the user's own words come back");
    assert!(out.note.is_some());
    let body = &llama.requests()[0];
    assert!(body["repeat_penalty"].as_f64().unwrap() > 1.0, "{body}");
    assert!(body["max_tokens"].as_u64().unwrap() <= 240);
}

/// A long idea leaves the helper room to restate it and still write the other lines.
#[tokio::test]
async fn improve_prompt_gives_a_long_idea_room_for_every_line() {
    let (_tmp, core, _rec) = new_core();
    let llama = MockLlamaServer::start("PROMPT: -", 0).await;
    use_external_captioner(&core, &llama.base_url());
    let idea = vec!["a quiet harbour town at dawn"; 25].join(", ");
    let _ = describe::improve_prompt(
        &core,
        &idea,
        Some("sdxl"),
        &[],
        describe::ImproveTarget::Create,
    )
    .await;
    let body = &llama.requests()[0];
    let words = idea.split_whitespace().count() as u64;
    assert!(
        body["max_tokens"].as_u64().unwrap() >= 200 + words * 2,
        "{body}"
    );
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
    let cs = describe::captioner_status(&core, describe::Purpose::Describe);
    assert!(!cs.available);
    assert!(
        cs.download_bytes > 1_000_000_000,
        "default captioner + engine to download"
    );
}
