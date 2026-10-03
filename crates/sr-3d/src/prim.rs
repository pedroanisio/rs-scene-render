//! Procedural primitives in scene units (x right, y down, z away from the
//! camera), centred on the origin. Every triangle winds so that
//! (b − a) × (c − a) points out of the surface.

use glam::{Vec2, Vec3};
use lyon_tessellation::math::point;
use lyon_tessellation::path::Path as LPath;
use lyon_tessellation::{BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers};

use crate::{compute_tangents, Primitive, Vertex};

fn v(p: Vec3, n: Vec3, uv: Vec2) -> Vertex {
    Vertex {
        pos: p.into(),
        normal: n.normalize_or_zero().into(),
        uv: uv.into(),
        tangent: [1.0, 0.0, 0.0, 1.0],
        ..Default::default()
    }
}

/// Flips triangles whose geometric normal disagrees with their vertex normals.
fn fix_winding(vs: &[Vertex], idx: &mut [u32]) {
    for t in idx.chunks_exact_mut(3) {
        let (a, b, c) =
            (Vec3::from(vs[t[0] as usize].pos), Vec3::from(vs[t[1] as usize].pos), Vec3::from(vs[t[2] as usize].pos));
        let g = (b - a).cross(c - a);
        let n = Vec3::from(vs[t[0] as usize].normal)
            + Vec3::from(vs[t[1] as usize].normal)
            + Vec3::from(vs[t[2] as usize].normal);
        if g.dot(n) < 0.0 {
            t.swap(1, 2);
        }
    }
}

fn finish(mut vs: Vec<Vertex>, mut idx: Vec<u32>) -> Primitive {
    fix_winding(&vs, &mut idx);
    compute_tangents(&mut vs, &idx);
    Primitive { vertices: vs, indices: idx, ..Default::default() }
}

