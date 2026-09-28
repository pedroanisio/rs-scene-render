// Resolves the multisampled depth (sample 0) into a float texture.
@group(0) @binding(0) var ms_depth: texture_depth_multisampled_2d;

struct PostOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_post(@builtin(vertex_index) vi: u32) -> PostOut {
    var o: PostOut;
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    o.pos = vec4(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2(p.x, 1.0 - p.y);
    return o;
}


@fragment
fn fs_depth_resolve(i: PostOut) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(ms_depth));
    let q = vec2<i32>(clamp(i.uv * dims, vec2(0.0), dims - 1.0));
    var d = 0.0;
    for (var s = 0; s < 4; s++) { d = max(d, textureLoad(ms_depth, q, s)); }
    return vec4(d, 0.0, 0.0, 1.0);
}
