//! A crater with `capture` arrests the body that makes it: after the impact the body loses its
//! speed at the rate that stops it within the depth of the crater, and then rests in the pit the
//! ground opens under it, with no energy from the ground and the same motion on any replay.

use sr_eval::Evaluator;

const GRAVITY: f64 = 9.80665;
const FLIGHT: f64 = 1.4888;
const MASS: f64 = 90478.0;
const STEP: f64 = 1.0 / 120.0;

/// The rock of the impact scenes arriving at the origin at 100 m/s and `angle` degrees above the ground.
fn scene(angle: f64, capture: &str) -> String {
    let (vx, vy) = (100.0 * angle.to_radians().cos(), 100.0 * angle.to_radians().sin());
    let launch = vy - GRAVITY * FLIGHT;
    let (x, y) = (-vx * FLIGHT, -2.0 - launch * FLIGHT - 0.5 * GRAVITY * FLIGHT * FLIGHT);
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="5"/><composition>
          <object3D id="rock" primitive="sphere" radius="2" x="{x}" y="{y}">
            <rigidBody shape="sphere" mass="{MASS}" velocityX="{vx}" velocityY="{launch}" restitution="0" linearDamping="0" angularDamping="0"/>
          </object3D>
          <object3D id="ground" primitive="plane" width="160" height="160" segments="160" y="0" rotationX="-90">
            <crater id="pit" source="rock" targetMaterial="softRock" {capture}/>
            <rigidBody type="static" shape="auto"/>
          </object3D>
        </composition>
        <physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none" fixInternalEdges="true"/></scene>"##
    )
}

fn evaluator(xml: &str) -> Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

fn pose(ev: &Evaluator, t: f64) -> [f64; 16] {
    let frame = ev.evaluate(t);
    assert!(
        frame.problems.is_empty() && frame.failures.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.problems,
        frame.failures
    );
    frame.nodes.iter().find(|n| &*n.id == "rock").unwrap().pose3.expect("simulated")
}

/// What the crater of the ground is: where it is centred (the ground's own axes, x along the ground), and its
/// rim radius and depth from the law.
fn crater(ev: &Evaluator) -> ([f64; 3], f64, f64) {
    let frame = ev.evaluate(4.0);
    let ground = frame.nodes.iter().find(|n| &*n.id == "ground").unwrap();
    let grown = ground.crater_impact.as_deref().expect("a crater");
    (grown.spec.center, grown.law().rim_radius, grown.law().depth)
}

/// Position, speed, kinetic energy (translation and the rotation of a solid sphere) and mechanical energy (with
/// the height) at `t`.
fn state(ev: &Evaluator, t: f64) -> ([f64; 3], f64, f64, f64) {
    let (a, b, c) = (pose(ev, t - STEP), pose(ev, t + STEP), pose(ev, t));
    let v: [f64; 3] = std::array::from_fn(|i| (b[12 + i] - a[12 + i]) / (2.0 * STEP));
    // rotation between the neighbours: the angle of b a^T
    let rot = |p: &[f64; 16]| [[p[0], p[4], p[8]], [p[1], p[5], p[9]], [p[2], p[6], p[10]]];
    let (ra, rb) = (rot(&a), rot(&b));
    let mut trace = 0.0;
    for i in 0..3 {
        for k in 0..3 {
            trace += rb[i][k] * ra[i][k];
        }
    }
    let spin = ((trace - 1.0) / 2.0).clamp(-1.0, 1.0).acos() / (2.0 * STEP);
    let speed = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    let kinetic = 0.5 * MASS * speed * speed + 0.5 * 0.4 * MASS * 4.0 * spin * spin;
    // scene y points down
    ([c[12], c[13], c[14]], speed, kinetic, kinetic + MASS * GRAVITY * -c[13])
}

