struct Material {
    base_color: vec4<f32>,
    // rgb, strength
    emissive: vec4<f32>,
    // metallic, roughness, opacity, alpha cutoff
    p0: vec4<f32>,
    // alpha mode (0 opaque, 1 mask, 2 blend), double sided, unlit, normal scale
    p1: vec4<f32>,
    // clearcoat, clearcoat roughness, transmission, ior
    p2: vec4<f32>,
    // thickness (scene units), attenuation distance (scene units, 0 = none), dispersion, specular
    p3: vec4<f32>,
    attenuation: vec4<f32>,
    // rgb, roughness
    sheen: vec4<f32>,
    specular_color: vec4<f32>,
    // factor, ior, thickness nm, occlusion strength
    irid: vec4<f32>,
    // strength, rotation (rad), displacement scale, map bits (1 base, 2 normal, 4 mr, 8 occ, 16 emissive, 32 displacement)
    aniso: vec4<f32>,
    // uv scale xy
    uv: vec4<f32>,
};

@group(1) @binding(0) var<uniform> mat: Material;
@group(1) @binding(1) var base_map: texture_2d<f32>;
@group(1) @binding(2) var normal_map: texture_2d<f32>;
@group(1) @binding(3) var mr_map: texture_2d<f32>;
@group(1) @binding(4) var occ_map: texture_2d<f32>;
@group(1) @binding(5) var emissive_map: texture_2d<f32>;
@group(1) @binding(6) var disp_map: texture_2d<f32>;
@group(1) @binding(7) var mat_smp: sampler;

struct Object {
    model: mat4x4<f32>,
    normal: mat4x4<f32>,
    // opacity, receive shadow, unused, unused
    params: vec4<f32>,
    // unused xyz, light view index for shadow passes
    spacing: vec4<f32>,
};

@group(2) @binding(0) var<uniform> obj: Object;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
};

fn displaced(v: VIn) -> vec3<f32> {
    var p = v.pos;
    if ((u32(mat.aniso.w) & 32u) != 0u) {
        let h = textureSampleLevel(disp_map, mat_smp, v.uv * mat.uv.xy, 0.0).r;
        p = p + normalize(v.normal) * h * mat.aniso.z;
    }
    return p;
}

@vertex
fn vs_main(v: VIn) -> VOut {
    var o: VOut;
    let local = displaced(v);
    let w = obj.model * vec4(local, 1.0);
    o.world = w.xyz;
    o.clip = fr.view_proj * w;
    o.normal = normalize((obj.normal * vec4(v.normal, 0.0)).xyz);
    o.tangent = vec4(normalize((obj.model * vec4(v.tangent.xyz, 0.0)).xyz), v.tangent.w);
    o.uv = v.uv * mat.uv.xy;
    return o;
}

// Shadow pass: obj.spacing.w selects the light view matrix.
@vertex
fn vs_shadow(v: VIn) -> @builtin(position) vec4<f32> {
    let local = displaced(v);
    let w = obj.model * vec4(local, 1.0);
    return shadow_mats[u32(obj.spacing.w)] * w;
}

// ---------------------------------------------------------------- BRDF pieces

fn d_ggx(nh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let f = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * f * f + 1e-7);
}

fn v_smith(nl: f32, nv: f32, a: f32) -> f32 {
    let a2 = a * a;
    let gv = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    let gl = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    return 0.5 / max(gv + gl, 1e-6);
}

// Anisotropic GGX (Burley) with tangent t and bitangent b.
fn d_aniso(nh: f32, th: f32, bh: f32, at: f32, ab: f32) -> f32 {
    let a2 = at * ab;
    let f = vec3(ab * th, at * bh, a2 * nh);
    let w2 = a2 / max(dot(f, f), 1e-7);
    return a2 * w2 * w2 / PI;
}

fn v_aniso(nl: f32, nv: f32, tv: f32, bv: f32, tl: f32, bl: f32, at: f32, ab: f32) -> f32 {
    let gv = nl * length(vec3(at * tv, ab * bv, nv));
    let gl = nv * length(vec3(at * tl, ab * bl, nl));
    return 0.5 / max(gv + gl, 1e-6);
}

fn f_schlick(f0: vec3<f32>, f90: vec3<f32>, vh: f32) -> vec3<f32> {
    return f0 + (f90 - f0) * pow(clamp(1.0 - vh, 0.0, 1.0), 5.0);
}

