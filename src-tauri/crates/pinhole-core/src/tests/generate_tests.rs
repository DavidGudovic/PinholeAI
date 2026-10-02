//! Create and Edit jobs against `MockSdServer`: requests, results and errors.

use super::*;

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

    // Save (default: only the AI marker), then with "settings (no prompt)".
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
    let is_marker =
        |c: &(String, Vec<u8>)| c.0 == "iTXt" && c.1.starts_with(b"XML:com.adobe.xmp\0");
    let chunks = pinhole_engine::png::text_chunks(&std::fs::read(&saved.path).unwrap());
    assert_eq!(chunks.len(), 1);
    assert!(is_marker(&chunks[0]));
    core.settings.write().saved_metadata = "settings".into();
    let saved2 = session::save_image(&core, &res.images[0].id).unwrap();
    assert_ne!(saved.path, saved2.path, "unique name on collision");
    let chunks = pinhole_engine::png::text_chunks(&std::fs::read(&saved2.path).unwrap());
    assert_eq!(chunks.len(), 2);
    assert!(is_marker(&chunks[0]));
    assert!(chunks[1].1.starts_with(b"pinhole\0"));
    assert!(has(&chunks[1].1, "\"seed\":1000"));

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
        words: None,
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
async fn models_from_another_apps_folder_generate_in_place() {
    let (tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    // Installs the SDXL parts (VAE) the linked checkpoint needs.
    register_fake_model(&core, "sdxl");
    let comfy = tmp.path().join("ComfyUI");
    crate::linked::fixtures::comfy(&comfy);
    crate::linked::add(&core, &comfy.display().to_string()).unwrap();
    let start = std::time::Instant::now();
    while crate::linked::list(&core).iter().any(|f| f.scanning) {
        assert!(start.elapsed() < std::time::Duration::from_secs(30));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let models = crate::models::list_models(&core).unwrap();
    let model = models.iter().find(|m| m.linked_folder.is_some()).unwrap();
    assert!(model.missing_components.is_empty(), "{model:?}");
    let lora = crate::models::list_loras(&core).unwrap()[0].id.clone();

    let mut req = GenerateRequest::txt2img(model.id.clone(), "a red boat");
    req.loras = vec![generate::LoraUse {
        lora_id: lora,
        weight: 0.8,
        words: None,
    }];
    generate::generate(&core, req).await.unwrap();
    let body = &mock.requests()[0];
    let path = body["lora"][0]["path"].as_str().unwrap();
    assert!(path.starts_with(".pinhole-linked/"), "{path}");
    assert!(
        path.ends_with("models/loras/watercolor.safetensors"),
        "{path}"
    );
    // The folder itself is untouched.
    assert!(!comfy.join("models/loras/.pinhole-linked").exists());
}

#[test]
fn trigger_words_follow_the_chip_and_the_users_list() {
    let (_tmp, core, _rec) = new_core();
    let model = register_fake_model(&core, "sdxl");
    let lora = register_fake_lora(&core, "sdxl", &["alpha look", "beta look", "gamma"]);
    let prompt_for = |words: Option<Vec<&str>>, prompt: &str| {
        let mut req = GenerateRequest::txt2img(model.clone(), prompt);
        req.loras = vec![generate::LoraUse {
            lora_id: lora.clone(),
            weight: 0.8,
            words: words.map(|w| w.into_iter().map(String::from).collect()),
        }];
        generate::preview_final_prompt(&core, &req).unwrap().prompt
    };
    // No pick = every word; a pick keeps only listed words, in the add-on's order.
    let all = prompt_for(None, "a boat");
    assert!(all.contains("alpha look, beta look, gamma"), "{all}");
    let some = prompt_for(Some(vec!["GAMMA", "alpha look", "made up"]), "a boat");
    assert!(
        some.contains("alpha look, gamma") && !some.contains("beta") && !some.contains("made up"),
        "{some}"
    );
    assert!(!prompt_for(Some(vec![]), "a boat").contains("look"));
    // Already typed (whole words) = not added twice; part of a longer word doesn't count.
    let typed = prompt_for(Some(vec!["gamma"]), "a Gamma boat");
    assert_eq!(typed.matches("amma").count(), 1, "{typed}");
    assert!(prompt_for(Some(vec!["gamma"]), "a gammaray boat").contains(", gamma"));

    // The user's own list replaces CivitAI's and survives a reload.
    let saved = crate::models::set_lora_trigger_words(
        &core,
        &lora,
        vec![" own word ".into(), "".into(), "Own Word".into()],
    )
    .unwrap();
    assert_eq!(saved.trained_words, ["own word"]);
    let reloaded = pinhole_store::installed::InstalledIndex::load(&core.data).unwrap();
    assert_eq!(reloaded.get(&lora).unwrap().trigger_words(), ["own word"]);
    assert!(prompt_for(None, "a boat").contains("own word"));
    assert!(!prompt_for(None, "a boat").contains("alpha"));
    // Not a LoRA / not installed.
    assert!(crate::models::set_lora_trigger_words(&core, &model, vec![]).is_err());
    assert!(crate::models::set_lora_trigger_words(&core, "nope", vec![]).is_err());
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
async fn fix_details_redraws_the_painted_box_and_blends_it_back() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let model = register_fake_model(&core, "sdxl");
    let src = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(1200, 900, [10, 20, 30, 255]),
    )
    .unwrap();
    // A small painted spot (a "face") at (600..660, 300..380).
    let mut px = Vec::new();
    for y in 0..900u32 {
        for x in 0..1200u32 {
            let on = (600..660).contains(&x) && (300..380).contains(&y);
            px.extend_from_slice(if on { &[255u8; 4] } else { &[0, 0, 0, 255] });
        }
    }
    let mask_png = pinhole_engine::image::encode_png_rgba(&px, 1200, 900).unwrap();
    let mask = session::import_image(&core, mask_png).unwrap();
    let fix_req = |mask_id: Option<String>| {
        let mut req = GenerateRequest::txt2img(model.clone(), "");
        req.mode = GenMode::Img2img;
        req.init_image_id = Some(src.id.clone());
        req.strength = Some(0.45);
        req.mask_image_id = mask_id;
        req.fix_details = true;
        req.dials.count = 4;
        req
    };

    let fake = FakeCheck::default();
    let checked = fake.sizes.clone();
    let sizes = checked.clone();
    use_check(&core, fake);
    let res = generate::generate(&core, fix_req(Some(mask.id.clone())))
        .await
        .unwrap();
    assert_eq!(res.images.len(), 1);
    // Regression: the redrawn box is checked on its own too, not only as a small part of
    // the whole picture.
    let sizes = sizes.lock().clone();
    assert_eq!(sizes.len(), 2, "{sizes:?}");
    assert_eq!(sizes[0], (1200, 900));
    let out = &res.images[0];
    assert_eq!(
        (out.width, out.height),
        (1200, 900),
        "the whole image comes back"
    );
    assert_eq!(out.parent_id.as_deref(), Some(src.id.as_str()));
    let body = &mock.requests()[0];
    let (w, h) = (
        body["width"].as_u64().unwrap(),
        body["height"].as_u64().unwrap(),
    );
    // The engine's own output, at the model's size, not the ~128 px box it becomes.
    assert!(
        sizes[1].0 >= 768 && sizes[1].1 >= 768,
        "the redraw is checked at the size it was drawn: {sizes:?}"
    );
    // The ~128x128 box is drawn at SDXL's native size, not at 128 px.
    assert!(
        w >= 768 && h >= 768 && w % 64 == 0 && h % 64 == 0,
        "{w}x{h}"
    );
    assert_eq!(body["batch_count"], 1);
    assert!(body.get("hires").is_none_or(|v| v.is_null()));
    assert!(body["mask_image"].as_str().unwrap().len() > 50);
    use base64::Engine as _;
    let init = base64::engine::general_purpose::STANDARD
        .decode(body["init_image"].as_str().unwrap())
        .unwrap();
    let info = pinhole_engine::image::sniff(&init).unwrap();
    assert_eq!((u64::from(info.width), u64::from(info.height)), (w, h));

    // The redraw landed inside the painted spot only.
    let img = core.session.get(&out.id).unwrap();
    let (px, _, _) = pinhole_engine::image::decode_rgba(img.bytes.as_slice()).unwrap();
    let at = |x: usize, y: usize| &px[(y * 1200 + x) * 4..][..4];
    assert_eq!(at(10, 10), &[10, 20, 30, 255]);
    assert_ne!(at(630, 340), &[10, 20, 30, 255]);

    // Nothing painted and no face found: nothing to add detail to.
    let n = mock.requests().len();
    let err = generate::generate(&core, fix_req(None)).await.unwrap_err();
    assert!(err.message.contains("No face found"), "{}", err.message);
    assert_eq!(mock.requests().len(), n, "no engine job");

    // A face already larger than the model draws (a close-up): left as it is.
    use_check(
        &core,
        FakeCheck {
            face_boxes: vec![[200.0, 100.0, 700.0, 700.0]],
            ..Default::default()
        },
    );
    let err = generate::generate(&core, fix_req(None)).await.unwrap_err();
    assert!(err.message.contains("No face found"), "{}", err.message);
    assert_eq!(mock.requests().len(), n, "no engine job");

    // Nothing painted, two faces: each is redrawn in its own pass at the model's size,
    // largest first, and the picture comes back at its own size.
    let fake = FakeCheck {
        face_boxes: vec![[100.0, 100.0, 200.0, 200.0], [800.0, 500.0, 60.0, 60.0]],
        ..Default::default()
    };
    let checked = fake.sizes.clone();
    use_check(&core, fake);
    let res = generate::generate(&core, fix_req(None)).await.unwrap();
    let out = &res.images[0];
    assert_eq!((out.width, out.height), (1200, 900));
    assert_eq!(out.parent_id.as_deref(), Some(src.id.as_str()));
    let reqs = mock.requests();
    assert_eq!(reqs.len(), n + 2, "one pass per face");
    for body in &reqs[n..] {
        assert!(body["mask_image"].as_str().unwrap().len() > 50);
        assert_eq!(body["batch_count"], 1);
        assert_eq!(body["strength"], 0.45);
        let (w, h) = (
            body["width"].as_u64().unwrap(),
            body["height"].as_u64().unwrap(),
        );
        assert!(w >= 768 && h >= 768, "drawn at the model's size: {w}x{h}");
    }
    let sizes = checked.lock().clone();
    assert_eq!(sizes.len(), 3, "the result and each redraw: {sizes:?}");
    assert_eq!(sizes[0], (1200, 900));
    let img = core.session.get(&out.id).unwrap();
    let (px, _, _) = pinhole_engine::image::decode_rgba(img.bytes.as_slice()).unwrap();
    let at = |x: usize, y: usize| &px[(y * 1200 + x) * 4..][..4];
    assert_eq!(
        at(10, 10),
        &[10, 20, 30, 255],
        "away from the faces: untouched"
    );
    // The second pass worked on the first pass's result: the first face stays redrawn.
    assert_ne!(at(200, 200), &[10, 20, 30, 255], "first face redrawn");
    assert_ne!(at(830, 530), &[10, 20, 30, 255], "second face redrawn");
}

#[tokio::test]
async fn extend_draws_the_bigger_canvas_and_keeps_the_source() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let model = register_fake_model(&core, "sdxl");
    let src = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(600, 600, [10, 20, 30, 255]),
    )
    .unwrap();
    let extend_req = |canvas: generate::ExtendCanvas| {
        let mut req = GenerateRequest::txt2img(model.clone(), "a beach");
        req.mode = GenMode::Img2img;
        req.init_image_id = Some(src.id.clone());
        req.strength = Some(0.55);
        req.extend = Some(canvas);
        req.dials.count = 4;
        req
    };

    // Square → wide, the new space on the right.
    let wide = generate::ExtendCanvas {
        width: 1050,
        height: 600,
        left: 0,
        top: 0,
    };
    let res = generate::generate(&core, extend_req(wide)).await.unwrap();
    assert_eq!(res.images.len(), 1);
    let out = &res.images[0];
    assert_eq!(
        (out.width, out.height),
        (1050, 600),
        "the canvas comes back"
    );
    assert_eq!(out.parent_id.as_deref(), Some(src.id.as_str()));
    let body = &mock.requests()[0];
    let (w, h) = (
        body["width"].as_u64().unwrap(),
        body["height"].as_u64().unwrap(),
    );
    // Drawn at SDXL's size in the canvas's shape.
    assert!(
        w > h && w % 64 == 0 && h % 64 == 0 && w * h >= 800_000,
        "{w}x{h}"
    );
    assert_eq!(body["strength"], 1.0);
    assert_eq!(body["batch_count"], 1);
    assert!(body.get("hires").is_none_or(|v| v.is_null()));
    assert!(body["mask_image"].as_str().unwrap().len() > 50);

    // The source's pixels are kept, away from the seam.
    let img = core.session.get(&out.id).unwrap();
    let (px, _, _) = pinhole_engine::image::decode_rgba(img.bytes.as_slice()).unwrap();
    let at = |x: usize, y: usize| &px[(y * 1050 + x) * 4..][..4];
    assert_eq!(at(10, 300), &[10, 20, 30, 255]);
    assert_eq!(at(500, 590), &[10, 20, 30, 255]);

    // Nothing to add, or with a mask: refused before any engine work.
    let same = generate::ExtendCanvas {
        width: 600,
        height: 600,
        left: 0,
        top: 0,
    };
    let err = generate::generate(&core, extend_req(same))
        .await
        .unwrap_err();
    assert!(
        err.message.contains("already this shape"),
        "{}",
        err.message
    );
    let mut masked = extend_req(wide);
    masked.mask_image_id = Some(src.id.clone());
    assert!(generate::generate(&core, masked).await.is_err());
    assert_eq!(mock.requests().len(), 1);
}

