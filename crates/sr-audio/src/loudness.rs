//! ITU-R BS.1770-4 loudness, true peak, normalisation and the true-peak limiter.

use crate::dsp::{db_to_lin, lin_to_db, Biquad};

/// Planar audio: one buffer per channel.
pub type Planar = Vec<Vec<f32>>;

/// The K-weighting pre-filter (high shelf) and RLB high-pass for any rate.
pub fn k_weighting(rate: f64) -> [Biquad; 2] {
    let shelf = {
        let (f0, g, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
        let k = (std::f64::consts::PI * f0 / rate).tan();
        let vh = 10f64.powf(g / 20.0);
        let vb = vh.powf(0.4996667741545416);
        let a0 = 1.0 + k / q + k * k;
        Biquad::raw(
            [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
            [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        )
    };
    let hp = {
        let (f0, q) = (38.13547087602444, 0.5003270373238773);
        let k = (std::f64::consts::PI * f0 / rate).tan();
        let a0 = 1.0 + k / q + k * k;
        Biquad::raw([1.0, -2.0, 1.0], [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0])
    };
    [shelf, hp]
}

/// Mean-square energy of K-weighted audio in blocks of `block` samples every `step`.
fn block_energies(buf: &Planar, rate: f64, weights: &[f64], block: usize, step: usize) -> Vec<f64> {
    let n = buf.first().map(Vec::len).unwrap_or(0);
    let mut weighted: Vec<Vec<f64>> = Vec::new();
    for (c, ch) in buf.iter().enumerate() {
        if weights.get(c).copied().unwrap_or(1.0) == 0.0 {
            weighted.push(Vec::new());
            continue;
        }
        let [mut a, mut b] = k_weighting(rate);
        weighted.push(
            ch.iter()
                .map(|&x| {
                    let y = b.tick(a.tick(x as f64));
                    y * y
                })
                .collect(),
        );
    }
    let mut out = Vec::new();
    if n < block {
        return out;
    }
    // prefix sums per channel
    let prefix: Vec<Vec<f64>> = weighted
        .iter()
        .map(|w| {
            let mut p = Vec::with_capacity(w.len() + 1);
            p.push(0.0);
            let mut s = 0.0;
            for v in w {
                s += v;
                p.push(s);
            }
            p
        })
        .collect();
    let mut start = 0;
    while start + block <= n {
        let mut e = 0.0;
        for (c, p) in prefix.iter().enumerate() {
            if p.len() > 1 {
                e += weights.get(c).copied().unwrap_or(1.0) * (p[start + block] - p[start]) / block as f64;
            }
        }
        out.push(e);
        start += step;
    }
    out
}

fn lufs(e: f64) -> f64 {
    -0.691 + 10.0 * e.max(1e-20).log10()
}

/// Integrated loudness in LUFS (−∞ as −200 for silence).
pub fn integrated(buf: &Planar, rate: f64, weights: &[f64]) -> f64 {
    let block = (0.4 * rate).round() as usize;
    let e = block_energies(buf, rate, weights, block, block / 4);
    let abs: Vec<f64> = e.into_iter().filter(|&x| lufs(x) > -70.0).collect();
    if abs.is_empty() {
        return -200.0;
    }
    let rel = lufs(abs.iter().sum::<f64>() / abs.len() as f64) - 10.0;
    let gated: Vec<f64> = abs.into_iter().filter(|&x| lufs(x) > rel).collect();
    if gated.is_empty() {
        return -200.0;
    }
    lufs(gated.iter().sum::<f64>() / gated.len() as f64)
}

/// Short-term (3 s) loudness every `hop` seconds.
pub fn short_term(buf: &Planar, rate: f64, weights: &[f64], hop: f64) -> Vec<f64> {
    let block = (3.0 * rate).round() as usize;
    block_energies(buf, rate, weights, block, (hop * rate).round().max(1.0) as usize).into_iter().map(lufs).collect()
}

/// 4× oversampling interpolator (windowed sinc, 12 taps per phase).
struct Oversampler {
    phases: [[f64; 12]; 4],
}

impl Oversampler {
    fn new() -> Oversampler {
        let mut phases = [[0.0; 12]; 4];
        for (p, row) in phases.iter_mut().enumerate() {
            for (k, v) in row.iter_mut().enumerate() {
                let x = (k as f64 - 5.5) - (p as f64 / 4.0 - 0.5);
                let t = x * std::f64::consts::PI;
                let sinc = if x.abs() < 1e-9 { 1.0 } else { t.sin() / t };
                let w = 0.42
                    + 0.5 * (std::f64::consts::PI * x / 6.5).cos()
                    + 0.08 * (2.0 * std::f64::consts::PI * x / 6.5).cos();
                *v = sinc * w.max(0.0);
            }
            let s: f64 = row.iter().sum();
            for v in row.iter_mut() {
                *v /= s;
            }
        }
        Oversampler { phases }
    }

    /// Largest absolute inter-sample value around sample `i` of `ch`.
    fn peak_at(&self, ch: &[f32], i: usize) -> f64 {
        let mut m = (ch[i] as f64).abs();
        for row in &self.phases {
            let mut acc = 0.0;
            for (k, c) in row.iter().enumerate() {
                let j = i as i64 + k as i64 - 5;
                if j >= 0 && (j as usize) < ch.len() {
                    acc += c * ch[j as usize] as f64;
                }
            }
            m = m.max(acc.abs());
        }
        m
    }
}

/// True peak in dBTP.
pub fn true_peak(buf: &Planar) -> f64 {
    let os = Oversampler::new();
    let mut m: f64 = 0.0;
    for ch in buf {
        for i in 0..ch.len() {
            m = m.max(os.peak_at(ch, i));
        }
    }
    lin_to_db(m)
}

/// Look-ahead true-peak limiter: gain never lets any channel exceed
/// `ceiling_db` true peak; recovery follows `release` seconds.
pub fn limit(buf: &mut Planar, rate: f64, ceiling_db: f64, release: f64) {
    let n = buf.first().map(Vec::len).unwrap_or(0);
    if n == 0 {
        return;
    }
    let os = Oversampler::new();
    let ceiling = db_to_lin(ceiling_db) * 0.999;
    let look = ((0.0015 * rate) as usize).max(1);
    // required gain per sample
    let mut need = vec![1.0f64; n];
    for ch in buf.iter() {
        for i in 0..n {
            let p = os.peak_at(ch, i);
            if p > ceiling {
                need[i] = need[i].min(ceiling / p);
            }
        }
    }
    // spread each requirement over the look-ahead window before it (so the gain is already down)
    let mut env = vec![1.0f64; n];
    let mut dq: std::collections::VecDeque<(usize, f64)> = Default::default();
    for i in 0..n + look {
        if i < n {
            while dq.back().is_some_and(|b| b.1 >= need[i]) {
                dq.pop_back();
            }
            dq.push_back((i, need[i]));
        }
        while dq.front().is_some_and(|f| f.0 + look < i) {
            dq.pop_front();
        }
        if i >= look {
            env[i - look] = dq.front().map(|f| f.1).unwrap_or(1.0);
        }
    }
    // smooth: instant attack (the look-ahead already anticipates), exponential release
    let r = crate::dsp::coeff(release, rate);
    let mut g = 1.0;
    let mut out = vec![1.0; n];
    for i in 0..n {
        let target = env[i];
        g = if target < g { target } else { target + (g - target) * r };
        out[i] = g.min(env[i]);
    }
    for ch in buf.iter_mut() {
        for (v, g) in ch.iter_mut().zip(&out) {
            *v = (*v as f64 * g) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, amp: f64, secs: f64, rate: f64, phase: f64) -> Vec<f32> {
        (0..(secs * rate) as usize)
            .map(|i| (amp * (2.0 * std::f64::consts::PI * freq * i as f64 / rate + phase).sin()) as f32)
            .collect()
    }

    #[test]
    fn ebu_tech_3341_reference_levels() {
        // stereo 1 kHz sine at −23 dBFS reads −23.0 LUFS; at −33 dBFS reads −33.0 LUFS
        for (level, want) in [(-23.0, -23.0), (-33.0, -33.0)] {
            let s = sine(1000.0, db_to_lin(level), 20.0, 48000.0, 0.0);
            let l = integrated(&vec![s.clone(), s], 48000.0, &[1.0, 1.0]);
            assert!((l - want).abs() < 0.1, "{level} dBFS → {l} LUFS");
        }
        // the relative gate removes a quiet section: 10 s at −36 then 10 s at −23 dBFS... (Tech 3341 case 3)
        let mut s = sine(1000.0, db_to_lin(-36.0), 10.0, 48000.0, 0.0);
        s.extend(sine(1000.0, db_to_lin(-23.0), 60.0, 48000.0, 0.0));
        s.extend(sine(1000.0, db_to_lin(-36.0), 10.0, 48000.0, 0.0));
        let l = integrated(&vec![s.clone(), s], 48000.0, &[1.0, 1.0]);
        assert!((l + 23.0).abs() < 0.1, "{l}");
    }

    #[test]
    fn true_peak_sees_between_samples() {
        // fs/4 sine at 45°: samples reach 0.707·A, the waveform reaches A
        let s = sine(12000.0, 0.5, 1.0, 48000.0, std::f64::consts::FRAC_PI_4);
        let sample_peak = lin_to_db(s.iter().fold(0f32, |m, v| m.max(v.abs())) as f64);
        let tp = true_peak(&vec![s]);
        assert!((sample_peak + 9.03).abs() < 0.1);
        assert!((tp + 6.02).abs() < 0.3, "true peak {tp}");
    }

    #[test]
    fn limiter_caps_true_peak() {
        let s = sine(997.0, 1.0, 2.0, 48000.0, 0.3);
        let mut b = vec![s.clone(), s];
        limit(&mut b, 48000.0, -1.0, 0.05);
        let tp = true_peak(&b);
        assert!(tp <= -0.95, "{tp}");
        assert!(tp > -1.5, "{tp}");
    }
}
