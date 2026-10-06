// ---------------------------------------------------------------- path tracing (see pathtrace.rs)

const PI: f32 = 3.14159265358979;
override HAS_MEDIA: bool = false;
override MEDIUM_LIGHTING: bool = false;
// The scene has transmissive materials: paths track the medium they are inside (absorption).
override WATER: bool = false;

struct Params {
    // camera → world
    cam_to_world: mat4x4<f32>,
    // world → clip of the frame (the composition plane's backdrop lookup)
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
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
    // Global origin (xy) and extent (zw) of this tile's working buffers.
    tile: vec4<u32>,
    // Packed medium row offset, domain count, conservative sample budget, reserved.
    media: vec4<u32>,
};

struct Mat {
    base: vec4<f32>,
    params: vec4<f32>,
    emissive: vec4<f32>,
    extra: vec4<f32>,
    maps: array<vec4<u32>, 6>,
    texture_params: vec4<f32>,
    borders: array<vec4<f32>, 6>,
    // absorption coefficient per unit inside the enclosed medium (rgb)
    attenuation: vec4<f32>,
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
// Each primitive occupies 24 rows. Triangles use six rows per corner for position,
// normal, colour and map coordinates; splats use covariance and harmonic records.
@group(0) @binding(1) var<storage, read> tverts: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read> mats: array<Mat>;
@group(0) @binding(5) var<storage, read> nodes: array<Node>;
@group(0) @binding(6) var<storage, read> plights: array<PLight>;
// radiance sum (rgb) and coverage sum (a)
@group(0) @binding(7) var<storage, read_write> accum: array<vec4<f32>>;
// denoising guides, two rows a pixel: first-hit albedo sum (rgb) and count (a); normal sum
// (xyz) and depth sum (w)
@group(0) @binding(8) var<storage, read_write> guide: array<vec4<f32>>;

fn tp(k: u32, c: u32) -> vec3<f32> { return tverts[k * 24u + c * 6u].xyz; }
fn tn(k: u32, c: u32) -> vec3<f32> { return tverts[k * 24u + c * 6u + 1u].xyz; }
fn tmat_of(k: u32) -> u32 { return bitcast<u32>(tverts[k * 24u].w); }
@group(0) @binding(10) var env_tex: texture_2d<f32>;
@group(0) @binding(11) var smp: sampler;
@group(0) @binding(12) var backdrop: texture_2d<f32>;

fn tuv(k: u32, c: u32, slot: u32) -> vec2<f32> {
    let row = tverts[k * 24u + c * 6u + 3u + slot / 2u];
    return select(row.xy, row.zw, slot % 2u == 1u);
}
fn texel(info: vec4<u32>, xy: vec2<i32>, border: vec4<f32>) -> vec4<f32> {
    let size = vec2<i32>(info.yz);
    let q = vec2(address_index(xy.x, size.x, (info.w >> 3u) & 3u), address_index(xy.y, size.y, (info.w >> 5u) & 3u));
    if (any(q < vec2(0))) { return border; }
    let index = info.x + u32(q.y) * info.y + u32(q.x);
    let c = unpack4x8unorm(bitcast<u32>(tverts[index / 4u][index % 4u]));
    if ((info.w & 1u) == 0u) { return c; }
    let rgb = select(pow((c.rgb + 0.055) / 1.055, vec3(2.4)), c.rgb / 12.92, c.rgb <= vec3(0.04045));
    return vec4(rgb, c.a);
}
fn map_sample(info: vec4<u32>, uv: vec2<f32>, border: vec4<f32>) -> vec4<f32> {
    if (info.y == 0u || info.z == 0u) { return vec4(1.0); }
    let p = sample_position(uv, vec2<f32>(info.yz), info.w);
    let q = vec2<i32>(floor(p)); let f = fract(p);
    let filtering = (info.w >> 1u) & 3u;
    if (filtering == 1u) { return texel(info, vec2<i32>(floor(p + 0.5)), border); }
    if (filtering == 2u) {
        let wx = cubic_weights(f.x); let wy = cubic_weights(f.y);
        var out = vec4(0.0);
        for (var y = 0; y < 4; y++) { for (var x = 0; x < 4; x++) {
            out += texel(info, q + vec2(x-1,y-1), border) * wx[x] * wy[y];
        }}
        return out;
    }
    return mix(mix(texel(info,q,border),texel(info,q+vec2(1,0),border),f.x),mix(texel(info,q+vec2(0,1),border),texel(info,q+vec2(1,1),border),f.x),f.y);
}

fn pt_sh_c(i:u32,k:u32)->vec3<f32> {
    let offset=i*96u+32u+k*3u;
    return vec3(tverts[offset/4u][offset%4u],tverts[(offset+1u)/4u][(offset+1u)%4u],tverts[(offset+2u)/4u][(offset+2u)%4u]);
}
fn pt_sh_color(i: u32, deg: u32, d: vec3<f32>) -> vec3<f32> {
    let c0 = 0.28209479177387814;
    var r = c0 * pt_sh_c(i, 0u);
    if (deg >= 1u) {
        let c1 = 0.4886025119029199;
        r += -c1 * d.y * pt_sh_c(i, 1u) + c1 * d.z * pt_sh_c(i, 2u) - c1 * d.x * pt_sh_c(i, 3u);
    }
    if (deg >= 2u) {
        let xx = d.x * d.x; let yy = d.y * d.y; let zz = d.z * d.z;
        r += 1.0925484305920792 * d.x * d.y * pt_sh_c(i, 4u)
            - 1.0925484305920792 * d.y * d.z * pt_sh_c(i, 5u)
            + 0.31539156525252005 * (2.0 * zz - xx - yy) * pt_sh_c(i, 6u)
            - 1.0925484305920792 * d.x * d.z * pt_sh_c(i, 7u)
            + 0.5462742152960396 * (xx - yy) * pt_sh_c(i, 8u);
        if (deg >= 3u) {
            r += -0.5900435899266435 * d.y * (3.0 * xx - yy) * pt_sh_c(i, 9u)
                + 2.890611442640554 * d.x * d.y * d.z * pt_sh_c(i, 10u)
                - 0.4570457994644658 * d.y * (4.0 * zz - xx - yy) * pt_sh_c(i, 11u)
                + 0.3731763325901154 * d.z * (2.0 * zz - 3.0 * xx - 3.0 * yy) * pt_sh_c(i, 12u)
                - 0.4570457994644658 * d.x * (4.0 * zz - xx - yy) * pt_sh_c(i, 13u)
                + 1.445305721320277 * d.z * (xx - yy) * pt_sh_c(i, 14u)
                - 0.5900435899266435 * d.x * (xx - 3.0 * yy) * pt_sh_c(i, 15u);
        }
    }
    return max(r + 0.5, vec3(0.0));
}

fn srgb_decode(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3(2.4));
    return select(hi, lo, c <= vec3(0.04045));
}

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
    instance: u32,
};

