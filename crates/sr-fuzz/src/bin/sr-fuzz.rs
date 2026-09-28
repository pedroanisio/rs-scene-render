//! `sr-fuzz [--seconds N] [--seed S] [--corpus DIR] [--out DIR]`: fuzzes the
//! document pipeline and the expression compiler for N seconds (default 60)
//! and writes every crashing input to the output directory.

use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let seconds: u64 = get("--seconds").and_then(|s| s.parse().ok()).unwrap_or(60);
    let seed: u64 = get("--seed").and_then(|s| s.parse().ok()).unwrap_or(0x5eed);
    let corpus = get("--corpus").unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus").into());
    let out = std::path::PathBuf::from(get("--out").unwrap_or_else(|| "fuzz-crashes".into()));
    let seeds = sr_fuzz::corpus(std::path::Path::new(&corpus));
    // panics are caught; keep their messages off the terminal
    std::panic::set_hook(Box::new(|_| {}));
    let start = Instant::now();
    let limit = Duration::from_secs(seconds);
    let mut last = Instant::now();
    let c = sr_fuzz::fuzz(seed, &seeds, |c| {
        if last.elapsed() > Duration::from_secs(60) {
            eprintln!(
                "{:>6.0} s: {} documents, {} expressions, {} crashes",
                start.elapsed().as_secs_f64(),
                c.docs,
                c.exprs,
                c.crashes.len()
            );
            last = Instant::now();
        }
        start.elapsed() >= limit
    });
    let secs = start.elapsed().as_secs_f64();
    println!(
        "{} seeds; {secs:.0} s: {} documents ({:.0}/s), {} expressions ({:.0}/s), {} crashes",
        seeds.len(),
        c.docs,
        c.docs as f64 / secs,
        c.exprs,
        c.exprs as f64 / secs,
        c.crashes.len()
    );
    if !c.crashes.is_empty() {
        std::fs::create_dir_all(&out).ok();
        for (k, crash) in c.crashes.iter().enumerate() {
            let p = out.join(format!("{k:04}-{}.txt", crash.target));
            std::fs::write(&p, &crash.input).ok();
            println!("{}: {} ({})", p.display(), crash.message, crash.target);
        }
        std::process::exit(1);
    }
}
