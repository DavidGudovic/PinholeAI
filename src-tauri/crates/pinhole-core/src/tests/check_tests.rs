//! The image and word checks around Create, Edit and Describe, and model licences.

use super::*;

#[tokio::test]
async fn missing_check_files_stop_create_and_edit_before_the_engine() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    use_check(
        &core,
        FakeCheck {
            missing: vec!["Safety check: face finder"],
            ..Default::default()
        },
    );
    let model = register_fake_model(&core, "sdxl");
    let e = generate::generate(&core, GenerateRequest::txt2img(model.clone(), "a boat"))
        .await
        .unwrap_err();
    assert_eq!(e.code, crate::imagecheck::MISSING);
    assert!(e.message.contains("Set up safety check"), "{}", e.message);
    let src = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(64, 64, [1, 2, 3, 255]),
    )
    .unwrap();
    let mut req = GenerateRequest::txt2img(model, "at sunset");
    req.mode = GenMode::Img2img;
    req.init_image_id = Some(src.id);
    let e = generate::generate(&core, req).await.unwrap_err();
    assert_eq!(e.code, crate::imagecheck::MISSING);
    assert!(mock.requests().is_empty(), "nothing reaches the engine");
    assert!(!crate::imagecheck::status(&core).ready);

    // The real check with no files in Data/check is not ready either.
    let tmp2 = tempfile::tempdir().unwrap();
    let bare = AppCore::new(
        ShippedPaths {
            config_dir: config_dir(),
        },
        DataDir::at(tmp2.path().join("Data"), false),
        Arc::new(crate::NullSink),
    )
    .unwrap();
    let st = crate::imagecheck::status(&bare);
    assert!(!st.ready);
    assert_eq!(st.download_bytes, pinhole_check::files::total_bytes());
    assert_eq!(
        crate::imagecheck::ensure_ready(&bare).unwrap_err().code,
        crate::imagecheck::MISSING
    );
}

#[tokio::test]
async fn a_blocked_result_drops_the_whole_batch() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let model = register_fake_model(&core, "sdxl");
    let mut readings = intimate_adult();
    readings.tags.as_mut().unwrap().minor = 0.9;
    let fake = FakeCheck {
        readings,
        ..Default::default()
    };
    let counts = fake.counts.clone();
    use_check(&core, fake);
    let mut req = GenerateRequest::txt2img(model, "a boat");
    req.dials.count = 2;
    let e = generate::generate(&core, req).await.unwrap_err();
    assert_eq!(e.code, "blocked");
    assert_eq!(e.message, crate::text_check::BLOCKED_MESSAGE);
    assert_eq!(core.session.len(), 0, "no picture of the batch is kept");
    assert!(counts.lock().0 >= 1);
    // Dev builds name the rule for tuning; the message never changes.
    if cfg!(debug_assertions) {
        assert!(e.details.unwrap().starts_with("looks_underage"));
    }
}

/// A face the brought-in picture didn't show clearly (too small, blurred, turned) counts
/// once a later step of the chain shows it.
#[tokio::test]
async fn a_face_that_shows_up_later_in_a_brought_in_chain_counts() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let model = register_fake_model(&core, "sdxl");
    let photo = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(64, 64, [1, 2, 3, 255]),
    )
    .unwrap()
    .id;
    let restyle = |src: &str| {
        let mut req = GenerateRequest::txt2img(model.clone(), "y");
        req.mode = GenMode::Img2img;
        req.init_image_id = Some(src.to_string());
        req
    };
    // No face on the brought-in picture; an ordinary first step.
    use_check(&core, FakeCheck::default());
    let step = generate::generate(&core, restyle(&photo))
        .await
        .unwrap()
        .images[0]
        .clone();
    // The step shows a face; an intimate edit of it is blocked.
    let fake = FakeCheck {
        readings: intimate_adult(),
        original_by_size: vec![(
            (step.width, step.height),
            pinhole_check::Original { has_face: true },
        )],
        ..Default::default()
    };
    use_check(&core, fake);
    let e = generate::generate(&core, restyle(&step.id))
        .await
        .unwrap_err();
    assert_eq!(e.code, "blocked");
    // Without a brought-in picture behind it, a made picture's face doesn't matter.
    let made = generate::generate(&core, GenerateRequest::txt2img(model.clone(), "x"))
        .await
        .unwrap()
        .images[0]
        .id
        .clone();
    generate::generate(&core, restyle(&made)).await.unwrap();
}

