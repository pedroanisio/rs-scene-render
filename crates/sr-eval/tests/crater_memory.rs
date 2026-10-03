//! Dedicated process: observe crater geometry admission before tessellation.
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
fn crater_budget_precedes_initial_rigid_surface_allocation() {
    let xml = r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="1"/>
    <composition><object3D id="ground" primitive="plane" segments="256">
    <crater maxMemoryMiB="1"/><rigidBody type="static"/></object3D></composition></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    TRACK.store(true, Relaxed);
    let result = ev.physics_cache();
    TRACK.store(false, Relaxed);
    assert!(result.unwrap_err().contains("crater collider exceeds memory budget"));
    let largest = LARGEST.load(Relaxed);
    assert!(largest <= 1 << 20, "1 MiB crater admission made a {largest}-byte allocation");

    // The smoke region has a closed, tessellated slab, so admission must
    // precede both the grid and its topology/BVH construction as well.
    let xml = r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/>
    <composition><object3D id="ground" primitive="plane" segments="256"><crater maxMemoryMiB="1"/></object3D>
    <object3D id="cloud" primitive="volume"><pyro width="4" height="4" depth="4" voxelSize="1" colliders="ground"/></object3D>
    </composition></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    LARGEST.store(0, Relaxed);
    TRACK.store(true, Relaxed);
    let frame = ev.evaluate(0.1);
    TRACK.store(false, Relaxed);
    assert!(
        frame.problems.iter().any(|e| e.contains("crater pyro tessellation exceeds geometry budget")),
        "{:?}",
        frame.problems
    );
    let largest = LARGEST.load(Relaxed);
    assert!(largest <= 1 << 20, "1 MiB pyro crater admission made a {largest}-byte allocation");
}
