// Mean colour of each cell of a coarse grid over a frame (flash analysis).

@group(0) @binding(0) var src: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u)) * 2.0 - 1.0;
    return vec4(p, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(src));
    let cell = vec2<i32>(pos.xy);
    // 48 × 27 grid; each cell samples an 8 × 8 lattice of source pixels
    let size = dims / vec2(48.0, 27.0);
    var acc = vec4(0.0);
    for (var y = 0; y < 8; y++) {
        for (var x = 0; x < 8; x++) {
            let p = (vec2<f32>(cell) + (vec2(f32(x), f32(y)) + 0.5) / 8.0) * size;
            acc += textureLoad(src, clamp(vec2<i32>(p), vec2(0), vec2<i32>(dims) - 1), 0);
        }
    }
    return acc / 64.0;
}
