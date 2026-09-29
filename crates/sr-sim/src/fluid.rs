//! Incompressible 2D fluid (Stam 1999, "Stable Fluids"; vorticity confinement after Fedkiw, Stam
//! and Jensen 2001, "Visual Simulation of Smoke") on a staggered MAC grid (Harlow and Welch 1965,
//! as in Bridson, "Fluid Simulation for Computer Graphics"): horizontal velocity on the vertical
//! cell faces, vertical velocity on the horizontal faces, pressure and dye at cell centres, so
//! divergence, pressure gradient and Laplacian are consistent and the projection can drive the
//! divergence to zero.
//!
//! Each step: sources add dye and velocity, forces (buoyancy, fields) act, vorticity confinement
//! restores swirl lost to numerical diffusion, viscosity diffuses velocity implicitly, the
//! velocity is made divergence-free (successive over-relaxation on the pressure Poisson
//! equation), advected semi-Lagrangianly and made divergence-free again, and the premultiplied
//! RGBA dye is advected with it, diffused and dissipated. Velocities are in box pixels per second.

use crate::timeline::State;

/// What happens at the grid's edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// Walls: no flow through them.
    Closed,
    /// Fluid and dye leave freely (zero pressure outside).
    Open,
    /// Periodic.
    Wrap,
}

/// A dye and velocity source, in box pixels (colour: linear, premultiplied RGBA).
#[derive(Clone, Debug)]
pub struct Source {
    pub pos: [f64; 2],
    pub radius: f64,
    pub color: [f32; 4],
    pub density: f64,
    pub velocity: [f64; 2],
    pub start: f64,
    pub end: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct FluidSpec {
    /// Box size in pixels.
    pub size: [f64; 2],
    /// Cells along the longer side.
    pub resolution: usize,
    pub viscosity: f64,
    pub diffusion: f64,
    pub dissipation: f64,
    pub velocity_dissipation: f64,
    pub vorticity: f64,
    /// Upward acceleration per unit of dye alpha, px/s².
    pub buoyancy: f64,
    pub iterations: usize,
    pub edge: Edge,
    pub sources: Vec<Source>,
}

impl FluidSpec {
    /// Grid size in cells and the cell size in pixels.
    pub fn grid(&self) -> (usize, usize, f64) {
        let h = self.size[0].max(self.size[1]) / self.resolution.max(2) as f64;
        (((self.size[0] / h).round() as usize).max(2), ((self.size[1] / h).round() as usize).max(2), h)
    }
}

/// The fluid's fields. `u` is (nx + 1) × ny (vertical faces), `v` is nx × (ny + 1)
/// (horizontal faces), both in cells per second; `dye` is nx × ny.
#[derive(Clone, Debug, PartialEq)]
pub struct Fluid {
    pub nx: usize,
    pub ny: usize,
    /// Cell size in pixels.
    pub h: f64,
    pub u: Vec<f32>,
    pub v: Vec<f32>,
    /// Premultiplied linear RGBA dye.
    pub dye: Vec<[f32; 4]>,
}

impl State for Fluid {
    fn bytes(&self) -> usize {
        self.dye.len() * 24
    }
}

pub fn init(s: &FluidSpec) -> Fluid {
    let (nx, ny, h) = s.grid();
    Fluid { nx, ny, h, u: vec![0.0; (nx + 1) * ny], v: vec![0.0; nx * (ny + 1)], dye: vec![[0.0; 4]; nx * ny] }
}

impl Fluid {
    fn ui(&self, i: usize, j: usize) -> usize {
        j * (self.nx + 1) + i
    }
    fn vi(&self, i: usize, j: usize) -> usize {
        j * self.nx + i
    }

    /// Velocity (cells per second) at a point in cell coordinates (cell (i, j) spans
    /// [i, i + 1] × [j, j + 1]).
    pub fn velocity(&self, x: f64, y: f64, edge: Edge) -> [f64; 2] {
        [
            bilinear(&self.u, self.nx + 1, self.ny, x, y - 0.5, edge, true),
            bilinear(&self.v, self.nx, self.ny + 1, x - 0.5, y, edge, false),
        ]
    }

