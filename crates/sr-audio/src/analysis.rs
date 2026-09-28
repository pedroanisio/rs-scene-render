//! The analysis table read by `audioAmplitude()`, `beat()` and audio links:
//! per-frame band envelopes and beat times.

use crate::dsp::{fft, hann, lin_to_db, Biquad, Shape};
use crate::loudness::Planar;

/// Envelopes of a signal at `fps`: full band, low (< 250 Hz), mid
/// (250 Hz – 4 kHz) and high (> 4 kHz), each the RMS of one frame mapped
/// linearly from −60 dBFS (0) to 0 dBFS (1).
pub fn envelopes(buf: &Planar, rate: f64, fps: f64, frames: usize) -> [Vec<f32>; 4] {
    let n = buf.first().map(Vec::len).unwrap_or(0);
    let mono: Vec<f32> = (0..n).map(|i| buf.iter().map(|c| c[i]).sum::<f32>() / buf.len().max(1) as f32).collect();
    let lr4 = |shape: Shape, f: f64| -> Vec<f32> {
        let mut out = mono.clone();
        for _ in 0..2 {
            Biquad::new(shape, rate, f, std::f64::consts::FRAC_1_SQRT_2, 0.0).run(&mut out);
        }
        out
    };
    let low = lr4(Shape::LowPass, 250.0);
    let high = lr4(Shape::HighPass, 4000.0);
    let mut mid = lr4(Shape::HighPass, 250.0);
    for _ in 0..2 {
        Biquad::new(Shape::LowPass, rate, 4000.0, std::f64::consts::FRAC_1_SQRT_2, 0.0).run(&mut mid);
    }
    let env = |x: &[f32]| -> Vec<f32> {
        (0..frames)
            .map(|k| {
                let a = ((k as f64 / fps) * rate) as usize;
                let b = (((k + 1) as f64 / fps) * rate) as usize;
                let (a, b) = (a.min(x.len()), b.min(x.len()));
                if b <= a {
                    return 0.0;
                }
                let ms = x[a..b].iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / (b - a) as f64;
                ((lin_to_db(ms.sqrt()) + 60.0) / 60.0).clamp(0.0, 1.0) as f32
            })
            .collect()
    };
    [env(&mono), env(&low), env(&mid), env(&high)]
}

/// Spectral-flux onset strength, one value per `hop` samples.
pub fn onsets(buf: &Planar, hop: usize) -> Vec<f64> {
    const N: usize = 1024;
    let n = buf.first().map(Vec::len).unwrap_or(0);
    let mono: Vec<f64> = (0..n).map(|i| buf.iter().map(|c| c[i] as f64).sum::<f64>()).collect();
    let w = hann(N);
    let mut prev = vec![0.0; N / 2];
    let mut out = Vec::new();
    let mut s = 0;
    while s + N <= n {
        let mut re: Vec<f64> = (0..N).map(|k| mono[s + k] * w[k]).collect();
        let mut im = vec![0.0; N];
        fft(&mut re, &mut im, false);
        let mut flux = 0.0;
        for b in 0..N / 2 {
            let m = (1.0 + (re[b] * re[b] + im[b] * im[b]).sqrt()).ln();
            flux += (m - prev[b]).max(0.0);
            prev[b] = m;
        }
        out.push(flux);
        s += hop;
    }
    out
}

/// Beat times over `duration` seconds. With `bpm`, only the phase is
/// estimated (the grid offset maximising onset strength on the beats);
/// without, the tempo comes from the onset autocorrelation between 60 and
/// 180 BPM.
pub fn beats(buf: &Planar, rate: f64, bpm: Option<f64>, duration: f64) -> (f64, Vec<f64>) {
    let hop = 512;
    let o = onsets(buf, hop);
    let fr = rate / hop as f64;
    let bpm = bpm.unwrap_or_else(|| {
        let mut best = (0.0, 120.0);
        for tenth in 600..=1800 {
            let b = tenth as f64 / 10.0;
            let lag = 60.0 / b * fr;
            let (l0, f) = (lag.floor() as usize, lag.fract());
            if l0 + 1 >= o.len() {
                continue;
            }
            let mut s = 0.0;
            for i in 0..o.len() - l0 - 1 {
                s += o[i] * (o[i + l0] * (1.0 - f) + o[i + l0 + 1] * f);
            }
            s /= (o.len() - l0) as f64;
            if s > best.0 {
                best = (s, b);
            }
        }
        best.1
    });
    let period = 60.0 / bpm;
    let steps = 100;
    let mut best = (f64::MIN, 0.0);
    for k in 0..steps {
        let phase = period * k as f64 / steps as f64;
        let mut s = 0.0;
        let mut t = phase;
        while t < duration {
            let i = (t * fr).round() as usize;
            s += o.get(i).copied().unwrap_or(0.0);
            t += period;
        }
        if s > best.0 {
            best = (s, phase);
        }
    }
    // onsets are measured at the start of their analysis window; centre them
    let phase = (best.1 + 512.0 / rate).rem_euclid(period);
    let mut out = Vec::new();
    let mut t = phase;
    while t < duration {
        out.push(t);
        t += period;
    }
    (bpm, out)
}
