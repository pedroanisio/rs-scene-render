// ---------------------------------------------------------------- screen-space reflections
// Reflections of what the camera sees: from each reflective opaque pixel of the prepass, the
// reflected view ray is marched in view space against the prepass depth, the hit refined by
// bisection, and the colour found there replaces the environment's share of the specular
// reflection (the reflectance is the prepass's split-sum specular albedo), faded toward screen
// edges, rough surfaces and rays that leave the frame.

@group(0) @binding(0) var<uniform> sfr: Frame;
@group(0) @binding(1) var ssr_color: texture_2d<f32>;
@group(0) @binding(2) var ssr_depth: texture_2d<f32>;
@group(0) @binding(3) var ssr_normal: texture_2d<f32>;
@group(0) @binding(4) var ssr_env: texture_2d<f32>;
@group(0) @binding(5) var ssr_smp: sampler;
@group(0) @binding(6) var ssr_env_smp: sampler;

struct SsrOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_ssr(@builtin(vertex_index) vi: u32) -> SsrOut {
    var o: SsrOut;
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    o.pos = vec4(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2(p.x, 1.0 - p.y);
    return o;
}

fn s_view_pos(px: vec2<f32>, z: f32) -> vec3<f32> {
    return vec3((px - sfr.screen.xy * 0.5) * z / sfr.lens.x, z);
}

fn s_to_px(p: vec3<f32>) -> vec2<f32> {
    return p.xy * sfr.lens.x / p.z + sfr.screen.xy * 0.5;
}

fn s_env(dir: vec3<f32>, level: f32) -> vec3<f32> {
    let d = normalize(dir);
    let u = (atan2(d.x, d.z) + 3.14159265 + sfr.params2.x) / (2.0 * 3.14159265);
    let v = acos(clamp(-d.y, -1.0, 1.0)) / 3.14159265;
    return textureSampleLevel(ssr_env, ssr_env_smp, vec2(u, v), level).rgb * sfr.params.w;
}

@fragment
fn fs_ssr(i: SsrOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(i.pos.xy);
    let base = textureLoad(ssr_color, px, 0);
    let zr = textureLoad(ssr_depth, px, 0);
    let z = zr.r;
    let refl = zr.g;
    if (z <= 0.0 || refl <= 1e-3) { return base; }
    let nr = textureLoad(ssr_normal, px, 0);
    let rough = nr.a;
    if (rough > 0.6) { return base; }
    let n = normalize(nr.xyz);
    // start just off the surface so the ray cannot find the pixel it leaves
    let p = s_view_pos(i.pos.xy, z) + n * max(z * 0.003, 0.5);
    let v = normalize(p);
    let r = reflect(v, n);
    let dims = vec2<f32>(textureDimensions(ssr_depth));
    // march length: across the frame at this depth, and never behind the near plane
    var len = z * max(dims.x, dims.y) / sfr.lens.x;
    if (r.z < 0.0) { len = min(len, (sfr.post.z - p.z) / r.z * 0.99); }
    let steps = 96;
    // the depth a step covers sets how thick a surface is taken to be
    let thick = max(abs(r.z) * len / f32(steps) * 2.0, max(z * 0.01, 1.0));
    // interleaved gradient noise shifts each pixel's samples, trading banding for fine noise
    let jitter = fract(52.9829189 * fract(dot(i.pos.xy, vec2(0.06711056, 0.00583715))));
    var hit = -1.0;
    var prev = 0.0;
    for (var k = 0; k < steps; k++) {
        let t = (f32(k) + jitter) / f32(steps);
        let q = p + r * len * t;
        if (q.z <= sfr.post.z) { break; }
        let qp = s_to_px(q);
        if (any(qp < vec2(0.0)) || any(qp >= dims)) { break; }
        let sz = textureLoad(ssr_depth, vec2<i32>(qp), 0).r;
        if (sz > 0.0 && q.z > sz && q.z - sz < thick) {
            // bisect between the last miss and this hit
            var lo = prev;
            var hi = t;
            for (var b = 0; b < 6; b++) {
                let m = 0.5 * (lo + hi);
                let qm = p + r * len * m;
                let smz = textureLoad(ssr_depth, vec2<i32>(s_to_px(qm)), 0).r;
                if (smz > 0.0 && qm.z > smz) { hi = m; } else { lo = m; }
            }
            hit = hi;
            break;
        }
        prev = t;
    }
    if (hit < 0.0) { return base; }
    let hp = s_to_px(p + r * len * hit);
    // a surface seen from behind is not what the ray reflects (the true hit is hidden)
    let hn = textureLoad(ssr_normal, vec2<i32>(clamp(hp, vec2(0.0), dims - 1.0)), 0).xyz;
    if (dot(hn, r) >= 0.0) { return base; }
    let uv = hp / dims;
    let found = textureSampleLevel(ssr_color, ssr_smp, uv, 0.0).rgb;
    let edge = 1.0 - smoothstep(0.8, 1.0, max(abs(uv.x * 2.0 - 1.0), abs(uv.y * 2.0 - 1.0)));
    // rays turning back toward the camera mostly reach surfaces the camera cannot see
    let toward = smoothstep(-0.6, -0.1, r.z);
    let fade = edge * toward * (1.0 - smoothstep(0.3, 0.6, rough)) * (1.0 - smoothstep(0.7, 1.0, hit));
    // the environment's specular share this pixel already holds
    var env = vec3(0.0);
    if (sfr.params2.z > 0.5) {
        let view3 = mat3x3<f32>(sfr.view[0].xyz, sfr.view[1].xyz, sfr.view[2].xyz);
        env = s_env(transpose(view3) * r, rough * (sfr.params2.w - 1.0));
    }
    let rgb = max(base.rgb + refl * fade * (found - env) * base.a, vec3(0.0));
    return vec4(rgb, base.a);
}