    /// Divergence of cell (i, j), in 1/s.
    pub fn divergence_at(&self, i: usize, j: usize) -> f64 {
        (self.u[self.ui(i + 1, j)] - self.u[self.ui(i, j)] + self.v[self.vi(i, j + 1)] - self.v[self.vi(i, j)]) as f64
    }
}

/// Bilinear sample of a `w` × `h` grid at (x, y) in its own index space. `staggered_x` marks a
/// grid whose last column duplicates the first under wrapping (u faces).
fn bilinear(f: &[f32], w: usize, h: usize, x: f64, y: f64, edge: Edge, staggered_x: bool) -> f64 {
    let (x, y) = match edge {
        Edge::Wrap => {
            let pw = if staggered_x { w - 1 } else { w } as f64;
            let ph = if staggered_x { h } else { h - 1 } as f64;
            (x.rem_euclid(pw), y.rem_euclid(ph))
        }
        _ => (x.clamp(0.0, (w - 1) as f64), y.clamp(0.0, (h - 1) as f64)),
    };
    let (i0, j0) = ((x.floor() as usize).min(w - 1), (y.floor() as usize).min(h - 1));
    let (i1, j1) = ((i0 + 1).min(w - 1), (j0 + 1).min(h - 1));
    let (fx, fy) = (x - i0 as f64, y - j0 as f64);
    let a = f[j0 * w + i0] as f64 * (1.0 - fx) + f[j0 * w + i1] as f64 * fx;
    let b = f[j1 * w + i0] as f64 * (1.0 - fx) + f[j1 * w + i1] as f64 * fx;
    a * (1.0 - fy) + b * fy
}

/// Boundary faces: closed walls carry no flow; wrapped grids share their first and last faces.
fn edges(f: &mut Fluid, edge: Edge) {
    let (nx, ny) = (f.nx, f.ny);
    match edge {
        Edge::Closed => {
            for j in 0..ny {
                let (a, b) = (f.ui(0, j), f.ui(nx, j));
                f.u[a] = 0.0;
                f.u[b] = 0.0;
            }
            for i in 0..nx {
                let (a, b) = (f.vi(i, 0), f.vi(i, ny));
                f.v[a] = 0.0;
                f.v[b] = 0.0;
            }
        }
        Edge::Wrap => {
            for j in 0..ny {
                let (a, b) = (f.ui(0, j), f.ui(nx, j));
                f.u[b] = f.u[a];
            }
            for i in 0..nx {
                let (a, b) = (f.vi(i, 0), f.vi(i, ny));
                f.v[b] = f.v[a];
            }
        }
        Edge::Open => {}
    }
}

/// Makes the velocity divergence-free: solves ∇²p = ∇·u by successive over-relaxation (a fixed
/// red-black order, so the result is deterministic) and subtracts ∇p.
fn project(f: &mut Fluid, edge: Edge, iters: usize) {
    let (nx, ny) = (f.nx, f.ny);
    let div: Vec<f64> = (0..nx * ny).map(|k| f.divergence_at(k % nx, k / nx)).collect();
    let mut p = vec![0.0f64; nx * ny];
    let omega = 1.9;
    for _ in 0..iters {
        for parity in 0..2 {
            for j in 0..ny {
                for i in 0..nx {
                    if (i + j) % 2 != parity {
                        continue;
                    }
                    let (mut sum, mut count) = (0.0, 0.0);
                    for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                        let (a, b) = (i as i64 + di, j as i64 + dj);
                        let inside = a >= 0 && b >= 0 && a < nx as i64 && b < ny as i64;
                        match (inside, edge) {
                            (true, _) => {
                                sum += p[b as usize * nx + a as usize];
                                count += 1.0;
                            }
                            (false, Edge::Wrap) => {
                                sum += p[b.rem_euclid(ny as i64) as usize * nx + a.rem_euclid(nx as i64) as usize];
                                count += 1.0;
                            }
                            // open: p = 0 outside, still a neighbour
                            (false, Edge::Open) => count += 1.0,
                            // closed: no flux through the wall, so no neighbour
                            (false, Edge::Closed) => {}
                        }
                    }
                    if count > 0.0 {
                        let k = j * nx + i;
                        let gs = (sum - div[k]) / count;
                        p[k] += omega * (gs - p[k]);
                    }
                }
            }
        }
    }
    let at = |i: i64, j: i64| -> f64 {
        match edge {
            Edge::Wrap => p[j.rem_euclid(ny as i64) as usize * nx + i.rem_euclid(nx as i64) as usize],
            _ if i < 0 || j < 0 || i >= nx as i64 || j >= ny as i64 => 0.0,
            _ => p[j as usize * nx + i as usize],
        }
    };
    for j in 0..ny {
        for i in 0..=nx {
            if edge == Edge::Closed && (i == 0 || i == nx) {
                continue;
            }
            let k = f.ui(i, j);
            f.u[k] -= (at(i as i64, j as i64) - at(i as i64 - 1, j as i64)) as f32;
        }
    }
    for j in 0..=ny {
        for i in 0..nx {
            if edge == Edge::Closed && (j == 0 || j == ny) {
                continue;
            }
            let k = f.vi(i, j);
            f.v[k] -= (at(i as i64, j as i64) - at(i as i64, j as i64 - 1)) as f32;
        }
    }
    edges(f, edge);
}

