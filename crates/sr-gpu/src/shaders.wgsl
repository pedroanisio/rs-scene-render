// scene-render compositor shaders.
//
// All colours are premultiplied RGBA in the working space, stored linear
// unless the project disables linear-light compositing. Blend modes follow
// the W3C Compositing and Blending Level 1 formulas where they exist and
// the After Effects / Photoshop definitions otherwise.


struct Draw {
  color: vec4<f32>,
  uv_rect: vec4<f32>,
  box_rect: vec4<f32>,
  target_size: vec2<f32>,
  opacity: f32,
  blend: u32,
  src_kind: u32,
  matte_mode: u32,
  mask_off: u32,
  mask_count: u32,
  paint: u32,
  lod: f32,
  seed: u32,
  flags: u32,
};

struct Mask {
  rect: vec4<f32>,
  kind: u32,
  mode: u32,
  invert: u32,
  fill_rule: u32,
  edge_off: u32,
  edge_count: u32,
  feather: f32,
  expansion: f32,
  radius: f32,
  opacity: f32,
  pad0: f32,
  pad1: f32,
};



@group(0) @binding(0) var<uniform> draw_params: Draw;
@group(0) @binding(1) var<storage, read> masks: array<Mask>;
@group(0) @binding(2) var<storage, read> edges: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> paints: array<PaintDesc>;
@group(0) @binding(4) var<storage, read> stops: array<Stop>;
@group(0) @binding(5) var samp: sampler;
@group(0) @binding(6) var samp_repeat: sampler;
@group(0) @binding(7) var<uniform> globals: Globals;
@group(1) @binding(0) var src: texture_2d<f32>;
// the same texture again, for the repeating sampler (GL allows one sampler per texture binding)
@group(1) @binding(1) var src_rep: texture_2d<f32>;
@group(2) @binding(0) var backdrop: texture_2d<f32>;
@group(2) @binding(1) var matte: texture_2d<f32>;

struct VIn {
  @location(0) clip: vec4<f32>,
  @location(1) uv: vec2<f32>,
  @location(2) local: vec2<f32>,
  @builtin(instance_index) draw: u32,
};

struct VOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) local: vec2<f32>,
  @location(2) @interpolate(flat) draw: u32,
};

@vertex
fn vs_main(v: VIn) -> VOut {
  var o: VOut;
  o.pos = v.clip;
  o.uv = v.uv;
  o.local = v.local;
  o.draw = v.draw;
  return o;
}

// ------------------------------------------------------------------ masks

fn sd_box(p: vec2<f32>, h: vec2<f32>, r: f32) -> f32 {
  let q = abs(p) - h + vec2(r);
  return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
  let pa = p - a; let ba = b - a;
  let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-12), 0.0, 1.0);
  return length(pa - ba * h);
}

override SINGLE_FEATHERED_ELLIPSE: bool = false;
fn mask_kind(m: Mask) -> u32 {
  if (SINGLE_FEATHERED_ELLIPSE) { return 1u; }
  return m.kind;
}

fn mask_distance(m: Mask, p: vec2<f32>) -> f32 {
  let half_size = m.rect.zw * 0.5;
  let c = m.rect.xy + half_size;
  let q = p - c;
  if (mask_kind(m) == 0u) { return sd_box(q, half_size, 0.0); }
  if (mask_kind(m) == 2u) { return sd_box(q, half_size, min(m.radius, min(half_size.x, half_size.y))); }
  if (mask_kind(m) == 1u) {
    let r = max(half_size, vec2(1e-6));
    let k = length(q / r);
    return (k - 1.0) * min(r.x, r.y);
  }
  // polygon edges: signed distance with winding (nonzero) or parity (evenodd)
  var d = 1e30;
  var wind = 0;
  var cross_count = 0u;
  for (var i = 0u; i < m.edge_count; i = i + 1u) {
    let e = edges[m.edge_off + i];
    let a = e.xy; let b = e.zw;
    d = min(d, sd_segment(p, a, b));
    let side = (b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y);
    if (a.y <= p.y) {
      if (b.y > p.y && side > 0.0) { wind = wind + 1; cross_count = cross_count + 1u; }
    } else if (b.y <= p.y && side < 0.0) { wind = wind - 1; cross_count = cross_count + 1u; }
  }
  var inside = wind != 0;
  if (m.fill_rule == 1u) { inside = (cross_count & 1u) == 1u; }
  return select(d, -d, inside);
}

