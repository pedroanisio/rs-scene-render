// Lights seen through a refracting surface (see water_source in pathtrace.rs): appended to the path
// tracer's shader, with the three hooks it replaces, only for scenes that have a transmissive
// material, so the shader of every other scene is unchanged.

// ---------------------------------------------------------------- lights seen through a refracting surface

// Foam is mixed into some surface's material (see the hook of surf_of in water_source).
override FOAM: bool = false;

struct Iface { found: bool, t: f32, n: vec3<f32>, ior: f32, trans: f32, tint: vec3<f32>, entering: bool, sigma: vec3<f32> };

fn fresnel_schlick(cosi: f32, eta: f32) -> f32 {
    let r0 = (1.0 - eta) / (1.0 + eta);
    return r0 * r0 + (1.0 - r0 * r0) * pow(1.0 - clamp(cosi, 0.0, 1.0), 5.0);
}

// The first transmissive surface along the ray, stepping over everything else: its distance, its
// normal facing the ray, its refractive index, transmission and tint, whether the ray enters the
// medium it encloses, and that medium's absorption.
fn first_interface(o: vec3<f32>, d: vec3<f32>, dist: f32) -> Iface {
    var out: Iface;
    out.found = false; out.t = 0.0; out.n = vec3(0.0, 1.0, 0.0); out.ior = 1.0; out.trans = 0.0; out.tint = vec3(1.0);
    out.entering = true; out.sigma = vec3(0.0);
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
                out.entering = dot(d, ng0) < 0.0; out.sigma = m.attenuation.rgb;
                if (FOAM && m.extra.w > 0.5) {
                    // foam is opaque and white where it covers the water: less light gets through
                    let bw = 1.0 - hit.u - hit.v;
                    let foam = clamp(tverts[hit.tri * 24u + 2u].a * bw + tverts[hit.tri * 24u + 8u].a * hit.u + tverts[hit.tri * 24u + 14u].a * hit.v, 0.0, 1.0);
                    out.trans = out.trans * (1.0 - foam);
                }
                return out;
            }
        }
        let adv = hit.t + 1e-3;
        org += d * adv; remaining -= adv; travelled += adv;
        if (remaining <= 0.0) { return out; }
    }
    return out;
}

// What a surface sees of a light through the interface above it: the direction of the light at the
// surface (`dir`) and everything it loses except the media between them (`vis`), and the two legs
// of the path the media must be crossed along: `inside` units from the surface along `dir` to the
// interface point `q`, then `outside` units from `q` along `la` toward the light.
struct Seen { dir: vec3<f32>, vis: vec3<f32>, inside: f32, q: vec3<f32>, la: vec3<f32>, outside: f32 };

// A light as a surface under a refracting surface sees it. `l` is the straight direction to the
// light sample, `ldist` its distance. The shadow ray is refracted at the interface (found by trace,
// refined until it settles) so that it leaves toward the light: `dir` is its direction at
// the surface (what the BSDF is evaluated with) and `vis` carries everything the light loses on
// the way: the blockers on both sides, the interface's tint, transmission and Fresnel
// transmittance, the factor cos(theta_air) / cos(theta_water) that goes with the cross-section the
// interface changes, and the absorption along the path inside. The radiance of the light is eta^2
// larger in the water and the solid angle it subtends eta^2 smaller: they cancel, and the paths
// that cross the interface carry the eta^2 themselves (see water_source).
// The cosine between the surface and `dir` is not in it: the caller multiplies by it. `shadows`
// is false for a light that casts none or a surface that receives none: the two blocker tests are
// skipped, the refraction and the interface's losses are not. A path whose refinement finds no
// interface or no way out toward the light gets no light from it (vis = 0), never a bright guess.
fn light_through(p: vec3<f32>, ng: vec3<f32>, n: vec3<f32>, l: vec3<f32>, ldist: f32, sigma: vec3<f32>, shadows: bool) -> Seen {
    var out: Seen;
    out.dir = l; out.vis = vec3(0.0); out.inside = 0.0; out.q = p + ng * 1e-2; out.la = l; out.outside = ldist - 2e-2;
    let start = p + ng * 1e-2;
    let directional = ldist > 1e20;
    let at_light = start + l * min(ldist, 1e6);
    var dir = l;
    var iface = first_interface(start, dir, ldist - 2e-2);
    if (!iface.found) { out.vis = vec3(select(1.0, visibility(start, l, ldist - 2e-2), shadows)); return out; }
    // Find the point of the interface where the refracted ray from the surface leaves toward the
    // light: refract at the point the current direction meets, trace the new direction, and repeat
    // until the direction found at the point it meets is the one that was traced. The first steps go
    // the whole way, which settles a flat interface at once; a step that made the agreement worse
    // is halved from then on, because the whole step diverges where the surface bends more than
    // the water is deep. A direction that is still more than 8 degrees from the
    // one refracted at the point it meets gets no light, never a bright guess.
    var la = l;
    var q = start;
    var agreement = 0.0;
    var previous = -1.0;
    var step = 1.0;
    for (var it = 0u; it < 8u; it++) {
        q = start + dir * iface.t;
        if (!directional) { la = normalize(at_light - q); }
        let w = refract(-la, -iface.n, 1.0 / iface.ior);
        if (dot(w, w) < 1e-8) { return out; }
        let next = -normalize(w);
        agreement = dot(next, dir);
        if (agreement > 0.99999) { break; }
        // a step that made it worse is halved from then on
        if (agreement < previous) { step = max(0.5 * step, 0.25); }
        previous = agreement;
        dir = normalize(dir + step * (next - dir));
        iface = first_interface(start, dir, ldist - 2e-2);
        if (!iface.found) { return out; }
    }
    if (agreement < 0.99) { return out; }
    var rest = 1e30;
    if (!directional) { let to = at_light - q; rest = length(to); la = to / max(rest, 1e-4); }
    let n_air = -iface.n;
    let cos_air = dot(la, n_air);
    if (cos_air <= 1e-4 || dot(n, dir) <= 1e-4) { return out; }
    var inside_view = 1.0;
    var outside_view = 1.0;
    if (shadows) {
        inside_view = visibility(start, dir, iface.t - 2e-3);
        outside_view = visibility(q + n_air * 1e-2, la, select(rest - 2e-2, 1e30, directional));
    }
    let fresnel = fresnel_schlick(cos_air, 1.0 / iface.ior);
    // The beam's power per area of the interface is T cos_air; inside it spreads over a cross-section
    // of cos_water. The cosine at the surface is the caller's.
    let cos_water = max(-dot(dir, iface.n), 1e-4);
    let carried = inside_view * outside_view * iface.trans * (1.0 - fresnel) * cos_air / cos_water;
    out.dir = dir;
    out.inside = iface.t;
    out.q = q + n_air * 1e-2;
    out.la = la;
    out.outside = select(rest - 2e-2, 1e30, directional);
    out.vis = min(carried, 1.0) * iface.tint * exp(-sigma * iface.t);
    return out;
}