fn tri_hit(k: u32, o: vec3<f32>, d: vec3<f32>, tmax: f32, h: ptr<function, Hit>, instance: u32) {
    if (tmat_of(k)==0xffffffffu) {
        let start=k*24u;
        let p=o-tverts[start].xyz;
        let inverse=mat3x3(tverts[start+1u].xyz,tverts[start+2u].xyz,tverts[start+3u].xyz);
        let t=-dot(d,inverse*p)/max(dot(d,inverse*d),1e-20);
        if (t<=1e-4 || t>=min(tmax,(*h).t)) {return;}
        let q=p+d*t;
        let radius=dot(q,inverse*q);
        let opacity=min(0.99,tverts[start+4u].w*exp(-0.5*radius));
        if (radius>9.0 || opacity<1.0/255.0) {return;}
        (*h).t=t;(*h).u=opacity;(*h).v=0.0;(*h).tri=k;(*h).instance=instance;return;
    }
    let p0 = tp(k, 0u);
    let e1 = tp(k, 1u) - p0;
    let e2 = tp(k, 2u) - p0;
    let pv = cross(d, e2);
    let det = dot(e1, pv);
    var determinant_scale = 1.0;
    if (instance != 0xffffffffu) { determinant_scale = tverts[instance*24u+13u].z; }
    if (abs(det * determinant_scale) < 1e-12) { return; }
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
        (*h).instance = instance;
    }
}

