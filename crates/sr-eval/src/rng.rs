//! Counter-based randomness and gradient noise.
//!
//! Every random value is a pure function of its key: project seed, element
//! seed, a stream id and a counter (usually the frame index). There is no
//! hidden state, so any frame can be evaluated in any order on any thread and
//! produce the same bits on every machine. Noise uses only integer hashing
//! and the portable `libm` implementations of transcendental functions.

/// SplitMix64 finaliser: a bijective 64-bit mixer with full avalanche.
#[inline]
pub fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

pub use sr_vector::d24::{d24_hash, d24_unit, fractal, index2, index3, noise, splitmix64, weighted_octaves};

/// Hashes a sequence of words into one.
#[inline]
pub fn hash(words: &[u64]) -> u64 {
    let mut h = 0x9e37_79b9_7f4a_7c15u64;
    for &w in words {
        h = mix64(h ^ mix64(w.wrapping_add(0x9e37_79b9_7f4a_7c15)));
    }
    h
}

/// FNV-1a over bytes, used to derive element seeds from ids.
pub fn hash_str(s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    mix64(h)
}

/// Uniform double in [0, 1) from 64 random bits.
#[inline]
pub fn unit(bits: u64) -> f64 {
    (bits >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

/// A random stream keyed by seed; values are addressed, not generated in sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Stream {
    key: u64,
}

impl Stream {
    /// Stream for a project seed, element seed and stream id.
    pub fn new(project_seed: u64, element_seed: u64, stream: u64) -> Self {
        Stream { key: hash(&[project_seed, element_seed, stream]) }
    }

    /// Uniform double in [0, 1) at `(frame, n)`.
    #[inline]
    pub fn at(self, frame: i64, n: u64) -> f64 {
        unit(hash(&[self.key, frame as u64, n]))
    }

    /// The key, for deriving sub-streams.
    pub fn key(self) -> u64 {
        self.key
    }
}

#[inline]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

#[inline]
fn grad1(seed: u64, i: i64) -> f64 {
    unit(hash(&[seed, i as u64])) * 2.0 - 1.0
}

#[inline]
fn grad2(seed: u64, i: i64, j: i64, x: f64, y: f64) -> f64 {
    let h = hash(&[seed, i as u64, j as u64]);
    // 8 unit gradients at 45 degree steps
    const G: [(f64, f64); 8] = [
        (1.0, 0.0),
        (-1.0, 0.0),
        (0.0, 1.0),
        (0.0, -1.0),
        (std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2),
        (-std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2),
        (std::f64::consts::FRAC_1_SQRT_2, -std::f64::consts::FRAC_1_SQRT_2),
        (-std::f64::consts::FRAC_1_SQRT_2, -std::f64::consts::FRAC_1_SQRT_2),
    ];
    let (gx, gy) = G[(h & 7) as usize];
    gx * x + gy * y
}

#[inline]
fn grad3(seed: u64, i: i64, j: i64, k: i64, x: f64, y: f64, z: f64) -> f64 {
    let h = hash(&[seed, i as u64, j as u64, k as u64]) % 12;
    // the 12 edge directions of a cube (Perlin 2002)
    match h {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        3 => -x - y,
        4 => x + z,
        5 => -x + z,
        6 => x - z,
        7 => -x - z,
        8 => y + z,
        9 => -y + z,
        10 => y - z,
        _ => -y - z,
    }
}

/// 1D gradient noise in [-1, 1], zero at integer lattice points.
pub fn noise1(seed: u64, x: f64) -> f64 {
    let i = libm::floor(x);
    let f = x - i;
    let i = i as i64;
    let a = grad1(seed, i) * f;
    let b = grad1(seed, i + 1) * (f - 1.0);
    (lerp(a, b, fade(f)) * 2.0).clamp(-1.0, 1.0)
}

/// 2D gradient noise in [-1, 1].
pub fn noise2(seed: u64, x: f64, y: f64) -> f64 {
    let (xi, yi) = (libm::floor(x), libm::floor(y));
    let (xf, yf) = (x - xi, y - yi);
    let (i, j) = (xi as i64, yi as i64);
    let (u, v) = (fade(xf), fade(yf));
    let n00 = grad2(seed, i, j, xf, yf);
    let n10 = grad2(seed, i + 1, j, xf - 1.0, yf);
    let n01 = grad2(seed, i, j + 1, xf, yf - 1.0);
    let n11 = grad2(seed, i + 1, j + 1, xf - 1.0, yf - 1.0);
    (lerp(lerp(n00, n10, u), lerp(n01, n11, u), v) * std::f64::consts::SQRT_2).clamp(-1.0, 1.0)
}

/// 3D gradient noise in [-1, 1].
pub fn noise3(seed: u64, x: f64, y: f64, z: f64) -> f64 {
    let (xi, yi, zi) = (libm::floor(x), libm::floor(y), libm::floor(z));
    let (xf, yf, zf) = (x - xi, y - yi, z - zi);
    let (i, j, k) = (xi as i64, yi as i64, zi as i64);
    let (u, v, w) = (fade(xf), fade(yf), fade(zf));
    let c =
        |di: i64, dj: i64, dk: i64| grad3(seed, i + di, j + dj, k + dk, xf - di as f64, yf - dj as f64, zf - dk as f64);
    let x00 = lerp(c(0, 0, 0), c(1, 0, 0), u);
    let x10 = lerp(c(0, 1, 0), c(1, 1, 0), u);
    let x01 = lerp(c(0, 0, 1), c(1, 0, 1), u);
    let x11 = lerp(c(0, 1, 1), c(1, 1, 1), u);
    (lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)).clamp(-1.0, 1.0)
}

