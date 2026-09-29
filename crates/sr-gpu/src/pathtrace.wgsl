// ---------------------------------------------------------------- path tracing (see pathtrace.rs)

const PI: f32 = 3.14159265358979;

struct Params {
    // camera → world
    cam_to_world: mat4x4<f32>,
    // world → clip of the frame (the composition plane's backdrop lookup)
    view_proj: mat4x4<f32>,
    // world → dome directions (the dome's yaw, pitch and roll)
    env_rot: mat4x4<f32>,
    // width, height, first sample of this dispatch, samples in it
    size: vec4<f32>,
    // focal length (px), lens radius, focus distance, maximum bounces
    cam: vec4<f32>,
    // environment present, intensity, unused, ambient radiance present
    env: vec4<f32>,
    // ambient radiance (rgb), light count
    ambient: vec4<f32>,
    // backdrop present, total samples, denoise step, exposure
    misc: vec4<f32>,
    // encode sRGB, `fin` holds means (denoised), first denoise pass (reads sums), unused
    out: vec4<f32>,
};

struct Mat {
    base: vec4<f32>,
    params: vec4<f32>,
    emissive: vec4<f32>,
    extra: vec4<f32>,
};

struct Node {
    lo: vec3<f32>,
    a: u32,
    hi: vec3<f32>,
    b: u32,
};

struct PLight {
    pos: vec4<f32>,
    dir: vec4<f32>,
    color: vec4<f32>,
    spot: vec4<f32>,
    size: vec4<f32>,
    right: vec4<f32>,
};

@group(0) @binding(0) var<uniform> pp: Params;
// two rows per triangle corner: position (w of the first corner: the material index's bits),
// normal
@group(0) @binding(1) var<storage, read> tverts: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read> mats: array<Mat>;
@group(0) @binding(5) var<storage, read> nodes: array<Node>;
@group(0) @binding(6) var<storage, read> plights: array<PLight>;
// radiance sum (rgb) and coverage sum (a)
@group(0) @binding(7) var<storage, read_write> accum: array<vec4<f32>>;
// denoising guides, two rows a pixel: first-hit albedo sum (rgb) and count (a); normal sum
// (xyz) and depth sum (w)
@group(0) @binding(8) var<storage, read_write> guide: array<vec4<f32>>;

fn tp(k: u32, c: u32) -> vec3<f32> { return tverts[(k * 3u + c) * 2u].xyz; }
fn tn(k: u32, c: u32) -> vec3<f32> { return tverts[(k * 3u + c) * 2u + 1u].xyz; }
fn tmat_of(k: u32) -> u32 { return bitcast<u32>(tverts[k * 6u].w); }
@group(0) @binding(10) var env_tex: texture_2d<f32>;
@group(0) @binding(11) var smp: sampler;
@group(0) @binding(12) var backdrop: texture_2d<f32>;

var<private> rng: u32;

fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

fn rnd() -> f32 {
    rng = pcg(rng);
    return f32(rng >> 8u) / 16777216.0;
}

struct Hit {
    t: f32,
    u: f32,
    v: f32,
    tri: u32,
};

fn tri_hit(k: u32, o: vec3<f32>, d: vec3<f32>, tmax: f32, h: ptr<function, Hit>) {
    let p0 = tp(k, 0u);
    let e1 = tp(k, 1u) - p0;
    let e2 = tp(k, 2u) - p0;
    let pv = cross(d, e2);
    let det = dot(e1, pv);
    if (abs(det) < 1e-12) { return; }
    let inv = 1.0 / det;
    let s = o - p0;
    let u = dot(s, pv) * inv;
    if (u < 0.0 || u > 1.0) { return; }
    let q = cross(s, e1);
    let v = dot(d, q) * inv;
    if (v < 0.0 || u + v > 1.0) { return; }
    let t = dot(e2, q) * inv;
    if (t > 1e-4 && t < min(tmax, (*h).t)) {
        (*h).t = t;
        (*h).u = u;
        (*h).v = v;
        (*h).tri = k;
    }
}

fn box_hit(lo: vec3<f32>, hi: vec3<f32>, o: vec3<f32>, inv: vec3<f32>, tmax: f32) -> bool {
    let t0 = (lo - o) * inv;
    let t1 = (hi - o) * inv;
    let tn = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), min(t0.z, t1.z));
    let tf = min(min(max(t0.x, t1.x), max(t0.y, t1.y)), max(t0.z, t1.z));
    return tf >= max(tn, 0.0) && tn < tmax;
}

