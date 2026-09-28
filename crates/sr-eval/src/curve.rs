//! The 41 interpolation curves of `curveType`.
//!
//! A curve maps segment progress `u ∈ [0, 1]` to eased progress. Most curves
//! are pure easing functions (`step`, `steps`, `linear`, the CSS keywords,
//! `cubic-bezier` and the 30 Penner curves). Three need more context and are
//! evaluated by the channel: `catmull-rom` and `tcb` interpolate in value
//! space from neighbouring keys, and `spring` runs in seconds.
//!
//! All transcendental functions come from `libm`, so results are bit-exact
//! across platforms.

use std::f64::consts::PI;

use sr_model::model::Curve;

/// A curve with its parameters resolved from key attributes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ease {
    /// Hold the start value until the next key (`step`, `hold`).
    Hold,
    /// `steps`: `n` jumps; `start` jumps at the beginning of each interval.
    Steps {
        /// Number of steps.
        n: u32,
        /// CSS `jump-start` when true, `jump-end` otherwise.
        start: bool,
    },
    /// `linear`.
    Linear,
    /// A cubic Bézier timing function with control points (x1, y1), (x2, y2).
    Bezier(f64, f64, f64, f64),
    /// One of the Penner curves.
    Penner(Penner),
    /// `catmull-rom`: value-space spline through neighbouring keys.
    CatmullRom,
    /// `tcb`: Kochanek–Bartels spline; parameters live on the keys.
    Tcb,
    /// `spring`: damped harmonic motion, in seconds.
    Spring {
        /// Stiffness k.
        stiffness: f64,
        /// Damping c.
        damping: f64,
        /// Mass m.
        mass: f64,
    },
}

/// The Penner easing family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Penner {
    SineIn,
    SineOut,
    SineInOut,
    QuadIn,
    QuadOut,
    QuadInOut,
    CubicIn,
    CubicOut,
    CubicInOut,
    QuartIn,
    QuartOut,
    QuartInOut,
    QuintIn,
    QuintOut,
    QuintInOut,
    ExpoIn,
    ExpoOut,
    ExpoInOut,
    CircIn,
    CircOut,
    CircInOut,
    BackIn,
    BackOut,
    BackInOut,
    ElasticIn,
    ElasticOut,
    ElasticInOut,
    BounceIn,
    BounceOut,
    BounceInOut,
}

/// CSS `ease-in`.
pub const EASE_IN: Ease = Ease::Bezier(0.42, 0.0, 1.0, 1.0);
/// CSS `ease-out`.
pub const EASE_OUT: Ease = Ease::Bezier(0.0, 0.0, 0.58, 1.0);
/// CSS `ease-in-out`.
pub const EASE_IN_OUT: Ease = Ease::Bezier(0.42, 0.0, 0.58, 1.0);

/// Key parameters a curve may read.
#[derive(Debug, Clone, Copy)]
pub struct KeyParams {
    /// `@bezier` parsed as x1,y1,x2,y2.
    pub bezier: Option<[f64; 4]>,
    /// `@easeOut` of this key: (influence, speed).
    pub ease_out: Option<[f64; 2]>,
    /// `@easeIn` of the next key: (influence, speed).
    pub next_ease_in: Option<[f64; 2]>,
    /// `@steps`.
    pub steps: Option<u32>,
    /// `@stepPosition="start"`.
    pub step_start: bool,
    /// Spring parameters (stiffness, damping, mass).
    pub spring: [f64; 3],
}

impl Default for KeyParams {
    fn default() -> Self {
        KeyParams {
            bezier: None,
            ease_out: None,
            next_ease_in: None,
            steps: None,
            step_start: false,
            spring: [100.0, 10.0, 1.0],
        }
    }
}

