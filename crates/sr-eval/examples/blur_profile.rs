//! Cost of evaluating frames the way a motion-blurred render asks for them: the frame, then
//! the shutter's samples (first, last, then each in order), as the renderer's sub-frame
//! source does.
//!
//! `cargo run --release -p sr-eval --example blur_profile -- scene.xml [first=0] [frames=40] [samples=16] [shutter=180]`
use sr_eval::Evaluator;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("scene path");
    let mut number = |default: f64| args.next().map_or(default, |v| v.parse().expect("number"));
    let (first, frames, samples, shutter) = (number(0.) as u64, number(40.) as u64, number(16.) as usize, number(180.));
    let xml = std::fs::read_to_string(&path).expect("scene");
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    let ev = Evaluator::new(&doc, &Default::default()).unwrap();
    let fps = doc.scene.project.fps.as_f64();
    let (mut whole, mut sub, mut particles, mut rigid) = (0., 0., 0., 0.);
    let mut key = 0u64;
    for n in first..first + frames {
        let t = n as f64 / fps;
        let started = Instant::now();
        let g = ev.evaluate(t);
        whole += started.elapsed().as_secs_f64();
        assert!(g.failures.is_empty() && g.problems.is_empty(), "{:?} {:?}", g.failures, g.problems);
        particles += g.sim_seconds.particles;
        rigid += g.sim_seconds.rigid;
        let times: Vec<f64> =
            (0..samples).map(|k| t + shutter / 360. * (k as f64 + 0.5) / samples as f64 / fps).collect();
        let started = Instant::now();
        let mut seen = std::collections::HashSet::new();
        for &s in [times[0], times[samples - 1]].iter().chain(&times) {
            // the renderer's sub-frame source keeps what it evaluated at each instant
            if !seen.insert((s * 1e6).round() as i64) {
                continue;
            }
            let g = ev.evaluate(s);
            assert!(g.failures.is_empty() && g.problems.is_empty(), "{:?} {:?}", g.failures, g.problems);
            particles += g.sim_seconds.particles;
            rigid += g.sim_seconds.rigid;
            for node in &g.nodes {
                if let Some(p) = &node.particles3d {
                    key = key.rotate_left(7) ^ p.key;
                }
            }
        }
        sub += started.elapsed().as_secs_f64();
    }
    println!(
        "frames {frames}, samples {samples}: frame {whole:.2} s, shutter samples {sub:.2} s (particles {particles:.2} s, rigid {rigid:.2} s in all); key {key:016x}"
    );
}
