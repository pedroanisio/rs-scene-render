//! Clay: solids defined by signed distance fields, joined by smooth unions (Quilez's polynomial
//! smooth minimum), carved by smooth subtraction, roughened by fingerprint-like displacement
//! and meshed every frame by naive surface nets (Gibson 1998, "Constrained elastic surface
//! nets"): one vertex per grid cell the surface crosses, at the mean of its edge crossings,
//! and one quad per crossed grid edge. Because the surface is re-extracted from the field,
//! blobs can merge and split as they animate, which a fixed mesh cannot.
//!
//! "Boil" re-seeds the displacement a number of times per second, the jitter of hand-animated
//! clay between frames.

use glam::{Quat, Vec3};

use crate::{Primitive, Vertex};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlobShape {
    Sphere,
    /// Width, height, depth.
    Box,
    /// Along local y, `length` between the cap centres.
    Capsule,
    /// In the local xz plane: ring `radius`, tube `width` / 2.
    Torus,
}

/// One solid, in the object's local scene units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blob {
    pub shape: BlobShape,
    pub center: Vec3,
    pub rotation: Quat,
    pub radius: f32,
    pub size: Vec3,
    pub length: f32,
    /// Smooth-union (or subtraction) width.
    pub blend: f32,
    pub subtract: bool,
}

/// Surface roughness: `amount` (0–1) of fingerprint-like displacement, re-seeded `boil` times a
/// second (0: never).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Finish {
    pub amount: f32,
    pub seed: u64,
    pub boil: f32,
}

fn smin(a: f32, b: f32, k: f32) -> f32 {
    if k <= 0.0 {
        return a.min(b);
    }
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b + (a - b) * h - k * h * (1.0 - h)
}

/// `base` with `cut` carved out, smoothly over `k`.
fn ssub(base: f32, cut: f32, k: f32) -> f32 {
    if k <= 0.0 {
        return base.max(-cut);
    }
    let h = (0.5 - 0.5 * (base + cut) / k).clamp(0.0, 1.0);
    base + (-cut - base) * h + k * h * (1.0 - h)
}

impl Blob {
    /// Signed distance from `p` (object space) to this solid.
    pub fn sdf(&self, p: Vec3) -> f32 {
        let q = self.rotation.inverse() * (p - self.center);
        match self.shape {
            BlobShape::Sphere => q.length() - self.radius,
            BlobShape::Box => {
                let d = q.abs() - self.size * 0.5;
                d.max(Vec3::ZERO).length() + d.x.max(d.y).max(d.z).min(0.0)
            }
            BlobShape::Capsule => {
                let h = self.length * 0.5;
                let y = q.y.clamp(-h, h);
                (q - Vec3::new(0.0, y, 0.0)).length() - self.radius
            }
            BlobShape::Torus => {
                let ring = (q.x * q.x + q.z * q.z).sqrt() - self.radius;
                (ring * ring + q.y * q.y).sqrt() - self.size.x * 0.5
            }
        }
    }

    /// A box containing the solid (with its blend width).
    fn bounds(&self) -> (Vec3, Vec3) {
        let r = match self.shape {
            BlobShape::Sphere => self.radius,
            BlobShape::Box => self.size.length() * 0.5,
            BlobShape::Capsule => self.radius + self.length * 0.5,
            BlobShape::Torus => self.radius + self.size.x * 0.5,
        } + self.blend;
        (self.center - Vec3::splat(r), self.center + Vec3::splat(r))
    }
}

/// The joined field: blobs in order, each unioned (or subtracted) into the ones before it.
pub fn field(blobs: &[Blob], p: Vec3) -> f32 {
    let mut d = f32::MAX;
    for (k, b) in blobs.iter().enumerate() {
        let s = b.sdf(p);
        d = if k == 0 {
            if b.subtract {
                f32::MAX
            } else {
                s
            }
        } else if b.subtract {
            ssub(d, s, b.blend)
        } else {
            smin(d, s, b.blend)
        };
    }
    d
}

