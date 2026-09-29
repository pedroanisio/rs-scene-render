//! Filters, envelopes, interpolation and an FFT.

use std::f64::consts::PI;

/// Filter shapes (RBJ Audio EQ Cookbook).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Peaking EQ.
    Peak,
    /// Low shelf.
    LowShelf,
    /// High shelf.
    HighShelf,
    /// 2nd-order high-pass.
    HighPass,
    /// 2nd-order low-pass.
    LowPass,
    /// Notch.
    Notch,
    /// Band-pass (constant 0 dB peak).
    BandPass,
}

/// A biquad in transposed direct form II.
#[derive(Debug, Clone, Copy, Default)]
pub struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    /// Designs a filter.
    pub fn new(shape: Shape, rate: f64, freq: f64, q: f64, gain_db: f64) -> Biquad {
        let f = freq.clamp(1.0, rate * 0.49);
        let w = 2.0 * PI * f / rate;
        let (cw, sw) = (w.cos(), w.sin());
        let q = q.max(1e-3);
        let alpha = sw / (2.0 * q);
        let a = 10f64.powf(gain_db / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match shape {
            Shape::Peak => (1.0 + alpha * a, -2.0 * cw, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cw, 1.0 - alpha / a),
            Shape::LowShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cw + s),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cw),
                    a * ((a + 1.0) - (a - 1.0) * cw - s),
                    (a + 1.0) + (a - 1.0) * cw + s,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cw),
                    (a + 1.0) + (a - 1.0) * cw - s,
                )
            }
            Shape::HighShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cw + s),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cw),
                    a * ((a + 1.0) + (a - 1.0) * cw - s),
                    (a + 1.0) - (a - 1.0) * cw + s,
                    2.0 * ((a - 1.0) - (a + 1.0) * cw),
                    (a + 1.0) - (a - 1.0) * cw - s,
                )
            }
            Shape::HighPass => ((1.0 + cw) / 2.0, -(1.0 + cw), (1.0 + cw) / 2.0, 1.0 + alpha, -2.0 * cw, 1.0 - alpha),
            Shape::LowPass => ((1.0 - cw) / 2.0, 1.0 - cw, (1.0 - cw) / 2.0, 1.0 + alpha, -2.0 * cw, 1.0 - alpha),
            Shape::Notch => (1.0, -2.0 * cw, 1.0, 1.0 + alpha, -2.0 * cw, 1.0 - alpha),
            Shape::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cw, 1.0 - alpha),
        };
        Biquad { b: [b0 / a0, b1 / a0, b2 / a0], a: [a1 / a0, a2 / a0], z: [0.0; 2] }
    }

    /// From raw normalised coefficients.
    pub fn raw(b: [f64; 3], a: [f64; 2]) -> Biquad {
        Biquad { b, a, z: [0.0; 2] }
    }

    /// Filters one sample.
    #[inline]
    pub fn tick(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }

    /// Filters a buffer in place.
    pub fn run(&mut self, buf: &mut [f32]) {
        for v in buf {
            *v = self.tick(*v as f64) as f32;
        }
    }

    /// State magnitude (2⁻²⁰⁰) below which, fed zeros, every later output
    /// rounds to zero as an `f32` (which has no value below 2⁻¹⁴⁹; 2⁵⁰ is
    /// left for the transient growth of a second-order section, a few hundred
    /// at most for the sections designed here).
    pub const SETTLED_F32: f64 = 6.223015277861142e-61;

    /// State magnitude (2⁻⁶⁰⁰) below which, fed zeros, the square of every
    /// later output is exactly zero in `f64` (`y * y` underflows below 2⁻⁵³⁷;
    /// 2⁶³ is left for transient growth and a cascaded section).
    pub const SETTLED_SQUARE: f64 = 2.409919865102884e-181;

    /// Whether both state variables are below `below` in magnitude: fed zeros
    /// from here the output only decays further. (The state never reaches
    /// exactly zero: it settles in subnormal noise around 10⁻³²².)
    pub fn settled(&self, below: f64) -> bool {
        self.z[0].abs() < below && self.z[1].abs() < below
    }
}

/// One-pole smoothing coefficient for a time constant in seconds.
pub fn coeff(seconds: f64, rate: f64) -> f64 {
    if seconds <= 0.0 {
        0.0
    } else {
        (-1.0 / (seconds * rate)).exp()
    }
}

/// Decibels ↔ linear.
pub fn db_to_lin(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// Linear → decibels (floor −200 dB).
pub fn lin_to_db(x: f64) -> f64 {
    20.0 * x.max(1e-10).log10()
}

/// Cubic Hermite interpolation of `s` at fractional index `x`.
pub fn hermite(s: &[f32], x: f64) -> f32 {
    let i = x.floor() as i64;
    let f = (x - i as f64) as f32;
    let at = |k: i64| if k < 0 || k as usize >= s.len() { 0.0 } else { s[k as usize] };
    let (y0, y1, y2, y3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * f + c2) * f + c1) * f + y1
}

