//! Map projections: the d3-geo pipeline of rotate → clip on the sphere →
//! adaptive resampling → project, scale, translate and rotate on the plane →
//! clip to a rectangle.
//!
//! Ported from d3-geo 3.1 (`projection/*.js`, ISC licence, © Mike Bostock).

use std::f64::consts::{FRAC_PI_2, PI};

use crate::clip::{Clip, Rect};
use crate::data::Geometry;
use crate::sphere::{asin, cartesian, Ll, Rotation, EPS, RAD};

/// A raw projection: (λ, φ) in radians → unscaled plane coordinates (y up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Raw {
    Mercator,
    Equirectangular,
    EqualEarth,
    NaturalEarth,
    /// Albers: conic equal-area on two standard parallels (radians).
    ConicEqualArea {
        n: f64,
        c: f64,
        r0: f64,
    },
    /// Lambert cylindrical equal-area (Albers with parallels symmetric about the equator).
    CylindricalEqualArea {
        cos_phi0: f64,
    },
    /// Lambert conformal conic on two standard parallels.
    ConicConformal {
        n: f64,
        f: f64,
    },
    Orthographic,
    Stereographic,
    AzimuthalEqualArea,
    AzimuthalEquidistant,
}

fn tany(y: f64) -> f64 {
    ((FRAC_PI_2 + y) / 2.0).tan()
}

impl Raw {
    /// Albers (d3 `conicEqualAreaRaw`) on parallels `y0`, `y1` (radians).
    pub fn conic_equal_area(y0: f64, y1: f64) -> Raw {
        let sy0 = y0.sin();
        let n = (sy0 + y1.sin()) / 2.0;
        if n.abs() < EPS {
            return Raw::CylindricalEqualArea { cos_phi0: y0.cos() };
        }
        let c = 1.0 + sy0 * (2.0 * n - sy0);
        Raw::ConicEqualArea { n, c, r0: c.sqrt() / n }
    }

    /// Lambert conformal conic (d3 `conicConformalRaw`) on parallels `y0`, `y1` (radians).
    pub fn conic_conformal(y0: f64, y1: f64) -> Raw {
        let cy0 = y0.cos();
        let n = if y0 == y1 { y0.sin() } else { (cy0 / y1.cos()).ln() / (tany(y1) / tany(y0)).ln() };
        if n == 0.0 {
            return Raw::Mercator;
        }
        Raw::ConicConformal { n, f: cy0 * tany(y0).powf(n) / n }
    }

    /// Projects a point.
    pub fn forward(&self, l: f64, p: f64) -> [f64; 2] {
        match *self {
            Raw::Mercator => [l, tany(p).ln()],
            Raw::Equirectangular => [l, p],
            Raw::EqualEarth => {
                const A1: f64 = 1.340264;
                const A2: f64 = -0.081106;
                const A3: f64 = 0.000893;
                const A4: f64 = 0.003796;
                let m = 3f64.sqrt() / 2.0;
                let t = asin(m * p.sin());
                let (t2, t6) = (t * t, t * t * t * t * t * t);
                [
                    l * t.cos() / (m * (A1 + 3.0 * A2 * t2 + t6 * (7.0 * A3 + 9.0 * A4 * t2))),
                    t * (A1 + A2 * t2 + t6 * (A3 + A4 * t2)),
                ]
            }
            Raw::NaturalEarth => {
                let (p2, p4) = (p * p, p * p * p * p);
                [
                    l * (0.8707 - 0.131979 * p2 + p4 * (-0.013791 + p4 * (0.003971 * p2 - 0.001529 * p4))),
                    p * (1.007226 + p2 * (0.015085 + p4 * (-0.044475 + 0.028874 * p2 - 0.005916 * p4))),
                ]
            }
            Raw::ConicEqualArea { n, c, r0 } => {
                let r = (c - 2.0 * n * p.sin()).sqrt() / n;
                let x = l * n;
                [r * x.sin(), r0 - r * x.cos()]
            }
            Raw::CylindricalEqualArea { cos_phi0 } => [l * cos_phi0, p.sin() / cos_phi0],
            Raw::ConicConformal { n, f } => {
                let p = if f > 0.0 { p.max(-FRAC_PI_2 + EPS) } else { p.min(FRAC_PI_2 - EPS) };
                let r = f / tany(p).powf(n);
                [r * (n * l).sin(), f - r * (n * l).cos()]
            }
            Raw::Orthographic => [p.cos() * l.sin(), p.sin()],
            Raw::Stereographic => {
                let cy = p.cos();
                let k = 1.0 + l.cos() * cy;
                [cy * l.sin() / k, p.sin() / k]
            }
            Raw::AzimuthalEqualArea | Raw::AzimuthalEquidistant => {
                let (cx, cy) = (l.cos(), p.cos());
                let cxcy = cx * cy;
                let k = if *self == Raw::AzimuthalEqualArea {
                    (2.0 / (1.0 + cxcy)).sqrt()
                } else {
                    let c = crate::sphere::acos(cxcy);
                    if c == 0.0 {
                        0.0
                    } else {
                        c / c.sin()
                    }
                };
                if k.is_infinite() {
                    return [2.0, 0.0];
                }
                [k * cy * l.sin(), k * p.sin()]
            }
        }
    }

