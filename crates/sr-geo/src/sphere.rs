//! Spherical geometry on the unit sphere, in radians: rotation, great-circle
//! interpolation and distance, polygon area and point-in-polygon.
//!
//! Ported from d3-geo 3.1 (ISC licence, © Mike Bostock), whose conventions
//! this crate keeps: polygon edges are great-circle arcs, and a ring's interior
//! is on its right (exterior rings clockwise seen from outside the sphere).

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

/// d3-geo's ε.
pub const EPS: f64 = 1e-6;
/// d3-geo's ε².
pub const EPS2: f64 = 1e-12;
/// Degrees → radians.
pub const RAD: f64 = PI / 180.0;
/// Radians → degrees.
pub const DEG: f64 = 180.0 / PI;

/// A point: longitude λ and latitude φ (radians).
pub type Ll = [f64; 2];
/// A unit vector.
pub type V3 = [f64; 3];

pub(crate) fn asin(x: f64) -> f64 {
    if x > 1.0 {
        FRAC_PI_2
    } else if x < -1.0 {
        -FRAC_PI_2
    } else {
        x.asin()
    }
}

pub(crate) fn acos(x: f64) -> f64 {
    if x > 1.0 {
        0.0
    } else if x < -1.0 {
        PI
    } else {
        x.acos()
    }
}

/// Unit vector of a point.
pub fn cartesian(p: Ll) -> V3 {
    let (l, f) = (p[0], p[1]);
    let cf = f.cos();
    [cf * l.cos(), cf * l.sin(), f.sin()]
}

/// Point of a vector.
pub fn spherical(c: V3) -> Ll {
    [c[1].atan2(c[0]), asin(c[2])]
}

pub(crate) fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub(crate) fn normalize(d: V3) -> V3 {
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    [d[0] / l, d[1] / l, d[2] / l]
}

/// Whether two points coincide within ε.
pub fn point_equal(a: &[f64], b: &[f64]) -> bool {
    (a[0] - b[0]).abs() < EPS && (a[1] - b[1]).abs() < EPS
}

fn wrap(l: f64) -> f64 {
    if l.abs() > PI {
        l - (l / TAU).round() * TAU
    } else {
        l
    }
}

/// A rotation of the sphere: first about the polar axis by δλ, then about the
/// y axis by δφ and the x axis by δγ (d3-geo `rotateRadians`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rotation {
    dl: f64,
    phi_gamma: Option<[f64; 4]>,
}

impl Rotation {
    /// The rotation by δλ, δφ, δγ (radians).
    pub fn new(dl: f64, dphi: f64, dgamma: f64) -> Rotation {
        let dl = dl % TAU;
        let phi_gamma = (dphi != 0.0 || dgamma != 0.0).then(|| [dphi.cos(), dphi.sin(), dgamma.cos(), dgamma.sin()]);
        Rotation { dl, phi_gamma }
    }

    /// Rotates a point.
    pub fn apply(&self, p: Ll) -> Ll {
        let (mut l, phi) = (p[0], p[1]);
        l = wrap(l + self.dl);
        let Some([cp, sp, cg, sg]) = self.phi_gamma else { return [l, phi] };
        let cphi = phi.cos();
        let (x, y, z) = (l.cos() * cphi, l.sin() * cphi, phi.sin());
        let k = z * cp + x * sp;
        [(y * cg - k * sg).atan2(x * cp - z * sp), asin(k * cg + y * sg)]
    }

    /// Rotates a point back.
    pub fn invert(&self, p: Ll) -> Ll {
        let (mut l, mut phi) = (p[0], p[1]);
        if let Some([cp, sp, cg, sg]) = self.phi_gamma {
            let cphi = phi.cos();
            let (x, y, z) = (l.cos() * cphi, l.sin() * cphi, phi.sin());
            let k = z * cg - y * sg;
            l = (y * cg + z * sg).atan2(x * cp + k * sp);
            phi = asin(k * cp - x * sp);
        }
        [wrap(l - self.dl), phi]
    }
}

