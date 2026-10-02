//! Upscale: picking, downloading and running the ESRGAN upscaler.

use super::*;

#[tokio::test]
async fn upscale_uses_installed_esrgan() {
    let (_tmp, core, rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let model = register_fake_model(&core, "sdxl");
    install_fake_upscaler(&core);
    let mut req = GenerateRequest::txt2img(model, "a cat");
    req.fine_tune.width = Some(64);
    req.fine_tune.height = Some(48);
    let res = generate::generate(&core, req).await.unwrap();
    let src = &res.images[0];
    let before = rec.0.lock().len();
    let up4 = generate::upscale_image(&core, &src.id, 4).await.unwrap();
    assert_eq!(
        up4.origin,
        generate::Origin::Generated,
        "keeps the source's origin"
    );
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

    // Upscales go through the image check like every made picture.
    let mut readings = intimate_adult();
    readings.tags.as_mut().unwrap().minor = 0.9;
    use_check(
        &core,
        FakeCheck {
            readings,
            ..Default::default()
        },
    );
    let kept = core.session.len();
    let e = generate::upscale_image(&core, &src.id, 4)
        .await
        .unwrap_err();
    assert_eq!(e.code, "blocked");
    assert_eq!(core.session.len(), kept);
    // And nothing is made without the check's files.
    use_check(
        &core,
        FakeCheck {
            missing: vec!["nudity"],
            ..Default::default()
        },
    );
    let e = generate::upscale_image(&core, &src.id, 4)
        .await
        .unwrap_err();
    assert_eq!(e.code, "check_missing");
}

/// 2× runs at 4× first: a source over 2048 px per side is refused for 2×
/// too, and the message says so (the 2× size itself would fit).
#[tokio::test]
async fn upscale_too_large_message_names_the_4x_limit() {
    let (_tmp, core, _rec) = new_core();
    let img = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(2100, 1000, [1, 2, 3, 255]),
    )
    .unwrap();
    let e = generate::upscale_image(&core, &img.id, 2)
        .await
        .unwrap_err();
    assert_eq!(e.code, "invalid");
    assert_eq!(e.message, generate::UPSCALE_TOO_LARGE);
    assert!(e.message.contains("4×") && e.message.contains("8192"));
}

/// The first-use upscaler download now runs inside the job (so Cancel
/// reaches it, see `cancel_ends_the_upscaler_download_wait`): a failed
/// download clears the job and adds no result.
#[tokio::test]
async fn failed_upscaler_download_clears_the_job() {
    let (_tmp, core, rec) = new_core();
    core.offline.set(true); // the download fails fast, nothing leaves the machine
    let img = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(8, 8, [1, 2, 3, 255]),
    )
    .unwrap();
    let before = rec.0.lock().len();
    let e = generate::upscale_image(&core, &img.id, 4)
        .await
        .unwrap_err();
    assert_eq!(e.code, "offline", "{e:?}");
    assert!(core.gen.active.lock().is_none());
    assert!(!rec.0.lock()[before..]
        .iter()
        .any(|e| matches!(e, CoreEvent::Generation(_))));
    assert_eq!(core.session.len(), 1, "only the source image");
}

/// Upscale uses the drawing upscaler for a picture without photo-style tags, the photo
/// upscaler for a photo-style one, and the Settings choice over both.
#[tokio::test]
async fn upscale_picks_the_upscaler_by_picture_style() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let model = register_fake_model(&core, "sdxl");
    install_fake_upscaler(&core);
    install_component(
        &core,
        ModelKind::Upscaler,
        "RealESRGAN_x4plus_anime_6B.pth",
        generate::UPSCALER_DRAWING_COMPONENT,
    );
    let mut req = GenerateRequest::txt2img(model, "a cat");
    req.fine_tune.width = Some(64);
    req.fine_tune.height = Some(48);
    let src = generate::generate(&core, req).await.unwrap().images[0].clone();
    let style = |realistic: f32| pinhole_check::Readings {
        tags: Some(pinhole_check::Tags {
            general: 0.9,
            realistic,
            ..Default::default()
        }),
        ..Default::default()
    };

    use_check(
        &core,
        FakeCheck {
            readings: style(0.0),
            ..Default::default()
        },
    );
    let drawn = generate::upscale_image(&core, &src.id, 4).await.unwrap();
    assert_eq!(drawn.upscaler.as_deref(), Some("drawing"));
    use_check(
        &core,
        FakeCheck {
            readings: style(0.6),
            ..Default::default()
        },
    );
    let photo = generate::upscale_image(&core, &src.id, 4).await.unwrap();
    assert_eq!(photo.upscaler.as_deref(), Some("photo"));
    core.settings.write().upscaler = "drawing".into();
    let forced = generate::upscale_image(&core, &src.id, 2).await.unwrap();
    assert_eq!(forced.upscaler.as_deref(), Some("drawing"));
    install_component(
        &core,
        ModelKind::Upscaler,
        "4xNomosWebPhoto_esrgan.safetensors",
        generate::UPSCALER_PHOTO_TEXTURE_COMPONENT,
    );
    core.settings.write().upscaler = "photo_texture".into();
    let texture = generate::upscale_image(&core, &src.id, 4).await.unwrap();
    assert_eq!(texture.upscaler.as_deref(), Some("photo_texture"));

    let names: Vec<String> = mock
        .upscale_requests()
        .iter()
        .map(|r| r["upscaler"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        names,
        [
            "RealESRGAN_x4plus_anime_6B",
            "RealESRGAN_x4plus",
            "RealESRGAN_x4plus_anime_6B",
            "4xNomosWebPhoto_esrgan"
        ]
    );
}

/// Cancel works while Upscale reads the picture style for the Auto pick: nothing is
/// downloaded or upscaled after it.
#[tokio::test]
#[allow(clippy::await_holding_lock)] // held on purpose: the reading waits for it
async fn cancel_ends_upscale_during_the_style_reading() {
    let (_tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    install_fake_upscaler(&core);
    let img = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(8, 8, [1, 2, 3, 255]),
    )
    .unwrap();
    let fake = FakeCheck::default();
    let (counts, hold) = (fake.counts.clone(), fake.hold_readings.clone());
    use_check(&core, fake);
    let held = hold.lock();
    let c2 = core.clone();
    let id = img.id.clone();
    let task = tokio::spawn(async move { generate::upscale_image(&c2, &id, 4).await });
    for _ in 0..200 {
        if counts.lock().0 > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(counts.lock().0, 1, "the style reading started");
    generate::cancel(&core);
    drop(held);
    let err = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("cancel ends the upscale")
        .unwrap()
        .unwrap_err();
    assert_eq!(err.code, "cancelled");
    assert!(mock.upscale_requests().is_empty());
    assert!(core.gen.active.lock().is_none());
    assert_eq!(core.session.len(), 1, "only the source image");
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
