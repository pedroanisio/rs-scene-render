//! ITU-R BS.1770-4 loudness, true peak, normalisation and the true-peak limiter.

use rayon::prelude::*;

use crate::dsp::{db_to_lin, lin_to_db, Biquad};

/// Samples per rayon task in the per-sample loops.
const CHUNK: usize = 8192;

/// Planar audio: one buffer per channel.
pub type Planar = Vec<Vec<f32>>;

/// The samples of a buffer that may be non-zero: every sample of every
/// channel outside `start..end` is a zero (of either sign). A bound, not
/// necessarily tight; empty when `end <= start`.
///
/// Work on a whole-programme buffer whose result outside the extent is
/// provably a zero, or unchanged, is skipped; work inside it runs with the
/// same operations in the same order, so the result is bit for bit what a
/// pass over the whole buffer produces. (A gain of zero or more keeps a
/// zero's sign, adding a zero to a running sum that starts positive
/// changes nothing, and everything the analysis derives is a square or a
/// magnitude, so the sign of a skipped zero never shows.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Extent {
    /// First sample that may be non-zero.
    pub start: usize,
    /// Past the last sample that may be non-zero.
    pub end: usize,
}

impl Extent {
    /// No samples.
    pub const EMPTY: Extent = Extent { start: 0, end: 0 };

    /// All `len` samples.
    pub fn full(len: usize) -> Extent {
        Extent { start: 0, end: len }
    }

    /// No sample may be non-zero.
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    /// Number of samples.
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Whether `i` is inside.
    pub fn contains(&self, i: usize) -> bool {
        self.start <= i && i < self.end
    }

    /// The smallest extent covering both.
    pub fn union(self, o: Extent) -> Extent {
        if self.is_empty() {
            o
        } else if o.is_empty() {
            self
        } else {
            Extent { start: self.start.min(o.start), end: self.end.max(o.end) }
        }
    }

    /// Clipped to a buffer of `len` samples.
    pub fn clip(self, len: usize) -> Extent {
        Extent { start: self.start.min(len), end: self.end.min(len) }
    }

    /// Widened by `before` and `after` samples within a buffer of `len`
    /// samples (an empty extent stays empty).
    pub fn widen(self, before: usize, after: usize, len: usize) -> Extent {
        if self.is_empty() {
            return self;
        }
        Extent { start: self.start.saturating_sub(before), end: self.end.saturating_add(after).min(len) }
    }

    /// The tight extent of `buf`: from its first to past its last non-zero
    /// sample.
    pub fn of(buf: &Planar) -> Extent {
        Extent::within(buf, Extent::full(buf.first().map(Vec::len).unwrap_or(0)))
    }

    /// The tight extent of `buf` inside `bound`, itself an extent of `buf`.
    pub fn within(buf: &Planar, bound: Extent) -> Extent {
        let mut out = Extent::EMPTY;
        for ch in buf {
            let b = bound.clip(ch.len());
            let seg = &ch[b.start..b.end];
            let first = seg.iter().position(|v| *v != 0.0);
            let last = seg.iter().rposition(|v| *v != 0.0);
            if let (Some(s), Some(e)) = (first, last) {
                out = out.union(Extent { start: b.start + s, end: b.start + e + 1 });
            }
        }
        out
    }
}

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

