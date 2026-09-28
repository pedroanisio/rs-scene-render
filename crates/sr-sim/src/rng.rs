//! Counter-based randomness and smooth noise.

/// A 64-bit hash of a seed and counters (SplitMix64 finaliser).
pub fn hash(seed: u64, a: u64, b: u64) -> u64 {
    let mut x = seed ^ a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Uniform in [0, 1).
pub fn unit(seed: u64, a: u64, b: u64) -> f64 {
    (hash(seed, a, b) >> 11) as f64 / (1u64 << 53) as f64
}

/// Uniform in [−1, 1).
pub fn signed(seed: u64, a: u64, b: u64) -> f64 {
    unit(seed, a, b) * 2.0 - 1.0
}

fn lattice(seed: u64, x: i64, y: i64) -> f64 {
    signed(seed, x as u64, y as u64)
}

/// Smooth 2D value noise in [−1, 1].
pub fn noise2(seed: u64, x: f64, y: f64) -> f64 {
    let (xi, yi) = (x.floor(), y.floor());
    let (fx, fy) = (x - xi, y - yi);
    let (u, v) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (x0, y0) = (xi as i64, yi as i64);
    let a = lattice(seed, x0, y0) + (lattice(seed, x0 + 1, y0) - lattice(seed, x0, y0)) * u;
    let b = lattice(seed, x0, y0 + 1) + (lattice(seed, x0 + 1, y0 + 1) - lattice(seed, x0, y0 + 1)) * u;
    a + (b - a) * v
}

/// Divergence-free 2D flow (the curl of value noise), roughly unit magnitude.
pub fn curl2(seed: u64, x: f64, y: f64) -> [f64; 2] {
    let e = 0.01;
    let dx = (noise2(seed, x + e, y) - noise2(seed, x - e, y)) / (2.0 * e);
    let dy = (noise2(seed, x, y + e) - noise2(seed, x, y - e)) / (2.0 * e);
    [dy * 0.5, -dx * 0.5]
}