    /// d3's default pre-clip for the projection.
    pub fn default_clip(&self) -> Clip {
        match self {
            Raw::Orthographic => Clip::Circle((90.0 + EPS) * RAD),
            Raw::Stereographic => Clip::Circle(142.0 * RAD),
            Raw::AzimuthalEqualArea | Raw::AzimuthalEquidistant => Clip::Circle((180.0 - 1e-3) * RAD),
            _ => Clip::Antimeridian,
        }
    }
}

/// Projected geometry in pixels (y down): points, line pieces, and polygons as rings without
/// their closing point (fill non-zero).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Planar {
    pub points: Vec<[f64; 2]>,
    pub lines: Vec<Vec<[f64; 2]>>,
    pub polygons: Vec<Vec<Vec<[f64; 2]>>>,
}

impl Planar {
    /// d3 `geoPath.area`: per polygon, the absolute signed area of its rings.
    pub fn area(&self) -> f64 {
        self.polygons
            .iter()
            .map(|poly| {
                let mut s = crate::sphere::Adder::default();
                for r in poly {
                    for (i, a) in r.iter().enumerate() {
                        let b = r[(i + 1) % r.len()];
                        s.add(a[1] * b[0] - a[0] * b[1]);
                    }
                }
                s.value().abs()
            })
            .sum::<f64>()
            / 2.0
    }

    /// d3 `geoPath.bounds`: [[x0, y0], [x1, y1]] over every emitted point.
    pub fn bounds(&self) -> Option<[[f64; 2]; 2]> {
        let mut b = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
        let all = self.points.iter().chain(self.lines.iter().flatten()).chain(self.polygons.iter().flatten().flatten());
        for p in all {
            b[0][0] = b[0][0].min(p[0]);
            b[0][1] = b[0][1].min(p[1]);
            b[1][0] = b[1][0].max(p[0]);
            b[1][1] = b[1][1].max(p[1]);
        }
        (b[0][0] <= b[1][0]).then_some(b)
    }

    fn extend(&mut self, o: Planar) {
        self.points.extend(o.points);
        self.lines.extend(o.lines);
        self.polygons.extend(o.polygons);
    }
}

/// A configured projection (d3 `projectionMutator`).
#[derive(Clone, Debug, PartialEq)]
pub struct Projection {
    raw: Raw,
    k: f64,
    tx: f64,
    ty: f64,
    center: Ll,
    rotate_deg: [f64; 3],
    rotation: Rotation,
    alpha: f64,
    reflect: [f64; 2],
    clip: Clip,
    extent: Option<Rect>,
    delta2: f64,
    // derived: plane transform after the raw projection
    ta: f64,
    tb: f64,
    dx: f64,
    dy: f64,
}

impl Projection {
    /// A projection with d3's defaults: scale 150, translate (480, 250), precision √0.5.
    pub fn new(raw: Raw) -> Projection {
        let mut p = Projection {
            raw,
            k: 150.0,
            tx: 480.0,
            ty: 250.0,
            center: [0.0, 0.0],
            rotate_deg: [0.0; 3],
            rotation: Rotation::new(0.0, 0.0, 0.0),
            alpha: 0.0,
            reflect: [1.0, 1.0],
            clip: raw.default_clip(),
            extent: None,
            delta2: 0.5,
            ta: 1.0,
            tb: 0.0,
            dx: 0.0,
            dy: 0.0,
        };
        p.recenter();
        p
    }

