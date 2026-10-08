//! Exporting a smoke to a volume stays within the bytes the export declares it needs: the bricks of a channel are
//! not all materialised before they are stored.

use sr_sim::pyro::{Boundary, Impulse, Inputs, Shape, Simulation, Source, Spec};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            let now = LIVE.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            PEAK.fetch_max(now, Ordering::SeqCst);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        System.dealloc(p, layout);
        LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        let q = System.realloc(p, layout, new);
        if !q.is_null() {
            if new >= layout.size() {
                let now = LIVE.fetch_add(new - layout.size(), Ordering::SeqCst) + new - layout.size();
                PEAK.fetch_max(now, Ordering::SeqCst);
            } else {
                LIVE.fetch_sub(layout.size() - new, Ordering::SeqCst);
            }
        }
        q
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn a_dense_export_peaks_within_the_bytes_it_declares() {
    // every cell of the domain holds smoke, heat and a flow, so every brick of every channel is stored
    let cells = [96, 96, 96];
    let mut sim = Simulation::new(Spec {
        cells,
        dt: 0.1,
        boundary: Boundary::Open,
        pressure_iterations: 300,
        pressure_tolerance: 1e-3,
        ..Spec::default()
    })
    .unwrap();
    let everywhere = Shape::Box { min: [0.0; 3], max: [96.0; 3] };
    sim.step(&Inputs {
        sources: vec![Source { shape: everywhere, density_rate: 1.0, temperature_rate: 100.0, ..Source::default() }],
        impulses: vec![Impulse {
            shape: Shape::Box { min: [0.0; 3], max: [96.0; 3] },
            time: 0.0,
            density: 0.0,
            temperature: 0.0,
            velocity: [1.0, 2.0, 3.0],
            expansion: 0.0,
        }],
        ..Inputs::default()
    })
    .unwrap();
    let bricks: usize = cells.iter().map(|n| n.div_ceil(8)).product();
    let declared = bricks * 5 * (512 * 4 + 128) + 4096;
    let before = LIVE.load(Ordering::SeqCst);
    PEAK.store(before, Ordering::SeqCst);
    let volume = sim.state().volume(declared).unwrap();
    let peak = PEAK.load(Ordering::SeqCst) - before;
    let kept = LIVE.load(Ordering::SeqCst) - before;
    println!("EXPORT declared {declared} bytes, peak {peak}, kept {kept}");
    drop(volume);
    // the peak is the grids and what is being built to go into them, and the declaration counts the grids
    assert!(peak as f64 <= 1.10 * declared as f64, "peak {peak} bytes against {declared} declared");
}
