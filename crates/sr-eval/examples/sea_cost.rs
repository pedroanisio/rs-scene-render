use sr_eval::Evaluator;
fn main() {
    let path = std::env::args().nth(1).expect("scene path");
    let xml = std::fs::read_to_string(path).unwrap();
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = Evaluator::new(&doc, &Default::default()).unwrap();
    let started = std::time::Instant::now();
    let mut slowest = 0.0f64;
    for k in 0..144 {
        let t0 = std::time::Instant::now();
        let frame = ev.evaluate(k as f64 / 24.0);
        assert!(
            frame.failures.is_empty() && frame.problems.is_empty(),
            "frame {k}: {:?} {:?}",
            frame.failures,
            frame.problems
        );
        slowest = slowest.max(t0.elapsed().as_secs_f64());
    }
    let total = started.elapsed().as_secs_f64();
    let hwm = std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<f64>().ok())
        .map_or(f64::NAN, |kb| kb / 1024.0);
    let frame = ev.evaluate(143.0 / 24.0);
    let alive = frame
        .nodes
        .iter()
        .find(|n| &*n.id == "debris")
        .and_then(|n| n.particles3d.as_ref())
        .map(|p| p.frame.particles.len());
    println!("COST alive at 6 s {alive:?}");
    println!("COST total {total:.2} s, {:.4} s/frame, slowest {slowest:.2} s, peak RSS {hwm:.0} MiB", total / 144.0);
}