#[tokio::test]
async fn instruction_edit_uses_a_generator_that_can_edit() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let src = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(256, 256, [10, 20, 30, 255]),
    )
    .unwrap();
    let edit_req = |model: String| {
        let mut req = GenerateRequest::txt2img(model, "make it evening");
        req.mode = GenMode::Edit;
        req.ref_image_ids = vec![src.id.clone()];
        req.dials.quality = Quality::Fast;
        req
    };

    // SDXL can only restyle: no instruction edit without an edit-capable model.
    let sdxl = register_fake_model(&core, "sdxl");
    let err = generate::generate(&core, edit_req(sdxl.clone()))
        .await
        .unwrap_err();
    assert!(err.message.contains("No edit model"), "{}", err.message);

    // FLUX.2 klein generates and edits: picked when chosen, and as the fallback.
    let klein = register_fake_model(&core, "flux2_klein_4b");
    for chosen in [klein.clone(), sdxl] {
        let res = generate::generate(&core, edit_req(chosen)).await.unwrap();
        assert_eq!(res.images[0].family_id, "flux2_klein_4b");
    }
    let body = mock.requests().last().cloned().unwrap();
    assert_eq!(body["ref_images"].as_array().unwrap().len(), 1);

    // A dedicated edit model still wins the fallback.
    let _qwen_edit = register_fake_model(&core, "qwen_image_edit_2511");
    let res = generate::generate(&core, edit_req(klein.clone()))
        .await
        .unwrap();
    assert_eq!(
        res.images[0].family_id, "flux2_klein_4b",
        "the chosen one runs"
    );
    let mut req = edit_req(klein);
    req.model_id = "gone".into();
    let res = generate::generate(&core, req).await.unwrap();
    assert_eq!(res.images[0].family_id, "qwen_image_edit_2511");
}

