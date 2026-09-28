//! As-rigid-as-possible puppet deformation (Sorkine and Alexa 2007) on a
//! triangulated grid over the node's box. Position pins are soft
//! constraints with a large weight; bend pins also fix the rotation of
//! the cells around them; starch pins stiffen the cells within their
//! radius. The Laplacian is factored once per rest configuration, and each
//! frame runs a fixed number of local/global iterations from the same
//! start, so the result is a pure function of the pins.

use crate::geom::{p, P};

/// Pin kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinKind {
    Position,
    Bend,
    Starch,
}

/// A pin: rest position, displacement, rotation (degrees, bend pins) and amount.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pin {
    pub kind: PinKind,
    pub rest: P,
    pub offset: P,
    pub rotation: f64,
    pub amount: f64,
}

/// A solved puppet: rest grid, deformed grid and point mapping.
#[derive(Debug, Clone)]
pub struct Puppet {
    rect: [f64; 4],
    cols: usize,
    rows: usize,
    rest: Vec<P>,
    pos: Vec<P>,
}

const ITERATIONS: usize = 12;
const PIN_WEIGHT: f64 = 1.0e4;

impl Puppet {
    /// Solves the puppet over `rect` (x, y, w, h) with a `cols`×`rows` cell grid.
    pub fn solve(rect: [f64; 4], cols: usize, rows: usize, pins: &[Pin]) -> Puppet {
        let (cols, rows) = (cols.max(2), rows.max(2));
        let (rest, _, tris) = crate::deform::grid(rect, cols, rows);
        let n = rest.len();
        let cell = [rect[2] / cols as f64, rect[3] / rows as f64];
        // edge weights: uniform, stiffened near starch pins
        let mut nbrs: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
        for t in tris.chunks(3) {
            for k in 0..3 {
                let (a, b) = (t[k] as usize, t[(k + 1) % 3] as usize);
                let mid = rest[a].lerp(rest[b], 0.5);
                let mut w = 1.0;
                for pin in pins.iter().filter(|p| p.kind == PinKind::Starch) {
                    let r = pin.amount.max(1.0) * cell[0].max(cell[1]) * 2.0;
                    let d = mid.dist(pin.rest);
                    if d < r {
                        w *= 1.0 + 20.0 * (1.0 - d / r);
                    }
                }
                if !nbrs[a].iter().any(|&(j, _)| j == b) {
                    nbrs[a].push((b, w));
                    nbrs[b].push((a, w));
                }
            }
        }
        // constraints: barycentric rows at each pin's rest position
        let mut cons: Vec<(Vec<(usize, f64)>, P)> = Vec::new();
        let mut bend: Vec<(usize, f64)> = Vec::new();
        let locate = |q: P| -> Vec<(usize, f64)> {
            let fx = ((q.x - rect[0]) / cell[0]).clamp(0.0, cols as f64 - 1e-9);
            let fy = ((q.y - rect[1]) / cell[1]).clamp(0.0, rows as f64 - 1e-9);
            let (c, r) = (fx.floor() as usize, fy.floor() as usize);
            let (u, v) = (fx - c as f64, fy - r as f64);
            let i = r * (cols + 1) + c;
            let j = i + cols + 1;
            vec![(i, (1.0 - u) * (1.0 - v)), (i + 1, u * (1.0 - v)), (j, (1.0 - u) * v), (j + 1, u * v)]
        };
        for pin in pins.iter().filter(|p| p.kind != PinKind::Starch) {
            cons.push((locate(pin.rest), pin.rest + pin.offset));
            if pin.kind == PinKind::Bend {
                for (i, w) in locate(pin.rest) {
                    if w > 0.0 {
                        bend.push((i, pin.rotation.to_radians()));
                    }
                }
            }
        }
        if cons.is_empty() {
            return Puppet { rect, cols, rows, pos: rest.clone(), rest };
        }
        // system matrix L + W·CᵀC (dense; the grids are small)
        let mut a = vec![0.0f64; n * n];
        for i in 0..n {
            for &(j, w) in &nbrs[i] {
                a[i * n + i] += w;
                a[i * n + j] -= w;
            }
        }
        for (row, _) in &cons {
            for &(i, wi) in row {
                for &(j, wj) in row {
                    a[i * n + j] += PIN_WEIGHT * wi * wj;
                }
            }
        }
        let chol = cholesky(a, n);
        let mut rot = vec![0.0f64; n];
        let mut pos = rest.clone();
        for it in 0..ITERATIONS {
            if it > 0 {
                // local step: best rotation per vertex from its one-ring
                for i in 0..n {
                    let (mut sc, mut ss) = (0.0, 0.0);
                    for &(j, w) in &nbrs[i] {
                        let e0 = rest[i] - rest[j];
                        let e1 = pos[i] - pos[j];
                        sc += w * e0.dot(e1);
                        ss += w * e0.cross(e1);
                    }
                    rot[i] = libm::atan2(ss, sc);
                }
                for &(i, r) in &bend {
                    rot[i] = r;
                }
            }
            // global step
            let mut bx = vec![0.0; n];
            let mut by = vec![0.0; n];
            for i in 0..n {
                for &(j, w) in &nbrs[i] {
                    let e = rest[i] - rest[j];
                    let rm = (rot[i] + rot[j]) * 0.5;
                    let re = if it == 0 { e } else { avg_rot(e, rot[i], rot[j], rm) };
                    bx[i] += w * re.x;
                    by[i] += w * re.y;
                }
            }
            for (row, t) in &cons {
                for &(i, wi) in row {
                    bx[i] += PIN_WEIGHT * wi * t.x;
                    by[i] += PIN_WEIGHT * wi * t.y;
                }
            }
            let (x, y) = (solve(&chol, n, &bx), solve(&chol, n, &by));
            for i in 0..n {
                pos[i] = p(x[i], y[i]);
            }
        }
        Puppet { rect, cols, rows, rest, pos }
    }