fn trace(o: vec3<f32>, d: vec3<f32>, tmax: f32) -> Hit {
    var h: Hit;
    h.t = tmax;
    h.tri = 0xffffffffu;
    let inv = 1.0 / select(d, vec3(1e-12), abs(d) < vec3(1e-12));
    var stack: array<u32, 64>;
    var sp = 0;
    stack[0] = 0u;
    sp = 1;
    while (sp > 0) {
        sp -= 1;
        let n = nodes[stack[sp]];
        if (!box_hit(n.lo, n.hi, o, inv, h.t)) { continue; }
        if (n.b > 0u) {
            for (var k = n.a; k < n.a + n.b; k++) { tri_hit(k, o, d, tmax, &h); }
        } else if (sp < 62) {
            stack[sp] = n.a;
            stack[sp + 1] = n.a - 1u;
            sp += 2;
        }
    }
    return h;
}

fn env_radiance(d: vec3<f32>) -> vec3<f32> {
    var c = vec3(0.0);
    if (pp.env.x > 0.5) {
        let r = normalize((pp.env_rot * vec4(d, 0.0)).xyz);
        let u = (atan2(r.x, r.z) + PI) / (2.0 * PI);
        let v = acos(clamp(-r.y, -1.0, 1.0)) / PI;
        c = textureSampleLevel(env_tex, smp, vec2(u, v), 0.0).rgb * pp.env.y;
    }
    return c + pp.ambient.rgb;
}

// ---------------------------------------------------------------- BSDF (the rasteriser's lobes)

fn d_ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let f = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * f * f + 1e-7);
}

fn v_smith(nl: f32, nv: f32, a: f32) -> f32 {
    let a2 = a * a;
    let gv = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    let gl = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    return 0.5 / max(gv + gl, 1e-7);
}

struct Surf {
    albedo: vec3<f32>,
    metallic: f32,
    a: f32,
    f0: vec3<f32>,
    specw: f32,
    trans: f32,
    ior: f32,
};

fn surf_of(m: Mat) -> Surf {
    var s: Surf;
    s.albedo = m.base.rgb;
    s.metallic = m.params.x;
    s.a = max(m.params.y * m.params.y, 1e-3);
    s.trans = m.params.z;
    s.ior = m.params.w;
    let r = (s.ior - 1.0) / (s.ior + 1.0);
    s.f0 = mix(vec3(r * r), s.albedo, s.metallic);
    s.specw = mix(select(1.0, m.extra.x, m.extra.x > 0.0), 1.0, s.metallic);
    return s;
}

fn bsdf(s: Surf, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    let nl = dot(n, l);
    let nv = dot(n, v);
    if (nl <= 0.0 || nv <= 0.0) { return vec3(0.0); }
    let h = normalize(l + v);
    let nh = max(dot(n, h), 0.0);
    let vh = max(dot(v, h), 0.0);
    let f = s.f0 + (vec3(1.0) - s.f0) * pow(1.0 - vh, 5.0);
    let spec = f * d_ggx(nh, s.a) * v_smith(nl, nv, s.a) * s.specw;
    let kd = (vec3(1.0) - f) * (1.0 - s.metallic) * (1.0 - s.trans);
    return kd * s.albedo / PI + spec;
}

fn frame_of(n: vec3<f32>) -> mat3x3<f32> {
    let up = select(vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0), abs(n.z) > 0.9);
    let t = normalize(cross(up, n));
    return mat3x3<f32>(t, cross(n, t), n);
}

/// Probability of sampling the specular lobe (else the cosine-weighted diffuse lobe).
fn p_spec(s: Surf) -> f32 {
    return clamp(mix(0.25, 1.0, s.metallic) , 0.05, 1.0);
}

fn pdf_of(s: Surf, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>) -> f32 {
    let nl = dot(n, l);
    if (nl <= 0.0) { return 0.0; }
    let h = normalize(l + v);
    let nh = max(dot(n, h), 0.0);
    let vh = max(dot(v, h), 1e-4);
    let ps = p_spec(s);
    return ps * d_ggx(nh, s.a) * nh / (4.0 * vh) + (1.0 - ps) * nl / PI;
}