/// Fractal (octave-summed) 1D noise, normalised to [-1, 1].
/// Octave `o` runs at `2^o` times the base frequency with weight `amp_mult^o`.
pub fn fbm1(seed: u64, x: f64, octaves: u32, amp_mult: f64) -> f64 {
    let (mut sum, mut norm, mut w, mut f) = (0.0, 0.0, 1.0, 1.0);
    for o in 0..octaves.clamp(1, 16) {
        let s = mix64(seed ^ o as u64);
        // a per-octave phase keeps lattice points (where gradient noise is 0) off integer inputs
        let phase = unit(mix64(s ^ 0x5eed)) * 1024.0;
        sum += w * noise1(s, x * f + phase);
        norm += w;
        w *= amp_mult;
        f *= 2.0;
    }
    if norm > 0.0 {
        sum / norm
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_are_addressable_and_stable() {
        let s = Stream::new(7, 42, 0);
        assert_eq!(s.at(10, 3), s.at(10, 3));
        assert_ne!(s.at(10, 3), s.at(11, 3));
        assert_ne!(s.at(10, 3), Stream::new(8, 42, 0).at(10, 3));
        // golden value: any change to the mixer breaks cross-version determinism
        assert_eq!(hash(&[1, 2, 3]), 0xd56d_0264_932a_233f);
        assert_eq!(Stream::new(1, 2, 3).at(4, 5), 0.1493229690615926);
        let mean: f64 = (0..10_000).map(|i| s.at(i, 0)).sum::<f64>() / 10_000.0;
        assert!((mean - 0.5).abs() < 0.01, "{mean}");
    }

    #[test]
    fn noise_is_bounded_and_continuous() {
        for i in 0..2000 {
            let x = i as f64 * 0.0137 - 7.0;
            let n = noise1(3, x);
            assert!((-1.0..=1.0).contains(&n));
            assert!((noise1(3, x + 1e-6) - n).abs() < 1e-4);
            assert!((-1.0..=1.0).contains(&noise2(3, x, x * 0.7)));
            assert!((-1.0..=1.0).contains(&noise3(3, x, -x, x * 0.3)));
        }
        assert_eq!(noise1(3, 5.0), 0.0);
        assert!((-1.0..=1.0).contains(&fbm1(9, 1.234, 4, 0.5)));
    }
}
