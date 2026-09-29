//! Clipping on the sphere, before projection: cutting along the antimeridian
//! (cylindrical, pseudo-cylindrical and conic projections) or to a small
//! circle around the view centre (orthographic and the azimuthal
//! projections). Polygons cut into pieces are rejoined along the clip edge,
//! so filled areas stay closed; a polygon containing a pole wraps round it.
//!
//! Ported from d3-geo 3.1 (`clip/*.js`, ISC licence, © Mike Bostock); d3's
//! streams become functions over point slices, with the same arithmetic.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use crate::sphere::{self, acos, cartesian, cross, dot, normalize, point_equal, spherical, Ll, EPS, RAD};

/// A point with d3's third component: 0, or a mark for the rejoin (1 degenerate, 2 exit, 3 re-entry).
type Pt = [f64; 3];

/// Collected pieces of a clipped line (d3's `clipBuffer`).
#[derive(Default)]
struct Pieces {
    lines: Vec<Vec<Pt>>,
}

impl Pieces {
    fn start(&mut self) {
        self.lines.push(Vec::new());
    }
    fn point(&mut self, p: Pt) {
        if self.lines.is_empty() {
            self.start();
        }
        self.lines.last_mut().expect("started").push(p);
    }
}

/// A clip region on the (rotated) sphere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Clip {
    /// Cut along λ = ±π.
    Antimeridian,
    /// Keep the points within `radius` (radians) of [0, 0].
    Circle(f64),
}

impl Clip {
    /// Whether a point survives the clip.
    pub fn visible(&self, p: Ll) -> bool {
        match *self {
            Clip::Antimeridian => true,
            Clip::Circle(r) => p[0].cos() * p[1].cos() > r.cos(),
        }
    }

    fn start(&self) -> Ll {
        match *self {
            Clip::Antimeridian => [-PI, -FRAC_PI_2],
            Clip::Circle(r) => {
                if r.cos() > 0.0 {
                    [0.0, -r]
                } else {
                    [-PI, r - PI]
                }
            }
        }
    }

    /// Cuts a line into its visible pieces. Returns d3's `clean` code: bit 0 when there were no
    /// intersections, bit 1 when the first and last pieces join.
    fn line(&self, pts: &[Ll], out: &mut Pieces) -> u8 {
        match *self {
            Clip::Antimeridian => antimeridian_line(pts, out),
            Clip::Circle(r) => CircleClip::new(r).line(pts, out),
        }
    }

    /// Points along the clip edge from `from` to `to` (both `None`: the whole edge).
    fn interpolate(&self, from: Option<Pt>, to: Option<Pt>, direction: f64, out: &mut Vec<Ll>) {
        match *self {
            Clip::Antimeridian => antimeridian_interpolate(from, to, direction, out),
            Clip::Circle(r) => circle_stream(out, r, 2.0 * RAD, direction, from, to),
        }
    }

    /// Clips a line string: its visible pieces.
    pub fn clip_line(&self, pts: &[Ll]) -> Vec<Vec<Ll>> {
        let mut out = Pieces::default();
        self.line(pts, &mut out);
        out.lines.into_iter().filter(|l| l.len() > 1).map(|l| l.iter().map(|p| [p[0], p[1]]).collect()).collect()
    }

    /// The outline of the clip region as one ring (d3's `Sphere`).
    pub fn sphere(&self) -> Vec<Ll> {
        let mut ring = Vec::new();
        self.interpolate(None, None, 1.0, &mut ring);
        ring
    }

