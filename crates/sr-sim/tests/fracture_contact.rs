//! A body that breaks when something hits it hard enough: the fracture fires on the step after the impact of its
//! projectile is noticed, gives the pieces the momentum the contact left the body with and a part of the relative
//! kinetic energy of the impact as a push they give each other, and is the same on every replay.

use sr_sim::{fields::Field, physics3d::*};

struct Drive;
impl Driver3 for Drive {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
}

const STEP: f64 = 0.005;
const FRACTION: f64 = 0.3;

fn body(shape: Shape3, mass: f64, at: [f64; 3], velocity: [f64; 3]) -> Body3Spec {
    Body3Spec {
        kind: BodyKind::Dynamic,
        shape,
        mass,
        friction: 0.,
        restitution: 0.,
        linear_damping: 0.,
        angular_damping: 0.,
        velocity,
        angular_velocity: [0.; 3],
        group: 0,
        collides_with: None,
        sensor: false,
        fixed_rotation: false,
        bullet: false,
        activate_at: 0.,
        start: Pose3 { pos: at, ..Pose3::default() },
    }
}

/// A target of 6 kg at x = 30, three pieces of 1, 2 and 3 kg that are not symmetric about its centre, and a projectile
/// of 2 kg that arrives from the left at `speed`, along the target's axis so that nothing spins.
fn spec(speed: f64) -> World3Spec {
    World3Spec {
        fix_internal_edges: false,
        start: 0.,
        step: STEP,
        gravity: [0.; 3],
        pixels_per_meter: 1.,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies: vec![
            body(Shape3::Box([4., 4., 4.]), 6., [30., 100., 0.], [0.; 3]),
            body(Shape3::Box([1.; 3]), 1., [30., 100., 0.], [0.; 3]),
            body(Shape3::Box([1.; 3]), 2., [30., 100., 0.], [0.; 3]),
            body(Shape3::Box([1.; 3]), 3., [30., 100., 0.], [0.; 3]),
            body(Shape3::Sphere(1.), 2., [10., 100., 0.], [speed, 0., 0.]),
        ],
    }
}

fn event() -> Fracture3 {
    Fracture3 {
        source: 0,
        at: 0.,
        radial_impulse: 0.,
        fragments: vec![
            Fragment3 { body: 1, offset: [6., 0., 0.], impulse: [0.; 3] },
            Fragment3 { body: 2, offset: [-1., 3., 0.], impulse: [0.; 3] },
            Fragment3 { body: 3, offset: [-4. / 3., -2., 0.], impulse: [0.; 3] },
        ],
        contact: Some(FractureContact { watch: 0, energy_fraction: FRACTION }),
    }
}

fn watch(min_impulse: f64) -> Vec<ImpactWatch> {
    vec![ImpactWatch { source: 4, owner: 0, min_impulse }]
}

fn world(speed: f64, min_impulse: f64) -> World3 {
    World3::new(spec(speed)).with_fractures(vec![event()]).unwrap().with_impact_watches(watch(min_impulse)).unwrap()
}

/// The same bodies with no fracture: what the target does when it is not broken.
fn intact(speed: f64) -> World3 {
    // the pieces are in the world too, and in the way unless they collide with nothing
    let mut s = spec(speed);
    for piece in &mut s.bodies[1..4] {
        piece.collides_with = Some(vec![]);
    }
    World3::new(s).with_impact_watches(watch(1.)).unwrap()
}

#[test]
fn nothing_breaks_before_the_impact_and_the_fracture_fires_the_step_after_it_is_noticed() {
    let mut w = world(20., 1.);
    let early = w.frame_at(0.4, &mut Drive);
    assert!(early.errors.is_empty(), "{:?}", early.errors);
    assert_eq!((early.fractured.clone(), early.enabled[..4].to_vec()), (vec![false], vec![true, false, false, false]));
    let late = w.frame_at(1.5, &mut Drive);
    let impact = late.impacts[0].expect("the projectile hit the target");
    assert_eq!(late.fractured, [true]);
    assert_eq!(late.enabled[..4], [false, true, true, true]);
    // the impact is reported from the step after the one that resolved it, and the fracture fires there, no sooner
    let before = w.frame_at(impact.time - 0.5 * STEP, &mut Drive);
    assert_eq!(before.fractured, [false], "the impact ended its step at {}", impact.time);
    assert_eq!(w.frame_at(impact.time + STEP, &mut Drive).fractured, [true]);
}

