//! The analysis table read by `audioAmplitude()`, `beat()` and audio links:
//! per-frame band envelopes and beat times.

use crate::dsp::{fft, hann, lin_to_db, Biquad, Shape};
use crate::loudness::{Extent, Planar};

/// Runs `f` over `buf[span]`, then on with zero input until it has settled
/// below `below` or the buffer ends, and returns the extent of its output:
/// the filter is at rest before the span, and from where it stops every
/// later output would round to zero as an `f32`.
fn run_span(f: &mut Biquad, buf: &mut [f32], span: Extent, below: f64) -> Extent {
    for v in &mut buf[span.start..span.end] {
        *v = f.tick(*v as f64) as f32;
    }
    let mut i = span.end;
    while i < buf.len() && !f.settled(below) {
        buf[i] = f.tick(0.0) as f32;
        i += 1;
    }
    Extent { start: span.start, end: i }
}

/// Envelopes of a signal at `fps`: full band, low (< 250 Hz), mid
/// (250 Hz – 4 kHz) and high (> 4 kHz), each the RMS of one frame mapped
/// linearly from −60 dBFS (0) to 0 dBFS (1).
pub fn envelopes(buf: &Planar, rate: f64, fps: f64, frames: usize) -> [Vec<f32>; 4] {
    envelopes_in(buf, Extent::of(buf), rate, fps, frames)
}

/// [`envelopes`] of a signal that is zero outside `ext`: only the extent
/// and the filters' tails after it are filtered and summed, the bands in
/// parallel.
pub fn envelopes_in(buf: &Planar, ext: Extent, rate: f64, fps: f64, frames: usize) -> [Vec<f32>; 4] {
    let n = buf.first().map(Vec::len).unwrap_or(0);
    let ext = ext.clip(n);
    let mut mono = vec![0f32; n];
    for i in ext.start..ext.end {
        mono[i] = buf.iter().map(|c| c[i]).sum::<f32>() / buf.len().max(1) as f32;
    }
    let q = std::f64::consts::FRAC_1_SQRT_2;
    // a band: the mono signal through the sections in turn, with the extent of the result
    let band = |sections: &[(Shape, f64)]| -> (Vec<f32>, Extent) {
        let mut out = vec![0f32; n];
        out[ext.start..ext.end].copy_from_slice(&mono[ext.start..ext.end]);
        let mut span = ext;
        for &(shape, f) in sections {
            span = run_span(&mut Biquad::new(shape, rate, f, q, 0.0), &mut out, span, Biquad::SETTLED_F32);
        }
        (out, span)
    };
    // RMS per frame from the samples inside the span; the rest are zero and add nothing
    let env = |x: &[f32], span: Extent| -> Vec<f32> {
        (0..frames)
            .map(|k| {
                let a = ((k as f64 / fps) * rate) as usize;
                let b = (((k + 1) as f64 / fps) * rate) as usize;
                let (a, b) = (a.min(x.len()), b.min(x.len()));
                if b <= a {
                    return 0.0;
                }
                let (lo, hi) = (a.max(span.start), b.min(span.end));
                let sum = if lo < hi { x[lo..hi].iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() } else { 0.0 };
                let ms = sum / (b - a) as f64;
                ((lin_to_db(ms.sqrt()) + 60.0) / 60.0).clamp(0.0, 1.0) as f32
            })
            .collect()
    };
    let band_env = |sections: &[(Shape, f64)]| {
        let (x, span) = band(sections);
        env(&x, span)
    };
    let (lp, hp) = (Shape::LowPass, Shape::HighPass);
    let (full, (low, (mid, high))) = rayon::join(
        || env(&mono, ext),
        || {
            rayon::join(
                || band_env(&[(lp, 250.0), (lp, 250.0)]),
                || {
                    rayon::join(
                        || band_env(&[(hp, 250.0), (hp, 250.0), (lp, 4000.0), (lp, 4000.0)]),
                        || band_env(&[(hp, 4000.0), (hp, 4000.0)]),
                    )
                },
            )
        },
    );
    [full, low, mid, high]
}

/// Spectral-flux onset strength, one value per `hop` samples.
pub fn onsets(buf: &Planar, hop: usize) -> Vec<f64> {
    onsets_in(buf, Extent::of(buf), hop)
}

