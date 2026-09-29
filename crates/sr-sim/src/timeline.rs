//! Fixed-step timelines with checkpoints, shared by the grid and agent simulations.
//!
//! A simulation state is stepped at a fixed rate from its start; a copy is kept every simulated
//! second so any time is reached by replaying at most that second. Large states would make one
//! copy per second expensive, so when the copies exceed a memory budget every other one is
//! dropped (and new ones are kept half as often), which doubles the worst replay each time.

use std::collections::BTreeMap;

/// Memory kept in checkpoints per simulation, in bytes.
pub const BUDGET: usize = 256 << 20;

/// A state that can be checkpointed.
pub trait State: Clone {
    /// Approximate size of one copy, in bytes.
    fn bytes(&self) -> usize;
}

/// The state of a simulation at fixed steps from `start`.
pub struct Timeline<S: State> {
    pub start: f64,
    pub dt: f64,
    state: S,
    step: u64,
    checkpoints: BTreeMap<u64, S>,
    /// Steps between checkpoints (one second, doubled when over budget).
    every: u64,
}

impl<S: State> Timeline<S> {
    pub fn new(start: f64, dt: f64, initial: S) -> Timeline<S> {
        let every = ((1.0 / dt.max(1e-6)).round() as u64).max(1);
        let mut checkpoints = BTreeMap::new();
        checkpoints.insert(0, initial.clone());
        Timeline { start, dt, state: initial, step: 0, checkpoints, every }
    }

    /// The state at time `t`: every step whose start lies before `t` has run. `step` advances
    /// the state by one step, given the step index and its start time.
    pub fn at(&mut self, t: f64, step: &mut dyn FnMut(&mut S, u64, f64)) -> &S {
        let target = if t <= self.start { 0 } else { ((t - self.start) / self.dt + 1e-9).floor() as u64 };
        if self.step > target || target - self.step > self.every {
            if let Some((&k, cp)) = self.checkpoints.range(..=target).next_back() {
                if k > self.step || self.step > target {
                    self.state = cp.clone();
                    self.step = k;
                }
            }
        }
        while self.step < target {
            let t0 = self.start + self.step as f64 * self.dt;
            step(&mut self.state, self.step, t0);
            self.step += 1;
            if self.step % self.every == 0 && !self.checkpoints.contains_key(&self.step) {
                self.checkpoints.insert(self.step, self.state.clone());
                self.thin();
            }
        }
        &self.state
    }

    /// The state of the last requested time.
    pub fn state(&self) -> &S {
        &self.state
    }

    fn thin(&mut self) {
        let size = self.state.bytes().max(1);
        while self.checkpoints.len() > 2 && self.checkpoints.len() * size > BUDGET {
            self.every *= 2;
            let every = self.every;
            self.checkpoints.retain(|k, _| k % every == 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct Big(Vec<u64>);
    impl State for Big {
        fn bytes(&self) -> usize {
            BUDGET / 4
        }
    }

    #[test]
    fn seeking_replays_the_same_steps_and_thinning_keeps_the_budget() {
        let run = |order: &[f64]| {
            let mut tl = Timeline::new(0.5, 0.25, Big(Vec::new()));
            let mut out = Vec::new();
            for &t in order {
                out.push(tl.at(t, &mut |s: &mut Big, k, t0| s.0.push(k * 1000 + (t0 * 100.0) as u64)).0.clone());
            }
            (out, tl.checkpoints.len())
        };
        let (fwd, kept) = run(&[0.0, 1.0, 3.0, 9.0]);
        let (back, _) = run(&[9.0, 3.0, 1.0, 0.0]);
        assert_eq!(fwd[3], back[0]);
        assert_eq!(fwd[1], back[2]);
        assert_eq!(fwd[1], vec![50, 1075]);
        assert!(kept * BUDGET / 4 <= BUDGET || kept <= 2, "{kept} checkpoints");
    }
}