// Standard normal cumulative distribution (Abramowitz and Stegun 7.1.26, error below 1.5e-7).
fn phi(x: f32) -> f32 {
  let z = abs(x) * 0.70710678;
  let t = 1.0 / (1.0 + 0.3275911 * z);
  let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * exp(-z * z);
  return 0.5 * (1.0 + select(y, -y, x < 0.0));
}

// Half width of a rectangle (half extents h) with corners of radius r at height y from its centre;
// negative outside it.
fn rounded_half_width(h: vec2<f32>, r: f32, y: f32) -> f32 {
  let ay = abs(y);
  if (ay >= h.y) { return -1.0; }
  let dy = ay - (h.y - r);
  if (dy <= 0.0) { return h.x; }
  return h.x - r + sqrt(max(r * r - dy * dy, 0.0));
}

// Coverage of one mask before invert and opacity. Feather is a Gaussian blur of standard deviation
// `feather` local units: for rectangles, rounded rectangles and ellipses it integrates the blurred
// outline row by row (exact for rectangles); polygons and paths take Φ of the signed distance, exact
// along straight edges. Expansion grows or shrinks the outline first (a disc's maximum or minimum).
fn mask_value(m: Mask, p: vec2<f32>, aa: f32) -> f32 {
  if (!SINGLE_FEATHERED_ELLIPSE && m.feather <= 0.0) {
    let dist = mask_distance(m, p) - m.expansion;
    return clamp(0.5 - dist / max(aa, 1e-4), 0.0, 1.0);
  }
  let sigma = max(m.feather, 0.4 * aa);
  let e = m.expansion;
  let h = m.rect.zw * 0.5 + vec2(e);
  let q = p - (m.rect.xy + m.rect.zw * 0.5);
  if (mask_kind(m) == 3u) {
    return phi(-(mask_distance(m, p) - e) / sigma);
  }
  if (any(h <= vec2(0.0))) { return 0.0; }
  if (mask_kind(m) == 0u && e <= 0.0) {
    // eroding a rectangle keeps it a rectangle: a product of two edge integrals
    let cx = phi((q.x + h.x) / sigma) - phi((q.x - h.x) / sigma);
    let cy = phi((q.y + h.y) / sigma) - phi((q.y - h.y) / sigma);
    return clamp(cx * cy, 0.0, 1.0);
  }
  var r = 0.0;
  if (mask_kind(m) == 0u) { r = e; }
  if (mask_kind(m) == 2u) { r = max(min(m.radius, min(m.rect.z, m.rect.w) * 0.5) + e, 0.0); }
  r = min(r, min(h.x, h.y));
  // ∫ G(v) [Φ((q.x + w(y + v)) / σ) − Φ((q.x − w(y + v)) / σ)] dv over ±4σ, w the outline's half width
  let n = 32;
  let step = 8.0 * sigma / f32(n);
  var sum = 0.0;
  var wsum = 0.0;
  for (var i = 0; i < n; i = i + 1) {
    let v = (f32(i) + 0.5) * step - 4.0 * sigma;
    let g = exp(-0.5 * (v / sigma) * (v / sigma));
    wsum = wsum + g;
    let y = q.y + v;
    var hw = -1.0;
    if (mask_kind(m) == 1u) {
      let k = y / h.y;
      if (abs(k) < 1.0) { hw = h.x * sqrt(1.0 - k * k); }
    } else {
      hw = rounded_half_width(h, r, y);
    }
    if (hw > 0.0) { sum = sum + g * (phi((q.x + hw) / sigma) - phi((q.x - hw) / sigma)); }
  }
  return clamp(sum / wsum, 0.0, 1.0);
}