    /// The raw projection.
    pub fn raw(&self) -> Raw {
        self.raw
    }
    /// Scale (pixels per radian at the projection's reference).
    pub fn scale(&self) -> f64 {
        self.k
    }
    /// Sets the scale.
    pub fn set_scale(&mut self, k: f64) -> &mut Self {
        self.k = k;
        self.recenter()
    }
    /// Sets the pixel position of the centre.
    pub fn set_translate(&mut self, x: f64, y: f64) -> &mut Self {
        (self.tx, self.ty) = (x, y);
        self.recenter()
    }
    /// Sets the centre (degrees): the point placed at the translate.
    pub fn set_center(&mut self, lon: f64, lat: f64) -> &mut Self {
        self.center = [lon % 360.0 * RAD, lat % 360.0 * RAD];
        self.recenter()
    }
    /// Sets the sphere's rotation (degrees): yaw λ, pitch φ, roll γ.
    pub fn set_rotate(&mut self, r: [f64; 3]) -> &mut Self {
        self.rotate_deg = r.map(|v| v % 360.0);
        self.recenter()
    }
    /// Sets the rotation of the plane (degrees, d3 `angle`).
    pub fn set_angle(&mut self, deg: f64) -> &mut Self {
        self.alpha = deg % 360.0 * RAD;
        self.recenter()
    }
    /// Sets the clip on the sphere.
    pub fn set_clip(&mut self, c: Clip) -> &mut Self {
        self.clip = c;
        self
    }
    /// Sets the planar clip rectangle ([[x0, y0], [x1, y1]]).
    pub fn set_extent(&mut self, e: Option<[[f64; 2]; 2]>) -> &mut Self {
        self.extent = e.map(|e| Rect { x0: e[0][0], y0: e[0][1], x1: e[1][0], y1: e[1][1] });
        self
    }
    /// Sets the resampling precision in pixels (0 turns resampling off).
    pub fn set_precision(&mut self, px: f64) -> &mut Self {
        self.delta2 = px * px;
        self
    }

    fn st(&self, x: f64, y: f64, dx: f64, dy: f64) -> [f64; 2] {
        let (x, y) = (x * self.reflect[0], y * self.reflect[1]);
        if self.alpha == 0.0 {
            [dx + self.k * x, dy - self.k * y]
        } else {
            let (a, b) = (self.alpha.cos() * self.k, self.alpha.sin() * self.k);
            [a * x - b * y + dx, dy - b * x - a * y]
        }
    }

    fn recenter(&mut self) -> &mut Self {
        let c = self.raw.forward(self.center[0], self.center[1]);
        let c = self.st(c[0], c[1], 0.0, 0.0);
        self.dx = self.tx - c[0];
        self.dy = self.ty - c[1];
        let [l, p, g] = self.rotate_deg.map(|v| v * RAD);
        self.rotation = Rotation::new(l, p, g);
        self.ta = self.alpha.cos();
        self.tb = self.alpha.sin();
        self
    }

    /// Projects a rotated point (radians) to the plane.
    fn transform(&self, l: f64, p: f64) -> [f64; 2] {
        let r = self.raw.forward(l, p);
        self.st(r[0], r[1], self.dx, self.dy)
    }

    /// The planar clip in force: the explicit extent, narrowed for Mercator to the square
    /// world (d3 `mercatorProjection`'s `reclip`).
    fn postclip(&self) -> Option<Rect> {
        if self.raw != Raw::Mercator {
            return self.extent;
        }
        // The square world round the projection of the point the rotation brings to [0, 0].
        let k = PI * self.k;
        let t = self.transform(0.0, 0.0);
        Some(match self.extent {
            None => Rect { x0: t[0] - k, y0: t[1] - k, x1: t[0] + k, y1: t[1] + k },
            Some(e) => Rect { x0: (t[0] - k).max(e.x0), y0: e.y0, x1: (t[0] + k).min(e.x1), y1: e.y1 },
        })
    }

