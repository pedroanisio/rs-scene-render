// Original per-pixel aperture calculation, retained independently for exact GPU regression tests.
@fragment
fn fs_dof_reference(i: PostOut) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(post_src));
    var uv = i.uv;
    if (pfr.post.x != 0.0) {
        let asp = vec2(dims.x / dims.y, 1.0);
        let p = (uv - 0.5) * asp;
        uv = 0.5 + p * (1.0 + pfr.post.x * dot(p, p)) / asp;
    }
    let exposure = pfr.params.x;
    let encode = pfr.lens.y > 0.5;
    let dq = vec2<i32>(clamp(uv * dims, vec2(0.0), dims - 1.0));
    let d0 = textureLoad(post_depth, dq, 0).r;
    let r0 = select(0.0, coc(d0), pfr.post.y > 0.5 && d0 > 0.0);
    var center = textureSampleLevel(post_src, post_smp, uv, 0.0);
    // how far any blur around this pixel reaches (the dilated tile maximum)
    let tdims = vec2<i32>(textureDimensions(post_tiles));
    let search = max(textureLoad(post_tiles, min(dq / DOF_TILE, tdims - 1), 0).r, r0);
    if (pfr.post.y < 0.5 || search < 0.5) { return finish(center, exposure, encode); }
    // scatter-as-gather: a sample contributes when its own CoC reaches this pixel
    var acc = center;
    var wsum = 1.0;
    let rmax = search;
    let blades = pfr.dof.w;
    let rings = clamp(i32(ceil(search / 3.0)), 1, 4);
    for (var ring = 1; ring <= rings; ring++) {
        let rr = f32(ring) / f32(rings) * rmax;
        let cnt = ring * 8;
        for (var k = 0; k < cnt; k++) {
            let a = f32(k) / f32(cnt) * 2.0 * PI;
            var sc = 1.0;
            if (blades >= 3.0) {
                let seg = 2.0 * PI / blades;
                let local = a - seg * floor(a / seg) - seg * 0.5;
                sc = cos(seg * 0.5) / cos(local);
            }
            let o = vec2(cos(a), sin(a)) * rr * sc;
            let suv = uv + o / dims;
            let q = vec2<i32>(clamp(suv * dims, vec2(0.0), dims - 1.0));
            let ds = textureLoad(post_depth, q, 0).r;
            let rs = select(0.0, coc(ds), ds > 0.0);
            // background samples only spread as far as the centre's own blur
            let reach = select(min(rs, r0), rs, ds >= d0);
            let w = smoothstep(rr - 1.0, rr + 1.0, reach);
            acc += textureSampleLevel(post_src, post_smp, suv, 0.0) * w;
            wsum += w;
        }
    }
    return finish(acc / wsum, exposure, encode);
}

// Test-only readback of the optimization predicate on a full-frame plane.
@fragment
fn fs_uniform_coc_classification(i: PostOut) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(post_depth));
    let q = vec2<i32>(clamp(i.uv * dims, vec2(0.0), dims - 1.0));
    let d = textureLoad(post_depth, q, 0);
    let r0 = select(0.0, sample_coc(d), d.r > 0.0);
    let tdims = vec2<i32>(textureDimensions(post_tiles));
    let tile = textureLoad(post_tiles, min(q / DOF_TILE, tdims - 1), 0);
    return vec4(select(0.0, 1.0, pfr.fx.w > 0.5 && r0 == max(tile.r, r0) && tile.g == max(tile.r, r0)), 0.0, 0.0, 1.0);
}