fn sample_dir(s: Surf, n: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    let tbn = frame_of(n);
    let u1 = rnd();
    let u2 = rnd();
    if (rnd() < p_spec(s)) {
        // GGX half vector
        let a2 = s.a * s.a;
        let ct = sqrt((1.0 - u1) / (1.0 + (a2 - 1.0) * u1));
        let st = sqrt(max(1.0 - ct * ct, 0.0));
        let ph = 2.0 * PI * u2;
        let h = tbn * vec3(st * cos(ph), st * sin(ph), ct);
        return reflect(-v, h);
    }
    let r = sqrt(u1);
    let ph = 2.0 * PI * u2;
    return tbn * vec3(r * cos(ph), r * sin(ph), sqrt(max(1.0 - u1, 0.0)));
}

// ---------------------------------------------------------------- lights

/// Radiance arriving from light `li` at `p` along the returned direction (w: distance), for
/// next-event estimation; area lights are sampled at a random point.
fn light_sample(li: PLight, p: vec3<f32>) -> vec4<f32> {
    let ty = u32(li.pos.w);
    if (ty == 1u) { return vec4(-li.dir.xyz, 1e30); }
    var q = li.pos.xyz;
    if (ty == 6u) {
        let z = 2.0 * rnd() - 1.0;
        let a = 2.0 * PI * rnd();
        let r = sqrt(max(1.0 - z * z, 0.0));
        q = q + vec3(r * cos(a), r * sin(a), z) * li.size.x;
    } else if (ty == 4u) {
        let up = normalize(cross(li.dir.xyz, li.right.xyz));
        q = q + li.right.xyz * (rnd() - 0.5) * li.spot.z + up * (rnd() - 0.5) * li.spot.w;
    } else if (ty == 5u) {
        let up = normalize(cross(li.dir.xyz, li.right.xyz));
        let r = sqrt(rnd()) * li.spot.z * 0.5;
        let a = 2.0 * PI * rnd();
        q = q + (li.right.xyz * cos(a) + up * sin(a)) * r;
    }
    let to = q - p;
    let dist = length(to);
    return vec4(to / max(dist, 1e-4), dist);
}

fn light_radiance(li: PLight, l: vec3<f32>, dist: f32) -> vec3<f32> {
    let ty = u32(li.pos.w);
    var rad = li.color.rgb;
    if (ty == 1u) { return rad; }
    let m = max(dist / 100.0, 0.01);
    var att = 1.0 / pow(m, li.color.w);
    if (li.dir.w > 0.0) {
        let q = dist / li.dir.w;
        att *= clamp(1.0 - q * q * q * q, 0.0, 1.0);
        att *= clamp(1.0 - q * q * q * q, 0.0, 1.0);
    }
    rad *= att;
    if (ty == 3u) { rad *= smoothstep(li.spot.x, li.spot.y, dot(-l, li.dir.xyz)); }
    if (ty == 4u || ty == 5u) { rad *= max(dot(-l, li.dir.xyz), 0.0); }
    return rad;
}

// ---------------------------------------------------------------- one path

