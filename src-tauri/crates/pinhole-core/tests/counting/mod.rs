//! A global allocator that counts the bytes held, for tests that check how much memory a
//! step needs. Each test file is its own binary, so a count only covers that file's test.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Counting;

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

/// Bytes held now.
#[allow(dead_code)] // not every test file uses every helper
pub fn live() -> usize {
    LIVE.load(Ordering::SeqCst)
}

/// Start counting the peak from what is held now; returns that.
#[allow(dead_code)]
pub fn reset_peak() -> usize {
    let now = live();
    PEAK.store(now, Ordering::SeqCst);
    now
}

/// Most bytes held since [`reset_peak`].
#[allow(dead_code)]
pub fn peak() -> usize {
    PEAK.load(Ordering::SeqCst)
}
