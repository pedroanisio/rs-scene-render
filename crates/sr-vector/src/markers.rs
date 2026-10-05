//! Stroke markers (SREP 15): shapes at the ends of the drawn part of an open outline, and the setback they take from
//! the stroke.

use crate::geom::{p, P};
use crate::path::Poly;
use crate::stroke::{self, Style};

/// The closed set of markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    /// Nothing.
    None,
    /// A filled triangle.
    Arrow,
    /// A stroked chevron.
    OpenArrow,
    /// A filled disc.
    Circle,
    /// A filled square.
    Square,
    /// A filled diamond.
    Diamond,
    /// A stroked bar across the end.
    Bar,
}

impl Marker {
    /// The marker an attribute value names; unknown values draw nothing.
    pub fn parse(s: &str) -> Marker {
        match s {
            "arrow" => Marker::Arrow,
            "open-arrow" => Marker::OpenArrow,
            "circle" => Marker::Circle,
            "square" => Marker::Square,
            "diamond" => Marker::Diamond,
            "bar" => Marker::Bar,
            _ => Marker::None,
        }
    }

    /// How far the stroke stops short of the end of the outline, for a marker of length `l`.
    fn setback(self, l: f64) -> f64 {
        match self {
            Marker::Arrow => l,
            Marker::Circle | Marker::Square | Marker::Diamond => l / 2.0,
            Marker::None | Marker::OpenArrow | Marker::Bar => 0.0,
        }
    }
}

/// The outline shortened at its ends by the markers' setbacks, and the marker geometry to fill together with the stroke.
pub struct Marked {
    /// The drawn outline after the setbacks.
    pub outline: Vec<Poly>,
    /// Marker polygons with positive area: the caller winds them like its stroke before the union is filled.
    pub fills: Vec<Poly>,
    /// Stroked marker parts (open arrows and bars), already wound like the stroke.
    pub stroked: Vec<Poly>,
}

fn len(a: P, b: P) -> f64 {
    ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt()
}

/// The unit direction of travel at the end of `pts` (along the last segment of nonzero length), if any.
fn end_direction(pts: &[P]) -> Option<P> {
    let last = *pts.last()?;
    pts.iter().rev().skip(1).find(|q| len(**q, last) > 1e-9).map(|q| {
        let d = len(*q, last);
        p((last.x - q.x) / d, (last.y - q.y) / d)
    })
}

/// Removes `d` of length from the end of `pts` (the whole polyline when it is shorter).
fn shorten_end(pts: &mut Vec<P>, mut d: f64) {
    while d > 0.0 && pts.len() >= 2 {
        let (a, b) = (pts[pts.len() - 2], pts[pts.len() - 1]);
        let l = len(a, b);
        if l > d {
            let t = (l - d) / l;
            let n = pts.len() - 1;
            pts[n] = p(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
            return;
        }
        d -= l;
        pts.pop();
    }
}

fn polygon(pts: &[P], at: P, dir: P) -> Poly {
    // the marker frame: +x along `dir`, +y = +x turned 90 degrees clockwise on screen (y down)
    let to_world = |q: P| p(at.x + q.x * dir.x - q.y * dir.y, at.y + q.x * dir.y + q.y * dir.x);
    Poly { pts: pts.iter().map(|q| to_world(*q)).collect(), closed: true }
}

pub fn area(q: &Poly) -> f64 {
    let n = q.pts.len();
    (0..n).map(|i| q.pts[i].cross(q.pts[(i + 1) % n])).sum::<f64>() / 2.0
}

/// Adds the geometry of marker `m` at `at`, travelling along `dir`; `l` is its length and `style` the stroke's.
fn marker(m: Marker, at: P, dir: P, l: f64, style: &Style, tol: f64, out: &mut Marked) {
    let h = l / 2.0;
    match m {
        Marker::None => {}
        Marker::Arrow => out.fills.push(polygon(&[p(0.0, 0.0), p(-l, -h), p(-l, h)], at, dir)),
        Marker::Square => out.fills.push(polygon(&[p(-l, -h), p(0.0, -h), p(0.0, h), p(-l, h)], at, dir)),
        Marker::Diamond => out.fills.push(polygon(&[p(0.0, 0.0), p(-h, -h), p(-l, 0.0), p(-h, h)], at, dir)),
        Marker::Circle => {
            let n = ((std::f64::consts::PI / (1.0 - tol.min(h) / h).clamp(-1.0, 1.0).acos().max(1e-3)).ceil() as usize)
                .clamp(16, 256);
            let pts: Vec<P> = (0..n)
                .map(|i| {
                    let a = std::f64::consts::TAU * i as f64 / n as f64;
                    p(-h + h * a.cos(), h * a.sin())
                })
                .collect();
            out.fills.push(polygon(&pts, at, dir));
        }
        Marker::OpenArrow | Marker::Bar => {
            let line =
                if m == Marker::Bar { vec![p(0.0, -h), p(0.0, h)] } else { vec![p(-l, -h), p(0.0, 0.0), p(-l, h)] };
            let mut q = polygon(&line, at, dir);
            q.closed = false;
            out.stroked.extend(stroke::stroke(&[q], style, tol));
        }
    }
}

/// Applies markers to the drawn `outline` (after trimming): the first point of the first subpath carries `start`, the last
/// point of the last subpath `end`, each only when that subpath is open. `size` is in stroke widths of `style`.
pub fn apply(outline: &[Poly], start: Marker, end: Marker, size: f64, style: &Style, tol: f64) -> Marked {
    let u = style.width;
    let l = size * u;
    let mut out = Marked { outline: outline.to_vec(), fills: Vec::new(), stroked: Vec::new() };
    let live = |q: &&Poly| q.pts.len() >= 2 && !q.closed;
    let first = out.outline.iter().position(|q| !q.pts.is_empty());
    let last = out.outline.iter().rposition(|q| !q.pts.is_empty());
    let total: f64 = out.outline.iter().map(|q| q.pts.windows(2).map(|w| len(w[0], w[1])).sum::<f64>()).sum();
    if u <= 0.0 || l <= 0.0 || total <= 0.0 {
        return out;
    }
    let mut jobs: Vec<(Marker, P, P, usize, bool)> = Vec::new();
    if let (Some(i), true) = (last, end != Marker::None) {
        if let Some(q) = [&out.outline[i]].into_iter().find(live) {
            if let Some(d) = end_direction(&q.pts) {
                jobs.push((end, *q.pts.last().unwrap(), d, i, true));
            }
        }
    }
    if let (Some(i), true) = (first, start != Marker::None) {
        if let Some(q) = [&out.outline[i]].into_iter().find(live) {
            let rev: Vec<P> = q.pts.iter().rev().copied().collect();
            if let Some(d) = end_direction(&rev) {
                jobs.push((start, q.pts[0], d, i, false));
            }
        }
    }
    for (m, at, dir, i, is_end) in jobs {
        marker(m, at, dir, l, style, tol, &mut out);
        let back = m.setback(l);
        if back > 0.0 {
            let q = &mut out.outline[i];
            if is_end {
                shorten_end(&mut q.pts, back);
            } else {
                q.pts.reverse();
                shorten_end(&mut q.pts, back);
                q.pts.reverse();
            }
        }
    }
    // a polyline shortened to nothing draws no stroke
    out.outline.retain(|q| q.pts.len() >= 2 || q.closed);
    // the polygons built here have positive area
    for q in &mut out.fills {
        if area(q) < 0.0 {
            q.pts.reverse();
        }
    }
    out
}