/// In-place radix-2 complex FFT (`inverse` scales by 1/n).
pub fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = 2.0 * PI / len as f64 * if inverse { 1.0 } else { -1.0 };
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
        }
        len <<= 1;
    }
    if inverse {
        for k in 0..n {
            re[k] /= n as f64;
            im[k] /= n as f64;
        }
    }
}

/// Periodic Hann window.
pub fn hann(n: usize) -> Vec<f64> {
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos()).collect()
}

/// Deterministic xorshift generator for dither and noise.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// Seeded generator.
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    /// Uniform in [0, 1).
    pub fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain_at(mut f: Biquad, freq: f64) -> f64 {
        let rate = 48000.0;
        let mut peak: f64 = 0.0;
        for i in 0..48000 {
            let y = f.tick((2.0 * PI * freq * i as f64 / rate).sin());
            if i > 24000 {
                peak = peak.max(y.abs());
            }
        }
        lin_to_db(peak)
    }

    #[test]
    fn filters_have_their_designed_responses() {
        let lp = Biquad::new(Shape::LowPass, 48000.0, 1000.0, std::f64::consts::FRAC_1_SQRT_2, 0.0);
        assert!(gain_at(lp, 100.0).abs() < 0.1);
        assert!((gain_at(lp, 1000.0) + 3.01).abs() < 0.1);
        assert!(gain_at(lp, 10000.0) < -38.0);
        let pk = Biquad::new(Shape::Peak, 48000.0, 2000.0, 1.0, 6.0);
        assert!((gain_at(pk, 2000.0) - 6.0).abs() < 0.05);
        let hs = Biquad::new(Shape::HighShelf, 48000.0, 1000.0, std::f64::consts::FRAC_1_SQRT_2, -12.0);
        assert!((gain_at(hs, 15000.0) + 12.0).abs() < 0.3);
    }

    #[test]
    fn settled_thresholds_are_the_documented_powers_of_two() {
        assert_eq!(Biquad::SETTLED_F32, 2f64.powi(-200));
        assert_eq!(Biquad::SETTLED_SQUARE, 2f64.powi(-600));
    }

    #[test]
    fn a_settled_section_fed_zeros_stays_silent() {
        // every section the analysis and loudness code runs, at the common rates: after a
        // burst, once the state is below the threshold no later output is non-zero as f32
        // (or squared, for the K-weighting), and the state never climbs back near it
        for rate in [44100.0, 48000.0, 96000.0, 192000.0] {
            let q = std::f64::consts::FRAC_1_SQRT_2;
            let bands = [
                Biquad::new(Shape::LowPass, rate, 250.0, q, 0.0),
                Biquad::new(Shape::HighPass, rate, 250.0, q, 0.0),
                Biquad::new(Shape::LowPass, rate, 4000.0, q, 0.0),
                Biquad::new(Shape::HighPass, rate, 4000.0, q, 0.0),
            ];
            let [shelf, hp] = crate::loudness::k_weighting(rate);
            let sections = bands
                .into_iter()
                .map(|f| (f, Biquad::SETTLED_F32, false))
                .chain([(shelf, Biquad::SETTLED_SQUARE, true), (hp, Biquad::SETTLED_SQUARE, true)]);
            for (mut f, below, square) in sections {
                let mut rng = Rng::new(7);
                for _ in 0..4800 {
                    f.tick(rng.uniform() * 2.0 - 1.0);
                }
                let mut n = 0;
                while !f.settled(below) {
                    f.tick(0.0);
                    n += 1;
                    assert!(n < 2_000_000, "never settles at {rate}");
                }
                for _ in 0..1_000_000 {
                    let y = f.tick(0.0);
                    if square {
                        assert_eq!(y * y, 0.0);
                    } else {
                        assert_eq!(y as f32, 0.0);
                    }
                    assert!(f.settled(below * 1048576.0), "state grew back within 2^20 of the threshold at {rate}");
                }
            }
        }
    }

    #[test]
    fn fft_round_trips_and_finds_a_bin() {
        let n = 64;
        let mut re: Vec<f64> = (0..n).map(|i| (2.0 * PI * 5.0 * i as f64 / n as f64).cos()).collect();
        let mut im = vec![0.0; n];
        let orig = re.clone();
        fft(&mut re, &mut im, false);
        assert!((re[5] - 32.0).abs() < 1e-9 && re[6].abs() < 1e-9);
        fft(&mut re, &mut im, true);
        assert!(re.iter().zip(&orig).all(|(a, b)| (a - b).abs() < 1e-12));
    }
}