/// A grid of (nu+1)×(nv+1) vertices from `f(u, v) -> (position, normal)`.
fn grid(nu: u32, nv: u32, f: impl Fn(f32, f32) -> (Vec3, Vec3), vs: &mut Vec<Vertex>, idx: &mut Vec<u32>) {
    let base = vs.len() as u32;
    for j in 0..=nv {
        for i in 0..=nu {
            let (u, w) = (i as f32 / nu as f32, j as f32 / nv as f32);
            let (p, n) = f(u, w);
            vs.push(v(p, n, Vec2::new(u, w)));
        }
    }
    for j in 0..nv {
        for i in 0..nu {
            let a = base + j * (nu + 1) + i;
            let (b, c, d) = (a + 1, a + nu + 1, a + nu + 2);
            idx.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
}

fn disc(center: Vec3, r: f32, n: Vec3, segs: u32, vs: &mut Vec<Vertex>, idx: &mut Vec<u32>) {
    let c = vs.len() as u32;
    vs.push(v(center, n, Vec2::splat(0.5)));
    for i in 0..=segs {
        let a = i as f32 / segs as f32 * std::f32::consts::TAU;
        let (s, co) = a.sin_cos();
        vs.push(v(center + Vec3::new(co * r, 0.0, s * r), n, Vec2::new(0.5 + co * 0.5, 0.5 + s * 0.5)));
    }
    for i in 0..segs {
        idx.extend_from_slice(&[c, c + 1 + i, c + 2 + i]);
    }
}

/// UV sphere; the poles lie on the y axis.
pub fn sphere(r: f32, segs: u32) -> Primitive {
    let (mut vs, mut idx) = (Vec::new(), Vec::new());
    let segs = segs.max(3);
    grid(
        segs,
        (segs / 2).max(2),
        |u, w| {
            let (th, ph) = (u * std::f32::consts::TAU, w * std::f32::consts::PI);
            let n = Vec3::new(ph.sin() * th.cos(), -ph.cos(), ph.sin() * th.sin());
            (n * r, n)
        },
        &mut vs,
        &mut idx,
    );
    finish(vs, idx)
}

/// Box of the given width (x), height (y) and depth (z).
pub fn cuboid(w: f32, h: f32, d: f32) -> Primitive {
    let (mut vs, mut idx) = (Vec::new(), Vec::new());
    let half = Vec3::new(w, h, d) * 0.5;
    for axis in 0..3 {
        for sign in [-1.0f32, 1.0] {
            let mut n = Vec3::ZERO;
            n[axis] = sign;
            let (ua, va) = ((axis + 1) % 3, (axis + 2) % 3);
            grid(
                1,
                1,
                |u, t| {
                    let mut p = Vec3::ZERO;
                    p[axis] = sign * half[axis];
                    p[ua] = (u * 2.0 - 1.0) * half[ua];
                    p[va] = (t * 2.0 - 1.0) * half[va];
                    (p, n)
                },
                &mut vs,
                &mut idx,
            );
        }
    }
    finish(vs, idx)
}

/// Plane in the xy plane facing the default camera (normal −z).
pub fn plane(w: f32, h: f32, segs: u32) -> Primitive {
    let (mut vs, mut idx) = (Vec::new(), Vec::new());
    let s = segs.clamp(1, 1024);
    grid(s, s, |u, t| (Vec3::new((u - 0.5) * w, (t - 0.5) * h, 0.0), Vec3::NEG_Z), &mut vs, &mut idx);
    finish(vs, idx)
}

/// Cylinder (`r_top` = `r_bottom`) or cone (`r_top` = 0) along y, the top at −y.
pub fn cylinder(r_top: f32, r_bottom: f32, h: f32, segs: u32) -> Primitive {
    let (mut vs, mut idx) = (Vec::new(), Vec::new());
    let segs = segs.max(3);
    let slope = (r_bottom - r_top) / h.max(1e-6);
    grid(
        segs,
        1,
        |u, t| {
            let a = u * std::f32::consts::TAU;
            let (s, c) = a.sin_cos();
            let r = r_top + (r_bottom - r_top) * t;
            (Vec3::new(c * r, (t - 0.5) * h, s * r), Vec3::new(c, -slope, s))
        },
        &mut vs,
        &mut idx,
    );
    if r_top > 0.0 {
        disc(Vec3::new(0.0, -h * 0.5, 0.0), r_top, Vec3::NEG_Y, segs, &mut vs, &mut idx);
    }
    if r_bottom > 0.0 {
        disc(Vec3::new(0.0, h * 0.5, 0.0), r_bottom, Vec3::Y, segs, &mut vs, &mut idx);
    }
    finish(vs, idx)
}

/// Torus lying in the xz plane: ring radius `r`, tube radius `tube`.
pub fn torus(r: f32, tube: f32, segs: u32) -> Primitive {
    let (mut vs, mut idx) = (Vec::new(), Vec::new());
    let segs = segs.max(3);
    grid(
        segs,
        (segs / 2).max(3),
        |u, t| {
            let (a, b) = (u * std::f32::consts::TAU, t * std::f32::consts::TAU);
            let ring = Vec3::new(a.cos(), 0.0, a.sin());
            let n = ring * b.cos() + Vec3::Y * b.sin();
            (ring * r + n * tube, n)
        },
        &mut vs,
        &mut idx,
    );
    finish(vs, idx)
}

/// Capsule along y: total height `h` (at least 2r).
pub fn capsule(r: f32, h: f32, segs: u32) -> Primitive {
    let (mut vs, mut idx) = (Vec::new(), Vec::new());
    let segs = segs.max(3);
    let half = (h * 0.5 - r).max(0.0);
    let rings = (segs / 2).max(2);
    // one grid over latitude, with the equator split to insert the straight section
    grid(
        segs,
        rings * 2 + 1,
        |u, t| {
            let th = u * std::f32::consts::TAU;
            let total = rings * 2 + 1;
            let k = (t * total as f32).round() as u32;
            let (ph, off) = if k <= rings {
                (k as f32 / rings as f32 * std::f32::consts::FRAC_PI_2, -half)
            } else {
                (
                    std::f32::consts::FRAC_PI_2 + (k - rings - 1) as f32 / rings as f32 * std::f32::consts::FRAC_PI_2,
                    half,
                )
            };
            let n = Vec3::new(ph.sin() * th.cos(), -ph.cos(), ph.sin() * th.sin());
            (n * r + Vec3::new(0.0, off, 0.0), n)
        },
        &mut vs,
        &mut idx,
    );
    finish(vs, idx)
}

/// Whether point `p` is inside the polygons under the nonzero rule.
fn inside(polys: &[Vec<Vec2>], p: Vec2) -> bool {
    let mut wind = 0i32;
    for ring in polys {
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            if a.y <= p.y {
                if b.y > p.y && (b - a).perp_dot(p - a) > 0.0 {
                    wind += 1;
                }
            } else if b.y <= p.y && (b - a).perp_dot(p - a) < 0.0 {
                wind -= 1;
            }
        }
    }
    wind != 0
}

fn tessellate(polys: &[Vec<Vec2>]) -> Result<(Vec<Vec2>, Vec<u32>), String> {
    let mut b = LPath::builder();
    for ring in polys.iter().filter(|r| r.len() >= 3) {
        b.begin(point(ring[0].x, ring[0].y));
        for p in &ring[1..] {
            b.line_to(point(p.x, p.y));
        }
        b.end(true);
    }
    let path = b.build();
    let mut geom: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    FillTessellator::new()
        .tessellate_path(
            &path,
            &FillOptions::default().with_fill_rule(FillRule::NonZero).with_tolerance(0.05),
            &mut BuffersBuilder::new(&mut geom, |fv: FillVertex| fv.position().to_array()),
        )
        .map_err(|e| format!("tessellation failed: {e:?}"))?;
    Ok((geom.vertices.into_iter().map(Vec2::from).collect(), geom.indices))
}

/// Extrudes closed polygons (nonzero fill) along z from −depth/2 to +depth/2,
/// with a chamfered bevel of width `bevel` on both edges.
pub fn extrude(polys: &[Vec<Vec2>], depth: f32, bevel: f32) -> Result<Primitive, String> {
    let rings: Vec<Vec<Vec2>> = polys
        .iter()
        .map(|r| {
            let mut r: Vec<Vec2> = r.clone();
            r.dedup_by(|a, b| a.distance_squared(*b) < 1e-10);
            if r.len() > 1 && r[0].distance_squared(r[r.len() - 1]) < 1e-10 {
                r.pop();
            }
            r
        })
        .filter(|r| r.len() >= 3)
        .collect();
    if rings.is_empty() {
        return Err("nothing to extrude".into());
    }
    let bevel = bevel.clamp(0.0, depth * 0.5);
    let (z0, z1) = (-depth * 0.5, depth * 0.5);
    // outward normal per edge: test which side of each ring the fill lies on
    let mut outward: Vec<Vec<Vec2>> = Vec::new();
    for ring in &rings {
        let n = ring.len();
        let (a, b) = (ring[0], ring[1 % n]);
        let left = (b - a).perp().normalize_or_zero();
        let probe = (a + b) * 0.5 + left * 1e-3;
        let sign = if inside(&rings, probe) { -1.0 } else { 1.0 };
        outward.push((0..n).map(|i| (ring[(i + 1) % n] - ring[i]).perp().normalize_or_zero() * sign).collect());
    }
    // inset rings for the caps (miter offset, clamped)
    let inset: Vec<Vec<Vec2>> = rings
        .iter()
        .zip(&outward)
        .map(|(ring, nrm)| {
            let n = ring.len();
            (0..n)
                .map(|i| {
                    let (na, nb) = (nrm[(i + n - 1) % n], nrm[i]);
                    let m = (na + nb).normalize_or_zero();
                    let k = (1.0 / m.dot(nb).max(0.25)).min(4.0);
                    ring[i] - m * bevel * k
                })
                .collect()
        })
        .collect();
    let (mut vs, mut idx) = (Vec::new(), Vec::new());
    // caps
    let (cap_pts, cap_tris) = tessellate(if bevel > 0.0 { &inset } else { &rings })?;
    let (lo, hi) =
        cap_pts.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(l, h), p| (l.min(*p), h.max(*p)));
    let span = (hi - lo).max(Vec2::splat(1e-6));
    for (z, nz) in [(z0, -1.0f32), (z1, 1.0)] {
        let base = vs.len() as u32;
        for p in &cap_pts {
            vs.push(v(Vec3::new(p.x, p.y, z), Vec3::new(0.0, 0.0, nz), (*p - lo) / span));
        }
        idx.extend(cap_tris.iter().map(|i| base + i));
    }
    // walls: bevel strip, side, bevel strip (flat per edge, smooth across shallow corners)
    let cos_smooth = 30f32.to_radians().cos();
    for (r, ring) in rings.iter().enumerate() {
        let n = ring.len();
        let nrm = &outward[r];
        let per = ring.iter().zip(ring.iter().cycle().skip(1)).map(|(a, b)| a.distance(*b)).sum::<f32>().max(1e-6);
        let mut along = 0.0;
        for i in 0..n {
            let j = (i + 1) % n;
            let ne = nrm[i];
            let corner = |other: Vec2| if other.dot(ne) > cos_smooth { (ne + other).normalize_or_zero() } else { ne };
            let (n_i, n_j) = (corner(nrm[(i + n - 1) % n]), corner(nrm[j]));
            let seg = ring[i].distance(ring[j]);
            let (u0, u1) = (along / per, (along + seg) / per);
            along += seg;
            let mut strip = |pa: Vec2, pb: Vec2, za: f32, qa: Vec2, qb: Vec2, zb: f32, na: Vec3, nb: Vec3| {
                let base = vs.len() as u32;
                vs.push(v(Vec3::new(pa.x, pa.y, za), na, Vec2::new(u0, 0.0)));
                vs.push(v(Vec3::new(pb.x, pb.y, za), nb, Vec2::new(u1, 0.0)));
                vs.push(v(Vec3::new(qa.x, qa.y, zb), na, Vec2::new(u0, 1.0)));
                vs.push(v(Vec3::new(qb.x, qb.y, zb), nb, Vec2::new(u1, 1.0)));
                idx.extend_from_slice(&[base, base + 1, base + 2, base + 1, base + 3, base + 2]);
            };
            let side = |n2: Vec2| Vec3::new(n2.x, n2.y, 0.0);
            if bevel > 0.0 {
                let (ia, ib) = (inset[r][i], inset[r][j]);
                let tilt = |n2: Vec2, z: f32| (side(n2) + Vec3::new(0.0, 0.0, z)).normalize();
                strip(ia, ib, z0, ring[i], ring[j], z0 + bevel, tilt(n_i, -1.0), tilt(n_j, -1.0));
                strip(ring[i], ring[j], z1 - bevel, ia, ib, z1, tilt(n_i, 1.0), tilt(n_j, 1.0));
            }
            strip(ring[i], ring[j], z0 + bevel, ring[i], ring[j], z1 - bevel, side(n_i), side(n_j));
        }
    }
    Ok(finish(vs, idx))
}