    /// Clips a polygon (rings without their closing point): the rings to fill.
    pub fn clip_polygon(&self, rings: &[Vec<Ll>]) -> Vec<Vec<Ll>> {
        let mut out: Vec<Vec<Ll>> = Vec::new();
        let mut segments: Vec<Vec<Pt>> = Vec::new();
        let mut polygon: Vec<Vec<Ll>> = Vec::new();
        for ring in rings {
            if ring.is_empty() {
                continue;
            }
            let mut closed = ring.clone();
            closed.push(ring[0]);
            let mut pieces = Pieces::default();
            let clean = self.line(&closed, &mut pieces);
            polygon.push(ring.clone());
            let mut ring_segments = pieces.lines;
            let n = ring_segments.len();
            if n == 0 {
                continue;
            }
            if clean & 1 != 0 {
                let seg = &ring_segments[0];
                if seg.len() > 1 {
                    out.push(seg[..seg.len() - 1].iter().map(|p| [p[0], p[1]]).collect());
                }
                continue;
            }
            if n > 1 && clean & 2 != 0 {
                let last = ring_segments.pop().expect("n > 1");
                let first = ring_segments.remove(0);
                ring_segments.push([last, first].concat());
            }
            segments.extend(ring_segments.into_iter().filter(|s| s.len() > 1));
        }
        let start_inside = sphere::polygon_contains(&polygon, self.start());
        if !segments.is_empty() {
            self.rejoin(segments, start_inside, &mut out);
        } else if start_inside {
            out.push(self.sphere());
        }
        out
    }

    fn rejoin(&self, segments: Vec<Vec<Pt>>, start_inside: bool, out: &mut Vec<Vec<Ll>>) {
        let key = |x: &Pt| if x[0] < 0.0 { x[1] - FRAC_PI_2 - EPS } else { FRAC_PI_2 - x[1] };
        rejoin(segments, &|a, b| key(a) - key(b), start_inside, &|f, t, d, o| self.interpolate(f, t, d, o), out);
    }
}

type Compare<'a> = &'a dyn Fn(&Pt, &Pt) -> f64;
type Interpolate<'a> = &'a dyn Fn(Option<Pt>, Option<Pt>, f64, &mut Vec<Ll>);

/// d3's `clipRejoin`: links the cut segments through their intersections with the clip edge,
/// sorted along it by `compare`, and walks the resulting rings, following the edge with
/// `interpolate` between segments.
fn rejoin(
    segments: Vec<Vec<Pt>>,
    compare: Compare,
    mut start_inside: bool,
    interpolate: Interpolate,
    out: &mut Vec<Vec<Ll>>,
) {
    struct X {
        x: Pt,
        z: Option<usize>,
        o: usize,
        e: bool,
        v: bool,
        n: usize,
        p: usize,
    }
    let mut segs = segments;
    let mut nodes: Vec<X> = Vec::new();
    let (mut subject, mut clip): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
    for (si, seg) in segs.iter_mut().enumerate() {
        let n = seg.len().saturating_sub(1);
        if n == 0 {
            continue;
        }
        let (p0, mut p1) = (seg[0], seg[n]);
        if point_equal(&p0, &p1) {
            if p0[2] == 0.0 && p1[2] == 0.0 {
                out.push(seg[..n].iter().map(|p| [p[0], p[1]]).collect());
                continue;
            }
            // Degenerate: move the end point.
            p1[0] += 2.0 * EPS;
            seg[n][0] = p1[0];
        }
        let mut add = |x: Pt, z: Option<usize>, e: bool| {
            nodes.push(X { x, z, o: 0, e, v: false, n: 0, p: 0 });
            nodes.len() - 1
        };
        let a = add(p0, Some(si), true);
        let b = add(p0, None, false);
        let c = add(p1, Some(si), false);
        let d = add(p1, None, true);
        nodes[a].o = b;
        nodes[b].o = a;
        nodes[c].o = d;
        nodes[d].o = c;
        subject.extend([a, c]);
        clip.extend([b, d]);
    }
    if subject.is_empty() {
        return;
    }
    // JavaScript's sort is stable; so is `sort_by`.
    clip.sort_by(|&a, &b| compare(&nodes[a].x, &nodes[b].x).partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal));
    for list in [&subject, &clip] {
        let n = list.len();
        for i in 0..n {
            let (a, b) = (list[i], list[(i + 1) % n]);
            nodes[a].n = b;
            nodes[b].p = a;
        }
    }
    for &c in &clip {
        start_inside = !start_inside;
        nodes[c].e = start_inside;
    }
    let start = subject[0];
    loop {
        let mut current = start;
        let mut is_subject = true;
        while nodes[current].v {
            current = nodes[current].n;
            if current == start {
                return;
            }
        }
        let mut points = nodes[current].z;
        let mut ring: Vec<Ll> = Vec::new();
        loop {
            let o = nodes[current].o;
            nodes[current].v = true;
            nodes[o].v = true;
            if nodes[current].e {
                if is_subject {
                    if let Some(z) = points {
                        ring.extend(segs[z].iter().map(|p| [p[0], p[1]]));
                    }
                } else {
                    let next = nodes[current].n;
                    interpolate(Some(nodes[current].x), Some(nodes[next].x), 1.0, &mut ring);
                }
                current = nodes[current].n;
            } else {
                let prev = nodes[current].p;
                if is_subject {
                    if let Some(z) = nodes[prev].z {
                        ring.extend(segs[z].iter().rev().map(|p| [p[0], p[1]]));
                    }
                } else {
                    interpolate(Some(nodes[current].x), Some(nodes[prev].x), -1.0, &mut ring);
                }
                current = prev;
            }
            current = nodes[current].o;
            points = nodes[current].z;
            is_subject = !is_subject;
            if nodes[current].v {
                break;
            }
        }
        out.push(ring);
    }
}

