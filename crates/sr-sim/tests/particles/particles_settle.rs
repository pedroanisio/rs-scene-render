//! Particles that come to rest on the ground are taken out where they settle, told to the driver as ground
//! and not as water, and without a rest speed nothing is taken: the same bits as before.

use sr_sim::particles3d::*;

const STEP: f64 = 0.1;

/// Ground at y = 0 (scene y points down: the ground is where y is greater), ten 40 kg particles dropped from
/// y = -5, and a record of what `absorbed` was told.
#[derive(Default)]
struct Ground {
    told: Vec<(u64, Vec<Absorbed>)>,
    disagreed: Vec<u64>,
}

impl Driver for Ground {
    fn emission(&mut self, _: f64) -> Result<Emission, Error> {
        Ok(Emission::default())
    }
    fn acceleration(&mut self, _: f64, _: [f64; 3], _: [f64; 3]) -> Result<[f64; 3], Error> {
        Ok([0.0; 3])
    }
    fn sweep(&mut self, _: f64, _: f64, from: [f64; 3], to: [f64; 3], _: f64) -> Result<Option<Hit>, Error> {
        if from[1] < 0.0 && to[1] >= 0.0 {
            let fraction = -from[1] / (to[1] - from[1]);
            let position = std::array::from_fn(|i| from[i] + fraction * (to[i] - from[i]));
            return Ok(Some(Hit { fraction, position, normal: [0.0, -1.0, 0.0], velocity: [0.0; 3] }));
        }
        Ok(None)
    }
    fn births(&mut self, lo: f64, hi: f64) -> Result<Vec<Birth>, Error> {
        Ok((0..10)
            .filter(|_| lo < 0.05 && 0.05 <= hi)
            .map(|i| Birth { time: 0.05, position: [i as f64, -5.0, 0.0], velocity: [0.0; 3], mass: 40.0 })
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

fn spec(settle: Option<f64>) -> Spec {
    Spec {
        step: STEP,
        lifetime: 20.0,
        gravity: [0.0, 9.80665, 0.0],
        max_particles: 100,
        restitution: 0.5,
        settle,
        ..Spec::default()
    }
}

#[test]
fn a_particle_that_hits_the_ground_slower_than_the_rest_speed_is_taken_out_and_told_as_ground() {
    // the fall of 5 units takes sqrt(10 / g) = 1.0097 s and hits at 9.903 m/s, and the rebound is half of it: 4.95 m/s
    let mut emitter = Emitter::new(spec(Some(6.0))).unwrap();
    let mut driver = Ground::default();
    emitter.at(3.0, &mut driver).unwrap();
    let all: Vec<&Absorbed> = driver.told.iter().flat_map(|(_, l)| l).collect();
    assert_eq!(all.len(), 10, "{all:?}");
    assert!(emitter.frame().particles.is_empty(), "all ten came to rest at their first impact");
    for a in &all {
        assert!(a.ground, "{a:?}");
        assert!((a.time - (0.05 + (10.0f64 / 9.80665).sqrt())).abs() < 1e-3, "{}", a.time);
        assert!(a.position[1].abs() < 2e-3, "on the ground: {:?}", a.position);
        assert_eq!(a.mass, 40.0);
        // what it carried when it settled: the rebound, up
        assert!((a.velocity[1] + 4.95).abs() < 0.01, "{:?}", a.velocity);
    }
    let ids: Vec<u64> = all.iter().map(|a| a.id).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "in order of their ids");
}

#[test]
fn a_particle_that_rebounds_faster_than_the_rest_speed_goes_on_and_settles_when_it_is_slow() {
    // rebounds of 4.95, 2.48, 1.24, 0.62 m/s: with a rest speed of 1 m/s the fourth impact settles it
    let mut emitter = Emitter::new(spec(Some(1.0))).unwrap();
    let mut driver = Ground::default();
    emitter.at(1.2, &mut driver).unwrap();
    assert_eq!(emitter.frame().particles.len(), 10, "still bouncing at 1.2 s");
    emitter.at(8.0, &mut driver).unwrap();
    assert!(emitter.frame().particles.is_empty());
    let all: Vec<&Absorbed> = driver.told.iter().flat_map(|(_, l)| l).collect();
    assert_eq!(all.len(), 10);
    assert!(all.iter().all(|a| a.ground && a.velocity[1].abs() < 1.0 && a.time > 1.5), "{all:?}");
}

#[test]
fn without_a_rest_speed_nothing_is_taken_and_a_replay_tells_the_same() {
    let mut off = Emitter::new(spec(None)).unwrap();
    let mut driver = Ground::default();
    off.at(3.0, &mut driver).unwrap();
    assert_eq!(off.frame().particles.len(), 10);
    assert!(driver.told.iter().all(|(_, l)| l.is_empty()));
    // seeking back and forward replays steps and tells them again, the same
    let mut small = Emitter::new(Spec { checkpoint_bytes: 600, ..spec(Some(6.0)) }).unwrap();
    let mut log = Ground::default();
    for t in [3.0, 0.7, 2.2, 1.1, 3.0, 0.2, 2.9] {
        small.at(t, &mut log).unwrap();
    }
    assert!(log.disagreed.is_empty(), "{:?}", log.disagreed);
    assert!(small.frame().particles.is_empty());
}
