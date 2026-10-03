//! Sphere CCD against piecewise-linear triangle motion. Face, edge and vertex
//! contact equations have degree at most six. Derivative roots split each
//! polynomial into monotone intervals, including even-multiplicity tangencies.
//! Candidate roots are verified against the closest point on the actual triangle.
use super::{checked, Error, Hit};
use rapier3d_f64::parry::{math::Vector, query::PointQueryWithLocation, shape::Triangle};

#[derive(Debug)]
struct Node {
    lo: Vector,
    hi: Vector,
    children: Option<[usize; 2]>,
    triangle: usize,
}
#[derive(Debug)]
pub(super) struct Surface {
    start: Vec<Vector>,
    end: Vec<Vector>,
    triangles: Vec<[u32; 3]>,
    nodes: Vec<Node>,
    root: usize,
    epoch: f64,
    duration: f64,
}
impl Surface {
    pub(super) fn new(
        start: &[[f64; 3]],
        end: &[[f64; 3]],
        triangles: &[[u32; 3]],
        epoch: f64,
        duration: f64,
        max_bytes: usize,
    ) -> Result<Self, Error> {
        let bytes = start
            .len()
            .checked_mul(96)
            .and_then(|v| triangles.len().checked_mul(512).and_then(|t| v.checked_add(t)))
            .and_then(|v| v.checked_add(4096))
            .ok_or(Error::Limit("deforming collider bytes"))?;
        if bytes > max_bytes || !(3..=1_000_000).contains(&start.len()) || !(1..=1_000_000).contains(&triangles.len()) {
            return Err(Error::Limit("deforming collider count or memory budget"));
        }
        if start.len() != end.len()
            || !epoch.is_finite()
            || !duration.is_finite()
            || duration <= 0.
            || !(epoch + duration).is_finite()
            || epoch + duration <= epoch
            || start.iter().chain(end).flatten().any(|v| !v.is_finite())
            || triangles.iter().flatten().any(|&i| i as usize >= start.len())
        {
            return Err(Error::Invalid("deforming collider endpoints, topology or interval"));
        }
        let mut surface = Self {
            start: start.iter().copied().map(Vector::from_array).collect(),
            end: end.iter().copied().map(Vector::from_array).collect(),
            triangles: triangles.to_vec(),
            nodes: Vec::with_capacity(triangles.len() * 2),
            root: 0,
            epoch,
            duration,
        };
        for (a, b) in surface.start.iter().zip(&surface.end) {
            if !((*b - *a) / duration).is_finite() {
                return Err(Error::Invalid("deforming collider velocity overflow"));
            }
        }
        let mut order: Vec<_> = (0..triangles.len()).collect();
        surface.root = surface.build(&mut order);
        Ok(surface)
    }
    fn bounds(&self, triangle: usize) -> (Vector, Vector) {
        let mut lo = Vector::splat(f64::MAX);
        let mut hi = Vector::splat(f64::MIN);
        for i in self.triangles[triangle] {
            lo = lo.min(self.start[i as usize]).min(self.end[i as usize]);
            hi = hi.max(self.start[i as usize]).max(self.end[i as usize]);
        }
        (lo, hi)
    }
    fn build(&mut self, order: &mut [usize]) -> usize {
        let (mut lo, mut hi) = (Vector::splat(f64::MAX), Vector::splat(f64::MIN));
        for &t in order.iter() {
            let (a, b) = self.bounds(t);
            lo = lo.min(a);
            hi = hi.max(b);
        }
        let children = if order.len() == 1 {
            None
        } else {
            let extent = hi * 0.5 - lo * 0.5;
            let axis = if extent.x >= extent.y && extent.x >= extent.z {
                0
            } else if extent.y >= extent.z {
                1
            } else {
                2
            };
            let middle = order.len() / 2;
            order.select_nth_unstable_by(middle, |&a, &b| {
                let center = |t| {
                    let (lo, hi) = self.bounds(t);
                    lo[axis] * 0.5 + hi[axis] * 0.5
                };
                center(a).total_cmp(&center(b)).then(a.cmp(&b))
            });
            let (left, right) = order.split_at_mut(middle);
            Some([self.build(left), self.build(right)])
        };
        let index = self.nodes.len();
        self.nodes.push(Node { lo, hi, children, triangle: order[0] });
        index
    }
    pub(super) fn sweep(
        &self,
        time: f64,
        dt: f64,
        from: [f64; 3],
        to: [f64; 3],
        radius: f64,
    ) -> Result<Option<Hit>, Error> {
        let (from, to) = (Vector::from_array(from), Vector::from_array(to));
        let u = (time - self.epoch) / self.duration;
        let span = dt / self.duration;
        if !u.is_finite()
            || !span.is_finite()
            || span <= 0.
            || u < -1e-12
            || u + span > 1. + 1e-12
            || !from.is_finite()
            || !to.is_finite()
            || !(to - from).is_finite()
            || !radius.is_finite()
            || radius < 0.
        {
            return Err(Error::Invalid("deforming particle sweep domain"));
        }
        let lo = from.min(to) - Vector::splat(radius);
        let hi = from.max(to) + Vector::splat(radius);
        if !lo.is_finite() || !hi.is_finite() {
            return Err(Error::Invalid("particle swept bounds overflow"));
        }
        let mut stack = vec![self.root];
        let mut earliest: Option<Hit> = None;
        while let Some(i) = stack.pop() {
            let n = &self.nodes[i];
            if !lo.cmple(n.hi).all() || !n.lo.cmple(hi).all() {
                continue;
            }
            if let Some([a, b]) = n.children {
                stack.push(b);
                stack.push(a);
                continue;
            }
            let indices = self.triangles[n.triangle];
            let delta = indices.map(|i| self.end[i as usize] - self.start[i as usize]);
            let a =
                indices.map(|i| self.start[i as usize] + (self.end[i as usize] - self.start[i as usize]) * u - from);
            let b = delta.map(|d| d * span);
            if let Some(hit) = triangle_hit(a, b, to - from, radius, dt)? {
                let hit = Hit { position: (Vector::from_array(hit.position) + from).to_array(), ..hit };
                if earliest.as_ref().is_none_or(|h| hit.fraction < h.fraction) {
                    earliest = Some(checked(hit)?);
                }
            }
        }
        Ok(earliest)
    }
}