/// A planar clip rectangle, after projection (d3 `clipRectangle`, `clipExtent`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    /// Left, top, right, bottom.
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

const CLIP_MAX: f64 = 1e9;

impl Rect {
    fn visible(&self, x: f64, y: f64) -> bool {
        self.x0 <= x && x <= self.x1 && self.y0 <= y && y <= self.y1
    }

    fn corner(&self, p: &Pt, direction: f64) -> i32 {
        if (p[0] - self.x0).abs() < EPS {
            if direction > 0.0 {
                0
            } else {
                3
            }
        } else if (p[0] - self.x1).abs() < EPS {
            if direction > 0.0 {
                2
            } else {
                1
            }
        } else if (p[1] - self.y0).abs() < EPS {
            if direction > 0.0 {
                1
            } else {
                0
            }
        } else if direction > 0.0 {
            3
        } else {
            2
        }
    }

    fn compare_point(&self, a: &Pt, b: &Pt) -> f64 {
        let (ca, cb) = (self.corner(a, 1.0), self.corner(b, 1.0));
        if ca != cb {
            (ca - cb) as f64
        } else {
            match ca {
                0 => b[1] - a[1],
                1 => a[0] - b[0],
                2 => a[1] - b[1],
                _ => b[0] - a[0],
            }
        }
    }

    fn interpolate(&self, from: Option<Pt>, to: Option<Pt>, direction: f64, out: &mut Vec<Ll>) {
        let d = direction as i32;
        let (mut a, mut a1) = (0, 0);
        let walk = match (from, to) {
            (Some(f), Some(t)) => {
                a = self.corner(&f, direction);
                a1 = self.corner(&t, direction);
                a != a1 || ((self.compare_point(&f, &t) < 0.0) ^ (direction > 0.0))
            }
            _ => true,
        };
        if walk {
            loop {
                out.push([if a == 0 || a == 3 { self.x0 } else { self.x1 }, if a > 1 { self.y1 } else { self.y0 }]);
                a = (a + d + 4) % 4;
                if a == a1 {
                    break;
                }
            }
        } else if let Some(t) = to {
            out.push([t[0], t[1]]);
        }
    }

    /// Liang–Barsky: shortens a–b to the rectangle; false when it misses.
    fn clip_segment(&self, a: &mut [f64; 2], b: &mut [f64; 2]) -> bool {
        let (ax, ay) = (a[0], a[1]);
        let (dx, dy) = (b[0] - ax, b[1] - ay);
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        for (r, d, lower) in
            [(self.x0 - ax, dx, true), (self.x1 - ax, dx, false), (self.y0 - ay, dy, true), (self.y1 - ay, dy, false)]
        {
            if d == 0.0 && if lower { r > 0.0 } else { r < 0.0 } {
                return false;
            }
            let r = r / d;
            if lower {
                if d < 0.0 {
                    if r < t0 {
                        return false;
                    }
                    if r < t1 {
                        t1 = r;
                    }
                } else if d > 0.0 {
                    if r > t1 {
                        return false;
                    }
                    if r > t0 {
                        t0 = r;
                    }
                }
            } else if d < 0.0 {
                if r > t1 {
                    return false;
                }
                if r > t0 {
                    t0 = r;
                }
            } else if d > 0.0 {
                if r < t0 {
                    return false;
                }
                if r < t1 {
                    t1 = r;
                }
            }
        }
        if t0 > 0.0 {
            *a = [ax + t0 * dx, ay + t0 * dy];
        }
        if t1 < 1.0 {
            *b = [ax + t1 * dx, ay + t1 * dy];
        }
        true
    }

