//! SVG path data for motion paths: parsing, conversion to lines and cubic
//! Béziers, and sampling by arc length or by segment parameter.

/// A 2D point.
pub type P = [f64; 2];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Seg {
    Line(P, P),
    Cubic(P, P, P, P),
}

impl Seg {
    fn at(&self, s: f64) -> P {
        match *self {
            Seg::Line(a, b) => [a[0] + (b[0] - a[0]) * s, a[1] + (b[1] - a[1]) * s],
            Seg::Cubic(a, b, c, d) => {
                let m = 1.0 - s;
                let (w0, w1, w2, w3) = (m * m * m, 3.0 * m * m * s, 3.0 * m * s * s, s * s * s);
                [w0 * a[0] + w1 * b[0] + w2 * c[0] + w3 * d[0], w0 * a[1] + w1 * b[1] + w2 * c[1] + w3 * d[1]]
            }
        }
    }

    fn tangent(&self, s: f64) -> P {
        match *self {
            Seg::Line(a, b) => [b[0] - a[0], b[1] - a[1]],
            Seg::Cubic(a, b, c, d) => {
                let m = 1.0 - s;
                let t = [
                    3.0 * m * m * (b[0] - a[0]) + 6.0 * m * s * (c[0] - b[0]) + 3.0 * s * s * (d[0] - c[0]),
                    3.0 * m * m * (b[1] - a[1]) + 6.0 * m * s * (c[1] - b[1]) + 3.0 * s * s * (d[1] - c[1]),
                ];
                if t[0] == 0.0 && t[1] == 0.0 {
                    [d[0] - a[0], d[1] - a[1]]
                } else {
                    t
                }
            }
        }
    }
}

/// A parsed path with an arc-length table.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionPath {
    segs: Vec<Seg>,
    /// Index of the first segment of each subpath.
    starts: Vec<usize>,
    /// Cumulative length at each of `SAMPLES` points per segment.
    table: Vec<(f64, usize, f64)>,
    length: f64,
}

const SAMPLES: usize = 32;

/// Parses path data into polylines (see [`MotionPath::polylines`]).
pub fn flatten(d: &str, tolerance: f64) -> Result<Vec<Vec<P>>, PathError> {
    Ok(MotionPath::parse(d)?.polylines(tolerance))
}

/// A parse error with the byte offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathError {
    /// Message.
    pub message: String,
    /// Byte offset.
    pub offset: usize,
}

struct Scan<'s> {
    s: &'s [u8],
    i: usize,
}

impl Scan<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',') {
            self.i += 1;
        }
    }

    fn num(&mut self) -> Result<f64, PathError> {
        self.ws();
        let start = self.i;
        let b = self.s;
        let mut i = self.i;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let mut dot = false;
        while i < b.len() && (b[i].is_ascii_digit() || (b[i] == b'.' && !dot)) {
            dot |= b[i] == b'.';
            i += 1;
        }
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            let mut j = i + 1;
            if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                j += 1;
            }
            if j < b.len() && b[j].is_ascii_digit() {
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                i = j;
            }
        }
        let t = std::str::from_utf8(&b[start..i]).unwrap_or("");
        let v = t.parse::<f64>().map_err(|_| PathError { message: "expected a number".into(), offset: start })?;
        self.i = i;
        Ok(v)
    }

    fn flag(&mut self) -> Result<bool, PathError> {
        self.ws();
        match self.s.get(self.i) {
            Some(b'0') => {
                self.i += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.i += 1;
                Ok(true)
            }
            _ => Err(PathError { message: "expected an arc flag 0 or 1".into(), offset: self.i }),
        }
    }

    fn more_numbers(&mut self) -> bool {
        self.ws();
        matches!(self.s.get(self.i), Some(c) if c.is_ascii_digit() || *c == b'-' || *c == b'+' || *c == b'.')
    }
}

