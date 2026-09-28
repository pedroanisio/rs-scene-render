//! Audiograms: bars, line, wave, circle and spectrum from the audio
//! analysis envelopes.

use sr_vector::geom::p;
use sr_vector::scene::{Cmd, FillRule, Paint};
use sr_vector::stroke::{self, Cap, Join, Style as StrokeStyle};
use sr_vector::{shapes, Poly};

use crate::glyph::Drawing;

/// Styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bars,
    Line,
    Wave,
    Circle,
    Spectrum,
}

impl Kind {
    /// Parses a style name (unknown names draw bars).
    pub fn parse(s: &str) -> Kind {
        match s {
            "line" => Kind::Line,
            "wave" => Kind::Wave,
            "circle" => Kind::Circle,
            "spectrum" => Kind::Spectrum,
            _ => Kind::Bars,
        }
    }
}

/// Draws an audiogram. `history` holds full-band levels in [0, 1], oldest
/// first; `bands` the current low, mid and high levels.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    kind: Kind,
    bars: usize,
    size: [f64; 2],
    color: Paint,
    history: &[f64],
    bands: [f64; 3],
    smoothing: f64,
    tol: f64,
) -> Drawing {
    let mut d = Drawing::default();
    let [w, h] = size;
    let bars = bars.max(2);
    // exponential smoothing along the history
    let mut sm = Vec::with_capacity(history.len());
    let mut acc = history.first().copied().unwrap_or(0.0);
    let k = smoothing.clamp(0.0, 0.99);
    for &v in history {
        acc = acc * k + v * (1.0 - k);
        sm.push(acc.clamp(0.0, 1.0));
    }
    let level = |i: usize| -> f64 {
        // the last `bars` samples, newest at the right
        let n = sm.len();
        if n == 0 {
            return 0.0;
        }
        let j = (n as i64 - bars as i64 + i as i64).clamp(0, n as i64 - 1) as usize;
        sm[j]
    };
    let spectrum = |i: usize| -> f64 {
        let u = i as f64 / (bars - 1) as f64 * 2.0;
        let (a, b, t) = if u < 1.0 { (bands[0], bands[1], u) } else { (bands[1], bands[2], u - 1.0) };
        (a + (b - a) * t).clamp(0.0, 1.0)
    };
    let fill = |d: &mut Drawing, polys: Vec<Poly>| {
        d.scene.cmds.push(Cmd::Fill { polys, rule: FillRule::NonZero, paint: color.clone(), opacity: 1.0 })
    };
    match kind {
        Kind::Bars | Kind::Spectrum => {
            let slot = w / bars as f64;
            for i in 0..bars {
                let v = if kind == Kind::Spectrum { spectrum(i) } else { level(i) };
                let bh = (h * v).max(1.0);
                let bw = slot * 0.7;
                fill(&mut d, shapes::rect(i as f64 * slot + slot * 0.15, h - bh, bw, bh, [bw * 0.3; 4]).flatten(tol));
            }
        }
        Kind::Line => {
            let pts: Vec<_> = (0..bars).map(|i| p(w * i as f64 / (bars - 1) as f64, h * (1.0 - level(i)))).collect();
            fill(
                &mut d,
                stroke::stroke(
                    &[Poly { pts, closed: false }],
                    &StrokeStyle { width: (h * 0.03).max(1.5), cap: Cap::Round, join: Join::Round, miter_limit: 4.0 },
                    tol,
                ),
            );
        }
        Kind::Wave => {
            let mid = h * 0.5;
            let top: Vec<_> = (0..bars).map(|i| p(w * i as f64 / (bars - 1) as f64, mid - mid * level(i))).collect();
            let mut pts = top.clone();
            pts.extend(top.iter().rev().map(|q| p(q.x, 2.0 * mid - q.y + 1.0)));
            fill(&mut d, vec![Poly { pts, closed: true }]);
        }
        Kind::Circle => {
            let c = p(w * 0.5, h * 0.5);
            let r0 = w.min(h) * 0.25;
            let len = w.min(h) * 0.25;
            let mut polys = Vec::new();
            for i in 0..bars {
                let a = std::f64::consts::TAU * i as f64 / bars as f64 - std::f64::consts::FRAC_PI_2;
                let dir = p(libm::cos(a), libm::sin(a));
                let a0 = c + dir * r0;
                let a1 = c + dir * (r0 + len * level(i).max(0.02));
                polys.extend(stroke::stroke(
                    &[Poly { pts: vec![a0, a1], closed: false }],
                    &StrokeStyle {
                        width: (std::f64::consts::TAU * r0 / bars as f64 * 0.6).max(1.0),
                        cap: Cap::Round,
                        join: Join::Round,
                        miter_limit: 4.0,
                    },
                    tol,
                ));
            }
            fill(&mut d, polys);
        }
    }
    d
}