fn d_charlie(nh: f32, rough: f32) -> f32 {
    let a = max(rough * rough, 1e-3);
    let inv = 1.0 / a;
    let s2 = 1.0 - nh * nh;
    return (2.0 + inv) * pow(s2, inv * 0.5) / (2.0 * PI);
}

fn v_neubelt(nl: f32, nv: f32) -> f32 {
    return clamp(1.0 / (4.0 * (nl + nv - nl * nv)), 0.0, 1.0);
}

// Thin-film interference (Airy reflectance at 650/510/475 nm) blended over the base Fresnel.
fn iridescence(base_f: vec3<f32>, cos_i: f32, n_film: f32, n_base: f32, thickness: f32, metallic: f32) -> vec3<f32> {
    let sin_t2 = (1.0 - cos_i * cos_i) / (n_film * n_film);
    let cos_t = sqrt(max(1.0 - sin_t2, 0.0));
    let r12 = (cos_i - n_film * cos_t) / (cos_i + n_film * cos_t);
    let r23 = (n_film - n_base) / (n_film + n_base);
    let lambda = vec3(650.0, 510.0, 475.0);
    let phase = 4.0 * PI * n_film * thickness * cos_t / lambda;
    let c = cos(phase);
    let num = r12 * r12 + r23 * r23 + 2.0 * r12 * r23 * c;
    let den = 1.0 + r12 * r12 * r23 * r23 + 2.0 * r12 * r23 * c;
    let r = clamp(num / den, vec3(0.0), vec3(1.0));
    // dielectrics reflect the film; metals keep their colour, tinted by the film's spectrum
    let tint = r / max((r.x + r.y + r.z) / 3.0, 1e-4);
    return mix(r, clamp(base_f * tint, vec3(0.0), vec3(1.0)), metallic);
}

// ---------------------------------------------------------------- shadows

fn shadow_factor(li: Light, world: vec3<f32>, nrm: vec3<f32>) -> f32 {
    let first = i32(li.spot.z);
    if (first < 0 || obj.params.y < 0.5) { return 1.0; }
    var view = u32(first);
    if (u32(li.pos.w) == 1u && li.flags.w > 1.5) {
        // directional cascades: the first whose far split lies beyond this point
        let z = (fr.view * vec4(world, 1.0)).z;
        var c = 3u;
        if (z < li.size.x) { c = 0u; } else if (z < li.size.y) { c = 1u; } else if (z < li.size.z) { c = 2u; }
        view = view + c;
    } else if (li.flags.w > 1.5) {
        // cube: pick the face of the dominant axis of (world − light)
        let d = world - li.pos.xyz;
        let a = abs(d);
        var face = 0u;
        if (a.x >= a.y && a.x >= a.z) { face = select(1u, 0u, d.x > 0.0); }
        else if (a.y >= a.z) { face = select(3u, 2u, d.y > 0.0); }
        else { face = select(5u, 4u, d.z > 0.0); }
        view = view + face;
    }
    let p = shadow_mats[view] * vec4(world + nrm * 0.5, 1.0);
    if (p.w <= 0.0) { return 1.0; }
    let ndc = p.xyz / p.w;
    let uv = vec2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (any(uv < vec2(0.0)) || any(uv > vec2(1.0)) || ndc.z > 1.0) { return 1.0; }
    let dim = vec2<f32>(textureDimensions(shadow_tex));
    let soft = max(li.spot.w, 1.0);
    var sum = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let o = vec2(f32(x), f32(y)) * soft / dim;
            sum += textureSampleCompareLevel(shadow_tex, shadow_cmp, uv + o, i32(view), ndc.z - li.flags.z);
        }
    }
    return sum / 9.0;
}

// ---------------------------------------------------------------- lights

fn ies_factor(li: Light, l: vec3<f32>) -> f32 {
    let row = li.size.w;
    if (row < 0.0) { return 1.0; }
    // vertical angle from the light's axis, azimuth around it
    let axis = li.dir.xyz;
    let d = -l;
    let theta = acos(clamp(dot(d, axis), -1.0, 1.0)) / PI;
    let rgt = li.right.xyz;
    let up = cross(axis, rgt);
    let phi = fract(atan2(dot(d, up), dot(d, rgt)) / (2.0 * PI) + 1.0);
    let dim = vec2<f32>(textureDimensions(ies_tex));
    let rows_per = 32.0;
    let v = (row * rows_per + phi * (rows_per - 1.0) + 0.5) / dim.y;
    return textureSampleLevel(ies_tex, clamp_smp, vec2(theta, v), 0.0).r;
}