#[tokio::test]
async fn adult_results_pass_unless_made_from_a_brought_in_photo_of_someone() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let model = register_fake_model(&core, "sdxl");
    let fake = FakeCheck {
        readings: intimate_adult(),
        original: pinhole_check::Original { has_face: true },
        ..Default::default()
    };
    let counts = fake.counts.clone();
    use_check(&core, fake);
    // Create: adult content of adults is allowed.
    let made = generate::generate(&core, GenerateRequest::txt2img(model.clone(), "x"))
        .await
        .unwrap()
        .images[0]
        .id
        .clone();
    assert_eq!(counts.lock().1, 0, "no originals to measure");

    let restyle = |src: &str| {
        let mut req = GenerateRequest::txt2img(model.clone(), "y");
        req.mode = GenMode::Img2img;
        req.init_image_id = Some(src.to_string());
        req
    };
    // Restyling a Pinhole picture: fine.
    generate::generate(&core, restyle(&made)).await.unwrap();
    // A brought-in photo of a person made intimate: blocked.
    let photo = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(64, 64, [1, 2, 3, 255]),
    )
    .unwrap()
    .id;
    let e = generate::generate(&core, restyle(&photo))
        .await
        .unwrap_err();
    assert_eq!(e.code, "blocked");
    assert_eq!(counts.lock().1, 1);

    // A chain with an ordinary step first, the original discarded, then intimate.
    use_check(
        &core,
        FakeCheck {
            original: pinhole_check::Original { has_face: true },
            ..Default::default()
        },
    );
    let step = generate::generate(&core, restyle(&photo))
        .await
        .unwrap()
        .images[0]
        .id
        .clone();
    session::discard(&core, &photo);
    let fake = FakeCheck {
        readings: intimate_adult(),
        original: pinhole_check::Original { has_face: true },
        ..Default::default()
    };
    let counts = fake.counts.clone();
    use_check(&core, fake);
    let e = generate::generate(&core, restyle(&step)).await.unwrap_err();
    assert_eq!(e.code, "blocked", "the chain still leads back to the photo");
    assert_eq!(
        counts.lock().1,
        1,
        "the original's readings were kept; only the fed-in step is measured"
    );

    // After Reset, a new brought-in picture.
    session::clear(&core).await;
    let photo = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(64, 64, [1, 2, 3, 255]),
    )
    .unwrap()
    .id;
    // Whatever a brought-in picture of a person already shows, it can't be made intimate:
    // what a picture shows says nothing about the consent of the person in it.
    let fake = FakeCheck {
        readings: intimate_adult(),
        original: pinhole_check::Original { has_face: true },
        ..Default::default()
    };
    use_check(&core, fake);
    let e = generate::generate(&core, restyle(&photo))
        .await
        .unwrap_err();
    assert_eq!(e.code, "blocked");
    // A brought-in picture without a person (a room, a landscape): fine.
    let room = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(64, 64, [4, 5, 6, 255]),
    )
    .unwrap()
    .id;
    use_check(
        &core,
        FakeCheck {
            readings: intimate_adult(),
            original: pinhole_check::Original { has_face: false },
            ..Default::default()
        },
    );
    generate::generate(&core, restyle(&room)).await.unwrap();
}

