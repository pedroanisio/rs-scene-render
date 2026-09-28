//! Points and affine transforms.

use std::ops::{Add, Mul, Neg, Sub};

/// A 2D point or vector.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct P {
    pub x: f64,
    pub y: f64,
}

/// Shorthand constructor.
pub const fn p(x: f64, y: f64) -> P {
    P { x, y }
}

impl P {
    /// Dot product.
    pub fn dot(self, o: P) -> f64 {
        self.x * o.x + self.y * o.y
    }
    /// 2D cross product (z of the 3D cross).
    pub fn cross(self, o: P) -> f64 {
        self.x * o.y - self.y * o.x
    }
    /// Length.
    pub fn len(self) -> f64 {
        libm::hypot(self.x, self.y)
    }
    /// Unit vector (zero stays zero).
    pub fn norm(self) -> P {
        let l = self.len();
        if l > 1e-12 {
            p(self.x / l, self.y / l)
        } else {
            p(0.0, 0.0)
        }
    }
    /// Rotated 90° (x, y) → (−y, x).
    pub fn perp(self) -> P {
        p(-self.y, self.x)
    }
    /// Linear interpolation.
    pub fn lerp(self, o: P, t: f64) -> P {
        p(self.x + (o.x - self.x) * t, self.y + (o.y - self.y) * t)
    }
    /// Distance.
    pub fn dist(self, o: P) -> f64 {
        (self - o).len()
    }
    /// Rotation by `rad` about the origin.
    pub fn rot(self, rad: f64) -> P {
        let (s, c) = (libm::sin(rad), libm::cos(rad));
        p(self.x * c - self.y * s, self.x * s + self.y * c)
    }
    /// Angle of the vector in radians.
    pub fn angle(self) -> f64 {
        libm::atan2(self.y, self.x)
    }
}

impl Add for P {
    type Output = P;
    fn add(self, o: P) -> P {
        p(self.x + o.x, self.y + o.y)
    }
}
impl Sub for P {
    type Output = P;
    fn sub(self, o: P) -> P {
        p(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f64> for P {
    type Output = P;
    fn mul(self, s: f64) -> P {
        p(self.x * s, self.y * s)
    }
}
impl Neg for P {
    type Output = P;
    fn neg(self) -> P {
        p(-self.x, -self.y)
    }
}

/// Affine transform `[a, b, c, d, e, f]`: x' = a·x + c·y + e, y' = b·x + d·y + f (SVG order).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Xf(pub [f64; 6]);

impl Default for Xf {
    fn default() -> Xf {
        Xf::IDENTITY
    }
}

impl Xf {
    /// Identity.
    pub const IDENTITY: Xf = Xf([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    /// Translation.
    pub fn translate(x: f64, y: f64) -> Xf {
        Xf([1.0, 0.0, 0.0, 1.0, x, y])
    }
    /// Scale.
    pub fn scale(x: f64, y: f64) -> Xf {
        Xf([x, 0.0, 0.0, y, 0.0, 0.0])
    }
    /// Rotation, degrees, clockwise on screen (y down).
    pub fn rotate(deg: f64) -> Xf {
        let r = deg.to_radians();
        let (s, c) = (libm::sin(r), libm::cos(r));
        Xf([c, s, -s, c, 0.0, 0.0])
    }
    /// Skew, degrees.
    pub fn skew(x: f64, y: f64) -> Xf {
        Xf([1.0, libm::tan(y.to_radians()), libm::tan(x.to_radians()), 1.0, 0.0, 0.0])
    }
    /// `self` applied after `inner`: (self ∘ inner)(p) = self(inner(p)).
    pub fn mul(&self, inner: &Xf) -> Xf {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = inner.0;
        Xf([
            a * a2 + c * b2,
            b * a2 + d * b2,
            a * c2 + c * d2,
            b * c2 + d * d2,
            a * e2 + c * f2 + e,
            b * e2 + d * f2 + f,
        ])
    }
    /// Applies to a point.
    pub fn apply(&self, q: P) -> P {
        let [a, b, c, d, e, f] = self.0;
        p(a * q.x + c * q.y + e, b * q.x + d * q.y + f)
    }
    /// Applies to a vector (no translation).
    pub fn apply_vec(&self, q: P) -> P {
        let [a, b, c, d, _, _] = self.0;
        p(a * q.x + c * q.y, b * q.x + d * q.y)
    }
    /// Inverse, if not singular.
    pub fn inverse(&self) -> Option<Xf> {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if det.abs() < 1e-300 {
            return None;
        }
        let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
        Some(Xf([ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)]))
    }
    /// Largest scale factor (square root of the larger singular value squared).
    pub fn max_scale(&self) -> f64 {
        let [a, b, c, d, _, _] = self.0;
        let s1 = a * a + b * b;
        let s2 = c * c + d * d;
        let off = a * c + b * d;
        let tr = (s1 + s2) * 0.5;
        let det = libm::sqrt(((s1 - s2) * 0.5).powi(2) + off * off);
        libm::sqrt(tr + det)
    }
    /// Rotation of the x axis, degrees.
    pub fn rotation_deg(&self) -> f64 {
        libm::atan2(self.0[1], self.0[0]).to_degrees()
    }
    /// Translation part.
    pub fn origin(&self) -> P {
        p(self.0[4], self.0[5])
    }
    /// Composes translate · rotate · scale about an anchor (the node transform convention).
    pub fn trs(pos: P, anchor: P, rot_deg: f64, sx: f64, sy: f64) -> Xf {
        Xf::translate(pos.x, pos.y)
            .mul(&Xf::rotate(rot_deg))
            .mul(&Xf::scale(sx, sy))
            .mul(&Xf::translate(-anchor.x, -anchor.y))
    }
}

/// Axis-aligned bounds `[x0, y0, x1, y1]`, empty when x0 > x1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect(pub [f64; 4]);

impl Rect {
    /// Empty bounds.
    pub const EMPTY: Rect = Rect([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY]);
    /// Grows to include a point.
    pub fn add(&mut self, q: P) {
        self.0[0] = self.0[0].min(q.x);
        self.0[1] = self.0[1].min(q.y);
        self.0[2] = self.0[2].max(q.x);
        self.0[3] = self.0[3].max(q.y);
    }
    /// Whether nothing was added.
    pub fn is_empty(&self) -> bool {
        self.0[0] > self.0[2]
    }
    /// Union.
    pub fn union(&self, o: &Rect) -> Rect {
        Rect([self.0[0].min(o.0[0]), self.0[1].min(o.0[1]), self.0[2].max(o.0[2]), self.0[3].max(o.0[3])])
    }
    /// Grows by `d` on every side.
    pub fn pad(&self, d: f64) -> Rect {
        Rect([self.0[0] - d, self.0[1] - d, self.0[2] + d, self.0[3] + d])
    }
    /// Width and height.
    pub fn size(&self) -> P {
        p((self.0[2] - self.0[0]).max(0.0), (self.0[3] - self.0[1]).max(0.0))
    }
}