/// [`onsets`] of a signal that is zero outside `ext`: a frame of zeros has
/// no flux and leaves zero magnitudes for the next frame to compare against.
pub fn onsets_in(buf: &Planar, ext: Extent, hop: usize) -> Vec<f64> {
    const N: usize = 1024;
    let n = buf.first().map(Vec::len).unwrap_or(0);
    let ext = ext.clip(n);
    let mut mono = vec![0f64; n];
    for i in ext.start..ext.end {
        mono[i] = buf.iter().map(|c| c[i] as f64).sum::<f64>();
    }
    let w = hann(N);
    let mut prev = vec![0.0; N / 2];
    let mut out = Vec::new();
    let mut s = 0;
    while s + N <= n {
        if s + N <= ext.start || s >= ext.end {
            out.push(0.0);
            prev.fill(0.0);
            s += hop;
            continue;
        }
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
    beats_in(buf, Extent::of(buf), rate, bpm, duration)
}

/// [`beats`] of a signal that is zero outside `ext`.
pub fn beats_in(buf: &Planar, ext: Extent, rate: f64, bpm: Option<f64>, duration: f64) -> (f64, Vec<f64>) {
    let hop = 512;
    let o = onsets_in(buf, ext, hop);
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

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48000.0;

    /// A 12 s stereo programme with a burst of tones in all three bands
    /// over `from..to` seconds, hard-edged, with a beat every half second
    /// and negative zeros just outside it.
    fn burst(from: f64, to: f64) -> (Planar, Extent) {
        let n = (12.0 * RATE) as usize;
        let mut l = vec![0f32; n];
        let mut r = vec![0f32; n];
        let (a, b) = ((from * RATE) as usize, (to * RATE) as usize);
        for i in a..b {
            let t = i as f64 / RATE;
            let beat = if (t * 2.0).fract() < 0.05 { 2.0 } else { 0.4 };
            let tones = (2.0 * std::f64::consts::PI * 100.0 * t).sin() * 0.5
                + (2.0 * std::f64::consts::PI * 1000.0 * t).sin() * 0.3
                + (2.0 * std::f64::consts::PI * 8000.0 * t + 0.3).sin() * 0.2;
            l[i] = (0.45 * beat * tones + 0.02) as f32;
            r[i] = (0.4 * beat * (2.0 * std::f64::consts::PI * 440.0 * t + 0.5).sin() - 0.02) as f32;
        }
        if a > 0 {
            l[a - 1] = -0.0;
        }
        if b < n {
            r[b] = -0.0;
        }
        (vec![l, r], if a < b { Extent { start: a, end: b } } else { Extent::EMPTY })
    }

    fn same(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
    }

    #[test]
    fn extent_limited_envelopes_are_bit_identical() {
        for (from, to) in [(4.3, 6.7), (0.0, 2.5), (10.2, 12.0), (5.0, 5.0)] {
            let (buf, ext) = burst(from, to);
            let n = buf[0].len();
            assert_eq!(Extent::of(&buf), ext, "the negative zeros at the edges are zeros");
            for fps in [24.0f64, 29.97, 60.0] {
                let frames = (12.0 * fps).ceil() as usize;
                let want = envelopes_in(&buf, Extent::full(n), RATE, fps, frames);
                let got = envelopes_in(&buf, ext, RATE, fps, frames);
                let scanned = envelopes(&buf, RATE, fps, frames);
                let wide = envelopes_in(&buf, ext.widen(3000, 7000, n), RATE, fps, frames);
                for band in 0..4 {
                    assert!(same(&got[band], &want[band]), "band {band}, {from}..{to} s at {fps} fps");
                    assert!(same(&scanned[band], &want[band]), "scanned band {band}, {from}..{to} s at {fps} fps");
                    assert!(same(&wide[band], &want[band]), "wide band {band}, {from}..{to} s at {fps} fps");
                    assert_eq!(want[band].iter().any(|v| *v > 0.3), from < to, "band {band} has content");
                }
            }
        }
    }

    #[test]
    fn extent_limited_onsets_and_beats_are_bit_identical() {
        for (from, to) in [(4.3, 6.7), (0.0, 2.5), (10.2, 12.0)] {
            let (buf, ext) = burst(from, to);
            let n = buf[0].len();
            let want = onsets_in(&buf, Extent::full(n), 512);
            let got = onsets_in(&buf, ext, 512);
            assert_eq!(want.len(), got.len());
            assert!(want.iter().zip(&got).all(|(x, y)| x.to_bits() == y.to_bits()), "{from}..{to} s");
            assert!(want.iter().any(|v| *v > 0.0));
            assert_eq!(beats_in(&buf, ext, RATE, Some(120.0), 12.0), beats(&buf, RATE, Some(120.0), 12.0));
            assert_eq!(beats_in(&buf, ext, RATE, None, 12.0), beats_in(&buf, Extent::full(n), RATE, None, 12.0));
        }
    }
}
