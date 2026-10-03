use super::{Boundary, Error, Spec, Q};

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

pub(super) fn step(spec: &Spec, bed: &[f64], q: &mut Vec<Q>, dt: f64) -> Result<(), Error> {
    let [nx, nz] = spec.cells;
    let mut next = q.clone();
    let factor = dt / spec.cell_size;
    for axis in 0..2 {
        let along = spec.cells[axis];
        let across = spec.cells[1 - axis];
        for row in 0..across {
            let index = |k: usize| if axis == 0 { row * nx + k } else { k * nx + row };
            for face in 0..=along {
                let li = if face > 0 { Some(index(face - 1)) } else { None };
                let ri = if face < along { Some(index(face)) } else { None };
                let lref =
                    li.unwrap_or_else(|| if spec.boundary == Boundary::Periodic { index(along - 1) } else { index(0) });
                let rref =
                    ri.unwrap_or_else(|| if spec.boundary == Boundary::Periodic { index(0) } else { index(along - 1) });
                let mut l = q[lref];
                let mut r = q[rref];
                if spec.boundary == Boundary::Closed {
                    if li.is_none() {
                        l[axis + 1] = -l[axis + 1];
                    }
                    if ri.is_none() {
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
    let decay = (-spec.damping * dt).exp();
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
    debug_assert_eq!(q.len(), nx * nz);
    *q = next;
    Ok(())
}
