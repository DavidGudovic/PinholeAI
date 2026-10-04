//! Memory held while Save as one sheet builds its picture.

mod counting;

use std::sync::Arc;

use pinhole_core::{session, AppCore, NullSink, ShippedPaths};
use pinhole_store::DataDir;

#[global_allocator]
static ALLOC: counting::Counting = counting::Counting;

#[test]
fn a_sheet_decodes_one_picture_at_a_time() {
    // Tall pictures: the sheet is scaled well down, so it is much smaller than the pictures.
    const W: u32 = 500;
    const H: u32 = 16_000;
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
    let ims: Vec<_> = (0..4u8)
        .map(|i| {
            let png = pinhole_engine::testutil::solid_png(W, H, [i * 40, 90, 200, 255]);
            let id = session::import_image(&core, png).unwrap().id;
            core.session.get(&id).unwrap()
        })
        .collect();
    let tile = (W * H * 4) as usize;
    let base = counting::reset_peak();
    let png = session::export_png(&core, &ims).unwrap();
    let peak = counting::peak() - base;
    let (w, h) = pinhole_engine::png::dimensions(&png).unwrap();
    let out = (w * h * 4) as usize;
    assert!(out < tile / 2);
    // One decoded picture and the scratch of shrinking it, not all four pictures at once.
    assert!(
        peak < 3 * tile,
        "peak {peak} bytes for a {out}-byte sheet of {tile}-byte pictures"
    );
}
