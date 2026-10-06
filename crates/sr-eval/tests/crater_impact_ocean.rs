//! A crater that an impact makes under an ocean is the bed the ocean sees: it makes a wave
//! without an authored impulse, and a faster body makes a bigger crater and a bigger wave.
use sr_eval::{Evaluator, FrameGraph};

/// A 256 x 256 ocean of 2-unit cells over 30 units of water; a rock of 8 units radius
/// (6.4e6 kg) falls through the air and the water onto the seabed, at `speed` metres a second.
fn scene(speed: f64, extra: &str) -> String {
    format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
          <object3D id="rock" primitive="sphere" radius="8" y="-60">
            <rigidBody shape="sphere" mass="6400000" velocityY="{speed}" restitution="0" linearDamping="0" angularDamping="0"/>
          </object3D>
          <object3D id="seabed" primitive="plane" width="400" height="400" segments="160" y="30" rotationX="-90">
            <crater source="rock" targetMaterial="softRock"/>
            <rigidBody type="static" shape="auto"/>
          </object3D>
          <ocean id="sea" bedResponse="hydrostatic" width="256" depth="256" cellSize="2" bottomDepth="30" dt="0.0416666666666667" boundary="closed" colliders="seabed">{extra}</ocean>
        </composition>
        <physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/></scene>"#
    )
}

fn evaluator(xml: &str) -> Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    Evaluator::new(&doc, &Default::default()).unwrap()
}

fn sea(frame: &FrameGraph) -> &sr_eval::ocean::SimOcean {
    frame.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap()
}

/// Largest change of water depth from the 30-unit rest layer.
fn peak(frame: &FrameGraph) -> f64 {
    sea(frame).frame.cells.iter().map(|c| (c.depth - 30.0).abs()).fold(0.0, f64::max)
}

#[test]
fn an_impact_crater_under_the_ocean_makes_a_wave_with_no_impulse_in_the_document() {
    assert!(!scene(100.0, "").contains("waterImpulse"));
    let ev = evaluator(&scene(100.0, ""));
    let before = ev.evaluate(0.3);
    assert!(before.problems.is_empty() && before.failures.is_empty(), "{:?} {:?}", before.problems, before.failures);
    assert!(sea(&before).frame.bed.iter().all(|&y| y == 30.0), "nothing has hit the seabed yet");
    assert!(sea(&before).frame.cells.iter().all(|c| c.depth == 30.0));
    let after = ev.evaluate(3.5);
    assert!(after.problems.is_empty() && after.failures.is_empty(), "{:?} {:?}", after.problems, after.failures);
    let centre = 64 * 128 + 64;
    assert!(sea(&after).frame.bed[centre] > 33.0, "the crater lowers the bed: {}", sea(&after).frame.bed[centre]);
    assert!(peak(&after) > 0.05, "no wave: {}", peak(&after));
    let volume: f64 = sea(&after).frame.cells.iter().map(|c| c.depth).sum();
    assert!((volume - 30.0 * 16384.0).abs() < 1e-8 * volume, "water volume {volume}");
}

#[test]
fn a_faster_body_makes_a_bigger_crater_and_a_bigger_wave() {
    let runs: Vec<(f64, f64)> = [60.0, 100.0, 150.0]
        .iter()
        .map(|&speed| {
            let frame = evaluator(&scene(speed, "")).evaluate(3.5);
            assert!(
                frame.problems.is_empty() && frame.failures.is_empty(),
                "{:?} {:?}",
                frame.problems,
                frame.failures
            );
            let lowest = sea(&frame).frame.bed.iter().cloned().fold(f64::MIN, f64::max);
            (lowest - 30.0, peak(&frame))
        })
        .collect();
    println!("IMPACT crater depth and wave peak for 60, 100, 150 m/s: {runs:?}");
    assert!(runs.windows(2).all(|w| w[0].0 < w[1].0), "crater depth by speed: {runs:?}");
    assert!(runs.windows(2).all(|w| w[0].1 < w[1].1), "wave by speed: {runs:?}");
}

#[test]
fn replay_after_a_backward_seek_reproduces_the_wave() {
    let ev = evaluator(&scene(100.0, ""));
    let first = ev.evaluate(3.5);
    ev.evaluate(0.7);
    ev.evaluate(1.6);
    let again = ev.evaluate(3.5);
    assert!(again.problems.is_empty() && again.failures.is_empty(), "{:?} {:?}", again.problems, again.failures);
    assert_eq!(sea(&first).frame, sea(&again).frame);
    assert_eq!(sea(&first).key, sea(&again).key);
}