#[tokio::test]
async fn create_sends_a_reference_picture_to_models_that_take_one() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let reference = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(600, 300, [10, 20, 30, 255]),
    )
    .unwrap();
    let with_ref = |model: String| {
        let mut req = GenerateRequest::txt2img(model, "a lighthouse in the style of the picture");
        req.ref_image_ids = vec![reference.id.clone()];
        req
    };

    // SDXL can't take one: a plain error instead of quietly ignoring the picture.
    let sdxl = register_fake_model(&core, "sdxl");
    let err = generate::generate(&core, with_ref(sdxl)).await.unwrap_err();
    assert!(err.message.contains("reference picture"), "{}", err.message);
    assert!(mock.requests().is_empty());

    // FLUX.2 klein: the picture goes in ref_images; the size follows the dials (square), and
    // it isn't the result's parent (that's for edits).
    let klein = register_fake_model(&core, "flux2_klein_4b");
    let res = generate::generate(&core, with_ref(klein)).await.unwrap();
    assert_eq!(res.images[0].family_id, "flux2_klein_4b");
    assert!(res.images[0].parent_id.is_none());
    let body = mock.requests().last().cloned().unwrap();
    assert_eq!(body["ref_images"].as_array().unwrap().len(), 1);
    assert!(body.get("init_image").is_none());
    assert_eq!(body["width"], body["height"]);
}