fn radiance(px: vec2<f32>, pix: u32, first: bool) -> vec4<f32> {
    let w = pp.size.x;
    let h = pp.size.y;
    // camera ray (thin lens when depth of field is on), in camera space: x right, y down, z forward
    var dir_c = vec3((px.x - w * 0.5) / pp.cam.x, (px.y - h * 0.5) / pp.cam.x, 1.0);
    var org_c = vec3(0.0);
    if (pp.cam.y > 0.0) {
        let r = sqrt(rnd()) * pp.cam.y;
        let a = 2.0 * PI * rnd();
        let focus = dir_c * pp.cam.z;
        org_c = vec3(r * cos(a), r * sin(a), 0.0);
        dir_c = focus - org_c;
    }
    var o = (pp.cam_to_world * vec4(org_c, 1.0)).xyz;
    var d = normalize((pp.cam_to_world * vec4(dir_c, 0.0)).xyz);
    var thr = vec3(1.0);
    var col = vec3(0.0);
    var alpha = 0.0;
    var only_glass = true;
    var bounce = 0u;
    // surfaces met so far (transmission and opacity events do not count as bounces)
    var met = 0u;
    let max_b = u32(pp.cam.w);
    for (var step = 0u; step < 64u; step++) {
        let hit = trace(o, d, 1e30);
        if (hit.tri == 0xffffffffu) {
            if (met == 0u) {
                // a primary miss: the pass is transparent there (the environment when visible)
                if (pp.env.x > 1.5) { col += thr * env_radiance(d); alpha = 1.0; }
                break;
            }
            if (only_glass && pp.misc.x > 0.5) {
                // seen through glass: the 2D layers on the composition plane (z = 0), where the
                // ray crosses it (or at the exit point when the glass reaches past the plane)
                var t = 0.0;
                if (d.z > 0.0) { t = max(-o.z / d.z, 0.0); }
                if (d.z > 0.0 || o.z >= 0.0) {
                    let c = pp.view_proj * vec4(o + d * t, 1.0);
                    let uv = vec2(c.x / c.w * 0.5 + 0.5, 0.5 - c.y / c.w * 0.5);
                    if (all(uv >= vec2(0.0)) && all(uv <= vec2(1.0))) {
                        col += thr * textureSampleLevel(backdrop, smp, uv, 0.0).rgb;
                        break;
                    }
                }
            }
            col += thr * env_radiance(d);
            break;
        }
        let k = hit.tri;
        let m = mats[tmat_of(k)];
        let p0 = tp(k, 0u);
        let ng0 = normalize(cross(tp(k, 1u) - p0, tp(k, 2u) - p0));
        let bw = 1.0 - hit.u - hit.v;
        var n = tn(k, 0u) * bw + tn(k, 1u) * hit.u + tn(k, 2u) * hit.v;
        n = select(normalize(n), ng0, dot(n, n) < 1e-8);
        let p = o + d * hit.t;
        // stochastic opacity: pass straight through
        if (m.base.a < 1.0 && rnd() > m.base.a) {
            o = p + d * 1e-3;
            continue;
        }
        alpha = 1.0;
        met += 1u;
        let entering = dot(d, ng0) < 0.0;
        let ng = select(-ng0, ng0, entering);
        if (dot(n, ng) < 0.0) { n = -n; }
        if (met == 1u && first) {
            guide[pix * 2u] += vec4(m.base.rgb, 1.0);
            guide[pix * 2u + 1u] += vec4(n, hit.t);
        }
        col += thr * m.emissive.rgb;
        if (m.emissive.w > 0.5) { break; }
        let s = surf_of(m);
        let v = -d;
        // dielectric transmission (smooth): Fresnel picks reflection or refraction
        if (s.trans > 0.0 && rnd() < s.trans) {
            let eta = select(s.ior, 1.0 / s.ior, entering);
            let cosi = clamp(dot(v, n), 0.0, 1.0);
            let r0 = (1.0 - eta) / (1.0 + eta);
            let fr = r0 * r0 + (1.0 - r0 * r0) * pow(1.0 - cosi, 5.0);
            let t = refract(d, n, eta);
            if (rnd() < fr || dot(t, t) < 1e-8) {
                d = reflect(d, n);
                o = p + n * 1e-3;
            } else {
                d = normalize(t);
                o = p - n * 1e-3;
                thr *= s.albedo;
            }
            continue;
        }
        only_glass = false;
        if (bounce >= max_b) { break; }
        // next-event estimation toward every analytic light
        let nl_count = u32(pp.ambient.w);
        for (var li = 0u; li < nl_count; li++) {
            let lt = plights[li];
            let ty = u32(lt.pos.w);
            if (ty == 0u || lt.pos.w < 0.0) { continue; }
            let ls = light_sample(lt, p);
            let l = ls.xyz;
            let nl = dot(n, l);
            if (nl <= 0.0) { continue; }
            let rad = light_radiance(lt, l, ls.w);
            if (max(rad.r, max(rad.g, rad.b)) <= 0.0) { continue; }
            if (lt.size.y > 0.5) {
                let sh = trace(p + ng * 1e-2, l, ls.w - 2e-2);
                if (sh.tri != 0xffffffffu) { continue; }
            }
            var c = thr * bsdf(s, n, v, l) * nl * rad;
            // clamp rare fireflies from indirect paths
            if (bounce > 0u) { c = min(c, vec3(20.0)); }
            col += c;
        }
        // continue the path by sampling the BSDF
        let l = sample_dir(s, n, v);
        let pdf = pdf_of(s, n, v, l);
        let nl = dot(n, l);
        if (pdf <= 1e-8 || nl <= 0.0) { break; }
        thr *= bsdf(s, n, v, l) * nl / pdf;
        bounce += 1u;
        if (bounce > 2u) {
            let q = min(max(thr.r, max(thr.g, thr.b)), 0.95);
            if (rnd() > q) { break; }
            thr /= q;
        }
        o = p + ng * 1e-3;
        d = l;
    }
    return vec4(col, alpha);
}

