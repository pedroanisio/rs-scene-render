//! The ocean responds to a crater that deforms the seabed, with no authored impulse.
use sr_eval::{Evaluator, FrameGraph};

/// A 128 x 128 ocean of 2-unit cells over a 12-unit layer of water, and a seabed
/// plane at the same depth whose crater (radius 40, rim 3) grows from 0.5 s to 1.5 s.
fn scene(crater_depth: f64, extra: &str) -> String {
    format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
          <object3D id="seabed" primitive="plane" width="400" height="400" segments="80" y="12" rotationX="-90">
            <crater radius="40" depth="{crater_depth}" rimHeight="3" rimWidth="8" start="0.5" end="1.5"/>
          </object3D>
          <ocean id="sea" width="128" depth="128" cellSize="2" bottomDepth="12" dt="0.0416666666666667" boundary="closed" colliders="seabed">{extra}</ocean>
        </composition></scene>"#
    )
}
fn evaluator(xml: &str) -> Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    Evaluator::new(&doc, &Default::default()).unwrap()
}
fn sea(frame: &FrameGraph) -> &sr_eval::ocean::SimOcean {
    frame.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap()
}
/// Largest change of water depth from the 12-unit rest layer.
fn peak(frame: &FrameGraph) -> f64 {
    sea(frame).frame.cells.iter().map(|c| (c.depth - 12.0).abs()).fold(0.0, f64::max)
}

#[test]
fn a_crater_alone_makes_a_wave_and_conserves_the_water() {
    let ev = evaluator(&scene(8.0, ""));
    let before = ev.evaluate(0.3);
    assert!(before.problems.is_empty(), "{:?}", before.problems);
    // Nothing has moved yet: every column is at rest over the original bed.
    assert!(sea(&before).frame.cells.iter().all(|c| c.depth == 12.0));
    assert!(sea(&before).frame.bed.iter().all(|&y| y == 12.0));
    let after = ev.evaluate(2.5);
    assert!(after.problems.is_empty(), "{:?}", after.problems);
    let frame = &sea(&after).frame;
    let centre = 32 * 64 + 32;
    assert!(frame.bed[centre] > 15.0, "the crater lowers the bed under the centre: {}", frame.bed[centre]);
    assert!(peak(&after) > 0.05, "no wave: {}", peak(&after));
    let volume: f64 = frame.cells.iter().map(|c| c.depth).sum();
    assert!((volume - 12.0 * 4096.0).abs() < 1e-8 * volume, "water volume {volume}");
    assert!(frame.cells.iter().all(|c| c.depth >= 0.0));
}

#[test]
fn a_larger_crater_makes_a_larger_wave() {
    let heights: Vec<f64> = [4.0, 8.0, 16.0]
        .iter()
        .map(|&depth| {
            let frame = evaluator(&scene(depth, "")).evaluate(2.5);
            assert!(frame.problems.is_empty(), "{:?}", frame.problems);
            peak(&frame)
        })
        .collect();
    println!("WAVE peak depth change for crater depths 4, 8, 16: {heights:?}");
    assert!(heights[0] < heights[1] && heights[1] < heights[2], "{heights:?}");
    assert!(heights[2] > 1.3 * heights[1], "{heights:?}");
}

#[test]
fn replay_after_a_backward_seek_reproduces_the_frame_and_its_key() {
    let ev = evaluator(&scene(8.0, ""));
    let first = ev.evaluate(2.5);
    ev.evaluate(0.7);
    ev.evaluate(1.2);
    let again = ev.evaluate(2.5);
    assert!(again.problems.is_empty(), "{:?}", again.problems);
    assert_eq!(sea(&first).frame, sea(&again).frame);
    assert_eq!(sea(&first).key, sea(&again).key);
}

#[test]
fn foam_follows_the_moving_bed() {
    let extra = r#"<whitewater emissionRate="1" threshold="0.5" maxParticles="100000" lifetime="2"/>"#;
    let frame = evaluator(&scene(10.0, extra)).evaluate(2.5);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    let foam = sea(&frame).whitewater.as_ref().unwrap();
    assert!(!foam.particles.is_empty(), "the collapsing crater makes no foam");
}

/// Cost of reading the crater into the bed at every step on a 720 x 720 ocean
/// under the authored 1000 x 1000 seabed of 96 segments. Run on request:
/// `cargo test --release -p sr-eval --test ocean_coupling -- --ignored --nocapture`.
#[test]
#[ignore = "timing measurement"]
fn bed_sampling_cost_per_canonical_step_at_720_by_720() {
    let hero = |colliders: &str| {
        format!(
            r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
              <object3D id="seabed" primitive="plane" width="1000" height="1000" segments="96" y="12" rotationX="-90">
                <crater radius="52" depth="25" rimHeight="7" rimWidth="10" start="1" end="2.5"/>
              </object3D>
              <ocean id="sea" width="1440" depth="1440" cellSize="2" bottomDepth="12" dt="0.0416666666666667" boundary="open" order="2" maxMemoryMiB="512" surfaceMemoryMiB="256" maxWork="100000000000" {colliders}/>
            </composition></scene>"#
        )
    };
    let run = |colliders: &str| {
        let ev = evaluator(&hero(colliders));
        ev.evaluate(1.0);
        let start = std::time::Instant::now();
        for k in 25..=48 {
            let frame = ev.evaluate(k as f64 / 24.0);
            assert!(frame.problems.is_empty(), "{:?}", frame.problems);
        }
        start.elapsed().as_secs_f64() / 24.0
    };
    let still = run("");
    let moving = run(r#"colliders="seabed""#);
    println!("BEDCOST per canonical step: static {still:.4} s, with the crater driving the bed {moving:.4} s; sampling adds {:.4} s", moving - still);
}
