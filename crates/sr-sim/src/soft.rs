//! Soft bodies: spring-mass lattices (jelly, cloth) and chains (rope),
//! integrated with symplectic Euler in substeps sized to the stiffest spring.
//! Positions are metres with y up, like the rigid world.

/// Where a point hit something: the corrected position and the outward normal.
pub type Hit = Option<([f64; 2], [f64; 2])>;

/// Soft body kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoftKind {
    Jelly,
    Cloth,
    Rope,
}

/// A soft body to simulate.
#[derive(Clone, Debug)]
pub struct SoftSpec {
    pub kind: SoftKind,
    pub rows: usize,
    pub cols: usize,
    /// Rest lattice in metres (y up), row-major.
    pub rest: Vec<[f64; 2]>,
    /// Total mass (kg).
    pub mass: f64,
    /// Spring stiffness (N/m).
    pub stiffness: f64,
    /// Fraction of critical damping along each spring.
    pub damping: f64,
    /// Area restoring pressure of jelly (N/m).
    pub pressure: f64,
    /// Pinned lattice points.
    pub pinned: Vec<bool>,
    pub self_collision: bool,
}

/// Substeps one fixed step needs for stable integration.
pub fn substeps(spec_mass: f64, stiffness: f64, nodes: usize, step: f64) -> u64 {
    let m = (spec_mass / nodes.max(1) as f64).max(1e-9);
    (step * (8.0 * stiffness / m).sqrt()).ceil().max(1.0) as u64
}

/// Simulation state of one soft body.
#[derive(Clone, Debug)]
pub struct SoftState {
    pub pos: Vec<[f64; 2]>,
    pub vel: Vec<[f64; 2]>,
    springs: Vec<(usize, usize, f64)>,
    boundary: Vec<usize>,
    area0: f64,
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn area(pos: &[[f64; 2]], ring: &[usize]) -> f64 {
    let mut s = 0.0;
    for k in 0..ring.len() {
        let (a, b) = (pos[ring[k]], pos[ring[(k + 1) % ring.len()]]);
        s += a[0] * b[1] - b[0] * a[1];
    }
    s * 0.5
}

impl SoftState {
    pub fn new(spec: &SoftSpec) -> SoftState {
        let (r, c) = (spec.rows, spec.cols);
        let idx = |i: usize, j: usize| i * c + j;
        let mut springs = Vec::new();
        let add = |a: usize, b: usize, springs: &mut Vec<(usize, usize, f64)>| {
            springs.push((a, b, dist(spec.rest[a], spec.rest[b])))
        };
        let pos: Vec<[f64; 2]> = if spec.kind == SoftKind::Rope {
            // the chain runs along the centre row
            let mid: Vec<[f64; 2]> = (0..c)
                .map(|j| {
                    let (a, b) = (spec.rest[idx(0, j)], spec.rest[idx(r - 1, j)]);
                    [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
                })
                .collect();
            for j in 0..c.saturating_sub(1) {
                springs.push((j, j + 1, dist(mid[j], mid[j + 1])));
            }
            mid
        } else {
            for i in 0..r {
                for j in 0..c {
                    if j + 1 < c {
                        add(idx(i, j), idx(i, j + 1), &mut springs);
                    }
                    if i + 1 < r {
                        add(idx(i, j), idx(i + 1, j), &mut springs);
                    }
                    if i + 1 < r && j + 1 < c {
                        add(idx(i, j), idx(i + 1, j + 1), &mut springs);
                        add(idx(i, j + 1), idx(i + 1, j), &mut springs);
                    }
                    if spec.kind == SoftKind::Cloth {
                        if j + 2 < c {
                            add(idx(i, j), idx(i, j + 2), &mut springs);
                        }
                        if i + 2 < r {
                            add(idx(i, j), idx(i + 2, j), &mut springs);
                        }
                    }
                }
            }
            spec.rest.clone()
        };
        let mut boundary = Vec::new();
        if spec.kind != SoftKind::Rope {
            boundary.extend((0..c).map(|j| idx(0, j)));
            boundary.extend((1..r).map(|i| idx(i, c - 1)));
            boundary.extend((0..c - 1).rev().map(|j| idx(r - 1, j)));
            boundary.extend((1..r - 1).rev().map(|i| idx(i, 0)));
        }
        let area0 = area(&pos, &boundary);
        SoftState { vel: vec![[0.0; 2]; pos.len()], pos, springs, boundary, area0 }
    }

    /// Advances one fixed step of `dt` seconds. `accel(p, v)` is the external
    /// acceleration (gravity and fields, m/s²); `collide(p)` returns a corrected
    /// position and outward normal when `p` is inside something.
    // positions, velocities and forces advance together, index by index
    #[allow(clippy::needless_range_loop)]
    pub fn step(
        &mut self,
        spec: &SoftSpec,
        dt: f64,
        accel: &dyn Fn([f64; 2], [f64; 2]) -> [f64; 2],
        collide: &dyn Fn([f64; 2]) -> Hit,
    ) {
        let n = self.pos.len();
        let m = (spec.mass / n.max(1) as f64).max(1e-9);
        let subs = substeps(spec.mass, spec.stiffness, n, dt).min(4096);
        let h = dt / subs as f64;
        let c_crit = 2.0 * (spec.stiffness * m).sqrt() * spec.damping;
        let pinned = |k: usize| -> bool {
            if spec.kind == SoftKind::Rope {
                // a pinned column pins its chain point
                (0..spec.rows).any(|i| spec.pinned.get(i * spec.cols + k).copied().unwrap_or(false))
            } else {
                spec.pinned.get(k).copied().unwrap_or(false)
            }
        };
        let min_rest = self.springs.iter().map(|s| s.2).fold(f64::MAX, f64::min).max(1e-6);
        let mut force = vec![[0.0f64; 2]; n];
        for _ in 0..subs {
            for (k, f) in force.iter_mut().enumerate() {
                let a = accel(self.pos[k], self.vel[k]);
                *f = [a[0] * m, a[1] * m];
            }
            for &(a, b, rest) in &self.springs {
                let d = [self.pos[b][0] - self.pos[a][0], self.pos[b][1] - self.pos[a][1]];
                let len = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-9);
                let u = [d[0] / len, d[1] / len];
                let rv = (self.vel[b][0] - self.vel[a][0]) * u[0] + (self.vel[b][1] - self.vel[a][1]) * u[1];
                let mag = spec.stiffness * (len - rest) + c_crit * rv;
                force[a][0] += u[0] * mag;
                force[a][1] += u[1] * mag;
                force[b][0] -= u[0] * mag;
                force[b][1] -= u[1] * mag;
            }
            if spec.kind == SoftKind::Jelly && spec.pressure != 0.0 && self.boundary.len() >= 3 {
                let a = area(&self.pos, &self.boundary).abs().max(1e-9);
                let p = spec.pressure * (self.area0.abs() / a - 1.0);
                let ring = &self.boundary;
                let sign = if self.area0 >= 0.0 { 1.0 } else { -1.0 };
                for k in 0..ring.len() {
                    let (i, j) = (ring[k], ring[(k + 1) % ring.len()]);
                    let e = [self.pos[j][0] - self.pos[i][0], self.pos[j][1] - self.pos[i][1]];
                    // outward normal of a counter-clockwise ring (y up): (e.y, −e.x)
                    let nrm = [e[1] * sign, -e[0] * sign];
                    for q in [i, j] {
                        force[q][0] += nrm[0] * p * 0.5;
                        force[q][1] += nrm[1] * p * 0.5;
                    }
                }
            }
            if spec.self_collision {
                let r = min_rest * 0.5;
                for i in 0..n {
                    for j in i + 1..n {
                        let d = [self.pos[j][0] - self.pos[i][0], self.pos[j][1] - self.pos[i][1]];
                        let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
                        if len < r && len > 1e-12 {
                            let push = spec.stiffness * (r - len) / len;
                            force[i][0] -= d[0] * push;
                            force[i][1] -= d[1] * push;
                            force[j][0] += d[0] * push;
                            force[j][1] += d[1] * push;
                        }
                    }
                }
            }
            for k in 0..n {
                if pinned(k) {
                    self.vel[k] = [0.0, 0.0];
                    continue;
                }
                self.vel[k][0] += force[k][0] / m * h;
                self.vel[k][1] += force[k][1] / m * h;
                let mut p = [self.pos[k][0] + self.vel[k][0] * h, self.pos[k][1] + self.vel[k][1] * h];
                if let Some((q, nrm)) = collide(p) {
                    p = q;
                    let vn = self.vel[k][0] * nrm[0] + self.vel[k][1] * nrm[1];
                    if vn < 0.0 {
                        self.vel[k][0] -= vn * nrm[0];
                        self.vel[k][1] -= vn * nrm[1];
                        self.vel[k][0] *= 0.9;
                        self.vel[k][1] *= 0.9;
                    }
                }
                self.pos[k] = p;
            }
        }
    }

