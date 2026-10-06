//! The nine shape modifiers, applied in document order to a shape's
//! geometry: repeater, offset-path, pucker-bloat, zig-zag, twist,
//! round-corners, wiggle-path, merge and trim.

use crate::geom::{p, Xf, P};
use crate::measure::{self, TrimMode};
use crate::path::{cubic_point, cubic_tangent, polys_to_path, Contour, Path, Poly};
use crate::scene::MaskOp;
use crate::stroke::{self, Join};

/// Items a repeater may produce (copies × items); further copies are dropped.
pub const MAX_REPEATER_ITEMS: usize = 65_536;
/// Zig-zag ridges per segment.
pub const MAX_RIDGES: u32 = 1024;
/// Points of a wiggled subpath.
pub const MAX_WIGGLE_POINTS: usize = 65_536;

/// One drawable piece of a shape: a path (or, after `merge`, several paths
/// combined by coverage) with an opacity multiplier.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// Paths and how each combines with those before it (the first is always `Add`).
    pub parts: Vec<(Path, MaskOp)>,
    /// Opacity multiplier (repeater copies fade).
    pub opacity: f64,
    /// Source tag (which shape the item came from), kept through modifiers.
    pub tag: u32,
}

impl Item {
    /// A single path at full opacity.
    pub fn new(path: Path) -> Item {
        Item { parts: vec![(path, MaskOp::Add)], opacity: 1.0, tag: 0 }
    }
    /// Whether the parts combine by more than plain union.
    pub fn is_compound(&self) -> bool {
        self.parts.len() > 1 && self.parts.iter().skip(1).any(|(_, op)| *op != MaskOp::Add)
    }
}

/// A modifier with its (animated) parameters resolved for this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Modifier {
    Repeater {
        copies: f64,
        offset: f64,
        offset_x: f64,
        offset_y: f64,
        rotation: f64,
        scale: f64,
        start_opacity: f64,
        end_opacity: f64,
        below: bool,
    },
    OffsetPath {
        amount: f64,
        join: Join,
        miter_limit: f64,
    },
    PuckerBloat {
        amount: f64,
    },
    ZigZag {
        size: f64,
        ridges: u32,
        smooth: bool,
    },
    Twist {
        amount: f64,
    },
    RoundCorners {
        radius: f64,
    },
    WigglePath {
        size: f64,
        detail: f64,
        frequency: f64,
        seed: u64,
        /// Join the wiggled points with a smooth curve through them instead of straight lines.
        smooth: bool,
    },
    Merge {
        op: MaskOp,
    },
    Trim {
        amount: f64,
        offset: f64,
        mode: TrimMode,
    },
}

/// A Catmull-Rom spline through `pts` as a contour of cubic segments (the tangent at a point is a sixth of the chord of its
/// neighbours; an open end uses itself as the missing neighbour).
fn spline_through(pts: &[P], closed: bool) -> Contour {
    let n = pts.len();
    let mut c = Contour { closed, ..Default::default() };
    for k in 0..n {
        let prev = if k > 0 {
            pts[k - 1]
        } else if closed {
            pts[n - 1]
        } else {
            pts[k]
        };
        let next = if k + 1 < n {
            pts[k + 1]
        } else if closed {
            pts[0]
        } else {
            pts[k]
        };
        let t = (next - prev) * (1.0 / 6.0);
        c.v.push(pts[k]);
        c.i.push(pts[k] - t);
        c.o.push(pts[k] + t);
    }
    c
}

/// Frame context of modifiers.
#[derive(Debug, Clone, Copy)]
pub struct Ctx {
    /// Centre of the shape's box (repeater and twist pivot).
    pub center: P,
    /// Seconds on the shape's timeline (wiggle-path).
    pub time: f64,
    /// Flattening tolerance.
    pub tol: f64,
}

fn map_paths(items: &mut [Item], f: &mut dyn FnMut(&Path) -> Path) {
    for it in items {
        for (path, _) in &mut it.parts {
            *path = f(path);
        }
    }
}