/// Mean-square energy of K-weighted audio in blocks of `block` samples every
/// `step`. The weighted energy is exactly zero before `ext` (the filters are
/// at rest) and once they have settled after it, so only that span is
/// filtered and summed; every block is still reported.
fn block_energies(buf: &Planar, ext: Extent, rate: f64, weights: &[f64], block: usize, step: usize) -> Vec<f64> {
    let n = buf.first().map(Vec::len).unwrap_or(0);
    let mut out = Vec::new();
    if n < block {
        return out;
    }
    let ext = ext.clip(n);
    // per weighted channel (independent, so in parallel): the span with non-zero energy and
    // prefix sums over it
    let prefix: Vec<Option<(Extent, Vec<f64>)>> = buf
        .par_iter()
        .enumerate()
        .map(|(c, ch)| {
            if weights.get(c).copied().unwrap_or(1.0) == 0.0 {
                return None;
            }
            let [mut a, mut b] = k_weighting(rate);
            let mut p = Vec::with_capacity(ext.len() + 1);
            p.push(0.0);
            let mut s = 0.0;
            let mut i = ext.start;
            while i < n && (i < ext.end || !(a.settled(Biquad::SETTLED_SQUARE) && b.settled(Biquad::SETTLED_SQUARE))) {
                let y = b.tick(a.tick(ch[i] as f64));
                s += y * y;
                p.push(s);
                i += 1;
            }
            Some((Extent { start: ext.start, end: i }, p))
        })
        .collect();
    let at =
        |span: Extent, p: &[f64], i: usize| if i <= span.start { 0.0 } else { p[(i - span.start).min(p.len() - 1)] };
    let mut start = 0;
    while start + block <= n {
        let mut e = 0.0;
        for (c, p) in prefix.iter().enumerate() {
            if let Some((span, p)) = p {
                let w = weights.get(c).copied().unwrap_or(1.0);
                e += w * (at(*span, p, start + block) - at(*span, p, start)) / block as f64;
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
    integrated_in(buf, Extent::of(buf), rate, weights)
}

/// [`integrated`] of a buffer that is zero outside `ext`.
pub fn integrated_in(buf: &Planar, ext: Extent, rate: f64, weights: &[f64]) -> f64 {
    let block = (0.4 * rate).round() as usize;
    let e = block_energies(buf, ext, rate, weights, block, block / 4);
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
    short_term_in(buf, Extent::of(buf), rate, weights, hop)
}

/// [`short_term`] of a buffer that is zero outside `ext`.
pub fn short_term_in(buf: &Planar, ext: Extent, rate: f64, weights: &[f64], hop: f64) -> Vec<f64> {
    let block = (3.0 * rate).round() as usize;
    let step = (hop * rate).round().max(1.0) as usize;
    block_energies(buf, ext, rate, weights, block, step).into_iter().map(lufs).collect()
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

    /// The samples whose inter-sample peak can be non-zero when the buffer
    /// is zero outside `ext`: the taps reach 5 samples back and 6 forward.
    fn reach(ext: Extent, len: usize) -> Extent {
        ext.clip(len).widen(6, 6, len)
    }
}

/// True peak in dBTP.
pub fn true_peak(buf: &Planar) -> f64 {
    true_peak_in(buf, Extent::of(buf))
}

/// [`true_peak`] of a buffer that is zero outside `ext`.
pub fn true_peak_in(buf: &Planar, ext: Extent) -> f64 {
    let os = Oversampler::new();
    // the maximum is exact whatever the order, and `f64::max` drops a NaN
    // against any partial maximum, as the running maximum did
    let mut m: f64 = 0.0;
    for ch in buf {
        let reach = Oversampler::reach(ext, ch.len());
        let peak = (reach.start..reach.end)
            .into_par_iter()
            .with_min_len(CHUNK)
            .map(|i| os.peak_at(ch, i))
            .reduce(|| 0.0, f64::max);
        m = m.max(peak);
    }
    lin_to_db(m)
}

/// Look-ahead true-peak limiter: gain never lets any channel exceed
/// `ceiling_db` true peak; recovery follows `release` seconds.
pub fn limit(buf: &mut Planar, rate: f64, ceiling_db: f64, release: f64) {
    limit_in(buf, Extent::of(buf), rate, ceiling_db, release)
}

/// [`limit`] of a buffer that is zero outside `ext`: the gain is 1 wherever
/// the oversampler sees only zeros, and applying it to zeros changes nothing.
pub fn limit_in(buf: &mut Planar, ext: Extent, rate: f64, ceiling_db: f64, release: f64) {
    let n = buf.first().map(Vec::len).unwrap_or(0);
    let ext = ext.clip(n);
    if ext.is_empty() {
        return;
    }
    let os = Oversampler::new();
    let ceiling = db_to_lin(ceiling_db) * 0.999;
    let look = ((0.0015 * rate) as usize).max(1);
    // required gain per sample, up to the last one the look-ahead of the extent reads
    let stop = (ext.end + look).min(n);
    let reach = Oversampler::reach(ext, stop);
    // (each sample's requirement is its own minimum over the channels, in channel order)
    let mut need = vec![1.0f64; stop];
    need[reach.start..reach.end].par_chunks_mut(CHUNK).enumerate().for_each(|(k, chunk)| {
        for (j, nd) in chunk.iter_mut().enumerate() {
            let i = reach.start + k * CHUNK + j;
            for ch in buf.iter() {
                let p = os.peak_at(ch, i);
                if p > ceiling {
                    *nd = nd.min(ceiling / p);
                }
            }
        }
    });
    // spread each requirement over the look-ahead window before it (so the gain is already down)
    let mut env = vec![1.0f64; ext.end];
    let mut dq: std::collections::VecDeque<(usize, f64)> = Default::default();
    for i in 0..ext.end + look {
        if i < stop {
            while dq.back().is_some_and(|b| b.1 >= need[i]) {
                dq.pop_back();
            }
            dq.push_back((i, need[i]));
        }
        while dq.front().is_some_and(|f| f.0 + look < i) {
            dq.pop_front();
        }
        if i >= look && i - look < ext.end {
            env[i - look] = dq.front().map(|f| f.1).unwrap_or(1.0);
        }
    }
    // smooth: instant attack (the look-ahead already anticipates), exponential release
    let r = crate::dsp::coeff(release, rate);
    let mut g = 1.0;
    let mut out = vec![1.0; ext.end];
    for i in 0..ext.end {
        let target = env[i];
        g = if target < g { target } else { target + (g - target) * r };
        out[i] = g.min(env[i]);
    }
    for ch in buf.iter_mut() {
        for i in ext.start..ext.end {
            ch[i] = (ch[i] as f64 * out[i]) as f32;
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

    /// A burst of loud tone inside a long silence, with negative zeros
    /// sprinkled in the silence and an odd channel weight.
    fn burst() -> (Planar, Extent) {
        let rate = 48000.0;
        let n = (12.0 * rate) as usize;
        let mut l = vec![0f32; n];
        let mut r = vec![0f32; n];
        let (a, b) = ((4.3 * rate) as usize, (6.7 * rate) as usize);
        for i in a..b {
            let t = i as f64 / rate;
            l[i] = (0.9 * (2.0 * std::f64::consts::PI * 997.0 * t).sin() * (1.0 + 0.3 * (7.0 * t).sin())) as f32;
            r[i] = (0.95 * (2.0 * std::f64::consts::PI * 12000.0 * t + 0.7).sin()) as f32;
        }
        l[a - 100] = -0.0;
        r[b + 7] = -0.0;
        (vec![l, r], Extent { start: a, end: b })
    }

    #[test]
    fn extent_scans_bit_patterns() {
        let (buf, ext) = burst();
        assert_eq!(Extent::of(&buf), ext, "negative zeros are zeros");
        assert_eq!(
            Extent::within(&buf, Extent { start: ext.start + 1000, end: ext.end - 1000 }),
            Extent { start: ext.start + 1000, end: ext.end - 1000 }
        );
        assert_eq!(Extent::of(&vec![vec![0f32; 10], vec![0f32; 10]]), Extent::EMPTY);
        assert_eq!(Extent::EMPTY.union(Extent { start: 3, end: 5 }), Extent { start: 3, end: 5 });
        assert_eq!(Extent { start: 3, end: 5 }.union(Extent { start: 9, end: 9 }), Extent { start: 3, end: 5 });
        assert_eq!(Extent { start: 3, end: 5 }.widen(10, 10, 8), Extent { start: 0, end: 8 });
        assert!(Extent { start: 5, end: 5 }.widen(10, 10, 8).is_empty());
    }

    #[test]
    fn extent_limited_loudness_is_bit_identical() {
        let (buf, ext) = burst();
        let rate = 48000.0;
        for weights in [&[1.0, 1.0][..], &[1.0, 0.0][..], &[1.41, 1.0][..]] {
            let full = Extent::full(buf[0].len());
            let want = integrated_in(&buf, full, rate, weights);
            assert_eq!(integrated_in(&buf, ext, rate, weights).to_bits(), want.to_bits());
            assert_eq!(integrated(&buf, rate, weights).to_bits(), want.to_bits());
            let (a, b) = (short_term_in(&buf, ext, rate, weights, 0.1), short_term_in(&buf, full, rate, weights, 0.1));
            assert_eq!(a.len(), b.len());
            assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
            let e = block_energies(&buf, ext, rate, weights, 19200, 4800);
            assert!(e
                .iter()
                .zip(block_energies(&buf, full, rate, weights, 19200, 4800))
                .all(|(x, y)| x.to_bits() == y.to_bits()));
        }
        let full = Extent::full(buf[0].len());
        assert_eq!(true_peak_in(&buf, ext).to_bits(), true_peak_in(&buf, full).to_bits());
        assert_eq!(true_peak(&buf).to_bits(), true_peak_in(&buf, full).to_bits());
        assert!(true_peak(&buf) > -1.0);
        let mut a = buf.clone();
        let mut b = buf.clone();
        limit_in(&mut a, ext, rate, -3.0, 0.05);
        limit_in(&mut b, full, rate, -3.0, 0.05);
        let mut c = buf.clone();
        limit(&mut c, rate, -3.0, 0.05);
        assert!(c.iter().flatten().zip(b.iter().flatten()).all(|(x, y)| x.to_bits() == y.to_bits()));
        assert!(a.iter().flatten().zip(b.iter().flatten()).all(|(x, y)| x.to_bits() == y.to_bits()));
        assert!(true_peak(&a) < -2.9);
        // a silent programme: nothing to do
        let silent = vec![vec![0f32; 48000]; 2];
        assert_eq!(integrated_in(&silent, Extent::EMPTY, rate, &[1.0, 1.0]), -200.0);
        assert_eq!(true_peak_in(&silent, Extent::EMPTY), lin_to_db(0.0));
    }
}
