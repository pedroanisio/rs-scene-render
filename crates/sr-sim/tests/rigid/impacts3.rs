//! The world notices the first impact of one body on another, keeps it with its state,
//! and tells the owner's surface about it so a crater can grow from it.

use sr_sim::{fields::Field, physics3d::*};

const STEP: f64 = 0.005;

#[derive(Default)]
struct Watching {
    /// What `surface` was told about the owner at each call: step time and the impact.
    told: Vec<(f64, Option<Impact3>)>,
}

impl Driver3 for Watching {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
    fn surface(
        &mut self,
        t: f64,
        which: usize,
        _revision: Option<u64>,
        impact: Option<&Impact3>,
    ) -> Result<Option<ColliderUpdate3>, String> {
        if which == 1 {
            self.told.push((t, impact.copied()));
        }
        Ok(None)
    }
}

fn sphere(at: [f64; 3], velocity: [f64; 3], mass: f64) -> Body3Spec {
    Body3Spec {
        kind: BodyKind::Dynamic,
        shape: Shape3::Sphere(10.0),
        mass,
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

/// A static slab, half extents `half`, with the given pose.
fn slab(pose: Pose3, half: [f64; 3]) -> Body3Spec {
    Body3Spec { kind: BodyKind::Static, shape: Shape3::Box(half), start: pose, ..sphere([0.0; 3], [0.0; 3], 1.0) }
}

fn upright(at: [f64; 3]) -> Pose3 {
    Pose3 { pos: at, ..Pose3::default() }
}

fn world(bodies: Vec<Body3Spec>, gravity: f64) -> World3 {
    World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.0,
        step: STEP,
        gravity: [0.0, -gravity, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies,
    })
}

fn watch(min_impulse: f64) -> Vec<ImpactWatch> {
    vec![ImpactWatch { source: 0, owner: 1, min_impulse }]
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance * b.abs().max(1.0)
}

/// Rotates `v` by the quaternion `q = [x, y, z, w]`.
fn rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let (u, w) = ([q[0], q[1], q[2]], q[3]);
    let cross =
        |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let t = cross(u, v).map(|c| 2.0 * c);
    let ut = cross(u, t);
    std::array::from_fn(|i| v[i] + w * t[i] + ut[i])
}

/// A 2 kg sphere falling at 200 px/s from y = 0 onto a slab whose top face is at y = 90.
fn drop_on_slab() -> World3 {
    world(vec![sphere([0.0; 3], [0.0, 200.0, 0.0], 2.0), slab(upright([30.0, 100.0, 0.0]), [200.0, 10.0, 200.0])], 9.81)
        .with_impact_watches(watch(1.0))
        .unwrap()
}

#[test]
fn the_first_impact_is_noticed_with_its_place_direction_and_speed() {
    let mut w = drop_on_slab();
    assert!(w.frame_at(0.1, &mut Watching::default()).impacts == [None], "nothing has been hit yet");
    let frame = w.frame_at(0.6, &mut Watching::default());
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    let impact = frame.impacts[0].expect("the sphere reached the slab");
    // the step that resolved it ends at `time`, and the owner is told from the next step on
    assert!(close(impact.time, (impact.step + 1) as f64 * STEP, 1e-12));
    assert!(impact.step > 0 && impact.time < 0.6);
    // the slab's frame is its own, offset by its position: the sphere is over x = 0, the slab at x = 30
    assert!(close(impact.point[0], -30.0, 1e-3), "{:?}", impact.point);
    assert!((impact.point[1] + 10.0).abs() < 1.5, "the top face, 10 above the slab's centre: {:?}", impact.point);
    // from the owner toward the source: up the screen, scene -y
    assert!(close(impact.normal[1], -1.0, 1e-9) && impact.normal[0].abs() < 1e-9, "{:?}", impact.normal);
    let speed = 200.0 + 9.81 * impact.step as f64 * STEP;
    assert!(close(impact.closing_speed, speed, 0.01), "{} vs {speed}", impact.closing_speed);
    // the source's velocity relative to the owner, in the world's axes
    assert!(close(impact.relative_velocity[1], impact.closing_speed, 1e-9), "{:?}", impact.relative_velocity);
    assert!(impact.impulse > 1.0 && impact.impulse < 2.0 * impact.closing_speed * 1.05, "{}", impact.impulse);
    // a later request still shows the first one, not a bounce
    assert_eq!(w.frame_at(2.0, &mut Watching::default()).impacts[0], Some(impact));
}

