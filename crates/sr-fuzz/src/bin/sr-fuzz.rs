//! `sr-fuzz [--seconds N] [--seed S] [--corpus DIR] [--out DIR] [--target NAME|all]`: fuzzes the
//! document pipeline and the expression compiler for N seconds (default 60)
//! and writes every crashing input to the output directory. `--target` fuzzes
//! a byte-level target instead (`all`: each in turn, sharing the time).

use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let seconds: u64 = get("--seconds").and_then(|s| s.parse().ok()).unwrap_or(60);
    let seed: u64 = get("--seed").and_then(|s| s.parse().ok()).unwrap_or(0x5eed);
    let corpus = get("--corpus").unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus").into());
    let out = std::path::PathBuf::from(get("--out").unwrap_or_else(|| "fuzz-crashes".into()));
    // panics are caught; keep their messages off the terminal, and each crash says where it panicked
    sr_fuzz::quiet_panics();
    if let Some(name) = get("--target") {
        // each place that panics, once
        static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
        std::panic::set_hook(Box::new(|info| {
            let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
            let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
            if !seen.contains(&at) {
                eprintln!("panic at {at}");
                seen.push(at);
            }
        }));
        std::process::exit(targets(&name, seconds, seed, &out));
    }
    let seeds = sr_fuzz::corpus(std::path::Path::new(&corpus));
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

/// Fuzzes the byte-level target `name` (or all of them); the exit code.
fn targets(name: &str, seconds: u64, seed: u64, out: &std::path::Path) -> i32 {
    use sr_fuzz::targets::{fuzz, target, TARGETS};
    let list: Vec<_> = match name {
        "all" => TARGETS.iter().collect(),
        n => match target(n) {
            Some(t) => vec![t],
            None => {
                let names: Vec<&str> = TARGETS.iter().map(|t| t.name).collect();
                eprintln!("unknown target {n}; targets: all, {}", names.join(", "));
                return 2;
            }
        },
    };
    let each = Duration::from_secs_f64(seconds as f64 / list.len() as f64);
    let mut found = 0;
    for t in list {
        let start = Instant::now();
        let c = fuzz(seed, t, |_| start.elapsed() >= each);
        let secs = start.elapsed().as_secs_f64();
        println!("{}: {} inputs ({:.0}/s), {} crashes", t.name, c.inputs, c.inputs as f64 / secs, c.crashes.len());
        if !c.crashes.is_empty() {
            std::fs::create_dir_all(out).ok();
        }
        for crash in &c.crashes {
            let p = out.join(format!("{found:04}-{}.bin", crash.target));
            std::fs::write(&p, &crash.input).ok();
            println!("{}: {}", p.display(), crash.message);
            found += 1;
        }
    }
    (found > 0) as i32
}
