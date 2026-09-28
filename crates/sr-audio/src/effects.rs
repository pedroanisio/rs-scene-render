//! The 16 audio effects of `audioEffectType`, processed offline on whole buffers.

use crate::dsp::{coeff, db_to_lin, fft, hann, lin_to_db, Biquad, Shape};
use crate::loudness::Planar;

/// Effect kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Eq,
    HighPass,
    LowPass,
    Compressor,
    Limiter,
    Gate,
    DeEsser,
    Reverb,
    Delay,
    Chorus,
    PitchShift,
    NoiseReduction,
    StereoWidth,
    Gain,
    Distortion,
    Telephone,
}

impl Kind {
    /// Parses the schema name.
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "eq" => Kind::Eq,
            "highpass" => Kind::HighPass,
            "lowpass" => Kind::LowPass,
            "compressor" => Kind::Compressor,
            "limiter" => Kind::Limiter,
            "gate" => Kind::Gate,
            "de-esser" => Kind::DeEsser,
            "reverb" => Kind::Reverb,
            "delay" => Kind::Delay,
            "chorus" => Kind::Chorus,
            "pitch-shift" => Kind::PitchShift,
            "noise-reduction" => Kind::NoiseReduction,
            "stereo-width" => Kind::StereoWidth,
            "gain" => Kind::Gain,
            "distortion" => Kind::Distortion,
            "telephone" => Kind::Telephone,
            _ => return None,
        })
    }
}

/// One EQ band.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    /// Filter shape.
    pub shape: Shape,
    /// Centre or corner frequency.
    pub frequency: f64,
    /// Gain in dB (peak and shelves).
    pub gain: f64,
    /// Quality factor.
    pub q: f64,
}

/// Parameters of one effect instance (schema attributes with their defaults).
#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    pub kind: Kind,
    pub enabled: bool,
    pub mix: f64,
    pub frequency: Option<f64>,
    pub gain: f64,
    pub threshold: f64,
    pub ratio: f64,
    pub attack: f64,
    pub release: f64,
    pub knee: f64,
    pub time: Option<f64>,
    pub feedback: f64,
    pub room_size: f64,
    pub width: f64,
    pub semitones: f64,
    pub amount: f64,
    pub bands: Vec<Band>,
    pub sidechain: Option<String>,
}

impl Effect {
    /// An effect with the schema's default attributes.
    pub fn new(kind: Kind) -> Effect {
        Effect {
            kind,
            enabled: true,
            mix: 1.0,
            frequency: None,
            gain: 0.0,
            threshold: -18.0,
            ratio: 4.0,
            attack: 0.01,
            release: 0.1,
            knee: 3.0,
            time: None,
            feedback: 0.3,
            room_size: 0.5,
            width: 1.0,
            semitones: 0.0,
            amount: 0.5,
            bands: Vec::new(),
            sidechain: None,
        }
    }
}

/// Soft-knee static curve (Giannoulis, Massberg and Reiss 2012): output level for input level, dB.
pub fn compressor_curve(x: f64, t: f64, r: f64, w: f64) -> f64 {
    if 2.0 * (x - t) < -w {
        x
    } else if w > 0.0 && 2.0 * (x - t).abs() <= w {
        x + (1.0 / r - 1.0) * (x - t + w / 2.0).powi(2) / (2.0 * w)
    } else {
        t + (x - t) / r
    }
}

/// Linked gain-reduction envelope in dB for a detector signal: a peak
/// envelope (instant attack, `release` decay) feeds the static curve, and
/// the gain moves with the attack and release times.
fn compressor_gain(detector: &[f64], e: &Effect, rate: f64) -> Vec<f64> {
    let (ca, cr) = (coeff(e.attack, rate), coeff(e.release, rate));
    let mut g = 0.0;
    let mut env = 0.0f64;
    detector
        .iter()
        .map(|&lvl| {
            env = lvl.max(env * cr);
            let x = lin_to_db(env);
            let target = compressor_curve(x, e.threshold, e.ratio.max(1.0), e.knee) - x;
            g = if target < g { target + (g - target) * ca } else { target + (g - target) * cr };
            g
        })
        .collect()
}

