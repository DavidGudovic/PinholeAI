//! Pictures brought in and exported: metadata removed, AI marker added.

use super::*;

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

/// RELEASE-SPEC §2: every picture Pinhole made leaves with the AI-generated
/// marker (Save, Save as, Copy), whatever the settings say; an untouched
/// import leaves without one.
#[tokio::test]
async fn exports_carry_the_ai_marker() {
    let (tmp, core, _rec) = new_core();
    let mock = MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    let sdxl = register_fake_model(&core, "sdxl");
    let xmp = |bytes: &[u8]| -> Option<String> {
        pinhole_engine::png::text_chunks(bytes)
            .into_iter()
            .find(|(k, d)| k == "iTXt" && d.starts_with(b"XML:com.adobe.xmp\0"))
            .map(|(_, d)| String::from_utf8_lossy(&d[22..]).into_owned())
    };
    let made = generate::generate(&core, GenerateRequest::txt2img(sdxl.clone(), "a boat"))
        .await
        .unwrap()
        .images[0]
        .id
        .clone();
    let photo = session::import_image(
        &core,
        pinhole_engine::testutil::solid_png(64, 64, [10, 20, 30, 255]),
    )
    .unwrap()
    .id;
    let mut req = GenerateRequest::txt2img(sdxl, "at sunset");
    req.mode = GenMode::Img2img;
    req.init_image_id = Some(photo.clone());
    req.strength = Some(0.5);
    let restyled = generate::generate(&core, req).await.unwrap().images[0]
        .id
        .clone();

    for with_settings in [false, true] {
        core.settings.write().saved_metadata =
            if with_settings { "settings" } else { "none" }.into();
        let saved = std::fs::read(session::save_image(&core, &made).unwrap().path).unwrap();
        let m = xmp(&saved).expect("marker on a Create result");
        assert!(
            m.contains("digitalsourcetype/trainedAlgorithmicMedia"),
            "{m}"
        );
        assert!(!m.contains("Pinhole"), "no app name: {m}");
        assert!(!m.contains("a boat"));
        let path = tmp.path().join(format!("as_{with_settings}.png"));
        let saved = std::fs::read(
            session::save_image_as(&core, &restyled, path.to_str().unwrap())
                .unwrap()
                .path,
        )
        .unwrap();
        let m = xmp(&saved).expect("marker on an edited photo");
        assert!(
            m.contains("digitalsourcetype/compositeWithTrainedAlgorithmicMedia"),
            "{m}"
        );
        let saved = std::fs::read(session::save_image(&core, &photo).unwrap().path).unwrap();
        assert!(xmp(&saved).is_none(), "an untouched import isn't AI-made");
    }
    // The pixel watermark: on Save and Copy of what Pinhole made, not on an untouched import.
    let marked = |bytes: &[u8]| {
        let (px, w, h) = pinhole_engine::image::decode_rgba(bytes).unwrap();
        pinhole_engine::watermark::is_marked(&px, w, h)
    };
    let saved = std::fs::read(session::save_image(&core, &made).unwrap().path).unwrap();
    assert!(marked(&saved));
    let (px, w, h) = session::decode_rgba(&core, &made).unwrap();
    assert!(pinhole_engine::watermark::is_marked(&px, w, h), "Copy");
    let saved = std::fs::read(session::save_image(&core, &photo).unwrap().path).unwrap();
    assert!(!marked(&saved));
    // What stays in memory (shown on screen) is unchanged.
    assert!(!marked(&session::get(&core, &made).unwrap()));
}
