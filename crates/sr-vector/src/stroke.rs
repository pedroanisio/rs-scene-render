//! Stroke outlines: polylines become closed contours that fill (nonzero)
//! to the stroked area, with butt, round and square caps and miter, round
//! and bevel joins. Inner joins pass through the vertex, so overlaps at
//! sharp turns keep a consistent winding instead of cutting holes.

use crate::geom::{p, P};
use crate::path::Poly;

/// Line cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    Butt,
    Round,
    Square,
}

/// Line join.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Join {
    Miter,
    Round,
    Bevel,
}

/// Stroke style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub width: f64,
    pub cap: Cap,
    pub join: Join,
    pub miter_limit: f64,
}

fn arc(out: &mut Vec<P>, c: P, r: f64, a0: f64, a1: f64, tol: f64) {
    let step = 2.0 * libm::acos((1.0 - tol / r.max(tol)).clamp(-1.0, 1.0));
    let n = ((a1 - a0).abs() / step.max(1e-3)).ceil().clamp(1.0, 256.0) as usize;
    for k in 1..n {
        let a = a0 + (a1 - a0) * k as f64 / n as f64;
        out.push(c + p(libm::cos(a), libm::sin(a)) * r);
    }
}

/// Signed sweep from direction `a` to direction `b` in (−π, π].
fn sweep(a: P, b: P) -> f64 {
    libm::atan2(a.cross(b), a.dot(b))
}

/// Offsets one side (`side` = 1 left of travel, −1 right) with joins at interior vertices.
fn side(pts: &[P], closed: bool, h: f64, s: f64, style: &Style, tol: f64, out: &mut Vec<P>) {
    let n = pts.len();
    let seg_dir = |k: usize| (pts[(k + 1) % n] - pts[k]).norm();
    let segs = if closed { n } else { n - 1 };
    for k in 0..segs {
        let d = seg_dir(k);
        let nrm = d.perp() * (s * h);
        let a = pts[k] + nrm;
        let b = pts[(k + 1) % n] + nrm;
        out.push(a);
        out.push(b);
        // join at the end vertex of this segment
        let has_next = closed || k + 1 < segs;
        if !has_next {
            break;
        }
        let v = pts[(k + 1) % n];
        let d2 = seg_dir((k + 1) % n);
        let turn = d.cross(d2);
        let outer = turn * s < 0.0 || (turn.abs() < 1e-12 && d.dot(d2) < 0.0);
        let n2 = d2.perp() * (s * h);
        if !outer {
            // inner join through the vertex keeps the winding consistent
            out.push(v);
            continue;
        }
        match style.join {
            Join::Round => {
                let a0 = nrm.angle();
                let mut sw = sweep(nrm, n2);
                if sw * s > 0.0 && sw.abs() > 1e-9 {
                    sw -= std::f64::consts::TAU * sw.signum();
                }
                arc(out, v, h, a0, a0 + sw, tol);
            }
            Join::Miter => {
                let cos_half = libm::sqrt(((1.0 + d.dot(d2)) * 0.5).max(0.0));
                if cos_half > 1e-9 && 1.0 / cos_half <= style.miter_limit {
                    let m = (nrm + n2).norm() * (h / cos_half);
                    out.push(v + m);
                }
            }
            Join::Bevel => {}
        }
    }
}

fn cap(out: &mut Vec<P>, end: P, dir: P, h: f64, style: &Style, tol: f64) {
    // from the left side of travel around the end to the right side
    let l = dir.perp() * h;
    match style.cap {
        Cap::Butt => {}
        Cap::Square => {
            out.push(end + l + dir * h);
            out.push(end - l + dir * h);
        }
        Cap::Round => {
            let a0 = l.angle();
            arc(out, end, h, a0, a0 - std::f64::consts::PI * 1.0, tol);
        }
    }
}

fn dedupe(q: &Poly) -> Vec<P> {
    let mut pts: Vec<P> = Vec::with_capacity(q.pts.len());
    for &pt in &q.pts {
        if pts.last().is_none_or(|l: &P| l.dist(pt) > 1e-9) {
            pts.push(pt);
        }
    }
    if q.closed && pts.len() > 1 && pts[0].dist(*pts.last().unwrap()) <= 1e-9 {
        pts.pop();
    }
    pts
}

/// Stroke outline contours of `ps`; fill them with the nonzero rule.
pub fn stroke(ps: &[Poly], style: &Style, tol: f64) -> Vec<Poly> {
    let h = style.width * 0.5;
    let mut out = Vec::new();
    if h <= 0.0 {
        return out;
    }
    for q in ps {
        let pts = dedupe(q);
        match pts.len() {
            0 => {}
            1 => {
                // zero-length subpath: round and square caps still draw a dot
                let c = pts[0];
                let mut v = Vec::new();
                match style.cap {
                    Cap::Round => {
                        v.push(c + p(h, 0.0));
                        arc(&mut v, c, h, 0.0, std::f64::consts::TAU, tol);
                    }
                    Cap::Square => v.extend([c + p(-h, -h), c + p(h, -h), c + p(h, h), c + p(-h, h)]),
                    Cap::Butt => continue,
                }
                out.push(Poly { pts: v, closed: true });
            }
            _ if q.closed && pts.len() > 2 => {
                let mut left = Vec::new();
                side(&pts, true, h, 1.0, style, tol, &mut left);
                let mut rev = pts.clone();
                rev.reverse();
                let mut right = Vec::new();
                side(&rev, true, h, 1.0, style, tol, &mut right);
                out.push(Poly { pts: left, closed: true });
                out.push(Poly { pts: right, closed: true });
            }
            n => {
                let mut v = Vec::new();
                side(&pts, false, h, 1.0, style, tol, &mut v);
                cap(&mut v, pts[n - 1], (pts[n - 1] - pts[n - 2]).norm(), h, style, tol);
                let mut rev = pts.clone();
                rev.reverse();
                side(&rev, false, h, 1.0, style, tol, &mut v);
                cap(&mut v, pts[0], (pts[0] - pts[1]).norm(), h, style, tol);
                out.push(Poly { pts: v, closed: true });
            }
        }
    }
    out
}