#[test]
fn the_point_and_the_direction_are_in_the_owners_own_frame() {
    // a slab turned a quarter turn about z: its thin axis is scene y again, but its x is up
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let turned = Pose3 { pos: [30.0, 100.0, 0.0], rot: [0.0, 0.0, s, s] };
    let mut w = world(vec![sphere([0.0; 3], [0.0, 200.0, 0.0], 2.0), slab(turned, [10.0, 200.0, 200.0])], 9.81)
        .with_impact_watches(watch(1.0))
        .unwrap();
    let impact = w.frame_at(0.6, &mut Watching::default()).impacts[0].expect("an impact");
    // the owner's frame turned back into the world's is where the sphere hit
    let world_normal = rotate(turned.rot, impact.normal);
    assert!(close(world_normal[1], -1.0, 1e-9) && world_normal[0].abs() < 1e-9, "{world_normal:?}");
    assert!(
        impact.normal[1].abs() < 1e-9 && impact.normal[0].abs() > 0.99,
        "the local normal is along the slab's x: {:?}",
        impact.normal
    );
    let world_point = rotate(turned.rot, impact.point);
    assert!(close(world_point[0] + turned.pos[0], 0.0, 1e-3), "{world_point:?}");
    assert!((world_point[1] + turned.pos[1] - 90.0).abs() < 1.5, "{world_point:?}");
}

#[test]
fn a_body_at_rest_on_the_owner_is_not_an_impact() {
    // the weight of 2 kg for one step is 0.098; the watch asks for several times that
    let mut w = world(
        vec![sphere([30.0, 80.0, 0.0], [0.0; 3], 2.0), slab(upright([30.0, 100.0, 0.0]), [200.0, 10.0, 200.0])],
        9.81,
    )
    .with_impact_watches(watch(0.5))
    .unwrap();
    let frame = w.frame_at(4.0, &mut Watching::default());
    assert!(frame.errors.is_empty() && frame.impacts == [None], "{:?}", frame.impacts);
}

#[test]
fn a_body_moving_away_is_not_an_impact_whatever_the_threshold() {
    let mut w = world(
        vec![
            sphere([30.0, 79.0, 0.0], [0.0, -50.0, 0.0], 2.0),
            slab(upright([30.0, 100.0, 0.0]), [200.0, 10.0, 200.0]),
        ],
        9.81,
    )
    .with_impact_watches(watch(0.0))
    .unwrap();
    assert_eq!(w.frame_at(0.05, &mut Watching::default()).impacts, [None]);
}

#[test]
fn the_owner_is_told_from_the_step_after_the_impact() {
    let mut w = drop_on_slab();
    let mut driver = Watching::default();
    let frame = w.frame_at(0.6, &mut driver);
    let impact = frame.impacts[0].unwrap();
    // `surface` is asked at the end of each step about to be taken
    assert!(
        driver.told.iter().all(|(t, told)| (*told == Some(impact)) == (*t > impact.time + 1e-9)),
        "{:?}",
        driver.told
    );
    assert!(driver.told.iter().any(|(_, told)| told.is_none()) && driver.told.iter().any(|(_, told)| told.is_some()));
}

#[test]
fn the_impact_is_part_of_the_state_replay_and_restore_reproduce_it() {
    let want = drop_on_slab().frame_at(1.5, &mut Watching::default());
    assert!(want.impacts[0].is_some());
    // backward and forward requests, with the frame memory on and off, from checkpoints
    for memory in [true, false] {
        let mut w = drop_on_slab();
        if !memory {
            w = w.with_frame_log_budget(0);
        }
        let mut driver = Watching::default();
        for t in [1.5, 0.05, 0.3, 1.2, 0.0, 1.5, 0.45] {
            let frame = w.frame_at(t, &mut driver);
            assert!(frame.errors.is_empty());
            let impact = want.impacts[0].unwrap();
            assert_eq!(frame.impacts[0].is_some(), t >= (impact.step + 1) as f64 * STEP, "t = {t}, memory {memory}");
            if let Some(found) = frame.impacts[0] {
                assert_eq!(found, impact);
            }
        }
        assert_eq!(w.frame_at(1.5, &mut driver), want);
    }
}

#[test]
fn the_world_with_a_watch_computes_what_the_world_without_one_computes() {
    let bodies =
        vec![sphere([0.0; 3], [0.0, 200.0, 0.0], 2.0), slab(upright([30.0, 100.0, 0.0]), [200.0, 10.0, 200.0])];
    let mut plain = world(bodies, 9.81);
    let mut watched = drop_on_slab();
    let a = plain.frame_at(1.5, &mut Watching::default());
    let b = watched.frame_at(1.5, &mut Watching::default());
    assert_eq!((&a.bodies, &a.velocities), (&b.bodies, &b.velocities));
}

