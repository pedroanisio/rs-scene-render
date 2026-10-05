//! Horizontal momentum that bodies in the water column give to the water.
use super::{BodySample, Forcing, Lift, Push, Spec, SplashCell, State, CAVITY_SHARE, Q};
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
pub(super) fn samples(spec: &Spec, state: &State, to: &Forcing, from: Option<&Forcing>) -> Vec<BodySample> {
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
    let pressures: Vec<(u32, [f64; 2])> = to
        .lifts
        .iter()
        .map(|lift| {
            (
                lift.owner,
                pressure(spec, &state.q, lift, from.and_then(|f| f.lifts.iter().find(|l| l.owner == lift.owner))),
            )
        })
        .collect();
    for (owner, _) in &pressures {
        footprints.entry(*owner).or_default();
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
                pressure: pressures
                    .iter()
                    .filter(|(o, _)| *o == owner)
                    .fold([0.0; 2], |a, (_, p)| [a[0] + p[0], a[1] + p[1]]),
                ..Default::default()
            };
            let (mut bed, mut depth, mut momentum) = (0.0, 0.0, [0.0; 2]);
            let mut wet = Vec::new();
            for &c in &cells {
                let c = c as usize;
                bed += to.bed[c] + to.raise.get(c).copied().unwrap_or(to.occupancy[c]);
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

/// Applies, over a substep of `dt` in a canonical step of `step`, the share `dt / step` of every
/// push: each column takes its share of the momentum, toward the body's velocity times the column's
/// depth and never past it, and nothing goes to a dry column. Returns the momentum given in all,
/// and adds each body's part to `by`: what was applied is what the body is credited with.
pub(super) fn push(spec: &Spec, q: &mut [Q], pushes: &[Push], by: &mut [[f64; 2]], dt: f64, step: f64) -> [f64; 2] {
    let area = spec.cell_size * spec.cell_size;
    let share = dt / step;
    let mut given = [0.0; 2];
    for push in pushes {
        for &(c, weight) in &push.columns {
            let cell = &mut q[c as usize];
            if cell[0] < spec.dry_tolerance {
                continue;
            }
            for a in 0..2 {
                let wanted = push.momentum[a] * weight / area * share;
                let room = push.target[a] * cell[0] - cell[a + 1];
                if wanted * room > 0.0 {
                    let amount = wanted.abs().min(room.abs()).copysign(wanted);
                    cell[a + 1] += amount;
                    given[a] += amount * area;
                    if let Some(by) = by.get_mut(push.owner as usize) {
                        by[a] += amount * area;
                    }
                }
            }
        }
    }
    given
}

/// Applies the splash of a step: of each cell's water the depth `volume / area` goes, no more than
/// [`CAVITY_SHARE`] of what the cell holds, and is given in equal parts to the neighbours the cell has
/// among the eight around it; the cell's momentum grows by `momentum / area` if it keeps water. The
/// momenta of the cells and of the neighbours are not touched by the water that moves.
pub(super) fn splash(spec: &Spec, q: &mut [Q], cells: &[SplashCell]) {
    let [nx, nz] = spec.cells;
    let area = spec.cell_size * spec.cell_size;
    for e in cells {
        let c = e.cell as usize;
        if q[c][0] < spec.dry_tolerance {
            continue;
        }
        let (x, z) = ((c % nx) as isize, (c / nx) as isize);
        let mut neighbours = [0usize; 8];
        let mut count = 0;
        for dz in -1..=1isize {
            for dx in -1..=1isize {
                let (nxp, nzp) = (x + dx, z + dz);
                if (dx != 0 || dz != 0) && nxp >= 0 && nzp >= 0 && (nxp as usize) < nx && (nzp as usize) < nz {
                    neighbours[count] = nzp as usize * nx + nxp as usize;
                    count += 1;
                }
            }
        }
        if count > 0 {
            let taken = (e.volume / area).min(CAVITY_SHARE * q[c][0]);
            q[c][0] -= taken;
            for &n in &neighbours[..count] {
                q[n][0] += taken / count as f64;
            }
        }
        if q[c][0] >= spec.dry_tolerance {
            q[c][1] += e.momentum[0] / area;
            q[c][2] += e.momentum[1] / area;
        }
    }
}

/// The horizontal momentum per unit density that the slope of what `lift` raises gives the water in one
/// canonical step: `-g h grad(raise) area dt` summed over the columns of the lift and the one around it, `h`
/// the depth in `q` and the gradient the central difference of the raise (zero outside the lift), taken
/// as the mean of the raise at the end of the step and at its start (`before`, when the body had one).
fn pressure(spec: &Spec, q: &[Q], lift: &Lift, before: Option<&Lift>) -> [f64; 2] {
    let [nx, nz] = spec.cells;
    let dx = spec.cell_size;
    let weighted: Vec<(&Lift, f64)> = match before {
        Some(b) => vec![(lift, 0.5), (b, 0.5)],
        None => vec![(lift, 1.0)],
    };
    let (mut low, mut high) = ([usize::MAX; 2], [0usize; 2]);
    for (l, _) in &weighted {
        for &(c, _) in &l.columns {
            let (x, z) = (c as usize % nx, c as usize / nx);
            low = [low[0].min(x), low[1].min(z)];
            high = [high[0].max(x), high[1].max(z)];
        }
    }
    if low[0] == usize::MAX {
        return [0.0; 2];
    }
    let lo = [low[0].saturating_sub(1), low[1].saturating_sub(1)];
    let hi = [(high[0] + 1).min(nx - 1), (high[1] + 1).min(nz - 1)];
    let (width, height) = (hi[0] - lo[0] + 1, hi[1] - lo[1] + 1);
    let mut raise = vec![0.0; width * height];
    for (l, weight) in &weighted {
        for &(c, h) in &l.columns {
            let (x, z) = (c as usize % nx, c as usize / nx);
            raise[(z - lo[1]) * width + (x - lo[0])] += weight * h;
        }
    }
    let at = |x: isize, z: isize| -> f64 {
        let (x, z) = (x - lo[0] as isize, z - lo[1] as isize);
        if x < 0 || z < 0 || x as usize >= width || z as usize >= height {
            0.0
        } else {
            raise[z as usize * width + x as usize]
        }
    };
    let mut force = [0.0; 2];
    for z in lo[1]..=hi[1] {
        for x in lo[0]..=hi[0] {
            let depth = q[z * nx + x][0];
            if depth < spec.dry_tolerance {
                continue;
            }
            let (xi, zi) = (x as isize, z as isize);
            let gradient =
                [(at(xi + 1, zi) - at(xi - 1, zi)) / (2.0 * dx), (at(xi, zi + 1) - at(xi, zi - 1)) / (2.0 * dx)];
            force[0] -= spec.gravity * depth * gradient[0];
            force[1] -= spec.gravity * depth * gradient[1];
        }
    }
    let scale = dx * dx * spec.dt;
    [force[0] * scale, force[1] * scale]
}
