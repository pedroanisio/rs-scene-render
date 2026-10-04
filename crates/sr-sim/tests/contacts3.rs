//! The 3D rigid world records the contacts it resolves: instant, bodies, point,
//! normal, normal impulse and the relative velocity before the contact.

use sr_sim::{fields::Field, physics3d::*};

struct Still;
impl Driver3 for Still {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
}

const STEP: f64 = 0.005;

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

/// Scene axes: y points down, so a floor at y = 100 is below a sphere at y = 0.
fn world(bodies: Vec<Body3Spec>, gravity: f64, bounds: Bounds3) -> World3 {
    World3::new(World3Spec {
        start: 0.0,
        step: STEP,
        gravity: [0.0, -gravity, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds,
        joints: vec![],
        bodies,
    })
}

fn limits() -> ContactLogConfig {
    ContactLogConfig::new(256, 1 << 20)
}

/// Every contact of steps `0..steps`, in step order.
fn all_contacts(w: &World3, steps: u64) -> Vec<Contact3> {
    (0..steps).flat_map(|s| w.contacts_at(s).expect("log enabled").to_vec()).collect()
}

fn first_contact_step(w: &World3, steps: u64) -> u64 {
    (0..steps).find(|&s| !w.contacts_at(s).unwrap().is_empty()).expect("a contact")
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance * b.abs().max(1.0)
}

#[test]
fn nothing_is_recorded_while_the_body_is_in_the_air_or_when_logging_is_off() {
    let mut w =
        world(vec![sphere([0.0; 3], [0.0; 3], 2.0)], 9.81, Bounds3::Floor { y: 1.0e6 }).with_contact_log(limits());
    let frame = w.frame_at(1.0, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert_eq!(w.progress().0, 200);
    assert!(all_contacts(&w, 200).is_empty());
    let mut off = world(vec![sphere([0.0; 3], [0.0; 3], 2.0)], 9.81, Bounds3::Floor { y: 1.0e6 });
    off.frame_at(0.5, &mut Still);
    assert!(off.contacts_at(0).is_none(), "logging is opt-in");
}

#[test]
fn a_sphere_dropped_on_the_floor_makes_one_contact_with_the_momentum_change() {
    // Mass 2, falling at 200 px/s onto the floor at y = 100 (sphere radius 10, restitution 0).
    let mut w = world(vec![sphere([0.0; 3], [0.0, 200.0, 0.0], 2.0)], 9.81, Bounds3::Floor { y: 100.0 })
        .with_contact_log(limits());
    let frame = w.frame_at(0.7, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    let steps = w.progress().0;
    let first = first_contact_step(&w, steps);
    let contacts = w.contacts_at(first).unwrap();
    assert_eq!(contacts.len(), 1, "{contacts:?}");
    let c = contacts[0];
    assert_eq!(c.step, first);
    assert!(close(c.time, (first + 1) as f64 * STEP, 1e-12), "time is the end of the resolving step");
    // The sphere is body 0; the floor slab belongs to no body.
    assert_eq!(c.bodies, [Some(0), None]);
    // Contact on the floor surface under the sphere; the normal points from body 0 to the floor (+y, down).
    assert!((c.point[1] - 100.0).abs() < 1.5, "{:?}", c.point);
    assert!(c.point[0].abs() < 1e-6 && c.point[2].abs() < 1e-6);
    assert!(close(c.normal[1], 1.0, 1e-9) && c.normal[0].abs() < 1e-9 && c.normal[2].abs() < 1e-9);
    // The floor approaches the sphere at the sphere's speed: negative along the normal.
    let closing = -c.relative_velocity[1];
    assert!(close(closing, 200.0 + 9.81 * first as f64 * STEP, 0.01), "{closing}");
    // Declared tolerance: the impulse equals mass x velocity change (restitution 0) within 5%.
    assert!(close(c.impulse, 2.0 * closing, 0.05), "impulse {} vs {}", c.impulse, 2.0 * closing);
}

/// A square ground of `cells` x `cells` quads, `size` wide, as a triangle mesh in the body's own axes.
fn ground_mesh(size: f64, cells: usize) -> Shape3 {
    let n = cells + 1;
    let points = (0..n * n)
        .map(|k| {
            [(k % n) as f64 * size / cells as f64 - size / 2.0, 0.0, (k / n) as f64 * size / cells as f64 - size / 2.0]
        })
        .collect();
    let mut triangles = Vec::new();
    for z in 0..cells {
        for x in 0..cells {
            let a = (z * n + x) as u32;
            let (b, c, d) = (a + 1, a + n as u32, a + n as u32 + 1);
            triangles.push([a, c, b]);
            triangles.push([b, c, d]);
        }
    }
    Shape3::TriMesh(points, triangles)
}

#[test]
fn a_sphere_landing_on_a_triangle_mesh_is_recorded_through_the_solver_clusters() {
    // Composite shapes (meshes) are solved through merged contact clusters, where the
    // plain manifolds carry no impulse; the record must still see the contact.
    let mut ground = sphere([0.0, 100.0, 0.0], [0.0; 3], 1.0);
    ground.kind = BodyKind::Static;
    ground.shape = ground_mesh(400.0, 8);
    let ball = sphere([0.0, 0.0, 0.0], [0.0, 200.0, 0.0], 2.0);
    let mut w = world(vec![ball, ground], 9.81, Bounds3::None).with_contact_log(limits());
    let frame = w.frame_at(0.7, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    let first = first_contact_step(&w, w.progress().0);
    let contacts = w.contacts_at(first).unwrap();
    assert!(contacts.iter().all(|c| c.bodies == [Some(0), Some(1)]), "{contacts:?}");
    let closing = -contacts[0].relative_velocity[1];
    assert!(close(closing, 200.0 + 9.81 * first as f64 * STEP, 0.01), "{closing}");
    // The record's points can be many (one per supporting triangle corner); their impulses add up.
    let total: f64 = contacts.iter().map(|c| c.impulse).sum();
    assert!(close(total, 2.0 * closing, 0.05), "total impulse {total} vs {}", 2.0 * closing);
    assert!(contacts.iter().all(|c| close(c.normal[1], 1.0, 1e-6) && (c.point[1] - 100.0).abs() < 1.5));
}

#[test]
fn an_oblique_impact_reports_the_relative_velocity_direction() {
    let v = [120.0, 160.0, 0.0];
    let mut w = world(vec![sphere([0.0; 3], v, 1.0)], 0.0, Bounds3::Floor { y: 100.0 }).with_contact_log(limits());
    w.frame_at(0.8, &mut Still);
    let first = first_contact_step(&w, w.progress().0);
    let c = w.contacts_at(first).unwrap()[0];
    // Zero gravity: the pre-step velocity is the launch velocity, and the floor moves relative to it as -v.
    for (got, launch) in c.relative_velocity.iter().zip(v) {
        assert!((got + launch).abs() < 1e-6, "{:?}", c.relative_velocity);
    }
    assert!(close(c.normal[1], 1.0, 1e-9));
    // The normal impulse follows the normal component of the approach only (friction is separate).
    assert!(close(c.impulse, 160.0, 0.05), "{}", c.impulse);
}

#[test]
fn two_bodies_meeting_head_on_share_the_contact() {
    // Equal masses of 1 closing at 100 px/s, restitution 0: each loses 50 px/s of normal velocity.
    let a = sphere([0.0, 0.0, 0.0], [50.0, 0.0, 0.0], 1.0);
    let b = sphere([100.0, 0.0, 0.0], [-50.0, 0.0, 0.0], 1.0);
    let mut w = world(vec![a, b], 0.0, Bounds3::None).with_contact_log(limits());
    w.frame_at(1.0, &mut Still);
    let first = first_contact_step(&w, w.progress().0);
    let contacts = w.contacts_at(first).unwrap();
    assert_eq!(contacts.len(), 1, "{contacts:?}");
    let c = contacts[0];
    assert_eq!(c.bodies, [Some(0), Some(1)]);
    // The normal points from body 0 to body 1 (+x); body 1 moves toward body 0 at 100 px/s.
    assert!(close(c.normal[0], 1.0, 1e-9));
    assert!((c.relative_velocity[0] + 100.0).abs() < 1e-6, "{:?}", c.relative_velocity);
    assert!((c.point[0] - 50.0).abs() < 1.5, "{:?}", c.point);
    assert!(close(c.impulse, 50.0, 0.05), "{}", c.impulse);
}

#[test]
fn contacts_are_listed_in_a_stable_order() {
    // Three spheres land together on the floor; the order is by body pair, then point.
    let bodies = vec![
        sphere([-60.0, 0.0, 0.0], [0.0, 200.0, 0.0], 1.0),
        sphere([0.0; 3], [0.0, 200.0, 0.0], 1.0),
        sphere([60.0, 0.0, 0.0], [0.0, 200.0, 0.0], 1.0),
    ];
    let mut w = world(bodies, 9.81, Bounds3::Floor { y: 100.0 }).with_contact_log(limits());
    w.frame_at(0.7, &mut Still);
    let first = first_contact_step(&w, w.progress().0);
    let contacts = w.contacts_at(first).unwrap();
    assert_eq!(
        contacts.iter().map(|c| c.bodies).collect::<Vec<_>>(),
        [[Some(0), None], [Some(1), None], [Some(2), None]]
    );
    for step in 0..w.progress().0 {
        let key = |c: &Contact3| (c.bodies.map(|b| b.unwrap_or(usize::MAX)), c.point.map(f64::to_bits));
        let list = w.contacts_at(step).unwrap();
        assert!(list.windows(2).all(|w| key(&w[0]) <= key(&w[1])) || list.len() < 2, "step {step}");
    }
}

#[test]
fn the_record_is_identical_after_a_backward_seek_and_in_a_fresh_world() {
    let bodies = vec![sphere([0.0; 3], [30.0, 200.0, 10.0], 2.0), sphere([40.0, -60.0, 0.0], [-20.0, 150.0, 0.0], 1.0)];
    let make = || world(bodies.clone(), 9.81, Bounds3::Floor { y: 100.0 }).with_contact_log(limits());
    let mut w = make();
    w.frame_at(1.5, &mut Still);
    let steps = w.progress().0;
    let forward = all_contacts(&w, steps);
    assert!(!forward.is_empty());
    // Backward seek: the world restores a checkpoint and replays; steps not yet replayed are hidden.
    w.frame_at(0.2, &mut Still);
    assert!(w.progress().0 < steps);
    assert!(w.contacts_at(steps - 1).is_none(), "steps beyond the current one are not reported");
    w.frame_at(1.5, &mut Still);
    assert_eq!(all_contacts(&w, steps), forward, "replay reproduces the record bit for bit");
    let mut fresh = make();
    fresh.frame_at(1.5, &mut Still);
    assert_eq!(all_contacts(&fresh, steps), forward);
    // Retained events are accounted, and can be released.
    assert!(w.contact_log_bytes() > 0 && w.contact_log_bytes() <= limits().max_bytes);
    w.discard_contacts_before(steps / 2);
    assert!(w.contacts_at(0).is_none() && w.contacts_at(steps - 1).is_some());
}

#[test]
fn limits_are_errors_not_truncation() {
    let falling = || world(vec![sphere([0.0; 3], [0.0, 200.0, 0.0], 1.0)], 9.81, Bounds3::Floor { y: 100.0 });
    let mut per_step = falling().with_contact_log(ContactLogConfig::new(0, 1 << 20));
    let frame = per_step.frame_at(0.7, &mut Still);
    assert!(frame.bodies.is_empty() && frame.errors[0].contains("contacts in one step"), "{:?}", frame.errors);
    let mut memory = falling().with_contact_log(ContactLogConfig::new(16, 200));
    let frame = memory.frame_at(0.7, &mut Still);
    assert!(frame.bodies.is_empty() && frame.errors[0].contains("contact log memory"), "{:?}", frame.errors);
    // A failed step records nothing and the failure repeats on retry: the world
    // returns to a checkpoint, so only a larger limit lets the step through.
    let again = memory.frame_at(0.7, &mut Still);
    assert_eq!(again.errors, frame.errors);
    assert!(memory.contacts_at(memory.progress().0).is_none());
    let mut roomy = memory.with_contact_log(ContactLogConfig::new(16, 1 << 20));
    assert!(roomy.frame_at(0.7, &mut Still).errors.is_empty());
    assert!(all_contacts(&roomy, roomy.progress().0).len() > 1);
}

#[test]
fn the_impulse_threshold_keeps_impacts_and_drops_resting_contacts() {
    let run = |min_impulse: f64| {
        let mut w = world(vec![sphere([0.0; 3], [0.0, 200.0, 0.0], 2.0)], 9.81, Bounds3::Floor { y: 100.0 })
            .with_contact_log(ContactLogConfig { min_impulse, ..limits() });
        w.frame_at(1.5, &mut Still);
        let steps = w.progress().0;
        all_contacts(&w, steps)
    };
    let everything = run(0.0);
    // At rest the sphere pushes with mass x gravity x step = 2 x 9.81 x 0.005 ~ 0.1 per step.
    assert!(everything.len() > 100, "{}", everything.len());
    let impacts = run(1.0);
    assert!(!impacts.is_empty() && impacts.len() < 10, "{}", impacts.len());
    assert!(impacts.iter().all(|c| c.impulse > 1.0));
    assert!(impacts.iter().map(|c| c.impulse).fold(0.0, f64::max) > 300.0);
    // The kept points are the same records, not recomputed ones.
    assert!(impacts.iter().all(|c| everything.contains(c)));
}

#[test]
fn recording_contacts_does_not_change_the_simulation() {
    let bodies = vec![sphere([0.0; 3], [30.0, 200.0, 10.0], 2.0), sphere([40.0, -60.0, 0.0], [-20.0, 150.0, 0.0], 1.0)];
    let mut plain = world(bodies.clone(), 9.81, Bounds3::Floor { y: 100.0 });
    let mut logged = world(bodies, 9.81, Bounds3::Floor { y: 100.0 }).with_contact_log(limits());
    for t in [0.3, 0.9, 1.5, 0.4, 2.0] {
        assert_eq!(plain.frame_at(t, &mut Still), logged.frame_at(t, &mut Still), "t={t}");
    }
}

/// `cargo test --release -p sr-sim --test contacts3 -- --ignored --nocapture logging_cost`
#[test]
#[ignore = "measurement"]
fn logging_cost() {
    for count in [10usize, 100, 400] {
        let side = (count as f64).sqrt().ceil() as usize;
        let bodies: Vec<Body3Spec> = (0..count)
            .map(|i| {
                let (x, z) = ((i % side) as f64 * 24.0, (i / side) as f64 * 24.0);
                // Close to the floor, so nearly every step has a contact per body.
                let height = 70.0 + (i * 7 % 5) as f64 * 3.0;
                sphere([x, height, z], [0.0, 50.0, 0.0], 1.0)
            })
            .collect();
        let steps = 600u64;
        let time = |logging: bool| {
            let mut w = world(bodies.clone(), 9.81, Bounds3::Floor { y: 100.0 });
            if logging {
                w = w.with_contact_log(ContactLogConfig::new(1 << 20, 1 << 30));
            }
            let started = std::time::Instant::now();
            let frame = w.frame_at(steps as f64 * STEP, &mut Still);
            assert!(frame.errors.is_empty(), "{:?}", frame.errors);
            let events = if logging { (0..steps).map(|s| w.contacts_at(s).unwrap().len()).sum() } else { 0 };
            (started.elapsed().as_secs_f64() * 1e3 / steps as f64, events)
        };
        let (mut off, mut on, mut events) = (f64::MAX, f64::MAX, 0);
        for _ in 0..3 {
            off = off.min(time(false).0);
            let (t, n) = time(true);
            on = on.min(t);
            events = n;
        }
        println!("bodies {count}: step {off:.3} ms without, {on:.3} ms with the record ({events} contacts over {steps} steps)");
    }
}
