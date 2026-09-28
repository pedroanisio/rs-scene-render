//! Paths: SVG path data, Bézier contours with in/out tangents (the Lottie
//! and After Effects vertex form), flattening to polylines within a
//! tolerance, and transforms.

use crate::geom::{p, Rect, Xf, P};

/// A path segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seg {
    /// Starts a subpath.
    Move(P),
    /// Straight line.
    Line(P),
    /// Quadratic Bézier (control, end).
    Quad(P, P),
    /// Cubic Bézier (control 1, control 2, end).
    Cubic(P, P, P),
    /// Closes the subpath.
    Close,
}

/// A path: one or more subpaths.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Path {
    pub segs: Vec<Seg>,
}

/// A flattened subpath.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Poly {
    pub pts: Vec<P>,
    pub closed: bool,
}

/// A subpath in vertex form: vertices with absolute in and out control points.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Contour {
    pub v: Vec<P>,
    pub i: Vec<P>,
    pub o: Vec<P>,
    pub closed: bool,
}

/// Path data errors.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("path data at byte {at}: {message}")]
pub struct PathError {
    pub at: usize,
    pub message: String,
}

impl Path {
    /// Starts a subpath.
    pub fn move_to(&mut self, q: P) {
        self.segs.push(Seg::Move(q));
    }
    /// Adds a line.
    pub fn line_to(&mut self, q: P) {
        self.segs.push(Seg::Line(q));
    }
    /// Adds a cubic.
    pub fn cubic_to(&mut self, a: P, b: P, c: P) {
        self.segs.push(Seg::Cubic(a, b, c));
    }
    /// Closes the subpath.
    pub fn close(&mut self) {
        self.segs.push(Seg::Close);
    }
    /// Appends another path.
    pub fn extend(&mut self, o: &Path) {
        self.segs.extend_from_slice(&o.segs);
    }
    /// Whether the path has no drawing segments.
    pub fn is_empty(&self) -> bool {
        !self.segs.iter().any(|s| !matches!(s, Seg::Move(_) | Seg::Close))
    }

    /// Transformed copy.
    pub fn transform(&self, x: &Xf) -> Path {
        let t = |q: P| x.apply(q);
        Path {
            segs: self
                .segs
                .iter()
                .map(|s| match *s {
                    Seg::Move(a) => Seg::Move(t(a)),
                    Seg::Line(a) => Seg::Line(t(a)),
                    Seg::Quad(a, b) => Seg::Quad(t(a), t(b)),
                    Seg::Cubic(a, b, c) => Seg::Cubic(t(a), t(b), t(c)),
                    Seg::Close => Seg::Close,
                })
                .collect(),
        }
    }

    /// Control-point bounds (contain the curve).
    pub fn bounds(&self) -> Rect {
        let mut r = Rect::EMPTY;
        for s in &self.segs {
            match *s {
                Seg::Move(a) | Seg::Line(a) => r.add(a),
                Seg::Quad(a, b) => {
                    r.add(a);
                    r.add(b)
                }
                Seg::Cubic(a, b, c) => {
                    r.add(a);
                    r.add(b);
                    r.add(c)
                }
                Seg::Close => {}
            }
        }
        r
    }