fn box_hit(lo: vec3<f32>, hi: vec3<f32>, o: vec3<f32>, inv: vec3<f32>, tmax: f32) -> bool {
    if (any(lo > hi)) { return false; }
    let t0 = (lo - o) * inv;
    let t1 = (hi - o) * inv;
    let tn = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), min(t0.z, t1.z));
    let tf = min(min(max(t0.x, t1.x), max(t0.y, t1.y)), max(t0.z, t1.z));
    return tf >= max(tn, 0.0) && tn < tmax;
}

fn instance_matrix(instance: u32, offset: u32) -> mat4x4<f32> {
    let start = instance*24u+offset;
    return mat4x4(tverts[start],tverts[start+1u],tverts[start+2u],tverts[start+3u]);
}
fn hit_material(h: Hit) -> u32 {
    if (h.instance != 0xffffffffu) { return bitcast<u32>(tverts[h.instance*24u+13u].y); }
    return tmat_of(h.tri);
}
fn hit_position(h: Hit, corner: u32) -> vec3<f32> {
    let p = tp(h.tri, corner);
    if (h.instance != 0xffffffffu) { return (instance_matrix(h.instance,1u)*vec4(p,1.0)).xyz; }
    return p;
}
fn hit_normal(h: Hit, corner: u32) -> vec3<f32> {
    let n = tn(h.tri, corner);
    if (h.instance != 0xffffffffu) {
        let transformed = (instance_matrix(h.instance,9u)*vec4(n,0.0)).xyz;
        if (dot(transformed,transformed) > 0.0) { return normalize(transformed); }
        return vec3(0.0);
    }
    return n;
}
fn prototype_hit(instance: u32, world_o: vec3<f32>, world_d: vec3<f32>, tmax: f32, h: ptr<function,Hit>) {
    let inverse = instance_matrix(instance,5u);
    let o = (inverse*vec4(world_o,1.0)).xyz;
    // Do not normalize: the ray parameter remains the world-space distance.
    let d = (inverse*vec4(world_d,0.0)).xyz;
    let inv = 1.0 / select(d,vec3(1e-12),abs(d)<vec3(1e-12));
    var stack: array<u32,64>;
    stack[0] = bitcast<u32>(tverts[instance*24u+13u].x);
    var sp = 1;
    while (sp > 0) {
        sp -= 1;
        let n = nodes[stack[sp]];
        if (!box_hit(n.lo,n.hi,o,inv,(*h).t)) { continue; }
        if (n.b > 0u) {
            for (var k=n.a; k<n.a+n.b; k++) { tri_hit(k,o,d,tmax,h,instance); }
        } else if (sp < 62) {
            stack[sp]=n.a;stack[sp+1]=n.a-1u;sp+=2;
        }
    }
}

fn trace(o: vec3<f32>, d: vec3<f32>, tmax: f32) -> Hit {
    var h: Hit;
    h.t = tmax;
    h.tri = 0xffffffffu;
    h.instance = 0xffffffffu;
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
            for (var k = n.a; k < n.a + n.b; k++) {
                if (tmat_of(k) == 0xfffffffeu) { prototype_hit(k,o,d,tmax,&h); }
                else { tri_hit(k, o, d, tmax, &h, 0xffffffffu); }
            }
        } else if (sp < 62) {
            stack[sp] = n.a;
            stack[sp + 1] = n.a - 1u;
            sp += 2;
        }
    }
    return h;
}

fn env_radiance(d: vec3<f32>, diffuse: vec3<f32>, specular: vec3<f32>) -> vec3<f32> {
    var c = vec3(0.0);
    if (pp.env.x > 0.5) {
        let r = normalize((pp.env_rot * vec4(d, 0.0)).xyz);
        let u = (atan2(r.x, r.z) + PI) / (2.0 * PI);
        let v = acos(clamp(-r.y, -1.0, 1.0)) / PI;
        c = textureSampleLevel(env_tex, smp, vec2(u, v), 0.0).rgb * pp.env.y;
    }
    for (var li = 0u; li < u32(pp.ambient.w); li++) {
        let light = plights[li];
        if (light.pos.w != 0.0) { continue; }
        let lobes = light_lobes(light);
        c += light.color.rgb * (diffuse * lobes.x + specular * lobes.y);
    }
    return c;
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
    s.specw = mix(clamp(m.extra.x, 0.0, 1.0), 1.0, s.metallic);
    return s;
}