struct Surface {
    world: vec3<f32>,
    n: vec3<f32>,
    v: vec3<f32>,
    t: vec3<f32>,
    b: vec3<f32>,
    albedo: vec3<f32>,
    alpha: f32,
    metallic: f32,
    rough: f32,
    f0: vec3<f32>,
    f90: vec3<f32>,
    occlusion: f32,
    specular_weight: f32,
};

fn shade_light(li: Light, s: Surface) -> vec3<f32> {
    let ty = u32(li.pos.w);
    var l = vec3(0.0);
    var radiance = li.color.rgb;
    var a = s.rough * s.rough;
    if (ty == 0u) {
        // ambient (conventions 5.20): dielectrics take it diffusely, albedo × radiance; metals reflect it
        // as a uniform environment of this radiance (split sum, as the dome), in proportion to metalness
        let nv = max(dot(s.n, s.v), 1e-3);
        let lut = textureSampleLevel(brdf_lut, clamp_smp, vec2(nv, s.rough), 0.0).rg;
        let diff = s.albedo * (1.0 - s.metallic) * (1.0 - mat.p2.z) * li.flags.x;
        let spec = (s.f0 * lut.x + s.f90 * lut.y) * s.specular_weight * s.metallic * li.flags.y;
        return (diff + spec) * radiance * s.occlusion;
    }
    if (ty == 1u) {
        l = -li.dir.xyz;
    } else {
        var to = li.pos.xyz - s.world;
        if (ty == 6u || ty == 4u || ty == 5u) {
            // representative point on a sphere of the light's radius (area lights use their equivalent radius)
            let r = select(li.size.z, sqrt(li.size.x * li.size.y / PI), ty == 4u);
            let rad = select(r, li.size.z, ty == 6u);
            let refl = reflect(-s.v, s.n);
            let c = dot(to, refl) * refl - to;
            to = to + c * clamp(rad / max(length(c), 1e-4), 0.0, 1.0);
            a = clamp(a + rad / (2.0 * max(length(li.pos.xyz - s.world), 1.0)), 0.0, 1.0);
        }
        let dist = length(to);
        l = to / max(dist, 1e-4);
        let m = max(dist / 100.0, 0.01);
        var att = 1.0 / pow(m, li.color.w);
        if (li.dir.w > 0.0) {
            let q = dist / li.dir.w;
            att *= clamp(1.0 - q * q * q * q, 0.0, 1.0);
            att *= clamp(1.0 - q * q * q * q, 0.0, 1.0);
        }
        radiance = radiance * att;
        if (ty == 3u) {
            let cd = dot(-l, li.dir.xyz);
            radiance = radiance * smoothstep(li.spot.x, li.spot.y, cd);
        }
        if (ty == 4u || ty == 5u) {
            // one-sided emitters face along their direction
            radiance = radiance * max(dot(-l, li.dir.xyz), 0.0);
        }
        radiance = radiance * ies_factor(li, l);
    }
    let nl = dot(s.n, l);
    if (nl <= 0.0) { return vec3(0.0); }
    let nv = max(dot(s.n, s.v), 1e-4);
    let h = normalize(l + s.v);
    let nh = max(dot(s.n, h), 0.0);
    let vh = max(dot(s.v, h), 0.0);
    let f = f_schlick(s.f0, s.f90, vh);
    var spec = vec3(0.0);
    if (abs(mat.aniso.x) > 1e-3) {
        let at = mix(a, 1.0, mat.aniso.x * mat.aniso.x);
        let ab = max(a, 1e-3);
        let d = d_aniso(nh, dot(s.t, h), dot(s.b, h), at, ab);
        let vis = v_aniso(nl, nv, dot(s.t, s.v), dot(s.b, s.v), dot(s.t, l), dot(s.b, l), at, ab);
        spec = f * d * vis;
    } else {
        spec = f * d_ggx(nh, max(a, 1e-3)) * v_smith(nl, nv, max(a, 1e-3));
    }
    let kd = (vec3(1.0) - f) * (1.0 - s.metallic) * (1.0 - mat.p2.z);
    var out = (kd * s.albedo / PI) * li.flags.x + spec * s.specular_weight * li.flags.y;
    // sheen
    if (max(mat.sheen.r, max(mat.sheen.g, mat.sheen.b)) > 0.0) {
        out = out * (1.0 - 0.157 * max(mat.sheen.r, max(mat.sheen.g, mat.sheen.b)));
        out += mat.sheen.rgb * d_charlie(nh, mat.sheen.w) * v_neubelt(nl, nv);
    }
    // clearcoat on top
    if (mat.p2.x > 0.0) {
        let ca = max(mat.p2.y * mat.p2.y, 1e-3);
        let fc = f_schlick(vec3(0.04), vec3(1.0), vh).x * mat.p2.x;
        out = out * (1.0 - fc) + vec3(fc * d_ggx(nh, ca) * v_smith(nl, nv, ca));
    }
    return out * radiance * nl;
}

