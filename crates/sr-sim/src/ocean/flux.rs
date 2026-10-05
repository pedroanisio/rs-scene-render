use super::{Boundary, Error, Order, Spec, Q};
use rayon::prelude::*;

/// To whom the bed variation of a column belongs: the owner with the largest lift there, and that lift,
/// for the columns bodies lift, sorted by column.
#[derive(Default)]
pub(super) struct Owners {
    columns: Vec<(u32, u32, f64)>,
}
impl Owners {
    pub(super) fn new(mut columns: Vec<(u32, u32, f64)>) -> Self {
        columns.sort_by_key(|c| c.0);
        Owners { columns }
    }
    pub(super) fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }
    fn of(&self, column: usize) -> Option<(u32, f64)> {
        let at = self.columns.binary_search_by_key(&(column as u32), |c| c.0).ok()?;
        Some((self.columns[at].1, self.columns[at].2))
    }
    /// The owner of the bed variation across the face between two columns: that of the one lifted more
    /// (the lowest owner on a tie).
    fn face(&self, a: usize, b: usize) -> Option<u32> {
        match (self.of(a), self.of(b)) {
            (Some(x), Some(y)) => Some(if y.1 > x.1 || (y.1 == x.1 && y.0 < x.0) { y.0 } else { x.0 }),
            (Some(x), None) => Some(x.0),
            (None, Some(y)) => Some(y.0),
            (None, None) => None,
        }
    }
}

/// What a sweep gives the owners of a step: per face with a bed step, the momentum the bed source term of
/// that face gives the water (per unit density, times the cell area).
type Partial = Vec<(u32, [f64; 2])>;

/// Where a step adds the momentum its bed source term gives the water, by owner, and with what weight (the
/// stage of the Runge-Kutta step).
pub(super) struct Sink<'a> {
    pub owners: &'a Owners,
    pub by: &'a mut [[f64; 2]],
}

fn velocity(q: Q, dry: f64) -> [f64; 2] {
    if q[0] < dry {
        [0.0; 2]
    } else {
        [q[1] / q[0], q[2] / q[0]]
    }
}
fn interface(l: Q, r: Q, bed_l: f64, bed_r: f64, axis: usize, spec: &Spec) -> (Q, Q) {
    let ul = velocity(l, spec.dry_tolerance);
    let ur = velocity(r, spec.dry_tolerance);
    // Bed elevations are -bed_y. Difference form avoids subtracting two
    // large absolute water levels and preserves a flat-bed state exactly.
    let hl = (l[0] - (bed_l - bed_r).max(0.0)).max(0.0);
    let hr = (r[0] - (bed_r - bed_l).max(0.0)).max(0.0);
    let ql = [hl, hl * ul[0], hl * ul[1]];
    let qr = [hr, hr * ur[0], hr * ur[1]];
    let f = |q: Q, u: [f64; 2]| {
        let mut out = [q[axis + 1], q[1] * u[axis], q[2] * u[axis]];
        out[axis + 1] += 0.5 * spec.gravity * q[0] * q[0];
        out
    };
    let fl = f(ql, ul);
    let fr = f(qr, ur);
    let a = (ul[axis].abs() + (spec.gravity * hl).sqrt()).max(ur[axis].abs() + (spec.gravity * hr).sqrt());
    let flux: Q = std::array::from_fn(|i| 0.5 * (fl[i] + fr[i]) - 0.5 * a * (qr[i] - ql[i]));
    let mut left = flux;
    let mut right = flux;
    left[axis + 1] += 0.5 * spec.gravity * (l[0] * l[0] - hl * hl);
    right[axis + 1] += 0.5 * spec.gravity * (r[0] * r[0] - hr * hr);
    (left, right)
}

/// Time-step bound on the unsplit sum of directional wave rates. MUSCL needs
/// half of the first-order number to keep reconstructed depths non-negative.
pub(super) fn cfl(spec: &Spec) -> f64 {
    match spec.order {
        Order::First => 0.45,
        Order::Second => 0.225,
    }
}