/// Great-circle distance between two points (radians).
pub fn distance(a: Ll, b: Ll) -> f64 {
    let dl = (b[0] - a[0]).abs();
    let (sdl, cdl) = dl.sin_cos();
    let (s0, c0) = a[1].sin_cos();
    let (s1, c1) = b[1].sin_cos();
    let x = c1 * sdl;
    let y = c0 * s1 - s0 * c1 * cdl;
    let z = s0 * s1 + c0 * c1 * cdl;
    (x * x + y * y).sqrt().atan2(z)
}

/// The point a fraction `t` of the way along the great circle from `a` to `b` (d3 `geoInterpolate`).
pub fn interpolate(a: Ll, b: Ll, t: f64) -> Ll {
    let d = distance(a, b);
    if d < EPS2 {
        return a;
    }
    let (s, t0) = (d.sin(), t * d);
    let (ka, kb) = ((d - t0).sin() / s, t0.sin() / s);
    let (ca, cb) = (cartesian(a), cartesian(b));
    spherical([ka * ca[0] + kb * cb[0], ka * ca[1] + kb * cb[1], ka * ca[2] + kb * cb[2]])
}

/// Neumaier-compensated sum (d3-array's `Adder` plays this role).
#[derive(Default, Clone, Copy)]
pub(crate) struct Adder {
    s: f64,
    c: f64,
}

impl Adder {
    pub(crate) fn add(&mut self, x: f64) {
        let t = self.s + x;
        if self.s.abs() >= x.abs() {
            self.c += (self.s - t) + x;
        } else {
            self.c += (x - t) + self.s;
        }
        self.s = t;
    }
    pub(crate) fn value(&self) -> f64 {
        self.s + self.c
    }
}

/// Spherical excess of one ring against the south pole (half-angle sum of d3's `areaRingSum`).
fn ring_sum(ring: &[Ll], sum: &mut Adder) {
    let Some(&first) = ring.first() else { return };
    let (mut l0, phi0) = (first[0], first[1] / 2.0 + FRAC_PI_4);
    let (mut c0, mut s0) = (phi0.cos(), phi0.sin());
    for &p in ring[1..].iter().chain(std::iter::once(&first)) {
        let (l, phi) = (p[0], p[1] / 2.0 + FRAC_PI_4);
        let dl = l - l0;
        let sdl = if dl >= 0.0 { 1.0 } else { -1.0 };
        let adl = sdl * dl;
        let (c, s) = (phi.cos(), phi.sin());
        let k = s0 * s;
        sum.add((k * sdl * adl.sin()).atan2(c0 * c + k * adl.cos()));
        (l0, c0, s0) = (l, c, s);
    }
}

/// Spherical area of a polygon (steradians, d3 `geoArea`): rings without their closing
/// point, exterior clockwise; a counter-clockwise exterior encloses the rest of the sphere.
pub fn polygon_area(rings: &[Vec<Ll>]) -> f64 {
    let mut sum = Adder::default();
    for r in rings {
        ring_sum(r, &mut sum);
    }
    let a = sum.value();
    2.0 * if a < 0.0 { TAU + a } else { a }
}