fn hash3(seed: u64, x: i64, y: i64, z: i64) -> f32 {
    let mut h = seed ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= (z as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Trilinear value noise in [0, 1].
fn noise(seed: u64, p: Vec3) -> f32 {
    let f = p.floor();
    let t = p - f;
    let s = t * t * (Vec3::splat(3.0) - 2.0 * t);
    let (x, y, z) = (f.x as i64, f.y as i64, f.z as i64);
    let mut v = 0.0;
    for dz in 0..2 {
        for dy in 0..2 {
            for dx in 0..2 {
                let w = (if dx == 1 { s.x } else { 1.0 - s.x })
                    * (if dy == 1 { s.y } else { 1.0 - s.y })
                    * (if dz == 1 { s.z } else { 1.0 - s.z });
                v += w * hash3(seed, x + dx, y + dy, z + dz);
            }
        }
    }
    v
}

/// Displacement of the surface at `p` (scene units): smudges with ridges like fingerprints.
fn roughness(f: &Finish, seed: u64, p: Vec3) -> f32 {
    if f.amount <= 0.0 {
        return 0.0;
    }
    let broad = noise(seed, p / 14.0) - 0.5;
    // ridged bands: fingerprint whorls on a smaller scale
    let ridges = 1.0 - (2.0 * noise(seed ^ 0x51, p / 3.0) - 1.0).abs();
    f.amount * (2.5 * broad + 0.6 * ridges)
}

/// The clay's surface as a triangle mesh at time `t` (seconds, for the boil).
pub fn mesh(blobs: &[Blob], finish: &Finish, resolution: u32, t: f64) -> Primitive {
    let mut prim = Primitive::default();
    if blobs.iter().all(|b| b.subtract) {
        return prim;
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for b in blobs.iter().filter(|b| !b.subtract) {
        let (l, h) = b.bounds();
        lo = lo.min(l);
        hi = hi.max(h);
    }
    let pad = 4.0 * finish.amount + 2.0;
    lo -= Vec3::splat(pad);
    hi += Vec3::splat(pad);
    let cell = (hi - lo).max_element() / resolution.max(4) as f32;
    let dims = ((hi - lo) / cell).ceil().as_uvec3() + glam::UVec3::ONE;
    let (nx, ny, nz) = (dims.x as usize, dims.y as usize, dims.z as usize);
    let seed = if finish.boil > 0.0 {
        finish.seed ^ ((t * finish.boil as f64).floor() as u64).wrapping_mul(0x2545_F491_4F6C_DD1D)
    } else {
        finish.seed
    };
    let sample = |p: Vec3| field(blobs, p) - roughness(finish, seed, p);
    // field at the grid points
    let at = |i: usize, j: usize, k: usize| lo + Vec3::new(i as f32, j as f32, k as f32) * cell;
    let mut d = vec![0.0f32; nx * ny * nz];
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                d[(k * ny + j) * nx + i] = sample(at(i, j, k));
            }
        }
    }
    let idx = |i: usize, j: usize, k: usize| (k * ny + j) * nx + i;
    // one vertex per cell crossed by the surface
    let (cx, cy, cz) = (nx - 1, ny - 1, nz - 1);
    let mut vert_of = vec![u32::MAX; cx * cy * cz];
    let corners = [(0, 0, 0), (1, 0, 0), (0, 1, 0), (1, 1, 0), (0, 0, 1), (1, 0, 1), (0, 1, 1), (1, 1, 1)];
    let edges = [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];
    let grad = |p: Vec3| {
        let e = cell * 0.5;
        Vec3::new(
            sample(p + Vec3::X * e) - sample(p - Vec3::X * e),
            sample(p + Vec3::Y * e) - sample(p - Vec3::Y * e),
            sample(p + Vec3::Z * e) - sample(p - Vec3::Z * e),
        )
        .normalize_or_zero()
    };
    for k in 0..cz {
        for j in 0..cy {
            for i in 0..cx {
                let v: [f32; 8] = std::array::from_fn(|c| {
                    let (a, b, e) = corners[c];
                    d[idx(i + a, j + b, k + e)]
                });
                let inside = v.iter().filter(|x| **x < 0.0).count();
                if inside == 0 || inside == 8 {
                    continue;
                }
                let (mut sum, mut count) = (Vec3::ZERO, 0.0);
                for (a, b) in edges {
                    if (v[a] < 0.0) != (v[b] < 0.0) {
                        let s = v[a] / (v[a] - v[b]);
                        let (pa, pb) = (corners[a], corners[b]);
                        let pa = Vec3::new(pa.0 as f32, pa.1 as f32, pa.2 as f32);
                        let pb = Vec3::new(pb.0 as f32, pb.1 as f32, pb.2 as f32);
                        sum += pa + (pb - pa) * s;
                        count += 1.0;
                    }
                }
                let p = lo + (Vec3::new(i as f32, j as f32, k as f32) + sum / count) * cell;
                vert_of[(k * cy + j) * cx + i] = prim.vertices.len() as u32;
                prim.vertices.push(Vertex {
                    pos: p.into(),
                    normal: grad(p).into(),
                    uv: [0.0, 0.0],
                    tangent: [1.0, 0.0, 0.0, 1.0],
                    ..Default::default()
                });
            }
        }
    }
    // one quad per grid edge the surface crosses, between the four cells around it
    let cell_vert = |i: usize, j: usize, k: usize| vert_of[(k * cy + j) * cx + i];
    for k in 1..nz - 1 {
        for j in 1..ny - 1 {
            for i in 1..nx - 1 {
                let here = d[idx(i, j, k)];
                for axis in 0..3 {
                    let (ni, nj, nk) = match axis {
                        0 => (i + 1, j, k),
                        1 => (i, j + 1, k),
                        _ => (i, j, k + 1),
                    };
                    if ni >= nx || nj >= ny || nk >= nz {
                        continue;
                    }
                    let there = d[idx(ni, nj, nk)];
                    if (here < 0.0) == (there < 0.0) {
                        continue;
                    }
                    // the four cells sharing the edge (i, j, k) → (ni, nj, nk)
                    let q = match axis {
                        0 => [
                            cell_vert(i, j - 1, k - 1),
                            cell_vert(i, j, k - 1),
                            cell_vert(i, j, k),
                            cell_vert(i, j - 1, k),
                        ],
                        1 => [
                            cell_vert(i - 1, j, k - 1),
                            cell_vert(i - 1, j, k),
                            cell_vert(i, j, k),
                            cell_vert(i, j, k - 1),
                        ],
                        _ => [
                            cell_vert(i - 1, j - 1, k),
                            cell_vert(i, j - 1, k),
                            cell_vert(i, j, k),
                            cell_vert(i - 1, j, k),
                        ],
                    };
                    if q.contains(&u32::MAX) {
                        continue;
                    }
                    // wind outward: inside → outside along the axis keeps the order
                    let (a, b, c, e) = if here < 0.0 { (q[0], q[1], q[2], q[3]) } else { (q[0], q[3], q[2], q[1]) };
                    prim.indices.extend_from_slice(&[a, b, c, a, c, e]);
                }
            }
        }
    }
    prim
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sphere(x: f32, r: f32, blend: f32) -> Blob {
        Blob {
            shape: BlobShape::Sphere,
            center: Vec3::new(x, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            radius: r,
            size: Vec3::splat(60.0),
            length: 60.0,
            blend,
            subtract: false,
        }
    }

    const SMOOTH: Finish = Finish { amount: 0.0, seed: 1, boil: 0.0 };

    /// Signed volume by the divergence theorem (positive for outward winding).
    fn volume(p: &Primitive) -> f32 {
        p.indices
            .chunks_exact(3)
            .map(|t| {
                let v = |k: usize| Vec3::from(p.vertices[t[k] as usize].pos);
                v(0).dot(v(1).cross(v(2))) / 6.0
            })
            .sum()
    }

    /// Connected components of the mesh (by shared vertices).
    fn components(p: &Primitive) -> usize {
        let mut parent: Vec<usize> = (0..p.vertices.len()).collect();
        fn find(p: &mut [usize], x: usize) -> usize {
            let mut x = x;
            while p[x] != x {
                p[x] = p[p[x]];
                x = p[x];
            }
            x
        }
        for t in p.indices.chunks_exact(3) {
            for k in 1..3 {
                let (a, b) = (find(&mut parent, t[0] as usize), find(&mut parent, t[k] as usize));
                parent[a] = b;
            }
        }
        let used: std::collections::BTreeSet<usize> =
            p.indices.iter().map(|&i| find(&mut parent, i as usize)).collect();
        used.len()
    }

    #[test]
    fn a_sphere_meshes_closed_with_the_right_volume_and_outward_normals() {
        let m = mesh(&[sphere(0.0, 40.0, 0.0)], &SMOOTH, 64, 0.0);
        let v = volume(&m);
        let want = 4.0 / 3.0 * std::f32::consts::PI * 40.0f32.powi(3);
        assert!((v - want).abs() / want < 0.02, "volume {v} vs {want}");
        for vx in &m.vertices {
            let p = Vec3::from(vx.pos);
            assert!((p.length() - 40.0).abs() < 1.0, "on the surface: {}", p.length());
            assert!(Vec3::from(vx.normal).dot(p.normalize()) > 0.9, "outward normal");
        }
        // every edge is shared by exactly two triangles: closed
        let mut edges = std::collections::HashMap::new();
        for t in m.indices.chunks_exact(3) {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        assert!(edges.values().all(|&c| c == 2), "open edges");
    }

    #[test]
    fn blobs_merge_when_they_meet_and_split_when_they_part() {
        // apart: two pieces; close with a smooth blend: one piece with a neck
        let apart = mesh(&[sphere(-50.0, 25.0, 10.0), sphere(50.0, 25.0, 10.0)], &SMOOTH, 64, 0.0);
        let near = mesh(&[sphere(-26.0, 25.0, 10.0), sphere(26.0, 25.0, 10.0)], &SMOOTH, 64, 0.0);
        assert_eq!(components(&apart), 2);
        assert_eq!(components(&near), 1);
        // subtracting a sphere carves a hole: less volume
        let mut cut = sphere(20.0, 20.0, 4.0);
        cut.subtract = true;
        let whole = volume(&mesh(&[sphere(0.0, 40.0, 0.0)], &SMOOTH, 64, 0.0));
        let carved = volume(&mesh(&[sphere(0.0, 40.0, 0.0), cut], &SMOOTH, 64, 0.0));
        assert!(carved < whole * 0.95, "{carved} vs {whole}");
    }

    #[test]
    fn fingerprints_roughen_and_boil_changes_the_surface_between_frames() {
        let clay = Finish { amount: 0.6, seed: 9, boil: 12.0 };
        let a = mesh(&[sphere(0.0, 40.0, 0.0)], &clay, 64, 0.0);
        let b = mesh(&[sphere(0.0, 40.0, 0.0)], &clay, 64, 0.04);
        let c = mesh(&[sphere(0.0, 40.0, 0.0)], &clay, 64, 0.1);
        // within one boil frame (1/12 s) the surface holds; across it, it changes
        assert_eq!(a.vertices.len(), b.vertices.len());
        assert!(a.vertices.iter().zip(&b.vertices).all(|(p, q)| p.pos == q.pos));
        assert!(
            a.vertices.len() != c.vertices.len() || a.vertices.iter().zip(&c.vertices).any(|(p, q)| p.pos != q.pos)
        );
        // the radius now varies around the sphere
        let r: Vec<f32> = a.vertices.iter().map(|v| Vec3::from(v.pos).length()).collect();
        let (lo, hi) = r.iter().fold((f32::MAX, f32::MIN), |(l, h), &x| (l.min(x), h.max(x)));
        assert!(hi - lo > 1.0, "{lo}..{hi}");
    }
}
