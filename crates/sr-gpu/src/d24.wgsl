// Seeded 64-bit hashing on the GPU (sr_vector::d24 on the CPU). WGSL has no 64-bit
// integers: a u64 is vec2<u32>(low, high) and products are built from 16-bit halves, so
// every draw has exactly the CPU's bits.

fn u64_add(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
  let lo = a.x + b.x;
  return vec2(lo, a.y + b.y + select(0u, 1u, lo < a.x));
}

// The full 64-bit product of two 32-bit words.
fn mul32_wide(a: u32, b: u32) -> vec2<u32> {
  let a0 = a & 0xffffu; let a1 = a >> 16u;
  let b0 = b & 0xffffu; let b1 = b >> 16u;
  let p00 = a0 * b0; let p01 = a0 * b1; let p10 = a1 * b0; let p11 = a1 * b1;
  let mid = (p00 >> 16u) + (p01 & 0xffffu) + (p10 & 0xffffu);
  return vec2((p00 & 0xffffu) | (mid << 16u), p11 + (p01 >> 16u) + (p10 >> 16u) + (mid >> 16u));
}

// The low 64 bits of a 64 × 64-bit product.
fn u64_mul(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
  let p = mul32_wide(a.x, b.x);
  return vec2(p.x, p.y + a.x * b.y + a.y * b.x);
}

// a >> n for 0 < n < 32.
fn u64_shr(a: vec2<u32>, n: u32) -> vec2<u32> {
  return vec2((a.x >> n) | (a.y << (32u - n)), a.y >> n);
}

// A signed 32-bit integer as two's-complement u64.
fn u64_of_i32(i: i32) -> vec2<u32> {
  return vec2(bitcast<u32>(i), select(0u, 0xffffffffu, i < 0));
}

fn splitmix64(x: vec2<u32>) -> vec2<u32> {
  var z = u64_add(x, vec2(0x7f4a7c15u, 0x9e3779b9u));
  z = u64_mul(z ^ u64_shr(z, 30u), vec2(0x1ce4e5b9u, 0xbf58476du));
  z = u64_mul(z ^ u64_shr(z, 27u), vec2(0x133111ebu, 0x94d049bbu));
  return z ^ u64_shr(z, 31u);
}

// splitmix64(seed ^ splitmix64(channel ^ splitmix64(i))).
fn d24_hash(seed: vec2<u32>, channel: vec2<u32>, i: vec2<u32>) -> vec2<u32> {
  return splitmix64(seed ^ splitmix64(channel ^ splitmix64(i)));
}

// Uniform in [0, 1): the top 24 of the 53 bits the CPU uses (exact in f32).
fn d24_unit(seed: vec2<u32>, channel: vec2<u32>, i: vec2<u32>) -> f32 {
  return f32(d24_hash(seed, channel, i).y >> 8u) * (1.0 / 16777216.0);
}

// Lattice index of (i, j, k): 21 bits each, i | j << 21 | k << 42.
fn d24_pack(i: i32, j: i32, k: i32) -> vec2<u32> {
  let m = 0x1fffffu;
  let a = bitcast<u32>(i) & m; let b = bitcast<u32>(j) & m; let c = bitcast<u32>(k) & m;
  return vec2(a | (b << 21u), (b >> 11u) | (c << 10u));
}

// A standard normal draw (Box-Muller over the hash's high and low words).
fn d24_gaussian(seed: vec2<u32>, channel: vec2<u32>, i: vec2<u32>) -> f32 {
  let h = d24_hash(seed, channel, i);
  let u1 = (f32(h.y) + 0.5) * (1.0 / 4294967296.0);
  let u2 = f32(h.x) * (1.0 / 4294967296.0);
  return sqrt(-2.0 * log(max(u1, 1e-38))) * cos(6.2831853 * u2);
}

// The dissolve hash of a frame pixel and the project seed: seed ^ x · φ64
// ^ y · 0xC2B2AE3D27D4EB4F, MurmurHash3's 64-bit finaliser, top 24 bits.
fn dissolve_hash(x: u32, y: u32, seed: vec2<u32>) -> f32 {
  var h = seed ^ u64_mul(vec2(x, 0u), vec2(0x7f4a7c15u, 0x9e3779b9u)) ^ u64_mul(vec2(y, 0u), vec2(0x27d4eb4fu, 0xc2b2ae3du));
  h = h ^ vec2(h.y >> 1u, 0u);
  h = u64_mul(h, vec2(0xed558ccdu, 0xff51afd7u));
  h = h ^ vec2(h.y >> 1u, 0u);
  h = u64_mul(h, vec2(0x1a85ec53u, 0xc4ceb9feu));
  h = h ^ vec2(h.y >> 1u, 0u);
  return f32(h.y >> 8u) * (1.0 / 16777216.0);
}
