//! A rigid body that floats: the weight of the water it displaces holds it up, a drag on its
//! vertical motion settles it, and a body too light for the step is reported.

use sr_sim::hydrostatics::{buoyant_load, halfway, place, submerged_mesh, submerged_sphere, Submerged, Surface, Water};
use sr_sim::{fields::Field, physics3d::*};

const RHO: f64 = 1000.0;
const G: f64 = 9.80665;

#[derive(Clone)]
enum Hull {
    Sphere(f64),
    Box([f64; 3]),
}

/// Applies the water's load, with the water at rest at y = 0 (scene axes: the water is below, y > 0).
struct Float {
    hull: Hull,
    mass: f64,
    water: Water,
    step: f64,
    surface: Surface,
}

impl Float {
    fn submerged(&self, state: &BodyState) -> Submerged {
        // the force is held for the whole step, so it is the force where the body will be halfway through it
        let ahead = |p: [f64; 3]| halfway(p, state.velocity.linear, self.step);
        match &self.hull {
            Hull::Sphere(r) => submerged_sphere(ahead(state.pose.pos), *r, &self.surface),
            Hull::Box(h) => {
                let corners: Vec<[f64; 3]> =
                    (0..8).map(|k| [0, 1, 2].map(|a| if (k >> a) & 1 == 1 { h[a] } else { -h[a] })).collect();
                let faces: [[u32; 3]; 12] = [
                    [0, 2, 1],
                    [1, 2, 3],
                    [4, 5, 6],
                    [5, 7, 6],
                    [0, 1, 4],
                    [1, 5, 4],
                    [2, 6, 3],
                    [3, 6, 7],
                    [0, 4, 2],
                    [2, 4, 6],
                    [1, 3, 5],
                    [3, 7, 5],
                ];
                let world = place(&corners, ahead(state.pose.pos), state.pose.rot);
                submerged_mesh(&world, &faces, &self.surface).unwrap()
            }
        }
    }
}

impl Driver3 for Float {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
    fn load(&mut self, _: u64, _: f64, _: usize, state: &BodyState) -> Result<Option<Load3>, String> {
        let sub = self.submerged(state);
        let buoyancy = buoyant_load(&sub, state.centre, self.mass, state.velocity.linear[1], &self.water, self.step)?;
        Ok(Some(buoyancy.load))
    }
}

fn water(drag: f64) -> Water {
    Water { density: RHO, gravity: G, drag, pixels_per_meter: 1.0 }
}

fn body(hull: &Hull, mass: f64, at: [f64; 3]) -> Body3Spec {
    Body3Spec {
        kind: BodyKind::Dynamic,
        shape: match hull {
            Hull::Sphere(r) => Shape3::Sphere(*r),
            Hull::Box(h) => Shape3::Box(*h),
        },
        mass,
        friction: 0.5,
        restitution: 0.0,
        linear_damping: 0.0,
        angular_damping: 0.0,
        velocity: [0.0; 3],
        angular_velocity: [0.0; 3],
        group: 0,
        collides_with: None,
        sensor: false,
        fixed_rotation: false,
        bullet: false,
        activate_at: 0.0,
        start: Pose3 { pos: at, ..Pose3::default() },
    }
}

