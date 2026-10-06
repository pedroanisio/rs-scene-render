//! The rigid world remembers the frames it has already simulated, so a request for an
//! earlier time is answered without restoring a checkpoint and replaying. The answer must
//! be the one a world without the memory gives, whatever the order of the requests.

use sr_sim::{fields::Field, physics3d::*};

const STEP: f64 = 0.01;

/// A scene with everything a replay has to reproduce: a surface that deforms while a body
/// rests on it, a body that appears at a time, a body that is released at a time, and a
/// fracture.
struct Scene {
    step_calls: u64,
}

impl Driver3 for Scene {
    fn collider(&mut self, t: f64, which: usize, revision: Option<u64>) -> Result<Option<ColliderUpdate3>, String> {
        if which != 0 {
            return Ok(None);
        }
        // the floor sinks by 4 units every half second
        let level = (t / 0.5).floor() as u64;
        if revision == Some(level) {
            return Ok(None);
        }
        let y = 60.0 + 4.0 * level as f64;
        Ok(Some(ColliderUpdate3 {
            revision: level,
            vertices: vec![[-400., y, -400.], [400., y, -400.], [400., y, 400.], [-400., y, 400.]],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            max_bytes: 1 << 20,
        }))
    }
    fn enabled(&mut self, t: f64, which: usize) -> bool {
        which != 4 || t >= 0.2
    }
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        which.iter().map(|_| Pose3 { pos: [60., -100., 0.], ..Pose3::default() }).collect()
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        self.step_calls += 1;
        vec![]
    }
}

fn body(kind: BodyKind, shape: Shape3, mass: f64, at: [f64; 3], velocity: [f64; 3], activate_at: f64) -> Body3Spec {
    Body3Spec {
        kind,
        shape,
        mass,
        friction: 0.4,
        restitution: 0.1,
        linear_damping: 0.0,
        angular_damping: 0.0,
        velocity,
        angular_velocity: [0.0, 0.0, 30.0],
        group: 0,
        collides_with: None,
        sensor: false,
        fixed_rotation: false,
        bullet: false,
        activate_at,
        start: Pose3 { pos: at, ..Pose3::default() },
    }
}

fn spec() -> World3Spec {
    World3Spec {
        fix_internal_edges: false,
        start: 0.0,
        step: STEP,
        gravity: [0.0, -200.0, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies: vec![
            body(BodyKind::Static, Shape3::Box([1.0; 3]), 1.0, [0.0; 3], [0.0; 3], 0.0),
            body(BodyKind::Dynamic, Shape3::Sphere(10.0), 4.0, [0.0, -100.0, 0.0], [5.0, 0.0, 0.0], 0.0),
            body(BodyKind::Dynamic, Shape3::Sphere(7.0), 2.0, [0.0, -100.0, 0.0], [0.0; 3], 0.0),
            body(BodyKind::Dynamic, Shape3::Sphere(7.0), 2.0, [0.0, -100.0, 0.0], [0.0; 3], 0.0),
            body(BodyKind::Dynamic, Shape3::Sphere(8.0), 3.0, [60.0, -100.0, 0.0], [-20.0, 0.0, 0.0], 0.5),
        ],
    }
}

fn fracture() -> Fracture3 {
    Fracture3 {
        source: 1,
        at: 0.95,
        radial_impulse: 30.0,
        fragments: vec![
            Fragment3 { body: 2, offset: [-3.0, 0.0, 0.0], impulse: [0.0; 3] },
            Fragment3 { body: 3, offset: [3.0, 0.0, 0.0], impulse: [0.0; 3] },
        ],
        contact: None,
    }
}

fn world() -> World3 {
    World3::new(spec()).with_fractures(vec![fracture()]).unwrap()
}

/// The same world, without the memory of frames.
fn reference() -> World3 {
    world().with_frame_log_budget(0)
}

/// Request times in a fixed pseudo-random order, on step boundaries and between them.
fn times(count: usize, end: f64) -> Vec<f64> {
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    (0..count)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 11) as f64 / (1u64 << 53) as f64 * end
        })
        .collect()
}

