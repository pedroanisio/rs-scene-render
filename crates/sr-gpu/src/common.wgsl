// Shared by the compositor and the vector rasteriser: frame globals, paint
// descriptors, colour conversions and paint evaluation. Each including
// shader declares `globals`, `paints` and `stops` bindings.

struct Globals {
  linear_light: u32,
  seed: u32,
  pad0: u32,
  pad1: u32,
  to_srgb: mat3x3<f32>,
  from_srgb: mat3x3<f32>,
};

struct PaintDesc {
  xform0: vec4<f32>,
  xform1: vec4<f32>,
  p0: vec4<f32>,
  p1: vec4<f32>,
  kind: u32,
  spread: u32,
  space: u32,
  stop_off: u32,
  stop_count: u32,
  dither: u32,
  pad0: u32,
  pad1: u32,
};

struct Stop {
  color: vec4<f32>,
  offset: f32,
  midpoint: f32,
  pad0: f32,
  pad1: f32,
};

// ------------------------------------------------------------------ colour helpers

fn srgb_encode1(v: f32) -> f32 {
  if (v <= 0.0031308) { return v * 12.92; }
  return 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}
fn srgb_decode1(v: f32) -> f32 {
  if (v <= 0.04045) { return v / 12.92; }
  return pow((v + 0.055) / 1.055, 2.4);
}
fn srgb_encode(c: vec3<f32>) -> vec3<f32> { return vec3(srgb_encode1(max(c.r, 0.0)), srgb_encode1(max(c.g, 0.0)), srgb_encode1(max(c.b, 0.0))); }
fn srgb_decode(c: vec3<f32>) -> vec3<f32> { return vec3(srgb_decode1(c.r), srgb_decode1(c.g), srgb_decode1(c.b)); }

// stored working value <-> linear working value
fn to_lin(c: vec3<f32>) -> vec3<f32> {
  if (globals.linear_light == 1u) { return c; }
  return srgb_decode(c);
}
fn from_lin(c: vec3<f32>) -> vec3<f32> {
  if (globals.linear_light == 1u) { return c; }
  return srgb_encode(c);
}

fn cbrt(x: f32) -> f32 { return sign(x) * pow(abs(x), 1.0 / 3.0); }

fn lin_srgb_to_oklab(c: vec3<f32>) -> vec3<f32> {
  let l = 0.4122214708 * c.r + 0.5363325363 * c.g + 0.0514459929 * c.b;
  let m = 0.2119034982 * c.r + 0.6806995451 * c.g + 0.1073969566 * c.b;
  let s = 0.0883024619 * c.r + 0.2817188376 * c.g + 0.6299787005 * c.b;
  let l_ = cbrt(l); let m_ = cbrt(m); let s_ = cbrt(s);
  return vec3(
    0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
    1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
    0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_);
}
fn oklab_to_lin_srgb(c: vec3<f32>) -> vec3<f32> {
  let l_ = c.x + 0.3963377774 * c.y + 0.2158037573 * c.z;
  let m_ = c.x - 0.1055613458 * c.y - 0.0638541728 * c.z;
  let s_ = c.x - 0.0894841775 * c.y - 1.2914855480 * c.z;
  let l = l_ * l_ * l_; let m = m_ * m_ * m_; let s = s_ * s_ * s_;
  return vec3(
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s);
}

// interpolation spaces: 0 linear, 1 srgb, 2 oklab, 3 oklch
fn to_space(c: vec3<f32>, space: u32) -> vec3<f32> {
  let lin = to_lin(c);
  if (space == 0u) { return lin; }
  let s = globals.to_srgb * lin;
  if (space == 1u) { return srgb_encode(s); }
  let lab = lin_srgb_to_oklab(s);
  if (space == 2u) { return lab; }
  let ch = length(lab.yz);
  return vec3(lab.x, ch, atan2(lab.z, lab.y));
}
fn from_space(c: vec3<f32>, space: u32) -> vec3<f32> {
  var lin: vec3<f32>;
  if (space == 0u) { lin = c; }
  else if (space == 1u) { lin = globals.from_srgb * srgb_decode(c); }
  else {
    var lab = c;
    if (space == 3u) { lab = vec3(c.x, c.y * cos(c.z), c.y * sin(c.z)); }
    lin = globals.from_srgb * oklab_to_lin_srgb(lab);
  }
  return from_lin(lin);
}
fn mix_space(a: vec3<f32>, b: vec3<f32>, t: f32, space: u32) -> vec3<f32> {
  if (space == 3u) {
    var dh = b.z - a.z;
    if (dh > 3.14159265) { dh = dh - 6.2831853; }
    if (dh < -3.14159265) { dh = dh + 6.2831853; }
    return vec3(mix(a.x, b.x, t), mix(a.y, b.y, t), a.z + dh * t);
  }
  return mix(a, b, t);
}

fn hash2(p: vec2<u32>, seed: u32) -> f32 {
  var h = p.x * 1664525u + p.y * 1013904223u + seed * 2654435761u;
  h = (h ^ (h >> 16u)) * 2246822519u;
  h = (h ^ (h >> 13u)) * 3266489917u;
  h = h ^ (h >> 16u);
  return f32(h & 16777215u) / 16777216.0;
}

