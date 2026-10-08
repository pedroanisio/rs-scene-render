//! A watch on a pair that does not touch must not depend on how many other bodies are in contact: only the
//! contacts of the watched pairs are looked at for the impact.

use sr_sim::{fields::Field, physics3d::*};

const STEP: f64 = 0.005;

struct Quiet;

impl Driver3 for Quiet {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
}

fn body(shape: Shape3, kind: BodyKind, at: [f64; 3]) -> Body3Spec {
    Body3Spec {
        kind,
        shape,
        mass: 1.0,
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

/// A watched sphere and its owner far from everything and from each other, and a field of boxes at rest on a ground:
/// each box touches the ground at four points, so `side * side` boxes make several times that many contact points.
fn world(side: usize) -> World3 {
    let mut bodies = vec![
        body(Shape3::Sphere(10.0), BodyKind::Dynamic, [100_000.0, 0.0, 0.0]),
        body(Shape3::Box([200.0, 10.0, 200.0]), BodyKind::Static, [200_000.0, 100.0, 0.0]),
        body(Shape3::Box([400.0, 10.0, 400.0]), BodyKind::Static, [0.0, 100.0, 0.0]),
    ];
    for i in 0..side {
        for j in 0..side {
            let (x, z) = ((i as f64 - side as f64 / 2.0) * 12.0, (j as f64 - side as f64 / 2.0) * 12.0);
            bodies.push(body(Shape3::Box([5.0, 5.0, 5.0]), BodyKind::Dynamic, [x, 85.0, z]));
        }
    }
    World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.0,
        step: STEP,
        gravity: [0.0, -9.81, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies,
    })
    .with_impact_watches(vec![ImpactWatch { source: 0, owner: 1, min_impulse: 0.0 }])
    .unwrap()
}

#[test]
fn a_watch_that_does_not_happen_is_not_stopped_by_contacts_of_other_pairs() {
    // 1156 boxes of four contact points each: more than the 4096 points a step may hold when every pair is counted
    let mut w = world(34);
    let frame = w.frame_at(0.05, &mut Quiet);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert_eq!(frame.impacts, [None], "the pair never touches");
    // and a later request is as good as the first
    assert!(w.frame_at(0.02, &mut Quiet).errors.is_empty());
}

/// A box on its owner: the watched pair is in contact at four points.
fn resting_on_the_owner(limit: Option<usize>) -> World3 {
    let bodies = vec![
        body(Shape3::Box([5.0, 5.0, 5.0]), BodyKind::Dynamic, [0.0, 85.0, 0.0]),
        body(Shape3::Box([400.0, 10.0, 400.0]), BodyKind::Static, [0.0, 100.0, 0.0]),
    ];
    let w = World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.0,
        step: STEP,
        gravity: [0.0, -9.81, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies,
    })
    .with_impact_watches(vec![ImpactWatch { source: 0, owner: 1, min_impulse: 0.0 }])
    .unwrap();
    match limit {
        Some(n) => w.with_watch_contacts_per_step(n).unwrap(),
        None => w,
    }
}

#[test]
fn the_limit_applies_to_the_contacts_of_the_watched_pair_and_can_be_set() {
    let frame = resting_on_the_owner(None).frame_at(0.05, &mut Quiet);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    let frame = resting_on_the_owner(Some(3)).frame_at(0.05, &mut Quiet);
    assert!(frame.errors.iter().any(|e| e.contains("exceed the contact log limit of 3")), "{:?}", frame.errors);
}

#[test]
fn the_limit_cannot_be_changed_once_the_world_has_stepped() {
    let mut w = resting_on_the_owner(None);
    let _ = w.frame_at(0.02, &mut Quiet);
    assert!(w.with_watch_contacts_per_step(10).is_err());
}