/// Monotonized-central limiter (van Leer 1977) for the difference across a
/// cell, from the one-sided differences `a` (backward) and `b` (forward):
/// `sign * min(2|a|, 2|b|, |a+b|/2)` when they agree in sign, otherwise zero.
fn limit(a: f64, b: f64) -> f64 {
    if a > 0.0 && b > 0.0 {
        (2.0 * a).min(2.0 * b).min(0.5 * (a + b))
    } else if a < 0.0 && b < 0.0 {
        (2.0 * a).max(2.0 * b).max(0.5 * (a + b))
    } else {
        0.0
    }
}

/// Cells at which sweeps run on the rayon pool; smaller grids stay serial.
/// Every cell's arithmetic is the same sequence of operations on either path,
/// so the result does not depend on the threshold or on the thread count.
pub(super) const PARALLEL_CELLS: usize = 16_384;
/// Rows of cells per task of the z sweep. Each band recomputes the fluxes of
/// its two outer faces, which makes bands independent.
const BAND_ROWS: usize = 16;

/// Positions of the neighbours of cell `k` of a line of `along` cells, if both
/// exist: periodic lines wrap, other lines have none at their ends.
fn line_neighbours(k: usize, along: usize, periodic: bool) -> Option<(usize, usize)> {
    if periodic {
        Some(((k + along - 1) % along, (k + 1) % along))
    } else if k > 0 && k + 1 < along {
        Some((k - 1, k + 1))
    } else {
        None
    }
}

/// Reconstructs the conserved states at both faces of cell `i`, given the
/// indices of its neighbours along the sweep. Surface elevation (depth minus
/// the downward bed ordinate) and velocity are limited with `limit`, depths are
/// capped so that face depths stay non-negative, and the bed stays constant per
/// cell. Cells without two wet neighbours along the sweep, and every cell at a
/// non-periodic end, keep their cell value, so wet/dry fronts and edges are
/// first order.
fn cell_faces(spec: &Spec, bed: &[f64], q: &[Q], i: usize, neighbours: Option<(usize, usize)>) -> [Q; 2] {
    let cell = q[i];
    let Some((im, ip)) = neighbours else { return [cell, cell] };
    let (m, p) = (q[im], q[ip]);
    let dry = spec.dry_tolerance;
    if cell[0] < dry || m[0] < dry || p[0] < dry {
        return [cell, cell];
    }
    let eta = |c: Q, k: usize| c[0] - bed[k];
    let slope = (limit(eta(cell, i) - eta(m, im), eta(p, ip) - eta(cell, i))).clamp(-2.0 * cell[0], 2.0 * cell[0]);
    let u = |c: Q, a: usize| c[a + 1] / c[0];
    let su: [f64; 2] = std::array::from_fn(|a| limit(u(cell, a) - u(m, a), u(p, a) - u(cell, a)));
    if slope == 0.0 && su == [0.0; 2] {
        return [cell, cell];
    }
    let at = |sign: f64| {
        let h = (cell[0] + sign * 0.5 * slope).max(0.0);
        let ua = u(cell, 0) + sign * 0.5 * su[0];
        let ub = u(cell, 1) + sign * 0.5 * su[1];
        [h, h * ua, h * ub]
    };
    [at(-1.0), at(1.0)]
}

/// Fluxes of one face. Closed walls mirror the interior state across the wall.
#[allow(clippy::too_many_arguments)]
fn face_flux(
    spec: &Spec,
    bed: &[f64],
    axis: usize,
    mut l: Q,
    mut r: Q,
    (lref, rref): (usize, usize),
    (first, last): (bool, bool),
) -> (Q, Q) {
    if spec.boundary == Boundary::Closed {
        if first {
            l = r;
            l[axis + 1] = -l[axis + 1];
        }
        if last {
            r = l;
            r[axis + 1] = -r[axis + 1];
        }
    }
    interface(l, r, bed[lref], bed[rref], axis, spec)
}