    /// Projects a point (degrees), or `None` when it is clipped.
    pub fn point(&self, lon: f64, lat: f64) -> Option<[f64; 2]> {
        let r = self.rotation.apply([lon * RAD, lat * RAD]);
        if !self.clip.visible(r) {
            return None;
        }
        let q = self.transform(r[0], r[1]);
        match self.postclip() {
            Some(e) if !(e.x0 <= q[0] && q[0] <= e.x1 && e.y0 <= q[1] && q[1] <= e.y1) => None,
            _ => Some(q),
        }
    }

    /// Projects a point (degrees) ignoring clipping.
    pub fn point_unclipped(&self, lon: f64, lat: f64) -> [f64; 2] {
        let r = self.rotation.apply([lon * RAD, lat * RAD]);
        self.transform(r[0], r[1])
    }

    /// Resamples and projects one line or ring (rotated radians).
    fn resample(&self, pts: &[Ll], ring: bool) -> Vec<[f64; 2]> {
        let mut out = Vec::with_capacity(pts.len());
        if self.delta2 <= 0.0 {
            out.extend(pts.iter().map(|p| self.transform(p[0], p[1])));
            return out;
        }
        let node = |p: Ll| {
            let c = cartesian(p);
            let q = self.transform(p[0], p[1]);
            (q, p[0], c)
        };
        let mut prev: Option<([f64; 2], f64, [f64; 3])> = None;
        let mut first = None;
        for &p in pts {
            let n = node(p);
            if let Some(pr) = prev {
                self.resample_to(pr, n, 16, &mut out);
            }
            out.push(n.0);
            prev = Some(n);
            first.get_or_insert(n);
        }
        if ring {
            if let (Some(pr), Some(f)) = (prev, first) {
                self.resample_to(pr, f, 16, &mut out);
            }
        }
        out
    }

    #[allow(clippy::type_complexity)]
    fn resample_to(
        &self,
        (p0, l0, c0): ([f64; 2], f64, [f64; 3]),
        (p1, l1, c1): ([f64; 2], f64, [f64; 3]),
        depth: u32,
        out: &mut Vec<[f64; 2]>,
    ) {
        let (dx, dy) = (p1[0] - p0[0], p1[1] - p0[1]);
        let d2 = dx * dx + dy * dy;
        if !(d2 > 4.0 * self.delta2 && depth > 0) {
            return;
        }
        let depth = depth - 1;
        let (a, b, c) = (c0[0] + c1[0], c0[1] + c1[1], c0[2] + c1[2]);
        let m = (a * a + b * b + c * c).sqrt();
        let c = c / m;
        let phi2 = asin(c);
        let l2 = if ((c.abs()) - 1.0).abs() < EPS || (l0 - l1).abs() < EPS { (l0 + l1) / 2.0 } else { b.atan2(a) };
        let p2 = self.transform(l2, phi2);
        let (dx2, dy2) = (p2[0] - p0[0], p2[1] - p0[1]);
        let dz = dy * dx2 - dx * dy2;
        const COS_MIN_DISTANCE: f64 = 0.8660254037844387; // cos 30°
        if dz * dz / d2 > self.delta2
            || ((dx * dx2 + dy * dy2) / d2 - 0.5).abs() > 0.3
            || c0[0] * c1[0] + c0[1] * c1[1] + c0[2] * c1[2] < COS_MIN_DISTANCE
        {
            let mid = (p2, l2, [a / m, b / m, c]);
            self.resample_to((p0, l0, c0), mid, depth, out);
            out.push(p2);
            self.resample_to(mid, (p1, l1, c1), depth, out);
        }
    }

    fn rotated(&self, pts: &[[f64; 2]]) -> Vec<Ll> {
        pts.iter().map(|p| self.rotation.apply([p[0] * RAD, p[1] * RAD])).collect()
    }

