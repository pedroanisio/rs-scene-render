//! Wall time of driven ocean steps with a moving body, with and without owner tags.
//!
//! usage: cargo run --release -p sr-sim --example ocean_bodies_profile -- ORDER THREADS SIDE STEPS
//!
//! A 40-unit square body of thickness 3 crosses the water at 8 units per second over a flat bed
//! of 10 (0.5-unit cells, g = 9.81, canonical step 1/24 s). Each mode runs STEPS canonical steps
//! on a fresh solver, three times, and prints the time per step of each run.
use sr_sim::ocean::{Boundary, Cell, Forcing, Lift, Ocean, Order, Spec};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [order, threads, side, steps] = args.as_slice() else {
        eprintln!("usage: ocean_bodies_profile ORDER(1|2) THREADS SIDE STEPS");
        std::process::exit(2);
    };
    let order = if order == "2" { Order::Second } else { Order::First };
    let (threads, n, steps): (usize, usize, u64) =
        (threads.parse().unwrap(), side.parse().unwrap(), steps.parse().unwrap());
    let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    println!("load {}", load.split_whitespace().next().unwrap_or("?"));
    let (h, dx, dt) = (10.0, 0.5, 1.0 / 24.0);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("pool");
    for round in 0..5 {
        // owners 0: no tags; 1: tags without lifts; 2: tags and the lift of the body (the pressure credited)
        for owners in [0usize, 1, 2] {
            let spec = Spec {
                cells: [n, n],
                origin: [-(n as f64) * dx / 2.0; 2],
                cell_size: dx,
                dt,
                order,
                boundary: Boundary::Closed,
                moving_bed: true,
                bodies: true,
                body_owners: owners.min(1),
                max_work: 1 << 50,
                max_bytes: 4 << 30,
                checkpoint_bytes: 0,
                ..Default::default()
            };
            let cells = vec![Cell { depth: h, velocity: [0.0; 2] }; n * n];
            let seconds = pool.install(|| {
                let mut ocean = Ocean::new(spec, vec![h; n * n], cells, vec![]).expect("ocean");
                let mut driver = |time: f64, f: &mut Forcing| -> Result<(), sr_sim::ocean::Error> {
                    let left = -(n as f64) * dx / 2.0 + 8.0 * time;
                    let half = (20.0 / dx) as usize;
                    let mut lifted = Vec::new();
                    for c in 0..n * n {
                        let (x, z) =
                            ((c % n) as f64 * dx - (n as f64) * dx / 2.0, (c / n) as f64 * dx - (n as f64) * dx / 2.0);
                        let inside = x >= left && x < left + 2.0 * half as f64 * dx && z.abs() < half as f64 * dx;
                        let t = if inside { 3.0 } else { 0.0 };
                        f.occupancy[c] = t;
                        f.bed[c] = h - t;
                        f.velocity[c] = if inside { [8.0, 0.0] } else { [0.0; 2] };
                        if owners > 0 {
                            f.owner[c] = 0;
                        }
                        if owners > 1 && inside {
                            lifted.push((c as u32, t));
                        }
                    }
                    if owners > 1 {
                        f.lifts = vec![Lift { owner: 0, columns: lifted }];
                    }
                    Ok(())
                };
                ocean.at_driven(dt, &mut driver).expect("first step");
                let started = Instant::now();
                ocean.at_driven((steps + 1) as f64 * dt, &mut driver).expect("steps");
                started.elapsed().as_secs_f64() / steps as f64
            });
            println!("round {round}: owners {owners}: {seconds:.4} s per step");
        }
    }
}
