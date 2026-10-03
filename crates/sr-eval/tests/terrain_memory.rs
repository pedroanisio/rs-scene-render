//! Dedicated process: observe allocations only during one hostile tile read.
use image::ImageEncoder;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

static TRACK: AtomicBool = AtomicBool::new(false);
static LARGEST: AtomicUsize = AtomicUsize::new(0);
struct ObservedSystem;
fn observe(size: usize) {
    if TRACK.load(Relaxed) {
        LARGEST.fetch_max(size, Relaxed);
    }
}
// SAFETY: all allocation operations delegate unchanged to System. Observation
// uses only atomics, never allocates, and does not access allocated memory.
unsafe impl GlobalAlloc for ObservedSystem {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        observe(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        observe(size);
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: ObservedSystem = ObservedSystem;

#[test]
fn terrain_png_metadata_cannot_allocate_past_the_budget_during_header_inspection() {
    let mut png = Vec::new();
    let mut encoder = image::codecs::png::PngEncoder::new(&mut png);
    encoder.set_icc_profile(vec![0; 8 << 20]).unwrap();
    encoder.write_image(&[128, 100, 0].repeat(4), 2, 2, image::ExtendedColorType::Rgb8).unwrap();
    assert!(png.len() < 65536, "the hostile metadata must fit the encoded section limit");
    let archive =
        sr_geo::pmtiles::write(&[((0, 0, 0), png)], sr_geo::pmtiles::TileType::Png, 1, &serde_json::json!({}));
    let path = std::env::temp_dir().join(format!("sr-terrain-metadata-{}.pmtiles", std::process::id()));
    std::fs::write(&path, archive).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _clean = Clean(path.clone());
    let mut dem = sr_eval::terrain::Dem::open(&path, 0, sr_eval::geo::DemEncoding::Terrarium, 2, 1 << 20).unwrap();
    TRACK.store(true, Relaxed);
    let result = dem.sample(0., 0., sr_eval::terrain::Missing::Error);
    TRACK.store(false, Relaxed);
    let largest = LARGEST.load(Relaxed);
    assert!(largest <= 1 << 20, "1 MiB terrain read made a {largest}-byte allocation: {result:?}");
}