// Equirect coordinates of a world direction: the image's centre column is the dome's +z (conventions 5.5).
fn env_uv(dir: vec3<f32>) -> vec2<f32> {
    let d = normalize((fr.env_rot * vec4(dir, 0.0)).xyz);
    let u = (atan2(d.x, d.z) + PI) / (2.0 * PI);
    let v = acos(clamp(-d.y, -1.0, 1.0)) / PI;
    return vec2(u, v);
}

fn env_sample(dir: vec3<f32>, level: f32) -> vec3<f32> {
    return textureSampleLevel(env_tex, env_smp, env_uv(dir), level).rgb * fr.params.w;
}

fn sh_irradiance(n: vec3<f32>) -> vec3<f32> {
    // basis in the environment's own orientation (env.rs)
    let d = normalize((fr.env_rot * vec4(n, 0.0)).xyz);
    var r = fr.sh[0].rgb * 0.282095;
    r += fr.sh[1].rgb * 0.488603 * d.y + fr.sh[2].rgb * 0.488603 * d.z + fr.sh[3].rgb * 0.488603 * d.x;
    r += fr.sh[4].rgb * 1.092548 * d.x * d.y + fr.sh[5].rgb * 1.092548 * d.y * d.z + fr.sh[6].rgb * 0.315392 * (3.0 * d.z * d.z - 1.0);
    r += fr.sh[7].rgb * 1.092548 * d.x * d.z + fr.sh[8].rgb * 0.546274 * (d.x * d.x - d.y * d.y);
    return max(r, vec3(0.0)) * fr.params.w;
}

// ---------------------------------------------------------------- surface

fn surface(i: VOut, front: bool) -> Surface {
    var s: Surface;
    let bits = u32(mat.aniso.w);
    var base = mat.base_color;
    if ((bits & 1u) != 0u) { base = base * textureSample(base_map, mat_smp, i.uv); }
    s.albedo = base.rgb;
    s.alpha = base.a * mat.p0.z * obj.params.x;
    var metallic = mat.p0.x;
    var rough = mat.p0.y;
    if ((bits & 4u) != 0u) {
        let mr = textureSample(mr_map, mat_smp, i.uv);
        rough = rough * mr.g;
        metallic = metallic * mr.b;
    }
    s.metallic = clamp(metallic, 0.0, 1.0);
    s.rough = clamp(rough, 0.03, 1.0);
    var n = normalize(i.normal);
    if (!front) { n = -n; }
    var t = normalize(i.tangent.xyz - n * dot(n, i.tangent.xyz));
    var b = cross(n, t) * i.tangent.w;
    if ((bits & 2u) != 0u) {
        var tn = textureSample(normal_map, mat_smp, i.uv).xyz * 2.0 - 1.0;
        tn = vec3(tn.xy * mat.p1.w, tn.z);
        // glTF normal maps are y-up in texture space; scene space flips y (and z) relative to glTF
        n = normalize(t * tn.x - b * tn.y + n * tn.z);
        t = normalize(t - n * dot(n, t));
        b = cross(n, t) * i.tangent.w;
    }
    // anisotropy direction rotated in the tangent plane
    let ar = mat.aniso.y;
    s.t = normalize(t * cos(ar) + b * sin(ar));
    s.b = normalize(cross(n, s.t));
    s.n = n;
    s.world = i.world;
    s.v = normalize(fr.eye.xyz - i.world);
    s.occlusion = 1.0;
    if ((bits & 8u) != 0u) { s.occlusion = 1.0 + mat.irid.w * (textureSample(occ_map, mat_smp, i.uv).r - 1.0); }
    let ior = mat.p2.w;
    let f0d = pow((ior - 1.0) / (ior + 1.0), 2.0);
    let dielectric = min(vec3(f0d) * mat.specular_color.rgb, vec3(1.0));
    s.f0 = mix(dielectric, s.albedo, s.metallic);
    s.f90 = vec3(1.0);
    s.specular_weight = mix(mat.p3.w, 1.0, s.metallic);
    if (mat.irid.x > 0.0) {
        let nv = max(dot(s.n, s.v), 1e-3);
        s.f0 = mix(s.f0, iridescence(s.f0, nv, mat.irid.y, ior, mat.irid.z, s.metallic), mat.irid.x);
    }
    return s;
}