#[test]
fn an_earlier_request_is_answered_without_stepping_again() {
    let mut w = world();
    let mut scene = Scene { step_calls: 0 };
    let last = w.frame_at(3.0, &mut scene);
    assert!(last.errors.is_empty(), "{:?}", last.errors);
    let (steps, _) = w.progress();
    let stepped = scene.step_calls;
    assert_eq!(steps, 300);
    for t in [2.99, 2.0, 1.5, 1.0, 0.5, 0.25, 0.0] {
        let f = w.frame_at(t, &mut scene);
        assert!(f.errors.is_empty(), "{:?}", f.errors);
    }
    assert_eq!(scene.step_calls, stepped, "no step was recomputed");
    assert_eq!(w.progress().0, steps, "the world stays where it had got to");
    assert_eq!(w.frame_at(3.0, &mut scene), last);
}

#[test]
fn any_order_of_requests_gives_the_frames_of_a_world_without_the_memory() {
    let mut with = world();
    let mut without = reference();
    let mut a = Scene { step_calls: 0 };
    let mut b = Scene { step_calls: 0 };
    let mut ahead = 0;
    for (i, t) in times(160, 3.2).into_iter().enumerate() {
        let got = with.frame_at(t, &mut a);
        let want = without.frame_at(t, &mut b);
        assert!(want.errors.is_empty(), "{:?}", want.errors);
        assert_eq!(got, want, "request {i} at t = {t}");
        ahead = ahead.max(with.progress().0);
    }
    assert!(ahead > 250);
    assert!(a.step_calls < b.step_calls, "the memory saves steps: {} vs {}", a.step_calls, b.step_calls);
    // and from scratch, forward and then backward
    for t in [0.0, 0.19, 0.2, 0.5, 0.95, 1.0, 2.5] {
        assert_eq!(with.frame_at(t, &mut a), world().frame_at(t, &mut Scene { step_calls: 0 }), "t = {t}");
    }
}

#[test]
fn a_time_between_two_steps_reports_the_earlier_step() {
    let mut w = world();
    let mut scene = Scene { step_calls: 0 };
    w.frame_at(1.5, &mut scene);
    for k in [0u64, 1, 37, 94, 95, 96, 120] {
        let on = w.frame_at(k as f64 * STEP, &mut scene);
        let between = w.frame_at((k as f64 + 0.4) * STEP, &mut scene);
        assert_eq!(on, between, "step {k}");
    }
}

#[test]
fn what_is_reported_matches_the_scene_at_the_instants_it_changes() {
    let mut w = world();
    let mut scene = Scene { step_calls: 0 };
    w.frame_at(2.0, &mut scene);
    let before = w.frame_at(0.19, &mut scene);
    let appears = w.frame_at(0.2, &mut scene);
    assert!(!before.enabled[4] && appears.enabled[4]);
    let held = w.frame_at(0.49, &mut scene);
    let released = w.frame_at(0.5, &mut scene);
    let moving = w.frame_at(0.52, &mut scene);
    // the velocity of a released body is set by the step that starts at the release, so
    // the frame at the release still shows it held, as the replayed world does
    assert_eq!(held.velocities[4].linear[0], 0.0);
    assert_eq!(released.velocities[4].linear[0], 0.0);
    assert!(moving.velocities[4].linear[0] < -19.0);
    let mut replayed = reference();
    let mut other = Scene { step_calls: 0 };
    for t in [0.19, 0.2, 0.49, 0.5, 0.52, 0.94, 0.95, 0.96] {
        assert_eq!(w.frame_at(t, &mut scene), replayed.frame_at(t, &mut other), "t = {t}");
    }
    let unfractured = w.frame_at(0.94, &mut scene);
    let fractured = w.frame_at(0.96, &mut scene);
    assert_eq!(unfractured.fractured, [false]);
    assert_eq!(fractured.fractured, [true]);
    assert_eq!(fractured.enabled, [true, false, true, true, true]);
}

#[test]
fn a_request_for_a_discarded_step_replays_to_the_same_frame() {
    let mut w = world();
    let mut scene = Scene { step_calls: 0 };
    w.frame_at(3.0, &mut scene);
    let per_frame = w.frame_log_bytes() / w.frame_log_len();
    // room for the last 40 steps only
    let mut small = world().with_frame_log_budget(40 * per_frame + per_frame / 2);
    let mut without = reference();
    let mut other = Scene { step_calls: 0 };
    for t in [3.0, 2.7, 1.0, 0.3, 2.95, 0.0, 2.0, 2.99, 1.25] {
        let got = small.frame_at(t, &mut scene);
        assert_eq!(got, without.frame_at(t, &mut other), "t = {t}");
        assert!(small.frame_log_len() <= 40, "{} frames kept", small.frame_log_len());
        assert!(small.frame_log_bytes() <= 40 * per_frame + per_frame / 2);
    }
    // the old step needed a replay, the recent one did not
    let calls = scene.step_calls;
    small.frame_at(2.99, &mut scene);
    assert_eq!(scene.step_calls, calls);
    small.frame_at(0.4, &mut scene);
    assert!(scene.step_calls > calls);
}

