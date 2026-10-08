//! The ejecta that come to rest on the ground of a crater with an angle of repose (`crater@repose`).
//!
//! An emitter of a crater's ejecta takes out each particle that settles on the ground and tells, once per fixed step
//! of its own, which did: where, in the plane of the crater, and how much volume of the target each stood for. The
//! volumes are poured, step by step, onto a [`sr_sim::granular::Bed`] over the final crater, which relaxes at the
//! angle of repose after each step. The bed after the steps that a frame's particles have computed is the deposit
//! that lies on the ground in that frame. It is a function of the log of what settled, so it is the same however the
//! particles were stepped; the bed is kept after each step it has folded and at checkpoints, so a frame a few steps
//! on from the last is a few steps of work and a frame back in time is a replay from a checkpoint.
//!
//! The particles themselves do not see the deposit: they land on the ground without it, and so does the rigid world.

use std::collections::BTreeMap;
use std::sync::Arc;

use sr_sim::exchange::Record;
use sr_sim::granular::Bed;

use crate::crater::ImpactCrater;

/// Cells to the crest radius of the crater, and how many crest radii the bed reaches from the crater's centre on each
/// side: a pile of ten cells' radius is where the angle of repose is within two degrees.
const CELLS_PER_RADIUS: f64 = 12.0;
const REACH: f64 = 6.0;
/// The transfer under which a pass of the relaxation is the last, in cells, and the most passes after a step.
const TOLERANCE: f64 = 1e-6;
const MAX_PASSES: u32 = 200_000;
/// A bed is kept every this many steps, and no more than `MAX_CHECKPOINTS` of them (when there are more the
/// interval doubles and the odd ones go).
const CHECKPOINT_EVERY: u64 = 8;
const MAX_CHECKPOINTS: usize = 128;

/// One particle that came to rest: where, in the plane of the crater from its centre (the crater's own axes), and the
/// volume of the target it stood for, in the owner's object units cubed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Settled {
    pub(crate) id: u64,
    pub(crate) plane: [f64; 2],
    pub(crate) volume: f64,
}

impl Record for Settled {
    fn same(&self, other: &Self) -> bool {
        self.id == other.id
            && self.volume.to_bits() == other.volume.to_bits()
            && self.plane.iter().zip(&other.plane).all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

/// What an emitter that deposits on a crater is told: the owner of the crater that its bursts name, and the angle.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Config {
    pub(crate) crater: Arc<str>,
    pub(crate) repose: f64,
}

/// What a bed was built for: the crater's impact and the angle. A bed made for another is not used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key([u64; 5]);

impl Key {
    fn of(impact: &ImpactCrater, repose: f64) -> Self {
        let s = impact.spec;
        Key([
            impact.impact_time().to_bits(),
            repose.to_bits(),
            s.radius.to_bits(),
            s.depth.to_bits(),
            s.rim_height.to_bits() ^ s.rim_width.to_bits().rotate_left(7),
        ])
    }
}

/// The beds of one emitter's deposit: the latest and the checkpoints.
#[derive(Default)]
pub(crate) struct State {
    key: Option<Key>,
    /// Steps folded into `latest`.
    steps: u64,
    latest: Option<Bed>,
    checkpoints: BTreeMap<u64, Bed>,
    every: u64,
}

/// The grid of a crater: its corner in the plane's coordinates, cells to a side and their size.
fn grid(impact: &ImpactCrater) -> ([f64; 2], usize, f64) {
    let crest = impact.spec.radius;
    let cell = crest / CELLS_PER_RADIUS;
    let n = (2.0 * REACH * CELLS_PER_RADIUS) as usize;
    ([-REACH * crest; 2], n, cell)
}

impl State {
    /// The deposit after `steps` fixed steps of the particles: `settled(step)` gives what settled in each, or none for
    /// a step the particles have not computed, which is an error.
    pub(crate) fn deposit(
        &mut self,
        impact: &ImpactCrater,
        repose: f64,
        steps: u64,
        settled: &dyn Fn(u64) -> Option<Vec<Settled>>,
    ) -> Result<sr_3d::crater::Deposit, String> {
        let key = Key::of(impact, repose);
        if self.key != Some(key) {
            *self = State { key: Some(key), every: CHECKPOINT_EVERY, ..State::default() };
        }
        let (origin, n, cell) = grid(impact);
        // from the latest bed if it is not past the request, else from the last checkpoint before it, else from the ground
        let from = if self.latest.is_some() && self.steps <= steps {
            self.steps
        } else {
            self.checkpoints.range(..=steps).next_back().map_or(0, |(s, _)| *s)
        };
        let mut bed = match (from, &self.latest) {
            (0, _) => None,
            (f, Some(latest)) if f == self.steps => Some(latest.clone()),
            (f, _) => self.checkpoints.get(&f).cloned(),
        };
        let mut bed = match bed.take() {
            Some(bed) => bed,
            None => {
                // the ground the deposit lies on: what the crater has made of it, as it is when grown
                let kernel = crate::crater::ground_kernel(impact)?;
                let (u, v) = kernel.plane_basis();
                let centre = impact.spec.center;
                let mut ground = Vec::with_capacity(n * n);
                for iz in 0..n {
                    for ix in 0..n {
                        let (a, b) = (origin[0] + (ix as f64 + 0.5) * cell, origin[1] + (iz as f64 + 0.5) * cell);
                        let point: [f64; 3] = std::array::from_fn(|k| centre[k] + a * u[k] + b * v[k]);
                        let moved = kernel.map(point, 1.0)?.position;
                        let axis = impact.spec.outward;
                        let length = axis.iter().map(|c| c * c).sum::<f64>().sqrt();
                        let rise: f64 = (0..3).map(|k| (moved[k] - point[k]) * axis[k] / length).sum();
                        ground.push(rise);
                    }
                }
                Bed::on(ground, [n, n], cell, repose)?
            }
        };
        let extent = n as f64 * cell;
        for step in from..steps {
            let list = settled(step).ok_or_else(|| {
                format!(
                    "the deposit at step {steps} needs fixed step {step} of the particles, which has not been computed"
                )
            })?;
            for s in list {
                let at = [(s.plane[0] - origin[0]).clamp(0.0, extent), (s.plane[1] - origin[1]).clamp(0.0, extent)];
                bed.deposit(at, s.volume)?;
            }
            bed.relax(TOLERANCE * cell, MAX_PASSES)?;
            if (step + 1) % self.every == 0 {
                self.checkpoints.insert(step + 1, bed.clone());
                if self.checkpoints.len() > MAX_CHECKPOINTS {
                    self.every *= 2;
                    let every = self.every;
                    self.checkpoints.retain(|s, _| s % every == 0);
                }
            }
        }
        let deposit = sr_3d::crater::Deposit::new(origin, cell, [n, n], bed.heights().to_vec())?;
        self.steps = steps;
        self.latest = Some(bed);
        Ok(deposit)
    }
}
