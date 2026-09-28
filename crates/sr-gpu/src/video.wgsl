// Video planes → premultiplied working-space RGBA. TEX_T and LOAD_SCALE are
// substituted per sample type (unorm planes or 16-bit uint planes).

struct Conv {
  to_working: mat3x3<f32>,
  size_out: vec2<u32>,
  coded: vec2<u32>,
  chroma_shift: vec2<u32>,
  packing: u32,       // 0 planar YUV, 1 packed RGBA
  rotation: u32,      // clockwise quarter turns
  transfer: u32,
  alpha_mode: u32,    // 0 straight, 1 premultiplied, 2 ignore
  full_range: u32,
  deep: u32,          // 16-bit samples
  linear_light: u32,
  store_transfer: u32,
  kr: f32,
  kb: f32,
  pad0: u32,
  pad1: u32,
};

@group(0) @binding(0) var<uniform> conv: Conv;
@group(0) @binding(1) var p0: texture_2d<TEX_T>;
@group(0) @binding(2) var p1: texture_2d<TEX_T>;
@group(0) @binding(3) var p2: texture_2d<TEX_T>;

struct VOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> VOut {
  var p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
  var o: VOut;
  o.pos = vec4(p[i], 0.0, 1.0);
  return o;
}

fn ld(t: texture_2d<TEX_T>, c: vec2<i32>) -> vec4<f32> {
  let d = vec2<i32>(textureDimensions(t)) - vec2(1);
  return vec4<f32>(textureLoad(t, clamp(c, vec2(0), d), 0)) * LOAD_SCALE;
}

// bilinear chroma with centred siting
fn chroma(t: texture_2d<TEX_T>, c: vec2<i32>) -> f32 {
  let s = vec2<f32>(f32(1u << conv.chroma_shift.x), f32(1u << conv.chroma_shift.y));
  let q = (vec2<f32>(c) + 0.5) / s - 0.5;
  let i = vec2<i32>(floor(q));
  let f = q - floor(q);
  let a = mix(ld(t, i).r, ld(t, i + vec2(1, 0)).r, f.x);
  let b = mix(ld(t, i + vec2(0, 1)).r, ld(t, i + vec2(1, 1)).r, f.x);
  return mix(a, b, f.y);
}

@fragment
fn fs_convert(v: VOut) -> @location(0) vec4<f32> {
  let d = vec2<i32>(v.pos.xy);
  let w = i32(conv.coded.x); let h = i32(conv.coded.y);
  var c: vec2<i32>;
  switch (conv.rotation) {
    case 1u: { c = vec2(d.y, h - 1 - d.x); }
    case 2u: { c = vec2(w - 1 - d.x, h - 1 - d.y); }
    case 3u: { c = vec2(w - 1 - d.y, d.x); }
    default: { c = d; }
  }
  var rgb: vec3<f32>;
  var a = 1.0;
  if (conv.packing == 0u) {
    var y = ld(p0, c).r;
    var cb = chroma(p1, c);
    var cr = chroma(p2, c);
    if (conv.full_range == 0u) {
      if (conv.deep == 1u) {
        y = (y * 65535.0 - 4096.0) / 56064.0;
        cb = (cb * 65535.0 - 32768.0) / 57344.0;
        cr = (cr * 65535.0 - 32768.0) / 57344.0;
      } else {
        y = (y * 255.0 - 16.0) / 219.0;
        cb = (cb * 255.0 - 128.0) / 224.0;
        cr = (cr * 255.0 - 128.0) / 224.0;
      }
    } else {
      cb = cb - select(128.0 / 255.0, 32768.0 / 65535.0, conv.deep == 1u);
      cr = cr - select(128.0 / 255.0, 32768.0 / 65535.0, conv.deep == 1u);
    }
    let kg = 1.0 - conv.kr - conv.kb;
    let r = y + 2.0 * (1.0 - conv.kr) * cr;
    let b = y + 2.0 * (1.0 - conv.kb) * cb;
    let g = (y - conv.kr * r - conv.kb * b) / kg;
    rgb = vec3(r, g, b);
  } else {
    let p = ld(p0, c);
    rgb = p.rgb;
    if (conv.alpha_mode != 2u) { a = p.a; }
    if (conv.alpha_mode == 1u && a > 0.0) { rgb = rgb / a; }
  }
  let lin = conv.to_working * tf_decode3(conv.transfer, rgb);
  var stored = lin;
  if (conv.linear_light == 0u) { stored = tf_encode3(conv.store_transfer, max(lin, vec3(0.0))); }
  return vec4(stored * a, a);
}