fn mask_coverage(d: Draw, p: vec2<f32>, aa: f32) -> f32 {
  if (!SINGLE_FEATHERED_ELLIPSE && d.mask_count == 0u) { return 1.0; }
  let first = masks[d.mask_off].mode;
  // add, lighten and difference start from nothing; the other modes carve from full coverage
  var cov = select(1.0, 0.0, first == 1u || first == 3u || first == 5u);
  for (var i = 0u; i < select(d.mask_count, 1u, SINGLE_FEATHERED_ELLIPSE); i = i + 1u) {
    let m = masks[d.mask_off + i];
    if (m.mode == 6u) { continue; }
    var v = mask_value(m, p, aa);
    if (m.invert == 1u) { v = 1.0 - v; }
    v = v * m.opacity;
    switch (m.mode) {
      case 0u: { cov = cov * v; }                  // intersect
      case 1u: { cov = cov + v - cov * v; }        // add (a + m − a·m)
      case 2u: { cov = cov * (1.0 - v); }          // subtract
      case 3u: { cov = max(cov, v); }              // lighten
      case 4u: { cov = min(cov, v); }              // darken
      case 5u: { cov = abs(cov - v); }             // difference
      default: {}
    }
  }
  return cov;
}

// ------------------------------------------------------------------ blend modes

fn blend_overlay(b: f32, s: f32) -> f32 { return select(1.0 - 2.0 * (1.0 - b) * (1.0 - s), 2.0 * b * s, b <= 0.5); }
fn blend_dodge(b: f32, s: f32) -> f32 {
  if (b <= 0.0) { return 0.0; }
  if (s >= 1.0) { return 1.0; }
  return min(1.0, b / (1.0 - s));
}
fn blend_burn(b: f32, s: f32) -> f32 {
  if (b >= 1.0) { return 1.0; }
  if (s <= 0.0) { return 0.0; }
  return 1.0 - min(1.0, (1.0 - b) / s);
}
fn blend_soft(b: f32, s: f32) -> f32 {
  if (s <= 0.5) { return b - (1.0 - 2.0 * s) * b * (1.0 - b); }
  var dd = sqrt(b);
  if (b <= 0.25) { dd = ((16.0 * b - 12.0) * b + 4.0) * b; }
  return b + (2.0 * s - 1.0) * (dd - b);
}
// W3C Compositing Level 1 luminance, for the non-separable modes and darker/lighter colour
fn lum(c: vec3<f32>) -> f32 { return dot(c, vec3(0.3, 0.59, 0.11)); }
fn set_lum(c: vec3<f32>, l: f32) -> vec3<f32> {
  let d = l - lum(c);
  var r = c + vec3(d);
  let ll = lum(r);
  let n = min(r.r, min(r.g, r.b));
  let x = max(r.r, max(r.g, r.b));
  if (n < 0.0) { r = vec3(ll) + (r - vec3(ll)) * ll / max(ll - n, 1e-6); }
  if (x > 1.0) { r = vec3(ll) + (r - vec3(ll)) * (1.0 - ll) / max(x - ll, 1e-6); }
  return r;
}
fn sat(c: vec3<f32>) -> f32 { return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b)); }
fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
  let mx = max(c.r, max(c.g, c.b));
  let mn = min(c.r, min(c.g, c.b));
  if (mx <= mn) { return vec3(0.0); }
  return (c - vec3(mn)) * s / (mx - mn);
}