    /// Parses SVG path data (every command, absolute and relative, arcs included).
    pub fn parse(d: &str) -> Result<Path, PathError> {
        let b = d.as_bytes();
        let mut i = 0usize;
        let mut path = Path::default();
        let (mut cur, mut start) = (p(0.0, 0.0), p(0.0, 0.0));
        let mut last_ctrl: Option<(u8, P)> = None;
        let mut cmd = 0u8;
        let skip = |i: &mut usize| {
            while *i < b.len() && (b[*i].is_ascii_whitespace() || b[*i] == b',') {
                *i += 1;
            }
        };
        let number = |i: &mut usize| -> Result<f64, PathError> {
            skip(i);
            let s = *i;
            if *i < b.len() && (b[*i] == b'+' || b[*i] == b'-') {
                *i += 1;
            }
            let mut dot = false;
            while *i < b.len() && (b[*i].is_ascii_digit() || (b[*i] == b'.' && !dot)) {
                dot |= b[*i] == b'.';
                *i += 1;
            }
            if *i < b.len() && (b[*i] == b'e' || b[*i] == b'E') {
                *i += 1;
                if *i < b.len() && (b[*i] == b'+' || b[*i] == b'-') {
                    *i += 1;
                }
                while *i < b.len() && b[*i].is_ascii_digit() {
                    *i += 1;
                }
            }
            d[s..*i].parse::<f64>().map_err(|_| PathError { at: s, message: "expected a number".into() })
        };
        let flag = |i: &mut usize| -> Result<bool, PathError> {
            skip(i);
            match b.get(*i) {
                Some(b'0') => {
                    *i += 1;
                    Ok(false)
                }
                Some(b'1') => {
                    *i += 1;
                    Ok(true)
                }
                _ => Err(PathError { at: *i, message: "expected an arc flag (0 or 1)".into() }),
            }
        };
        loop {
            skip(&mut i);
            if i >= b.len() {
                break;
            }
            let c = b[i];
            if c.is_ascii_alphabetic() {
                if cmd == 0 && c != b'M' && c != b'm' {
                    return Err(PathError { at: i, message: "path data must start with a moveto".into() });
                }
                cmd = c;
                i += 1;
                if cmd == b'Z' || cmd == b'z' {
                    path.close();
                    cur = start;
                    last_ctrl = None;
                    continue;
                }
            } else if cmd == 0 {
                return Err(PathError { at: i, message: "path data must start with a command".into() });
            }
            let rel = cmd.is_ascii_lowercase();
            let o = if rel { cur } else { p(0.0, 0.0) };
            let up = cmd.to_ascii_uppercase();
            match up {
                b'M' => {
                    let q = o + p(number(&mut i)?, number(&mut i)?);
                    path.move_to(q);
                    cur = q;
                    start = q;
                    // further pairs are implicit line-tos
                    cmd = if rel { b'l' } else { b'L' };
                    last_ctrl = None;
                }
                b'L' => {
                    cur = o + p(number(&mut i)?, number(&mut i)?);
                    path.line_to(cur);
                    last_ctrl = None;
                }
                b'H' => {
                    let x = number(&mut i)?;
                    cur = p(if rel { cur.x + x } else { x }, cur.y);
                    path.line_to(cur);
                    last_ctrl = None;
                }
                b'V' => {
                    let y = number(&mut i)?;
                    cur = p(cur.x, if rel { cur.y + y } else { y });
                    path.line_to(cur);
                    last_ctrl = None;
                }
                b'C' | b'S' => {
                    let c1 = if up == b'C' {
                        o + p(number(&mut i)?, number(&mut i)?)
                    } else {
                        match last_ctrl {
                            Some((b'C', lc)) => cur * 2.0 - lc,
                            _ => cur,
                        }
                    };
                    let c2 = o + p(number(&mut i)?, number(&mut i)?);
                    let e = o + p(number(&mut i)?, number(&mut i)?);
                    path.cubic_to(c1, c2, e);
                    last_ctrl = Some((b'C', c2));
                    cur = e;
                }
                b'Q' | b'T' => {
                    let c1 = if up == b'Q' {
                        o + p(number(&mut i)?, number(&mut i)?)
                    } else {
                        match last_ctrl {
                            Some((b'Q', lc)) => cur * 2.0 - lc,
                            _ => cur,
                        }
                    };
                    let e = o + p(number(&mut i)?, number(&mut i)?);
                    path.segs.push(Seg::Quad(c1, e));
                    last_ctrl = Some((b'Q', c1));
                    cur = e;
                }
                b'A' => {
                    let (rx, ry, rot) = (number(&mut i)?, number(&mut i)?, number(&mut i)?);
                    let (large, sweep) = (flag(&mut i)?, flag(&mut i)?);
                    let e = o + p(number(&mut i)?, number(&mut i)?);
                    arc_to(&mut path, cur, e, rx, ry, rot, large, sweep);
                    cur = e;
                    last_ctrl = None;
                }
                _ => return Err(PathError { at: i - 1, message: format!("unknown command {:?}", cmd as char) }),
            }
        }
        Ok(path)
    }

    /// Subpaths in vertex form (quadratics raised to cubics).
    pub fn contours(&self) -> Vec<Contour> {
        let mut out: Vec<Contour> = Vec::new();
        let mut cur: Option<Contour> = None;
        let finish = |c: Option<Contour>, out: &mut Vec<Contour>| {
            if let Some(mut c) = c {
                if c.closed && c.v.len() > 1 && c.v[0].dist(*c.v.last().unwrap()) < 1e-9 {
                    // drop the duplicate closing vertex, keeping its in-tangent
                    let n = c.v.len() - 1;
                    c.i[0] = c.i[n];
                    c.v.pop();
                    c.i.pop();
                    c.o.pop();
                }
                if !c.v.is_empty() {
                    out.push(c);
                }
            }
        };
        for s in &self.segs {
            match *s {
                Seg::Move(a) => {
                    finish(cur.take(), &mut out);
                    cur = Some(Contour { v: vec![a], i: vec![a], o: vec![a], closed: false });
                }
                Seg::Close => {
                    if let Some(c) = cur.as_mut() {
                        c.closed = true;
                    }
                    let start = cur.as_ref().map(|c| c.v[0]);
                    finish(cur.take(), &mut out);
                    cur = start.map(|a| Contour { v: vec![a], i: vec![a], o: vec![a], closed: false });
                }
                _ => {
                    let c = cur.get_or_insert_with(|| Contour {
                        v: vec![p(0.0, 0.0)],
                        i: vec![p(0.0, 0.0)],
                        o: vec![p(0.0, 0.0)],
                        closed: false,
                    });
                    let last = *c.v.last().unwrap();
                    let (c1, c2, e) = match *s {
                        Seg::Line(e) => (last, e, e),
                        Seg::Quad(q, e) => (last + (q - last) * (2.0 / 3.0), e + (q - e) * (2.0 / 3.0), e),
                        Seg::Cubic(a, b, e) => (a, b, e),
                        _ => unreachable!(),
                    };
                    *c.o.last_mut().unwrap() = c1;
                    c.v.push(e);
                    c.i.push(c2);
                    c.o.push(e);
                }
            }
        }
        // a lone trailing move after a close carries nothing
        if let Some(c) = &cur {
            if c.v.len() > 1 {
                finish(cur, &mut out);
            }
        }
        out
    }