fn light_lobes(light: PLight) -> vec2<f32> {
    let flags = u32(light.right.w);
    return vec2(f32(flags & 1u), f32((flags >> 1u) & 1u));
}

fn bsdf(s: Surf, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, lobes: vec2<f32>) -> vec3<f32> {
    let nl = dot(n, l);
    let nv = dot(n, v);
    if (nl <= 0.0 || nv <= 0.0) { return vec3(0.0); }
    let h = normalize(l + v);
    let nh = max(dot(n, h), 0.0);
    let vh = max(dot(v, h), 0.0);
    let f = s.f0 + (vec3(1.0) - s.f0) * pow(1.0 - vh, 5.0);
    let spec = f * d_ggx(nh, s.a) * v_smith(nl, nv, s.a) * s.specw;
    let kd = (vec3(1.0) - f) * (1.0 - s.metallic) * (1.0 - s.trans);
    return kd * s.albedo / PI * lobes.x + spec * lobes.y;
}

fn frame_of(n: vec3<f32>) -> mat3x3<f32> {
    let up = select(vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0), abs(n.z) > 0.9);
    let t = normalize(cross(up, n));
    return mat3x3<f32>(t, cross(n, t), n);
}

/// Probability of sampling the specular lobe (else the cosine-weighted diffuse lobe): the share
/// of the surface's reflectance that is specular at this view angle, so that a dielectric seen at
/// a grazing angle (water, glossy paint) spends its samples where the energy is. Any value in
/// (0, 1) gives an unbiased estimate; a surface with no diffuse lobe (a metal) takes 1.
fn p_spec(s: Surf, n: vec3<f32>, v: vec3<f32>) -> f32 {
    let lum = vec3(0.2126, 0.7152, 0.0722);
    let f = s.f0 + (vec3(1.0) - s.f0) * pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 5.0);
    let ls = dot(f, lum) * s.specw;
    let ld = dot((vec3(1.0) - f) * s.albedo, lum) * (1.0 - s.metallic) * (1.0 - s.trans);
    if (ld <= 0.0) { return 1.0; }
    return clamp(ls / max(ls + ld, 1e-6), 0.05, 0.95);
}

fn pdf_of(s: Surf, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>) -> f32 {
    let nl = dot(n, l);
    if (nl <= 0.0) { return 0.0; }
    let h = normalize(l + v);
    let nh = max(dot(n, h), 0.0);
    let vh = max(dot(v, h), 1e-4);
    let ps = p_spec(s, n, v);
    return ps * d_ggx(nh, s.a) * nh / (4.0 * vh) + (1.0 - ps) * nl / PI;
}