#[test]
fn an_arrested_rock_rests_inside_the_crater_it_made() {
    for angle in [90.0, 60.0, 30.0] {
        let ev = evaluator(&scene(angle, r#"capture="true""#));
        let (centre, rim, depth) = crater(&ev);
        // the ground's plane is rotated: its z is up out of the ground, x and y lie in it; the rock's scene
        // coordinates are along the ground in x and z
        let at = |t: f64| state(&ev, t);
        let (rest, speed, ..) = at(4.0);
        let from = ((rest[0] - centre[0]).powi(2) + (rest[2] + centre[1]).powi(2)).sqrt();
        println!("CAPTURE {angle} degrees: rim {rim:.2} m, depth {depth:.2} m; at 4 s the rock is {from:.2} m from the impact point, at y {:.2}, speed {speed:.4}", rest[1]);
        // a ball in a bowl rocks a little on the floor it sank to: a metre a second at most, in the pit's inner half
        assert!(speed < 2.0, "{angle}: it rests: {speed} m/s");
        assert!(from < 0.5 * rim, "{angle}: in the pit near the impact point, {from} m from it (the rim is at {rim})");
        // it has followed the ground down into the pit: lower than the 2 m a rock rests at on flat ground
        assert!(
            rest[1] > -2.0 + 0.5 * depth,
            "{angle}: at y = {} the rock should sit in the pit, {depth} deep",
            rest[1]
        );
        // and it did not travel more than of the order of the depth after the impact
        let (soon, ..) = at(1.7);
        let travelled = ((soon[0] - centre[0]).powi(2) + (soon[2] + centre[1]).powi(2)).sqrt();
        assert!(
            travelled < 2.0 * depth + 2.0,
            "{angle}: {travelled} m from the impact point at 1.7 s, the depth is {depth}"
        );
    }
}

#[test]
fn it_takes_energy_out_and_the_ground_puts_none_in() {
    for angle in [90.0, 60.0, 30.0] {
        let ev = evaluator(&scene(angle, r#"capture="true""#));
        let free = evaluator(&scene(angle, ""));
        let before = state(&ev, 1.45).3;
        let times: Vec<f64> = (0..60).map(|k| 1.55 + 0.05 * k as f64).collect();
        let mechanical: Vec<f64> = times.iter().map(|&t| state(&ev, t).3).collect();
        let (kept, rolling) = (state(&ev, 4.0).2, state(&free, 4.0).2);
        println!("CAPTURE {angle} degrees: kinetic energy at 4 s {kept:.3e} against the free rock's {rolling:.3e} (impact {before:.3e})");
        // the mechanical energy never grows, to a ten-thousandth of the impact's: the speed here is a finite
        // difference of the poses and the ground's own motion moves the rock a little
        for pair in mechanical.windows(2) {
            assert!(pair[1] <= pair[0] + 1e-4 * before, "{angle}: grew from {} to {}", pair[0], pair[1]);
        }
        assert!(kept < 1e-3 * before, "{angle}: {kept} left, of {before}");
        // a rock that arrives straight down has nothing to roll: only a glancing one differs from the free rock
        assert!(angle == 90.0 || kept < 0.01 * rolling, "{angle}: {kept} against the free rock's {rolling}");
    }
}

#[test]
fn without_capture_the_rock_is_as_it_was_and_the_same_in_any_order() {
    let off = evaluator(&scene(60.0, ""));
    let explicit = evaluator(&scene(60.0, r#"capture="false""#));
    for t in [1.0, 1.6, 2.5, 4.0] {
        assert_eq!(pose(&off, t).map(f64::to_bits), pose(&explicit, t).map(f64::to_bits), "t = {t}");
    }
    // it rolls out of the crater, which is why the attribute exists
    assert!(state(&off, 4.0).0[0] > 50.0, "it rolls out of the crater");
    let on = evaluator(&scene(60.0, r#"capture="true""#));
    let times = [3.0, 1.55, 4.0, 1.4, 2.2, 3.0];
    let first: Vec<_> = times.iter().map(|&t| pose(&on, t).map(f64::to_bits)).collect();
    let fresh = evaluator(&scene(60.0, r#"capture="true""#));
    for k in [4, 0, 2, 5, 1, 3] {
        assert_eq!(first[k], pose(&fresh, times[k]).map(f64::to_bits), "t = {}", times[k]);
    }
}
