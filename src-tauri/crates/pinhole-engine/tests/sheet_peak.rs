//! Memory held while a sheet is built. Its own test binary, so the counting allocator only
//! sees this test.

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
fn sheet_with_holds_one_picture_at_a_time() {
    const SIDE: u32 = 2048;
    let tile = (SIDE * SIDE * 4) as usize;
    let sizes = [(SIDE, SIDE); 4];
    let base = LIVE.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let (px, w, h) = pinhole_engine::image::sheet_with::<()>(
        &sizes,
        (0..4u8).map(|i| Ok((vec![i * 40 + 1; tile], SIDE, SIDE))),
    )
    .unwrap();
    let peak = PEAK.load(Ordering::SeqCst) - base;
    let out = (w * h * 4) as usize;
    assert_eq!(px.len(), out);
    // The sheet plus the one picture being placed, which is taken without a copy.
    assert!(
        peak < out + tile + tile / 2,
        "peak {peak} bytes for a {out}-byte sheet of {tile}-byte pictures"
    );
}
