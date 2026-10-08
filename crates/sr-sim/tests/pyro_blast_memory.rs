//! The memory of a step with a blast: the live bytes at its peak, per cell, counted by an allocator, against the step without one and the budget
//! that the solver checks (288 bytes a cell plus 8192).

use sr_sim::pyro::{Blast, Boundary, Inputs, Simulation, Spec};
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

const N: usize = 48;

/// The peak live bytes per cell of the `steps`-th step (counting from 1) of a simulation that has a blast in every step of the first `pulses`, and
/// the live bytes per cell that it holds when that step is over.
fn peak_per_cell(blast: bool, steps: usize) -> (f64, f64) {
    let h = 0.5;
    let half = N as f64 * h / 2.0;
    let mut sim = Simulation::new(Spec {
        cells: [N; 3],
        origin: [-half; 3],
        voxel_size: h,
        dt: 5e-4,
        boundary: Boundary::Open,
        pressure_iterations: 300,
        pressure_tolerance: 1e-6,
        max_bytes: 1 << 32,
        ..Spec::default()
    })
    .unwrap();
    let blasts =
        if blast { vec![Blast::new([0.0; 3], 0.0, 3.75e9, 1.2, 101_325.0, 1.4, 1.0).unwrap()] } else { vec![] };
    let mut last = (0.0, 0.0);
    for step in 0..steps {
        // the peak of this step alone, from what is held when it starts
        PEAK.store(LIVE.load(Ordering::SeqCst), Ordering::SeqCst);
        sim.step(&Inputs { blasts: blasts.clone(), ..Inputs::default() }).unwrap();
        let cells = (N * N * N) as f64;
        last = (PEAK.load(Ordering::SeqCst) as f64 / cells, LIVE.load(Ordering::SeqCst) as f64 / cells);
        let _ = step;
    }
    last
}

#[test]
fn a_step_with_a_blast_peaks_within_the_budget_and_a_step_after_it_holds_no_flow() {
    // the second step: the first has allocated what a simulation keeps (and a blast of this energy is in the strong phase for several steps)
    let (without, held_without) = peak_per_cell(false, 2);
    let (with, held_with) = peak_per_cell(true, 2);
    println!("peak per cell: without a blast {without:.1} (holds {held_without:.1}), with one {with:.1} (holds {held_with:.1})");
    assert!(with <= 288.0, "a step with a blast peaks at {with:.1} bytes a cell, over the 288 of the budget");
    // the flow of the step before is dropped when the step starts, and the blast's flow makes no copy of the density and the temperature that it only
    // carries: 248 bytes a cell measured before that was done (the previous flow, 24, and a whole copy of the flow's state and two of the smoke, 57)
    assert!(with <= 215.0, "a step with a blast peaks at {with:.1} bytes a cell");
    // the flow of the blast is the step's: what the simulation holds after the step is its state and the flow of this step
    assert!(
        held_with - held_without <= 25.0 + 1.0,
        "{held_with:.1} bytes a cell held after a blast step, {held_without:.1} without"
    );
}
