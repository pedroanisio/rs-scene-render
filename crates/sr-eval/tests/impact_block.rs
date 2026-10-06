//! The block scene of the cinematic-impact examples, read as a document: a rock hits a block of rock lying on the
//! ground and the block breaks. Nothing in the document says when, so these tests check that it breaks at the
//! contact, keeps its mass, throws its pieces farther with the speed and the mass of the rock, and is the same
//! however it is asked for. The scene is used as authored; a test changes only the rock.

use sr_eval::{Evaluator, FrameGraph};

const BLOCK: &str = include_str!("../../../examples/cinematic-impact/impact-block.scene.xml");

const GRAVITY: f64 = 9.80665;
/// Seconds the rock flies from its launch to the top of the block, the same in every variant.
const FLIGHT: f64 = 1.4888;
const RADIUS: f64 = 2.0;
/// The top of the block, y in the scene (down), and the mass of the block.
const TOP: f64 = -6.0;
const BLOCK_MASS: f64 = 583200.0;

#[derive(Clone, Copy)]
struct Hit {
    speed: f64,
    angle: f64,
    mass: f64,
}

const AUTHORED: Hit = Hit { speed: 100.0, angle: 60.0, mass: 90478.0 };

/// The document with the rock launched so that it reaches the top of the block as `hit`.
fn with_hit(hit: Hit) -> String {
    let (vx, vy) = (hit.speed * hit.angle.to_radians().cos(), hit.speed * hit.angle.to_radians().sin());
    let launch_vy = vy - GRAVITY * FLIGHT;
    let x = -vx * FLIGHT;
    let y = TOP - RADIUS - launch_vy * FLIGHT - 0.5 * GRAVITY * FLIGHT * FLIGHT;
    let once = |text: &str, from: &str, to: &str| {
        assert_eq!(text.matches(from).count(), 1, "the scene has exactly one {from}");
        text.replacen(from, to, 1)
    };
    let xml = once(BLOCK, r#"x="-74.4" y="-126""#, &format!(r#"x="{x}" y="{y}""#));
    let xml = once(&xml, r#"velocityX="50" velocityY="72""#, &format!(r#"velocityX="{vx}" velocityY="{launch_vy}""#));
    once(&xml, r#"mass="90478""#, &format!(r#"mass="{}""#, hit.mass))
}

fn evaluator(xml: &str) -> Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

fn at(ev: &Evaluator, t: f64) -> FrameGraph {
    let frame = ev.evaluate(t);
    assert!(
        frame.problems.is_empty() && frame.failures.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.problems,
        frame.failures
    );
    frame
}

fn pieces(frame: &FrameGraph) -> Option<Vec<[f64; 3]>> {
    let block = frame.nodes.iter().find(|n| &*n.id == "block").unwrap();
    block.fracture.as_ref().map(|f| f.poses.iter().map(|p| p.pos).collect())
}

#[test]
fn the_scene_says_nothing_about_when_the_block_breaks() {
    let fracture = BLOCK.split("<fracture").nth(1).and_then(|s| s.split("/>").next()).expect("a fracture");
    for time in [" at=", " start=", " end=", " time=", " duration="] {
        assert!(!fracture.contains(time), "the fracture says when: {fracture}");
    }
    assert!(fracture.contains(r#"source="impactor""#));
}

#[test]
fn the_block_lies_still_until_the_rock_hits_it_and_then_breaks_in_the_pieces_of_its_partition() {
    let ev = evaluator(&with_hit(AUTHORED));
    // it rests on the ground, and the rock reaches its top about 1.49 s in
    for t in [0.0, 0.5, 1.0, 1.4] {
        assert!(pieces(&at(&ev, t)).is_none(), "t = {t}: broken before the contact");
    }
    let broken = at(&ev, 1.6);
    let parts = pieces(&broken).expect("broken by 1.6 s");
    assert_eq!(parts.len(), 12);
    let block = broken.nodes.iter().find(|n| &*n.id == "block").unwrap();
    let total: f64 = block.fracture.as_ref().unwrap().geometry.pieces.iter().map(|p| p.mass).sum();
    assert!((total - BLOCK_MASS).abs() < 1e-6 * BLOCK_MASS, "{total}");
}

/// How far the pieces have gone from where the block was, on average, a second and a half after the contact.
fn spread(hit: Hit) -> f64 {
    let ev = evaluator(&with_hit(hit));
    let parts = pieces(&at(&ev, 3.0)).expect("broken by 3 s");
    parts.iter().map(|p| p[0].hypot(p[2])).sum::<f64>() / parts.len() as f64
}

fn increasing(values: &[f64]) -> bool {
    values.windows(2).all(|w| w[0] < w[1])
}

#[test]
fn the_pieces_go_farther_with_the_speed_and_the_mass_of_the_rock() {
    let speeds: Vec<f64> = [60.0, 100.0, 150.0].iter().map(|&speed| spread(Hit { speed, ..AUTHORED })).collect();
    let masses: Vec<f64> = [30000.0, 90478.0, 270000.0].iter().map(|&mass| spread(Hit { mass, ..AUTHORED })).collect();
    println!("BLOCK mean distance of the pieces at 3 s by speed {speeds:?}, by mass {masses:?}");
    assert!(increasing(&speeds), "by speed: {speeds:?}");
    assert!(increasing(&masses), "by mass: {masses:?}");
}

#[test]
fn the_same_pieces_in_any_order_of_frames_and_from_a_fresh_evaluator() {
    let xml = with_hit(AUTHORED);
    let first = evaluator(&xml);
    let times = [3.0, 0.5, 1.6, 2.2, 1.4, 3.0];
    let forward: Vec<_> = times.iter().map(|&t| pieces(&at(&first, t))).collect();
    let fresh = evaluator(&xml);
    for k in [3, 0, 5, 1, 2, 4] {
        assert_eq!(forward[k], pieces(&at(&fresh, times[k])), "t = {}", times[k]);
    }
    assert_eq!(forward[0], forward[5]);
}
