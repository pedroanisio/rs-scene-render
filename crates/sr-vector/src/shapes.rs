//! Shape primitives in their box `[0, w] × [0, h]`.

use crate::geom::{p, P};
use crate::path::{Contour, Path};

/// Circle-arc handle ratio for a quarter turn.
pub const KAPPA: f64 = 0.552_284_749_830_793_4;

/// Points of a star or polygon.
pub const MAX_POINTS: u32 = 10_000;

/// Rectangle with per-corner radii (top-left, top-right, bottom-right, bottom-left), clamped to fit.
pub fn rect(x: f64, y: f64, w: f64, h: f64, radii: [f64; 4]) -> Path {
    let mut r = radii.map(|v| v.max(0.0));
    // CSS rule: scale every radius down until adjacent radii fit their side
    let f = [w / (r[0] + r[1]), h / (r[1] + r[2]), w / (r[2] + r[3]), h / (r[3] + r[0])]
        .into_iter()
        .filter(|v| v.is_finite())
        .fold(1.0f64, f64::min);
    r = r.map(|v| v * f.min(1.0));
    let mut path = Path::default();
    let k = 1.0 - KAPPA;
    path.move_to(p(x + r[0], y));
    path.line_to(p(x + w - r[1], y));
    if r[1] > 0.0 {
        path.cubic_to(p(x + w - r[1] * k, y), p(x + w, y + r[1] * k), p(x + w, y + r[1]));
    }
    path.line_to(p(x + w, y + h - r[2]));
    if r[2] > 0.0 {
        path.cubic_to(p(x + w, y + h - r[2] * k), p(x + w - r[2] * k, y + h), p(x + w - r[2], y + h));
    }
    path.line_to(p(x + r[3], y + h));
    if r[3] > 0.0 {
        path.cubic_to(p(x + r[3] * k, y + h), p(x, y + h - r[3] * k), p(x, y + h - r[3]));
    }
    path.line_to(p(x, y + r[0]));
    if r[0] > 0.0 {
        path.cubic_to(p(x, y + r[0] * k), p(x + r[0] * k, y), p(x + r[0], y));
    }
    path.close();
    path
}

/// Ellipse about (cx, cy), starting at 3 o'clock and running clockwise on screen, as SVG 2 draws
/// `<ellipse>` (trims and dashes start there).
pub fn ellipse(cx: f64, cy: f64, rx: f64, ry: f64) -> Path {
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let mut path = Path::default();
    path.move_to(p(cx + rx, cy));
    path.cubic_to(p(cx + rx, cy + ky), p(cx + kx, cy + ry), p(cx, cy + ry));
    path.cubic_to(p(cx - kx, cy + ry), p(cx - rx, cy + ky), p(cx - rx, cy));
    path.cubic_to(p(cx - rx, cy - ky), p(cx - kx, cy - ry), p(cx, cy - ry));
    path.cubic_to(p(cx + kx, cy - ry), p(cx + rx, cy - ky), p(cx + rx, cy));
    path.close();
    path
}

/// Ellipse about (cx, cy), clockwise from the top (Lottie's ellipse).
pub fn ellipse_top(cx: f64, cy: f64, rx: f64, ry: f64) -> Path {
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let mut path = Path::default();
    path.move_to(p(cx, cy - ry));
    path.cubic_to(p(cx + kx, cy - ry), p(cx + rx, cy - ky), p(cx + rx, cy));
    path.cubic_to(p(cx + rx, cy + ky), p(cx + kx, cy + ry), p(cx, cy + ry));
    path.cubic_to(p(cx - kx, cy + ry), p(cx - rx, cy + ky), p(cx - rx, cy));
    path.cubic_to(p(cx - rx, cy - ky), p(cx - kx, cy - ry), p(cx, cy - ry));
    path.close();
    path
}

/// Star with `points` tips (Lottie and After Effects geometry): outer and
/// inner radii, roundness 0–1 as tangent length relative to the vertex
/// spacing, first tip straight up, rotated by `rot_deg`.
pub fn star(c: P, points: u32, outer: f64, inner: f64, outer_round: f64, inner_round: f64, rot_deg: f64) -> Path {
    star_on(c, points, p(outer, outer), p(inner, inner), outer_round, inner_round, rot_deg)
}

/// Regular polygon (Lottie polystar with one radius).
pub fn polygon(c: P, points: u32, r: f64, roundness: f64, rot_deg: f64) -> Path {
    polygon_on(c, points, p(r, r), roundness, rot_deg)
}

/// [`star`] with its tips on the ellipse of radii `outer` and its inner vertices on the ellipse of
/// radii `inner` (a star in a non-square box); the roundness handles are
/// tangent to those ellipses.
pub fn star_on(c: P, points: u32, outer: P, inner: P, outer_round: f64, inner_round: f64, rot_deg: f64) -> Path {
    let n = points.clamp(2, MAX_POINTS) as usize * 2;
    let angle = std::f64::consts::TAU / n as f64;
    // handle length relative to the ellipse's derivative (the vertex spacing over the radius on a circle)
    let k = std::f64::consts::TAU / (n as f64 * 2.0);
    let mut cn = Contour { closed: true, ..Default::default() };
    let mut a = -std::f64::consts::FRAC_PI_2 + rot_deg.to_radians();
    for i in 0..n {
        let (r, round) = if i % 2 == 0 { (outer, outer_round) } else { (inner, inner_round) };
        vertex(&mut cn, c, r, a, round * k);
        a += angle;
    }
    Path::from_contours(&[cn])
}

/// [`polygon`] with its vertices on the ellipse of radii `r`.
pub fn polygon_on(c: P, points: u32, r: P, roundness: f64, rot_deg: f64) -> Path {
    let n = points.clamp(3, MAX_POINTS) as usize;
    let angle = std::f64::consts::TAU / n as f64;
    let k = std::f64::consts::TAU / (n as f64 * 4.0);
    let mut cn = Contour { closed: true, ..Default::default() };
    let mut a = -std::f64::consts::FRAC_PI_2 + rot_deg.to_radians();
    for _ in 0..n {
        vertex(&mut cn, c, r, a, roundness * k);
        a += angle;
    }
    Path::from_contours(&[cn])
}

/// A vertex at angle `a` on the ellipse of radii `r` about `c`, with handles `k` times the
/// ellipse's derivative there.
fn vertex(cn: &mut Contour, c: P, r: P, a: f64, k: f64) {
    let (s, co) = (libm::sin(a), libm::cos(a));
    let v = p(r.x * co, r.y * s);
    let t = p(-r.x * s, r.y * co);
    cn.v.push(c + v);
    cn.o.push(c + v + t * k);
    cn.i.push(c + v - t * k);
}

/// Line across the middle of its box, left to right.
pub fn line(w: f64, h: f64) -> Path {
    let mut path = Path::default();
    path.move_to(p(0.0, h * 0.5));
    path.line_to(p(w, h * 0.5));
    path
}
