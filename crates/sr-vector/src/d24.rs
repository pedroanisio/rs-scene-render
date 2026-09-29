//! D24 noise and hashing (c-scene-render docs/xml11-definitions.md; CONVENTIONS.md 5.19).
//!
//! Every seeded function of the renderer draws from these: N(seed, channel, x), its
//! fractal sum, and the lattice hash splitmix64(seed ⊕ splitmix64(channel ⊕ splitmix64(i)))
//! whose top 53 bits give a uniform value. Integers enter as two's-complement u64.

/// SplitMix64 finaliser.
#[inline]
fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Uniform double in [0, 1) from the top 53 bits.
#[inline]
fn unit(bits: u64) -> f64 {
    (bits >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

#[inline]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// The index of a pair of integers, for 2D lattices and pixels:
/// splitmix64(a ⊕ splitmix64(b)).
#[inline]
pub fn index2(a: i64, b: i64) -> u64 {
    splitmix64(a as u64 ^ splitmix64(b as u64))
}

/// The index of a triple: splitmix64(a ⊕ splitmix64(b ⊕ splitmix64(c))).
#[inline]
pub fn index3(a: i64, b: i64, c: i64) -> u64 {
    splitmix64(a as u64 ^ splitmix64(b as u64 ^ splitmix64(c as u64)))
}

/// The standard SplitMix64 step: adds the golden-ratio increment, then finalises (D24).
#[inline]
pub fn splitmix64(x: u64) -> u64 {
    mix64(x.wrapping_add(0x9e37_79b9_7f4a_7c15))
}

/// D24's lattice hash: splitmix64(seed ⊕ splitmix64(channel ⊕ splitmix64(i))).
#[inline]
pub fn d24_hash(seed: u64, channel: u64, i: u64) -> u64 {
    splitmix64(seed ^ splitmix64(channel ^ splitmix64(i)))
}

/// Uniform double in [0, 1) from the top 53 bits of [`d24_hash`]: the value every
/// seeded per-element or per-pixel draw uses.
#[inline]
pub fn d24_unit(seed: u64, channel: u64, i: u64) -> f64 {
    unit(d24_hash(seed, channel, i))
}

/// D24 noise N(seed, channel, x): 1D Perlin gradient noise with gradients
/// h / 2⁵³ · 2 − 1 at the lattice points and the quintic fade. In [−1, 1], zero at integers.
pub fn noise(seed: u64, channel: u64, x: f64) -> f64 {
    let fl = libm::floor(x);
    let f = x - fl;
    let i = fl as i64;
    let g = |i: i64| d24_unit(seed, channel, i as u64) * 2.0 - 1.0;
    let a = g(i) * f;
    let b = g(i.wrapping_add(1)) * (f - 1.0);
    2.0 * (a + (b - a) * fade(f))
}

/// D24 fractal noise: octave k weighs 0.5ᵏ and samples channel · 1024 + k at x · 2ᵏ;
/// the sum is divided by the sum of the weights.
pub fn fractal(seed: u64, channel: u64, x: f64, octaves: u32) -> f64 {
    weighted_octaves(seed, channel, x, octaves, 0.5)
}

/// Octave sum with weights `mult`ᵏ over channels `channel` · 1024 + k (D24 fractal
/// noise at `mult` = 0.5, D25 `wiggle` otherwise), normalised by the weights.
pub fn weighted_octaves(seed: u64, channel: u64, x: f64, octaves: u32, mult: f64) -> f64 {
    let (mut sum, mut norm, mut w, mut f) = (0.0, 0.0, 1.0, 1.0);
    for k in 0..octaves.clamp(1, 16) {
        sum += w * noise(seed, channel.wrapping_mul(1024).wrapping_add(k as u64), x * f);
        norm += w;
        w *= mult;
        f *= 2.0;
    }
    if norm != 0.0 {
        sum / norm
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden values of D24, computed by an independent reference of the definition
    /// (and matching c-scene-render's `sr_noise1` / `sr_noise_fractal`).
    #[test]
    fn d24_golden_values() {
        assert_eq!(splitmix64(0), 0xe220_a839_7b1d_cdaf);
        assert_eq!(splitmix64(1), 0x910a_2dec_8902_5cc1);
        assert_eq!(d24_hash(1, 2, 3), 0xd62a_7f6e_291f_ef64);
        assert_eq!(d24_hash(42, 7, -5i64 as u64), 0x2bf4_c61d_835a_19bb);
        let close = |a: f64, b: f64| assert!((a - b).abs() < 1e-12, "{a} != {b}");
        close(d24_unit(0, 0, 0) * 2.0 - 1.0, -0.7225811797088915);
        close(d24_unit(42, 7, -1i64 as u64) * 2.0 - 1.0, 0.6293362413092893);
        close(noise(0, 0, 0.5), -0.554802728864675);
        close(noise(1, 0, 0.25), -0.2591138999211539);
        close(noise(42, 7, -1.3), -0.22195948101587748);
        close(noise(12345, 3, 99.75), 0.052042264166522356);
        close(noise(7, 1024, 3.14159), 0.14007526307765447);
        assert_eq!(noise(0, 0, 2.0), 0.0);
        close(fractal(1, 0, 0.37, 1), -0.29073717200676585);
        close(fractal(1, 0, 0.37, 4), -0.11429312869138876);
        close(fractal(42, 3, -2.6, 3), 0.13784387603697973);
    }
}