    /// Feeds one line (closing it when `ring`), d3's `linePoint` state machine; `clean` turns
    /// false at any crossing.
    fn feed(&self, pts: &[Ll], ring: bool, out: &mut Pieces, clean: &mut bool) {
        let mut first: Option<(Ll, bool)> = None;
        let (mut xp, mut yp, mut vp) = (f64::NAN, f64::NAN, false);
        let closing = if ring { pts.first().copied() } else { None };
        for &[mut x, mut y] in pts.iter().chain(closing.iter()) {
            let v = self.visible(x, y);
            if first.is_none() {
                first = Some(([x, y], v));
                if v {
                    out.start();
                    out.point([x, y, 0.0]);
                }
            } else if v && vp {
                out.point([x, y, 0.0]);
            } else {
                xp = xp.clamp(-CLIP_MAX, CLIP_MAX);
                yp = yp.clamp(-CLIP_MAX, CLIP_MAX);
                x = x.clamp(-CLIP_MAX, CLIP_MAX);
                y = y.clamp(-CLIP_MAX, CLIP_MAX);
                let (mut a, mut b) = ([xp, yp], [x, y]);
                if self.clip_segment(&mut a, &mut b) {
                    if !vp {
                        out.start();
                        out.point([a[0], a[1], 0.0]);
                    }
                    out.point([b[0], b[1], 0.0]);
                    if !v {
                        // lineEnd: nothing to do for collected pieces
                    }
                    *clean = false;
                } else if v {
                    out.start();
                    out.point([x, y, 0.0]);
                    *clean = false;
                }
            }
            (xp, yp, vp) = (x, y, v);
        }
        if ring {
            if let Some((_, v_first)) = first {
                if v_first && vp && out.lines.len() > 1 {
                    let last = out.lines.pop().expect("len > 1");
                    let head = out.lines.remove(0);
                    out.lines.push([last, head].concat());
                }
            }
        }
    }

    /// Clips a projected line: its pieces inside the rectangle.
    pub fn clip_line(&self, pts: &[Ll]) -> Vec<Vec<Ll>> {
        let mut out = Pieces::default();
        let mut clean = true;
        self.feed(pts, false, &mut out, &mut clean);
        out.lines.into_iter().filter(|l| l.len() > 1).map(|l| l.iter().map(|p| [p[0], p[1]]).collect()).collect()
    }

    /// Clips a projected polygon (rings without closing points).
    pub fn clip_polygon(&self, rings: &[Vec<Ll>]) -> Vec<Vec<Ll>> {
        let mut segments: Vec<Vec<Pt>> = Vec::new();
        let mut clean = true;
        let mut winding = 0i64;
        for ring in rings.iter().filter(|r| !r.is_empty()) {
            let mut pieces = Pieces::default();
            self.feed(ring, true, &mut pieces, &mut clean);
            segments.extend(pieces.lines);
            // d3's polygonInside: winding of the ring round the top-left corner's ray.
            let closed: Vec<Ll> = ring.iter().chain(std::iter::once(&ring[0])).copied().collect();
            for w in closed.windows(2) {
                let ([a0, a1], [b0, b1]) = (w[0], w[1]);
                if a1 <= self.y1 {
                    if b1 > self.y1 && (b0 - a0) * (self.y1 - a1) > (b1 - a1) * (self.x0 - a0) {
                        winding += 1;
                    }
                } else if b1 <= self.y1 && (b0 - a0) * (self.y1 - a1) < (b1 - a1) * (self.x0 - a0) {
                    winding -= 1;
                }
            }
        }
        let mut out = Vec::new();
        let start_inside = winding != 0;
        if clean && start_inside {
            let mut r = Vec::new();
            self.interpolate(None, None, 1.0, &mut r);
            out.push(r);
        }
        if !segments.is_empty() {
            rejoin(
                segments,
                &|a, b| self.compare_point(a, b),
                start_inside,
                &|f, t, d, o| self.interpolate(f, t, d, o),
                &mut out,
            );
        }
        out
    }
}