/// Mean |divergence| over the cells, in 1/s.
pub fn divergence(f: &Fluid) -> f64 {
    (0..f.nx * f.ny).map(|k| f.divergence_at(k % f.nx, k / f.nx).abs()).sum::<f64>() / (f.nx * f.ny) as f64
}

/// Implicit diffusion of a `w` × `h` grid by Jacobi iterations: (1 − a∇²) x = x0.
fn diffuse(x: &mut [f32], w: usize, h: usize, a: f32, iters: usize) {
    if a <= 0.0 {
        return;
    }
    let x0 = x.to_vec();
    let mut next = x.to_vec();
    for _ in 0..iters {
        for j in 0..h {
            for i in 0..w {
                let g = |a: i64, b: i64| x[(b.clamp(0, h as i64 - 1) as usize) * w + a.clamp(0, w as i64 - 1) as usize];
                let (ii, jj) = (i as i64, j as i64);
                let n = g(ii - 1, jj) + g(ii + 1, jj) + g(ii, jj - 1) + g(ii, jj + 1);
                next[j * w + i] = (x0[j * w + i] + a * n) / (1.0 + 4.0 * a);
            }
        }
        x.copy_from_slice(&next);
    }
}

/// Advances the fluid from `t` by `dt`; `accel` gives an extra acceleration (px/s²) at a box position.
pub fn step(s: &FluidSpec, f: &mut Fluid, t: f64, dt: f64, accel: &mut dyn FnMut([f64; 2]) -> [f64; 2]) {
    let (nx, ny, h) = (f.nx, f.ny, f.h);
    let edge = s.edge;
    // ---- sources (velocities to cells per second)
    for src in &s.sources {
        if t < src.start || src.end.map(|e| t >= e).unwrap_or(false) {
            continue;
        }
        let r = src.radius / h;
        let (cx, cy) = (src.pos[0] / h, src.pos[1] / h);
        let weight = |x: f64, y: f64| {
            let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            if d < r {
                (1.0 - d / r.max(1e-6)) as f32
            } else {
                0.0
            }
        };
        for j in 0..ny {
            for i in 0..nx {
                let w = weight(i as f64 + 0.5, j as f64 + 0.5);
                if w > 0.0 {
                    let amt = (src.density * dt) as f32 * w;
                    for c in 0..4 {
                        f.dye[j * nx + i][c] += src.color[c] * amt;
                    }
                }
            }
        }
        if src.velocity != [0.0, 0.0] {
            let (su, sv) = ((src.velocity[0] / h) as f32, (src.velocity[1] / h) as f32);
            for j in 0..ny {
                for i in 0..=nx {
                    let w = weight(i as f64, j as f64 + 0.5);
                    let k = f.ui(i, j);
                    f.u[k] += (su - f.u[k]) * w;
                }
            }
            for j in 0..=ny {
                for i in 0..nx {
                    let w = weight(i as f64 + 0.5, j as f64);
                    let k = f.vi(i, j);
                    f.v[k] += (sv - f.v[k]) * w;
                }
            }
        }
    }
    // ---- forces: fields, and buoyancy lifting dye (−y)
    let mut ax = vec![0.0f64; nx * ny];
    let mut ay = vec![0.0f64; nx * ny];
    for j in 0..ny {
        for i in 0..nx {
            let a = accel([(i as f64 + 0.5) * h, (j as f64 + 0.5) * h]);
            ax[j * nx + i] = a[0] / h;
            ay[j * nx + i] = a[1] / h - s.buoyancy / h * f.dye[j * nx + i][3].min(1.0) as f64;
        }
    }
    // ---- vorticity confinement (cell centres)
    if s.vorticity > 0.0 {
        let c = |i: usize, j: usize| f.velocity(i as f64 + 0.5, j as f64 + 0.5, edge);
        let mut w = vec![0.0f64; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                let (l, r) = (c(i.saturating_sub(1), j), c((i + 1).min(nx - 1), j));
                let (d, u) = (c(i, j.saturating_sub(1)), c(i, (j + 1).min(ny - 1)));
                w[j * nx + i] = 0.5 * (r[1] - l[1]) - 0.5 * (u[0] - d[0]);
            }
        }
        let wa =
            |i: i64, j: i64| w[(j.clamp(0, ny as i64 - 1) as usize) * nx + i.clamp(0, nx as i64 - 1) as usize].abs();
        for j in 0..ny as i64 {
            for i in 0..nx as i64 {
                let (gx, gy) = (0.5 * (wa(i + 1, j) - wa(i - 1, j)), 0.5 * (wa(i, j + 1) - wa(i, j - 1)));
                let l = (gx * gx + gy * gy).sqrt() + 1e-9;
                let k = j as usize * nx + i as usize;
                ax[k] += s.vorticity * gy / l * w[k];
                ay[k] -= s.vorticity * gx / l * w[k];
            }
        }
    }
    // cell-centre accelerations onto the faces
    let cell =
        |g: &[f64], i: i64, j: i64| g[(j.clamp(0, ny as i64 - 1) as usize) * nx + i.clamp(0, nx as i64 - 1) as usize];
    for j in 0..ny {
        for i in 0..=nx {
            let a = 0.5 * (cell(&ax, i as i64 - 1, j as i64) + cell(&ax, i as i64, j as i64));
            let k = f.ui(i, j);
            f.u[k] += (a * dt) as f32;
        }
    }
    for j in 0..=ny {
        for i in 0..nx {
            let a = 0.5 * (cell(&ay, i as i64, j as i64 - 1) + cell(&ay, i as i64, j as i64));
            let k = f.vi(i, j);
            f.v[k] += (a * dt) as f32;
        }
    }
    edges(f, edge);
    // ---- viscosity, projection, advection, projection
    let visc = (dt * s.viscosity / (h * h)) as f32;
    diffuse(&mut f.u, nx + 1, ny, visc, 20);
    diffuse(&mut f.v, nx, ny + 1, visc, 20);
    edges(f, edge);
    project(f, edge, s.iterations);
    let old = f.clone();
    let clamp =
        |x: f64, y: f64| if edge == Edge::Wrap { (x, y) } else { (x.clamp(0.0, nx as f64), y.clamp(0.0, ny as f64)) };
    for j in 0..ny {
        for i in 0..=nx {
            let (x, y) = (i as f64, j as f64 + 0.5);
            let vel = old.velocity(x, y, edge);
            let (bx, by) = clamp(x - dt * vel[0], y - dt * vel[1]);
            let k = f.ui(i, j);
            f.u[k] = old.velocity(bx, by, edge)[0] as f32;
        }
    }
    for j in 0..=ny {
        for i in 0..nx {
            let (x, y) = (i as f64 + 0.5, j as f64);
            let vel = old.velocity(x, y, edge);
            let (bx, by) = clamp(x - dt * vel[0], y - dt * vel[1]);
            let k = f.vi(i, j);
            f.v[k] = old.velocity(bx, by, edge)[1] as f32;
        }
    }
    edges(f, edge);
    project(f, edge, s.iterations);
    // ---- dye
    let d0 = f.dye.clone();
    let sample_dye = |x: f64, y: f64| -> [f32; 4] {
        let (x, y) = (x - 0.5, y - 0.5);
        let (x, y) = if edge == Edge::Wrap {
            (x.rem_euclid(nx as f64), y.rem_euclid(ny as f64))
        } else {
            (x.clamp(0.0, (nx - 1) as f64), y.clamp(0.0, (ny - 1) as f64))
        };
        let (i0, j0) = (x.floor() as usize % nx, y.floor() as usize % ny);
        let (i1, j1) = if edge == Edge::Wrap {
            ((i0 + 1) % nx, (j0 + 1) % ny)
        } else {
            ((i0 + 1).min(nx - 1), (j0 + 1).min(ny - 1))
        };
        let (fx, fy) = ((x - x.floor()) as f32, (y - y.floor()) as f32);
        std::array::from_fn(|c| {
            let a = d0[j0 * nx + i0][c] * (1.0 - fx) + d0[j0 * nx + i1][c] * fx;
            let b = d0[j1 * nx + i0][c] * (1.0 - fx) + d0[j1 * nx + i1][c] * fx;
            a * (1.0 - fy) + b * fy
        })
    };
    for j in 0..ny {
        for i in 0..nx {
            let (x, y) = (i as f64 + 0.5, j as f64 + 0.5);
            let vel = f.velocity(x, y, edge);
            let (bx, by) = clamp(x - dt * vel[0], y - dt * vel[1]);
            f.dye[j * nx + i] = sample_dye(bx, by);
        }
    }
    // semi-Lagrangian advection is not conservative: where no dye can leave (closed or wrapped
    // edges), each channel is rescaled to its total before advection
    if edge != Edge::Open {
        for c in 0..4 {
            let before: f64 = d0.iter().map(|d| d[c] as f64).sum();
            let after: f64 = f.dye.iter().map(|d| d[c] as f64).sum();
            if after > 1e-12 && before > 0.0 {
                let k = (before / after) as f32;
                f.dye.iter_mut().for_each(|d| d[c] *= k);
            }
        }
    }
    if edge == Edge::Open {
        for i in 0..nx {
            f.dye[i] = [0.0; 4];
            f.dye[(ny - 1) * nx + i] = [0.0; 4];
        }
        for j in 0..ny {
            f.dye[j * nx] = [0.0; 4];
            f.dye[j * nx + nx - 1] = [0.0; 4];
        }
    }
    let dif = (dt * s.diffusion / (h * h)) as f32;
    if dif > 0.0 {
        for c in 0..4 {
            let mut ch: Vec<f32> = f.dye.iter().map(|d| d[c]).collect();
            diffuse(&mut ch, nx, ny, dif, 20);
            for (d, x) in f.dye.iter_mut().zip(ch) {
                d[c] = x;
            }
        }
    }
    let kd = (-s.dissipation * dt).exp() as f32;
    let kv = (-s.velocity_dissipation * dt).exp() as f32;
    f.dye.iter_mut().for_each(|d| d.iter_mut().for_each(|x| *x *= kd));
    f.u.iter_mut().chain(f.v.iter_mut()).for_each(|x| *x *= kv);
}