    /// Builds a path from vertex-form contours.
    pub fn from_contours(cs: &[Contour]) -> Path {
        let mut path = Path::default();
        for c in cs {
            if c.v.is_empty() {
                continue;
            }
            path.move_to(c.v[0]);
            let n = c.v.len();
            let segs = if c.closed { n } else { n - 1 };
            for k in 0..segs {
                let j = (k + 1) % n;
                let (a, b, e) = (c.o[k], c.i[j], c.v[j]);
                if a.dist(c.v[k]) < 1e-12 && b.dist(e) < 1e-12 {
                    path.line_to(e);
                } else {
                    path.cubic_to(a, b, e);
                }
            }
            if c.closed {
                path.close();
            }
        }
        path
    }

    /// Flattens to polylines, within `tol` of the curve.
    pub fn flatten(&self, tol: f64) -> Vec<Poly> {
        let tol = tol.max(1e-4);
        let mut out = Vec::new();
        let mut cur: Vec<P> = Vec::new();
        let mut start = p(0.0, 0.0);
        let flush = |cur: &mut Vec<P>, closed: bool, out: &mut Vec<Poly>| {
            if cur.len() > 1 || (closed && !cur.is_empty()) {
                let mut pts = std::mem::take(cur);
                if closed && pts.len() > 1 && pts[0].dist(*pts.last().unwrap()) < 1e-12 {
                    pts.pop();
                }
                out.push(Poly { pts, closed });
            } else if cur.len() == 1 {
                // a lone point (zero-length subpath) stays for round and square caps
                out.push(Poly { pts: std::mem::take(cur), closed: false });
            }
            cur.clear();
        };
        for s in &self.segs {
            match *s {
                Seg::Move(a) => {
                    flush(&mut cur, false, &mut out);
                    cur.push(a);
                    start = a;
                }
                Seg::Line(e) => {
                    if cur.is_empty() {
                        cur.push(start);
                    }
                    cur.push(e);
                }
                Seg::Quad(c, e) => {
                    let a = *cur.last().unwrap_or(&start);
                    if cur.is_empty() {
                        cur.push(a);
                    }
                    let dd = (a - c * 2.0 + e).len();
                    let n = (libm::sqrt(0.25 * dd / tol).ceil() as usize).clamp(1, 1000);
                    for k in 1..=n {
                        let t = k as f64 / n as f64;
                        let u = 1.0 - t;
                        cur.push(a * (u * u) + c * (2.0 * u * t) + e * (t * t));
                    }
                }
                Seg::Cubic(c1, c2, e) => {
                    let a = *cur.last().unwrap_or(&start);
                    if cur.is_empty() {
                        cur.push(a);
                    }
                    let dd = (a - c1 * 2.0 + c2).len().max((c1 - c2 * 2.0 + e).len());
                    let n = (libm::sqrt(0.75 * dd / tol).ceil() as usize).clamp(1, 1000);
                    for k in 1..=n {
                        cur.push(cubic_point(a, c1, c2, e, k as f64 / n as f64));
                    }
                }
                Seg::Close => flush(&mut cur, true, &mut out),
            }
        }
        if cur.len() > 1 {
            flush(&mut cur, false, &mut out);
        }
        out
    }
}

/// Point on a cubic.
pub fn cubic_point(a: P, b: P, c: P, d: P, t: f64) -> P {
    let u = 1.0 - t;
    a * (u * u * u) + b * (3.0 * u * u * t) + c * (3.0 * u * t * t) + d * (t * t * t)
}

