//! A request a few steps back (the samples of a shutter, a scrub) restarts from a recent step, not
//! from the checkpoint up to a second earlier, and gets the same frame; the steps kept stay within
//! the byte budget together with the checkpoints.
use sr_sim::particles3d::{Driver, Emission, Emitter, Error, Hit, Spec};

/// A rate emitter that counts how many fixed-step windows it was asked for.
#[derive(Default)]
struct Counting {
    windows: u64,
}
impl Driver for Counting {
    fn emission(&mut self, _t: f64) -> Result<Emission, Error> {
        Ok(Emission { rate: 400.0, ..Default::default() })
    }
    fn acceleration(&mut self, _t: f64, _p: [f64; 3], _v: [f64; 3]) -> Result<[f64; 3], Error> {
        Ok([0.0, 9.8, 0.0])
    }
    fn sweep(&mut self, _t: f64, _dt: f64, _a: [f64; 3], _b: [f64; 3], _r: f64) -> Result<Option<Hit>, Error> {
        Ok(None)
    }
    fn births(&mut self, _lo: f64, _hi: f64) -> Result<Vec<sr_sim::particles3d::Birth>, Error> {
        self.windows += 1;
        Ok(Vec::new())
    }
}

fn emitter(checkpoint_bytes: usize) -> Emitter {
    Emitter::new(Spec {
        step: 0.01,
        lifetime: 20.0,
        speed: 3.0,
        speed_variance: 1.0,
        spread: 90.0,
        max_particles: 4000,
        checkpoint_bytes,
        seed: 5,
        ..Default::default()
    })
    .unwrap()
}

const ROOMY: usize = 256 << 20;
/// The shutter's order: the first sample, the last, then each in turn, around fixed steps.
const SHUTTER: [f64; 6] = [3.052, 3.078, 3.052, 3.058, 3.064, 3.071];

#[test]
fn going_back_a_few_steps_costs_no_replay_and_gives_the_same_frames() {
    let mut recent = emitter(ROOMY);
    let mut plain = emitter(1 << 10); // too small to keep anything but the first state
    let (mut a, mut b) = (Counting::default(), Counting::default());
    let mut cost_recent = Vec::new();
    for frame in 0..6 {
        let base = 1.0 + frame as f64 * 0.5;
        recent.at(base, &mut a).unwrap();
        plain.at(base, &mut b).unwrap();
    }
    for t in SHUTTER {
        let (before_a, before_b) = (a.windows, b.windows);
        let fa = recent.at(t, &mut a).unwrap().clone();
        let fb = plain.at(t, &mut b).unwrap().clone();
        assert_eq!(fa, fb, "the frame at {t} does not depend on where the replay started");
        cost_recent.push((a.windows - before_a, b.windows - before_b));
    }
    println!("windows asked per shutter sample (recent steps, none): {cost_recent:?}");
    // every sample costs the one window of its own partial step, whichever way it goes
    assert!(cost_recent.iter().all(|(kept, _)| *kept <= 1), "{cost_recent:?}");
    // while without the recent steps a sample that goes back replays from the last checkpoint
    let (kept, none): (u64, u64) = cost_recent.iter().fold((0, 0), |a, c| (a.0 + c.0, a.1 + c.1));
    assert!(none > 50 * kept, "{none} windows without the recent steps against {kept}");
}

#[test]
fn scrubbing_in_any_order_agrees_with_a_fresh_emitter() {
    let times = [2.5, 2.43, 2.431, 2.512, 0.2, 2.5, 2.499, 2.0, 3.3, 3.29, 3.31, 1.234, 3.3];
    let mut scrubbed = emitter(ROOMY);
    let mut d = Counting::default();
    for t in times {
        let got = scrubbed.at(t, &mut d).unwrap().clone();
        let mut fresh = emitter(1 << 10);
        let want = fresh.at(t, &mut Counting::default()).unwrap().clone();
        assert_eq!(got, want, "t = {t}");
    }
}

#[test]
fn the_steps_kept_stay_within_the_budget_together_with_the_checkpoints() {
    // each state of up to 4000 particles; budgets from none to roomy
    for budget in [1 << 10, 64 << 10, 400 << 10, 1 << 20, 8 << 20, ROOMY] {
        let mut e = emitter(budget);
        let mut d = Counting::default();
        for t in [0.5, 1.7, 3.05, 3.2, 3.07, 3.5, 0.9, 6.1, 6.05, 6.12, 6.0] {
            e.at(t, &mut d).unwrap();
            assert!(e.checkpoint_bytes() <= budget.max(300), "budget {budget} at {t}: {} bytes", e.checkpoint_bytes());
        }
    }
}
