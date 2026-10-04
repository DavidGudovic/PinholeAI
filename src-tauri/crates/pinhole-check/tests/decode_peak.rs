//! Memory held while a picture is decoded for the check. Its own test binary, so the counting
//! allocator only sees this test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(by: usize) {
    let now = LIVE.fetch_add(by, Ordering::SeqCst) + by;
    PEAK.fetch_max(now, Ordering::SeqCst);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
            grew(new_size);
        }
        p
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

#[test]
fn an_rgb_png_is_decoded_without_a_copy() {
    const SIDE: u32 = 2048;
    let img = image::RgbImage::from_pixel(SIDE, SIDE, image::Rgb([40, 90, 200]));
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let pixels = (SIDE * SIDE * 3) as usize;
    let base = LIVE.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let rgb = pinhole_check::run::decode(&png).unwrap();
    let peak = PEAK.load(Ordering::SeqCst) - base;
    assert_eq!(rgb.as_raw().len(), pixels);
    // The decoded pixels are the result's own buffer.
    assert!(
        peak < pixels + pixels / 2,
        "peak {peak} bytes for {pixels} bytes of pixels"
    );
}