fn antimeridian_line(pts: &[Ll], out: &mut Pieces) -> u8 {
    let (mut lambda0, mut phi0, mut sign0) = (f64::NAN, f64::NAN, f64::NAN);
    let mut clean = 1u8;
    out.start();
    for &[mut lambda1, phi1] in pts {
        let sign1 = if lambda1 > 0.0 { PI } else { -PI };
        let delta = (lambda1 - lambda0).abs();
        if (delta - PI).abs() < EPS {
            // The line crosses a pole.
            phi0 = if (phi0 + phi1) / 2.0 > 0.0 { FRAC_PI_2 } else { -FRAC_PI_2 };
            out.point([lambda0, phi0, 0.0]);
            out.point([sign0, phi0, 0.0]);
            out.start();
            out.point([sign1, phi0, 0.0]);
            out.point([lambda1, phi0, 0.0]);
            clean = 0;
        } else if sign0 != sign1 && delta >= PI {
            // The line crosses the antimeridian.
            if (lambda0 - sign0).abs() < EPS {
                lambda0 -= sign0 * EPS;
            }
            if (lambda1 - sign1).abs() < EPS {
                lambda1 -= sign1 * EPS;
            }
            phi0 = antimeridian_intersect(lambda0, phi0, lambda1, phi1);
            out.point([sign0, phi0, 0.0]);
            out.start();
            out.point([sign1, phi0, 0.0]);
            clean = 0;
        }
        lambda0 = lambda1;
        phi0 = phi1;
        out.point([lambda0, phi0, 0.0]);
        sign0 = sign1;
    }
    2 - clean
}

fn antimeridian_intersect(lambda0: f64, phi0: f64, lambda1: f64, phi1: f64) -> f64 {
    let s = (lambda0 - lambda1).sin();
    if s.abs() > EPS {
        let (c0, c1) = (phi0.cos(), phi1.cos());
        ((phi0.sin() * c1 * lambda1.sin() - phi1.sin() * c0 * lambda0.sin()) / (c0 * c1 * s)).atan()
    } else {
        (phi0 + phi1) / 2.0
    }
}

fn antimeridian_interpolate(from: Option<Pt>, to: Option<Pt>, direction: f64, out: &mut Vec<Ll>) {
    match (from, to) {
        (Some(f), Some(t)) if (f[0] - t[0]).abs() > EPS => {
            let lambda = if f[0] < t[0] { PI } else { -PI };
            let phi = direction * lambda / 2.0;
            out.extend([[-lambda, phi], [0.0, phi], [lambda, phi]]);
        }
        (Some(_), Some(t)) => out.push([t[0], t[1]]),
        _ => {
            let phi = direction * FRAC_PI_2;
            out.extend([
                [-PI, phi],
                [0.0, phi],
                [PI, phi],
                [PI, 0.0],
                [PI, -phi],
                [0.0, -phi],
                [-PI, -phi],
                [-PI, 0.0],
                [-PI, phi],
            ]);
        }
    }
}

/// Points of the small circle of `radius` around [0, 0], stepping by `delta` (d3 `circleStream`).
fn circle_stream(out: &mut Vec<Ll>, radius: f64, delta: f64, direction: f64, from: Option<Pt>, to: Option<Pt>) {
    let (cr, sr) = (radius.cos(), radius.sin());
    let step = direction * delta;
    let (mut t0, t1) = match (from, to) {
        (Some(f), Some(t)) => {
            let (mut t0, t1) = (circle_radius(cr, f), circle_radius(cr, t));
            if if direction > 0.0 { t0 < t1 } else { t0 > t1 } {
                t0 += direction * TAU;
            }
            (t0, t1)
        }
        _ => (radius + direction * TAU, radius - step / 2.0),
    };
    while if direction > 0.0 { t0 > t1 } else { t0 < t1 } {
        out.push(spherical([cr, -sr * t0.cos(), -sr * t0.sin()]));
        t0 -= step;
    }
}