// Separable and non-separable B(Cb, Cs) on straight colours. add, multiply and difference pass HDR
// values through; the other modes take their inputs clamped to [0, 1].
fn blend_fn(mode: u32, cb: vec3<f32>, cs: vec3<f32>) -> vec3<f32> {
  var b = cb; var s = cs;
  if (mode != 2u && mode != 17u && mode != 4u && mode != 7u) {
    b = clamp(cb, vec3(0.0), vec3(1.0));
    s = clamp(cs, vec3(0.0), vec3(1.0));
  }
  switch (mode) {
    case 2u, 17u: { return b + s; }                                    // add, linear-dodge
    case 4u: { return b * s; }                                          // multiply
    case 5u: { return b + s - b * s; }                                  // screen
    case 6u: { return vec3(blend_overlay(b.r, s.r), blend_overlay(b.g, s.g), blend_overlay(b.b, s.b)); }
    case 7u: { return abs(b - s); }                                     // difference
    case 8u: { return b + s - 2.0 * b * s; }                            // exclusion
    case 9u: { return max(b - s, vec3(0.0)); }                          // subtract
    case 10u: {                                                         // divide (1 where cs is 0 and cb is not)
      return select(min(b / max(s, vec3(1e-30)), vec3(1.0)), select(vec3(0.0), vec3(1.0), b > vec3(0.0)), s <= vec3(0.0));
    }
    case 11u: { return min(b, s); }                                     // darken
    case 12u: { return max(b, s); }                                     // lighten
    case 13u: { return select(b, s, lum(s) < lum(b)); }                 // darker-color
    case 14u: { return select(b, s, lum(s) > lum(b)); }                 // lighter-color
    case 15u: { return vec3(blend_dodge(b.r, s.r), blend_dodge(b.g, s.g), blend_dodge(b.b, s.b)); }
    case 16u: { return vec3(blend_burn(b.r, s.r), blend_burn(b.g, s.g), blend_burn(b.b, s.b)); }
    case 18u: { return max(b + s - vec3(1.0), vec3(0.0)); }             // linear-burn
    case 19u: { return vec3(blend_soft(b.r, s.r), blend_soft(b.g, s.g), blend_soft(b.b, s.b)); }
    case 20u: { return vec3(blend_overlay(s.r, b.r), blend_overlay(s.g, b.g), blend_overlay(s.b, b.b)); } // hard-light
    case 21u: { return clamp(b + 2.0 * s - vec3(1.0), vec3(0.0), vec3(1.0)); }                            // linear-light
    case 22u: {                                                                                             // vivid-light
      var r: vec3<f32>;
      for (var i = 0; i < 3; i = i + 1) {
        r[i] = select(blend_dodge(b[i], 2.0 * (s[i] - 0.5)), blend_burn(b[i], 2.0 * s[i]), s[i] <= 0.5);
      }
      return r;
    }
    case 23u: {                                                                                             // pin-light
      var r: vec3<f32>;
      for (var i = 0; i < 3; i = i + 1) {
        r[i] = select(max(b[i], 2.0 * s[i] - 1.0), min(b[i], 2.0 * s[i]), s[i] <= 0.5);
      }
      return r;
    }
    case 24u: { return select(vec3(0.0), vec3(1.0), b + s >= vec3(1.0)); }                                 // hard-mix
    case 25u: { return set_lum(set_sat(s, sat(b)), lum(b)); }                                             // hue
    case 26u: { return set_lum(set_sat(b, sat(s)), lum(b)); }                                             // saturation
    case 27u: { return set_lum(s, lum(b)); }                                                              // color
    case 28u: { return set_lum(b, lum(s)); }                                                              // luminosity
    default: { return s; }
  }
}

// Composites premultiplied source `s` over premultiplied backdrop `bd` with `mode`.
fn composite(mode: u32, bd: vec4<f32>, s: vec4<f32>) -> vec4<f32> {
  let ab = bd.a; let as_ = s.a;
  switch (mode) {
    case 3u: { return min(bd + s, vec4(1.0)); }                         // plus-lighter
    case 29u: { return bd * as_; }                                      // stencil-alpha
    case 30u: {                                                         // stencil-luma
      let cs = select(vec3(0.0), s.rgb / as_, as_ > 0.0);
      return bd * luma(cs) * as_;
    }
    case 31u: { return bd * (1.0 - as_); }                              // silhouette-alpha
    case 32u: {                                                         // silhouette-luma
      let cs = select(vec3(0.0), s.rgb / as_, as_ > 0.0);
      return bd * (1.0 - luma(cs) * as_);
    }
    case 33u: { return vec4(s.rgb + bd.rgb * (1.0 - as_), min(1.0, as_ + ab)); } // alpha-add
    case 34u: { return bd + s * (1.0 - ab); }                          // behind
    default: {}
  }
  let cb = select(vec3(0.0), bd.rgb / ab, ab > 0.0);
  let cs = select(vec3(0.0), s.rgb / as_, as_ > 0.0);
  var mixed: vec3<f32>;
  mixed = blend_fn(mode, cb, cs);
  let rgb = (1.0 - ab) * s.rgb + (1.0 - as_) * bd.rgb + as_ * ab * mixed;
  return vec4(rgb, as_ + ab * (1.0 - as_));
}

// ------------------------------------------------------------------ fragment shading

// The coverage (masks, matte, opacity) shade() applied last.
var<private> COV: f32 = 1.0;

