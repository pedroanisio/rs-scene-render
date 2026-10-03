//! Frozen-velocity midpoint backtracing for motion-aware cache interpolation.
//!
//! Velocity samples are asset-world vectors in scene units per second. The grid
//! transform locates samples; it does not rotate or scale their vector values.
//! Positive elapsed time traces into the past; negative time traces forward.
//! This follows semi-Lagrangian characteristic tracing with midpoint RK2:
//! <https://www.cs.ubc.ca/~rbridson/fluidsimulation/fluids_notes.pdf>.

use crate::{Error, SparseGrid};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Advection {
    grids: [Arc<SparseGrid>; 3],
    elapsed: f64,
    displacement: [f64; 3],
}

impl Advection {
    /// Components must share a sample transform. They may use a different
    /// transform and resolution from the density and temperature fields.
    pub fn new(grids: [Arc<SparseGrid>; 3], elapsed: f64) -> Result<Self, Error> {
        if !elapsed.is_finite() {
            return Err(Error::Invalid("advection time must be finite"));
        }
        if grids.iter().any(|g| g.transform() != grids[0].transform()) {
            return Err(Error::Invalid("velocity components require matching transforms"));
        }
        let displacement = std::array::from_fn(|i| {
            let peak = grids[i]
                .bricks()
                .flat_map(|(_, values)| values.iter().copied())
                .chain(std::iter::once(grids[i].background()))
                .map(|v| f64::from(v).abs())
                .fold(0., f64::max);
            peak * elapsed.abs()
        });
        if displacement.iter().any(|v| !v.is_finite()) {
            return Err(Error::Invalid("velocity displacement exceeds finite coordinates"));
        }
        Ok(Self { grids, elapsed, displacement })
    }

    pub fn grids(&self) -> &[Arc<SparseGrid>; 3] {
        &self.grids
    }

    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }

    /// Conservative componentwise travel bound, including background velocity.
    pub fn displacement(&self) -> [f64; 3] {
        self.displacement
    }

    /// Trace a finite asset-world point using a frozen endpoint velocity field.
    /// Medium construction validates its domain and trace support against overflow.
    pub fn backtrace(&self, point: [f64; 3]) -> [f64; 3] {
        if self.elapsed == 0. {
            return point;
        }
        let mid =
            std::array::from_fn(|i| point[i] - (self.elapsed * 0.5) * f64::from(self.grids[i].sample_world(point)));
        std::array::from_fn(|i| point[i] - self.elapsed * f64::from(self.grids[i].sample_world(mid)))
    }

    pub(crate) fn validate_domain(&self, lo: [f64; 3], hi: [f64; 3]) -> Result<(), Error> {
        for k in 0..8 {
            let p = std::array::from_fn(|i| {
                if k & (1 << i) == 0 {
                    lo[i] - self.displacement[i]
                } else {
                    hi[i] + self.displacement[i]
                }
            });
            if p.iter().any(|v| !v.is_finite())
                || self.grids[0].transform().world_to_index(p).iter().any(|v| !v.is_finite())
            {
                return Err(Error::Invalid("advected domain exceeds finite coordinates"));
            }
        }
        Ok(())
    }
}