    /// Projects a geometry (degrees).
    pub fn project(&self, g: &Geometry) -> Planar {
        let mut out = Planar::default();
        let post = self.postclip();
        match g {
            Geometry::Points(ps) => {
                out.points.extend(ps.iter().filter_map(|p| {
                    let r = self.rotation.apply([p[0] * RAD, p[1] * RAD]);
                    if !self.clip.visible(r) {
                        return None;
                    }
                    let q = self.transform(r[0], r[1]);
                    match post {
                        Some(e) if !(e.x0 <= q[0] && q[0] <= e.x1 && e.y0 <= q[1] && q[1] <= e.y1) => None,
                        _ => Some(q),
                    }
                }));
            }
            Geometry::Lines(ls) => {
                for l in ls {
                    for piece in self.clip.clip_line(&self.rotated(l)) {
                        let q = self.resample(&piece, false);
                        match post {
                            Some(e) => out.lines.extend(e.clip_line(&q)),
                            None => out.lines.push(q),
                        }
                    }
                }
            }
            Geometry::Polygons(polys) => {
                for rings in polys {
                    let rings: Vec<Vec<Ll>> = rings.iter().map(|r| self.rotated(open_ring(r))).collect();
                    let clipped = self.clip.clip_polygon(&rings);
                    if clipped.is_empty() {
                        continue;
                    }
                    let q: Vec<Vec<[f64; 2]>> = clipped.iter().map(|r| self.resample(r, true)).collect();
                    let q = match post {
                        Some(e) => e.clip_polygon(&q),
                        None => q,
                    };
                    if !q.is_empty() {
                        out.polygons.push(q);
                    }
                }
            }
            Geometry::Sphere => {
                let q = vec![self.resample(&self.clip.sphere(), true)];
                let q = match post {
                    Some(e) => e.clip_polygon(&q),
                    None => q,
                };
                if !q.is_empty() {
                    out.polygons.push(q);
                }
            }
            Geometry::Collection(gs) => {
                for g in gs {
                    out.extend(self.project(g));
                }
            }
        }
        out
    }

    /// d3 `fitExtent`: sets scale and translate so `geometries` fill `extent` ([[x0, y0], [x1, y1]]).
    pub fn fit_extent(&mut self, extent: [[f64; 2]; 2], geometries: &[&Geometry]) -> &mut Self {
        let saved = self.extent.take();
        self.set_scale(150.0).set_translate(0.0, 0.0);
        let mut b: Option<[[f64; 2]; 2]> = None;
        for g in geometries {
            if let Some(gb) = self.project(g).bounds() {
                b = Some(match b {
                    None => gb,
                    Some(b) => {
                        [[b[0][0].min(gb[0][0]), b[0][1].min(gb[0][1])], [b[1][0].max(gb[1][0]), b[1][1].max(gb[1][1])]]
                    }
                });
            }
        }
        if let Some(b) = b {
            let (w, h) = (extent[1][0] - extent[0][0], extent[1][1] - extent[0][1]);
            let k = (w / (b[1][0] - b[0][0])).min(h / (b[1][1] - b[0][1]));
            let x = extent[0][0] + (w - k * (b[1][0] + b[0][0])) / 2.0;
            let y = extent[0][1] + (h - k * (b[1][1] + b[0][1])) / 2.0;
            self.set_scale(150.0 * k).set_translate(x, y);
        }
        self.extent = saved;
        self
    }

    /// The pixel position of the centre.
    pub fn translate(&self) -> [f64; 2] {
        [self.tx, self.ty]
    }

    /// The sphere's rotation in degrees.
    pub fn rotate(&self) -> [f64; 3] {
        self.rotate_deg
    }

    /// Whether a point (degrees) is on the visible side of the clip (the near hemisphere of a globe).
    pub fn visible(&self, lon: f64, lat: f64) -> bool {
        self.clip.visible(self.rotation.apply([lon * RAD, lat * RAD]))
    }
}

/// A ring without its closing point (GeoJSON repeats the first point; d3 streams n − 1 points).
fn open_ring(r: &[[f64; 2]]) -> &[[f64; 2]] {
    match (r.first(), r.last()) {
        (Some(a), Some(b)) if r.len() > 1 && a == b => &r[..r.len() - 1],
        _ => r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mercator_matches_the_textbook_formula() {
        let mut p = Projection::new(Raw::Mercator);
        p.set_scale(100.0).set_translate(0.0, 0.0);
        let q = p.point(90.0, 45.0).unwrap();
        assert!((q[0] - 100.0 * FRAC_PI_2).abs() < 1e-9);
        assert!((q[1] + 100.0 * (PI / 4.0 + PI / 8.0).tan().ln()).abs() < 1e-9);
    }
}