// ---------------------------------------------------------------- media in absorbing water

struct Transport { color: vec3<f32>, trans: vec3<f32>, open: f32 };

// volume_transport for a ray inside absorbing water (`sigma` per unit, per channel): the light the
// media add at a distance is dimmed by the water between there and the ray's origin, not by the whole
// ray at once, and the water absorbs across the gaps between the media too. `open` is the media's own
// transmittance (the coverage the pass reports). The march follows volume_transport step by step;
// with `sigma` zero it gives the same colour and transmittance.
fn water_transport(o: vec3<f32>, d: vec3<f32>, distance: f32, sigma_water: vec3<f32>) -> Transport {
    var color = vec3(0.0); var trans = vec3(1.0); var open = 1.0; var cursor = 0.0;
    for (var boundary = 0u; boundary <= pp.media.y * 2u; boundary++) {
        var next = distance; var step = 1e30; var occupied = false;
        for (var i = 0u; i < pp.media.y; i++) {
            let base = volume_base(i); let interval = volume_interval(base, o, d, distance);
            if (interval.y <= interval.x || interval.y <= cursor) { continue; }
            if (interval.x > cursor) { next = min(next, interval.x); }
            else { next = min(next, interval.y); step = min(step, tverts[base + 12u].w); occupied = true; }
        }
        if (next <= cursor) { break; }
        if (occupied) {
            let n = max(1u, u32(ceil((next - cursor) / step))); let ds = (next - cursor) / f32(n);
            for (var k = 0u; k < n; k++) {
                let point = o + d * (cursor + (f32(k) + 0.5) * ds);
                var sigma = 0.0; var source = vec3(0.0);
                for (var i = 0u; i < pp.media.y; i++) {
                    let base = volume_base(i); let density = volume_density(base, point);
                    if (density <= 0.0) { continue; }
                    let extinction = density * tverts[base + 5u].w;
                    sigma += extinction; source += density * volume_emission(base, point);
                    let albedo = tverts[base + 6u];
                    if (MEDIUM_LIGHTING && any(albedo.rgb > vec3(0.0))) { source += extinction * albedo.rgb * volume_incident(point, -d, albedo.w, tverts[base + 13u].y > 0.5); }
                }
                let total = vec3(sigma) + sigma_water;
                let optical_depth = total * ds;
                let attenuation = exp(-optical_depth);
                var weight = ds * (vec3(1.0) - 0.5 * optical_depth + optical_depth * optical_depth / 6.0);
                let exact = (vec3(1.0) - attenuation) / max(total, vec3(1e-30));
                weight = select(weight, exact, optical_depth > vec3(1e-3));
                color += trans * source * weight; trans *= attenuation; open *= exp(-sigma * ds);
            }
        } else {
            trans *= exp(-sigma_water * min(next - cursor, 1e4));
        }
        cursor = next;
        if (cursor >= distance) { break; }
    }
    return Transport(color, trans, open);
}