/// Resolves a schema curve with key parameters into an [`Ease`].
pub fn resolve(c: Curve, k: &KeyParams) -> Ease {
    use Penner::*;
    let p = Ease::Penner;
    match c {
        Curve::Step | Curve::Hold => Ease::Hold,
        Curve::Steps => Ease::Steps { n: k.steps.unwrap_or(1).max(1), start: k.step_start },
        Curve::Linear => Ease::Linear,
        Curve::EaseIn => EASE_IN,
        Curve::EaseOut => EASE_OUT,
        Curve::EaseInOut => EASE_IN_OUT,
        Curve::CubicBezier => {
            if let Some([x1, y1, x2, y2]) = k.bezier {
                Ease::Bezier(x1, y1, x2, y2)
            } else {
                // After Effects temporal handles: influence along x, influence × speed along y.
                let [io, so] = k.ease_out.unwrap_or([1.0 / 3.0, 1.0]);
                let [ii, si] = k.next_ease_in.unwrap_or([1.0 / 3.0, 1.0]);
                Ease::Bezier(io.clamp(0.0, 1.0), io * so, 1.0 - ii.clamp(0.0, 1.0), 1.0 - ii * si)
            }
        }
        Curve::CatmullRom => Ease::CatmullRom,
        Curve::Tcb => Ease::Tcb,
        Curve::Spring => Ease::Spring { stiffness: k.spring[0], damping: k.spring[1], mass: k.spring[2] },
        Curve::SineIn => p(SineIn),
        Curve::SineOut => p(SineOut),
        Curve::SineInOut => p(SineInOut),
        Curve::QuadIn => p(QuadIn),
        Curve::QuadOut => p(QuadOut),
        Curve::QuadInOut => p(QuadInOut),
        Curve::CubicIn => p(CubicIn),
        Curve::CubicOut => p(CubicOut),
        Curve::CubicInOut => p(CubicInOut),
        Curve::QuartIn => p(QuartIn),
        Curve::QuartOut => p(QuartOut),
        Curve::QuartInOut => p(QuartInOut),
        Curve::QuintIn => p(QuintIn),
        Curve::QuintOut => p(QuintOut),
        Curve::QuintInOut => p(QuintInOut),
        Curve::ExpoIn => p(ExpoIn),
        Curve::ExpoOut => p(ExpoOut),
        Curve::ExpoInOut => p(ExpoInOut),
        Curve::CircIn => p(CircIn),
        Curve::CircOut => p(CircOut),
        Curve::CircInOut => p(CircInOut),
        Curve::BackIn => p(BackIn),
        Curve::BackOut => p(BackOut),
        Curve::BackInOut => p(BackInOut),
        Curve::ElasticIn => p(ElasticIn),
        Curve::ElasticOut => p(ElasticOut),
        Curve::ElasticInOut => p(ElasticInOut),
        Curve::BounceIn => p(BounceIn),
        Curve::BounceOut => p(BounceOut),
        Curve::BounceInOut => p(BounceInOut),
    }
}

/// Parses `@bezier` ("x1,y1,x2,y2"). x1 and x2 must lie in [0, 1].
pub fn parse_bezier(s: &str) -> Option<[f64; 4]> {
    let v: Vec<f64> =
        s.split([',', ' ']).filter(|t| !t.is_empty()).map(|t| t.trim().parse().ok()).collect::<Option<_>>()?;
    if v.len() != 4 || !(0.0..=1.0).contains(&v[0]) || !(0.0..=1.0).contains(&v[2]) || v.iter().any(|x| !x.is_finite())
    {
        return None;
    }
    Some([v[0], v[1], v[2], v[3]])
}

