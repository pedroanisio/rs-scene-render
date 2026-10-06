//! A body that has made an impact can be arrested by the driver: from the impact until its
//! velocity relative to the surface is zero, a deceleration of constant size along minus that
//! velocity, never more than stops it in the step, and only where the driver says. It takes
//! energy out of the body and cannot put any in.

use sr_sim::{fields::Field, physics3d::*};

const STEP: f64 = 0.005;
/// Scene units a second squared.
const DECELERATION: f64 = 2000.0;
/// Scene units from the impact point within which the body is arrested.
const RADIUS: f64 = 60.0;

struct Arrest {
    /// What `capture` was asked: step, the body's centre in the owner's frame and the impact's point.
    asked: Vec<(u64, [f64; 3], [f64; 3])>,
}

impl Driver3 for Arrest {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
    fn capture(
        &mut self,
        step: u64,
        _t: f64,
        _source: usize,
        _owner: usize,
        centre: [f64; 3],
        impact: &Impact3,
    ) -> Result<Option<f64>, String> {
        self.asked.push((step, centre, impact.point));
        let from: f64 = (0..3).map(|i| (centre[i] - impact.point[i]).powi(2)).sum::<f64>().sqrt();
        Ok((from < RADIUS).then_some(DECELERATION))
    }
}

struct Plain;
impl Driver3 for Plain {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
}

fn body(shape: Shape3, kind: BodyKind, at: [f64; 3], velocity: [f64; 3]) -> Body3Spec {
    Body3Spec {
        kind,
        shape,
        mass: 50.0,
        friction: 0.5,
        restitution: 0.0,
        linear_damping: 0.0,
        angular_damping: 0.0,
        velocity,
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

/// A sphere of radius 10 that lands at the origin of a flat ground at y = 100 going `along` px/s along x and
/// 400 down.
fn landing(along: f64) -> World3 {
    World3::new(World3Spec {
        start: 0.0,
        step: STEP,
        gravity: [0.0, -9.81, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies: vec![
            body(Shape3::Sphere(10.0), BodyKind::Dynamic, [-along * 0.5, -100.0, 0.0], [along, 400.0, 0.0]),
            body(Shape3::Box([2000.0, 10.0, 2000.0]), BodyKind::Static, [0.0, 110.0, 0.0], [0.0; 3]),
        ],
        fix_internal_edges: false,
    })
    .with_impact_watches(vec![ImpactWatch { source: 0, owner: 1, min_impulse: 1.0 }])
    .unwrap()
}

/// The motion of the sphere from `from` to `to` seconds in steps of `STEP`: position, speed along the ground
/// and the kinetic energy (translation and rotation, a solid sphere), and the impact's point.
fn track(w: &mut World3, driver: &mut dyn Driver3, to: f64) -> Vec<(f64, [f64; 3], f64)> {
    let mut rows = Vec::new();
    let mut t = STEP;
    while t <= to + 1e-9 {
        let frame = w.frame_at(t, driver);
        assert!(frame.errors.is_empty(), "{:?}", frame.errors);
        let b = frame.bodies[0];
        let v = frame.velocities[0];
        let linear: f64 = v.linear.iter().map(|c| c * c).sum::<f64>();
        let spin: f64 = v.angular.iter().map(|c| c.to_radians().powi(2)).sum::<f64>();
        // m = 50, r = 10 px = 10 m at one pixel a metre
        let energy = 0.5 * 50.0 * linear + 0.5 * 0.4 * 50.0 * 100.0 * spin;
        rows.push((t, b.pos, energy));
        t += STEP;
    }
    rows
}

#[test]
fn an_arrested_body_stops_where_it_hit_and_its_energy_never_grows() {
    for along in [0.0, 300.0] {
        let mut w = landing(along);
        let rows = track(&mut w, &mut Arrest { asked: vec![] }, 4.0);
        let impact = w.frame_at(4.0, &mut Plain).impacts[0].expect("the impact was noticed");
        let first = rows.iter().position(|r| r.0 > impact.time + STEP).expect("steps after the impact");
        // from the step after the impact the energy does not grow (the numbers are exact to the solver's tolerance)
        for pair in rows[first..].windows(2) {
            assert!(
                pair[1].2 <= pair[0].2 * (1.0 + 1e-9) + 1e-9,
                "energy grew at t = {}: {} -> {}",
                pair[1].0,
                pair[0].2,
                pair[1].2
            );
        }
        let last = rows.last().unwrap();
        // it rests: no motion left, and it is within a few radii of where it hit, however fast it came
        assert!(last.2 < 1e-6 * rows[first].2.max(1.0), "{along}: energy at 4 s {} of {}", last.2, rows[first].2);
        let travelled = ((last.1[0] - impact.point[0]).powi(2) + (last.1[2] - impact.point[2]).powi(2)).sqrt();
        assert!(travelled < 20.0, "{along}: it came to rest {travelled} px from the impact point");
    }
}

#[test]
fn a_driver_that_does_not_arrest_changes_nothing_and_the_body_rolls_on() {
    let (mut plain, mut arrested) = (landing(300.0), landing(300.0));
    let on = track(&mut plain, &mut Plain, 2.0);
    let off = track(&mut arrested, &mut Arrest { asked: vec![] }, 2.0);
    // with `capture` unanswered the world is the one there always was: the sphere rolls off
    assert!(on.last().unwrap().1[0] > 100.0, "{:?}", on.last());
    assert!(off.last().unwrap().1[0] < 60.0, "{:?}", off.last());
}

#[test]
fn only_after_the_impact_and_the_same_in_any_order_of_requests() {
    let mut w = landing(300.0);
    let mut driver = Arrest { asked: vec![] };
    let late = w.frame_at(3.0, &mut driver);
    let first_asked = driver.asked.first().expect("asked after the impact").0;
    let impact = late.impacts[0].unwrap();
    assert!(first_asked as f64 * STEP >= impact.time, "asked at step {first_asked}, the impact is at {}", impact.time);
    let again = {
        let _ = w.frame_at(0.5, &mut driver);
        w.frame_at(3.0, &mut driver)
    };
    assert_eq!(late.bodies, again.bodies);
    let mut fresh = landing(300.0);
    assert_eq!(late.bodies, fresh.frame_at(3.0, &mut Arrest { asked: vec![] }).bodies);
}
