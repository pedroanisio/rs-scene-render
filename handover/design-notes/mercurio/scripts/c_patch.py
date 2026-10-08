import re
p='crates/sr-gpu/src/pathtrace.wgsl'
s=open(p).read()

helpers='''
// ---------------------------------------------------------------- lights seen through a refracting surface

struct Iface { found: bool, t: f32, n: vec3<f32>, ior: f32, trans: f32, tint: vec3<f32> };

fn fresnel_schlick(cosi: f32, eta: f32) -> f32 {
    let r0 = (1.0 - eta) / (1.0 + eta);
    return r0 * r0 + (1.0 - r0 * r0) * pow(1.0 - clamp(cosi, 0.0, 1.0), 5.0);
}

// The first transmissive surface along the ray, stepping over everything else: its distance, its
// normal facing the ray, its refractive index, transmission and tint.
fn first_interface(o: vec3<f32>, d: vec3<f32>, dist: f32) -> Iface {
    var out: Iface;
    out.found = false; out.t = 0.0; out.n = vec3(0.0, 1.0, 0.0); out.ior = 1.0; out.trans = 0.0; out.tint = vec3(1.0);
    var org = o; var remaining = dist; var travelled = 0.0;
    for (var step = 0u; step < 16u; step++) {
        let hit = trace(org, d, remaining);
        if (hit.tri == 0xffffffffu) { return out; }
        if (tmat_of(hit.tri) != 0xffffffffu) {
            let m = mats[hit_material(hit)];
            if (m.params.z > 0.0) {
                let p0 = hit_position(hit, 0u);
                let ng0 = normalize(cross(hit_position(hit, 1u) - p0, hit_position(hit, 2u) - p0));
                out.found = true; out.t = travelled + hit.t; out.n = select(-ng0, ng0, dot(d, ng0) < 0.0);
                out.ior = m.params.w; out.trans = m.params.z; out.tint = m.base.rgb;
                return out;
            }
        }
        let adv = hit.t + 1e-3;
        org += d * adv; remaining -= adv; travelled += adv;
        if (remaining <= 0.0) { return out; }
    }
    return out;
}

struct Seen { dir: vec3<f32>, vis: vec3<f32> };

// A light as a surface under a refracting surface sees it. `l` is the straight direction to the
// light sample, `ldist` its distance. The shadow ray is refracted at the interface (found by trace,
// refined once from the first guess) so that it leaves toward the light: `dir` is its direction at
// the surface (what the BSDF is evaluated with) and `vis` carries everything the light loses on
// the way: the blockers on both sides, the interface's tint, transmission and Fresnel
// transmittance, the radiance scale cos(theta_air) / eta^2 that goes with the solid angle the
// interface changes, and the absorption along the path inside. A path whose refinement finds no
// interface or no way out toward the light gets no light from it (vis = 0), never a bright guess.
fn light_through(p: vec3<f32>, ng: vec3<f32>, n: vec3<f32>, l: vec3<f32>, ldist: f32, sigma: vec3<f32>) -> Seen {
    var out: Seen;
    out.dir = l; out.vis = vec3(0.0);
    let start = p + ng * 1e-2;
    let directional = ldist > 1e20;
    let at_light = start + l * min(ldist, 1e6);
    var dir = l;
    var iface = first_interface(start, dir, ldist - 2e-2);
    if (!iface.found) { out.vis = vec3(visibility(start, l, ldist - 2e-2)); return out; }
    let q0 = start + dir * iface.t;
    var la = l;
    if (!directional) { la = normalize(at_light - q0); }
    let w = refract(-la, -iface.n, 1.0 / iface.ior);
    if (dot(w, w) < 1e-8) { return out; }
    dir = -normalize(w);
    iface = first_interface(start, dir, ldist - 2e-2);
    if (!iface.found) { return out; }
    let q = start + dir * iface.t;
    var rest = 1e30;
    if (!directional) { let to = at_light - q; rest = length(to); la = to / max(rest, 1e-4); }
    let n_air = -iface.n;
    let cos_air = dot(la, n_air);
    if (cos_air <= 1e-4 || dot(n, dir) <= 1e-4) { return out; }
    let inside_view = visibility(start, dir, iface.t - 2e-3);
    let outside_view = visibility(q + n_air * 1e-2, la, select(rest - 2e-2, 1e30, directional));
    let fresnel = fresnel_schlick(cos_air, 1.0 / iface.ior);
    let carried = inside_view * outside_view * iface.trans * (1.0 - fresnel) * cos_air / (iface.ior * iface.ior);
    out.dir = dir;
    out.vis = min(carried, vec3(1.0)) * iface.tint * exp(-sigma * iface.t);
    return out;
}
'''
anchor="// ---------------------------------------------------------------- one path"
assert anchor in s
s=s.replace(anchor,helpers+"\n"+anchor,1)

s=s.replace("    var in_sigma = vec3(0.0);\n","    var in_sigma = vec3(0.0);\n    var inside = false;\n",1)
old="                if (WATER) { in_sigma = select(vec3(0.0), m.attenuation.rgb, entering); }\n"
assert old in s
s=s.replace(old,"                if (WATER) { in_sigma = select(vec3(0.0), m.attenuation.rgb, entering); inside = entering; }\n",1)

old="""            var visible = 1.0;
            if (lt.size.y > 0.5 && m.extra.z > 0.5) { visible = visibility(p + ng * 1e-2, l, ls.w - 2e-2); }"""
assert old in s
new="""            if (WATER && inside && lt.size.y > 0.5 && m.extra.z > 0.5) {
                // a surface under water (or glass): the light reaches it refracted
                let seen = light_through(p, ng, n, l, ls.w, in_sigma);
                var c = thr * bsdf(s, n, v, seen.dir, light_lobes(lt)) * rad * seen.vis;
                if (bounce > 0u) { c = min(c, vec3(20.0)); }
                col += c;
                continue;
            }
            var visible = 1.0;
            if (lt.size.y > 0.5 && m.extra.z > 0.5) { visible = visibility(p + ng * 1e-2, l, ls.w - 2e-2); }"""
s=s.replace(old,new,1)
open(p,'w').write(s)
print("ok")