fn linked_level(buf: &Planar) -> Vec<f64> {
    let n = buf.first().map(Vec::len).unwrap_or(0);
    (0..n).map(|i| buf.iter().map(|c| (c[i] as f64).abs()).fold(0.0, f64::max)).collect()
}

fn apply_gain_db(buf: &mut Planar, g: &[f64], extra_db: f64) {
    for ch in buf.iter_mut() {
        for (v, gd) in ch.iter_mut().zip(g) {
            *v = (*v as f64 * db_to_lin(gd + extra_db)) as f32;
        }
    }
}

fn filter_all(buf: &mut Planar, make: impl Fn() -> Vec<Biquad>) {
    for ch in buf.iter_mut() {
        for mut f in make() {
            f.run(ch);
        }
    }
}

/// Processes `buf` in place. `key` is the sidechain signal when the effect names one.
pub fn process(e: &Effect, buf: &mut Planar, rate: f64, key: Option<&Planar>) {
    if !e.enabled || buf.is_empty() {
        return;
    }
    let dry = if e.mix < 1.0 { Some(buf.clone()) } else { None };
    let n = buf[0].len();
    match e.kind {
        Kind::Eq => {
            let bands = if e.bands.is_empty() {
                e.frequency
                    .map(|f| vec![Band { shape: Shape::Peak, frequency: f, gain: e.gain, q: 1.0 }])
                    .unwrap_or_default()
            } else {
                e.bands.clone()
            };
            filter_all(buf, || bands.iter().map(|b| Biquad::new(b.shape, rate, b.frequency, b.q, b.gain)).collect());
        }
        Kind::HighPass => {
            let f = e.frequency.unwrap_or(80.0);
            filter_all(buf, || vec![Biquad::new(Shape::HighPass, rate, f, std::f64::consts::FRAC_1_SQRT_2, 0.0)]);
        }
        Kind::LowPass => {
            let f = e.frequency.unwrap_or(8000.0);
            filter_all(buf, || vec![Biquad::new(Shape::LowPass, rate, f, std::f64::consts::FRAC_1_SQRT_2, 0.0)]);
        }
        Kind::Compressor => {
            let det = linked_level(key.unwrap_or(buf));
            let g = compressor_gain(&det[..n.min(det.len())], e, rate);
            apply_gain_db(buf, &g, e.gain);
        }
        Kind::Limiter => crate::loudness::limit(buf, rate, e.threshold, e.release.max(0.01)),
        Kind::Gate => {
            let det = linked_level(key.unwrap_or(buf));
            let (ca, cr) = (coeff(e.attack, rate), coeff(e.release, rate));
            let mut env = 0.0;
            let mut g = 0.0;
            let gains: Vec<f64> = det
                .iter()
                .map(|&x| {
                    env = if x > env { x } else { x + (env - x) * cr };
                    let lvl = lin_to_db(env);
                    let target =
                        if lvl < e.threshold { (-(e.threshold - lvl) * (e.ratio - 1.0)).max(-80.0) } else { 0.0 };
                    g = if target > g { target + (g - target) * ca } else { target + (g - target) * cr };
                    g
                })
                .collect();
            apply_gain_db(buf, &gains, e.gain);
        }
        Kind::DeEsser => {
            let f = e.frequency.unwrap_or(6000.0);
            let mut det = vec![0.0f64; n];
            for ch in buf.iter() {
                let mut bp = Biquad::new(Shape::BandPass, rate, f, 2.0, 0.0);
                for (i, &v) in ch.iter().enumerate() {
                    det[i] = det[i].max(bp.tick(v as f64).abs());
                }
            }
            let g = compressor_gain(&det, e, rate);
            for ch in buf.iter_mut() {
                let mut hp = Biquad::new(Shape::HighPass, rate, f * 0.7, std::f64::consts::FRAC_1_SQRT_2, 0.0);
                for (i, v) in ch.iter_mut().enumerate() {
                    let x = *v as f64;
                    let hi = hp.tick(x);
                    *v = ((x - hi) + hi * db_to_lin(g[i])) as f32;
                }
            }
        }
        Kind::Reverb => reverb(buf, rate, e),
        Kind::Delay => {
            let d = ((e.time.unwrap_or(0.25) * rate) as usize).max(1);
            for ch in buf.iter_mut() {
                let x = ch.clone();
                let mut w = vec![0f32; n];
                for i in d..n {
                    w[i] = x[i - d] + e.feedback as f32 * w[i - d];
                }
                *ch = w;
            }
        }
        Kind::Chorus => {
            let rate_hz = e.frequency.unwrap_or(0.8);
            let depth = (1.0 + 4.0 * e.amount) * 1e-3 * rate;
            let base = 0.015 * rate;
            for (c, ch) in buf.iter_mut().enumerate() {
                let x = ch.clone();
                for (i, v) in ch.iter_mut().enumerate() {
                    let mut acc = 0.0;
                    for voice in 0..2 {
                        let ph = 2.0
                            * std::f64::consts::PI
                            * (rate_hz * i as f64 / rate + voice as f64 * 0.5 + c as f64 * 0.25);
                        let delay = base + depth * (0.5 + 0.5 * ph.sin());
                        acc += crate::dsp::hermite(&x, i as f64 - delay) as f64;
                    }
                    *v = (acc * 0.5) as f32;
                }
            }
        }
        Kind::PitchShift => {
            let ratio = 2f64.powf(e.semitones / 12.0);
            let win = 0.05 * rate;
            for ch in buf.iter_mut() {
                let x = ch.clone();
                for (i, v) in ch.iter_mut().enumerate() {
                    // two taps whose delay ramps at (1 − ratio), crossfaded with triangular windows
                    let phase = ((i as f64 * (1.0 - ratio)) / win).rem_euclid(1.0);
                    let mut acc = 0.0;
                    for k in 0..2 {
                        let p = (phase + k as f64 * 0.5).fract();
                        let delay = p * win;
                        let w = 1.0 - (2.0 * p - 1.0).abs();
                        acc += w * crate::dsp::hermite(&x, i as f64 - delay) as f64;
                    }
                    *v = acc as f32;
                }
            }
        }
        Kind::NoiseReduction => {
            for ch in buf.iter_mut() {
                *ch = spectral_gate(ch, e.amount);
            }
        }
        Kind::StereoWidth => {
            if buf.len() >= 2 {
                let (l, r) = buf.split_at_mut(1);
                for (a, b) in l[0].iter_mut().zip(r[0].iter_mut()) {
                    let m = (*a + *b) * 0.5;
                    let s = (*a - *b) * 0.5 * e.width as f32;
                    *a = m + s;
                    *b = m - s;
                }
            }
        }
        Kind::Gain => {
            let g = db_to_lin(e.gain) as f32;
            for ch in buf.iter_mut() {
                for v in ch.iter_mut() {
                    *v *= g;
                }
            }
        }
        Kind::Distortion => {
            let drive = 1.0 + 30.0 * e.amount;
            let norm = drive.tanh();
            for ch in buf.iter_mut() {
                for v in ch.iter_mut() {
                    *v = ((drive * *v as f64).tanh() / norm) as f32;
                }
            }
        }
        Kind::Telephone => {
            let mono: Vec<f32> = (0..n).map(|i| buf.iter().map(|c| c[i]).sum::<f32>() / buf.len() as f32).collect();
            let mut m = vec![mono];
            filter_all(&mut m, || {
                vec![
                    Biquad::new(Shape::HighPass, rate, 300.0, std::f64::consts::FRAC_1_SQRT_2, 0.0),
                    Biquad::new(Shape::HighPass, rate, 300.0, std::f64::consts::FRAC_1_SQRT_2, 0.0),
                    Biquad::new(Shape::LowPass, rate, 3400.0, std::f64::consts::FRAC_1_SQRT_2, 0.0),
                    Biquad::new(Shape::LowPass, rate, 3400.0, std::f64::consts::FRAC_1_SQRT_2, 0.0),
                ]
            });
            let drive = 1.0 + 4.0 * e.amount;
            let out: Vec<f32> = m[0].iter().map(|&v| ((drive * v as f64).tanh() / drive.tanh()) as f32).collect();
            for ch in buf.iter_mut() {
                ch.clone_from(&out);
            }
        }
    }
    if let Some(dry) = dry {
        let (w, d) = (e.mix as f32, 1.0 - e.mix as f32);
        for (ch, dc) in buf.iter_mut().zip(dry) {
            for (v, x) in ch.iter_mut().zip(dc) {
                *v = *v * w + x * d;
            }
        }
    }
}