    /// The lattice (rows × cols, metres) of the current state; a rope's
    /// rows follow its chain, offset across it as at rest.
    pub fn lattice(&self, spec: &SoftSpec) -> Vec<[f64; 2]> {
        if spec.kind != SoftKind::Rope {
            return self.pos.clone();
        }
        let (r, c) = (spec.rows, spec.cols);
        let mut out = vec![[0.0; 2]; r * c];
        for j in 0..c {
            let (a, b) = (self.pos[j.saturating_sub(1)], self.pos[(j + 1).min(c - 1)]);
            let t = [b[0] - a[0], b[1] - a[1]];
            let tl = (t[0] * t[0] + t[1] * t[1]).sqrt().max(1e-12);
            let nrm = [-t[1] / tl, t[0] / tl];
            // rest offset of each row from the centre line, measured across the rest chain
            let (ra, rb) = (spec.rest[j.saturating_sub(1)], spec.rest[(j + 1).min(c - 1)]);
            let rt = [rb[0] - ra[0], rb[1] - ra[1]];
            let rtl = (rt[0] * rt[0] + rt[1] * rt[1]).sqrt().max(1e-12);
            let rn = [-rt[1] / rtl, rt[0] / rtl];
            let mid = [
                (spec.rest[j][0] + spec.rest[(r - 1) * c + j][0]) * 0.5,
                (spec.rest[j][1] + spec.rest[(r - 1) * c + j][1]) * 0.5,
            ];
            for i in 0..r {
                let q = spec.rest[i * c + j];
                let off = (q[0] - mid[0]) * rn[0] + (q[1] - mid[1]) * rn[1];
                out[i * c + j] = [self.pos[j][0] + nrm[0] * off, self.pos[j][1] + nrm[1] * off];
            }
        }
        out
    }
}
