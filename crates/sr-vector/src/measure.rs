//! Arc length, trim paths and dashes on flattened polylines.

use crate::geom::P;
use crate::path::Poly;

/// Dashes one [`dash`] call produces at most; a denser pattern is drawn solid.
pub const MAX_DASHES: usize = 1 << 20;

/// How trim applies across several subpaths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimMode {
    /// Each subpath is trimmed by the same fractions.
    Simultaneous,
    /// The subpaths are trimmed as one path, in order.
    Sequential,
}

/// Points of a polyline as an open list, the closing point repeated when closed.
fn open_pts(q: &Poly) -> Vec<P> {
    let mut v = q.pts.clone();
    if q.closed && !v.is_empty() {
        v.push(v[0]);
    }
    v
}

/// The piece of an open point list between arc lengths `a` and `b`.
fn cut(pts: &[P], a: f64, b: f64) -> Vec<P> {
    cut_from(pts, a, b, &mut (0, 0.0))
}

/// [`cut`] starting at the segment `at` (its index and the arc length where it begins), left on
/// the segment holding `b`: successive pieces further along the list each resume where the last ended.
fn cut_from(pts: &[P], a: f64, b: f64, at: &mut (usize, f64)) -> Vec<P> {
    let mut out = Vec::new();
    let (mut k, mut s) = *at;
    let mut resume = None;
    while k + 1 < pts.len() {
        let w = &pts[k..k + 2];
        let l = w[0].dist(w[1]);
        let (s0, s1) = (s, s + l);
        if resume.is_none() && s1 >= b {
            resume = Some((k, s0));
        }
        if s1 >= a && s0 <= b && l > 0.0 {
            let t0 = ((a - s0) / l).clamp(0.0, 1.0);
            let t1 = ((b - s0) / l).clamp(0.0, 1.0);
            let q0 = w[0].lerp(w[1], t0);
            if out.last().is_none_or(|l: &P| l.dist(q0) > 1e-12) {
                out.push(q0);
            }
            out.push(w[0].lerp(w[1], t1));
        }
        s = s1;
        if s > b {
            break;
        }
        k += 1;
    }
    *at = resume.unwrap_or((k, s));
    out
}

/// Visible ranges `[a, b]` of a unit interval for trim start, end and offset (fractions).
fn ranges(start: f64, end: f64, offset: f64) -> Vec<(f64, f64)> {
    let (mut s, mut e) = (start.min(end), start.max(end));
    if e - s >= 1.0 - 1e-12 {
        return vec![(0.0, 1.0)];
    }
    if e - s <= 1e-12 {
        return Vec::new();
    }
    let sh = offset - libm::floor(s + offset);
    s += sh;
    e += sh;
    if e <= 1.0 {
        vec![(s, e)]
    } else {
        vec![(s, 1.0), (0.0, e - 1.0)]
    }
}

/// Trims polylines to the fractions `start`–`end`, shifted by `offset` (a fraction of the length, wrapping).
pub fn trim(ps: &[Poly], start: f64, end: f64, offset: f64, mode: TrimMode) -> Vec<Poly> {
    let rs = ranges(start, end, offset);
    if rs == [(0.0, 1.0)] {
        return ps.to_vec();
    }
    let mut out = Vec::new();
    match mode {
        TrimMode::Simultaneous => {
            for q in ps {
                let pts = open_pts(q);
                let total = q.length();
                let mut pieces: Vec<Vec<P>> = rs.iter().map(|&(a, b)| cut(&pts, a * total, b * total)).collect();
                // a wrapped range on a closed path joins across the seam
                if q.closed && pieces.len() == 2 && rs[0].1 >= 1.0 && rs[1].0 <= 0.0 {
                    let mut first = pieces.remove(0);
                    first.extend(pieces.remove(0).into_iter().skip(1));
                    pieces = vec![first];
                }
                out.extend(pieces.into_iter().filter(|v| v.len() > 1).map(|pts| Poly { pts, closed: false }));
            }
        }
        TrimMode::Sequential => {
            let lens: Vec<f64> = ps.iter().map(Poly::length).collect();
            let total: f64 = lens.iter().sum();
            for &(a, b) in &rs {
                let (a, b) = (a * total, b * total);
                let mut s = 0.0;
                for (q, &l) in ps.iter().zip(&lens) {
                    if s + l >= a && s <= b {
                        let pts = cut(&open_pts(q), a - s, b - s);
                        if pts.len() > 1 {
                            out.push(Poly { pts, closed: false });
                        }
                    }
                    s += l;
                }
            }
        }
    }
    out
}

/// Dashes polylines with an on/off pattern starting `offset` into it.
pub fn dash(ps: &[Poly], pattern: &[f64], offset: f64) -> Vec<Poly> {
    let mut pat: Vec<f64> = pattern.iter().map(|v| v.max(0.0)).collect();
    if pat.len() % 2 == 1 {
        pat.extend_from_within(..);
    }
    let period: f64 = pat.iter().sum();
    if pat.is_empty() || period <= 1e-9 {
        return ps.to_vec();
    }
    // more dashes than anything can show: solid
    let dashes = ps.iter().map(Poly::length).sum::<f64>() / period * (pat.len() / 2) as f64;
    if dashes.is_nan() || dashes > MAX_DASHES as f64 {
        return ps.to_vec();
    }
    let mut out = Vec::new();
    for q in ps {
        let pts = open_pts(q);
        let total = q.length();
        let mut at = (0, 0.0);
        // phase: position inside the pattern at arc length 0
        let mut phase = offset.rem_euclid(period);
        let mut k = 0;
        while phase >= pat[k] {
            phase -= pat[k];
            k = (k + 1) % pat.len();
        }
        let mut s = -phase;
        while s < total {
            let e = s + pat[k];
            if k % 2 == 0 && e > 0.0 {
                let piece = cut_from(&pts, s.max(0.0), e.min(total), &mut at);
                if piece.len() > 1 || (pat[k] == 0.0 && !piece.is_empty()) {
                    out.push(Poly { pts: piece, closed: false });
                }
            }
            s = e;
            k = (k + 1) % pat.len();
        }
    }
    out
}

/// Point and unit tangent at arc length `s` along an open point list.
pub fn at_length(pts: &[P], s: f64) -> Option<(P, P)> {
    let mut acc = 0.0;
    for w in pts.windows(2) {
        let l = w[0].dist(w[1]);
        if acc + l >= s && l > 0.0 {
            let t = ((s - acc) / l).clamp(0.0, 1.0);
            return Some((w[0].lerp(w[1], t), (w[1] - w[0]).norm()));
        }
        acc += l;
    }
    let n = pts.len();
    (n >= 2).then(|| (pts[n - 1], (pts[n - 1] - pts[n - 2]).norm()))
}

/// Point and tangent at fraction `u` of a polyline's length.
pub fn at_fraction(q: &Poly, u: f64) -> Option<(P, P)> {
    at_length(&open_pts(q), u.clamp(0.0, 1.0) * q.length())
}