#[tokio::test]
async fn a_create_reference_picture_needs_the_vision_encoder() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let reference = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(64, 64, [10, 20, 30, 255]),
    )
    .unwrap();
    let qwen = register_fake_model(&core, "qwen_image_21");
    core.installed
        .lock()
        .files
        .retain(|f| f.component_id.as_deref() != Some("qwen3vl_8b_mmproj"));
    let mut req = GenerateRequest::txt2img(qwen, "a lighthouse in the style of the picture");
    req.ref_image_ids = vec![reference.id.clone()];
    let err = generate::generate(&core, req).await.unwrap_err();
    assert_eq!(err.code, "not_found");
    assert!(err.message.contains("Get"), "{}", err.message);
    assert!(mock.requests().is_empty());
}

#[tokio::test]
async fn edit_add_ons_go_only_to_the_model_they_were_picked_for() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let src = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(256, 256, [10, 20, 30, 255]),
    )
    .unwrap();
    let sdxl = register_fake_model(&core, "sdxl");
    let _qwen_edit = register_fake_model(&core, "qwen_image_edit_2511");
    let lora = register_fake_lora(&core, "sdxl", &["zxc_trigger"]);
    let with_lora = |mode: GenMode| {
        let mut req = GenerateRequest::txt2img(sdxl.clone(), "make it evening");
        req.mode = mode;
        req.dials.quality = Quality::Fast;
        req.loras = vec![generate::LoraUse {
            lora_id: lora.clone(),
            weight: 0.7,
            words: None,
        }];
        req
    };

    // Restyle with SDXL: the add-on and its trigger word are used.
    let mut req = with_lora(GenMode::Img2img);
    req.init_image_id = Some(src.id.clone());
    req.strength = Some(0.5);
    generate::generate(&core, req).await.unwrap();
    let body = mock.requests().last().cloned().unwrap();
    assert_eq!(body["lora"].as_array().unwrap().len(), 1);
    assert!(body["prompt"].as_str().unwrap().contains("zxc_trigger"));

    // An instruction edit that falls back to Qwen Image Edit drops the SDXL add-on.
    let mut req = with_lora(GenMode::Edit);
    req.ref_image_ids = vec![src.id.clone()];
    let res = generate::generate(&core, req).await.unwrap();
    assert_eq!(res.images[0].family_id, "qwen_image_edit_2511");
    let body = mock.requests().last().cloned().unwrap();
    assert!(body["lora"].as_array().is_none_or(|l| l.is_empty()));
    assert!(!body["prompt"].as_str().unwrap().contains("zxc_trigger"));
}

