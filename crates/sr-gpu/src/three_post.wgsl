// ---------------------------------------------------------------- post: depth resolve, mips, DOF

@group(0) @binding(0) var<uniform> pfr: Frame;
@group(0) @binding(1) var post_src: texture_2d<f32>;
@group(0) @binding(2) var post_depth: texture_2d<f32>;
@group(0) @binding(3) var post_smp: sampler;
// dilated per-tile maximum circle of confusion (depth of field)
@group(0) @binding(4) var post_tiles: texture_2d<f32>;
@group(0) @binding(5) var<uniform> aperture: array<vec4<f32>, 128>;

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

// The paired 32-bit depth target retains the original CoC without rounding.
fn sample_coc(d: vec4<f32>) -> f32 {
    if (pfr.fx.w > 0.5) { return d.g; }
    return coc(d.r);
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

// The second tile channel holds a proved common radius, or -1 when nonuniform.
fn uniform_coc(tile: vec4<f32>, center: f32, search: f32) -> bool {
    return pfr.fx.w > 0.5 && center == search && tile.g == search;
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
    let depth0 = textureLoad(post_depth, dq, 0);
    let d0 = depth0.r;
    let r0 = select(0.0, sample_coc(depth0), pfr.post.y > 0.5 && d0 > 0.0);
    var center = textureSampleLevel(post_src, post_smp, uv, 0.0);
    // how far any blur around this pixel reaches (the dilated tile maximum)
    let tdims = vec2<i32>(textureDimensions(post_tiles));
    let tile = textureLoad(post_tiles, min(dq / DOF_TILE, tdims - 1), 0);
    let search = max(tile.r, r0);
    let same_reach = uniform_coc(tile, r0, search);
    if (pfr.post.y < 0.5 || search < 0.5) { return finish(center, exposure, encode); }
    // scatter-as-gather: a sample contributes when its own CoC reaches this pixel
    var acc = center;
    var wsum = 1.0;
    let rmax = search;
    let rings = clamp(i32(ceil(search / 3.0)), 1, 4);
    for (var ring = 1; ring <= rings; ring++) {
        let rr = f32(ring) / f32(rings) * rmax;
        let cnt = ring * 8;
        let ring_weight = smoothstep(rr - 1.0, rr + 1.0, r0);
        for (var k = 0; k < cnt; k++) {
            let tap = aperture[(ring - 1) * 32 + k];
            let o = tap.xy * rr * tap.z;
            let suv = uv + o / dims;
            var w = ring_weight;
            if (!same_reach) {
                let q = vec2<i32>(clamp(suv * dims, vec2(0.0), dims - 1.0));
                let depth_s = textureLoad(post_depth, q, 0);
                let ds = depth_s.r;
                let rs = select(0.0, sample_coc(depth_s), ds > 0.0);
                // Background samples only spread as far as the centre's own blur.
                let reach = select(min(rs, r0), rs, ds >= d0);
                w = smoothstep(rr - 1.0, rr + 1.0, reach);
            }
            if (w != 0.0) {
                acc += textureSampleLevel(post_src, post_smp, suv, 0.0) * w;
            }
            wsum += w;
        }
    }
    return finish(acc / wsum, exposure, encode);
}


// Maximum and minimum circle of confusion of each 16 x 16 tile.
@fragment
fn fs_tile_max(i: PostOut) -> @location(0) vec4<f32> {
    let t = vec2<i32>(i.pos.xy);
    let dims = vec2<i32>(textureDimensions(post_depth));
    var m = 0.0;
    var lo = pfr.dof.z;
    for (var y = 0; y < DOF_TILE; y++) {
        for (var x = 0; x < DOF_TILE; x++) {
            let d = textureLoad(post_depth, min(t * DOF_TILE + vec2(x, y), dims - 1), 0);
            if (d.r > 0.0) {
                let radius = sample_coc(d);
                m = max(m, radius);
                lo = min(lo, radius);
            } else {
                lo = 0.0;
            }
        }
    }
    return vec4(m, lo, 0.0, 1.0);
}

// Preserve the original maximum neighborhood. Prove uniformity over a larger
// region so rounding at aperture and pixel boundaries cannot admit a false match.
@fragment
fn fs_tile_dilate(i: PostOut) -> @location(0) vec4<f32> {
    let t = vec2<i32>(i.pos.xy);
    let dims = vec2<i32>(textureDimensions(post_depth));
    let reach = i32(ceil(pfr.dof.z / f32(DOF_TILE)));
    let min_reach = select(reach, i32(ceil((pfr.dof.z + 2.0) / f32(DOF_TILE))), pfr.fx.w > 0.5);
    var m = 0.0;
    var lo = pfr.dof.z;
    var hi = 0.0;
    for (var y = -min_reach; y <= min_reach; y++) {
        for (var x = -min_reach; x <= min_reach; x++) {
            let tile = textureLoad(post_depth, clamp(t + vec2(x, y), vec2(0), dims - 1), 0);
            if (abs(x) <= reach && abs(y) <= reach) { m = max(m, tile.r); }
            lo = min(lo, tile.g);
            hi = max(hi, tile.r);
        }
    }
    return vec4(m, select(-1.0, lo, lo == hi), 0.0, 1.0);
}

// ---------------------------------------------------------------- screen-space ambient occlusion
// post_src: prepass normals (view space) and roughness; post_depth: view depth.

fn view_pos(px: vec2<f32>, z: f32) -> vec3<f32> {
    let c = pfr.screen.xy * 0.5;
    return vec3((px - c) * z / pfr.lens.x, z);
}

fn to_px(p: vec3<f32>) -> vec2<f32> {
    return p.xy * pfr.lens.x / p.z + pfr.screen.xy * 0.5;
}

@fragment
fn fs_ssao(i: PostOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(i.pos.xy);
    let z = textureLoad(post_depth, px, 0).r;
    if (z <= 0.0) { return vec4(1.0); }
    let p = view_pos(i.pos.xy, z);
    let n = normalize(textureLoad(post_src, px, 0).xyz);
    let radius = pfr.fx.x;
    // a tangent frame turned per pixel by a 4 × 4 interleaved pattern
    let rot = f32((px.x & 3) * 4 + (px.y & 3)) * (6.2831853 / 16.0);
    var t = normalize(cross(n, select(vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0), abs(n.z) > 0.9)));
    let b = cross(n, t);
    t = t * cos(rot) + b * sin(rot);
    let bb = cross(n, t);
    let dims = vec2<i32>(textureDimensions(post_depth));
    var occ = 0.0;
    let count = 16;
    for (var k = 0; k < count; k++) {
        // a fixed hemisphere kernel (golden-angle spiral), denser near the centre
        let f = (f32(k) + 0.5) / f32(count);
        let a = f32(k) * 2.39996323;
        let r = sqrt(f);
        let h = sqrt(max(1.0 - f, 0.0));
        let dir = t * (cos(a) * r) + bb * (sin(a) * r) + n * h;
        let scale = mix(0.1, 1.0, f * f);
        let s = p + dir * radius * scale;
        if (s.z <= 0.0) { continue; }
        let q = vec2<i32>(to_px(s));
        if (any(q < vec2(0)) || any(q >= dims)) { continue; }
        let sz = textureLoad(post_depth, q, 0).r;
        if (sz <= 0.0) { continue; }
        let range = smoothstep(0.0, 1.0, radius / max(abs(p.z - sz), 1e-3));
        if (sz < s.z - 0.02 * radius) { occ += range; }
    }
    let ao = clamp(1.0 - pfr.fx.y * occ / f32(count), 0.0, 1.0);
    return vec4(ao, ao, ao, 1.0);
}

// 4 × 4 depth-aware box blur of the occlusion (removes the interleaved pattern)
@fragment
fn fs_ao_blur(i: PostOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(i.pos.xy);
    let z = textureLoad(post_depth, px, 0).r;
    let dims = vec2<i32>(textureDimensions(post_src));
    var sum = 0.0;
    var w = 0.0;
    for (var y = -2; y < 2; y++) {
        for (var x = -2; x < 2; x++) {
            let q = clamp(px + vec2(x, y), vec2(0), dims - 1);
            let qz = textureLoad(post_depth, q, 0).r;
            let wt = select(0.0, 1.0, abs(qz - z) <= max(0.05 * z, 1.0));
            sum += textureLoad(post_src, q, 0).r * wt;
            w += wt;
        }
    }
    let ao = select(1.0, sum / w, w > 0.0);
    return vec4(ao, ao, ao, 1.0);
}