type Poly = [f64; 7];
fn dot(a: &[Vector], b: &[Vector]) -> Poly {
    let mut p = [0.; 7];
    for (i, a) in a.iter().enumerate() {
        for (j, b) in b.iter().enumerate() {
            p[i + j] += a.dot(*b);
        }
    }
    p
}
fn mul(a: &Poly, b: &Poly) -> Poly {
    let mut p = [0.; 7];
    for i in 0..7 {
        for j in 0..7 - i {
            p[i + j] += a[i] * b[j];
        }
    }
    p
}
fn cross(a: [Vector; 2], b: [Vector; 2]) -> [Vector; 3] {
    [a[0].cross(b[0]), a[0].cross(b[1]) + a[1].cross(b[0]), a[1].cross(b[1])]
}
fn eval(p: &[f64], x: f64) -> f64 {
    p.iter().rev().fold(0., |v, c| v * x + c)
}
fn roots(p: &[f64]) -> Vec<f64> {
    let degree = p.iter().rposition(|&v| v != 0.).unwrap_or(0);
    if degree == 0 {
        return Vec::new();
    }
    let scale = p[..=degree].iter().map(|v| v.abs()).fold(0., f64::max);
    let p: Vec<_> = p[..=degree].iter().map(|v| v / scale).collect();
    if degree == 1 {
        let x = -p[0] / p[1];
        return if (0. ..=1.).contains(&x) { vec![x] } else { vec![] };
    }
    let derivative: Vec<_> = (1..=degree).map(|i| p[i] * i as f64).collect();
    let mut splits = roots(&derivative);
    splits.extend([0., 1.]);
    splits.sort_by(f64::total_cmp);
    let mut out = Vec::new();
    for &x in &splits {
        if eval(&p, x).abs() < 1e-12 {
            out.push(x);
        }
    }
    for pair in splits.windows(2) {
        let (mut a, mut b) = (pair[0], pair[1]);
        let mut fa = eval(&p, a);
        if fa * eval(&p, b) >= 0. {
            continue;
        }
        for _ in 0..64 {
            let m = (a + b) * 0.5;
            let fm = eval(&p, m);
            if fa * fm <= 0. {
                b = m;
            } else {
                a = m;
                fa = fm;
            }
        }
        out.push((a + b) * 0.5);
    }
    out.sort_by(f64::total_cmp);
    out.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    out
}