fn refracted_uv(s: Surface, ior: f32) -> vec2<f32> {
    let dir = refract(-s.v, s.n, 1.0 / ior);
    let p = fr.view_proj * vec4(s.world + dir * mat.p3.x, 1.0);
    return vec2(p.x / p.w * 0.5 + 0.5, 0.5 - p.y / p.w * 0.5);
}

fn transmission(s: Surface) -> vec3<f32> {
    // refract through a slab of the material's thickness and read the scene behind at the exit point
    let ior = mat.p2.w;
    let levels = f32(textureNumLevels(scene_color));
    let lod = s.rough * s.rough * clamp(ior * 2.0 - 2.0, 0.0, 1.0) * (levels - 1.0) * 1.5;
    var col = textureSampleLevel(scene_color, clamp_smp, refracted_uv(s, ior), lod).rgb;
    if (mat.p3.z > 0.0) {
        // dispersion: red and blue refract with the spread indices of KHR_materials_dispersion
        let spread = (ior - 1.0) * 0.025 * mat.p3.z;
        col.r = textureSampleLevel(scene_color, clamp_smp, refracted_uv(s, ior - spread), lod).r;
        col.b = textureSampleLevel(scene_color, clamp_smp, refracted_uv(s, ior + spread), lod).b;
    }
    // Beer–Lambert through the thickness
    if (mat.p3.y > 0.0) {
        let sigma = -log(max(mat.attenuation.rgb, vec3(1e-4))) / mat.p3.y;
        col = col * exp(-sigma * mat.p3.x);
    }
    return col * s.albedo;
}

struct FOut {
    @location(0) color: vec4<f32>,
};

/// Screen-space contact shadow: marches from the surface toward the light for `len` scene
/// units against the prepass depth; 0 when something in front of the ray's path hides it.
fn contact_shadow(world: vec3<f32>, l: vec3<f32>, len: f32) -> f32 {
    let dims = vec2<f32>(textureDimensions(gb_depth));
    let thick = max(len * 0.35, 2.0);
    let steps = 16;
    for (var k = 1; k <= steps; k++) {
        let p = world + l * (len * f32(k) / f32(steps));
        let c = fr.view_proj * vec4(p, 1.0);
        if (c.w <= 0.0) { break; }
        let ndc = c.xy / c.w;
        let uv = vec2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if (any(uv < vec2(0.0)) || any(uv >= vec2(1.0))) { break; }
        let scene_z = textureLoad(gb_depth, vec2<i32>(uv * dims), 0).r;
        let ray_z = (fr.view * vec4(p, 1.0)).z;
        let d = ray_z - scene_z;
        if (scene_z > 0.0 && d > 0.5 && d < thick) { return 0.0; }
    }
    return 1.0;
}

