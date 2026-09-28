// Frame interpolation: frame mix and DIS optical flow (Kroeger et al.,
// "Fast Optical Flow using Dense Inverse Search", ECCV 2016) on a luma
// pyramid: inverse-compositional patch search per level, densification by
// photometric-error weighting, then warp-and-blend at the requested phase.

struct FlowParams {
  size: vec2<u32>,      // current level size
  grid: vec2<u32>,      // patch grid size
  coarse: vec2<u32>,    // coarser flow size (0 at the coarsest level)
  t: f32,               // interpolation phase
  iterations: u32,
};

@group(0) @binding(0) var<uniform> fp: FlowParams;

// ---------------------------------------------------------------- luma

@group(1) @binding(0) var src_a: texture_2d<f32>;
@group(1) @binding(1) var src_b: texture_2d<f32>;
@group(1) @binding(2) var dst_a: texture_storage_2d<r32float, write>;
@group(1) @binding(3) var dst_b: texture_storage_2d<r32float, write>;

fn lum(c: vec4<f32>) -> f32 { return dot(c.rgb, vec3(0.2126, 0.7152, 0.0722)); }

// 2×2 box reduction; level 0 reads the RGBA frames
@compute @workgroup_size(8, 8)
fn cs_reduce(@builtin(global_invocation_id) id: vec3<u32>) {
  if (id.x >= fp.size.x || id.y >= fp.size.y) { return; }
  let s = vec2<i32>(textureDimensions(src_a)) - vec2(1);
  let b = vec2<i32>(id.xy) * 2;
  var sa = 0.0; var sb = 0.0;
  for (var k = 0; k < 4; k = k + 1) {
    let c = min(b + vec2(k & 1, k >> 1u), s);
    sa = sa + lum(textureLoad(src_a, c, 0));
    sb = sb + lum(textureLoad(src_b, c, 0));
  }
  textureStore(dst_a, vec2<i32>(id.xy), vec4(sa * 0.25));
  textureStore(dst_b, vec2<i32>(id.xy), vec4(sb * 0.25));
}

// ---------------------------------------------------------------- patch search

@group(2) @binding(0) var i0: texture_2d<f32>;
@group(2) @binding(1) var i1: texture_2d<f32>;
@group(2) @binding(2) var coarse: texture_2d<f32>;
@group(2) @binding(3) var<storage, read_write> patches: array<vec2<f32>>;
@group(2) @binding(4) var dense: texture_storage_2d<rgba32float, write>;

fn ld(t: texture_2d<f32>, c: vec2<i32>) -> f32 {
  return textureLoad(t, clamp(c, vec2(0), vec2<i32>(textureDimensions(t)) - vec2(1)), 0).r;
}

fn bil(t: texture_2d<f32>, p: vec2<f32>) -> f32 {
  let i = vec2<i32>(floor(p));
  let f = p - floor(p);
  return mix(mix(ld(t, i), ld(t, i + vec2(1, 0)), f.x), mix(ld(t, i + vec2(0, 1)), ld(t, i + vec2(1, 1)), f.x), f.y);
}

fn coarse_flow(p: vec2<f32>) -> vec2<f32> {
  if (fp.coarse.x == 0u) { return vec2(0.0); }
  let q = p * 0.5 - 0.5;
  let i = vec2<i32>(floor(q));
  let f = q - floor(q);
  let d = vec2<i32>(fp.coarse) - vec2(1);
  let a = textureLoad(coarse, clamp(i, vec2(0), d), 0).xy;
  let b = textureLoad(coarse, clamp(i + vec2(1, 0), vec2(0), d), 0).xy;
  let c = textureLoad(coarse, clamp(i + vec2(0, 1), vec2(0), d), 0).xy;
  let e = textureLoad(coarse, clamp(i + vec2(1, 1), vec2(0), d), 0).xy;
  return mix(mix(a, b, f.x), mix(c, e, f.x), f.y) * 2.0;
}

const PS: i32 = 8;
const STRIDE: i32 = 4;

