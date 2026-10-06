//! Particles that fall into a water plane are taken out of the frame where they cross it, and the
//! driver is told which, every canonical step, the same on every replay.

use sr_sim::particles3d::*;

const STEP: f64 = 0.1;

/// Records what `absorbed` was told: step and list, and fails if a step is told twice with another list.
#[derive(Default)]
struct Log {
    told: Vec<(u64, Vec<Absorbed>)>,
    disagreed: Vec<u64>,
}

impl Driver for Log {
    fn emission(&mut self, _: f64) -> Result<Emission, Error> {
        Ok(Emission::default())
    }
    fn acceleration(&mut self, _: f64, _: [f64; 3], _: [f64; 3]) -> Result<[f64; 3], Error> {
        Ok([0.0; 3])
    }
    fn sweep(&mut self, _: f64, _: f64, _: [f64; 3], _: [f64; 3], _: f64) -> Result<Option<Hit>, Error> {
        Ok(None)
    }
    fn births(&mut self, lo: f64, hi: f64) -> Result<Vec<Birth>, Error> {
        // ten particles of 40 kg, born at 0.05 s above the water at x = 0..9, falling and drifting along x
        Ok((0..10)
            .filter(|_| lo < 0.05 && 0.05 <= hi)
            .map(|i| Birth { time: 0.05, position: [i as f64, -5.0, 0.0], velocity: [2.0, 0.0, 0.0], mass: 40.0 })
            .collect())
    }
    fn absorbed(&mut self, step: u64, list: &[Absorbed]) -> Result<(), Error> {
        match self.told.iter().find(|(s, _)| *s == step) {
            Some((_, first)) if first.as_slice() != list => self.disagreed.push(step),
            Some(_) => {}
            None => self.told.push((step, list.to_vec())),
        }
        Ok(())
    }
}