/// Cells on the two sides of face `face` of a line of `along` cells.
fn face_cells(face: usize, along: usize, periodic: bool) -> (usize, usize) {
    let left = if face > 0 {
        face - 1
    } else if periodic {
        along - 1
    } else {
        0
    };
    let right = if face < along {
        face
    } else if periodic {
        0
    } else {
        along - 1
    };
    (left, right)
}

/// x sweep of one row: adds the flux differences of its faces to `out`, the
/// row's slice of the next state. Faces run in ascending order, so a cell gets
/// its left-face contribution before its right-face one.
#[allow(clippy::too_many_arguments)]
fn sweep_x(
    spec: &Spec,
    bed: &[f64],
    q: &[Q],
    row: usize,
    out: &mut [Q],
    factor: f64,
    faces: &mut Vec<[Q; 2]>,
    credit: Option<(&Owners, f64)>,
    partial: &mut Partial,
) {
    let nx = spec.cells[0];
    let base = row * nx;
    let periodic = spec.boundary == Boundary::Periodic;
    let second = spec.order == Order::Second;
    if second {
        faces.clear();
        faces.extend((0..nx).map(|k| {
            cell_faces(spec, bed, q, base + k, line_neighbours(k, nx, periodic).map(|(a, b)| (base + a, base + b)))
        }));
    }
    for face in 0..=nx {
        let (kl, kr) = face_cells(face, nx, periodic);
        let refs = (base + kl, base + kr);
        let (l, r) = if second { (faces[kl][1], faces[kr][0]) } else { (q[refs.0], q[refs.1]) };
        let (fl, fr) = face_flux(spec, bed, 0, l, r, refs, (face == 0, face == nx));
        if let Some((owners, weight)) = credit {
            // across a face with a bed step the two sides' momentum fluxes differ by the bed source term
            // (the face at the end of a periodic row is the one at its start)
            if (face > 0 || periodic) && face < nx && bed[refs.0] != bed[refs.1] {
                if let Some(owner) = owners.face(refs.0, refs.1) {
                    partial.push((owner, [weight * factor * spec.cell_size * spec.cell_size * (fr[1] - fl[1]), 0.0]));
                }
            }
        }
        for a in 0..3 {
            if face > 0 {
                out[face - 1][a] -= factor * fl[a];
            }
            if face < nx {
                out[face][a] += factor * fr[a];
            }
        }
    }
}

/// z sweep of rows `z0..z1`: `out` holds those rows of the next state, already
/// updated by the x sweep. Faces `z0..=z1` are visited in ascending order; a
/// cell outside the band is left to the band that owns it.
#[allow(clippy::too_many_arguments)]
fn sweep_z(
    spec: &Spec,
    bed: &[f64],
    q: &[Q],
    (z0, z1): (usize, usize),
    out: &mut [Q],
    factor: f64,
    rows: &mut (Vec<[Q; 2]>, Vec<[Q; 2]>),
    credit: Option<(&Owners, f64)>,
    partial: &mut Partial,
) {
    let [nx, nz] = spec.cells;
    let periodic = spec.boundary == Boundary::Periodic;
    let second = spec.order == Order::Second;
    let reconstruct = |z: usize, into: &mut Vec<[Q; 2]>| {
        into.clear();
        into.extend((0..nx).map(|x| {
            cell_faces(
                spec,
                bed,
                q,
                z * nx + x,
                line_neighbours(z, nz, periodic).map(|(a, b)| (a * nx + x, b * nx + x)),
            )
        }));
    };
    let (below, above) = rows;
    let mut below_row = usize::MAX;
    for face in z0..=z1 {
        let (kl, kr) = face_cells(face, nz, periodic);
        if second {
            if below_row != kl {
                reconstruct(kl, below);
            }
            reconstruct(kr, above);
        }
        for x in 0..nx {
            let refs = (kl * nx + x, kr * nx + x);
            let (l, r) = if second { (below[x][1], above[x][0]) } else { (q[refs.0], q[refs.1]) };
            let (fl, fr) = face_flux(spec, bed, 1, l, r, refs, (face == 0, face == nz));
            if let Some((owners, weight)) = credit {
                // a face belongs to the band that starts at it, so that none is counted twice
                if (face > 0 || periodic) && face < z1 && bed[refs.0] != bed[refs.1] {
                    if let Some(owner) = owners.face(refs.0, refs.1) {
                        partial
                            .push((owner, [0.0, weight * factor * spec.cell_size * spec.cell_size * (fr[2] - fl[2])]));
                    }
                }
            }
            for a in 0..3 {
                if face > z0 {
                    out[(face - 1 - z0) * nx + x][a] -= factor * fl[a];
                }
                if face < z1 {
                    out[(face - z0) * nx + x][a] += factor * fr[a];
                }
            }
        }
        if second {
            std::mem::swap(below, above);
            below_row = kr;
        }
    }
}

