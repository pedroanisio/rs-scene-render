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

/// Lattice index of (i, j, k) for fields over the plane or space: 21 bits of each
/// (two's complement), i | j << 21 | k << 42 (the Python renderer's `noise.pack`).
#[inline]
pub fn pack(i: i64, j: i64, k: i64) -> u64 {
    const M: u64 = (1 << 21) - 1;
    (i as u64 & M) | (j as u64 & M) << 21 | (k as u64 & M) << 42
}

/// A standard normal draw from one hash (Box–Muller): u1 = (high 32 bits + ½) / 2³² and
/// u2 = low 32 bits / 2³² of [`d24_hash`].
pub fn gaussian(seed: u64, channel: u64, i: u64) -> f64 {
    let h = d24_hash(seed, channel, i);
    let u1 = ((h >> 32) as f64 + 0.5) / 4294967296.0;
    let u2 = (h & 0xffff_ffff) as f64 / 4294967296.0;
    libm::sqrt(-2.0 * libm::log(u1)) * libm::cos(std::f64::consts::TAU * u2)
}

/// A permutation of 0..n: Fisher–Yates over the draws U(seed, channel, 0), U(seed, channel, 1), …,
/// swapping i with ⌊u · (i + 1)⌋ for i = n − 1 down to 1.
pub fn permutation(seed: u64, channel: u64, n: usize) -> Vec<usize> {
    let mut p: Vec<usize> = (0..n).collect();
    for (k, i) in (1..n).rev().enumerate() {
        let j = (d24_unit(seed, channel, k as u64) * (i + 1) as f64) as usize;
        p.swap(i, j.min(i));
    }
    p
}

/// CRC-32 (IEEE, as zlib's `crc32`): per-element seeds and the channels of properties
/// outside D25's list are CRC-32s of text.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in bytes {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xedb8_8320 } else { c >> 1 };
        }
    }
    !c
}

/// A per-element seed: CRC-32 of "project seed:element id:@seed:purpose" (the Python
/// renderer's `seed_for`, used where D24 leaves the seed of an element's draws open).
pub fn element_seed(project: u64, id: &str, seed_attr: Option<u64>, purpose: &str) -> u64 {
    let s = seed_attr.map(|v| v.to_string()).unwrap_or_default();
    crc32(format!("{project}:{id}:{s}:{purpose}").as_bytes()) as u64
}

/// Improved Perlin noise (2002) over a 256-entry permutation, clamped to [−1, 1]: the
/// Python renderer's multi-dimensional noise, with the permutation drawn by [`permutation`].
pub fn perlin3(perm: &[usize], x: f64, y: f64, z: f64) -> f64 {
    if !(x.is_finite() && y.is_finite() && z.is_finite()) {
        return f64::NAN;
    }
    let p = |i: usize| perm[i & 255];
    let (fx, fy, fz) = (libm::floor(x), libm::floor(y), libm::floor(z));
    let (xi, yi, zi) = ((fx as i64 & 255) as usize, (fy as i64 & 255) as usize, (fz as i64 & 255) as usize);
    let (x, y, z) = (x - fx, y - fy, z - fz);
    let (u, v, w) = (fade(x), fade(y), fade(z));
    let grad = |h: usize, x: f64, y: f64, z: f64| {
        let h = h & 15;
        let a = if h < 8 { x } else { y };
        let b = if h < 4 {
            y
        } else if h == 12 || h == 14 {
            x
        } else {
            z
        };
        (if h & 1 == 0 { a } else { -a }) + (if h & 2 == 0 { b } else { -b })
    };
    let lerp = |t: f64, a: f64, b: f64| a + t * (b - a);
    let (a, b) = (p(xi) + yi, p(xi + 1) + yi);
    let (aa, ab, ba, bb) = (p(a) + zi, p(a + 1) + zi, p(b) + zi, p(b + 1) + zi);
    let r = lerp(
        w,
        lerp(
            v,
            lerp(u, grad(p(aa), x, y, z), grad(p(ba), x - 1.0, y, z)),
            lerp(u, grad(p(ab), x, y - 1.0, z), grad(p(bb), x - 1.0, y - 1.0, z)),
        ),
        lerp(
            v,
            lerp(u, grad(p(aa + 1), x, y, z - 1.0), grad(p(ba + 1), x - 1.0, y, z - 1.0)),
            lerp(u, grad(p(ab + 1), x, y - 1.0, z - 1.0), grad(p(bb + 1), x - 1.0, y - 1.0, z - 1.0)),
        ),
    );
    r.clamp(-1.0, 1.0)
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

    /// The Python renderer's choices where D24 is silent (scenerender/noise.py), with golden
    /// values from that module.
    #[test]
    fn draws_beyond_d24() {
        assert!((gaussian(1, 2, 3) - 0.3180452816872121).abs() < 1e-12);
        assert_eq!(permutation(5, 0, 10), [9, 8, 3, 7, 2, 6, 5, 0, 1, 4]);
        assert_eq!(pack(-1, 2, 3), 0xc00_005f_ffff);
        assert_eq!(crc32(b"fontSize"), 1178744372);
        assert_eq!(element_seed(1, "", Some(42), "order"), 3996963903);
        let perm = permutation(1, 7, 256);
        assert!((perlin3(&perm, 0.3, 0.7, 0.0) - 0.28006378512).abs() < 1e-9);
    }
}
