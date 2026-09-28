// ---------------------------------------------------------------- post: depth resolve, mips, DOF

@group(0) @binding(0) var<uniform> pfr: Frame;
@group(0) @binding(1) var post_src: texture_2d<f32>;
@group(0) @binding(2) var post_depth: texture_2d<f32>;
@group(0) @binding(3) var post_smp: sampler;
// dilated per-tile maximum circle of confusion (depth of field)
@group(0) @binding(4) var post_tiles: texture_2d<f32>;

const DOF_TILE: i32 = 16;

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
fn fs_blit(i: PostOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(post_src, post_smp, i.uv, 0.0);
}

// Straight "over": src (3D result) over the compositor's backdrop in aux.
@fragment
fn fs_under(i: PostOut) -> @location(0) vec4<f32> {
    let a = textureSampleLevel(post_src, post_smp, i.uv, 0.0);
    let b = textureLoad(post_depth, vec2<i32>(i.pos.xy), 0);
    return a + b * (1.0 - a.a);
}

fn lin_depth(d: f32) -> f32 {
    // reverse-Z perspective: d = n(f − z)/(z(f − n))  ⇒  z = n f / (d (f − n) + n)
    let n = pfr.post.z;
    let f = pfr.post.w;
    return n * f / (d * (f - n) + n);
}

fn coc(d: f32) -> f32 {
    let z = lin_depth(d);
    return min(pfr.dof.x * abs(1.0 / pfr.dof.y - 1.0 / max(z, 1e-3)), pfr.dof.z);
}

fn srgb_encode(v: vec3<f32>) -> vec3<f32> {
    let c = max(v, vec3(0.0));
    return select(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3(0.0031308));
}

// Exposure, then (for encoded working spaces) the sRGB curve on straight colour, re-premultiplied.
fn finish(c: vec4<f32>, exposure: f32, encode: bool) -> vec4<f32> {
    var rgb = c.rgb * exposure;
    if (encode && c.a > 0.0) {
        rgb = srgb_encode(rgb / c.a) * c.a;
    }
    return vec4(rgb, c.a);
}

// Depth of field (gather with polygonal blades), lens distortion and exposure.
@fragment
fn fs_dof(i: PostOut) -> @location(0) vec4<f32> {
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


// Maximum circle of confusion of each 16 × 16 tile (one output pixel per tile).
@fragment
fn fs_tile_max(i: PostOut) -> @location(0) vec4<f32> {
    let t = vec2<i32>(i.pos.xy);
    let dims = vec2<i32>(textureDimensions(post_depth));
    var m = 0.0;
    for (var y = 0; y < DOF_TILE; y++) {
        for (var x = 0; x < DOF_TILE; x++) {
            let d = textureLoad(post_depth, min(t * DOF_TILE + vec2(x, y), dims - 1), 0).r;
            if (d > 0.0) { m = max(m, coc(d)); }
        }
    }
    return vec4(m, 0.0, 0.0, 1.0);
}

// Spreads each tile's maximum over the tiles its blur can reach.
@fragment
fn fs_tile_dilate(i: PostOut) -> @location(0) vec4<f32> {
    let t = vec2<i32>(i.pos.xy);
    let dims = vec2<i32>(textureDimensions(post_depth));
    let reach = i32(ceil(pfr.dof.z / f32(DOF_TILE)));
    var m = 0.0;
    for (var y = -reach; y <= reach; y++) {
        for (var x = -reach; x <= reach; x++) {
            m = max(m, textureLoad(post_depth, clamp(t + vec2(x, y), vec2(0), dims - 1), 0).r);
        }
    }
    return vec4(m, 0.0, 0.0, 1.0);
}
