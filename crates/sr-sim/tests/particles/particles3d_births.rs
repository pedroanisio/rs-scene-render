//! Births supplied by the driver: an event with its own instant, position, velocity and
//! mass, next to the rate and burst births of an emitter.
use sr_sim::particles3d::{Birth, Burst, Driver, Emission, Emitter, Error, Hit, Spec};

struct Events {
    births: Vec<Birth>,
    start: f64,
    /// The window of every call, in order.
    calls: Vec<(f64, f64)>,
    /// Gives back a birth outside the window it was asked for.
    misbehave: bool,
}
impl Events {
    fn new(births: Vec<Birth>) -> Self {
        Events { births, start: 0.0, calls: Vec::new(), misbehave: false }
    }
}
impl Driver for Events {
    fn emission(&mut self, _t: f64) -> Result<Emission, Error> {
        Ok(Emission::default())
    }
    fn acceleration(&mut self, _t: f64, _p: [f64; 3], _v: [f64; 3]) -> Result<[f64; 3], Error> {
        Ok([0.0; 3])
    }
    fn sweep(&mut self, _t: f64, _dt: f64, _from: [f64; 3], _to: [f64; 3], _r: f64) -> Result<Option<Hit>, Error> {
        Ok(None)
    }
    /// `lo < time <= hi`, and `time == lo` as well on the first step.
    fn births(&mut self, lo: f64, hi: f64) -> Result<Vec<Birth>, Error> {
        self.calls.push((lo, hi));
        if self.misbehave {
            return Ok(vec![Birth { time: hi + 1.0, position: [0.0; 3], velocity: [0.0; 3], mass: 1.0 }]);
        }
        Ok(self
            .births
            .iter()
            .copied()
            .filter(|b| (b.time > lo && b.time <= hi) || (lo == self.start && b.time == lo))
            .collect())
    }
}
fn birth(time: f64, x: f64, vx: f64, mass: f64) -> Birth {
    Birth { time, position: [x, 1.0, -2.0], velocity: [vx, 0.5, 0.25], mass }
}
fn spec() -> Spec {
    Spec { step: 0.1, lifetime: 10.0, max_particles: 100, ..Default::default() }
}

#[test]
fn a_birth_appears_at_its_exact_instant_with_its_own_state_and_mass() {
    let events = vec![
        birth(0.0, 1.0, 2.0, 5.0),
        birth(0.25, 3.0, -1.0, 7.5),
        birth(0.3, 0.0, 0.0, 0.125),
        birth(0.55, 9.0, 4.0, 2.0),
    ];
    let mut driver = Events::new(events.clone());
    let mut emitter = Emitter::new(spec()).unwrap();
    // Just before each event it is absent; at the next frame it exists, born at its instant.
    for e in &events {
        let before = emitter.at(e.time - 1e-9, &mut driver).unwrap().clone();
        assert!(before.particles.iter().all(|p| p.birth != e.time), "t={}", e.time);
    }
    let frame = emitter.at(0.6, &mut driver).unwrap().clone();
    assert_eq!(frame.particles.len(), 4);
    assert_eq!(frame.emitted, 4);
    for (p, e) in frame.particles.iter().zip(&events) {
        assert_eq!(p.birth, e.time, "born exactly at its instant");
        assert_eq!(p.mass, e.mass);
        // No force acts, so each particle has flown in a straight line since its birth.
        for a in 0..3 {
            let expected = e.position[a] + e.velocity[a] * (0.6 - e.time);
            assert!((p.position[a] - expected).abs() < 1e-12, "axis {a}: {} vs {expected}", p.position[a]);
            assert_eq!(p.velocity[a], e.velocity[a]);
        }
    }
}

#[test]
fn replay_backwards_and_checkpoints_give_identical_frames() {
    let events: Vec<Birth> =
        (0..40).map(|i| birth(0.05 + 0.07 * i as f64, i as f64, 1.0 + i as f64 * 0.1, 1.0 + i as f64)).collect();
    let make = |checkpoint_bytes: usize| {
        Emitter::new(Spec { checkpoint_bytes, step: 0.1, lifetime: 10.0, max_particles: 100, ..Default::default() })
            .unwrap()
    };
    let times = [3.37, 0.2, 2.0, 2.79, 1.04, 2.2, 0.0, 2.95, 1.5];
    let mut reference = make(64 << 20);
    let mut driver = Events::new(events.clone());
    let expected: Vec<_> = times.iter().map(|&t| reference.at(t, &mut driver).unwrap().clone()).collect();
    for budget in [0, 4_000, 64 << 20] {
        let mut emitter = make(budget.max(2_000));
        let mut driver = Events::new(events.clone());
        emitter.at(3.5, &mut driver).unwrap();
        for round in 0..2 {
            for (t, want) in times.iter().zip(&expected) {
                assert_eq!(emitter.at(*t, &mut driver).unwrap(), want, "budget {budget}, round {round}, t={t}");
            }
        }
    }
}