    /// Maps a point through the deformed grid (bilinear within its cell).
    pub fn map(&self, q: P) -> P {
        let (cols, rows) = (self.cols, self.rows);
        let fx = (q.x - self.rect[0]) / (self.rect[2] / cols as f64);
        let fy = (q.y - self.rect[1]) / (self.rect[3] / rows as f64);
        let (c, r) = ((fx.floor().max(0.0) as usize).min(cols - 1), (fy.floor().max(0.0) as usize).min(rows - 1));
        let (u, v) = (fx - c as f64, fy - r as f64);
        let i = r * (cols + 1) + c;
        let j = i + cols + 1;
        let d = |k: usize| self.pos[k] - self.rest[k];
        let off =
            d(i) * ((1.0 - u) * (1.0 - v)) + d(i + 1) * (u * (1.0 - v)) + d(j) * ((1.0 - u) * v) + d(j + 1) * (u * v);
        q + off
    }

    /// Deformed grid vertices.
    pub fn vertices(&self) -> &[P] {
        &self.pos
    }
}

fn avg_rot(e: P, ri: f64, rj: f64, _rm: f64) -> P {
    (e.rot(ri) + e.rot(rj)) * 0.5
}

/// Dense Cholesky factor (lower triangle, row-major).
fn cholesky(mut a: Vec<f64>, n: usize) -> Vec<f64> {
    for j in 0..n {
        let mut d = a[j * n + j];
        for k in 0..j {
            d -= a[j * n + k] * a[j * n + k];
        }
        let d = libm::sqrt(d.max(1e-12));
        a[j * n + j] = d;
        for i in j + 1..n {
            let mut s = a[i * n + j];
            for k in 0..j {
                s -= a[i * n + k] * a[j * n + k];
            }
            a[i * n + j] = s / d;
        }
    }
    a
}

fn solve(l: &[f64], n: usize, b: &[f64]) -> Vec<f64> {
    let mut y = b.to_vec();
    for i in 0..n {
        for k in 0..i {
            y[i] -= l[i * n + k] * y[k];
        }
        y[i] /= l[i * n + i];
    }
    for i in (0..n).rev() {
        for k in i + 1..n {
            y[i] -= l[k * n + i] * y[k];
        }
        y[i] /= l[i * n + i];
    }
    y
}