/// Freeverb: eight damped combs and four all-passes per channel.
fn reverb(buf: &mut Planar, rate: f64, e: &Effect) {
    const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
    const ALLPASS: [usize; 4] = [556, 441, 341, 225];
    let scale = rate / 44100.0;
    let feedback = 0.7 + 0.28 * e.room_size;
    let damp = 0.2 + 0.6 * e.amount;
    let n = buf[0].len();
    let mut wet: Planar = Vec::with_capacity(buf.len());
    for (c, ch) in buf.iter().enumerate() {
        let spread = if c % 2 == 1 { 23 } else { 0 };
        let mut out = vec![0f64; n];
        for &len in &COMBS {
            let d = (((len + spread) as f64) * scale) as usize;
            let mut line = vec![0f64; d.max(1)];
            let (mut idx, mut store) = (0, 0.0);
            for i in 0..n {
                let y = line[idx];
                store = y * (1.0 - damp) + store * damp;
                line[idx] = ch[i] as f64 * 0.015 + store * feedback;
                idx = (idx + 1) % line.len();
                out[i] += y;
            }
        }
        for &len in &ALLPASS {
            let d = (((len + spread) as f64) * scale) as usize;
            let mut line = vec![0f64; d.max(1)];
            let mut idx = 0;
            for v in out.iter_mut() {
                let b = line[idx];
                line[idx] = *v + b * 0.5;
                *v = b - *v;
                idx = (idx + 1) % line.len();
            }
        }
        wet.push(out.into_iter().map(|v| v as f32).collect());
    }
    // width mixes the two reverb channels
    if wet.len() >= 2 {
        let w = e.width.clamp(0.0, 1.0) as f32;
        let (w1, w2) = (w / 2.0 + 0.5, (1.0 - w) / 2.0);
        for i in 0..n {
            let (l, r) = (wet[0][i], wet[1][i]);
            wet[0][i] = l * w1 + r * w2;
            wet[1][i] = r * w1 + l * w2;
        }
    }
    *buf = wet;
}