#[test]
fn births_join_rate_and_burst_births_in_time_order_with_consecutive_ids() {
    let mut s = spec();
    s.bursts = vec![Burst { time: 0.2, count: 2, repeat: 0, interval: 1.0 }];
    let mut driver =
        Events::new(vec![birth(0.1, 0.0, 0.0, 1.0), birth(0.2, 0.0, 0.0, 2.0), birth(0.35, 0.0, 0.0, 3.0)]);
    let mut emitter = Emitter::new(s).unwrap();
    let frame = emitter.at(0.5, &mut driver).unwrap().clone();
    assert_eq!(frame.particles.len(), 5);
    assert_eq!(frame.emitted, 5);
    let mut ids: Vec<u64> = frame.particles.iter().map(|p| p.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, (0..5).collect::<Vec<_>>());
    let masses: Vec<f64> = frame.particles.iter().filter(|p| p.mass > 0.0).map(|p| p.mass).collect();
    assert_eq!(masses, vec![1.0, 2.0, 3.0]);
    // Ordinary births carry no mass.
    assert_eq!(frame.particles.iter().filter(|p| p.mass == 0.0).count(), 2);
}

#[test]
fn emitters_without_births_are_untouched() {
    // A driver that never overrides `births` behaves as before: same frames as one that
    // returns an empty list for every window.
    struct Plain;
    impl Driver for Plain {
        fn emission(&mut self, _t: f64) -> Result<Emission, Error> {
            Ok(Emission { rate: 7.0, ..Default::default() })
        }
        fn acceleration(&mut self, _t: f64, _p: [f64; 3], _v: [f64; 3]) -> Result<[f64; 3], Error> {
            Ok([0.0, 1.0, 0.0])
        }
        fn sweep(&mut self, _t: f64, _dt: f64, _f: [f64; 3], _to: [f64; 3], _r: f64) -> Result<Option<Hit>, Error> {
            Ok(None)
        }
    }
    struct Empty(Plain);
    impl Driver for Empty {
        fn emission(&mut self, t: f64) -> Result<Emission, Error> {
            self.0.emission(t)
        }
        fn acceleration(&mut self, t: f64, p: [f64; 3], v: [f64; 3]) -> Result<[f64; 3], Error> {
            self.0.acceleration(t, p, v)
        }
        fn sweep(&mut self, t: f64, dt: f64, f: [f64; 3], to: [f64; 3], r: f64) -> Result<Option<Hit>, Error> {
            self.0.sweep(t, dt, f, to, r)
        }
        fn births(&mut self, _lo: f64, _hi: f64) -> Result<Vec<Birth>, Error> {
            Ok(Vec::new())
        }
    }
    let mut s = spec();
    s.speed = 2.0;
    s.spread = 40.0;
    s.bursts = vec![Burst { time: 0.15, count: 5, repeat: 2, interval: 0.4 }];
    let (mut a, mut b) = (Emitter::new(s.clone()).unwrap(), Emitter::new(s).unwrap());
    for t in [0.3, 1.7, 0.9, 2.5] {
        assert_eq!(a.at(t, &mut Plain).unwrap(), b.at(t, &mut Empty(Plain)).unwrap(), "t={t}");
    }
    assert!(a.frame().particles.iter().all(|p| p.mass == 0.0));
}

#[test]
fn more_births_than_maxparticles_is_an_error_not_a_truncation_and_leaves_the_frame_alone() {
    let mut s = spec();
    s.max_particles = 3;
    let mut driver = Events::new((0..5).map(|i| birth(0.01 + 0.01 * i as f64, 0.0, 0.0, 1.0)).collect());
    let mut emitter = Emitter::new(s).unwrap();
    let before = emitter.at(0.0, &mut driver).unwrap().clone();
    let error = emitter.at(0.5, &mut driver).unwrap_err();
    assert!(matches!(error, Error::Limit(_)), "{error:?}");
    assert_eq!(emitter.frame(), &before);
    // Room that suffices works: three births fit.
    let mut driver = Events::new((0..3).map(|i| birth(0.01 + 0.01 * i as f64, 0.0, 0.0, 1.0)).collect());
    let mut emitter = Emitter::new(spec_with_max(3)).unwrap();
    assert_eq!(emitter.at(0.5, &mut driver).unwrap().particles.len(), 3);
}
fn spec_with_max(max: usize) -> Spec {
    Spec { max_particles: max, ..spec() }
}

#[test]
fn a_birth_outside_the_window_it_was_asked_for_is_a_driver_error() {
    let mut driver = Events::new(vec![]);
    driver.misbehave = true;
    let mut emitter = Emitter::new(spec()).unwrap();
    assert!(matches!(emitter.at(0.5, &mut driver), Err(Error::Driver(_))));
    // Nonfinite state is invalid too.
    let mut driver =
        Events::new(vec![Birth { time: 0.05, position: [f64::NAN, 0.0, 0.0], velocity: [0.0; 3], mass: 1.0 }]);
    assert!(Emitter::new(spec()).unwrap().at(0.5, &mut driver).is_err());
    let mut driver = Events::new(vec![Birth { time: 0.05, position: [0.0; 3], velocity: [0.0; 3], mass: -1.0 }]);
    assert!(Emitter::new(spec()).unwrap().at(0.5, &mut driver).is_err());
}

#[test]
fn the_driver_is_asked_for_each_fixed_step_window_once_going_forward() {
    let mut driver = Events::new(vec![]);
    let mut emitter = Emitter::new(spec()).unwrap();
    emitter.at(0.5, &mut driver).unwrap();
    let windows: Vec<(f64, f64)> =
        driver.calls.iter().copied().filter(|(lo, hi)| (hi - lo - 0.1).abs() < 1e-9).collect();
    assert!(windows.len() >= 5, "{:?}", driver.calls);
    assert!(windows.windows(2).all(|w| (w[1].0 - w[0].1).abs() < 1e-12), "windows must tile time: {windows:?}");
}