/// Tangent (derivative) of a cubic.
pub fn cubic_tangent(a: P, b: P, c: P, d: P, t: f64) -> P {
    let u = 1.0 - t;
    (b - a) * (3.0 * u * u) + (c - b) * (6.0 * u * t) + (d - c) * (3.0 * t * t)
}

/// Appends an SVG elliptical arc as cubics.
#[allow(clippy::too_many_arguments)]
pub fn arc_to(path: &mut Path, from: P, to: P, rx: f64, ry: f64, rot_deg: f64, large: bool, sweep: bool) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if from.dist(to) < 1e-12 {
        return;
    }
    if rx < 1e-12 || ry < 1e-12 {
        path.line_to(to);
        return;
    }
    let phi = rot_deg.to_radians();
    let (sp, cp) = (libm::sin(phi), libm::cos(phi));
    let d = (from - to) * 0.5;
    let x1 = cp * d.x + sp * d.y;
    let y1 = -sp * d.x + cp * d.y;
    let lam = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lam > 1.0 {
        let s = libm::sqrt(lam);
        rx *= s;
        ry *= s;
    }
    let num = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut co = libm::sqrt(num / den.max(1e-300));
    if large == sweep {
        co = -co;
    }
    let cxp = co * rx * y1 / ry;
    let cyp = -co * ry * x1 / rx;
    let m = (from + to) * 0.5;
    let c = p(cp * cxp - sp * cyp + m.x, sp * cxp + cp * cyp + m.y);
    let ang = |u: P, v: P| libm::atan2(u.cross(v), u.dot(v));
    let t1 = ang(p(1.0, 0.0), p((x1 - cxp) / rx, (y1 - cyp) / ry));
    let mut dt = ang(p((x1 - cxp) / rx, (y1 - cyp) / ry), p((-x1 - cxp) / rx, (-y1 - cyp) / ry));
    if !sweep && dt > 0.0 {
        dt -= std::f64::consts::TAU;
    } else if sweep && dt < 0.0 {
        dt += std::f64::consts::TAU;
    }
    let n = (dt.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = dt / n as f64;
    let k = 4.0 / 3.0 * libm::tan(step / 4.0);
    let at = |t: f64| -> (P, P) {
        let (s, cc) = (libm::sin(t), libm::cos(t));
        let q = p(rx * cc, ry * s);
        let dq = p(-rx * s, ry * cc);
        let rotq = |v: P| p(cp * v.x - sp * v.y, sp * v.x + cp * v.y);
        (rotq(q) + c, rotq(dq))
    };
    for j in 0..n {
        let (a0, a1) = (t1 + step * j as f64, t1 + step * (j + 1) as f64);
        let (p0, d0) = at(a0);
        let (p1, d1) = at(a1);
        let end = if j + 1 == n { to } else { p1 };
        path.cubic_to(p0 + d0 * k, p1 - d1 * k, end);
    }
}

impl Poly {
    /// Signed area (positive clockwise on screen, y down).
    pub fn area(&self) -> f64 {
        let n = self.pts.len();
        (0..n).map(|k| self.pts[k].cross(self.pts[(k + 1) % n])).sum::<f64>() * 0.5
    }
    /// Length, including the closing segment when closed.
    pub fn length(&self) -> f64 {
        let n = self.pts.len();
        let mut l: f64 = self.pts.windows(2).map(|w| w[0].dist(w[1])).sum();
        if self.closed && n > 1 {
            l += self.pts[n - 1].dist(self.pts[0]);
        }
        l
    }
    /// Splits segments longer than `max` so point-wise deformation bends them smoothly.
    pub fn subdivide(&self, max: f64) -> Poly {
        let n = self.pts.len();
        let mut out = Vec::with_capacity(n);
        let segs = if self.closed { n } else { n.saturating_sub(1) };
        if n == 0 {
            return self.clone();
        }
        for k in 0..segs {
            let (a, b) = (self.pts[k], self.pts[(k + 1) % n]);
            let m = ((a.dist(b) / max.max(1e-6)).ceil() as usize).max(1);
            for j in 0..m {
                out.push(a.lerp(b, j as f64 / m as f64));
            }
        }
        if !self.closed {
            out.push(self.pts[n - 1]);
        }
        Poly { pts: out, closed: self.closed }
    }
}

/// Bounds of polylines.
pub fn poly_bounds(ps: &[Poly]) -> Rect {
    let mut r = Rect::EMPTY;
    for q in ps {
        for &pt in &q.pts {
            r.add(pt);
        }
    }
    r
}

/// Polylines back to a path of lines.
pub fn polys_to_path(ps: &[Poly]) -> Path {
    let mut path = Path::default();
    for q in ps {
        if let Some((&f, rest)) = q.pts.split_first() {
            path.move_to(f);
            for &pt in rest {
                path.line_to(pt);
            }
            if q.closed {
                path.close();
            }
        }
    }
    path
}
