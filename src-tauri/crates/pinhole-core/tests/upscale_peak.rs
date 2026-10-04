//! Memory held while an upscaled picture waits for the image check.

mod counting;

use std::sync::Arc;
use std::time::Duration;

use pinhole_core::testing::{use_check, use_external_engine, FakeCheck};
use pinhole_core::{generate, session, AppCore, NullSink, ShippedPaths};
use pinhole_store::datadir::ModelKind;
use pinhole_store::{DataDir, InstalledFile};

#[global_allocator]
static ALLOC: counting::Counting = counting::Counting;

/// A picture that does not compress, so its PNG is about as big as its pixels.
fn noise_png(side: u32) -> Vec<u8> {
    let mut x = 0x2545_f491_u32;
    let px: Vec<u8> = (0..side * side * 4)
        .map(|i| {
            if i % 4 == 3 {
                return 255;
            }
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    pinhole_engine::image::encode_png_rgba(&px, side, side).unwrap()
}

/// Pretend the photo upscaler is installed.
fn install_upscaler(core: &AppCore) {
    let dir = core.data.models(ModelKind::Upscaler);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("RealESRGAN_x4plus.pth");
    std::fs::write(&path, b"dummy").unwrap();
    core.installed.lock().upsert(InstalledFile {
        id: "up".into(),
        rel_path: core.data.relative(&path).unwrap(),
        kind: ModelKind::Upscaler,
        sha256: "0".repeat(64),
        size_bytes: 5,
        family: None,
        component_id: Some(generate::UPSCALER_COMPONENT.into()),
        friendly_name: "ESRGAN".into(),
        civitai: None,
        added_at: 0,
        last_used: None,
        observed_vram_gb: None,
        dtype: None,
        trigger_words: None,
        lookup: None,
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::await_holding_lock)] // held on purpose: the check waits for it
async fn upscale_lets_go_of_the_request_before_the_check() {
    let tmp = tempfile::tempdir().unwrap();
    let core: Arc<AppCore> = AppCore::new(
        ShippedPaths {
            config_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../config"),
        },
        DataDir::at(tmp.path().join("Data"), false),
        Arc::new(NullSink),
    )
    .unwrap();
    let mock = pinhole_engine::testutil::MockSdServer::start().await;
    use_external_engine(&core, &mock.base_url());
    install_upscaler(&core);
    // No style reading first: the only reading is the result's check.
    core.settings.write().upscaler = "photo".into();
    let src = noise_png(1024);
    let src_len = src.len();
    let id = session::import_image(&core, src).unwrap().id;
    let fake = FakeCheck::default();
    let (counts, hold) = (fake.counts.clone(), fake.hold_readings.clone());
    use_check(&core, fake);

    let held = hold.lock();
    let base = counting::live();
    let c2 = core.clone();
    let task = tokio::spawn(async move { generate::upscale_image(&c2, &id, 4).await });
    for _ in 0..1000 {
        if counts.lock().0 > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(counts.lock().0, 1, "the result's check started");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let waiting = counting::live().saturating_sub(base);
    drop(held);
    let up = task.await.unwrap().unwrap();
    assert_eq!((up.width, up.height), (4096, 4096));
    // The source went to the engine as base64 (a third bigger than its PNG); while the check
    // runs only the upscaled PNG (one colour here, so small) is held.
    assert!(
        waiting < src_len / 2,
        "{waiting} bytes held during the check for a {src_len}-byte source"
    );
}
