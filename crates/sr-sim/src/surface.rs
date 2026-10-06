//! The normal of a rigid body's surface at a point, from the body's own geometry.
//!
//! A contact reports the normal of the feature it touched, and on a mesh of triangles that is
//! the normal of whichever triangle, edge or corner the shape happened to meet, which tilts with
//! the tessellation. What the surface does at a point does not: a flat surface has one normal,
//! and a curved one has the normal that its triangles' corners agree on, interpolated across the
//! triangle the point is on.

use crate::physics3d::Shape3;

type V = [f64; 3];

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(a: V) -> Option<V> {
    let length = dot(a, a).sqrt();
    (length.is_finite() && length > 0.0).then(|| a.map(|c| c / length))
}

/// The barycentric coordinates, over `a`, `b` and `c`, of the point of the triangle nearest `p`.
fn nearest_in_triangle(p: V, a: V, b: V, c: V) -> [f64; 3] {
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return [1.0, 0.0, 0.0];
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return [0.0, 1.0, 0.0];
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return [1.0 - v, v, 0.0];
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return [0.0, 0.0, 1.0];
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return [1.0 - w, 0.0, w];
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return [0.0, 1.0 - w, w];
    }
    let denominator = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denominator, vc * denominator);
    [1.0 - v - w, v, w]
}

/// The unit normal of the surface of `shape` nearest `point`, both in the body's own frame, or
/// `None` where the geometry gives none (a shape without a plain normal, a mesh with no usable
/// triangle). For a mesh it is on the side the nearest triangle's winding faces: a caller that
/// knows which side it came from picks the sign.
pub fn normal(shape: &Shape3, point: V) -> Option<V> {
    match shape {
        Shape3::Sphere(_) => unit(point),
        Shape3::Box(half) => {
            let scaled: Vec<f64> = (0..3).map(|i| if half[i] > 0.0 { point[i] / half[i] } else { 0.0 }).collect();
            let axis = (0..3).max_by(|&i, &j| scaled[i].abs().total_cmp(&scaled[j].abs()))?;
            let mut n = [0.0; 3];
            n[axis] = if scaled[axis] < 0.0 { -1.0 } else { 1.0 };
            Some(n)
        }
        Shape3::TriMesh(points, triangles) | Shape3::Decomposition(points, triangles) => {
            mesh_normal(points, triangles, point)
        }
        _ => None,
    }
}

fn mesh_normal(points: &[V], triangles: &[[u32; 3]], p: V) -> Option<V> {
    let corner = |t: &[u32; 3]| -> Option<[V; 3]> {
        Some([*points.get(t[0] as usize)?, *points.get(t[1] as usize)?, *points.get(t[2] as usize)?])
    };
    // the triangle whose nearest point is nearest
    let mut best: Option<(f64, usize, [f64; 3])> = None;
    for (k, t) in triangles.iter().enumerate() {
        let Some([a, b, c]) = corner(t) else { continue };
        if unit(cross(sub(b, a), sub(c, a))).is_none() {
            continue;
        }
        let w = nearest_in_triangle(p, a, b, c);
        let q: V = std::array::from_fn(|i| w[0] * a[i] + w[1] * b[i] + w[2] * c[i]);
        let d = sub(p, q);
        let distance = dot(d, d);
        if best.is_none_or(|(least, ..)| distance < least) {
            best = Some((distance, k, w));
        }
    }
    let (_, hit, w) = best?;
    let [a, b, c] = corner(&triangles[hit])?;
    let face = unit(cross(sub(b, a), sub(c, a)))?;
    // each corner's normal: the area-weighted faces around it, every one on the hit face's side
    let mut sum = [0.0; 3];
    for (corner_index, weight) in triangles[hit].iter().zip(w) {
        if weight == 0.0 {
            continue;
        }
        let mut around = [0.0; 3];
        for t in triangles.iter().filter(|t| t.contains(corner_index)) {
            let Some([a, b, c]) = corner(t) else { continue };
            let twice_area = cross(sub(b, a), sub(c, a));
            let side = if dot(twice_area, face) < 0.0 { -1.0 } else { 1.0 };
            for i in 0..3 {
                around[i] += side * twice_area[i];
            }
        }
        let around = unit(around)?;
        for i in 0..3 {
            sum[i] += weight * around[i];
        }
    }
    unit(sum).or(Some(face))
}

/// The point of the surface of `shape` nearest `point`, both in the body's own frame, or `None`
/// where the geometry gives none (a shape that is not a sphere, a box or a mesh, or a point at the
/// centre of a sphere). Where a body that has just met this surface is: the contact point of
/// a body that went on a little before the contact was found is inside it by as much.
pub fn nearest(shape: &Shape3, point: V) -> Option<V> {
    match shape {
        Shape3::Sphere(radius) => unit(point).map(|u| u.map(|c| c * radius)),
        Shape3::Box(half) => {
            let inside = (0..3).all(|i| point[i].abs() < half[i]);
            if inside {
                // the face it is nearest
                let axis = (0..3).min_by(|&i, &j| (half[i] - point[i].abs()).total_cmp(&(half[j] - point[j].abs())))?;
                let mut out = point;
                out[axis] = half[axis].copysign(point[axis]);
                Some(out)
            } else {
                Some(std::array::from_fn(|i| point[i].clamp(-half[i], half[i])))
            }
        }
        Shape3::TriMesh(points, triangles) | Shape3::Decomposition(points, triangles) => {
            let mut best: Option<(f64, V)> = None;
            for t in triangles {
                let (Some(a), Some(b), Some(c)) =
                    (points.get(t[0] as usize), points.get(t[1] as usize), points.get(t[2] as usize))
                else {
                    continue;
                };
                let w = nearest_in_triangle(point, *a, *b, *c);
                let q: V = std::array::from_fn(|i| w[0] * a[i] + w[1] * b[i] + w[2] * c[i]);
                let d = sub(point, q);
                let distance = dot(d, d);
                if best.is_none_or(|(least, _)| distance < least) {
                    best = Some((distance, q));
                }
            }
            best.map(|(_, q)| q)
        }
        _ => None,
    }
}