/// Same character (a Create reference picture, or Edit), Fix details and Extend run through
/// the same result intake: made intimate from a brought-in photo of someone, each is blocked.
#[tokio::test]
async fn reference_fix_details_and_extend_of_a_brought_in_photo_cant_be_made_intimate() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let sdxl = register_fake_model(&core, "sdxl");
    let klein = register_fake_model(&core, "flux2_klein_4b");
    let photo = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(512, 512, [1, 2, 3, 255]),
    )
    .unwrap()
    .id;
    let mut px = Vec::new();
    for y in 0..512u32 {
        for x in 0..512u32 {
            let on = (200..260).contains(&x) && (200..260).contains(&y);
            px.extend_from_slice(if on { &[255u8; 4] } else { &[0, 0, 0, 255] });
        }
    }
    let mask = session::import_image(
        &core,
        pinhole_engine::image::encode_png_rgba(&px, 512, 512).unwrap(),
    )
    .unwrap()
    .id;
    use_check(
        &core,
        FakeCheck {
            readings: intimate_adult(),
            original: pinhole_check::Original { has_face: true },
            face_boxes: vec![[200.0, 200.0, 60.0, 60.0]],
            ..Default::default()
        },
    );

    let mut reference = GenerateRequest::txt2img(klein.clone(), "y");
    reference.ref_image_ids = vec![photo.clone()];
    let mut edit = GenerateRequest::txt2img(klein, "y");
    edit.mode = GenMode::Edit;
    edit.ref_image_ids = vec![photo.clone()];
    let mut fix = GenerateRequest::txt2img(sdxl.clone(), "");
    fix.mode = GenMode::Img2img;
    fix.init_image_id = Some(photo.clone());
    fix.mask_image_id = Some(mask);
    fix.fix_details = true;
    let mut fix_faces = fix.clone();
    fix_faces.mask_image_id = None;
    let mut extend = GenerateRequest::txt2img(sdxl, "y");
    extend.mode = GenMode::Img2img;
    extend.init_image_id = Some(photo.clone());
    extend.extend = Some(generate::ExtendCanvas {
        width: 900,
        height: 512,
        left: 0,
        top: 0,
    });
    for (name, req) in [
        ("reference", reference),
        ("edit", edit),
        ("fix details", fix),
        ("add detail (faces)", fix_faces),
        ("extend", extend),
    ] {
        let e = generate::generate(&core, req).await.unwrap_err();
        assert_eq!(e.code, "blocked", "{name}");
        if cfg!(debug_assertions) {
            assert!(
                e.details.unwrap().starts_with("photo_made_intimate"),
                "{name}"
            );
        }
    }
}

#[tokio::test]
async fn a_reopened_save_keeps_the_photo_it_was_made_from() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let model = register_fake_model(&core, "sdxl");
    let face = pinhole_check::Original { has_face: true };
    use_check(
        &core,
        FakeCheck {
            original: face,
            ..Default::default()
        },
    );
    let photo = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(64, 64, [1, 2, 3, 255]),
    )
    .unwrap()
    .id;
    let mut req = GenerateRequest::txt2img(model.clone(), "y");
    req.mode = GenMode::Img2img;
    req.init_image_id = Some(photo.clone());
    let step = generate::generate(&core, req.clone()).await.unwrap().images[0]
        .id
        .clone();
    let saved = session::save_image(&core, &step).unwrap();
    let file = std::fs::read(&saved.path).unwrap();
    session::discard(&core, &photo);
    session::discard(&core, &step);
    let reopened = session::import_image(&core, file).unwrap().id;
    let sources = core.session.get(&reopened).unwrap().sources();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].id, photo, "the chain leads back to the photo");
    use_check(
        &core,
        FakeCheck {
            readings: intimate_adult(),
            original: face,
            ..Default::default()
        },
    );
    req.init_image_id = Some(reopened);
    assert_eq!(
        generate::generate(&core, req).await.unwrap_err().code,
        "blocked"
    );
}

