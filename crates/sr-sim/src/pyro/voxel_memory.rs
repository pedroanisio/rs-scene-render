//! The list of solid-cell face velocities is built to its final size: the 56 bytes a solid cell is charged are what
//! it holds at the peak, not a vector that doubles and holds the old copy and the new one together.

use super::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

thread_local! {
    static LIVE: Cell<usize> = const { Cell::new(0) };
    static PEAK: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            LIVE.with(|l| {
                l.set(l.get() + layout.size());
                PEAK.with(|k| k.set(k.get().max(l.get())));
            });
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        System.dealloc(p, layout);
        LIVE.with(|l| l.set(l.get().saturating_sub(layout.size())));
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        let q = System.realloc(p, layout, new);
        if !q.is_null() {
            LIVE.with(|l| {
                l.set((l.get() + new).saturating_sub(layout.size()));
                PEAK.with(|k| k.set(k.get().max(l.get())));
            });
        }
        q
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn a_domain_of_solid_cells_peaks_at_what_it_is_charged() {
    // 266 240 cells, just past a power of two, which is where a vector that grows by doubling holds the most
    let cells = [65, 64, 64];
    let count: usize = cells.iter().product();
    let everything = Obstacle::stationary(Shape::Box { min: [-1e6; 3], max: [1e6; 3] });
    let before = LIVE.with(Cell::get);
    PEAK.with(|k| k.set(before));
    let (solid, faces) = voxelize(cells, [0.0; 3], 1.0, &[everything]).unwrap();
    let peak = PEAK.with(Cell::get) - before;
    assert_eq!((solid.iter().filter(|s| **s).count(), faces.len()), (count, count));
    let charged = count * (1 + std::mem::size_of::<SolidFaces>());
    println!("VOXEL peak {peak} bytes for {charged} charged");
    assert!(peak as f64 <= 1.05 * charged as f64, "peak {peak} bytes against {charged} charged");
}