fn shade(v: VOut, cached_mask: bool) -> vec4<f32> {
  let d = draw_params;
  let pixel = vec2<u32>(v.pos.xy);
  let dlx = dpdx(v.local);
  let dly = dpdy(v.local);
  let aa = max(length(dlx), length(dly));
  let aa2 = vec2(max(length(vec2(dlx.x, dly.x)), 1e-6), max(length(vec2(dlx.y, dly.y)), 1e-6));
  var c: vec4<f32>;
  switch (d.src_kind) {
    case 0u: { c = textureSample(src, samp, v.uv); }
    case 1u: { c = vec4(d.color.rgb * d.color.a, d.color.a); }
    case 2u: {
      let p = eval_paint(d.paint, v.local, pixel);
      c = vec4(p.rgb * p.a, p.a);
    }
    case 3u: { c = textureSampleLevel(src, samp, v.uv, d.lod); }
    case 4u: {
      let pg = paints[d.paint];
      let puv = vec2(pg.xform0.x * v.local.x + pg.xform0.z * v.local.y + pg.xform1.x, pg.xform0.y * v.local.x + pg.xform0.w * v.local.y + pg.xform1.y);
      c = textureSample(src_rep, samp_repeat, puv);
    }
    default: { c = vec4(0.0); }
  }
  if ((d.flags & 16u) != 0u) {
    c = vec4(vec3(d.color.r) * c.a, c.a);
  }
  var coverage = 0.0;
  if (cached_mask) { coverage = textureLoad(matte, vec2<i32>(pixel), 0).r; }
  else { coverage = mask_coverage(d, v.local, aa); }
  var cov = coverage * d.opacity;
  if ((d.flags & 8u) != 0u) {
    // analytic edge coverage against the content rectangle
    let lo = min(d.box_rect.xy, d.box_rect.zw);
    let hi = max(d.box_rect.xy, d.box_rect.zw);
    let half_size = (hi - lo) * 0.5;
    let q = abs(v.local - (lo + half_size)) - half_size;
    let e = clamp(vec2(0.5) - q / aa2, vec2(0.0), vec2(1.0));
    cov = cov * e.x * e.y;
  }
  if ((d.flags & 2u) != 0u) {
    let m = textureLoad(matte, vec2<i32>(pixel), 0);
    var mv = m.a;
    if (d.matte_mode >= 2u) { mv = luma(to_lin(m.rgb)); }
    if (d.matte_mode == 1u || d.matte_mode == 3u) { mv = 1.0 - mv; }
    cov = cov * clamp(mv, 0.0, 1.0);
  }
  c = c * cov;
  COV = cov;
  if (d.blend == 1u) {                                             // dissolve
    let r = dissolve_hash(pixel.x, pixel.y, vec2(globals.seed, globals.pad0));
    if (c.a > 0.0 && r < c.a) { c = vec4(c.rgb / c.a, 1.0); } else { c = vec4(0.0); }
  }
  return c;
}

@fragment
fn fs_over(v: VOut) -> @location(0) vec4<f32> {
  return shade(v, false);
}

// Full-precision coverage is reusable only with the identical rasterized geometry.
@fragment
fn fs_mask_coverage(v: VOut) -> @location(0) f32 {
  let aa = max(length(dpdx(v.local)), length(dpdy(v.local)));
  return mask_coverage(draw_params, v.local, aa);
}

@fragment
fn fs_cached_mask(v: VOut) -> @location(0) vec4<f32> {
  return shade(v, true);
}

// The CPU selects these only for unmasked draws without mattes, analytic
// edges, contrast replacement, or dissolve. Keep the premultiplication and
// opacity arithmetic in the same order as shade().
@fragment
fn fs_texture(v: VOut) -> @location(0) vec4<f32> {
  return textureSample(src, samp, v.uv) * draw_params.opacity;
}