#[test]
fn the_memory_does_not_change_what_the_world_computes() {
    let mut with = world();
    let mut without = reference();
    let mut a = Scene { step_calls: 0 };
    let mut b = Scene { step_calls: 0 };
    for t in [0.7, 0.2, 1.4, 0.0, 2.2, 1.0, 3.0] {
        with.frame_at(t, &mut a);
        without.frame_at(t, &mut b);
    }
    // the world that went on to step 300 and the one that kept going back agree from there on
    for t in [3.0, 3.5, 4.0] {
        assert_eq!(with.frame_at(t, &mut a), without.frame_at(t, &mut b), "t = {t}");
    }
}

#[test]
fn a_failed_step_leaves_nothing_in_the_memory() {
    struct Failing(Scene, bool);
    impl Driver3 for Failing {
        fn collider(&mut self, t: f64, which: usize, revision: Option<u64>) -> Result<Option<ColliderUpdate3>, String> {
            if self.1 && t > 1.0 {
                return Err("surface unavailable".into());
            }
            self.0.collider(t, which, revision)
        }
        fn enabled(&mut self, t: f64, which: usize) -> bool {
            self.0.enabled(t, which)
        }
        fn kinematic(&mut self, t: f64, which: &[usize]) -> Vec<Pose3> {
            self.0.kinematic(t, which)
        }
        fn fields(&mut self, t: f64) -> Vec<Field> {
            self.0.fields(t)
        }
    }
    let mut w = world();
    let mut driver = Failing(Scene { step_calls: 0 }, true);
    let failed = w.frame_at(2.0, &mut driver);
    assert!(failed.bodies.is_empty());
    assert!(failed.errors[0].contains("unavailable"));
    // an earlier time that was simulated is still answered; a later one still fails
    assert!(w.frame_at(0.5, &mut driver).errors.is_empty());
    assert!(!w.frame_at(2.0, &mut driver).errors.is_empty());
    driver.1 = false;
    let repaired = w.frame_at(2.0, &mut driver);
    assert_eq!(repaired, reference().frame_at(2.0, &mut Scene { step_calls: 0 }));
}

#[test]
fn the_budget_is_shared_with_the_contact_record_which_has_priority() {
    let config = ContactLogConfig { min_impulse: 1.0, ..ContactLogConfig::new(256, 1 << 20) };
    let mut probe = world().with_contact_log(config);
    let mut scene = Scene { step_calls: 0 };
    probe.frame_at(3.0, &mut scene);
    let per_frame = probe.frame_log_bytes() / probe.frame_log_len();
    let contacts = probe.contact_log_bytes();
    assert!(contacts > 0, "the scene has contacts");
    let budget = contacts + 25 * per_frame;
    let mut w = world().with_contact_log(config).with_frame_log_budget(budget);
    w.frame_at(3.0, &mut scene);
    assert_eq!(w.contact_log_bytes(), contacts, "frames never displace contacts");
    assert!(w.frame_log_bytes() + w.contact_log_bytes() <= budget);
    assert!(w.frame_log_len() >= 20, "the budget left room for frames: {}", w.frame_log_len());
    // a budget the contacts alone exceed leaves no frames, and the world still answers
    let mut none = world().with_contact_log(config).with_frame_log_budget(contacts / 2);
    assert_eq!(none.frame_at(3.0, &mut scene), reference().frame_at(3.0, &mut Scene { step_calls: 0 }));
    assert_eq!(none.frame_log_len(), 0);
    assert_eq!(none.contact_log_bytes(), contacts);
}

#[test]
fn recording_frames_costs_a_bounded_amount_per_step() {
    let mut w = world();
    let mut scene = Scene { step_calls: 0 };
    w.frame_at(3.0, &mut scene);
    let per_frame = w.frame_log_bytes() / w.frame_log_len();
    assert!(per_frame < 1024, "five bodies cost {per_frame} bytes a step");
    assert!(w.frame_log_bytes() < w.frame_log_budget());
    assert_eq!(world().with_frame_log_budget(0).frame_log_len(), 0);
}
