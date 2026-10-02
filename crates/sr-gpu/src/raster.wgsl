// Fine stage of the vector rasteriser: one 16×16 workgroup per tile, one
// invocation per pixel. Each tile walks its command list in order,
// computing exact area coverage from edge pieces and per-row backdrops
// (see sr-vector's `tile` module) and compositing paints through a small
// layer stack for group opacity, masks and mattes.

struct RCmd {
  kind: u32,
  piece_off: u32,
  piece_count: u32,
  backdrop: u32,
  backdrop_val: f32,
  paint: u32,
  flags: u32,
  param: f32,
};

struct RParams {
  tiles_x: u32,
  tiles_y: u32,
  width: u32,
  height: u32,
  active_count: u32,
  pad0: u32,
  pad1: u32,
  pad2: u32,
};

@group(0) @binding(0) var<storage, read> rcmds: array<RCmd>;
@group(0) @binding(1) var<storage, read> ranges: array<vec2<u32>>;
@group(0) @binding(2) var<storage, read> pieces: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> backdrops: array<f32>;
@group(0) @binding(4) var<storage, read> paints: array<PaintDesc>;
@group(0) @binding(5) var<storage, read> stops: array<Stop>;
@group(0) @binding(6) var<uniform> globals: Globals;
@group(0) @binding(7) var<uniform> rp: RParams;
@group(0) @binding(8) var out_tex: texture_storage_2d<rgba16float, write>;

@group(0) @binding(9) var<storage, read> active_tiles: array<u32>;

override SPARSE_TILES: bool = false;

// Selected only when the complete command stream consists of solid fills.
override SOLID_FILLS: bool = false;

fn g_clamp(x: f32) -> f32 {
  if (x <= 0.0) { return 0.0; }
  if (x < 1.0) { return x * x * 0.5; }
  return x - 0.5;
}

fn piece_cov(px: f32, py: f32, pc: vec4<f32>) -> f32 {
  let ya = clamp(pc.y - py, 0.0, 1.0);
  let yb = clamp(pc.w - py, 0.0, 1.0);
  let dy = yb - ya;
  if (dy == 0.0) { return 0.0; }
  let inv = 1.0 / (pc.w - pc.y);
  let xa = pc.x + (pc.z - pc.x) * ((ya + py - pc.y) * inv) - px;
  let xb = pc.x + (pc.z - pc.x) * ((yb + py - pc.y) * inv) - px;
  var m: f32;
  if (abs(xb - xa) < 1e-4) { m = clamp((xa + xb) * 0.5, 0.0, 1.0); }
  else { m = (g_clamp(xb) - g_clamp(xa)) / (xb - xa); }
  return dy * (1.0 - m);
}

fn cmd_cov(c: RCmd, px: u32, py: u32) -> f32 {
  var area = c.backdrop_val;
  if (c.backdrop != 0xffffffffu) { area = backdrops[c.backdrop + py]; }
  for (var k = 0u; k < c.piece_count; k = k + 1u) {
    area = area + piece_cov(f32(px), f32(py), pieces[c.piece_off + k]);
  }
  let a = abs(area);
  var cov = min(a, 1.0);
  if ((c.flags & 1u) == 1u) { cov = abs(a - 2.0 * round(a * 0.5)); }
  if ((c.flags & 2u) == 2u) { cov = 1.0 - cov; }
  return cov;
}

fn mask_op(m: f32, v: f32, op: u32) -> f32 {
  switch op {
    case 1u: { return m * (1.0 - v); }
    case 2u: { return m * v; }
    case 3u: { return m + v - 2.0 * m * v; }
    case 4u: { return max(m, v); }
    case 5u: { return min(m, v); }
    default: { return m + v - m * v; }
  }
}

@compute @workgroup_size(16, 16)
fn raster_main(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) li: vec3<u32>) {
  var tile = wg.xy;
  if (SPARSE_TILES) {
    let index = wg.x + wg.y * 65535u;
    if (index >= rp.active_count) { return; }
    let tile_index = active_tiles[index];
    tile = vec2(tile_index % rp.tiles_x, tile_index / rp.tiles_x);
  }
  let x = tile.x * 16u + li.x;
  let y = tile.y * 16u + li.y;
  if (x >= rp.width || y >= rp.height) { return; }
  let r = ranges[tile.y * rp.tiles_x + tile.x];
  var acc = vec4<f32>(0.0);
  if (r.y == 0u) {
    textureStore(out_tex, vec2<i32>(i32(x), i32(y)), acc);
    return;
  }
  if (SOLID_FILLS) {
    for (var i = r.x; i < r.x + r.y; i = i + 1u) {
      let c = rcmds[i];
      let cov = cmd_cov(c, li.x, li.y) * c.param;
      if (cov > 0.0) {
        let s = stops[paints[c.paint].stop_off].color;
        let a = s.a * cov;
        acc = vec4<f32>(s.rgb * a, a) + acc * (1.0 - a);
      }
    }
    textureStore(out_tex, vec2<i32>(i32(x), i32(y)), acc);
    return;
  }
  var m = 1.0;
  var st: array<vec4<f32>, 8>;
  var sm: array<f32, 8>;
  var d = 0u;
  for (var i = r.x; i < r.x + r.y; i = i + 1u) {
    let c = rcmds[i];
    switch c.kind {
      case 0u: {
        let cov = cmd_cov(c, li.x, li.y) * c.param;
        if (cov > 0.0) {
          let s = eval_paint(c.paint, vec2<f32>(f32(x) + 0.5, f32(y) + 0.5), vec2<u32>(x, y));
          let a = s.a * cov;
          acc = vec4<f32>(s.rgb * a, a) + acc * (1.0 - a);
        }
      }
      case 1u: {
        if (d < 8u) { st[d] = acc; sm[d] = m; }
        d = d + 1u;
        acc = vec4<f32>(0.0);
        m = c.param;
      }
      case 2u: {
        m = mask_op(m, cmd_cov(c, li.x, li.y) * c.param, (c.flags >> 4u) & 15u);
      }
      case 3u: {
        let layer = acc * (m * c.param);
        d = max(d, 1u) - 1u;
        acc = layer + st[min(d, 7u)] * (1.0 - layer.a);
        m = sm[min(d, 7u)];
      }
      case 4u: {
        let mt = acc;
        let lum = dot(mt.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        var mv = mt.a;
        let mode = (c.flags >> 4u) & 15u;
        if (mode == 1u) { mv = 1.0 - mt.a; }
        if (mode == 2u) { mv = lum; }
        if (mode == 3u) { mv = 1.0 - lum; }
        d = max(d, 1u) - 1u;
        let content = st[min(d, 7u)];
        let layer = content * (sm[min(d, 7u)] * mv * c.param);
        d = max(d, 1u) - 1u;
        acc = layer + st[min(d, 7u)] * (1.0 - layer.a);
        m = sm[min(d, 7u)];
      }
      default: {}
    }
  }
  textureStore(out_tex, vec2<i32>(i32(x), i32(y)), acc);
}