@compute @workgroup_size(8, 8)
fn cs_trace(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = u32(pp.size.x);
    let h = u32(pp.size.y);
    if (gid.x >= w || gid.y >= h) { return; }
    let pix = gid.y * w + gid.x;
    var sum = vec4(0.0);
    let start = u32(pp.size.z);
    let count = u32(pp.size.w);
    for (var s = 0u; s < count; s++) {
        let si = start + s;
        rng = pcg(pix * 9781u + pcg(si * 6271u + 1u));
        let jitter = vec2(rnd(), rnd());
        sum += radiance(vec2<f32>(gid.xy) + jitter, pix, true);
    }
    accum[pix] += sum;
}

// ---------------------------------------------------------------- à-trous denoising
// One pass of the edge-aware à-trous filter at `misc.z` pixels between taps: src → dst. The
// first pass reads the sample sums (out.z set) and writes means.

@group(1) @binding(0) var<storage, read> src: array<vec4<f32>>;
@group(1) @binding(1) var<storage, read_write> dst: array<vec4<f32>>;

@compute @workgroup_size(8, 8)
fn cs_atrous(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = i32(pp.size.x);
    let h = i32(pp.size.y);
    if (i32(gid.x) >= w || i32(gid.y) >= h) { return; }
    let step = i32(pp.misc.z);
    let spp = max(pp.misc.y, 1.0);
    let scale = select(1.0, 1.0 / spp, pp.out.z > 0.5);
    let pi = i32(gid.y) * w + i32(gid.x);
    let cp = src[pi] * scale;
    let gp = guide[pi * 2];
    let gcount = max(gp.w, 1e-6);
    let ap = gp.rgb / gcount;
    let np = guide[pi * 2 + 1].xyz / gcount;
    if (gp.w <= 0.0) { dst[pi] = cp; return; }
    let kern = array<f32, 3>(0.375, 0.25, 0.0625);
    var sum = vec4(0.0);
    var wsum = 0.0;
    let lum_p = dot(cp.rgb, vec3(0.2126, 0.7152, 0.0722));
    let sigma_c = 0.4 * max(lum_p, 0.05) + 0.02;
    for (var dy = -2; dy <= 2; dy++) {
        for (var dx = -2; dx <= 2; dx++) {
            let q = vec2(i32(gid.x) + dx * step, i32(gid.y) + dy * step);
            if (q.x < 0 || q.y < 0 || q.x >= w || q.y >= h) { continue; }
            let qi = q.y * w + q.x;
            let gq = guide[qi * 2].w;
            if (gq <= 0.0) { continue; }
            let cq = src[qi] * scale;
            let aq = guide[qi * 2].rgb / gq;
            let nq = guide[qi * 2 + 1].xyz / gq;
            let k = kern[abs(dx)] * kern[abs(dy)];
            let dc = cq.rgb - cp.rgb;
            let wc = exp(-dot(dc, dc) / (sigma_c * sigma_c));
            let wn = pow(max(dot(np, nq), 0.0), 64.0);
            let da = aq - ap;
            let wa = exp(-dot(da, da) / 0.02);
            let wt = k * wc * wn * wa;
            sum += cq * wt;
            wsum += wt;
        }
    }
    dst[pi] = select(cp, sum / wsum, wsum > 1e-8);
}

// ---------------------------------------------------------------- output

@group(1) @binding(0) var<storage, read> fin: array<vec4<f32>>;

struct VO {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_out(@builtin(vertex_index) vi: u32) -> VO {
    var o: VO;
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    o.pos = vec4(p * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3(0.0031308));
}

@fragment
fn fs_out(i: VO) -> @location(0) vec4<f32> {
    let w = u32(pp.size.x);
    let px = vec2<u32>(i.pos.xy);
    // `fin` holds per-pixel means when denoised, sums over the samples otherwise
    var c = fin[px.y * w + px.x] / select(max(pp.misc.y, 1.0), 1.0, pp.out.y > 0.5);
    var rgb = c.rgb * pp.misc.w;
    let a = clamp(c.a, 0.0, 1.0);
    if (pp.out.x > 0.5 && a > 0.0) {
        rgb = srgb_encode(rgb / a) * a;
    }
    return vec4(rgb, a);
}