/// Spectral gating against a noise floor estimated from the quietest frames.
fn spectral_gate(x: &[f32], amount: f64) -> Vec<f32> {
    const N: usize = 2048;
    const HOP: usize = 512;
    let n = x.len();
    if n < N {
        return x.to_vec();
    }
    let w = hann(N);
    let frames = (n - N) / HOP + 1;
    let mut spectra: Vec<(Vec<f64>, Vec<f64>)> = Vec::with_capacity(frames);
    for f in 0..frames {
        let mut re: Vec<f64> = (0..N).map(|k| x[f * HOP + k] as f64 * w[k]).collect();
        let mut im = vec![0.0; N];
        fft(&mut re, &mut im, false);
        spectra.push((re, im));
    }
    // noise profile: 10th percentile magnitude per bin
    let mut noise = vec![0.0; N / 2 + 1];
    for (b, nb) in noise.iter_mut().enumerate() {
        let mut mags: Vec<f64> = spectra.iter().map(|(r, i)| (r[b] * r[b] + i[b] * i[b]).sqrt()).collect();
        mags.sort_by(|a, c| a.partial_cmp(c).unwrap());
        *nb = mags[mags.len() / 10];
    }
    let mut out = vec![0f64; n];
    let mut norm = vec![0f64; n];
    let mut prev = vec![1.0; N / 2 + 1];
    for (f, (re, im)) in spectra.iter_mut().enumerate() {
        for b in 0..=N / 2 {
            let mag = (re[b] * re[b] + im[b] * im[b]).sqrt();
            let target = (1.0 - amount * 2.0 * noise[b] / mag.max(1e-12)).clamp(0.1, 1.0);
            let g = if target < prev[b] { target } else { prev[b] + (target - prev[b]) * 0.3 };
            prev[b] = g;
            re[b] *= g;
            im[b] *= g;
            if b > 0 && b < N / 2 {
                re[N - b] *= g;
                im[N - b] *= g;
            }
        }
        fft(re, im, true);
        for k in 0..N {
            out[f * HOP + k] += re[k] * w[k];
            norm[f * HOP + k] += w[k] * w[k];
        }
    }
    out.iter().zip(&norm).zip(x).map(|((o, nm), orig)| if *nm > 1e-3 { (o / nm) as f32 } else { *orig }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f64, amp: f64, secs: f64) -> Vec<f32> {
        (0..(secs * 48000.0) as usize)
            .map(|i| (amp * (2.0 * std::f64::consts::PI * f * i as f64 / 48000.0).sin()) as f32)
            .collect()
    }

    fn peak_db(x: &[f32]) -> f64 {
        lin_to_db(x.iter().fold(0f32, |m, v| m.max(v.abs())) as f64)
    }

    #[test]
    fn compressor_reaches_its_static_curve() {
        let mut b = vec![sine(1000.0, db_to_lin(-6.0), 2.0)];
        let e = Effect { knee: 0.0, ..Effect::new(Kind::Compressor) };
        process(&e, &mut b, 48000.0, None);
        let tail = &b[0][48000..];
        assert!((peak_db(tail) + 15.0).abs() < 0.3, "{}", peak_db(tail));
        assert!((compressor_curve(-10.0, -18.0, 4.0, 6.0) + 16.0).abs() < 1e-12);
        // centre of a 6 dB knee: x + (1/R − 1)(W/2)²/(2W)
        assert!((compressor_curve(-18.0, -18.0, 4.0, 6.0) - (-18.0 - 0.75 * 9.0 / 12.0)).abs() < 1e-12);
    }

    #[test]
    fn filters_and_gain() {
        let mut b = vec![sine(10000.0, 0.5, 1.0)];
        process(&Effect { frequency: Some(1000.0), ..Effect::new(Kind::LowPass) }, &mut b, 48000.0, None);
        assert!(peak_db(&b[0][24000..]) < -40.0);
        let mut g = vec![sine(1000.0, 0.5, 0.2)];
        process(&Effect { gain: -6.0, ..Effect::new(Kind::Gain) }, &mut g, 48000.0, None);
        assert!((peak_db(&g[0]) - (lin_to_db(0.5) - 6.0)).abs() < 0.05);
    }

    #[test]
    fn gate_silences_quiet_material_and_width_collapses_stereo() {
        let mut quiet = vec![sine(1000.0, db_to_lin(-60.0), 1.0)];
        process(&Effect { threshold: -40.0, ..Effect::new(Kind::Gate) }, &mut quiet, 48000.0, None);
        assert!(peak_db(&quiet[0][24000..]) < -80.0);
        let (l, r) = (sine(500.0, 0.5, 0.1), sine(700.0, 0.5, 0.1));
        let mut st = vec![l, r];
        process(&Effect { width: 0.0, ..Effect::new(Kind::StereoWidth) }, &mut st, 48000.0, None);
        assert_eq!(st[0], st[1]);
    }

    #[test]
    fn pitch_shift_moves_the_frequency() {
        let mut b = vec![sine(440.0, 0.5, 2.0)];
        process(&Effect { semitones: 12.0, ..Effect::new(Kind::PitchShift) }, &mut b, 48000.0, None);
        let tail = &b[0][24000..72000];
        let crossings = tail.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() as f64;
        assert!((crossings / 2.0 - 880.0).abs() < 25.0, "{} Hz", crossings / 2.0);
    }

    #[test]
    fn every_effect_is_finite_and_audible() {
        for name in [
            "eq",
            "highpass",
            "lowpass",
            "compressor",
            "limiter",
            "gate",
            "de-esser",
            "reverb",
            "delay",
            "chorus",
            "pitch-shift",
            "noise-reduction",
            "stereo-width",
            "gain",
            "distortion",
            "telephone",
        ] {
            let kind = Kind::parse(name).unwrap();
            let mut e = Effect::new(kind);
            e.gain = 3.0;
            e.frequency = Some(1500.0);
            e.semitones = 3.0;
            e.width = 1.5;
            e.threshold = -12.0;
            let src = vec![sine(440.0, 0.5, 1.0), sine(660.0, 0.4, 1.0)];
            let mut b = src.clone();
            process(&e, &mut b, 48000.0, None);
            assert!(b.iter().flatten().all(|v| v.is_finite()), "{name}");
            assert!(b.iter().flatten().any(|v| v.abs() > 1e-4), "{name} is silent");
            assert!(b != src, "{name} changed nothing");
        }
    }
}