/// Momentum and kinetic energy of the three pieces, and of the target of a world that does not break, at `t`.
fn totals(w: &mut World3, pieces: bool, t: f64) -> ([f64; 3], f64) {
    let f = w.frame_at(t, &mut Drive);
    assert!(f.errors.is_empty(), "{:?}", f.errors);
    let who: &[(usize, f64)] = if pieces { &[(1, 1.), (2, 2.), (3, 3.)] } else { &[(0, 6.)] };
    let (mut momentum, mut energy) = ([0.; 3], 0.);
    for &(k, m) in who {
        let v = f.velocities[k].linear;
        for c in 0..3 {
            momentum[c] += m * v[c];
        }
        energy += 0.5 * m * v.iter().map(|x| x * x).sum::<f64>();
    }
    (momentum, energy)
}

#[test]
fn the_pieces_keep_the_momentum_the_contact_left_the_target_and_a_part_of_the_relative_energy_pushes_them_apart() {
    let (mut broken, mut whole) = (world(20., 1.), intact(20.));
    let impact = broken.frame_at(1.5, &mut Drive).impacts[0].expect("an impact");
    // the fracture fires at the boundary that ends the step of the impact: the pieces and the whole target are the
    // same world until then, and are compared at that boundary
    let t = impact.time;
    let (pieces_momentum, pieces_energy) = totals(&mut broken, true, t);
    let (target_momentum, target_energy) = totals(&mut whole, false, t);
    // linear momentum: the target's, to rounding
    for c in 0..3 {
        assert!(
            (pieces_momentum[c] - target_momentum[c]).abs() < 1e-9,
            "along {c}: {pieces_momentum:?} against {target_momentum:?}"
        );
    }
    // energy: the fraction of the relative kinetic energy of the impact, 1/2 mu v_n^2 with mu the reduced mass
    let mu = 2. * 6. / (2. + 6.);
    let wanted = FRACTION * 0.5 * mu * impact.closing_speed * impact.closing_speed;
    let gained = pieces_energy - target_energy;
    println!("FRACTURE energy gained {gained:.6} against {wanted:.6}, closing speed {}", impact.closing_speed);
    assert!((gained - wanted).abs() < 1e-6 * wanted, "{gained} against {wanted}");
}

#[test]
fn a_weak_impact_and_no_impact_break_nothing() {
    // the threshold is above the impulse of the impact
    let mut hard_to_break = world(20., 1e6);
    let f = hard_to_break.frame_at(1.5, &mut Drive);
    assert!(f.errors.is_empty() && f.fractured == [false], "{:?} {:?}", f.errors, f.fractured);
    // a projectile that does not move never touches it
    let mut still = world(0., 1.);
    assert_eq!(still.frame_at(3., &mut Drive).fractured, [false]);
}

#[test]
fn the_same_pieces_in_any_order_of_requests_and_from_a_fresh_world() {
    let bits = |f: &Frame3| -> Vec<u64> {
        f.bodies
            .iter()
            .flat_map(|p| p.pos.iter().chain(&p.rot))
            .chain(f.velocities.iter().flat_map(|v| v.linear.iter().chain(&v.angular)))
            .map(|v| v.to_bits())
            .collect()
    };
    let mut a = world(20., 1.);
    let times = [1.5, 0.2, 2.5, 0.9, 1.5];
    let first: Vec<_> = times.iter().map(|&t| bits(&a.frame_at(t, &mut Drive))).collect();
    let mut b = world(20., 1.);
    for k in [4, 0, 3, 1, 2] {
        assert_eq!(first[k], bits(&b.frame_at(times[k], &mut Drive)), "t = {}", times[k]);
    }
    assert_eq!(first[0], first[4], "the same instant asked twice");
}

#[test]
fn a_contact_that_names_no_watch_of_the_body_is_refused() {
    let w = World3::new(spec(20.)).with_fractures(vec![event()]).unwrap();
    assert!(w.with_impact_watches(vec![]).is_err(), "no watch to read");
    let w = World3::new(spec(20.)).with_fractures(vec![event()]).unwrap();
    let wrong = vec![ImpactWatch { source: 4, owner: 1, min_impulse: 1. }];
    assert!(w.with_impact_watches(wrong).is_err(), "a watch against another body");
}
