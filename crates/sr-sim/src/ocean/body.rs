//! Horizontal momentum that bodies in the water column give to the water.
use super::{Spec, Q};

/// Relaxes the horizontal velocity of every column a body occupies toward the
/// body's, and returns the momentum (mass-weighted, per unit density) it gave to
/// the water over the substep.
///
/// A column of depth `h` holding a body of thickness `t` has the fraction
/// `f = t / (h + t)` of its height occupied; `f` is the fraction of the velocity
/// difference that closes over one canonical step, so over a substep of `dt` the
/// fraction is `1 - (1 - f)^(dt / step)` and the result does not depend on how
/// many substeps the CFL bound chose. Dry columns and columns without a body are
/// untouched.
pub(super) fn transfer(
    spec: &Spec,
    q: &mut [Q],
    occupancy: &[f64],
    velocity: &[[f64; 2]],
    dt: f64,
    step: f64,
) -> [f64; 2] {
    let area = spec.cell_size * spec.cell_size;
    let mut given = [0.0; 2];
    for ((cell, &thickness), body) in q.iter_mut().zip(occupancy).zip(velocity) {
        if thickness <= 0.0 || cell[0] < spec.dry_tolerance {
            continue;
        }
        let occupied = thickness / (cell[0] + thickness);
        let fraction = 1.0 - (1.0 - occupied).powf(dt / step);
        for a in 0..2 {
            let delta = fraction * (cell[0] * body[a] - cell[a + 1]);
            cell[a + 1] += delta;
            given[a] += delta * area;
        }
    }
    given
}
