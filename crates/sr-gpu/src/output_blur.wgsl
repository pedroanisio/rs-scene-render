// The blurred cover behind a fit-blur frame: the frame averaged over 8 × 8 blocks, then a separable
// Gaussian (σ = 3 texels of the reduced image, 13 taps) across and down.

struct Blur {
  dir: vec2<i32>,   // (0, 0) downsample; (1, 0) across; (0, 1) down
  pad: vec2<i32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> b: Blur;

@compute @workgroup_size(8, 8)
fn cs_blur(@builtin(global_invocation_id) id: vec3<u32>) {
  let size = textureDimensions(dst);
  if (id.x >= size.x || id.y >= size.y) { return; }
  let p = vec2<i32>(id.xy);
  let smax = vec2<i32>(textureDimensions(src)) - 1;
  var sum = vec4(0.0);
  var wsum = 0.0;
  if (all(b.dir == vec2(0))) {
    for (var j = 0; j < 8; j = j + 1) {
      for (var i = 0; i < 8; i = i + 1) {
        sum = sum + textureLoad(src, clamp(p * 8 + vec2(i, j), vec2(0), smax), 0);
      }
    }
    textureStore(dst, p, sum / 64.0);
    return;
  }
  for (var k = -6; k <= 6; k = k + 1) {
    let w = exp(-f32(k * k) / 18.0);
    sum = sum + textureLoad(src, clamp(p + b.dir * k, vec2(0), smax), 0) * w;
    wsum = wsum + w;
  }
  textureStore(dst, p, sum / wsum);
}