#[allow(clippy::too_many_arguments)]
fn arc(p0: P, rx: f64, ry: f64, phi_deg: f64, large: bool, sweep: bool, p1: P, out: &mut Vec<Seg>) {
    // SVG 1.1 implementation notes F.6.5 / F.6.6
    if p0 == p1 {
        return;
    }
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 {
        out.push(Seg::Line(p0, p1));
        return;
    }
    let phi = phi_deg.to_radians();
    let (sin, cos) = (libm::sin(phi), libm::cos(phi));
    let dx = (p0[0] - p1[0]) / 2.0;
    let dy = (p0[1] - p1[1]) / 2.0;
    let x1 = cos * dx + sin * dy;
    let y1 = -sin * dx + cos * dy;
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        let s = libm::sqrt(lambda);
        rx *= s;
        ry *= s;
    }
    let num = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut co = libm::sqrt(num / den);
    if large == sweep {
        co = -co;
    }
    let cx1 = co * rx * y1 / ry;
    let cy1 = -co * ry * x1 / rx;
    let cx = cos * cx1 - sin * cy1 + (p0[0] + p1[0]) / 2.0;
    let cy = sin * cx1 + cos * cy1 + (p0[1] + p1[1]) / 2.0;
    let ang = |ux: f64, uy: f64, vx: f64, vy: f64| libm::atan2(ux * vy - uy * vx, ux * vx + uy * vy);
    let th1 = ang(1.0, 0.0, (x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut dth = ang((x1 - cx1) / rx, (y1 - cy1) / ry, (-x1 - cx1) / rx, (-y1 - cy1) / ry);
    if !sweep && dth > 0.0 {
        dth -= 2.0 * std::f64::consts::PI;
    } else if sweep && dth < 0.0 {
        dth += 2.0 * std::f64::consts::PI;
    }
    let n = libm::ceil(dth.abs() / (std::f64::consts::FRAC_PI_2) - 1e-9).max(1.0) as usize;
    let step = dth / n as f64;
    let k = 4.0 / 3.0 * libm::tan(step / 4.0);
    let pt = |t: f64| {
        let (ct, st) = (libm::cos(t), libm::sin(t));
        [cx + rx * ct * cos - ry * st * sin, cy + rx * ct * sin + ry * st * cos]
    };
    let dpt = |t: f64| {
        let (ct, st) = (libm::cos(t), libm::sin(t));
        [-rx * st * cos - ry * ct * sin, -rx * st * sin + ry * ct * cos]
    };
    let mut start = p0;
    for i in 0..n {
        let (t0, t1) = (th1 + step * i as f64, th1 + step * (i + 1) as f64);
        let end = if i + 1 == n { p1 } else { pt(t1) };
        let (d0, d1) = (dpt(t0), dpt(t1));
        out.push(Seg::Cubic(
            start,
            [start[0] + k * d0[0], start[1] + k * d0[1]],
            [end[0] - k * d1[0], end[1] - k * d1[1]],
            end,
        ));
        start = end;
    }
}

impl MotionPath {
    /// Parses SVG path data (all commands, absolute and relative).
    pub fn parse(d: &str) -> Result<MotionPath, PathError> {
        let mut sc = Scan { s: d.as_bytes(), i: 0 };
        let mut segs = Vec::new();
        let mut starts = vec![0usize];
        let (mut cur, mut start): (P, P) = ([0.0, 0.0], [0.0, 0.0]);
        let mut last_ctrl: Option<(u8, P)> = None;
        let mut cmd: Option<u8> = None;
        loop {
            sc.ws();
            if sc.i >= sc.s.len() {
                break;
            }
            let c = sc.s[sc.i];
            if c.is_ascii_alphabetic() {
                sc.i += 1;
                cmd = Some(c);
            } else if cmd.is_none() {
                return Err(PathError { message: "path data must start with a command".into(), offset: sc.i });
            } else if matches!(cmd, Some(b'Z' | b'z')) {
                return Err(PathError { message: "numbers after Z".into(), offset: sc.i });
            }
            let c = cmd.unwrap();
            let rel = c.is_ascii_lowercase();
            let off = |p: P, cur: P| if rel { [p[0] + cur[0], p[1] + cur[1]] } else { p };
            let upper = c.to_ascii_uppercase();
            let mut ctrl = None;
            match upper {
                b'M' => {
                    if starts.last() != Some(&segs.len()) {
                        starts.push(segs.len());
                    }
                    let p = off([sc.num()?, sc.num()?], cur);
                    cur = p;
                    start = p;
                    cmd = Some(if rel { b'l' } else { b'L' });
                }
                b'L' => {
                    let p = off([sc.num()?, sc.num()?], cur);
                    segs.push(Seg::Line(cur, p));
                    cur = p;
                }
                b'H' => {
                    let x = sc.num()?;
                    let p = [if rel { cur[0] + x } else { x }, cur[1]];
                    segs.push(Seg::Line(cur, p));
                    cur = p;
                }
                b'V' => {
                    let y = sc.num()?;
                    let p = [cur[0], if rel { cur[1] + y } else { y }];
                    segs.push(Seg::Line(cur, p));
                    cur = p;
                }
                b'C' | b'S' => {
                    let c1 = if upper == b'C' {
                        off([sc.num()?, sc.num()?], cur)
                    } else {
                        match last_ctrl {
                            Some((b'C', q)) => [2.0 * cur[0] - q[0], 2.0 * cur[1] - q[1]],
                            _ => cur,
                        }
                    };
                    let c2 = off([sc.num()?, sc.num()?], cur);
                    let p = off([sc.num()?, sc.num()?], cur);
                    segs.push(Seg::Cubic(cur, c1, c2, p));
                    ctrl = Some((b'C', c2));
                    cur = p;
                }
                b'Q' | b'T' => {
                    let q = if upper == b'Q' {
                        off([sc.num()?, sc.num()?], cur)
                    } else {
                        match last_ctrl {
                            Some((b'Q', q)) => [2.0 * cur[0] - q[0], 2.0 * cur[1] - q[1]],
                            _ => cur,
                        }
                    };
                    let p = off([sc.num()?, sc.num()?], cur);
                    let c1 = [cur[0] + 2.0 / 3.0 * (q[0] - cur[0]), cur[1] + 2.0 / 3.0 * (q[1] - cur[1])];
                    let c2 = [p[0] + 2.0 / 3.0 * (q[0] - p[0]), p[1] + 2.0 / 3.0 * (q[1] - p[1])];
                    segs.push(Seg::Cubic(cur, c1, c2, p));
                    ctrl = Some((b'Q', q));
                    cur = p;
                }
                b'A' => {
                    let (rx, ry, rot) = (sc.num()?, sc.num()?, sc.num()?);
                    let (large, sweep) = (sc.flag()?, sc.flag()?);
                    let p = off([sc.num()?, sc.num()?], cur);
                    arc(cur, rx, ry, rot, large, sweep, p, &mut segs);
                    cur = p;
                }
                b'Z' => {
                    if cur != start {
                        segs.push(Seg::Line(cur, start));
                    }
                    cur = start;
                }
                other => {
                    return Err(PathError {
                        message: format!("unknown path command '{}'", other as char),
                        offset: sc.i - 1,
                    });
                }
            }
            last_ctrl = ctrl;
            if upper != b'Z' && !sc.more_numbers() && sc.i < sc.s.len() && !sc.s[sc.i].is_ascii_alphabetic() {
                return Err(PathError { message: "unexpected character in path data".into(), offset: sc.i });
            }
        }
        if segs.is_empty() {
            segs.push(Seg::Line(cur, cur));
        }
        let mut table = Vec::with_capacity(segs.len() * SAMPLES + 1);
        let mut acc = 0.0;
        let mut prev = segs[0].at(0.0);
        table.push((0.0, 0, 0.0));
        for (i, s) in segs.iter().enumerate() {
            for k in 1..=SAMPLES {
                let t = k as f64 / SAMPLES as f64;
                let p = s.at(t);
                acc += libm::hypot(p[0] - prev[0], p[1] - prev[1]);
                table.push((acc, i, t));
                prev = p;
            }
        }
        Ok(MotionPath { segs, starts, table, length: acc })
    }

    /// Subpaths as polylines, curves subdivided until the chord deviates by
    /// at most `tolerance` pixels.
    pub fn polylines(&self, tolerance: f64) -> Vec<Vec<P>> {
        let mut out = Vec::new();
        let mut bounds = self.starts.clone();
        bounds.push(self.segs.len());
        for w in bounds.windows(2) {
            let segs = &self.segs[w[0]..w[1]];
            let Some(first) = segs.first() else { continue };
            let mut poly = vec![first.at(0.0)];
            for s in segs {
                match *s {
                    Seg::Line(_, b) => poly.push(b),
                    Seg::Cubic(a, b, c, d) => {
                        // Wang's bound on the number of segments
                        let dd = |p: P, q: P, r: P| {
                            ((p[0] - 2.0 * q[0] + r[0]).powi(2) + (p[1] - 2.0 * q[1] + r[1]).powi(2)).sqrt()
                        };
                        let m = dd(a, b, c).max(dd(b, c, d));
                        let n = ((0.75 * m / tolerance.max(1e-6)).sqrt().ceil() as usize).clamp(1, 256);
                        for k in 1..=n {
                            poly.push(s.at(k as f64 / n as f64));
                        }
                    }
                }
            }
            out.push(poly);
        }
        out
    }

    /// Total length in pixels.
    pub fn length(&self) -> f64 {
        self.length
    }

    /// Point and tangent angle (degrees, clockwise on screen) at progress
    /// `p ∈ [0, 1]`. With `constant_speed`, progress is a fraction of arc
    /// length; otherwise every segment takes an equal share of progress.
    pub fn sample(&self, p: f64, constant_speed: bool) -> (P, f64) {
        let p = p.clamp(0.0, 1.0);
        let (seg, s) = if constant_speed && self.length > 0.0 {
            let target = p * self.length;
            let k = self.table.partition_point(|e| e.0 < target).clamp(1, self.table.len() - 1);
            let (l0, i0, t0) = self.table[k - 1];
            let (l1, i1, t1) = self.table[k];
            let f = if l1 > l0 { (target - l0) / (l1 - l0) } else { 0.0 };
            if i0 == i1 || t0 == 1.0 {
                let t0 = if i0 != i1 { 0.0 } else { t0 };
                (i1, t0 + (t1 - t0) * f)
            } else {
                (i1, t1 * f)
            }
        } else {
            let n = self.segs.len() as f64;
            let x = p * n;
            let i = (libm::floor(x) as usize).min(self.segs.len() - 1);
            (i, x - i as f64)
        };
        let s = s.clamp(0.0, 1.0);
        let pt = self.segs[seg].at(s);
        let tg = self.segs[seg].tangent(s);
        (pt, libm::atan2(tg[1], tg[0]).to_degrees())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: P, b: P) -> bool {
        (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
    }

    #[test]
    fn lines_and_relative_commands() {
        let p = MotionPath::parse("M0 0 h100 v100 L0,100 z").unwrap();
        assert!((p.length() - 400.0).abs() < 1e-9);
        assert!(close(p.sample(0.25, true).0, [100.0, 0.0]));
        assert!(close(p.sample(0.5, true).0, [100.0, 100.0]));
        assert!((p.sample(0.3, true).1 - 90.0).abs() < 1e-9, "heading down");
        let q = MotionPath::parse("m10,10 l10-10").unwrap();
        assert!(close(q.sample(1.0, true).0, [20.0, 0.0]));
    }

    #[test]
    fn curves_and_arcs() {
        let c = MotionPath::parse("M0 0 C0 100 100 100 100 0").unwrap();
        assert!(close(c.sample(0.5, false).0, [50.0, 75.0]));
        let a = MotionPath::parse("M0 0 A50 50 0 0 1 100 0").unwrap();
        let (mid, _) = a.sample(0.5, true);
        assert!((mid[0] - 50.0).abs() < 1e-3 && (mid[1] + 50.0).abs() < 0.05, "{mid:?}");
        assert!((a.length() - std::f64::consts::PI * 50.0).abs() < 0.05, "{}", a.length());
        let q = MotionPath::parse("M0 0 Q50 100 100 0 T200 0").unwrap();
        assert!(q.sample(0.75, false).0[1] < 0.0, "T reflects the control point");
        let s = MotionPath::parse("M0 0 C0 50 50 50 50 0 S100 -50 100 0").unwrap();
        assert!(close(s.sample(1.0, false).0, [100.0, 0.0]));
    }

    #[test]
    fn subpaths_flatten_separately() {
        let polys = flatten("M0 0 L10 0 L10 10 Z M20 20 C20 30 30 30 30 20", 0.1).unwrap();
        assert_eq!(polys.len(), 2);
        assert_eq!(polys[0], vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 0.0]]);
        assert!(polys[1].len() > 4 && polys[1].last() == Some(&[30.0, 20.0]));
    }

    #[test]
    fn errors() {
        assert!(MotionPath::parse("0 0 L 1 1").is_err());
        assert!(MotionPath::parse("M0 0 X 1 1").is_err());
        assert!(MotionPath::parse("M0 0 A 5 5 0 2 0 1 1").is_err());
    }
}
