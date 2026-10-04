use super::{Boundary, Error, Order, Spec, Sponge, Q};

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

fn minmod(a: f64, b: f64) -> f64 {
    if a > 0.0 && b > 0.0 {
        a.min(b)
    } else if a < 0.0 && b < 0.0 {
        a.max(b)
    } else {
        0.0
    }
}

/// Reconstructs conserved states at both faces of every cell of one row.
/// `faces[k] = [left face, right face]`. Surface elevation (depth minus the
/// downward bed ordinate) and velocity are limited, depths are capped so that
/// face depths stay non-negative, and the bed stays constant per cell. Cells
/// without two wet neighbours along the row, and every cell of a non-periodic
/// row end, keep their cell value, so wet/dry fronts and edges are first order.
fn reconstruct(spec: &Spec, bed: &[f64], q: &[Q], index: impl Fn(usize) -> usize, along: usize, faces: &mut [[Q; 2]]) {
    let periodic = spec.boundary == Boundary::Periodic;
    let dry = spec.dry_tolerance;
    for k in 0..along {
        let cell = q[index(k)];
        faces[k] = [cell, cell];
        let (km, kp) = if periodic {
            ((k + along - 1) % along, (k + 1) % along)
        } else if k > 0 && k + 1 < along {
            (k - 1, k + 1)
        } else {
            continue;
        };
        let (m, p) = (q[index(km)], q[index(kp)]);
        if cell[0] < dry || m[0] < dry || p[0] < dry {
            continue;
        }
        let eta = |c: Q, k: usize| c[0] - bed[index(k)];
        let slope = (minmod(eta(cell, k) - eta(m, km), eta(p, kp) - eta(cell, k))).clamp(-2.0 * cell[0], 2.0 * cell[0]);
        let u = |c: Q, a: usize| c[a + 1] / c[0];
        let su: [f64; 2] = std::array::from_fn(|a| minmod(u(cell, a) - u(m, a), u(p, a) - u(cell, a)));
        if slope == 0.0 && su == [0.0; 2] {
            continue;
        }
        let at = |sign: f64| {
            let h = (cell[0] + sign * 0.5 * slope).max(0.0);
            let ua = u(cell, 0) + sign * 0.5 * su[0];
            let ub = u(cell, 1) + sign * 0.5 * su[1];
            [h, h * ua, h * ub]
        };
        faces[k] = [at(-1.0), at(1.0)];
    }
}

/// One conservative forward-Euler update with the face fluxes of `q`.
fn euler(spec: &Spec, bed: &[f64], q: &[Q], dt: f64, decay: f64) -> Result<Vec<Q>, Error> {
    let [nx, nz] = spec.cells;
    let second = spec.order == Order::Second;
    let mut next = q.to_vec();
    let mut faces = vec![[[0.0; 3]; 2]; if second { nx.max(nz) } else { 0 }];
    let factor = dt / spec.cell_size;
    for axis in 0..2 {
        let along = spec.cells[axis];
        let across = spec.cells[1 - axis];
        for row in 0..across {
            let index = |k: usize| if axis == 0 { row * nx + k } else { k * nx + row };
            if second {
                reconstruct(spec, bed, q, index, along, &mut faces);
            }
            for face in 0..=along {
                let li = if face > 0 { Some(index(face - 1)) } else { None };
                let ri = if face < along { Some(index(face)) } else { None };
                let kl = if face > 0 {
                    face - 1
                } else if spec.boundary == Boundary::Periodic {
                    along - 1
                } else {
                    0
                };
                let kr = if face < along {
                    face
                } else if spec.boundary == Boundary::Periodic {
                    0
                } else {
                    along - 1
                };
                let (lref, rref) = (index(kl), index(kr));
                let (mut l, mut r) = if second { (faces[kl][1], faces[kr][0]) } else { (q[lref], q[rref]) };
                if spec.boundary == Boundary::Closed {
                    // The ghost state mirrors the interior state across the wall.
                    if li.is_none() {
                        l = r;
                        l[axis + 1] = -l[axis + 1];
                    }
                    if ri.is_none() {
                        r = l;
                        r[axis + 1] = -r[axis + 1];
                    }
                }
                let (fl, fr) = interface(l, r, bed[lref], bed[rref], axis, spec);
                for a in 0..3 {
                    if let Some(i) = li {
                        next[i][a] -= factor * fl[a];
                    }
                    if let Some(i) = ri {
                        next[i][a] += factor * fr[a];
                    }
                }
            }
        }
    }
    for (old, new) in q.iter().zip(&mut next) {
        if new.iter().any(|v| !v.is_finite()) {
            return Err(Error::Numerical("flux overflow"));
        }
        // Only repair round-off at zero, never an unstable negative solution.
        if new[0] < -64.0 * f64::EPSILON * old[0].max(1.0) {
            return Err(Error::Numerical("negative water depth"));
        }
        new[0] = new[0].max(0.0);
        let dry = new[0] < spec.dry_tolerance;
        for v in &mut new[1..] {
            *v = if dry { 0.0 } else { *v * decay };
        }
    }
    Ok(next)
}

/// Exponential drag on the momenta of wet cells.
fn drag(spec: &Spec, q: &mut [Q], decay: f64) {
    for cell in q {
        if cell[0] >= spec.dry_tolerance {
            cell[1] *= decay;
            cell[2] *= decay;
        }
    }
}

pub(super) fn step(spec: &Spec, bed: &[f64], sponge: Option<&Sponge>, q: &mut Vec<Q>, dt: f64) -> Result<(), Error> {
    debug_assert_eq!(q.len(), spec.cells[0] * spec.cells[1]);
    match spec.order {
        Order::First => {
            *q = euler(spec, bed, q, dt, (-spec.damping * dt).exp())?;
            if let Some(sponge) = sponge {
                sponge.relax(spec, bed, q, dt);
            }
        }
        Order::Second => {
            // Strang splitting of drag and sponge around SSP-RK2 (Heun).
            let half = (-0.5 * spec.damping * dt).exp();
            let mut next = q.clone();
            if let Some(sponge) = sponge {
                sponge.relax(spec, bed, &mut next, 0.5 * dt);
            }
            drag(spec, &mut next, half);
            let stage = euler(spec, bed, &next, dt, 1.0)?;
            let stage = euler(spec, bed, &stage, dt, 1.0)?;
            for ((out, old), new) in q.iter_mut().zip(&next).zip(&stage) {
                *out = std::array::from_fn(|i| 0.5 * (old[i] + new[i]));
                if out[0] < spec.dry_tolerance {
                    out[1] = 0.0;
                    out[2] = 0.0;
                }
            }
            drag(spec, q, half);
            if let Some(sponge) = sponge {
                sponge.relax(spec, bed, q, 0.5 * dt);
            }
        }
    }
    Ok(())
}
