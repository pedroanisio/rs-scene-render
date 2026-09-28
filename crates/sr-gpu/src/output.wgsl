// Working-space frame → encoder input bytes. One invocation writes one
// 32-bit word of the output buffer, so any byte layout is race-free.

struct Out {
  to_out: mat3x3<f32>,
  fsize: vec2<u32>,       // frame size
  osize: vec2<u32>,       // output size
  offset: vec2<f32>,      // letterbox offset (output pixels)
  scale: f32,             // output pixels per frame pixel
  format: u32,            // 0 NV12, 1 P010, 2 RGBA8, 3 RGBA16, 4 GBRAPF32
  transfer: u32,          // output transfer id
  full_range: u32,
  linear_light: u32,
  store_transfer: u32,
  keep_alpha: u32,
  kr: f32,
  kb: f32,
  words: u32,
  seed: u32,
  pad: u32,
};

@group(0) @binding(0) var<uniform> o: Out;
@group(0) @binding(1) var frame: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<storage, read_write> dst: array<u32>;

fn hash(p: vec2<u32>, s: u32) -> f32 {
  var h = p.x * 1664525u + p.y * 1013904223u + s * 2654435761u;
  h = (h ^ (h >> 16u)) * 2246822519u;
  h = (h ^ (h >> 13u)) * 3266489917u;
  h = h ^ (h >> 16u);
  return f32(h & 16777215u) / 16777216.0;
}

// straight, output-encoded RGB and alpha of output pixel (x, y); linear premultiplied for float output
fn px(x: u32, y: u32) -> vec4<f32> {
  let q = (vec2<f32>(f32(x), f32(y)) + 0.5 - o.offset) / o.scale;
  var c = vec4(0.0);
  if (all(q >= vec2(0.0)) && all(q <= vec2<f32>(o.fsize))) {
    c = textureSampleLevel(frame, samp, q / vec2<f32>(o.fsize), 0.0);
  }
  var a = clamp(c.a, 0.0, 1.0);
  var rgb = select(vec3(0.0), c.rgb / a, a > 0.0);
  if (o.linear_light == 0u) { rgb = tf_decode3(o.store_transfer, rgb); }
  rgb = o.to_out * rgb;
  if (o.format == 4u) {
    // scene-linear, premultiplied
    if (o.keep_alpha == 0u) { return vec4(rgb * a, 1.0); }
    return vec4(rgb * a, a);
  }
  if (o.keep_alpha == 0u) {
    // opaque delivery: composite over black
    rgb = rgb * a;
    a = 1.0;
  }
  return vec4(clamp(tf_encode3(o.transfer, max(rgb, vec3(0.0))), vec3(0.0), vec3(1.0)), a);
}

fn yuv(c: vec3<f32>) -> vec3<f32> {
  let kg = 1.0 - o.kr - o.kb;
  let y = o.kr * c.r + kg * c.g + o.kb * c.b;
  return vec3(y, (c.b - y) / (2.0 * (1.0 - o.kb)), (c.r - y) / (2.0 * (1.0 - o.kr)));
}

fn chroma(cx: u32, cy: u32) -> vec3<f32> {
  var s = vec3(0.0);
  var n = 0.0;
  for (var k = 0u; k < 4u; k = k + 1u) {
    let x = cx * 2u + (k & 1u);
    let y = cy * 2u + (k >> 1u);
    if (x < o.osize.x && y < o.osize.y) {
      s = s + px(x, y).rgb;
      n = n + 1.0;
    }
  }
  return yuv(s / n);
}

// quantised luma or chroma code at `bits` (8 or 10)
fn code(v: f32, luma: bool, bits: u32, dither: f32) -> u32 {
  let m = f32(1u << (bits - 8u));
  var x: f32;
  if (o.full_range == 1u) {
    x = select(v + 0.5, v, luma) * (255.0 * m);
  } else if (luma) {
    x = (16.0 + 219.0 * v) * m;
  } else {
    x = (128.0 + 224.0 * v) * m;
  }
  return u32(clamp(floor(x + 0.5 + dither), 0.0, 255.0 * m + (m - 1.0)));
}

// Y or C sample `i` of a 4:2:0 frame (NV12/P010 order)
fn yuv_sample(i: u32, bits: u32) -> u32 {
  let w = o.osize.x; let h = o.osize.y;
  let d = hash(vec2(i, 7u), o.seed) - hash(vec2(i, 11u), o.seed);
  if (i < w * h) {
    let p = px(i % w, i / w);
    return code(yuv(p.rgb).x, true, bits, d * select(0.0, 1.0, bits == 8u));
  }
  let j = i - w * h;
  let cw = (w + 1u) / 2u;
  let pair = j / 2u;
  let c = chroma(pair % cw, pair / cw);
  let v = select(c.z, c.y, (j & 1u) == 0u);
  return code(v, false, bits, d * select(0.0, 1.0, bits == 8u));
}

@compute @workgroup_size(64)
fn cs_pack(@builtin(global_invocation_id) gid: vec3<u32>) {
  let wi = gid.x + gid.y * 65535u * 64u;
  if (wi >= o.words) { return; }
  let w = o.osize.x; let h = o.osize.y;
  let n = w * h;
  let total_y = n;
  let cw = (w + 1u) / 2u; let ch = (h + 1u) / 2u;
  var word = 0u;
  switch (o.format) {
    case 0u: {
      let samples = total_y + cw * ch * 2u;
      for (var k = 0u; k < 4u; k = k + 1u) {
        let s = wi * 4u + k;
        if (s < samples) { word = word | (yuv_sample(s, 8u) << (8u * k)); }
      }
    }
    case 1u: {
      let samples = total_y + cw * ch * 2u;
      for (var k = 0u; k < 2u; k = k + 1u) {
        let s = wi * 2u + k;
        if (s < samples) { word = word | ((yuv_sample(s, 10u) << 6u) << (16u * k)); }
      }
    }
    case 2u: {
      let p = px(wi % w, wi / w);
      let d = (hash(vec2(wi, 3u), o.seed) - hash(vec2(wi, 5u), o.seed)) / 255.0;
      word = pack4x8unorm(clamp(vec4(p.rgb + vec3(d), p.a), vec4(0.0), vec4(1.0)));
    }
    case 3u: {
      let pi = wi / 2u;
      let p = px(pi % w, pi / w);
      let pair = select(p.ba, p.rg, (wi & 1u) == 0u);
      let q = vec2<u32>(floor(clamp(pair, vec2(0.0), vec2(1.0)) * 65535.0 + 0.5));
      word = q.x | (q.y << 16u);
    }
    default: {
      let plane = wi / n;
      let pi = wi % n;
      let p = px(pi % w, pi / w);
      var v = p.a;
      if (plane == 0u) { v = p.g; } else if (plane == 1u) { v = p.b; } else if (plane == 2u) { v = p.r; }
      word = bitcast<u32>(v);
    }
  }
  dst[wi] = word;
}