fn map_contours(path: &Path, f: &mut dyn FnMut(&mut Contour)) -> Path {
    let mut cs = path.contours();
    for c in &mut cs {
        f(c);
    }
    Path::from_contours(&cs)
}

/// Applies `m` to the shape's items.
pub fn apply(items: &mut Vec<Item>, m: &Modifier, ctx: &Ctx) {
    match *m {
        Modifier::Repeater {
            copies,
            offset,
            offset_x,
            offset_y,
            rotation,
            scale,
            start_opacity,
            end_opacity,
            below,
        } => {
            let n = (copies.max(0.0).floor() as usize).min(MAX_REPEATER_ITEMS / items.len().max(1));
            let step = |j: f64| -> Xf {
                let s = scale.max(1e-6).powf(j);
                Xf::translate(ctx.center.x + offset_x * j, ctx.center.y + offset_y * j)
                    .mul(&Xf::rotate(rotation * j))
                    .mul(&Xf::scale(s, s))
                    .mul(&Xf::translate(-ctx.center.x, -ctx.center.y))
            };
            let mut out = Vec::with_capacity(items.len() * n);
            for k in 0..n {
                let x = step(offset + k as f64);
                let u = if n > 1 { k as f64 / (n - 1) as f64 } else { 0.0 };
                let op = start_opacity + (end_opacity - start_opacity) * u;
                for it in items.iter() {
                    out.push(Item {
                        parts: it.parts.iter().map(|(pa, o)| (pa.transform(&x), *o)).collect(),
                        opacity: it.opacity * op,
                        tag: it.tag,
                    });
                }
            }
            if below {
                out.reverse();
            }
            *items = out;
        }
        Modifier::OffsetPath { amount, join, miter_limit } => {
            if amount.abs() < 1e-9 {
                return;
            }
            map_paths(items, &mut |path| {
                let polys = path.flatten(ctx.tol);
                let mut out = Vec::new();
                for q in polys.iter().filter(|q| q.closed && q.pts.len() > 2) {
                    let s = if q.area() > 0.0 { -amount.signum() } else { amount.signum() };
                    let style = stroke::Style { width: amount.abs() * 2.0, cap: stroke::Cap::Butt, join, miter_limit };
                    // one side of a stroke is the offset curve
                    let mut pts = q.pts.clone();
                    if s < 0.0 {
                        pts.reverse();
                    }
                    let side = stroke::stroke(&[Poly { pts, closed: true }], &style, ctx.tol);
                    if let Some(first) = side.into_iter().next() {
                        let mut r = first;
                        if (r.area() > 0.0) != (q.area() > 0.0) {
                            r.pts.reverse();
                        }
                        out.push(r);
                    }
                }
                out.extend(polys.into_iter().filter(|q| !q.closed));
                polys_to_path(&out)
            });
        }
        Modifier::PuckerBloat { amount } => {
            let a = amount / 100.0;
            map_paths(items, &mut |path| {
                map_contours(path, &mut |c| {
                    if c.v.is_empty() {
                        return;
                    }
                    let center = c.v.iter().fold(p(0.0, 0.0), |s, &q| s + q) * (1.0 / c.v.len() as f64);
                    for k in 0..c.v.len() {
                        c.v[k] = c.v[k] + (center - c.v[k]) * a;
                        c.o[k] = c.o[k] - (center - c.o[k]) * a;
                        c.i[k] = c.i[k] - (center - c.i[k]) * a;
                    }
                })
            });
        }
        Modifier::ZigZag { size, ridges, smooth } => {
            let r = ridges.clamp(1, MAX_RIDGES) as usize;
            map_paths(items, &mut |path| {
                map_contours(path, &mut |c| {
                    let n = c.v.len();
                    if n < 2 {
                        return;
                    }
                    let segs = if c.closed { n } else { n - 1 };
                    let mut out = Contour { closed: c.closed, ..Default::default() };
                    for k in 0..segs {
                        let j = (k + 1) % n;
                        let (a, b, cc, d) = (c.v[k], c.o[k], c.i[j], c.v[j]);
                        let len = a.dist(d).max(1e-9);
                        let h = if smooth { len / (4.0 * r as f64) } else { 0.0 };
                        for m in 0..2 * r {
                            let t = m as f64 / (2 * r) as f64;
                            let pt = cubic_point(a, b, cc, d, t);
                            let tg = cubic_tangent(a, b, cc, d, t).norm();
                            let sign = if m % 2 == 0 { 0.0 } else { 1.0 };
                            let q = pt + tg.perp() * (size * sign);
                            out.v.push(q);
                            out.i.push(q - tg * h);
                            out.o.push(q + tg * h);
                        }
                    }
                    if !c.closed {
                        let last = c.v[n - 1];
                        out.v.push(last);
                        out.i.push(last);
                        out.o.push(last);
                    }
                    *c = out;
                })
            });
        }
        Modifier::Twist { amount } => {
            let mut rmax: f64 = 0.0;
            for it in items.iter() {
                for (pa, _) in &it.parts {
                    for q in pa.flatten(ctx.tol) {
                        for &pt in &q.pts {
                            rmax = rmax.max(pt.dist(ctx.center));
                        }
                    }
                }
            }
            let rmax = rmax.max(1e-9);
            map_paths(items, &mut |path| {
                let polys: Vec<Poly> = path.flatten(ctx.tol).iter().map(|q| q.subdivide(4.0)).collect();
                let out: Vec<Poly> = polys
                    .into_iter()
                    .map(|q| Poly {
                        pts: q
                            .pts
                            .iter()
                            .map(|&pt| {
                                ctx.center + (pt - ctx.center).rot((amount * pt.dist(ctx.center) / rmax).to_radians())
                            })
                            .collect(),
                        closed: q.closed,
                    })
                    .collect();
                polys_to_path(&out)
            });
        }
        Modifier::RoundCorners { radius } => {
            if radius <= 0.0 {
                return;
            }
            const K: f64 = 0.5519;
            map_paths(items, &mut |path| {
                map_contours(path, &mut |c| {
                    let n = c.v.len();
                    if n < 2 {
                        return;
                    }
                    let mut out = Contour { closed: c.closed, ..Default::default() };
                    for k in 0..n {
                        let v = c.v[k];
                        let sharp = c.i[k].dist(v) < 1e-9 && c.o[k].dist(v) < 1e-9;
                        let interior = c.closed || (k > 0 && k + 1 < n);
                        if !sharp || !interior {
                            out.v.push(v);
                            out.i.push(c.i[k]);
                            out.o.push(c.o[k]);
                            continue;
                        }
                        let prev = c.v[(k + n - 1) % n];
                        let next = c.v[(k + 1) % n];
                        let r = radius.min(v.dist(prev) * 0.5).min(v.dist(next) * 0.5);
                        let a = v + (prev - v).norm() * r;
                        let b = v + (next - v).norm() * r;
                        out.v.push(a);
                        out.i.push(a);
                        out.o.push(a + (v - a) * K);
                        out.v.push(b);
                        out.i.push(b + (v - b) * K);
                        out.o.push(b);
                    }
                    *c = out;
                })
            });
        }
        Modifier::WigglePath { size, detail, frequency, seed, smooth } => {
            map_paths(items, &mut |path| {
                let polys = path.flatten(ctx.tol);
                let mut out = Vec::new();
                for (ci, q) in polys.iter().enumerate() {
                    let len = q.length();
                    let count = ((len * detail.max(0.0) / 100.0).round() as usize).clamp(3, MAX_WIGGLE_POINTS);
                    let pts = if q.closed {
                        let mut v = q.pts.clone();
                        v.push(v[0]);
                        v
                    } else {
                        q.pts.clone()
                    };
                    let segs = if q.closed { count } else { count - 1 };
                    let mut w = Vec::with_capacity(count);
                    for k in 0..count {
                        let s = len * k as f64 / segs.max(1) as f64;
                        if let Some((pt, tg)) = measure::at_length(&pts, s) {
                            let off = size * smooth_noise(seed, (ci * 7919 + k) as u64, ctx.time * frequency);
                            w.push(pt + tg.perp() * off);
                        }
                    }
                    out.push(Poly { pts: w, closed: q.closed });
                }
                if smooth {
                    Path::from_contours(&out.iter().map(|q| spline_through(&q.pts, q.closed)).collect::<Vec<_>>())
                } else {
                    polys_to_path(&out)
                }
            });
        }
        Modifier::Merge { op } => {
            let mut parts = Vec::new();
            let tag = items.first().map(|i| i.tag).unwrap_or(0);
            for it in items.drain(..) {
                for (k, (pa, o)) in it.parts.into_iter().enumerate() {
                    let o = if parts.is_empty() {
                        MaskOp::Add
                    } else if k == 0 {
                        op
                    } else {
                        o
                    };
                    parts.push((pa, o));
                }
            }
            if !parts.is_empty() {
                items.push(Item { parts, opacity: 1.0, tag });
            }
        }
        Modifier::Trim { amount, offset, mode } => {
            let end = 1.0 - (amount / 100.0).clamp(0.0, 1.0);
            let off = offset / 360.0;
            match mode {
                TrimMode::Simultaneous => map_paths(items, &mut |path| {
                    polys_to_path(&measure::trim(&path.flatten(ctx.tol), 0.0, end, off, TrimMode::Simultaneous))
                }),
                TrimMode::Sequential => {
                    // one outline across every path of the shape
                    let mut all: Vec<(usize, usize, Poly)> = Vec::new();
                    for (ii, it) in items.iter().enumerate() {
                        for (pi, (pa, _)) in it.parts.iter().enumerate() {
                            for q in pa.flatten(ctx.tol) {
                                all.push((ii, pi, q));
                            }
                        }
                    }
                    let lens: Vec<f64> = all.iter().map(|x| x.2.length()).collect();
                    let total: f64 = lens.iter().sum();
                    let trimmed_all = measure::trim(
                        &all.iter().map(|x| x.2.clone()).collect::<Vec<_>>(),
                        0.0,
                        end,
                        off,
                        TrimMode::Sequential,
                    );
                    // reassign pieces to their source path by arc-length order
                    let mut buckets: Vec<Vec<Vec<Poly>>> =
                        items.iter().map(|it| vec![Vec::new(); it.parts.len()]).collect();
                    let _ = total;
                    let mut src = 0usize;
                    for piece in trimmed_all {
                        let start = piece.pts[0];
                        while src + 1 < all.len() && !on_poly(&all[src].2, start) {
                            src += 1;
                        }
                        let (ii, pi) = (all[src.min(all.len() - 1)].0, all[src.min(all.len() - 1)].1);
                        buckets[ii][pi].push(piece);
                    }
                    for (it, b) in items.iter_mut().zip(buckets) {
                        for ((pa, _), polys) in it.parts.iter_mut().zip(b) {
                            *pa = polys_to_path(&polys);
                        }
                    }
                }
            }
        }
    }
}

fn on_poly(q: &Poly, pt: P) -> bool {
    let n = q.pts.len();
    let segs = if q.closed { n } else { n.saturating_sub(1) };
    (0..segs).any(|k| {
        let (a, b) = (q.pts[k], q.pts[(k + 1) % n]);
        let ab = b - a;
        let t = ((pt - a).dot(ab) / ab.dot(ab).max(1e-18)).clamp(0.0, 1.0);
        (a + ab * t).dist(pt) < 1e-6
    })
}

/// Smooth 1D value noise in [−1, 1], keyed by seed and channel: lattice values
/// 2 · U(seed, channel, k) − 1 of the seeded lattice hash (`d24::d24_unit`), smoothstep between.
pub fn smooth_noise(seed: u64, channel: u64, x: f64) -> f64 {
    let i = libm::floor(x);
    let f = x - i;
    let u = f * f * (3.0 - 2.0 * f);
    let h = |k: i64| -> f64 { crate::d24::d24_unit(seed, channel, k as u64) * 2.0 - 1.0 };
    let a = h(i as i64);
    let b = h(i as i64 + 1);
    a + (b - a) * u
}