#[tokio::test]
async fn a_part_deleted_by_hand_shows_as_missing_and_asks_for_its_licence() {
    let (_tmp, core, _rec) = new_core();
    core.offline.set(true); // nothing leaves the machine
    let id = register_fake_model(&core, "flux1_dev");
    let model = |core: &AppCore| crate::models::list_models(core).unwrap().remove(0);
    assert!(model(&core).missing_components.is_empty());
    let (part, comp, kind) = {
        let idx = core.installed.lock();
        let f = idx.files.iter().find(|f| f.component_id.is_some()).unwrap();
        (idx.abs_path(&core.data, f), f.component_id.clone(), f.kind)
    };
    std::fs::remove_file(&part).unwrap();
    assert_eq!(model(&core).missing_components.len(), 1);
    // FLUX.1 dev's parts download only after its licence was accepted.
    let e = crate::models::install_missing_parts(&core, &id)
        .await
        .unwrap_err();
    assert_eq!(e.code, crate::licence::LICENSE_NEEDED);

    // Downloaded again under another name: it replaces the stale entry.
    let again = part.with_file_name("again.safetensors");
    std::fs::write(&again, b"1234").unwrap();
    let file = pinhole_net::download::DownloadedFile {
        path: again,
        sha256: "ab".repeat(32),
        size_bytes: 4,
    };
    let reg = crate::models::Registration {
        kind,
        friendly_name: "part".into(),
        family: None,
        component_id: comp.clone(),
        civitai: None,
        dtype: None,
        lookup: None,
    };
    crate::models::register_download(&core, &file, reg).unwrap();
    let entries = core
        .installed
        .lock()
        .files
        .iter()
        .filter(|f| f.component_id == comp)
        .count();
    assert_eq!(entries, 1);
    assert!(model(&core).missing_components.is_empty());
}

#[tokio::test]
async fn licences_are_accepted_once_and_only_by_id() {
    let (_tmp, core, _rec) = new_core();
    core.offline.set(true); // nothing leaves the machine
    let mut st = crate::app::get_settings(&core);
    st.engine_backend = "cpu".into();
    crate::app::set_settings(&core, st).unwrap();

    // FLUX.1 dev needs its licence accepted; SDXL has none to accept.
    let e = crate::licence::require_family(&core, Some("flux1_dev")).unwrap_err();
    assert_eq!(e.code, crate::licence::LICENSE_NEEDED);
    assert_eq!(e.details.as_deref(), Some("flux1-dev-non-commercial"));
    assert!(crate::licence::require_family(&core, Some("sdxl")).is_ok());
    assert!(crate::licence::require_family(&core, None).is_ok());

    // The default Describe helper (Qwen research licence) asks before its files download.
    let e = describe::install_captioner(&core, None).await.unwrap_err();
    assert_eq!(e.code, crate::licence::LICENSE_NEEDED);
    assert!(e.message.contains("Qwen Research License"), "{}", e.message);

    // Unknown ids are refused; a plain settings save can't add or drop one.
    assert_eq!(
        crate::licence::accept_license(&core, "anything-goes")
            .unwrap_err()
            .code,
        "invalid"
    );
    let mut st = crate::app::get_settings(&core);
    st.accepted_licenses = vec!["flux1-dev-non-commercial".into()];
    crate::app::set_settings(&core, st).unwrap();
    assert!(crate::licence::require_family(&core, Some("flux1_dev")).is_err());

    // Accepting once covers every family with that licence (dev and Kontext) and is saved.
    crate::licence::accept_license(&core, "flux1-dev-non-commercial").unwrap();
    crate::licence::accept_license(&core, "flux1-dev-non-commercial").unwrap();
    assert!(crate::licence::require_family(&core, Some("flux1_dev")).is_ok());
    assert!(crate::licence::require_family(&core, Some("flux1_kontext")).is_ok());
    let saved = pinhole_store::settings::load(&core.data).unwrap();
    assert_eq!(
        saved.accepted_licenses,
        vec!["flux1-dev-non-commercial".to_string()]
    );
    let mut st = crate::app::get_settings(&core);
    st.accepted_licenses.clear();
    crate::app::set_settings(&core, st).unwrap();
    assert!(crate::licence::require_family(&core, Some("flux1_dev")).is_ok());

    crate::licence::accept_license(&core, "qwen-research").unwrap();
    assert!(describe::install_captioner(&core, None).await.is_ok());
}

#[tokio::test]
async fn setting_up_the_check_twice_does_not_hang() {
    let (_tmp, core, _rec) = new_core();
    core.offline.set(true); // the download fails at once, nothing leaves the machine
    for _ in 0..3 {
        let r = tokio::time::timeout(Duration::from_secs(20), crate::imagecheck::install(&core))
            .await
            .expect("install must not hang");
        assert_eq!(r.unwrap_err().code, "offline");
    }
    let st = crate::imagecheck::status(&core);
    assert!(!st.downloading);
}