fn triangle_hit(a: [Vector; 3], b: [Vector; 3], travel: Vector, radius: f64, dt: f64) -> Result<Option<Hit>, Error> {
    let scale = a.iter().chain(&b).chain([&travel]).map(|v| v.abs().max_element()).fold(radius, f64::max);
    if !scale.is_finite() || scale <= 0. {
        return Err(Error::Invalid("deforming triangle scale"));
    }
    let a = a.map(|v| v / scale);
    let b = b.map(|v| v / scale);
    let travel = travel / scale;
    let r = radius / scale;
    if radius > 0. && r < 1e-15 {
        return Err(Error::Invalid("particle radius below deforming sweep precision; reduce the step or scene extent"));
    }
    let mut candidates = vec![0.];
    for i in 0..3 {
        let w = [-a[i], travel - b[i]];
        let mut vertex = dot(&w, &w);
        vertex[0] -= r * r;
        candidates.extend(roots(&vertex));
        let j = (i + 1) % 3;
        let edge = [a[j] - a[i], b[j] - b[i]];
        let cross = cross(w, edge);
        let mut p = dot(&cross, &cross);
        let length = dot(&edge, &edge);
        for k in 0..7 {
            p[k] -= r * r * length[k];
        }
        candidates.extend(roots(&p));
    }
    let normal = cross([a[1] - a[0], b[1] - b[0]], [a[2] - a[0], b[2] - b[0]]);
    let distance = dot(&[-a[0], travel - b[0]], &normal);
    let mut face = mul(&distance, &distance);
    let n2 = dot(&normal, &normal);
    for k in 0..7 {
        face[k] -= r * r * n2[k];
    }
    candidates.extend(roots(&face));
    candidates.sort_by(f64::total_cmp);
    let triangle_at = |t: f64| {
        let [p, q, s] = std::array::from_fn(|i| a[i] + b[i] * t);
        Triangle::new(p, q, s)
    };
    let distance_at = |t: f64| {
        let triangle = triangle_at(t);
        let (projection, _) = triangle.project_local_point_and_get_location(travel * t, false);
        (travel * t - projection.point).length()
    };
    let tolerance = 1e-10 * r.max(1e-6);
    let mut outside = None;
    for mut t in candidates {
        let distance = distance_at(t);
        if !distance.is_finite() {
            return Err(Error::Invalid("deforming triangle closest point"));
        }
        if distance > r + tolerance {
            outside = Some(t);
            continue;
        }
        if distance < r - tolerance {
            if let Some(mut lo) = outside {
                // Squaring the face/edge equations can lose a small radius to
                // cancellation on a very long sweep. A penetrating candidate
                // brackets entry with the preceding verified outside point.
                // Refine using geometric distances, which retain that radius.
                let mut hi = t;
                for _ in 0..64 {
                    let mid = (lo + hi) * 0.5;
                    let d = distance_at(mid);
                    if !d.is_finite() {
                        return Err(Error::Invalid("deforming contact refinement"));
                    }
                    if d > r {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                t = hi;
            }
        }
        let triangle = triangle_at(t);
        let center = travel * t;
        let (projection, location) = triangle.project_local_point_and_get_location(center, false);
        let offset = center - projection.point;
        let distance = offset.length();
        if !distance.is_finite() {
            return Err(Error::Invalid("deforming triangle closest point"));
        }
        if distance > r + tolerance {
            continue;
        }
        let weights =
            location.barycentric_coordinates().ok_or(Error::Invalid("deforming triangle contact coordinates"))?;
        let velocity = b[0] * weights[0] + b[1] * weights[1] + b[2] * weights[2];
        let relative = travel - velocity;
        let normal = if distance > tolerance {
            offset / distance
        } else {
            let n = (triangle.b - triangle.a).cross(triangle.c - triangle.a);
            let length = n.length();
            if length == 0. || !length.is_finite() {
                return Err(Error::Invalid("collapsed deforming contact triangle"));
            }
            let n = n / length;
            if n.dot(relative) > 0. {
                -n
            } else {
                n
            }
        };
        let inward = relative.dot(normal);
        if t == 0. && distance >= r - tolerance && inward >= 0. {
            continue;
        }
        if t > 0. && inward > 1e-10 {
            continue;
        }
        return checked(Hit {
            fraction: t,
            position: ((center + normal * (r - distance)) * scale).to_array(),
            normal: normal.to_array(),
            velocity: (velocity * (scale / dt)).to_array(),
        })
        .map(Some);
    }
    Ok(None)
}
