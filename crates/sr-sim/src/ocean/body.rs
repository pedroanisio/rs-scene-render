//! Horizontal momentum that bodies in the water column give to the water.
use super::{BodySample, Forcing, Spec, State, Q};
use std::collections::BTreeMap;

/// Relaxes the horizontal velocity of every column a body occupies toward the
/// body's, and returns the momentum (mass-weighted, per unit density) it gave to
/// the water over the substep.
///
/// A column of depth `h` holding a body of thickness `t` has the fraction
/// `f = t / (h + t)` of its height occupied; `f` is the fraction of the velocity
/// difference that closes over one canonical step, so over a substep of `dt` the
/// fraction is `1 - (1 - f)^(dt / step)` and the result does not depend on how
/// many substeps the CFL bound chose. Dry columns and columns without a body are
/// untouched. With owners (`(tag of every cell, momentum given by each tag)`; empty
/// when nothing is tagged) the momentum a column gets is also added to its owner's.
pub(super) fn transfer(
    spec: &Spec,
    q: &mut [Q],
    occupancy: &[f64],
    velocity: &[[f64; 2]],
    owners: (&[u32], &mut [[f64; 2]]),
    dt: f64,
    step: f64,
) -> [f64; 2] {
    let (owner, by) = owners;
    let area = spec.cell_size * spec.cell_size;
    let mut given = [0.0; 2];
    for (c, ((cell, &thickness), body)) in q.iter_mut().zip(occupancy).zip(velocity).enumerate() {
        if thickness <= 0.0 || cell[0] < spec.dry_tolerance {
            continue;
        }
        let occupied = thickness / (cell[0] + thickness);
        let fraction = 1.0 - (1.0 - occupied).powf(dt / step);
        for a in 0..2 {
            let delta = fraction * (cell[0] * body[a] - cell[a + 1]);
            cell[a + 1] += delta;
            given[a] += delta * area;
            if let Some(by) = owner.get(c).and_then(|&o| by.get_mut(o as usize)) {
                by[a] += delta * area;
            }
        }
    }
    given
}

/// The water around each body at the end of a canonical step: `to` is the sample of the
/// instant the state is at. A body is reported when it holds most of some cell then, or gave
/// the water momentum during the step. Empty without `Spec::body_owners`.
pub(super) fn samples(spec: &Spec, state: &State, to: &Forcing) -> Vec<BodySample> {
    if spec.body_owners == 0 {
        return Vec::new();
    }
    let mut footprints: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (c, (&thickness, &owner)) in to.occupancy.iter().zip(&to.owner).enumerate() {
        if thickness > 0.0 {
            footprints.entry(owner).or_default().push(c as u32);
        }
    }
    for (owner, by) in state.exchange_by.iter().enumerate() {
        if *by != [0.0; 2] {
            footprints.entry(owner as u32).or_default();
        }
    }
    let nx = spec.cells[0];
    let centre = |c: usize| {
        [
            spec.origin[0] + ((c % nx) as f64 + 0.5) * spec.cell_size,
            spec.origin[1] + ((c / nx) as f64 + 0.5) * spec.cell_size,
        ]
    };
    footprints
        .into_iter()
        .map(|(owner, cells)| {
            let mut sample = BodySample {
                owner,
                columns: cells.len(),
                impulse: state.exchange_by.get(owner as usize).copied().unwrap_or([0.0; 2]),
                ..Default::default()
            };
            let (mut bed, mut depth, mut momentum) = (0.0, 0.0, [0.0; 2]);
            let mut wet = Vec::new();
            for &c in &cells {
                let c = c as usize;
                bed += to.bed[c] + to.occupancy[c];
                if state.q[c][0] >= spec.dry_tolerance {
                    wet.push(c);
                    depth += state.q[c][0];
                    momentum[0] += state.q[c][1];
                    momentum[1] += state.q[c][2];
                }
            }
            sample.wet = wet.len();
            if !cells.is_empty() {
                sample.bed = bed / cells.len() as f64;
            }
            if wet.is_empty() {
                sample.surface = [sample.bed, 0.0, 0.0];
                let all = cells.iter().map(|&c| centre(c as usize));
                let n = cells.len().max(1) as f64;
                sample.centroid = all.fold([0.0; 2], |a, p| [a[0] + p[0] / n, a[1] + p[1] / n]);
                return sample;
            }
            sample.velocity = [momentum[0] / depth, momentum[1] / depth];
            let n = wet.len() as f64;
            let top = |c: usize| to.bed[c] - state.q[c][0];
            let mean = wet.iter().fold([0.0; 3], |a, &c| {
                let p = centre(c);
                [a[0] + p[0], a[1] + p[1], a[2] + top(c)]
            });
            let (mx, mz, my) = (mean[0] / n, mean[1] / n, mean[2] / n);
            let (mut sxx, mut sxz, mut szz, mut sxy, mut szy) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for &c in &wet {
                let p = centre(c);
                let (dx, dz, dy) = (p[0] - mx, p[1] - mz, top(c) - my);
                sxx += dx * dx;
                sxz += dx * dz;
                szz += dz * dz;
                sxy += dx * dy;
                szy += dz * dy;
            }
            sample.centroid = [mx, mz];
            sample.surface = [my, 0.0, 0.0];
            // Principal axes of the footprint: a direction it does not extend along (a single
            // row of cells, one cell) leaves no slope there.
            let (half, off) = (0.5 * (sxx + szz), 0.5 * (sxx - szz));
            let radius = (off * off + sxz * sxz).sqrt();
            let (large, small) = (half + radius, half - radius);
            if large <= 0.0 {
                return sample;
            }
            let along = if sxz == 0.0 {
                if sxx >= szz {
                    [1.0, 0.0]
                } else {
                    [0.0, 1.0]
                }
            } else {
                let v = [large - szz, sxz];
                let length = (v[0] * v[0] + v[1] * v[1]).sqrt();
                [v[0] / length, v[1] / length]
            };
            let across = [-along[1], along[0]];
            let gradient = [sxy, szy];
            let mut slope = [0.0; 2];
            for (axis, value) in [(along, large), (across, small)] {
                if value > 1e-9 * large {
                    let g = (axis[0] * gradient[0] + axis[1] * gradient[1]) / value;
                    slope[0] += g * axis[0];
                    slope[1] += g * axis[1];
                }
            }
            sample.surface = [my, slope[0], slope[1]];
            sample
        })
        .collect()
}