impl Ease {
    /// Eased progress for easing curves. Value-space curves (Catmull-Rom,
    /// TCB) return `u`; springs need seconds and use [`spring_segment`].
    pub fn apply(self, u: f64) -> f64 {
        let u = u.clamp(0.0, 1.0);
        match self {
            Ease::Hold => {
                if u >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Ease::Steps { n, start } => {
                let n = n as f64;
                let k = if start { libm::floor(u * n) + 1.0 } else { libm::floor(u * n) };
                (k / n).min(1.0)
            }
            Ease::Linear | Ease::CatmullRom | Ease::Tcb | Ease::Spring { .. } => u,
            Ease::Bezier(x1, y1, x2, y2) => cubic_bezier(x1, y1, x2, y2, u),
            Ease::Penner(p) => penner(p, u),
        }
    }

    /// True for curves that hold the start value until the next key.
    pub fn is_hold(self) -> bool {
        matches!(self, Ease::Hold)
    }
}

/// CSS cubic-bezier timing function: solves x(s) = u, returns y(s).
pub fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, u: f64) -> f64 {
    if u <= 0.0 {
        return 0.0;
    }
    if u >= 1.0 {
        return 1.0;
    }
    let (cx, bx) = (3.0 * x1, 3.0 * (x2 - x1) - 3.0 * x1);
    let ax = 1.0 - cx - bx;
    let (cy, by) = (3.0 * y1, 3.0 * (y2 - y1) - 3.0 * y1);
    let ay = 1.0 - cy - by;
    let x = |s: f64| ((ax * s + bx) * s + cx) * s;
    let dx = |s: f64| (3.0 * ax * s + 2.0 * bx) * s + cx;
    // Newton-Raphson from s = u, falling back to bisection.
    let mut s = u;
    for _ in 0..8 {
        let err = x(s) - u;
        if err.abs() < 1e-12 {
            return ((ay * s + by) * s + cy) * s;
        }
        let d = dx(s);
        if d.abs() < 1e-9 {
            break;
        }
        s -= err / d;
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    s = u;
    for _ in 0..64 {
        let xs = x(s);
        if (xs - u).abs() < 1e-12 {
            break;
        }
        if xs < u {
            lo = s;
        } else {
            hi = s;
        }
        s = 0.5 * (lo + hi);
    }
    ((ay * s + by) * s + cy) * s
}

fn bounce_out(u: f64) -> f64 {
    const N1: f64 = 7.5625;
    const D1: f64 = 2.75;
    if u < 1.0 / D1 {
        N1 * u * u
    } else if u < 2.0 / D1 {
        let v = u - 1.5 / D1;
        N1 * v * v + 0.75
    } else if u < 2.5 / D1 {
        let v = u - 2.25 / D1;
        N1 * v * v + 0.9375
    } else {
        let v = u - 2.625 / D1;
        N1 * v * v + 0.984375
    }
}

fn pow(a: f64, b: f64) -> f64 {
    libm::pow(a, b)
}

fn power_in_out(u: f64, n: i32) -> f64 {
    if u < 0.5 {
        pow(2.0, (n - 1) as f64) * u.powi(n)
    } else {
        1.0 - (-2.0 * u + 2.0).powi(n) / 2.0
    }
}

/// The Penner easing equations (easings.net definitions).
pub fn penner(p: Penner, u: f64) -> f64 {
    use Penner::*;
    const C1: f64 = 1.70158;
    const C2: f64 = C1 * 1.525;
    const C3: f64 = C1 + 1.0;
    const C4: f64 = 2.0 * PI / 3.0;
    const C5: f64 = 2.0 * PI / 4.5;
    match p {
        SineIn => 1.0 - libm::cos(u * PI / 2.0),
        SineOut => libm::sin(u * PI / 2.0),
        SineInOut => -(libm::cos(PI * u) - 1.0) / 2.0,
        QuadIn => u * u,
        QuadOut => 1.0 - (1.0 - u) * (1.0 - u),
        QuadInOut => power_in_out(u, 2),
        CubicIn => u.powi(3),
        CubicOut => 1.0 - (1.0 - u).powi(3),
        CubicInOut => power_in_out(u, 3),
        QuartIn => u.powi(4),
        QuartOut => 1.0 - (1.0 - u).powi(4),
        QuartInOut => power_in_out(u, 4),
        QuintIn => u.powi(5),
        QuintOut => 1.0 - (1.0 - u).powi(5),
        QuintInOut => power_in_out(u, 5),
        ExpoIn => {
            if u == 0.0 {
                0.0
            } else {
                pow(2.0, 10.0 * u - 10.0)
            }
        }
        ExpoOut => {
            if u == 1.0 {
                1.0
            } else {
                1.0 - pow(2.0, -10.0 * u)
            }
        }
        ExpoInOut => {
            if u == 0.0 {
                0.0
            } else if u == 1.0 {
                1.0
            } else if u < 0.5 {
                pow(2.0, 20.0 * u - 10.0) / 2.0
            } else {
                (2.0 - pow(2.0, -20.0 * u + 10.0)) / 2.0
            }
        }
        CircIn => 1.0 - libm::sqrt(1.0 - u * u),
        CircOut => libm::sqrt(1.0 - (u - 1.0) * (u - 1.0)),
        CircInOut => {
            if u < 0.5 {
                (1.0 - libm::sqrt(1.0 - (2.0 * u) * (2.0 * u))) / 2.0
            } else {
                (libm::sqrt(1.0 - (-2.0 * u + 2.0) * (-2.0 * u + 2.0)) + 1.0) / 2.0
            }
        }
        BackIn => C3 * u * u * u - C1 * u * u,
        BackOut => 1.0 + C3 * (u - 1.0).powi(3) + C1 * (u - 1.0).powi(2),
        BackInOut => {
            if u < 0.5 {
                ((2.0 * u).powi(2) * ((C2 + 1.0) * 2.0 * u - C2)) / 2.0
            } else {
                ((2.0 * u - 2.0).powi(2) * ((C2 + 1.0) * (u * 2.0 - 2.0) + C2) + 2.0) / 2.0
            }
        }
        ElasticIn => {
            if u == 0.0 || u == 1.0 {
                u
            } else {
                -pow(2.0, 10.0 * u - 10.0) * libm::sin((u * 10.0 - 10.75) * C4)
            }
        }
        ElasticOut => {
            if u == 0.0 || u == 1.0 {
                u
            } else {
                pow(2.0, -10.0 * u) * libm::sin((u * 10.0 - 0.75) * C4) + 1.0
            }
        }
        ElasticInOut => {
            if u == 0.0 || u == 1.0 {
                u
            } else if u < 0.5 {
                -(pow(2.0, 20.0 * u - 10.0) * libm::sin((20.0 * u - 11.125) * C5)) / 2.0
            } else {
                (pow(2.0, -20.0 * u + 10.0) * libm::sin((20.0 * u - 11.125) * C5)) / 2.0 + 1.0
            }
        }
        BounceIn => 1.0 - bounce_out(1.0 - u),
        BounceOut => bounce_out(u),
        BounceInOut => {
            if u < 0.5 {
                (1.0 - bounce_out(1.0 - 2.0 * u)) / 2.0
            } else {
                (1.0 + bounce_out(2.0 * u - 1.0)) / 2.0
            }
        }
    }
}

/// Step response of a damped spring released from 0 toward 1, `tau` seconds
/// after release. Underdamped springs overshoot; critically and over-damped
/// springs approach 1 monotonically.
pub fn spring(tau: f64, stiffness: f64, damping: f64, mass: f64) -> f64 {
    if tau <= 0.0 {
        return 0.0;
    }
    let (k, c, m) = (stiffness.max(1e-9), damping.max(0.0), mass.max(1e-9));
    let w0 = libm::sqrt(k / m);
    let zeta = c / (2.0 * libm::sqrt(k * m));
    if zeta < 1.0 - 1e-9 {
        let wd = w0 * libm::sqrt(1.0 - zeta * zeta);
        1.0 - libm::exp(-zeta * w0 * tau) * (libm::cos(wd * tau) + (zeta * w0 / wd) * libm::sin(wd * tau))
    } else if zeta <= 1.0 + 1e-9 {
        1.0 - libm::exp(-w0 * tau) * (1.0 + w0 * tau)
    } else {
        let s = libm::sqrt(zeta * zeta - 1.0);
        let r1 = -w0 * (zeta - s);
        let r2 = -w0 * (zeta + s);
        1.0 - (r2 * libm::exp(r1 * tau) - r1 * libm::exp(r2 * tau)) / (r2 - r1)
    }
}

/// Spring progress across a key segment of `duration` seconds, `tau` seconds
/// in. The residual `1 − spring(duration)` is distributed linearly over the
/// segment, so the value reaches the next key exactly at its time.
pub fn spring_segment(tau: f64, duration: f64, stiffness: f64, damping: f64, mass: f64) -> f64 {
    if duration <= 0.0 {
        return 1.0;
    }
    let tau = tau.clamp(0.0, duration);
    let end = spring(duration, stiffness, damping, mass);
    spring(tau, stiffness, damping, mass) + (1.0 - end) * tau / duration
}

/// Cubic Hermite basis for progress `u`: (h00, h10, h01, h11).
#[inline]
pub fn hermite(u: f64) -> [f64; 4] {
    let u2 = u * u;
    let u3 = u2 * u;
    [2.0 * u3 - 3.0 * u2 + 1.0, u3 - 2.0 * u2 + u, -2.0 * u3 + 3.0 * u2, u3 - u2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_curve_starts_at_zero_and_ends_at_one() {
        for &c in Curve::ALL {
            let e = resolve(c, &KeyParams { steps: Some(4), ..Default::default() });
            assert!(e.apply(0.0).abs() < 1e-9 || matches!(e, Ease::Steps { start: true, .. }), "{c} at 0");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-9, "{c} at 1: {}", e.apply(1.0));
            for i in 0..=100 {
                assert!(e.apply(i as f64 / 100.0).is_finite(), "{c}");
            }
        }
        assert_eq!(Curve::ALL.len(), 41);
    }

    #[test]
    fn known_values() {
        assert!((cubic_bezier(0.42, 0.0, 0.58, 1.0, 0.5) - 0.5).abs() < 1e-9);
        assert!((cubic_bezier(0.25, 0.1, 0.25, 1.0, 0.25) - 0.4085).abs() < 2e-3);
        assert!((penner(Penner::QuadIn, 0.5) - 0.25).abs() < 1e-12);
        assert!((penner(Penner::BounceOut, 0.5) - 0.765625).abs() < 1e-9);
        assert!(penner(Penner::BackIn, 0.3) < 0.0, "back-in overshoots below 0");
        assert!(penner(Penner::ElasticOut, 0.2) > 1.0, "elastic-out overshoots above 1");
        assert_eq!(Ease::Steps { n: 4, start: false }.apply(0.3), 0.25);
        assert_eq!(Ease::Steps { n: 4, start: true }.apply(0.3), 0.5);
        assert_eq!(Ease::Hold.apply(0.99), 0.0);
    }

    #[test]
    fn springs() {
        assert_eq!(spring(0.0, 100.0, 10.0, 1.0), 0.0);
        let peak = (1..200).map(|i| spring(i as f64 * 0.005, 100.0, 10.0, 1.0)).fold(0.0, f64::max);
        assert!(peak > 1.1, "underdamped spring overshoots: {peak}");
        assert!((spring(10.0, 100.0, 10.0, 1.0) - 1.0).abs() < 1e-9);
        assert!((spring(10.0, 100.0, 20.0, 1.0) - 1.0).abs() < 1e-6, "critical");
        assert!((spring(10.0, 100.0, 60.0, 1.0) - 1.0).abs() < 1e-3, "overdamped");
        assert!((spring_segment(0.2, 0.2, 100.0, 10.0, 1.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn bezier_strings() {
        assert_eq!(parse_bezier("0.25,0.1,0.25,1"), Some([0.25, 0.1, 0.25, 1.0]));
        assert_eq!(parse_bezier("0.1, -0.5, 0.9, 1.5"), Some([0.1, -0.5, 0.9, 1.5]));
        assert!(parse_bezier("1.5,0,0,1").is_none());
        assert!(parse_bezier("0,0,1").is_none());
    }
}