/// Flattens SVG path data into polygons (for `primitive="extrude"`).
pub fn path_polygons(d: &str, tol: f64) -> Result<Vec<Vec<Vec2>>, String> {
    let p = sr_vector::path::Path::parse(d).map_err(|e| format!("path: {e:?}"))?;
    Ok(polys_of(&p, tol))
}

/// Polygons of a vector path.
pub fn polys_of(p: &sr_vector::path::Path, tol: f64) -> Vec<Vec<Vec2>> {
    p.flatten(tol)
        .into_iter()
        .map(|poly| poly.pts.iter().map(|q| Vec2::new(q.x as f32, q.y as f32)).collect())
        .collect()
}

#[cfg(test)]
mod conical_normals {
    use super::*;

    #[test]
    fn tapered_cylinder_side_normals_point_outward_and_are_perpendicular_to_slope() {
        for (top, bottom) in [(0., 3.), (3., 0.), (1., 3.), (3., 1.)] {
            let height = 2.;
            let mesh = cylinder(top, bottom, height, 12);
            for vertex in &mesh.vertices[..26] {
                let uv = vertex.uv;
                let theta = uv[0] * std::f32::consts::TAU;
                let outward = Vec3::new(theta.cos(), 0., theta.sin());
                let slope = outward * (bottom - top) + Vec3::Y * height;
                let normal = Vec3::from(vertex.normal);
                assert!(normal.dot(outward) > 0.);
                assert!(normal.dot(slope).abs() < 1e-5, "{top}->{bottom}: normal {normal:?}, slope {slope:?}");
            }
        }
    }
}