#[test]
fn an_impact_found_from_a_recorded_contact_list_is_the_impact_the_world_found() {
    let config = ContactLogConfig { min_impulse: 0.5, ..ContactLogConfig::new(256, 1 << 20) };
    let mut w = drop_on_slab().with_contact_log(config);
    let frame = w.frame_at(1.5, &mut Watching::default());
    let steps = w.progress().0;
    let contacts: Vec<Contact3> = (0..steps).flat_map(|s| w.contacts_at(s).unwrap().to_vec()).collect();
    let poses: Vec<Pose3> =
        (0..steps).map(|s| w.frame_at(s as f64 * STEP, &mut Watching::default()).bodies[1]).collect();
    let found = find_impact(&watch(1.0)[0], &contacts, |step| poses[step as usize]).unwrap();
    assert_eq!(Some(found), frame.impacts[0], "what a cache holds gives the same impact bit for bit");
}

#[test]
fn watches_must_name_two_different_bodies() {
    let bodies = || vec![sphere([0.0; 3], [0.0; 3], 1.0), slab(upright([0.0, 100.0, 0.0]), [10.0; 3])];
    for (source, owner) in [(0, 0), (0, 2), (5, 1)] {
        let error = world(bodies(), 9.81).with_impact_watches(vec![ImpactWatch { source, owner, min_impulse: 0.0 }]);
        assert!(error.is_err(), "{source} -> {owner}");
    }
}

#[test]
fn shapes_report_the_volume_they_enclose() {
    use std::f64::consts::PI;
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * b;
    assert!(close(shape_volume(&Shape3::Box([1.0, 2.0, 3.0])).unwrap(), 48.0));
    assert!(close(shape_volume(&Shape3::Sphere(2.0)).unwrap(), 4.0 / 3.0 * PI * 8.0));
    assert!(close(shape_volume(&Shape3::Cylinder(1.5, 2.0)).unwrap(), PI * 4.0 * 3.0));
    assert!(close(shape_volume(&Shape3::Cone(1.5, 2.0)).unwrap(), PI * 4.0 * 3.0 / 3.0));
    assert!(close(shape_volume(&Shape3::Capsule(1.0, 1.0)).unwrap(), PI * 2.0 + 4.0 / 3.0 * PI));
    // a unit cube as a closed mesh and as points, wherever it sits
    let corners: Vec<[f64; 3]> =
        (0..8).map(|k| [(k & 1) as f64 + 5.0, ((k >> 1) & 1) as f64 - 2.0, ((k >> 2) & 1) as f64]).collect();
    let faces = [
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
    assert!(close(shape_volume(&Shape3::TriMesh(corners.clone(), faces.to_vec())).unwrap(), 1.0));
    assert!(close(shape_volume(&Shape3::Convex(corners)).unwrap(), 1.0));
    assert!(shape_volume(&Shape3::Sphere(0.0)).is_err());
    assert!(shape_volume(&Shape3::TriMesh(vec![[0.0; 3]], vec![[0, 1, 2]])).is_err());
}

#[test]
fn with_two_sources_watched_against_one_owner_the_owner_is_told_the_earliest_impact() {
    // the first watch's sphere falls from 0 and the second's from a lower start: the second one lands first
    let mut w = world(
        vec![
            sphere([0.0; 3], [0.0, 200.0, 0.0], 2.0),
            slab(upright([30.0, 100.0, 0.0]), [200.0, 10.0, 200.0]),
            sphere([60.0, 40.0, 0.0], [0.0, 200.0, 0.0], 2.0),
        ],
        9.81,
    )
    .with_impact_watches(vec![
        ImpactWatch { source: 0, owner: 1, min_impulse: 1.0 },
        ImpactWatch { source: 2, owner: 1, min_impulse: 1.0 },
    ])
    .unwrap();
    let mut driver = Watching::default();
    let frame = w.frame_at(0.6, &mut driver);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    let (first, second) = (frame.impacts[0].expect("the first lands"), frame.impacts[1].expect("the second lands"));
    assert!(second.time < first.time, "{} {}", second.time, first.time);
    // the surface is asked about the earlier of them, from the step after it, and about no other
    let told: Vec<Impact3> = driver.told.iter().filter_map(|(_, i)| *i).collect();
    assert!(told.iter().all(|i| *i == second || *i == first), "{told:?}");
    assert_eq!(driver.told.last().unwrap().1, Some(second), "the earliest impact is the one the owner is told");
}