fn sample_dir(s: Surf, n: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    let tbn = frame_of(n);
    let u1 = rnd();
    let u2 = rnd();
    if (rnd() < p_spec(s, n, v)) {
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

// Alpha-tested and blended surfaces attenuate shadow rays using the same base map
// as camera rays. Captured Gaussian splats do not cast shadows.
fn visibility(origin: vec3<f32>, direction: vec3<f32>, distance: f32) -> f32 {
    var o = origin;
    var remaining = distance;
    var result = 1.0;
    for (var step = 0u; step < 64u; step++) {
        let hit = trace(o, direction, remaining);
        if (hit.tri == 0xffffffffu) { return result; }
        let k = hit.tri;
        if (tmat_of(k) != 0xffffffffu) {
            let m = mats[hit_material(hit)];
            var opacity = m.base.a;
            if (m.texture_params.y != 0.0) {
                let b = vec3(1.0-hit.u-hit.v,hit.u,hit.v);
                let uv = tuv(k,0u,0u)*b.x+tuv(k,1u,0u)*b.y+tuv(k,2u,0u)*b.z;
                let vertex_alpha = tverts[k*24u+2u].a*b.x+tverts[k*24u+8u].a*b.y+tverts[k*24u+14u].a*b.z;
                opacity *= map_sample(m.maps[0], uv, m.borders[0]).a*vertex_alpha;
                if (m.texture_params.y == 1.0) { opacity = select(0.0,1.0,opacity >= m.texture_params.x); }
            }
            if (m.extra.y > 0.5) { result *= 1.0-clamp(opacity,0.0,1.0); }
            if (result < 1e-5) { return 0.0; }
        }
        let advance = hit.t+1e-3;
        o += direction*advance;
        remaining -= advance;
        if (remaining <= 0.0) { return result; }
    }
    return 0.0;
}

// Direction and distance to light `li` at `p` for next-event estimation;
// area lights are sampled at a random point.
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

fn ies_value(li: PLight, l: vec3<f32>) -> f32 {
    if (li.size.w == 0.0) { return 1.0; }
    let direction=-l;
    let theta=acos(clamp(dot(direction,li.dir.xyz),-1.0,1.0))/PI;
    let up=cross(li.dir.xyz,li.right.xyz);
    let phi=fract(atan2(dot(direction,up),dot(direction,li.right.xyz))/(2.0*PI)+1.0);
    let p=vec2(theta*127.0,phi*32.0);
    let q=vec2<u32>(floor(p));let f=fract(p);
    let offset=bitcast<u32>(li.size.z);
    let ix=array<u32,4>(offset+(q.y%32u)*128u+q.x,offset+(q.y%32u)*128u+min(q.x+1u,127u),offset+((q.y+1u)%32u)*128u+q.x,offset+((q.y+1u)%32u)*128u+min(q.x+1u,127u));
    let a=tverts[ix[0]/4u][ix[0]%4u];let b=tverts[ix[1]/4u][ix[1]%4u];
    let c=tverts[ix[2]/4u][ix[2]%4u];let d=tverts[ix[3]/4u][ix[3]%4u];
    return mix(mix(a,b,f.x),mix(c,d,f.x),f.y);
}

fn light_radiance(li: PLight, l: vec3<f32>, dist: f32) -> vec3<f32> {
    let ty = u32(li.pos.w);
    var rad = li.color.rgb * ies_value(li,l);
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

// What a path carries out of a smooth dielectric surface it has chosen to cross: the next ray, its
// throughput, and the absorption of the medium it is now in; `refracted` says whether the path went
// through (a reflection leaves the medium it is in unchanged). `inside` is set by the water shader,
// which tracks whether the path is inside the medium, and only means something when `refracted`.
struct Crossing {
    d: vec3<f32>,
    o: vec3<f32>,
    thr: vec3<f32>,
    in_sigma: vec3<f32>,
    inside: bool,
    refracted: bool,
}

// Fresnel picks reflection or refraction at a smooth dielectric surface the path crosses at `p`
// (`n` on the side of the ray `v` comes from, `entering` when the ray goes into the denser medium).
fn dielectric_crossing(
    s: Surf, m: Mat, d_in: vec3<f32>, o_in: vec3<f32>, p: vec3<f32>, n: vec3<f32>, v: vec3<f32>,
    entering: bool, thr_in: vec3<f32>, sigma_in: vec3<f32>,
) -> Crossing {
    var d = d_in;
    var o = o_in;
    var thr = thr_in;
    var in_sigma = sigma_in;
    var inside = false;
    var refracted = false;
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
        refracted = true;
        thr *= s.albedo;
        if (WATER) { in_sigma = select(vec3(0.0), m.attenuation.rgb, entering); }
    }
    return Crossing(d, o, thr, in_sigma, inside, refracted);
}

fn radiance(px: vec2<f32>, pix: u32, first: bool) -> vec4<f32> {
    let w = pp.size.x;
    let h = pp.size.y;
    // Unprojection supports perspective, orthographic and offscreen clip transforms.
    var ndc = vec2(px.x/w*2.0-1.0,1.0-px.y/h*2.0);
    ndc *= 1.0 + pp.cam.x * dot(ndc,ndc);
    let a = pp.inv_view_proj * vec4(ndc,1.0,1.0);
    let b = pp.inv_view_proj * vec4(ndc,0.0,1.0);
    let near = a.xyz/a.w;
    let far = b.xyz/b.w;
    var o = (pp.cam_to_world*vec4(0.0,0.0,0.0,1.0)).xyz;
    if (pp.env.z > 0.5) { o=near; }
    var d = normalize(far-near);
    if (pp.cam.y>0.0 && pp.env.z<0.5) {
        let forward=(pp.cam_to_world*vec4(0.0,0.0,1.0,0.0)).xyz;
        let focus=o+d*(pp.cam.z/max(dot(d,forward),1e-6));
        let radius=sqrt(rnd())*pp.cam.y; let angle=2.0*PI*rnd();
        o+=(pp.cam_to_world*vec4(radius*cos(angle),radius*sin(angle),0.0,0.0)).xyz;
        d=normalize(focus-o);
    }
    var thr = vec3(1.0);
    // Fraction of the last scattering event carried by each BSDF lobe.
    // This lets ambient lights select lobes without changing dome illumination.
    var ambient_diffuse = vec3(0.5);
    var ambient_specular = vec3(0.5);
    var col = vec3(0.0);
    var alpha = 0.0;
    var only_glass = true;
    // absorption of the medium the path is inside (set where it refracts in, cleared where it leaves)
    var in_sigma = vec3(0.0);
    var bounce = 0u;
    // surfaces met so far (transmission and opacity events do not count as bounces)
    var met = 0u;
    let max_b = u32(pp.cam.w);
    for (var step = 0u; step < 64u; step++) {
        let hit = trace(o, d, 1e30);
        if (WATER && any(in_sigma > vec3(0.0))) { thr *= exp(-in_sigma * min(hit.t, 1e4)); }
        if (HAS_MEDIA) {
            let fog = volume_transport(o, d, hit.t);
            col += thr * fog.rgb;
            thr *= fog.a;
            if (met == 0u) { alpha += (1.0 - alpha) * (1.0 - fog.a); }
        }
        if (hit.tri == 0xffffffffu) {
            if (met == 0u) {
                // a primary miss: the pass is transparent there (the environment when visible)
                if (pp.env.x > 1.5) { col += thr * env_radiance(d, ambient_diffuse, ambient_specular); alpha = 1.0; }
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
                        // premultiplied: where the layers leave the pixel open, the visible dome shows
                        let behind = textureSampleLevel(backdrop, smp, uv, 0.0);
                        var seen = behind.rgb;
                        if (pp.env.x > 1.5) { seen += (1.0 - behind.a) * env_radiance(d, ambient_diffuse, ambient_specular); }
                        col += thr * seen;
                        break;
                    }
                }
            }
            col += thr * env_radiance(d, ambient_diffuse, ambient_specular);
            break;
        }
        let k = hit.tri;
        if (tmat_of(k)==0xffffffffu) {
            let start=k*24u;
            var rgb=tverts[start+4u].rgb;
            let degree=u32(tverts[start+20u].x);
            if (degree>0u) {
                let local=mat3x3(tverts[start+5u].xyz,tverts[start+6u].xyz,tverts[start+7u].xyz);
                rgb=srgb_decode(min(pt_sh_color(k,degree,normalize(local*d)),vec3(1.0)));
            }
            col+=thr*rgb*hit.u;thr*=1.0-hit.u;
            if (met==0u) {alpha+=(1.0-alpha)*hit.u;}
            o+=d*(hit.t+1e-3);
            if (max(thr.x,max(thr.y,thr.z))<1e-5) {break;}
            continue;
        }
        var m = mats[hit_material(hit)];
        let bary = vec3(1.0 - hit.u - hit.v, hit.u, hit.v);
        var uv: array<vec2<f32>,6>;
        for (var slot = 0u; slot < 6u; slot++) { uv[slot] = tuv(k,0u,slot)*bary.x + tuv(k,1u,slot)*bary.y + tuv(k,2u,slot)*bary.z; }
        let base_texel = map_sample(m.maps[0], uv[0], m.borders[0]);
        let color = tverts[k*24u+2u]*bary.x+tverts[k*24u+8u]*bary.y+tverts[k*24u+14u]*bary.z;
        m.base = m.base * base_texel * color;
        let mr = map_sample(m.maps[2], uv[2], m.borders[2]);
        m.params.x *= mr.b; m.params.y *= mr.g;
        m.emissive = vec4(m.emissive.rgb * map_sample(m.maps[4], uv[4], m.borders[4]).rgb, m.emissive.w);
        if (m.emissive.w > 0.5) { m.emissive = vec4(m.base.rgb, m.emissive.w); }
        if (m.texture_params.y == 0.0) { m.base.a = mats[hit_material(hit)].base.a; }
        if (m.texture_params.y == 1.0) { m.base.a = select(0.0,1.0,m.base.a >= m.texture_params.x); }
        let p0 = hit_position(hit, 0u);
        let ng0 = normalize(cross(hit_position(hit, 1u) - p0, hit_position(hit, 2u) - p0));
        let bw = 1.0 - hit.u - hit.v;
        var n = hit_normal(hit, 0u) * bw + hit_normal(hit, 1u) * hit.u + hit_normal(hit, 2u) * hit.v;
        n = select(normalize(n), ng0, dot(n, n) < 1e-8);
        if (m.maps[1].y != 0u) {
            let duv1 = tuv(k,1u,1u)-tuv(k,0u,1u); let duv2 = tuv(k,2u,1u)-tuv(k,0u,1u);
            let det = duv1.x*duv2.y-duv1.y*duv2.x;
            if (abs(det)>1e-8) {
                let tangent = ((hit_position(hit,1u)-p0)*duv2.y-(hit_position(hit,2u)-p0)*duv1.y)/det;
                let t = normalize(tangent-n*dot(n,tangent));
                let b = cross(n,t)*sign(det);
                var mapped = map_sample(m.maps[1], uv[1], m.borders[1]).xyz*2.0-1.0;
                mapped = vec3(mapped.xy*m.texture_params.z,mapped.z);
                n = normalize(t*mapped.x-b*mapped.y+n*mapped.z);
            }
        }
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
            ambient_diffuse = vec3(0.0);
            ambient_specular = vec3(1.0);
            let c = dielectric_crossing(s, m, d, o, p, n, v, entering, thr, in_sigma);
            d = c.d;
            o = c.o;
            thr = c.thr;
            in_sigma = c.in_sigma;
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
            var visible = 1.0;
            if (lt.size.y > 0.5 && m.extra.z > 0.5) { visible = visibility(p + ng * 1e-2, l, ls.w - 2e-2); }
            if (HAS_MEDIA && m.extra.z > 0.5) { visible *= volume_transmittance(p + ng * 1e-2, l, ls.w - 2e-2); }
            var c = thr * bsdf(s, n, v, l, light_lobes(lt)) * nl * rad * visible;
            // clamp rare fireflies from indirect paths
            if (bounce > 0u) { c = min(c, vec3(20.0)); }
            col += c;
        }
        // continue the path by sampling the BSDF
        let l = sample_dir(s, n, v);
        let pdf = pdf_of(s, n, v, l);
        let nl = dot(n, l);
        if (pdf <= 1e-8 || nl <= 0.0) { break; }
        let occlusion = 1.0 + m.texture_params.w * (map_sample(m.maps[3], uv[3], m.borders[3]).r - 1.0);
        let diffuse = bsdf(s, n, v, l, vec2(1.0, 0.0));
        let specular = bsdf(s, n, v, l, vec2(0.0, 1.0));
        let total = diffuse + specular;
        ambient_diffuse = diffuse / max(total, vec3(1e-20));
        ambient_specular = specular / max(total, vec3(1e-20));
        thr *= total * nl / pdf * occlusion;
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
    let w = pp.tile.z;
    let h = pp.tile.w;
    if (gid.x >= w || gid.y >= h) { return; }
    let pix = gid.y * w + gid.x;
    let global_xy = pp.tile.xy + gid.xy;
    let global_pix = global_xy.y * u32(pp.size.x) + global_xy.x;
    var sum = vec4(0.0);
    let start = u32(pp.size.z);
    let count = u32(pp.size.w);
    for (var s = 0u; s < count; s++) {
        let si = start + s;
        rng = pcg(global_pix * 9781u + pcg(si * 6271u + 1u));
        let jitter = vec2(rnd(), rnd());
        sum += radiance(vec2<f32>(global_xy) + jitter, pix, true);
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
    let w = i32(pp.tile.z);
    let h = i32(pp.tile.w);
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
    let w = pp.tile.z;
    let px = vec2<u32>(i.pos.xy) - pp.tile.xy;
    // `fin` holds per-pixel means when denoised, sums over the samples otherwise
    var c = fin[px.y * w + px.x] / select(max(pp.misc.y, 1.0), 1.0, pp.out.y > 0.5);
    var rgb = c.rgb * pp.misc.w;
    let a = clamp(c.a, 0.0, 1.0);
    if (pp.out.x > 0.5 && a > 0.0) {
        rgb = srgb_encode(rgb / a) * a;
    }
    return vec4(rgb, a);
}
