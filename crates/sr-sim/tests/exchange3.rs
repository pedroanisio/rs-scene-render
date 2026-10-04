//! What solvers in a coupled group pass each other: an append-only log of small records, and a
//! per-step load on the rigid bodies read from it. A replay must see what the first run saw.

use sr_sim::exchange::{ExchangeLog, Put, Record};
use sr_sim::{fields::Field, physics3d::*};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Sample {
    level: f64,
    velocity: [f64; 2],
}

impl Record for Sample {
    fn same(&self, other: &Self) -> bool {
        self.level.to_bits() == other.level.to_bits()
            && self.velocity.iter().zip(&other.velocity).all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

fn sample(level: f64) -> Sample {
    Sample { level, velocity: [level * 0.5, -level] }
}

#[test]
fn records_are_kept_by_channel_and_step_and_replayed_unchanged() {
    let mut log = ExchangeLog::new(1 << 16);
    assert_eq!(log.put(0, 7, &[sample(1.0), sample(2.0)]), Ok(Put::Stored));
    assert_eq!(log.put(1, 7, &[sample(3.0)]), Ok(Put::Stored));
    assert_eq!(log.get(0, 7), Some(&[sample(1.0), sample(2.0)][..]));
    assert_eq!(log.get(1, 7), Some(&[sample(3.0)][..]));
    assert_eq!(log.get(0, 8), None);
    // a replay that reproduces the record is fine and changes nothing
    let bytes = log.bytes();
    assert_eq!(log.put(0, 7, &[sample(1.0), sample(2.0)]), Ok(Put::Replayed));
    assert_eq!(log.bytes(), bytes);
    assert_eq!(log.len(), 2);
}

#[test]
fn a_replay_that_differs_is_an_error_to_the_last_bit() {
    let mut log = ExchangeLog::new(1 << 16);
    log.put(0, 3, &[sample(1.0)]).unwrap();
    for other in [sample(1.0 + f64::EPSILON), sample(-0.0)] {
        let error = log.put(0, 3, &[other]).unwrap_err();
        assert!(error.contains("diverge"), "{error}");
    }
    assert!(log.put(0, 3, &[sample(1.0), sample(2.0)]).unwrap_err().contains("diverge"), "a different count");
    // zero and negative zero are different bits
    log.put(0, 4, &[sample(0.0)]).unwrap();
    assert!(log.put(0, 4, &[Sample { level: -0.0, velocity: [0.0, -0.0] }]).unwrap_err().contains("diverge"));
    assert_eq!(log.get(0, 3), Some(&[sample(1.0)][..]), "the first record stands");
}

#[test]
fn the_log_has_a_budget_and_exceeding_it_is_an_error_that_stores_nothing() {
    let mut log = ExchangeLog::new(600);
    let mut stored = 0;
    let error = loop {
        match log.put(0, stored, &[sample(stored as f64)]) {
            Ok(_) => stored += 1,
            Err(e) => break e,
        }
        assert!(stored < 100, "the budget never ran out");
    };
    assert!(error.contains("budget"), "{error}");
    assert!(log.bytes() <= 600);
    assert_eq!(log.get(0, stored), None);
    assert_eq!(log.len() as u64, stored);
}

// ----------------------------------------------------------------------------- loads

const STEP: f64 = 0.01;

struct Loaded {
    log: ExchangeLog<LoadRecord>,
    asked: Vec<(u64, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct LoadRecord(Load3);

impl Record for LoadRecord {
    fn same(&self, other: &Self) -> bool {
        self.0
            .force
            .iter()
            .chain(&self.0.torque)
            .zip(other.0.force.iter().chain(&other.0.torque))
            .all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

impl Driver3 for Loaded {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
    fn load(&mut self, step: u64, _t: f64, body: usize) -> Result<Option<Load3>, String> {
        self.asked.push((step, body));
        match self.log.get(body as u32, step) {
            Some([record]) => Ok(Some(record.0)),
            Some(_) => Err("a load record is not one record".into()),
            None => Err(format!("the load of step {step} on body {body} is not computed yet")),
        }
    }
}

fn body() -> Body3Spec {
    Body3Spec {
        kind: BodyKind::Dynamic,
        shape: Shape3::Sphere(10.0),
        mass: 4.0,
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
        start: Pose3::default(),
    }
}

fn world() -> World3 {
    World3::new(World3Spec {
        start: 0.0,
        step: STEP,
        gravity: [0.0; 3],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies: vec![body(), Body3Spec { start: Pose3 { pos: [200.0, 0.0, 0.0], ..Pose3::default() }, ..body() }],
    })
}

/// A load that depends on the step and the body only.
fn load_of(step: u64, body: usize) -> Load3 {
    let s = step as f64;
    Load3 { force: [8.0 + s * 0.01, 0.0, body as f64], torque: [0.0, 0.0, 2.0] }
}

fn filled(steps: u64) -> Loaded {
    let mut log = ExchangeLog::new(1 << 20);
    for step in 0..steps {
        for body in 0..2 {
            log.put(body as u32, step, &[LoadRecord(load_of(step, body))]).unwrap();
        }
    }
    Loaded { log, asked: vec![] }
}

#[test]
fn a_load_changes_a_bodys_momentum_by_its_impulse() {
    let mut w = world();
    let mut driver = filled(200);
    let frame = w.frame_at(1.0, &mut driver);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    // force 8 + 0.01 s on body 0 for 100 steps of 0.01 s, mass 4, in scene axes: dv = F dt / m
    let impulse: f64 = (0..100).map(|s| (8.0 + 0.01 * s as f64) * STEP).sum();
    assert!((frame.velocities[0].linear[0] - impulse / 4.0).abs() < 1e-9, "{:?}", frame.velocities[0]);
    assert!(frame.velocities[1].linear[2] > frame.velocities[0].linear[2], "a different load on each body");
    // the torque turns it about z: a sphere of mass 4 and radius 10 has I = 0.4 m r^2 = 160 (scene units)
    let spin = (2.0 / 160.0f64 * 1.0).to_degrees();
    assert!((frame.velocities[0].angular[2] - spin).abs() < 1e-6 * spin, "{:?} vs {spin}", frame.velocities[0]);
}

#[test]
fn a_driver_without_loads_gives_exactly_the_world_there_was() {
    struct Plain;
    impl Driver3 for Plain {
        fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
            vec![Pose3::default(); which.len()]
        }
        fn fields(&mut self, _: f64) -> Vec<Field> {
            vec![]
        }
    }
    struct None3(Plain);
    impl Driver3 for None3 {
        fn kinematic(&mut self, t: f64, which: &[usize]) -> Vec<Pose3> {
            self.0.kinematic(t, which)
        }
        fn fields(&mut self, t: f64) -> Vec<Field> {
            self.0.fields(t)
        }
        fn load(&mut self, _: u64, _: f64, _: usize) -> Result<Option<Load3>, String> {
            Ok(None)
        }
    }
    let mut moving = body();
    moving.velocity = [5.0, 0.0, 1.0];
    let with_motion = || {
        World3::new(World3Spec {
            start: 0.0,
            step: STEP,
            gravity: [0.0, -9.8, 0.0],
            pixels_per_meter: 1.0,
            iterations: 8,
            bounds: Bounds3::None,
            joints: vec![],
            bodies: vec![moving.clone()],
        })
    };
    let (mut d, mut e) = (with_motion(), with_motion());
    assert_eq!(d.frame_at(2.0, &mut Plain), e.frame_at(2.0, &mut None3(Plain)));
}

#[test]
fn replay_from_a_checkpoint_reads_the_same_loads_and_gives_the_same_world() {
    let want = world().frame_at(2.5, &mut filled(400));
    for memory in [true, false] {
        let mut w = world();
        if !memory {
            w = w.with_frame_log_budget(0);
        }
        let mut driver = filled(400);
        for t in [2.5, 0.4, 1.7, 0.0, 2.2, 1.0, 2.5] {
            assert!(w.frame_at(t, &mut driver).errors.is_empty(), "t = {t}");
        }
        assert_eq!(w.frame_at(2.5, &mut driver), want, "memory {memory}");
        // a world restored to a checkpoint asked for the loads of the steps it replayed
        if !memory {
            assert!(driver.asked.len() > 2 * 250, "{}", driver.asked.len());
        }
    }
}

#[test]
fn a_load_that_is_not_there_yet_is_an_error_and_the_world_goes_on_when_it_is() {
    let mut w = world();
    let mut driver = filled(50);
    let failed = w.frame_at(1.0, &mut driver);
    assert!(failed.bodies.is_empty());
    assert!(failed.errors[0].contains("not computed yet"), "{:?}", failed.errors);
    // the step that failed was not taken; filling the log lets the same world continue
    let stopped = w.progress().0;
    assert!(stopped <= 50, "{stopped}");
    for step in 50..200 {
        for body in 0..2 {
            driver.log.put(body as u32, step, &[LoadRecord(load_of(step, body))]).unwrap();
        }
    }
    let frame = w.frame_at(1.0, &mut driver);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert_eq!(frame, world().frame_at(1.0, &mut filled(200)));
}
