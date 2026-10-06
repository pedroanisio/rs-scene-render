//! Target-resolution impact workload. It evaluates the shipped high-resolution
//! scene without a GPU, so it takes minutes and gigabytes: run it on request with
//! `cargo test --release -p sr-eval --test hero_hires -- --ignored --nocapture`.
use sr_eval::{Evaluator, FrameGraph, FrameNode};
use std::time::Instant;

fn node<'a>(frame: &'a FrameGraph, id: &str) -> &'a FrameNode {
    frame.nodes.iter().find(|node| &*node.id == id).unwrap()
}

/// Peak resident set size of this process, in MiB.
fn peak_rss_mib() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find(|l| l.starts_with("VmHWM"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|kib| kib.parse::<f64>().ok())
        .map_or(f64::NAN, |kib| kib / 1024.0)
}

#[test]
#[ignore = "target-resolution workload: minutes of CPU and gigabytes of memory"]
fn hires_hero_scene_evaluates_and_replays_backwards_without_a_gpu() {
    let xml = include_str!("../../../examples/cinematic-impact/hero-hires.scene.xml");
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let evaluator = Evaluator::new(&doc, &Default::default()).unwrap();
    let sample = |time: f64| {
        let start = Instant::now();
        let frame = evaluator.evaluate(time);
        println!("HIRES t={time}: {:.2} s wall, peak RSS {:.0} MiB", start.elapsed().as_secs_f64(), peak_rss_mib());
        assert!(frame.problems.is_empty(), "{time}: {:?}", frame.problems);
        frame
    };
    let density_present = |frame: &FrameGraph| {
        let plume = node(frame, "plume").sim_volume.as_ref().expect("plume volume");
        plume.data.grid("density").unwrap().bricks().any(|(_, b)| b.iter().any(|&v| v > 0.))
    };

    let early = sample(0.5);
    assert!(node(&early, "sea").sim_ocean.is_some());
    assert!(node(&early, "ejecta").particles3d.as_ref().unwrap().frame.particles.is_empty());
    sample(1.0);
    let after = sample(1.5);
    assert!(density_present(&after), "no density 0.5 s after the impact");
    let ejecta = node(&after, "ejecta").particles3d.as_ref().unwrap().frame.particles.len();
    println!("HIRES t=1.5: {ejecta} ejecta particles");
    assert!(ejecta > 0);
    let late = sample(3.0);
    assert!(density_present(&late), "no density 2 s after the impact");
    assert!(node(&late, "sea").sim_ocean.is_some());
    assert!(!node(&late, "ejecta").particles3d.as_ref().unwrap().frame.particles.is_empty());

    let replay = sample(1.5);
    assert_eq!(
        node(&replay, "sea").sim_ocean.as_ref().unwrap().frame,
        node(&after, "sea").sim_ocean.as_ref().unwrap().frame
    );
    assert_eq!(
        node(&replay, "ejecta").particles3d.as_ref().unwrap().frame,
        node(&after, "ejecta").particles3d.as_ref().unwrap().frame
    );
    assert_eq!(
        node(&replay, "plume").sim_volume.as_ref().unwrap().key,
        node(&after, "plume").sim_volume.as_ref().unwrap().key
    );

    // The end of the scene, reached by a forward seek from the replayed frame.
    let end = sample(6.0);
    assert!(node(&end, "sea").sim_ocean.is_some());
}
