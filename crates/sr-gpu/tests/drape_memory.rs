//! All allocations in this isolated test binary count, including Rayon workers.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
struct Counting;
fn added(n: usize) {
    PEAK.fetch_max(LIVE.fetch_add(n, Relaxed) + n, Relaxed);
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            added(l.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            added(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let out = unsafe { System.realloc(p, l, n) };
        if !out.is_null() {
            LIVE.fetch_sub(l.size(), Relaxed);
            added(n);
        }
        out
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;
#[test]
fn drapes_use_one_float_frame_plus_output_and_bounded_worker_bands() {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap();
    let d = sr_text::Drawing::default();
    pool.install(|| {
        sr_gpu::drape::rasterize(&d, [16, 16]);
    });
    let base = LIVE.load(Relaxed);
    PEAK.store(base, Relaxed);
    let got = pool.install(|| sr_gpu::drape::rasterize(&d, [512, 513]));
    let peak = PEAK.load(Relaxed).saturating_sub(base);
    println!("drape peak additional allocation: {peak} bytes for 512x513");
    assert!(got.iter().all(|&v| v == 0));
    assert_eq!(got.len(), 512 * 513 * 4);
    assert!(peak < 512 * 513 * 32, "peak {peak} bytes retains extra full float images");
    for size in [[0, 0], [0, 3], [3, 0]] {
        assert!(sr_gpu::drape::rasterize(&d, size).is_empty());
    }
}