@compute @workgroup_size(8, 8)
fn cs_patch(@builtin(global_invocation_id) id: vec3<u32>) {
  if (id.x >= fp.grid.x || id.y >= fp.grid.y) { return; }
  let o = vec2<i32>(id.xy) * STRIDE;
  var u = coarse_flow(vec2<f32>(o) + vec2(f32(PS) * 0.5));
  // template statistics and inverse-compositional Hessian
  var mean0 = 0.0;
  for (var k = 0; k < PS * PS; k = k + 1) { mean0 = mean0 + ld(i0, o + vec2(k % PS, k / PS)); }
  mean0 = mean0 / f32(PS * PS);
  var h = vec3(0.0);
  for (var k = 0; k < PS * PS; k = k + 1) {
    let c = o + vec2(k % PS, k / PS);
    let g = vec2(ld(i0, c + vec2(1, 0)) - ld(i0, c - vec2(1, 0)), ld(i0, c + vec2(0, 1)) - ld(i0, c - vec2(0, 1))) * 0.5;
    h = h + vec3(g.x * g.x, g.x * g.y, g.y * g.y);
  }
  let det = h.x * h.z - h.y * h.y;
  if (abs(det) > 1e-9) {
    for (var it = 0u; it < fp.iterations; it = it + 1u) {
      var mean1 = 0.0;
      for (var k = 0; k < PS * PS; k = k + 1) { mean1 = mean1 + bil(i1, vec2<f32>(o + vec2(k % PS, k / PS)) + u); }
      mean1 = mean1 / f32(PS * PS);
      var bv = vec2(0.0);
      for (var k = 0; k < PS * PS; k = k + 1) {
        let c = o + vec2(k % PS, k / PS);
        let g = vec2(ld(i0, c + vec2(1, 0)) - ld(i0, c - vec2(1, 0)), ld(i0, c + vec2(0, 1)) - ld(i0, c - vec2(0, 1))) * 0.5;
        let e = (bil(i1, vec2<f32>(c) + u) - mean1) - (ld(i0, c) - mean0);
        bv = bv + g * e;
      }
      let du = vec2(h.z * bv.x - h.y * bv.y, h.x * bv.y - h.y * bv.x) / det;
      u = u - du;
      if (dot(du, du) < 1e-4) { break; }
    }
  }
  patches[id.y * fp.grid.x + id.x] = u;
}

@compute @workgroup_size(8, 8)
fn cs_densify(@builtin(global_invocation_id) id: vec3<u32>) {
  if (id.x >= fp.size.x || id.y >= fp.size.y) { return; }
  let p = vec2<i32>(id.xy);
  let lo = max((p - vec2(PS - 1) + vec2(STRIDE - 1)) / STRIDE, vec2(0));
  let hi = min(p / STRIDE, vec2<i32>(fp.grid) - vec2(1));
  var acc = vec2(0.0);
  var wsum = 0.0;
  let v0 = ld(i0, p);
  for (var y = lo.y; y <= hi.y; y = y + 1) {
    for (var x = lo.x; x <= hi.x; x = x + 1) {
      let u = patches[u32(y) * fp.grid.x + u32(x)];
      let w = 1.0 / max(abs(bil(i1, vec2<f32>(p) + u) - v0), 0.01);
      acc = acc + u * w;
      wsum = wsum + w;
    }
  }
  var u = coarse_flow(vec2<f32>(p) + 0.5);
  if (wsum > 0.0) { u = acc / wsum; }
  textureStore(dense, p, vec4(u, 0.0, 1.0));
}

// ---------------------------------------------------------------- warp and blend

@group(3) @binding(0) var fa: texture_2d<f32>;
@group(3) @binding(1) var fb: texture_2d<f32>;
@group(3) @binding(2) var flow0: texture_2d<f32>;
@group(3) @binding(3) var samp: sampler;

struct VOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> VOut {
  var p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
  var o: VOut;
  o.pos = vec4(p[i], 0.0, 1.0);
  return o;
}

@fragment
fn fs_mix(v: VOut) -> @location(0) vec4<f32> {
  let c = vec2<i32>(v.pos.xy);
  return mix(textureLoad(fa, c, 0), textureLoad(fb, c, 0), fp.t);
}

@fragment
fn fs_warp(v: VOut) -> @location(0) vec4<f32> {
  let full = vec2<f32>(textureDimensions(fa));
  // the flow field lives at half resolution
  let q = v.pos.xy * 0.5 - 0.5;
  let i = vec2<i32>(floor(q));
  let f = q - floor(q);
  let d = vec2<i32>(textureDimensions(flow0)) - vec2(1);
  let a = textureLoad(flow0, clamp(i, vec2(0), d), 0).xy;
  let b = textureLoad(flow0, clamp(i + vec2(1, 0), vec2(0), d), 0).xy;
  let c = textureLoad(flow0, clamp(i + vec2(0, 1), vec2(0), d), 0).xy;
  let e = textureLoad(flow0, clamp(i + vec2(1, 1), vec2(0), d), 0).xy;
  let u = mix(mix(a, b, f.x), mix(c, e, f.x), f.y) * 2.0;
  let pa = (v.pos.xy - fp.t * u) / full;
  let pb = (v.pos.xy + (1.0 - fp.t) * u) / full;
  return mix(textureSampleLevel(fa, samp, pa, 0.0), textureSampleLevel(fb, samp, pb, 0.0), fp.t);
}