/// A water plane at y = 0 (scene y points down: the water is where y is greater), 6 units wide in x and 4 in z.
fn spec() -> Spec {
    Spec {
        step: STEP,
        lifetime: 20.0,
        gravity: [0.0, 9.80665, 0.0],
        max_particles: 100,
        water: Some(Water {
            origin: [0.0; 3],
            normal: [0.0, -1.0, 0.0],
            axes: [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            extent: [[-1.0, 5.5], [-2.0, 2.0]],
        }),
        ..Spec::default()
    }
}

fn alive(e: &Emitter) -> Vec<u64> {
    e.frame().particles.iter().map(|p| p.id).collect()
}

#[test]
fn a_particle_that_crosses_the_plane_inside_the_rectangle_is_taken_out_and_told_to_the_driver() {
    let mut emitter = Emitter::new(spec()).unwrap();
    let mut log = Log::default();
    emitter.at(0.05 + 0.2, &mut log).unwrap();
    assert_eq!(alive(&emitter).len(), 10, "all still falling at 0.25 s: {:?}", alive(&emitter));
    emitter.at(3.0, &mut log).unwrap();
    // falling 5 units under 9.80665 takes sqrt(2 * 5 / 9.80665) = 1.0097 s from birth at 0.05: 1.0597 s
    let all: Vec<&Absorbed> = log.told.iter().flat_map(|(_, l)| l).collect();
    // x runs 0..9 plus the drift of 2 units a second for 1.0097 s (2.02): inside [-1, 5.5] for x0 + 2.02 <= 5.5, x0 <= 3
    assert_eq!(all.len(), 4, "x0 = 0, 1, 2, 3 land in the rectangle: {all:?}");
    for a in &all {
        assert!((a.time - (0.05 + (10.0f64 / 9.80665).sqrt())).abs() < 1e-3, "{}", a.time);
        assert!(a.position[1].abs() < 2e-3, "on the plane: {:?}", a.position);
        assert_eq!(a.mass, 40.0);
        assert!((a.velocity[1] - 9.80665 * (a.time - 0.05)).abs() < 1e-3 && (a.velocity[0] - 2.0).abs() < 1e-9);
    }
    // the rest fell past the rectangle's edge and go on, below the plane
    assert_eq!(alive(&emitter).len(), 6);
    assert!(emitter.frame().particles.iter().all(|p| p.position[1] > 0.0 && p.position[0] > 5.5 - 1e-9));
    // every canonical step was told, the empty ones too, in order, each once
    let steps: Vec<u64> = log.told.iter().map(|(s, _)| *s).collect();
    assert_eq!(steps, (0..steps.len() as u64).collect::<Vec<_>>());
    assert!(log.told.iter().filter(|(_, l)| !l.is_empty()).count() >= 1 && log.told[0].1.is_empty());
}

#[test]
fn a_particle_born_in_the_water_is_not_taken_and_without_a_plane_nothing_is() {
    struct Below(Log);
    impl Driver for Below {
        fn emission(&mut self, t: f64) -> Result<Emission, Error> {
            self.0.emission(t)
        }
        fn acceleration(&mut self, t: f64, p: [f64; 3], v: [f64; 3]) -> Result<[f64; 3], Error> {
            self.0.acceleration(t, p, v)
        }
        fn sweep(&mut self, a: f64, b: f64, c: [f64; 3], d: [f64; 3], e: f64) -> Result<Option<Hit>, Error> {
            self.0.sweep(a, b, c, d, e)
        }
        fn births(&mut self, lo: f64, hi: f64) -> Result<Vec<Birth>, Error> {
            Ok(self.0.births(lo, hi)?.into_iter().map(|b| Birth { position: [b.position[0], 3.0, 0.0], ..b }).collect())
        }
        fn absorbed(&mut self, step: u64, list: &[Absorbed]) -> Result<(), Error> {
            self.0.absorbed(step, list)
        }
    }
    let mut under = Emitter::new(spec()).unwrap();
    let mut driver = Below(Log::default());
    under.at(2.0, &mut driver).unwrap();
    assert!(driver.0.told.iter().all(|(_, l)| l.is_empty()), "already in the water: nothing to cross");
    assert_eq!(alive(&under).len(), 10);
    let mut dry = Emitter::new(Spec { water: None, ..spec() }).unwrap();
    let mut log = Log::default();
    dry.at(3.0, &mut log).unwrap();
    assert_eq!(alive(&dry).len(), 10, "with no water plane every particle falls on");
    assert!(log.told.iter().all(|(_, l)| l.is_empty()));
}

#[test]
fn the_same_lists_on_any_replay_and_the_survivors_do_not_depend_on_the_plane_they_never_met() {
    let mut first = Emitter::new(spec()).unwrap();
    let mut log = Log::default();
    first.at(3.0, &mut log).unwrap();
    // going back and forward, with no checkpoint but the first, asks the driver again for the same steps
    let mut small = Emitter::new(Spec { checkpoint_bytes: 600, ..spec() }).unwrap();
    for t in [3.0, 0.7, 2.2, 1.1, 3.0, 0.2, 2.9] {
        small.at(t, &mut log).unwrap();
    }
    assert!(log.disagreed.is_empty(), "a replayed step told a different list: {:?}", log.disagreed);
    assert_eq!(first.frame().particles, small.at(3.0, &mut log).unwrap().particles);
    // the particles that never reach the plane are those of a world with none
    let mut no_water = Emitter::new(Spec { water: None, ..spec() }).unwrap();
    let before = no_water.at(1.0, &mut Log::default()).unwrap().particles.clone();
    let mut with = Emitter::new(spec()).unwrap();
    assert_eq!(with.at(1.0, &mut Log::default()).unwrap().particles, before, "nothing has crossed by 1 s");
}

#[test]
fn a_particle_that_crosses_the_plane_and_dies_in_the_same_step_is_told_too() {
    // the crossing is at 1.0597 s and the step that holds it is (1.0, 1.1]: a lifetime of 1.03 from the birth at 0.05
    // ends the particle at 1.08, after the crossing and inside that step
    let mut emitter = Emitter::new(Spec { lifetime: 1.03, ..spec() }).unwrap();
    let mut log = Log::default();
    emitter.at(3.0, &mut log).unwrap();
    let all: Vec<&Absorbed> = log.told.iter().flat_map(|(_, l)| l).collect();
    assert_eq!(all.len(), 4, "x0 = 0, 1, 2, 3 reach the water before they die: {all:?}");
    assert!(alive(&emitter).is_empty());
    // one that dies before the water is not told
    let mut early = Emitter::new(Spec { lifetime: 0.9, ..spec() }).unwrap();
    let mut log = Log::default();
    early.at(3.0, &mut log).unwrap();
    assert!(log.told.iter().all(|(_, l)| l.is_empty()), "{:?}", log.told);
}
