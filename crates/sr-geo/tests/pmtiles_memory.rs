//! Dedicated process: observe allocation admission while opening a hostile archive.
use sr_geo::pmtiles::{self, Archive, TileType};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct ObservedSystem;
static OBSERVE: AtomicBool = AtomicBool::new(false);
static LARGEST: AtomicUsize = AtomicUsize::new(0);

// SAFETY: allocation and deallocation delegate unchanged to System. Observation
// only updates atomics and never accesses allocated memory or allocates itself.
unsafe impl GlobalAlloc for ObservedSystem {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if OBSERVE.load(Ordering::Relaxed) {
            LARGEST.fetch_max(layout.size(), Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if OBSERVE.load(Ordering::Relaxed) {
            LARGEST.fetch_max(layout.size(), Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if OBSERVE.load(Ordering::Relaxed) {
            LARGEST.fetch_max(size, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: ObservedSystem = ObservedSystem;

#[test]
fn oversized_in_bounds_sections_are_rejected_before_allocation() {
    use std::io::Write;
    let path = std::env::temp_dir().join(format!("sr-pmtiles-memory-{}.pmtiles", std::process::id()));
    let mut bytes = pmtiles::write(&[], TileType::Png, 1, &serde_json::Value::Null);
    bytes[8..16].copy_from_slice(&(pmtiles::HEADER_LEN as u64).to_le_bytes());
    bytes[16..24].copy_from_slice(&(pmtiles::MAX_DECOMPRESSED + 1).to_le_bytes());
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
    file.write_all(&bytes).unwrap();
    file.set_len(pmtiles::HEADER_LEN as u64 + pmtiles::MAX_DECOMPRESSED + 1).unwrap();
    drop(file);
    OBSERVE.store(true, Ordering::Relaxed);
    let result = Archive::open(&path).map(|_| ());
    OBSERVE.store(false, Ordering::Relaxed);
    let largest = LARGEST.load(Ordering::Relaxed);
    std::fs::remove_file(path).unwrap();
    assert!(result.is_err(), "oversized section must be rejected");
    assert!(largest < 1 << 20, "rejection made a {largest}-byte allocation: {result:?}");
}
