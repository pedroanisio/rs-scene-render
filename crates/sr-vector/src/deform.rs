//! The 14 deform modifiers as point maps in a node's local space. Vector
//! content deforms its flattened outline point by point; raster content
//! deforms a grid mesh (see [`grid`]) that the compositor draws.

use crate::arap::Puppet;
use crate::geom::{p, P};
use crate::modifiers::smooth_noise;

/// Which axis a deformer acts along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

/// A deformer with parameters resolved for this frame.
#[derive(Debug, Clone)]
pub enum Deformer {
    /// Bends the box around an arc: the centre line along `axis` turns through `amount` degrees.
    Bend { amount: f64, axis: Axis, center: P, extent: f64 },
    /// Rotates by `amount` degrees at `center`, falling off to nothing at `radius`.
    Twist { amount: f64, center: P, radius: f64 },
    /// Sine displacement across `axis`: `amount` px, `frequency` cycles over the box, `phase` degrees.
    Wave { amount: f64, frequency: f64, phase: f64, axis: Axis, size: P },
    /// Area-preserving scale about `center`: along `axis` by 1 − amount (squash) or 1 + amount (stretch).
    Squash { amount: f64, axis: Axis, center: P },
    /// Bicubic grid of control-point displacements over the box.
    MeshWarp { rows: usize, cols: usize, size: P, offsets: Vec<P> },
    /// As-rigid-as-possible puppet.
    Puppet(Box<Puppet>),
    /// Linear blend skinning: per-bone matrices from rest to current pose, with automatic weights.
    Skin(Box<crate::rig::Skin>),
    /// Radial magnification inside `radius` (negative amount pinches).
    Bulge { amount: f64, center: P, radius: f64 },
    /// Maps the disc onto a hemisphere, blended by `amount`.
    Spherize { amount: f64, center: P, radius: f64 },
    /// Circular waves: `amount` px, `frequency` waves per 100 px, `phase` degrees, fading at `radius`.
    Ripple { amount: f64, frequency: f64, phase: f64, center: P, radius: f64 },
    /// Smooth noise displacement: `amount` px, `frequency` per 100 px, `phase` scrolls it.
    Turbulence { amount: f64, frequency: f64, phase: f64, seed: u64 },
    /// Homography taking the box corners to four points (TL, TR, BR, BL).
    CornerPin { h: [f64; 9] },
}

impl Deformer {
    /// Maps a local point.
    pub fn apply(&self, q: P) -> P {
        match self {
            Deformer::Bend { amount, axis, center, extent } => {
                let theta = amount.to_radians();
                if theta.abs() < 1e-9 || *extent <= 0.0 {
                    return q;
                }
                let r = extent / theta;
                let (u, v) = match axis {
                    Axis::X => (q.x - center.x, q.y - center.y),
                    Axis::Y => (q.y - center.y, center.x - q.x),
                };
                let phi = u / r;
                let rr = r - v;
                let (du, dv) = (rr * libm::sin(phi), r - rr * libm::cos(phi));
                match axis {
                    Axis::X => p(center.x + du, center.y + dv),
                    Axis::Y => p(center.x - dv, center.y + du),
                }
            }
            Deformer::Twist { amount, center, radius } => {
                let d = q.dist(*center);
                if d >= *radius || *radius <= 0.0 {
                    return q;
                }
                let f = 1.0 - d / radius;
                *center + (q - *center).rot((amount * f * f).to_radians())
            }
            Deformer::Wave { amount, frequency, phase, axis, size } => {
                let tau = std::f64::consts::TAU;
                match axis {
                    Axis::Y => {
                        p(q.x, q.y + amount * libm::sin(tau * (frequency * q.x / size.x.max(1e-9) + phase / 360.0)))
                    }
                    Axis::X => {
                        p(q.x + amount * libm::sin(tau * (frequency * q.y / size.y.max(1e-9) + phase / 360.0)), q.y)
                    }
                }
            }
            Deformer::Squash { amount, axis, center } => {
                let s = (1.0 - amount).max(1e-3);
                let d = q - *center;
                match axis {
                    Axis::Y => *center + p(d.x / s, d.y * s),
                    Axis::X => *center + p(d.x * s, d.y / s),
                }
            }
            Deformer::MeshWarp { rows, cols, size, offsets } => q + mesh_offset(*rows, *cols, *size, offsets, q),
            Deformer::Puppet(pp) => pp.map(q),
            Deformer::Skin(s) => s.map(q),
            Deformer::Bulge { amount, center, radius } => {
                let d = q.dist(*center);
                if d >= *radius || *radius <= 0.0 {
                    return q;
                }
                let f = 1.0 - d / radius;
                *center + (q - *center) * (1.0 + amount * f * f)
            }
            Deformer::Spherize { amount, center, radius } => {
                let d = q.dist(*center);
                if d >= *radius || d < 1e-12 || *radius <= 0.0 {
                    return q;
                }
                let target = radius * libm::sin(std::f64::consts::FRAC_PI_2 * d / radius);
                let nd = d + (target - d) * amount.clamp(-1.0, 1.0);
                *center + (q - *center) * (nd / d)
            }
            Deformer::Ripple { amount, frequency, phase, center, radius } => {
                let d = q.dist(*center);
                if d < 1e-12 {
                    return q;
                }
                let fall = if *radius > 0.0 { (1.0 - d / radius).max(0.0) } else { 1.0 };
                let w = amount * fall * libm::sin(std::f64::consts::TAU * (d * frequency / 100.0 - phase / 360.0));
                q + (q - *center) * (w / d)
            }
            Deformer::Turbulence { amount, frequency, phase, seed } => {
                let f = frequency / 100.0;
                let nx = noise2(*seed, q.x * f + phase, q.y * f);
                let ny = noise2(seed.wrapping_add(0x5bd1), q.x * f, q.y * f + phase);
                q + p(nx, ny) * *amount
            }
            Deformer::CornerPin { h } => {
                let w = h[6] * q.x + h[7] * q.y + h[8];
                let w = if w.abs() < 1e-12 { 1e-12 } else { w };
                p((h[0] * q.x + h[1] * q.y + h[2]) / w, (h[3] * q.x + h[4] * q.y + h[5]) / w)
            }
        }
    }
}