#[tokio::test]
async fn safe_images_only_models_cant_make_intimate_pictures() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let model = register_fake_model(&core, "sdxl");
    use_check(
        &core,
        FakeCheck {
            readings: intimate_adult(),
            ..Default::default()
        },
    );
    let made = generate::generate(&core, GenerateRequest::txt2img(model.clone(), "x"))
        .await
        .unwrap()
        .images[0]
        .id
        .clone();
    // Dev builds measure a picture on request for the readings view; release builds
    // never show readings.
    let readings = crate::imagecheck::readings_of(&core, &made).await.unwrap();
    if cfg!(debug_assertions) {
        let r = readings.unwrap();
        assert!(r.starts_with("nudity 0.95"), "{r}");
        assert!(r.contains("Made with AI watermark: no"), "{r}");
    } else {
        assert!(readings.is_none());
    }
    {
        let mut idx = core.installed.lock();
        let m = idx.get_mut(&model).unwrap();
        let mut c = pinhole_store::installed::CivitaiRef {
            model_id: 1,
            version_id: 2,
            model_name: None,
            version_name: None,
            base_model: None,
            trained_words: vec![],
            license: None,
            creator_notes: None,
            sfw_only: false,
        };
        c.sfw_only = true;
        m.civitai = Some(c);
    }
    let e = generate::generate(&core, GenerateRequest::txt2img(model.clone(), "x"))
        .await
        .unwrap_err();
    assert_eq!(e.code, "blocked");
    // Ordinary pictures from it are fine.
    use_check(&core, FakeCheck::default());
    let ordinary = generate::generate(&core, GenerateRequest::txt2img(model.clone(), "x"))
        .await
        .unwrap()
        .images[0]
        .id
        .clone();
    // An upscale of one stays under the rule: the upscaler adds no model of its own.
    install_fake_upscaler(&core);
    use_check(
        &core,
        FakeCheck {
            readings: intimate_adult(),
            ..Default::default()
        },
    );
    let e = generate::upscale_image(&core, &ordinary, 4)
        .await
        .unwrap_err();
    assert_eq!(e.code, "blocked");
    if cfg!(debug_assertions) {
        assert!(e.details.unwrap().starts_with("safe_images_only"));
    }
    // Nor does restyling it with another, unmarked model lift the rule.
    let other = register_fake_model(&core, "sdxl");
    assert_ne!(other, model);
    let mut restyle = GenerateRequest::txt2img(other.clone(), "y");
    restyle.mode = GenMode::Img2img;
    restyle.init_image_id = Some(ordinary.clone());
    let e = generate::generate(&core, restyle).await.unwrap_err();
    assert_eq!(e.code, "blocked");
    // Nor does saving it and opening the file again.
    let saved = session::export_png(&core, &[core.session.get(&ordinary).unwrap()]).unwrap();
    let reopened = session::import_image(&core, saved).unwrap().id;
    let e = generate::upscale_image(&core, &reopened, 4)
        .await
        .unwrap_err();
    assert_eq!(e.code, "blocked");
    // An upscale or restyle of a picture from an unmarked model isn't under it.
    generate::upscale_image(&core, &made, 4).await.unwrap();
    let mut restyle = GenerateRequest::txt2img(other, "y");
    restyle.mode = GenMode::Img2img;
    restyle.init_image_id = Some(made);
    generate::generate(&core, restyle).await.unwrap();
}