/// Whether `point` lies inside a spherical polygon (d3 `polygonContains`).
pub fn polygon_contains(polygon: &[Vec<Ll>], point: Ll) -> bool {
    let longitude = |p: Ll| if p[0].abs() <= PI { p[0] } else { p[0].signum() * ((p[0].abs() + PI) % TAU - PI) };
    let lambda = longitude(point);
    let mut phi = point[1];
    let sin_phi = phi.sin();
    let normal = [lambda.sin(), -lambda.cos(), 0.0];
    let mut angle = 0.0;
    let mut winding = 0i64;
    let mut sum = Adder::default();
    if sin_phi == 1.0 {
        phi = FRAC_PI_2 + EPS;
    } else if sin_phi == -1.0 {
        phi = -FRAC_PI_2 - EPS;
    }
    for ring in polygon {
        let m = ring.len();
        if m == 0 {
            continue;
        }
        let mut point0 = ring[m - 1];
        let mut lambda0 = longitude(point0);
        let phi0 = point0[1] / 2.0 + FRAC_PI_4;
        let (mut sin0, mut cos0) = (phi0.sin(), phi0.cos());
        for &point1 in ring {
            let lambda1 = longitude(point1);
            let phi1 = point1[1] / 2.0 + FRAC_PI_4;
            let (sin1, cos1) = (phi1.sin(), phi1.cos());
            let delta = lambda1 - lambda0;
            let sign = if delta >= 0.0 { 1.0 } else { -1.0 };
            let abs_delta = sign * delta;
            let antimeridian = abs_delta > PI;
            let k = sin0 * sin1;
            sum.add((k * sign * abs_delta.sin()).atan2(cos0 * cos1 + k * abs_delta.cos()));
            angle += if antimeridian { delta + sign * TAU } else { delta };
            if antimeridian ^ (lambda0 >= lambda) ^ (lambda1 >= lambda) {
                let arc = normalize(cross(cartesian(point0), cartesian(point1)));
                let inter = normalize(cross(normal, arc));
                let phi_arc = if antimeridian ^ (delta >= 0.0) { -1.0 } else { 1.0 } * asin(inter[2]);
                if phi > phi_arc || (phi == phi_arc && (arc[0] != 0.0 || arc[1] != 0.0)) {
                    winding += if antimeridian ^ (delta >= 0.0) { 1 } else { -1 };
                }
            }
            lambda0 = lambda1;
            sin0 = sin1;
            cos0 = cos1;
            point0 = point1;
        }
    }
    let south_inside = angle < -EPS || (angle < EPS && sum.value() < -EPS2);
    south_inside ^ (winding & 1 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_round_trips() {
        let r = Rotation::new(0.7, -0.4, 0.2);
        for p in [[0.3, 0.2], [-2.9, -1.2], [3.1, 1.5]] {
            let q = r.invert(r.apply(p));
            assert!((q[0] - p[0]).abs() < 1e-12 && (q[1] - p[1]).abs() < 1e-12, "{p:?} {q:?}");
        }
    }

    #[test]
    fn distances_and_areas() {
        // London–New York ≈ 5570 km on a 6371 km sphere.
        let d = distance([-0.1278 * RAD, 51.5074 * RAD], [-74.006 * RAD, 40.7128 * RAD]) * 6371.0;
        assert!((d - 5570.0).abs() < 10.0, "{d}");
        // A clockwise 90°×90° octant has an eighth of the sphere's 4π.
        let oct = [[0.0, 0.0], [0.0, FRAC_PI_2], [FRAC_PI_2, 0.0]];
        let a = polygon_area(&[oct.to_vec()]);
        assert!((a - PI / 2.0).abs() < 1e-9, "{a}");
        let rev: Vec<Ll> = oct.iter().rev().copied().collect();
        assert!((polygon_area(&[rev]) - 3.5 * PI).abs() < 1e-9);
        let mid = interpolate([0.0, 0.0], [FRAC_PI_2, 0.0], 0.5);
        assert!((mid[0] - FRAC_PI_4).abs() < 1e-12);
    }

    #[test]
    fn containment_follows_winding() {
        let square = vec![vec![[-0.1, -0.1], [-0.1, 0.1], [0.1, 0.1], [0.1, -0.1]]];
        assert!(polygon_contains(&square, [0.0, 0.0]));
        assert!(!polygon_contains(&square, [1.0, 0.0]));
        let inverted: Vec<Vec<Ll>> = square.iter().map(|r| r.iter().rev().copied().collect()).collect();
        assert!(!polygon_contains(&inverted, [0.0, 0.0]));
        assert!(polygon_contains(&inverted, [1.0, 0.0]));
    }
}