#[tokio::test]
async fn qwen_image_21_creates_without_its_vision_encoder_but_edits_need_it() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let q21 = register_fake_model(&core, "qwen_image_21");
    {
        let mut idx = core.installed.lock();
        let vision = idx.find_component("qwen3vl_8b_mmproj").unwrap().id.clone();
        idx.remove(&vision);
    }
    let mut req = GenerateRequest::txt2img(q21.clone(), "a lighthouse");
    req.dials.quality = Quality::Fast;
    generate::generate(&core, req).await.unwrap();

    let src = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(256, 256, [10, 20, 30, 255]),
    )
    .unwrap();
    let mut req = GenerateRequest::txt2img(q21, "make it evening");
    req.mode = GenMode::Edit;
    req.ref_image_ids = vec![src.id];
    let err = generate::generate(&core, req).await.unwrap_err();
    assert!(
        err.message.contains("mmproj-Qwen3VL-8B-Instruct-F16.gguf"),
        "{}",
        err.message
    );
}

#[tokio::test]
async fn two_image_edit_needs_a_model_that_combines_them() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let img = |c: u8| {
        session::import_image(
            &core,
            pinhole_engine::testutil::solid_png(256, 256, [c, 20, 30, 255]),
        )
        .unwrap()
        .id
    };
    let a = img(10);
    let sdxl = register_fake_model(&core, "sdxl");
    let b = generate::generate(&core, GenerateRequest::txt2img(sdxl, "a green bottle"))
        .await
        .unwrap()
        .images[0]
        .id
        .clone();
    let edit_req = |model: String| {
        let mut req = GenerateRequest::txt2img(model, "put the bottle from image 2 on the shelf");
        req.mode = GenMode::Edit;
        req.ref_image_ids = vec![a.clone(), b.clone()];
        req.dials.quality = Quality::Fast;
        req
    };

    // Kontext edits one image only.
    let kontext = register_fake_model(&core, "flux1_kontext");
    let err = generate::generate(&core, edit_req(kontext.clone()))
        .await
        .unwrap_err();
    assert!(
        err.message.contains("combine two images"),
        "{}",
        err.message
    );

    let klein = register_fake_model(&core, "flux2_klein_4b");
    for chosen in [klein, kontext] {
        let res = generate::generate(&core, edit_req(chosen)).await.unwrap();
        assert_eq!(res.images[0].family_id, "flux2_klein_4b");
        let body = mock.requests().last().cloned().unwrap();
        assert_eq!(body["ref_images"].as_array().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn origin_follows_every_image_a_result_is_made_from() {
    use crate::generate::Origin;
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let sdxl = register_fake_model(&core, "sdxl");
    let klein = register_fake_model(&core, "flux2_klein_4b");
    let origin = |id: &str| core.session.get(id).unwrap().origin;
    let photo = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(256, 256, [10, 20, 30, 255]),
    )
    .unwrap()
    .id;
    assert_eq!(origin(&photo), Origin::Imported);

    // Create: made in Pinhole; with a brought-in reference picture it isn't.
    let made = generate::generate(&core, GenerateRequest::txt2img(sdxl.clone(), "a boat"))
        .await
        .unwrap()
        .images[0]
        .clone();
    assert_eq!(made.origin, Origin::Generated);
    assert_eq!(origin(&made.id), Origin::Generated);
    let mut req = GenerateRequest::txt2img(klein.clone(), "a boat like this");
    req.ref_image_ids = vec![photo.clone()];
    let res = generate::generate(&core, req).await.unwrap();
    assert_eq!(res.images[0].origin, Origin::Imported);

    // Restyle and Edit keep the source's origin.
    let restyle = |src: &str| {
        let mut req = GenerateRequest::txt2img(sdxl.clone(), "at sunset");
        req.mode = GenMode::Img2img;
        req.init_image_id = Some(src.to_string());
        req.strength = Some(0.5);
        req
    };
    let r = generate::generate(&core, restyle(&made.id)).await.unwrap();
    assert_eq!(r.images[0].origin, Origin::Generated);
    let r = generate::generate(&core, restyle(&photo)).await.unwrap();
    let restyled_photo = r.images[0].id.clone();
    assert_eq!(r.images[0].origin, Origin::Imported);
    // ... and so does a result made from that result.
    let r = generate::generate(&core, restyle(&restyled_photo))
        .await
        .unwrap();
    assert_eq!(r.images[0].origin, Origin::Imported);

    // Edit: either image brought in → Imported.
    let edit = |ids: Vec<String>| {
        let mut req = GenerateRequest::txt2img(klein.clone(), "put the boat from image 2 here");
        req.mode = GenMode::Edit;
        req.ref_image_ids = ids;
        req.dials.quality = Quality::Fast;
        req
    };
    let r = generate::generate(&core, edit(vec![photo.clone(), made.id.clone()]))
        .await
        .unwrap();
    assert_eq!(r.images[0].origin, Origin::Imported);
    let r = generate::generate(&core, edit(vec![made.id.clone()]))
        .await
        .unwrap();
    assert_eq!(r.images[0].origin, Origin::Generated);
    // A brought-in image 2 makes the result Imported too (the image check covers it).
    let r = generate::generate(&core, edit(vec![made.id.clone(), restyled_photo]))
        .await
        .unwrap();
    assert_eq!(r.images[0].origin, Origin::Imported);
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
    assert_eq!(err.message, crate::memory::RAM_MESSAGE);

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
        trigger_words: None,
        lookup: None,
    };
    // No shared components installed: fine for an all-in-one checkpoint…
    let files = crate::generate::model_files(&core, &aio, &fam, &hw, false).unwrap();
    assert_eq!(files.layout, pinhole_registry::Layout::AllInOne);
    let args = pinhole_registry::wiring::launch_args(&reg, &files, &hw, &Default::default());
    assert!(args.iter().any(|a| a == "--model" || a == "-m"), "{args:?}");
    assert!(!args.iter().any(|a| a == "--diffusion-model"), "{args:?}");
    // …but a diffusion-only file of the same family must have them.
    let mut dif = aio.clone();
    dif.kind = ModelKind::Diffusion;
    let (rel, _) = write_dummy(&core, ModelKind::Diffusion, "flux-dit.safetensors");
    dif.rel_path = rel;
    let err = crate::generate::model_files(&core, &dif, &fam, &hw, false).unwrap_err();
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
        trigger_words: None,
        lookup: None,
    };
    let err = crate::generate::model_files(&core, &zit, &fam, &hw, false).unwrap_err();
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
                trigger_words: None,
                lookup: None,
            });
        }
    }
    let files = crate::generate::model_files(&core, &zit, &fam, &hw, false).unwrap();
    let llm = files.components.get("llm").unwrap();
    assert!(
        llm.ends_with(&reg.component("qwen3_4b").unwrap().file),
        "{llm:?}"
    );
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