fn circle_radius(cr: f64, p: Pt) -> f64 {
    let mut c = cartesian([p[0], p[1]]);
    c[0] -= cr;
    let c = normalize(c);
    let r = acos(-c[1]);
    ((if -c[2] < 0.0 { -r } else { r }) + TAU - EPS) % TAU
}

struct CircleClip {
    radius: f64,
    cr: f64,
    small: bool,
    not_hemisphere: bool,
}

impl CircleClip {
    fn new(radius: f64) -> CircleClip {
        let cr = radius.cos();
        CircleClip { radius, cr, small: cr > 0.0, not_hemisphere: cr.abs() > EPS }
    }

    fn visible(&self, l: f64, p: f64) -> bool {
        l.cos() * p.cos() > self.cr
    }

    fn code(&self, l: f64, p: f64) -> u8 {
        let r = if self.small { self.radius } else { PI - self.radius };
        let mut c = 0;
        if l < -r {
            c |= 1;
        } else if l > r {
            c |= 2;
        }
        if p < -r {
            c |= 4;
        } else if p > r {
            c |= 8;
        }
        c
    }

    /// Intersections of the great circle through `a` and `b` with the clip circle: the first
    /// (`two` = false; `Some(a)` for two polar points), or both when they lie between a and b.
    fn intersect(&self, a: Ll, b: Ll, two: bool) -> Option<[Ll; 2]> {
        let (pa, pb) = (cartesian(a), cartesian(b));
        let n1 = [1.0, 0.0, 0.0];
        let n2 = cross(pa, pb);
        let n2n2 = dot(n2, n2);
        let n1n2 = n2[0];
        let det = n2n2 - n1n2 * n1n2;
        if det == 0.0 {
            return (!two).then_some([a, a]);
        }
        let c1 = self.cr * n2n2 / det;
        let c2 = -self.cr * n1n2 / det;
        let n1xn2 = cross(n1, n2);
        let aa = [n1[0] * c1 + n2[0] * c2, n1[1] * c1 + n2[1] * c2, n1[2] * c1 + n2[2] * c2];
        let u = n1xn2;
        let w = dot(aa, u);
        let uu = dot(u, u);
        let t2 = w * w - uu * (dot(aa, aa) - 1.0);
        if t2 < 0.0 {
            return None;
        }
        let t = t2.sqrt();
        let k = (-w - t) / uu;
        let q = spherical([u[0] * k + aa[0], u[1] * k + aa[1], u[2] * k + aa[2]]);
        if !two {
            return Some([q, q]);
        }
        let (mut l0, mut l1, mut p0, mut p1) = (a[0], b[0], a[1], b[1]);
        if l1 < l0 {
            std::mem::swap(&mut l0, &mut l1);
        }
        let delta = l1 - l0;
        let polar = (delta - PI).abs() < EPS;
        let meridian = polar || delta < EPS;
        if !polar && p1 < p0 {
            std::mem::swap(&mut p0, &mut p1);
        }
        let between = if meridian {
            if polar {
                (p0 + p1 > 0.0) ^ (q[1] < if (q[0] - l0).abs() < EPS { p0 } else { p1 })
            } else {
                p0 <= q[1] && q[1] <= p1
            }
        } else {
            (delta > PI) ^ (l0 <= q[0] && q[0] <= l1)
        };
        if between {
            let k = (-w + t) / uu;
            let q1 = spherical([u[0] * k + aa[0], u[1] * k + aa[1], u[2] * k + aa[2]]);
            return Some([q, q1]);
        }
        None
    }