/// Repairs round-off at zero, zeroes momentum of dry cells, applies `decay`.
fn settle(spec: &Spec, old: &Q, new: &mut Q, decay: f64) -> Option<Error> {
    if new.iter().any(|v| !v.is_finite()) {
        return Some(Error::Numerical("flux overflow"));
    }
    // Only repair round-off at zero, never an unstable negative solution.
    if new[0] < -64.0 * f64::EPSILON * old[0].max(1.0) {
        return Some(Error::Numerical("negative water depth"));
    }
    new[0] = new[0].max(0.0);
    let dry = new[0] < spec.dry_tolerance;
    for v in &mut new[1..] {
        *v = if dry { 0.0 } else { *v * decay };
    }
    None
}

/// One conservative forward-Euler update with the face fluxes of `q`. The
/// reported error is the one of the lowest cell index on every path.
///
/// With a `sink` the momentum that the bed source term of each face with a bed step gives the water is added to
/// its owner, in the order of the rows and then of the bands, whatever the number of threads.
fn euler(
    spec: &Spec,
    bed: &[f64],
    q: &[Q],
    dt: f64,
    decay: f64,
    mut sink: Option<(&mut Sink<'_>, f64)>,
) -> Result<Vec<Q>, Error> {
    let [nx, nz] = spec.cells;
    let parallel = nx * nz >= PARALLEL_CELLS;
    let mut next = q.to_vec();
    let factor = dt / spec.cell_size;
    let credit = sink.as_ref().map(|(s, w)| (s.owners, *w));
    let mut give = |partials: &mut dyn Iterator<Item = Partial>| {
        if let Some((sink, _)) = sink.as_mut() {
            for list in partials {
                for (owner, v) in list {
                    if let Some(by) = sink.by.get_mut(owner as usize) {
                        by[0] += v[0];
                        by[1] += v[1];
                    }
                }
            }
        }
    };
    if parallel {
        if credit.is_some() {
            let rows: Vec<Partial> = next
                .par_chunks_mut(nx)
                .enumerate()
                .map_init(Vec::new, |faces, (row, out)| {
                    let mut partial = Vec::new();
                    sweep_x(spec, bed, q, row, out, factor, faces, credit, &mut partial);
                    partial
                })
                .collect();
            give(&mut rows.into_iter());
            let bands: Vec<Partial> = next
                .par_chunks_mut(BAND_ROWS * nx)
                .enumerate()
                .map_init(Default::default, |rows, (band, out)| {
                    let z0 = band * BAND_ROWS;
                    let mut partial = Vec::new();
                    sweep_z(spec, bed, q, (z0, (z0 + BAND_ROWS).min(nz)), out, factor, rows, credit, &mut partial);
                    partial
                })
                .collect();
            give(&mut bands.into_iter());
        } else {
            next.par_chunks_mut(nx).enumerate().for_each_init(Vec::new, |faces, (row, out)| {
                sweep_x(spec, bed, q, row, out, factor, faces, None, &mut Vec::new());
            });
            next.par_chunks_mut(BAND_ROWS * nx).enumerate().for_each_init(Default::default, |rows, (band, out)| {
                let z0 = band * BAND_ROWS;
                sweep_z(spec, bed, q, (z0, (z0 + BAND_ROWS).min(nz)), out, factor, rows, None, &mut Vec::new());
            });
        }
        if let Some(e) =
            q.par_iter().zip(next.par_iter_mut()).find_map_first(|(old, new)| settle(spec, old, new, decay))
        {
            return Err(e);
        }
    } else {
        let mut faces = Vec::new();
        let mut partial = Vec::new();
        for (row, out) in next.chunks_mut(nx).enumerate() {
            sweep_x(spec, bed, q, row, out, factor, &mut faces, credit, &mut partial);
            give(&mut std::iter::once(std::mem::take(&mut partial)));
        }
        let mut rows = Default::default();
        for (band, out) in next.chunks_mut(BAND_ROWS * nx).enumerate() {
            let z0 = band * BAND_ROWS;
            sweep_z(spec, bed, q, (z0, (z0 + BAND_ROWS).min(nz)), out, factor, &mut rows, credit, &mut partial);
            give(&mut std::iter::once(std::mem::take(&mut partial)));
        }
        if let Some(e) = q.iter().zip(next.iter_mut()).find_map(|(old, new)| settle(spec, old, new, decay)) {
            return Err(e);
        }
    }
    Ok(next)
}

/// Exponential drag on the momenta of wet cells.
fn drag(spec: &Spec, q: &mut [Q], decay: f64) {
    let apply = |cell: &mut Q| {
        if cell[0] >= spec.dry_tolerance {
            cell[1] *= decay;
            cell[2] *= decay;
        }
    };
    if q.len() >= PARALLEL_CELLS {
        q.par_iter_mut().for_each(apply);
    } else {
        q.iter_mut().for_each(apply);
    }
}

///
/// With a `sink` the momentum that the bed source term gives the water over the step is added to the owners'.
pub(super) fn step(
    spec: &Spec,
    bed: &[f64],
    q: &mut Vec<Q>,
    dt: f64,
    mut sink: Option<&mut Sink<'_>>,
) -> Result<(), Error> {
    debug_assert_eq!(q.len(), spec.cells[0] * spec.cells[1]);
    match spec.order {
        Order::First => {
            *q = euler(spec, bed, q, dt, (-spec.damping * dt).exp(), sink.map(|s| (s, 1.0)))?;
        }
        Order::Second => {
            // Strang splitting of drag around SSP-RK2 (Heun).
            let half = (-0.5 * spec.damping * dt).exp();
            let mut next = q.clone();
            drag(spec, &mut next, half);
            // the step is the mean of the state and the second stage: each stage's change counts for half
            let stage = {
                let first = euler(spec, bed, &next, dt, 1.0, sink.as_deref_mut().map(|s| (s, 0.5)))?;
                euler(spec, bed, &first, dt, 1.0, sink.map(|s| (s, 0.5)))?
            };
            let combine = |(out, (old, new)): (&mut Q, (&Q, &Q))| {
                *out = std::array::from_fn(|i| 0.5 * (old[i] + new[i]));
                if out[0] < spec.dry_tolerance {
                    out[1] = 0.0;
                    out[2] = 0.0;
                }
            };
            if q.len() >= PARALLEL_CELLS {
                q.par_iter_mut().zip(next.par_iter().zip(stage.par_iter())).for_each(combine);
            } else {
                q.iter_mut().zip(next.iter().zip(stage.iter())).for_each(combine);
            }
            drag(spec, q, half);
        }
    }
    Ok(())
}