/// RELEASE-SPEC §5: an add-on or model added by hand or linked counts as "safe images
/// only" until a CivitAI lookup clears it (not looked up yet, or no match), and one
/// CivitAI marks as a real person or a minor can't be used at all.
#[tokio::test]
async fn unchecked_hand_added_files_are_safe_images_only() {
    use pinhole_store::installed::Lookup;
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let model = register_fake_model(&core, "sdxl");
    let lora = register_fake_lora(&core, "sdxl", &[]);
    use_check(
        &core,
        FakeCheck {
            readings: intimate_adult(),
            ..Default::default()
        },
    );
    let with_lora = || {
        let mut req = GenerateRequest::txt2img(model.clone(), "x");
        req.loras = vec![generate::LoraUse {
            lora_id: lora.clone(),
            weight: 0.7,
            words: None,
        }];
        req
    };
    let set = |id: &str, l: Option<Lookup>| {
        core.installed.lock().get_mut(id).unwrap().lookup = l;
    };
    generate::generate(&core, with_lora()).await.unwrap();
    for state in [Lookup::NotYet, Lookup::NoMatch] {
        set(&lora, Some(state));
        let e = generate::generate(&core, with_lora()).await.unwrap_err();
        assert_eq!(e.code, "blocked", "add-on {state:?}");
        set(&lora, None);
        set(&model, Some(state));
        let e = generate::generate(&core, with_lora()).await.unwrap_err();
        assert_eq!(e.code, "blocked", "model {state:?}");
        set(&model, None);
    }
    // Cleared by a lookup.
    set(&lora, Some(Lookup::Found));
    generate::generate(&core, with_lora()).await.unwrap();
    // Ordinary pictures from an unchecked add-on are fine.
    set(&lora, Some(Lookup::NotYet));
    use_check(&core, FakeCheck::default());
    generate::generate(&core, with_lora()).await.unwrap();
    // A real person or a minor: refused before anything runs.
    set(&lora, Some(Lookup::Refused));
    let e = generate::generate(&core, with_lora()).await.unwrap_err();
    assert_eq!(e.message, pinhole_catalog::api::PERSON_OR_MINOR_REASON);
    set(&lora, None);
    set(&model, Some(Lookup::Refused));
    let e = generate::generate(&core, GenerateRequest::txt2img(model.clone(), "x"))
        .await
        .unwrap_err();
    assert_eq!(e.message, pinhole_catalog::api::PERSON_OR_MINOR_REASON);
}

#[tokio::test]
async fn word_check_blocks_generate_before_the_engine() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let model = register_fake_model(&core, "sdxl");
    // The sexual half comes from a saved style: the combined prompt is what is checked.
    let style = crate::library::save_style(
        &core,
        pinhole_store::styles::Style {
            id: String::new(),
            name: "Check".into(),
            positive: "nude".into(),
            negative: None,
            families: vec![],
            thumbnail: None,
            builtin: false,
        },
    )
    .unwrap();
    let mut req = GenerateRequest::txt2img(model.clone(), "a child");
    req.style_id = Some(style.id.clone());
    let e = generate::generate(&core, req).await.unwrap_err();
    assert_eq!(e.code, "blocked");
    assert!(e.details.is_none());
    assert!(mock.requests().is_empty(), "nothing reaches the engine");

    // Edit (Restyle, Fix details and Describe a change) goes through the same check.
    let src = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(64, 64, [1, 2, 3, 255]),
    )
    .unwrap();
    let mut req = GenerateRequest::txt2img(model.clone(), "make her a teenager, topless");
    req.mode = GenMode::Img2img;
    req.init_image_id = Some(src.id.clone());
    assert_eq!(
        generate::generate(&core, req).await.unwrap_err().code,
        "blocked"
    );
    let mut req = GenerateRequest::txt2img(model.clone(), "make them look 12 years old, naked");
    req.mode = GenMode::Edit;
    req.ref_image_ids = vec![src.id.clone()];
    assert_eq!(
        generate::generate(&core, req).await.unwrap_err().code,
        "blocked"
    );
    assert!(mock.requests().is_empty());

    // An add-on's trigger words count even when they aren't added to the prompt.
    let lora = register_fake_lora(&core, "sdxl", &["loli"]);
    let mut req = GenerateRequest::txt2img(model.clone(), "1girl, nude");
    req.loras = vec![generate::LoraUse {
        lora_id: lora,
        weight: 1.0,
        words: None,
    }];
    req.add_trigger_words = false;
    let e = generate::generate(&core, req.clone()).await.unwrap_err();
    assert_eq!(e.code, "blocked");
    assert!(mock.requests().is_empty());
    // CivitAI's trigger words stay part of the word check when the user clears the
    // add-on's own trigger words.
    crate::models::set_lora_trigger_words(&core, &req.loras[0].lora_id, vec![]).unwrap();
    let e = generate::generate(&core, req).await.unwrap_err();
    assert_eq!(e.code, "blocked");
    assert!(mock.requests().is_empty());

    // The negative prompt is not part of the word check.
    let mut req = GenerateRequest::txt2img(model.clone(), "a nude woman, oil painting");
    req.fine_tune.negative_prompt = Some("child".into());
    generate::generate(&core, req).await.unwrap();
    assert_eq!(mock.requests().len(), 1);

    // The request never carries a CFG below 1, whatever Fine-tune or a pasted setting says.
    for cfg in [0.0, 0.5, -3.0] {
        let mut req = GenerateRequest::txt2img(model.clone(), "a boat");
        req.fine_tune.cfg = Some(cfg);
        generate::generate(&core, req).await.unwrap();
        let body = mock.requests().pop().unwrap();
        assert_eq!(body["sample_params"]["guidance"]["txt_cfg"], 1.0, "{cfg}");
    }
}