    fn line(&self, pts: &[Ll], out: &mut Pieces) -> u8 {
        let mut point0: Option<Pt> = None;
        let (mut c0, mut v0, mut v00) = (0u8, false, false);
        let mut clean = 1u8;
        for &[l, p] in pts {
            let mut point1: Pt = [l, p, 0.0];
            let v = self.visible(l, p);
            let c = if self.small {
                if v {
                    0
                } else {
                    self.code(l, p)
                }
            } else if v {
                self.code(l + if l < 0.0 { PI } else { -PI }, p)
            } else {
                0
            };
            if point0.is_none() {
                v0 = v;
                v00 = v;
                if v {
                    out.start();
                }
            }
            let ll = |q: &Pt| [q[0], q[1]];
            if v != v0 {
                let p0 = point0.expect("v changed after the first point");
                match self.intersect(ll(&p0), ll(&point1), false) {
                    Some([q, _]) if !point_equal(&p0, &q) && !point_equal(&point1, &q) => {}
                    _ => point1[2] = 1.0,
                }
            }
            if v != v0 {
                let p0 = point0.expect("v changed after the first point");
                clean = 0;
                let q = if v {
                    // outside going in
                    out.start();
                    let q = self.intersect(ll(&point1), ll(&p0), false).map(|r| r[0]).unwrap_or(ll(&point1));
                    out.point([q[0], q[1], 0.0]);
                    q
                } else {
                    // inside going out
                    let q = self.intersect(ll(&p0), ll(&point1), false).map(|r| r[0]).unwrap_or(ll(&p0));
                    out.point([q[0], q[1], 2.0]);
                    q
                };
                point0 = Some([q[0], q[1], 0.0]);
            } else if self.not_hemisphere && point0.is_some() && (self.small ^ v) {
                let p0 = point0.expect("checked");
                if c & c0 == 0 {
                    if let Some([t0, t1]) = self.intersect(ll(&point1), ll(&p0), true) {
                        clean = 0;
                        if self.small {
                            out.start();
                            out.point([t0[0], t0[1], 0.0]);
                            out.point([t1[0], t1[1], 0.0]);
                        } else {
                            out.point([t1[0], t1[1], 0.0]);
                            out.start();
                            out.point([t0[0], t0[1], 3.0]);
                        }
                    }
                }
            }
            if v && point0.is_none_or(|p0| !point_equal(&p0, &point1)) {
                out.point(point1);
            }
            point0 = Some(point1);
            v0 = v;
            c0 = c;
        }
        clean | (((v00 && v0) as u8) << 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_split_at_the_antimeridian() {
        let pieces = Clip::Antimeridian.clip_line(&[[170.0 * RAD, 0.0], [-170.0 * RAD, 0.0]]);
        assert_eq!(pieces.len(), 2);
        assert!((pieces[0][1][0] - PI).abs() < 1e-12 && (pieces[1][0][0] + PI).abs() < 1e-12);
    }

    #[test]
    fn a_polygon_round_the_south_pole_fills_to_the_edge() {
        // A clockwise ring round the south pole along φ = -60°, crossing the antimeridian.
        let ring: Vec<Ll> = (0..36).map(|i| [PI - (i as f64 * 10.0) * RAD, -60.0 * RAD]).collect();
        let out = Clip::Antimeridian.clip_polygon(std::slice::from_ref(&ring));
        // d3-geo: one ring of 42 points, either winding.
        assert_eq!(out.iter().map(Vec::len).collect::<Vec<_>>(), vec![42]);
        let rev: Vec<Ll> = ring.into_iter().rev().collect();
        assert_eq!(Clip::Antimeridian.clip_polygon(&[rev]).iter().map(Vec::len).collect::<Vec<_>>(), vec![42]);
    }

    #[test]
    fn circle_clip_hides_the_far_side() {
        let c = Clip::Circle(FRAC_PI_2);
        assert!(c.visible([0.0, 0.0]) && !c.visible([PI, 0.0]));
        let pieces = c.clip_line(&[[0.0, 0.0], [100.0 * RAD, 0.0]]);
        assert_eq!(pieces.len(), 1);
        assert!((pieces[0].last().unwrap()[0] - FRAC_PI_2).abs() < 1e-9);
        assert!(c.sphere().len() > 170);
    }
}
