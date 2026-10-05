//! Wall time of the ocean solver per canonical 1/24 s step on a square grid.
//!
//! usage: cargo run --release -p sr-sim --example ocean_profile -- ORDER THREADS SIDE
//!
//! ORDER is 1 or 2, THREADS the size of the rayon pool and SIDE the number of cells
//! per side (1024 gives 1.05 million cells, 2000 gives 4 million). The water is 10
//! deep with 0.5-unit cells, g = 9.81, closed edges and a Gaussian hump on top. The
//! first step runs from the initial state and the second from the first; both are
//! printed with the load average at the start, and the peak resident memory.
use sr_sim::ocean::{Boundary, Cell, Ocean, Order, Spec};
use std::time::Instant;

fn field(path: &str, key: &str) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .find(|l| l.starts_with(key))
        .map(|l| l.trim_start_matches(key).trim_start_matches(':').trim().to_string())
        .unwrap_or_default()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [order, threads, side] = args.as_slice() else {
        eprintln!("usage: ocean_profile ORDER(1|2) THREADS SIDE");
        std::process::exit(2);
    };
    let order = if order == "2" { Order::Second } else { Order::First };
    let threads: usize = threads.parse().expect("threads");
    let n: usize = side.parse().expect("side");
    let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let load = load.split_whitespace().next().unwrap_or("?").to_string();

    let (h, g, dx, dt) = (10.0, 9.81, 0.5, 1.0 / 24.0);
    let spec = Spec {
        cells: [n, n],
        origin: [-(n as f64) * dx / 2.0; 2],
        cell_size: dx,
        dt,
        gravity: g,
        boundary: Boundary::Closed,
        order,
        max_work: 1 << 50,
        max_bytes: 4 << 30,
        checkpoint_bytes: 0,
        ..Default::default()
    };
    let cells: Vec<_> = (0..n * n)
        .map(|i| {
            let (x, z) = ((i % n) as f64 - n as f64 / 2.0, (i / n) as f64 - n as f64 / 2.0);
            Cell { depth: h + 0.3 * (-(x * x + z * z) / 800.0).exp(), velocity: [0.0; 2] }
        })
        .collect();
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("pool");
    pool.install(|| {
        let mut ocean = Ocean::new(spec, vec![h; n * n], cells, vec![]).expect("ocean");
        let first = Instant::now();
        ocean.at(dt).expect("first step");
        let first = first.elapsed().as_secs_f64();
        let second = Instant::now();
        ocean.at(2.0 * dt).expect("second step");
        let second = second.elapsed().as_secs_f64();
        let peak = field("/proc/self/status", "VmHWM").trim_end_matches("kB").trim().parse::<f64>().unwrap_or(0.0);
        println!(
            "order {} threads {threads} cells {} load {load}: step1 {first:.3} s, step2 {second:.3} s, best {:.3} s, peak RSS {:.0} MiB",
            if order == Order::First { 1 } else { 2 },
            n * n,
            first.min(second),
            peak / 1024.0
        );
    });
}