#[tokio::test]
async fn word_check_blocks_describe_and_improve() {
    let (_tmp, core, _rec) = new_core();
    let llama = MockLlamaServer::start("a child, naked", 0).await;
    use_external_captioner(&core, &llama.base_url());
    let img = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(32, 32, [1, 2, 3, 255]),
    )
    .unwrap();
    let e = describe::describe_image(&core, &img.id, describe::DescribeStyle::Tags)
        .await
        .unwrap_err();
    assert_eq!(e.code, "blocked", "the model's text isn't shown");
    let e = describe::improve_prompt(
        &core,
        "a lighthouse",
        None,
        &[],
        describe::ImproveTarget::Create,
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "blocked", "the improved text isn't shown");
    let sent = llama.requests().len();
    let e = describe::improve_prompt(
        &core,
        "loli, lewd",
        None,
        &[],
        describe::ImproveTarget::Create,
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "blocked");
    assert_eq!(
        llama.requests().len(),
        sent,
        "the idea isn't sent to the model"
    );
}

/// A brought-in picture is checked once, before it is first described.
#[tokio::test]
async fn describe_checks_a_brought_in_picture_first() {
    let (_tmp, core, _rec) = new_core();
    let llama = MockLlamaServer::start("Prompt: a lighthouse at dusk", 0).await;
    use_external_captioner(&core, &llama.base_url());
    let img = |c: [u8; 4]| {
        session::import_image(&core, pinhole_engine::testutil::solid_png(32, 32, c))
            .unwrap()
            .id
    };
    let fine = img([1, 2, 3, 255]);
    let fake = FakeCheck::default();
    let counts = fake.counts.clone();
    use_check(&core, fake);
    for _ in 0..2 {
        describe::describe_image(&core, &fine, describe::DescribeStyle::Sentence)
            .await
            .unwrap();
    }
    assert_eq!(counts.lock().0, 1, "checked once");

    let mut readings = intimate_adult();
    readings.tags.as_mut().unwrap().minor = 0.9;
    use_check(
        &core,
        FakeCheck {
            readings,
            ..Default::default()
        },
    );
    let sent = llama.requests().len();
    let e = describe::describe_image(&core, &img([4, 5, 6, 255]), describe::DescribeStyle::Tags)
        .await
        .unwrap_err();
    assert_eq!(e.code, "blocked");
    assert_eq!(
        llama.requests().len(),
        sent,
        "the picture isn't sent to the model"
    );

    // And nothing is described without the check's files.
    use_check(
        &core,
        FakeCheck {
            missing: vec!["nudity"],
            ..Default::default()
        },
    );
    let e = describe::describe_image(&core, &img([7, 8, 9, 255]), describe::DescribeStyle::Tags)
        .await
        .unwrap_err();
    assert_eq!(e.code, "check_missing");
}