@fragment
fn fs_main(i: VOut, @builtin(front_facing) front: bool) -> FOut {
    var o: FOut;
    let s = surface(i, front);
    let mode = u32(mat.p1.x);
    if (mode == 1u && s.alpha < mat.p0.w) { discard; }
    if (mat.p1.z > 0.5) {
        // unlit
        let a = select(1.0, s.alpha, mode == 2u);
        o.color = vec4(s.albedo * a, a);
        return o;
    }
    var col = vec3(0.0);
    // screen-space ambient occlusion darkens the ambient and environment light only
    var ao = 1.0;
    if (fr.lens.z > 0.5) { ao = textureLoad(ao_tex, vec2<i32>(i.clip.xy), 0).r; }
    let tile = vec2<u32>(i.clip.xy) / TILE;
    let tiles_x = u32(fr.params.z);
    let base = (tile.y * tiles_x + tile.x) * (MAX_PER_TILE + 1u);
    let count = min(tiles[base], MAX_PER_TILE);
    for (var k = 0u; k < count; k++) {
        let li = lights[tiles[base + 1u + k]];
        let ty = u32(li.pos.w);
        var sh = 1.0;
        if (ty != 0u) {
            sh = shadow_factor(li, s.world, s.n);
            if (li.right.w > 0.0 && sh > 0.0) {
                var l = -li.dir.xyz;
                if (ty != 1u) { l = normalize(li.pos.xyz - s.world); }
                sh = sh * contact_shadow(s.world + s.n * 0.5, l, li.right.w);
            }
        } else {
            sh = ao;
        }
        col += shade_light(li, s) * sh;
    }
    // image-based lighting from the dome
    if (fr.params2.z > 0.5) {
        let nv = max(dot(s.n, s.v), 1e-3);
        let lut = textureSampleLevel(brdf_lut, clamp_smp, vec2(nv, s.rough), 0.0).rg;
        let r = reflect(-s.v, s.n);
        let pre = env_sample(r, s.rough * (fr.params2.w - 1.0));
        let spec = pre * (s.f0 * lut.x + s.f90 * lut.y) * s.specular_weight;
        let fr_avg = s.f0 + (1.0 - s.f0) * pow(1.0 - nv, 5.0);
        let diff = sh_irradiance(s.n) * s.albedo * (1.0 - s.metallic) * (1.0 - mat.p2.z) * (vec3(1.0) - fr_avg);
        var ibl = (diff + spec) * s.occlusion * ao;
        if (mat.p2.x > 0.0) {
            let fc = (0.04 + 0.96 * pow(1.0 - nv, 5.0)) * mat.p2.x;
            ibl = ibl * (1.0 - fc) + env_sample(r, mat.p2.y * (fr.params2.w - 1.0)) * fc;
        }
        col += ibl;
    }
    if (mat.p2.z > 0.0) {
        let nv = max(dot(s.n, s.v), 1e-3);
        let ft = vec3(1.0) - f_schlick(s.f0, s.f90, nv);
        col += transmission(s) * ft * mat.p2.z * (1.0 - s.metallic);
    }
    var em = mat.emissive.rgb * mat.emissive.w;
    if ((u32(mat.aniso.w) & 16u) != 0u) { em = em * textureSample(emissive_map, mat_smp, i.uv).rgb; }
    col += em;
    // transmissive surfaces are opaque layers over what they refract
    var a = 1.0;
    if (mode == 2u) { a = s.alpha; }
    else { a = select(1.0, s.alpha, obj.params.x < 1.0); }
    o.color = vec4(col * a, a);
    return o;
}

// ---------------------------------------------------------------- depth and normal prepass

struct GOut {
    // view-space normal, roughness
    @location(0) nr: vec4<f32>,
    // view depth, reflectance (split-sum specular albedo × specular weight), unused ×2
    @location(1) zr: vec4<f32>,
};

@fragment
fn fs_prepass(i: VOut, @builtin(front_facing) front: bool) -> GOut {
    var o: GOut;
    let s = surface(i, front);
    let mode = u32(mat.p1.x);
    if (mode == 1u && s.alpha < mat.p0.w) { discard; }
    let nv = max(dot(s.n, s.v), 1e-3);
    let lut = textureSampleLevel(brdf_lut, clamp_smp, vec2(nv, s.rough), 0.0).rg;
    let spec = (s.f0 * lut.x + s.f90 * lut.y) * s.specular_weight;
    let refl = select(dot(spec, vec3(0.2126, 0.7152, 0.0722)), 0.0, mat.p1.z > 0.5);
    o.nr = vec4(normalize((fr.view * vec4(s.n, 0.0)).xyz), s.rough);
    o.zr = vec4((fr.view * vec4(s.world, 1.0)).z, refl, 0.0, 0.0);
    return o;
}

// ---------------------------------------------------------------- dome background

struct BgOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> BgOut {
    var o: BgOut;
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u)) * 2.0 - 1.0;
    // background at the far plane (reverse Z: depth 0)
    o.pos = vec4(p, 0.0, 1.0);
    o.ndc = p;
    return o;
}

@fragment
fn fs_dome(i: BgOut) -> @location(0) vec4<f32> {
    let far = fr.inv_view_proj * vec4(i.ndc, 0.0001, 1.0);
    let dir = far.xyz / far.w - fr.eye.xyz;
    return vec4(textureSampleLevel(sky_tex, env_smp, env_uv(dir), 0.0).rgb * fr.params.w, 1.0);
}