/// Applies a chain of deformers in order.
pub fn apply_all(ds: &[Deformer], q: P) -> P {
    ds.iter().fold(q, |q, d| d.apply(q))
}

fn noise2(seed: u64, x: f64, y: f64) -> f64 {
    // separable smooth value noise, bilinear across rows
    let j = libm::floor(y);
    let fy = y - j;
    let u = fy * fy * (3.0 - 2.0 * fy);
    let a = smooth_noise(seed, j as i64 as u64, x);
    let b = smooth_noise(seed, (j as i64 + 1) as u64, x);
    a + (b - a) * u
}

/// Catmull-Rom weights.
fn cr(t: f64) -> [f64; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [(-t3 + 2.0 * t2 - t) * 0.5, (3.0 * t3 - 5.0 * t2 + 2.0) * 0.5, (-3.0 * t3 + 4.0 * t2 + t) * 0.5, (t3 - t2) * 0.5]
}

fn mesh_offset(rows: usize, cols: usize, size: P, offsets: &[P], q: P) -> P {
    if rows < 2 || cols < 2 || offsets.len() < rows * cols {
        return p(0.0, 0.0);
    }
    let gx = (q.x / size.x.max(1e-9)).clamp(0.0, 1.0) * (cols - 1) as f64;
    let gy = (q.y / size.y.max(1e-9)).clamp(0.0, 1.0) * (rows - 1) as f64;
    let (c0, r0) = ((gx.floor() as usize).min(cols - 2), (gy.floor() as usize).min(rows - 2));
    let (wx, wy) = (cr(gx - c0 as f64), cr(gy - r0 as f64));
    let at =
        |r: i64, c: i64| offsets[(r.clamp(0, rows as i64 - 1) as usize) * cols + c.clamp(0, cols as i64 - 1) as usize];
    let mut s = p(0.0, 0.0);
    for (j, wyj) in wy.iter().enumerate() {
        for (i, wxi) in wx.iter().enumerate() {
            s = s + at(r0 as i64 + j as i64 - 1, c0 as i64 + i as i64 - 1) * (wyj * wxi);
        }
    }
    s
}

/// Homography mapping the corners of a `w`×`h` box to `to` (TL, TR, BR, BL).
pub fn corner_pin(w: f64, h: f64, to: [P; 4]) -> Option<[f64; 9]> {
    let from = [p(0.0, 0.0), p(w, 0.0), p(w, h), p(0.0, h)];
    // 8×8 linear system for h0..h7 with h8 = 1
    let mut a = [[0.0f64; 9]; 8];
    for k in 0..4 {
        let (x, y, u, v) = (from[k].x, from[k].y, to[k].x, to[k].y);
        a[2 * k] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        a[2 * k + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    for c in 0..8 {
        let piv = (c..8).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[piv][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, piv);
        for r in 0..8 {
            if r != c {
                let f = a[r][c] / a[c][c];
                let pivot = a[c];
                for (x, pv) in a[r].iter_mut().zip(pivot.iter()).skip(c) {
                    *x -= f * pv;
                }
            }
        }
    }
    let mut hm = [0.0; 9];
    for (k, row) in a.iter().enumerate() {
        hm[k] = row[8] / row[k];
    }
    hm[8] = 1.0;
    Some(hm)
}

/// A regular triangle mesh over `rect` (x, y, w, h): (positions, uvs, indices).
pub fn grid(rect: [f64; 4], cols: usize, rows: usize) -> (Vec<P>, Vec<P>, Vec<u32>) {
    let (cols, rows) = (cols.max(1), rows.max(1));
    let mut pos = Vec::with_capacity((cols + 1) * (rows + 1));
    let mut uv = Vec::with_capacity(pos.capacity());
    for r in 0..=rows {
        for c in 0..=cols {
            let (u, v) = (c as f64 / cols as f64, r as f64 / rows as f64);
            pos.push(p(rect[0] + rect[2] * u, rect[1] + rect[3] * v));
            uv.push(p(u, v));
        }
    }
    let mut idx = Vec::with_capacity(cols * rows * 6);
    for r in 0..rows {
        for c in 0..cols {
            let i = (r * (cols + 1) + c) as u32;
            let j = i + cols as u32 + 1;
            idx.extend_from_slice(&[i, i + 1, j, i + 1, j + 1, j]);
        }
    }
    (pos, uv, idx)
}