@fragment
fn fs_texture_edge(v: VOut) -> @location(0) vec4<f32> {
  let d = draw_params;
  let c = textureSample(src, samp, v.uv);
  let dlx = dpdx(v.local);
  let dly = dpdy(v.local);
  let aa2 = vec2(max(length(vec2(dlx.x, dly.x)), 1e-6), max(length(vec2(dlx.y, dly.y)), 1e-6));
  let lo = min(d.box_rect.xy, d.box_rect.zw);
  let hi = max(d.box_rect.xy, d.box_rect.zw);
  let half_size = (hi - lo) * 0.5;
  let q = abs(v.local - (lo + half_size)) - half_size;
  let e = clamp(vec2(0.5) - q / aa2, vec2(0.0), vec2(1.0));
  return c * (d.opacity * e.x * e.y);
}

@fragment
fn fs_solid(v: VOut) -> @location(0) vec4<f32> {
  let d = draw_params;
  var c = d.color;
  if (d.src_kind == 2u) { c = stops[paints[d.paint].stop_off].color; }
  return vec4(c.rgb * c.a, c.a) * d.opacity;
}

@fragment
fn fs_blend(v: VOut) -> @location(0) vec4<f32> {
  let d = draw_params;
  let s = shade(v, false);
  let bd = textureLoad(backdrop, vec2<i32>(v.pos.xy), 0);
  let mode = d.blend;
  if (mode == 35u) { return s + bd * (1.0 - COV); } // adjustment: mix(backdrop, effect, coverage)
  return composite(mode, bd, s);
}

// ------------------------------------------------------------------ generators

struct Gen {
  size: vec2<f32>,
  kind: u32,
  octaves: u32,
  scale: f32,
  evolution: f32,
  contrast: f32,
  angle: f32,
  seed: u32,
  paint_a: u32,
  paint_b: u32,
  seed_hi: u32,
  grain: vec4<u32>,
  perm: array<vec4<u32>, 64>,
};

@group(3) @binding(0) var<uniform> gen: Gen;