fn luma(c: vec3<f32>) -> f32 { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }

// ------------------------------------------------------------------ paints

fn spread_t(t: f32, spread: u32) -> f32 {
  if (spread == 1u) { return 1.0 - abs(fract(t * 0.5) * 2.0 - 1.0); }
  if (spread == 2u) { return fract(t); }
  return clamp(t, 0.0, 1.0);
}

fn eval_stops(pd: PaintDesc, t: f32) -> vec4<f32> {
  let n = pd.stop_count;
  if (n == 0u) { return vec4(0.0); }
  let first = stops[pd.stop_off];
  if (t <= first.offset || n == 1u) { return first.color; }
  let last = stops[pd.stop_off + n - 1u];
  if (t >= last.offset) { return last.color; }
  for (var i = 1u; i < n; i = i + 1u) {
    let b = stops[pd.stop_off + i];
    if (t <= b.offset) {
      let a = stops[pd.stop_off + i - 1u];
      var u = select(0.0, (t - a.offset) / (b.offset - a.offset), b.offset > a.offset);
      let mid = clamp(a.midpoint, 0.001, 0.999);
      if (abs(mid - 0.5) > 0.0001) { u = pow(u, log(0.5) / log(mid)); }
      let rgb = from_space(mix_space(to_space(a.color.rgb, pd.space), to_space(b.color.rgb, pd.space), u, pd.space), pd.space);
      return vec4(rgb, mix(a.color.a, b.color.a, u));
    }
  }
  return last.color;
}

// straight colour of paint `i` at local position `p`
fn eval_paint(i: u32, p: vec2<f32>, pixel: vec2<u32>) -> vec4<f32> {
  let pd = paints[i];
  let g = vec2(pd.xform0.x * p.x + pd.xform0.z * p.y + pd.xform1.x, pd.xform0.y * p.x + pd.xform0.w * p.y + pd.xform1.y);
  var t = 0.0;
  if (pd.kind == 0u) {
    return stops[pd.stop_off].color;
  } else if (pd.kind == 1u) {
    let a = pd.p0.xy; let b = pd.p0.zw;
    let d = b - a;
    t = dot(g - a, d) / max(dot(d, d), 1e-12);
  } else if (pd.kind == 2u) {
    let c = pd.p0.xy; let r = pd.p0.z; let aspect = pd.p0.w;
    let f = pd.p1.xy; let fr = pd.p1.z;
    let q = vec2(g.x, c.y + (g.y - c.y) / aspect);
    let fq = vec2(f.x, c.y + (f.y - c.y) / aspect);
    let dir = q - fq;
    let len = length(dir);
    if (len < 1e-9) { t = 0.0; } else {
      let dn = dir / len;
      // distance from the focal point to the circle along dn
      let oc = fq - c;
      let bq = dot(oc, dn);
      let cq = dot(oc, oc) - r * r;
      let s = -bq + sqrt(max(bq * bq - cq, 0.0));
      t = (len - fr) / max(s - fr, 1e-9);
    }
  } else if (pd.kind == 3u) {
    let d = g - pd.p0.xy;
    var ang = degrees(atan2(d.x, -d.y)) - pd.p0.z;
    t = fract(ang / 360.0);
  } else if (pd.kind == 4u) {
    let rows = u32(pd.p0.x); let cols = u32(pd.p0.y);
    let gx = clamp(g.x, 0.0, 1.0) * f32(cols - 1u);
    let gy = clamp(g.y, 0.0, 1.0) * f32(rows - 1u);
    let c0 = min(u32(floor(gx)), cols - 2u); let r0 = min(u32(floor(gy)), rows - 2u);
    let fx = smoothstep(0.0, 1.0, gx - f32(c0)); let fy = smoothstep(0.0, 1.0, gy - f32(r0));
    let s00 = stops[pd.stop_off + r0 * cols + c0].color;
    let s01 = stops[pd.stop_off + r0 * cols + c0 + 1u].color;
    let s10 = stops[pd.stop_off + (r0 + 1u) * cols + c0].color;
    let s11 = stops[pd.stop_off + (r0 + 1u) * cols + c0 + 1u].color;
    let top = mix_space(to_space(s00.rgb, pd.space), to_space(s01.rgb, pd.space), fx, pd.space);
    let bot = mix_space(to_space(s10.rgb, pd.space), to_space(s11.rgb, pd.space), fx, pd.space);
    let rgb = from_space(mix_space(top, bot, fy, pd.space), pd.space);
    return vec4(rgb, mix(mix(s00.a, s01.a, fx), mix(s10.a, s11.a, fx), fy));
  }
  var c = eval_stops(pd, spread_t(t, pd.spread));
  if (pd.dither == 1u) {
    let n = (hash2(pixel, 7u) - 0.5) / 255.0;
    c = vec4(from_lin(to_lin(c.rgb) + vec3(n) * select(1.0, 0.25, globals.linear_light == 1u)), c.a);
  }
  return c;
}