/// Premultiplied RGBA of the dye, alpha capped at 1.
pub fn image(f: &Fluid) -> Vec<[f32; 4]> {
    f.dye
        .iter()
        .map(|d| {
            let k = if d[3] > 1.0 { 1.0 / d[3] } else { 1.0 };
            [d[0] * k, d[1] * k, d[2] * k, d[3] * k].map(|x| x.max(0.0))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(edge: Edge) -> FluidSpec {
        FluidSpec {
            size: [200.0, 200.0],
            resolution: 50,
            viscosity: 0.0,
            diffusion: 0.0,
            dissipation: 0.0,
            velocity_dissipation: 0.0,
            vorticity: 0.0,
            buoyancy: 0.0,
            iterations: 60,
            edge,
            sources: Vec::new(),
        }
    }

    #[test]
    fn projection_removes_divergence() {
        let s = spec(Edge::Closed);
        let mut f = init(&s);
        // a radial (purely divergent) blow-out in the middle, in cells per second
        let blow = |x: f64, y: f64| {
            let (dx, dy) = (x - 25.0, y - 25.0);
            let l = (dx * dx + dy * dy).sqrt().max(1.0);
            [4.0 * dx / l * (-l / 8.0).exp(), 4.0 * dy / l * (-l / 8.0).exp()]
        };
        for j in 0..f.ny {
            for i in 0..=f.nx {
                let k = f.ui(i, j);
                f.u[k] = blow(i as f64, j as f64 + 0.5)[0] as f32;
            }
        }
        for j in 0..=f.ny {
            for i in 0..f.nx {
                let k = f.vi(i, j);
                f.v[k] = blow(i as f64 + 0.5, j as f64)[1] as f32;
            }
        }
        edges(&mut f, s.edge);
        let before = divergence(&f);
        project(&mut f, s.edge, 400);
        let after = divergence(&f);
        assert!(after < before * 1e-3, "divergence {before} -> {after}");
        // a full step (with advection between two projections) stays divergence-free
        step(&s, &mut f, 0.0, 1.0 / 60.0, &mut |_| [0.0, 0.0]);
        assert!(divergence(&f) < before * 0.05, "after a step: {}", divergence(&f));
    }

    #[test]
    fn a_rising_source_carries_dye_up_and_conserves_it_in_a_closed_box() {
        let mut s = spec(Edge::Closed);
        s.sources.push(Source {
            pos: [100.0, 170.0],
            radius: 12.0,
            color: [1.0, 0.5, 0.2, 1.0],
            density: 4.0,
            velocity: [0.0, -150.0],
            start: 0.0,
            end: Some(0.5),
        });
        let mut f = init(&s);
        let dt = 1.0 / 60.0;
        for k in 0..90 {
            step(&s, &mut f, k as f64 * dt, dt, &mut |_| [0.0, 0.0]);
        }
        // dye centroid moved up from the source
        let (mut m, mut cy) = (0.0f64, 0.0f64);
        for j in 0..f.ny {
            for i in 0..f.nx {
                let a = f.dye[j * f.nx + i][3] as f64;
                m += a;
                cy += a * (j as f64 + 0.5) * f.h;
            }
        }
        assert!(m > 0.0 && cy / m < 150.0, "centroid y {}", cy / m);
        // in a closed box without dissipation the dye is conserved once the source stops
        let mass = |f: &Fluid| f.dye.iter().map(|d| d[3] as f64).sum::<f64>();
        let m1 = mass(&f);
        for k in 90..150 {
            step(&s, &mut f, k as f64 * dt, dt, &mut |_| [0.0, 0.0]);
        }
        let m2 = mass(&f);
        assert!((m2 - m1).abs() / m1 < 1e-4, "dye {m1} -> {m2}");
        assert!(f.dye.iter().all(|d| d.iter().all(|x| x.is_finite())));
    }

    #[test]
    fn dissipation_decays_dye_exponentially() {
        let mut s = spec(Edge::Closed);
        s.dissipation = 1.0;
        let mut f = init(&s);
        f.dye.iter_mut().for_each(|d| *d = [0.5, 0.5, 0.5, 0.5]);
        for k in 0..60 {
            step(&s, &mut f, k as f64 / 60.0, 1.0 / 60.0, &mut |_| [0.0, 0.0]);
        }
        let a = f.dye[25 * f.nx + 25][3];
        assert!((a - 0.5 * (-1.0f32).exp()).abs() < 1e-3, "{a}");
    }
}