fn world(hull: &Hull, mass: f64, at: [f64; 3], step: f64) -> World3 {
    World3::new(World3Spec {
        start: 0.0,
        step,
        // scene axes: gravity is toward +y, and the world takes it as physics y-up
        gravity: [0.0, -G, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies: vec![body(hull, mass, at)],
    })
}

fn floater(hull: Hull, mass: f64, drag: f64, step: f64) -> Float {
    Float { hull, mass, water: water(drag), step, surface: Surface { offset: 0.0, slope: [0.0, 0.0] } }
}

/// The height of the centre of the ball over the surface at which it displaces `mass` of water.
fn ball_draft(r: f64, mass: f64) -> f64 {
    // cap volume as the centre goes from y = -r (just touching) to y = r (just under)
    let want = mass / RHO;
    let (mut lo, mut hi) = (-r, r);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        let v = submerged_sphere([0.0, mid, 0.0], r, &Surface { offset: 0.0, slope: [0.0, 0.0] }).volume;
        // deeper submerges more: too much displaced means the centre is too deep
        if v > want {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    0.5 * (lo + hi)
}

#[test]
fn a_ball_that_floats_settles_at_the_draft_that_displaces_its_own_weight() {
    let r: f64 = 1.0;
    for density in [300.0, 500.0, 800.0] {
        let mass = density * 4.0 / 3.0 * std::f64::consts::PI * r.powi(3);
        let hull = Hull::Sphere(r);
        let mut w = world(&hull, mass, [0.0, -0.6, 0.0], 1.0 / 240.0);
        let mut driver = floater(hull, mass, 1.0, 1.0 / 240.0);
        // quadratic drag takes the last of the motion out slowly, so give it time
        let frame = w.frame_at(100.0, &mut driver);
        assert!(frame.errors.is_empty(), "{:?}", frame.errors);
        let want = ball_draft(r, mass);
        let got = frame.bodies[0].pos[1];
        assert!((got - want).abs() < 0.02, "density {density}: centre at {got}, the displaced weight needs {want}");
        assert!(frame.velocities[0].linear[1].abs() < 0.1, "at rest: {:?}", frame.velocities[0]);
    }
}

#[test]
fn a_ball_denser_than_water_sinks() {
    let r: f64 = 1.0;
    let mass = 2000.0 * 4.0 / 3.0 * std::f64::consts::PI * r.powi(3);
    let hull = Hull::Sphere(r);
    let mut w = world(&hull, mass, [0.0, -2.0, 0.0], 1.0 / 240.0);
    let mut driver = floater(hull, mass, 1.0, 1.0 / 240.0);
    let (a, b) = (w.frame_at(4.0, &mut driver).bodies[0].pos[1], w.frame_at(8.0, &mut driver).bodies[0].pos[1]);
    assert!(a > 1.0 && b > a + 3.0, "it goes down and keeps going: {a} then {b}");
}

/// How far the ball's centre is from its draft over time.
fn excursion(drag: f64, step: f64, until: f64) -> Vec<f64> {
    let r: f64 = 1.0;
    let mass = 500.0 * 4.0 / 3.0 * std::f64::consts::PI * r.powi(3);
    let hull = Hull::Sphere(r);
    let draft = ball_draft(r, mass);
    let mut w = world(&hull, mass, [0.0, -0.8, 0.0], step);
    let mut driver = floater(hull, mass, drag, step);
    (1..=(until / 0.05) as usize)
        .map(|k| (w.frame_at(k as f64 * 0.05, &mut driver).bodies[0].pos[1] - draft).abs())
        .collect()
}

#[test]
fn with_form_drag_the_oscillation_dies_away_and_without_it_never_grows() {
    let damped = excursion(1.0, 1.0 / 240.0, 60.0);
    // seconds 0 to 1 and then windows of 5 s: the envelope only falls, from 0.8 to a few centimetres
    let window = |from: usize, to: usize| damped[from..to].iter().cloned().fold(0.0, f64::max);
    assert!(window(0, 20) > 0.5, "it bobs at first");
    let envelope: Vec<f64> = (0..11).map(|k| window(k * 100 + 20, (k + 1) * 100 + 20)).collect();
    assert!(envelope.windows(2).all(|w| w[1] < w[0]), "{envelope:?}");
    assert!(envelope[10] < 0.03, "and has all but settled: {envelope:?}");
}

/// Mechanical energy of a ball of mass `m` and radius 1 at height `y` (scene axes, y down) moving at `v`:
/// kinetic, plus gravity, plus the potential of the buoyant force, whose integral is taken numerically.
fn energy(m: f64, y: f64, v: f64) -> f64 {
    let flat = Surface { offset: 0.0, slope: [0.0, 0.0] };
    let steps = 4000;
    let h = (y + 1.0) / steps as f64;
    let work: f64 = (0..steps)
        .map(|k| {
            let at = -1.0 + (k as f64 + 0.5) * h;
            submerged_sphere([0.0, at, 0.0], 1.0, &flat).volume * h
        })
        .sum();
    0.5 * m * v * v - m * G * y + RHO * G * work
}

#[test]
fn without_drag_the_water_takes_no_energy_from_the_ball_and_the_step_gives_it_none() {
    let r: f64 = 1.0;
    let mass = 500.0 * 4.0 / 3.0 * std::f64::consts::PI * r.powi(3);
    let step = 1.0 / 480.0;
    let hull = Hull::Sphere(r);
    let mut w = world(&hull, mass, [0.0, -0.8, 0.0], step);
    let mut driver = floater(hull, mass, 0.0, step);
    let e0 = energy(mass, -0.8, 0.0);
    // the lowest the energy can be: at the draft, at rest
    let rest = energy(mass, ball_draft(r, mass), 0.0);
    let range = e0 - rest;
    assert!(range > 0.0);
    let mut worst = f64::MIN;
    let mut still_moving = 0.0f64;
    for k in 1..=600 {
        let f = w.frame_at(k as f64 * 0.05, &mut driver);
        let e = energy(mass, f.bodies[0].pos[1], f.velocities[0].linear[1]);
        worst = worst.max(e - e0);
        if k > 500 {
            still_moving = still_moving.max((f.bodies[0].pos[1] - ball_draft(r, mass)).abs());
        }
    }
    assert!(worst < 0.01 * range, "the energy never rises by more than 1 % of its range: {worst} of {range}");
    assert!(still_moving > 0.3, "and with no drag it is still bobbing after 30 s: {still_moving}");
}

#[test]
fn halving_the_step_brings_the_motion_closer_to_its_limit() {
    // the position at 3 s with the step halved each time: the differences between runs shrink
    let at = |step: f64| *excursion(1.0, step, 3.0).last().unwrap();
    let (a, b, c, reference) = (at(1.0 / 60.0), at(1.0 / 120.0), at(1.0 / 240.0), at(1.0 / 1920.0));
    let errors = [a, b, c].map(|v| (v - reference).abs());
    println!("FLOAT error at 3 s for steps 1/60, 1/120, 1/240: {errors:?}");
    assert!(errors[0] > errors[1] && errors[1] > errors[2], "{errors:?}");
}

#[test]
fn a_body_too_light_for_the_step_is_an_error_not_an_unstable_motion() {
    let r: f64 = 1.0;
    let hull = Hull::Sphere(r);
    // 0.1 kg of ball: the waterline spring is far too stiff for it at 1/120 s
    let mut w = world(&hull, 0.1, [0.0, -0.5, 0.0], 1.0 / 120.0);
    let mut driver = floater(hull, 0.1, 1.0, 1.0 / 120.0);
    let frame = w.frame_at(2.0, &mut driver);
    assert!(frame.bodies.is_empty());
    assert!(frame.errors[0].contains("unstable") && frame.errors[0].contains("omega * dt"), "{:?}", frame.errors);
    // a smaller step makes the same body fine
    let hull = Hull::Sphere(0.1);
    let mass = 500.0 * 4.0 / 3.0 * std::f64::consts::PI * 0.1f64.powi(3);
    let mut w = world(&hull, mass, [0.0, -0.05, 0.0], 1.0 / 120.0);
    let mut driver = floater(hull.clone(), mass, 1.0, 1.0 / 120.0);
    assert!(w.frame_at(1.0, &mut driver).errors.is_empty());
}

#[test]
fn a_box_floats_at_the_draft_its_weight_needs_and_rights_itself() {
    // a 2 x 1 x 2 m box of 600 kg: draft = 600 / (1000 x 4) = 0.15 m of its 1 m height
    let hull = Hull::Box([1.0, 0.5, 1.0]);
    let mass = 600.0;
    let step = 1.0 / 240.0;
    let mut w = world(&hull, mass, [0.0, -0.3, 0.0], step);
    let mut driver = floater(hull.clone(), mass, 1.0, step);
    let frame = w.frame_at(25.0, &mut driver);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    let draft = mass / (RHO * 4.0);
    // the bottom face is at centre + 0.5, and sits `draft` below the surface
    assert!((frame.bodies[0].pos[1] + 0.5 - draft).abs() < 0.01, "{:?}", frame.bodies[0]);
    // Tilted when released, it is turned back toward upright and swings through it. Only the
    // vertical motion is damped, so the roll does not die away.
    let q = |angle: f64| [0.0, 0.0, (angle / 2.0).sin(), (angle / 2.0).cos()];
    let mut spec = body(&hull, mass, [0.0, -0.1, 0.0]);
    spec.start.rot = q(0.4);
    let mut w = World3::new(World3Spec {
        start: 0.0,
        step,
        gravity: [0.0, -G, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies: vec![spec],
    });
    let tilt = |f: &Frame3| 2.0 * f.bodies[0].rot[2].abs().atan2(f.bodies[0].rot[3].abs());
    let angles: Vec<f64> = (0..=200).map(|k| tilt(&w.frame_at(k as f64 * 0.05, &mut driver))).collect();
    assert!((angles[0] - 0.4).abs() < 1e-9);
    assert!(angles.iter().cloned().fold(f64::MAX, f64::min) < 0.05, "it swings through upright");
    assert!(angles.iter().cloned().fold(0.0, f64::max) < 0.8, "and does not capsize");
}

#[test]
fn the_load_is_the_weight_of_the_displaced_water_through_the_centroid() {
    let sphere = |y: f64| submerged_sphere([0.0, y, 0.0], 1.0, &Surface { offset: 0.0, slope: [0.0, 0.0] });
    // dry: nothing
    let dry = buoyant_load(&sphere(-3.0), [0.0, -3.0, 0.0], 100.0, 0.0, &water(1.0), 0.01).unwrap();
    assert_eq!((dry.load, dry.omega), (Load3::default(), 0.0));
    // all under: rho g V up (scene -y), no torque from a centroid at the centre of mass, no waterline spring
    let down = sphere(5.0);
    let load = buoyant_load(&down, [0.0, 5.0, 0.0], 100.0, 0.0, &water(1.0), 0.01).unwrap();
    let want = RHO * G * 4.0 / 3.0 * std::f64::consts::PI;
    assert!((load.load.force[1] + want).abs() < 1e-9 * want && load.load.force[0] == 0.0, "{:?}", load.load);
    assert!(load.load.torque.iter().all(|t| t.abs() < 1e-9 * want) && load.omega == 0.0);
    // a centre of mass off to the side makes the buoyant force turn the body: torque = arm x force
    let turned = buoyant_load(&down, [0.5, 5.0, 0.0], 100.0, 0.0, &water(1.0), 0.01).unwrap();
    // arm (-0.5, 0, 0) x force (0, -rho g V, 0) = (0, 0, +0.5 rho g V)
    assert!((turned.load.torque[2] - 0.5 * want).abs() < 1e-6 * want, "{:?}", turned.load.torque);
    // the units follow the scene scale: a hundred scene units to the metre gives the same force in kg x units / s^2 / 100
    let scaled = Water { pixels_per_meter: 100.0, ..water(1.0) };
    let big = submerged_sphere([0.0, 500.0, 0.0], 100.0, &Surface { offset: 0.0, slope: [0.0, 0.0] });
    let l = buoyant_load(&big, [0.0, 500.0, 0.0], 100.0, 0.0, &scaled, 0.01).unwrap();
    assert!((l.load.force[1] + want * 100.0).abs() < 1e-9 * want * 100.0, "{:?}", l.load.force);
}

#[test]
fn the_drag_opposes_the_motion_and_cannot_reverse_it_within_a_step() {
    let half = submerged_sphere([0.0, 0.0, 0.0], 1.0, &Surface { offset: 0.0, slope: [0.0, 0.0] });
    let lift = buoyant_load(&half, [0.0; 3], 100.0, 0.0, &water(0.0), 0.01).unwrap().load.force[1];
    for v in [-3.0, 3.0] {
        let moving = buoyant_load(&half, [0.0; 3], 5000.0, v, &water(1.0), 0.01).unwrap().load.force[1];
        // moving down (v > 0) the drag pushes up, moving up it pushes down
        assert!((moving - lift) * v < 0.0, "{v}: {moving} against {lift}");
    }
    // so heavy a drag at this speed that it would stop the body ten times over in one step: it only stops it
    let fast = buoyant_load(&half, [0.0; 3], 10.0, 50.0, &water(1.0), 0.01).unwrap().load.force[1];
    let stops = 10.0 * 50.0 / 0.01;
    assert!(((fast - lift).abs() - stops).abs() < 1e-6 * stops, "{fast} {lift} {stops}");
    // no drag with the coefficient at zero
    let none = buoyant_load(&half, [0.0; 3], 5000.0, 3.0, &water(0.0), 0.01).unwrap().load.force[1];
    assert_eq!(none, lift);
}

#[test]
fn nonsense_is_an_error() {
    let half = submerged_sphere([0.0; 3], 1.0, &Surface { offset: 0.0, slope: [0.0, 0.0] });
    for (mass, water, step) in [
        (0.0, water(1.0), 0.01),
        (100.0, Water { density: 0.0, ..water(1.0) }, 0.01),
        (100.0, Water { drag: -1.0, ..water(1.0) }, 0.01),
        (100.0, Water { pixels_per_meter: 0.0, ..water(1.0) }, 0.01),
        (100.0, water(1.0), 0.0),
    ] {
        assert!(buoyant_load(&half, [0.0; 3], mass, 0.0, &water, step).is_err());
    }
}