struct GOut {
  @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> GOut {
  var p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
  var o: GOut;
  o.pos = vec4(p[i], 0.0, 1.0);
  return o;
}

// Noise kinds over seeded 64-bit hash draws: improved
// Perlin noise with a fixed gradient table over a
// permutation of seeded 64-bit hash draws, and lattice values U(seed, k, pack(i, j)).

var<private> GRAD3: array<vec3<f32>, 16> = array<vec3<f32>, 16>(
  vec3(1.0, 1.0, 0.0), vec3(-1.0, 1.0, 0.0), vec3(1.0, -1.0, 0.0), vec3(-1.0, -1.0, 0.0),
  vec3(1.0, 0.0, 1.0), vec3(-1.0, 0.0, 1.0), vec3(1.0, 0.0, -1.0), vec3(-1.0, 0.0, -1.0),
  vec3(0.0, 1.0, 1.0), vec3(0.0, -1.0, 1.0), vec3(0.0, 1.0, -1.0), vec3(0.0, -1.0, -1.0),
  vec3(1.0, 1.0, 0.0), vec3(0.0, -1.0, 1.0), vec3(-1.0, 1.0, 0.0), vec3(0.0, -1.0, -1.0));

fn gperm(i: i32) -> i32 {
  let k = bitcast<u32>(i) & 255u;
  return i32(gen.perm[k >> 2u][k & 3u]);
}

fn ggrad(h: i32, d: vec3<f32>) -> f32 {
  return dot(GRAD3[u32(gperm(h)) & 15u], d);
}

fn gperlin3(p: vec3<f32>) -> f32 {
  let fl = floor(p);
  let X = i32(fl.x) & 255; let Y = i32(fl.y) & 255; let Z = i32(fl.z) & 255;
  let f = p - fl;
  let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
  let A = gperm(X) + Y; let B = gperm(X + 1) + Y;
  let AA = gperm(A) + Z; let AB = gperm(A + 1) + Z; let BA = gperm(B) + Z; let BB = gperm(B + 1) + Z;
  let x1 = mix(ggrad(AA, f), ggrad(BA, f - vec3(1.0, 0.0, 0.0)), u.x);
  let x2 = mix(ggrad(AB, f - vec3(0.0, 1.0, 0.0)), ggrad(BB, f - vec3(1.0, 1.0, 0.0)), u.x);
  let x3 = mix(ggrad(AA + 1, f - vec3(0.0, 0.0, 1.0)), ggrad(BA + 1, f - vec3(1.0, 0.0, 1.0)), u.x);
  let x4 = mix(ggrad(AB + 1, f - vec3(0.0, 1.0, 1.0)), ggrad(BB + 1, f - vec3(1.0, 1.0, 1.0)), u.x);
  return mix(mix(x1, x2, u.y), mix(x3, x4, u.y), u.z);
}

// Octave k at 2^k with weight 0.5^k, offset by (17.3 k, 5.1 k) and z · (1 + k / 2); normalised.
fn gfbm(x: f32, y: f32, z: f32, octaves: u32) -> f32 {
  var total = 0.0; var amp = 1.0; var norm = 0.0;
  for (var o = 0u; o < max(octaves, 1u) && o < 16u; o = o + 1u) {
    let f = exp2(f32(o));
    total = total + amp * gperlin3(vec3(x * f + 17.3 * f32(o), y * f + 5.1 * f32(o), z * (1.0 + 0.5 * f32(o))));
    norm = norm + amp;
    amp = amp * 0.5;
  }
  return total / norm;
}

// U(seed, k, pack(i, j)).
fn glattice(seed: vec2<u32>, i: i32, j: i32, k: u32) -> f32 {
  return d24_unit(seed, vec2(k, 0u), d24_pack(i, j, 0));
}

fn is_noise_kind(k: u32) -> bool {
  return k == 2u || k == 3u || k == 4u || k == 8u || k == 9u;
}

// The noise kinds' field in [0, 1] at asset position p (1 shows paint, 0 paint2).
fn noise_field(p: vec2<f32>) -> f32 {
  let seed = vec2(gen.seed, gen.seed_hi);
  let s = max(gen.scale, 1e-6);
  let a = radians(gen.angle);
  let c = gen.size * 0.5;
  let d = p - c;
  let r = vec2(d.x * cos(a) + d.y * sin(a), -d.x * sin(a) + d.y * cos(a));
  let con = gen.contrast;
  switch (gen.kind) {
    case 2u, 3u: {                                                                // noise, fractal noise
      let oct = select(gen.octaves, 1u, gen.kind == 2u);
      let n = gfbm(r.x / s, r.y / s, gen.evolution, oct);
      let v = 0.5 + n * select(0.85, 0.65, oct == 1u);
      return clamp(0.5 + (v - 0.5) * con, 0.0, 1.0);
    }
    case 4u: {                                                                    // cells (Worley F1)
      let q = r / s;
      let ci = vec2<i32>(floor(q));
      let e = gen.evolution;
      var best = 9.0;
      for (var dj = -1; dj <= 1; dj = dj + 1) {
        for (var di = -1; di <= 1; di = di + 1) {
          let ii = ci.x + di; let jj = ci.y + dj;
          let h1 = glattice(seed, ii, jj, 1u); let h2 = glattice(seed, ii, jj, 2u);
          let fp = vec2(f32(ii) + 0.5 + 0.4 * sin(6.2831853 * (h1 + e * (0.5 + 0.5 * h2))),
                        f32(jj) + 0.5 + 0.4 * cos(6.2831853 * (h2 + e * (0.5 + 0.5 * h1))));
          best = min(best, length(q - fp));
        }
      }
      return clamp(0.5 + (clamp(best, 0.0, 1.0) - 0.5) * con, 0.0, 1.0);
    }
    case 8u: {                                                                    // film grain: per frame
      let g = max(0.25, s / 100.0);
      let i = i32(floor(p.x / g)); let j = i32(floor(p.y / g));
      let fs = gen.grain.xy;
      let u = (glattice(fs, i, j, 1u) + glattice(fs, i, j, 2u) + glattice(fs, i, j, 3u)) / 3.0;
      return clamp(0.5 + (u - 0.5) * 2.0 * con, 0.0, 1.0);
    }
    default: {                                                                    // light rays
      let diag = length(gen.size);
      let dir = vec2(-sin(a), cos(a));
      let o = c - dir * diag * 0.75;
      let v = p - o;
      let dist = length(v) + 1e-9;
      let cos_t = dot(v, dir) / dist;
      let theta = atan2(v.x * dir.y - v.y * dir.x, dot(v, dir));
      let n = gfbm(theta * diag * 0.75 / s, 3.7, gen.evolution, 3u);
      let rays = pow(clamp(0.5 + n * 1.6, 0.0, 1.0), 2.0);
      let cone = pow(clamp(cos_t, 0.0, 1.0), 6.0);
      let fall = clamp(1.2 - (dist - diag * 0.25) / (diag * 1.3), 0.0, 1.0);
      return clamp(rays * cone * fall * (0.6 + 0.8 * con), 0.0, 1.0);
    }
  }
}

fn rotate(p: vec2<f32>, deg: f32) -> vec2<f32> {
  let r = radians(deg);
  return vec2(p.x * cos(r) - p.y * sin(r), p.x * sin(r) + p.y * cos(r));
}

// Asset position → pattern space: about the centre, turned clockwise by `angle`, scrolled by
// `evolution` periods of `period`.
fn pattern_space(p: vec2<f32>, period: f32) -> vec2<f32> {
  return rotate(p - gen.size * 0.5, -gen.angle) - vec2(gen.evolution * period, 0.0);
}

fn gen_paint(i: u32, p: vec2<f32>, pixel: vec2<u32>) -> vec4<f32> {
  return eval_paint(i, p, pixel);
}

@fragment
fn fs_generator(v: GOut) -> @location(0) vec4<f32> {
  let pixel = vec2<u32>(v.pos.xy);
  let p = v.pos.xy;
  let a = gen_paint(gen.paint_a, p, pixel);
  let b = gen_paint(gen.paint_b, p, pixel);
  if (is_noise_kind(gen.kind)) {
    // paint2 → paint by the field, premultiplied in display sRGB (as 8-bit
    // surfaces mix)
    let pa = vec4(to_space(a.rgb, 1u) * a.a, a.a);
    let pb = vec4(to_space(b.rgb, 1u) * b.a, b.a);
    let m = mix(pb, pa, noise_field(p));
    if (m.a <= 0.0) { return vec4(0.0); }
    return vec4(from_space(m.rgb / m.a, 1u) * m.a, m.a);
  }
  let s = max(gen.scale, 1e-3);
  var t = 0.0;
  switch (gen.kind) {
    case 0u: { t = 0.0; }                                                         // solid
    case 1u: {                                                                    // gradient along angle
      let c = gen.size * 0.5;
      let dir = rotate(vec2(1.0, 0.0), gen.angle);
      let ext = abs(dir.x) * gen.size.x * 0.5 + abs(dir.y) * gen.size.y * 0.5;
      t = clamp(dot(p - c, dir) / max(ext, 1e-3) * 0.5 + 0.5, 0.0, 1.0);
    }
    // The patterns: pattern space has its origin at the
    // asset's centre, turns clockwise by `angle` and scrolls along its x by `evolution` periods.
    case 5u: {                                                                    // checkerboard: `scale` squares, paint at the origin's
      let q = floor(pattern_space(p, 2.0 * s) / s);
      t = f32((i32(q.x) + i32(q.y)) & 1);
    }
    case 6u: {                                                                    // grid: lines from each multiple of `scale`
      let n = clamp(round(s), 2.0, 256.0);
      let w = max(1.0, round(n * 0.04)) / n;
      let f = fract(pattern_space(p, s) / s);
      t = select(1.0, 0.0, min(f.x, f.y) < w);
    }
    case 7u: {                                                                    // stripes: period 2 × `scale`, paint first
      let q = pattern_space(p, 2.0 * s) / (2.0 * s);
      t = select(0.0, 1.0, fract(q.x) >= 0.5);
    }
    default: {}
  }
  t = clamp((t - 0.5) * gen.contrast + 0.5, 0.0, 1.0);
  // t = 0 → paint, t = 1 → paint2
  if (gen.kind == 1u) {
    // the gradient mixes premultiplied sRGB-encoded colours
    let pa = vec4(to_space(a.rgb, 1u) * a.a, a.a);
    let pb = vec4(to_space(b.rgb, 1u) * b.a, b.a);
    let m = mix(pa, pb, t);
    let rgb = select(vec3(0.0), from_space(m.rgb / max(m.a, 1e-6), 1u), m.a > 0.0);
    return vec4(rgb * m.a, m.a);
  }
  let c = mix(a, b, t);
  return vec4(c.rgb * c.a, c.a);
}
