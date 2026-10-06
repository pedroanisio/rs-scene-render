// Null geodesics of the Schwarzschild metric (G = c = 1), traced from the camera backwards.
//
// A ray from a static observer at radius r_o stays in the plane of the observer, the hole and the
// ray, where u = 1/r follows the orbit equation u'' = -u + 3 M u^2 in the angle phi measured in
// that plane from the observer's direction (Binet's form). The ray is integrated with fixed-step
// RK4 in phi; its last step before each crossing of the disk's plane is shortened to land on it
// (the crossings are at phi0 + k pi, with phi0 in closed form from the plane's orientation).
// Three outcomes: the ray falls through the horizon (black), escapes (its asymptotic direction
// indexes a star field), or crosses the disk's plane within [r_in, r_out], where the disk, thin
// and opaque, emits as a blackbody of the local temperature times the redshift factor g.

struct Params {
    eye: vec4<f32>,   // xyz observer, w = mass
    hole: vec4<f32>,  // xyz centre of the hole, w = inner radius of the disk
    right: vec4<f32>, // xyz camera right, w = focal length in pixels
    down: vec4<f32>,  // xyz camera image-down, w = samples per pixel
    fwd: vec4<f32>,   // xyz camera forward, w = output mode
    dx: vec4<f32>,    // xyz first axis of the disk's plane, w = outer radius
    dy: vec4<f32>,    // xyz second axis of the disk's plane, w = peak temperature (K)
    dz: vec4<f32>,    // xyz axis of the disk's spin, w = intensity
    size: vec4<f32>,  // width, height, pattern contrast, geometric time
    misc: vec4<u32>,  // pattern (0 none, 1 clumps, 2 spiral), seed, flags, star seed
}

@group(0) @binding(0) var<uniform> P: Params;
// visible blackbody radiance (linear sRGB) at 50000 K * (i / 1024)^2
@group(0) @binding(1) var<uniform> BB: array<vec4<f32>, 1025>;

const PI: f32 = 3.14159265358979;
const TAU: f32 = 6.28318530717959;
const STEP: f32 = 0.02;
const MAX_STEPS: u32 = 4096u;
// flags: bit 0 keeps tracing past the first crossing inside the disk (all crossings are reported)
const FLAG_ALL: u32 = 1u;

struct Trace {
    kind: u32,      // 0 captured, 1 escaped
    phi_inf: f32,   // phi where an escaping ray reaches u = 0
    n: u32,         // crossings recorded
    r: vec4<f32>,   // radius of crossing k, or -1
    hit: i32,       // first crossing within the disk, or -1
}

fn accel(u: f32, m: f32) -> f32 {
    return -u + 3.0 * m * u * u;
}

// phi0 in (0, pi]: the first angle at which the ray's plane meets the disk's plane (never reached
// when the two coincide)
fn first_crossing(e1: vec3<f32>, e2: vec3<f32>, z: vec3<f32>) -> f32 {
    let a = dot(e1, z);
    let b = dot(e2, z);
    if (a * a + b * b < 1e-12) { return 1e30; }
    var phi0 = atan2(-a, b);
    if (phi0 < 0.0) { phi0 += PI; }
    if (phi0 < 1e-6) { phi0 += PI; }
    return phi0;
}

fn crossing_radius(t: Trace, k: i32) -> f32 {
    if (k == 0) { return t.r.x; }
    if (k == 1) { return t.r.y; }
    if (k == 2) { return t.r.z; }
    return t.r.w;
}

fn trace(m: f32, r_o: f32, b: f32, ingoing: bool, phi0: f32, r_in: f32, r_out: f32, all: bool) -> Trace {
    var t: Trace;
    t.kind = 0u;
    t.phi_inf = 0.0;
    t.n = 0u;
    t.r = vec4<f32>(-1.0);
    t.hit = -1;
    let u0 = 1.0 / r_o;
    var u = u0;
    var w = sqrt(max(1.0 / (b * b) - u0 * u0 + 2.0 * m * u0 * u0 * u0, 0.0));
    if (!ingoing) { w = -w; }
    var phi = 0.0;
    var k = 0u;
    let horizon = 0.5 / m;
    for (var i = 0u; i < MAX_STEPS; i++) {
        var h = STEP;
        var landing = false;
        var target_phi = 0.0;
        if (k < 4u) {
            target_phi = phi0 + f32(k) * PI;
            if (target_phi - phi <= STEP) {
                h = target_phi - phi;
                landing = true;
            }
        }
        let k1u = w;
        let k1w = accel(u, m);
        let k2u = w + 0.5 * h * k1w;
        let k2w = accel(u + 0.5 * h * k1u, m);
        let k3u = w + 0.5 * h * k2w;
        let k3w = accel(u + 0.5 * h * k2u, m);
        let k4u = w + h * k3w;
        let k4w = accel(u + h * k3u, m);
        let un = u + h / 6.0 * (k1u + 2.0 * k2u + 2.0 * k3u + k4u);
        let wn = w + h / 6.0 * (k1w + 2.0 * k2w + 2.0 * k3w + k4w);
        if (un >= horizon) {
            t.kind = 0u;
            return t;
        }
        if (un <= 0.0) {
            t.kind = 1u;
            t.phi_inf = phi + h * u / (u - un);
            return t;
        }
        u = un;
        w = wn;
        phi += h;
        if (landing) {
            phi = target_phi;
            let r = 1.0 / u;
            if (k == 0u) { t.r.x = r; } else if (k == 1u) { t.r.y = r; } else if (k == 2u) { t.r.z = r; } else { t.r.w = r; }
            t.n = k + 1u;
            if (t.hit < 0 && r >= r_in && r <= r_out) {
                t.hit = i32(k);
                if (!all) { return t; }
            }
            k += 1u;
        }
    }
    t.kind = 0u;
    return t;
}

// ---------------------------------------------------------------- hashes and the star field

fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let word = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (word >> 22u) ^ word;
}

fn hash3(a: u32, b: u32, c: u32) -> u32 {
    return pcg(a ^ pcg(b + pcg(c)));
}

fn unit(h: u32) -> f32 {
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

const STAR_CELLS: f32 = 512.0;
const STAR_RATE: f32 = 0.006;

// A star field on the sphere of directions: cube-face cells, a star in a few of them.
fn stars(dir: vec3<f32>, pixel_angle: f32) -> vec3<f32> {
    let a = abs(dir);
    var face = 0u;
    var major = a.x;
    var uv = dir.yz;
    if (a.x >= a.y && a.x >= a.z) {
        face = select(0u, 1u, dir.x < 0.0);
    } else if (a.y >= a.z) {
        face = select(2u, 3u, dir.y < 0.0);
        major = a.y;
        uv = dir.xz;
    } else {
        face = select(4u, 5u, dir.z < 0.0);
        major = a.z;
        uv = dir.xy;
    }
    let st = (uv / major * 0.5 + 0.5) * STAR_CELLS;
    let cell = floor(st);
    let local = st - cell;
    let h = hash3(face + P.misc.w * 8u, u32(cell.x), u32(cell.y));
    if (unit(h) >= STAR_RATE) { return vec3<f32>(0.0); }
    let centre = vec2<f32>(0.3 + 0.4 * unit(pcg(h + 1u)), 0.3 + 0.4 * unit(pcg(h + 2u)));
    // a cell spans about (pi / 2) / STAR_CELLS radians near the middle of its face
    let cell_angle = 1.5707963 / STAR_CELLS;
    let radius = clamp(0.6 * pixel_angle / cell_angle, 0.02, 0.3);
    let d = length(local - centre);
    if (d >= radius) { return vec3<f32>(0.0); }
    let bright = 0.25 + 1.5 * pow(unit(pcg(h + 3u)), 3.0);
    let tint = unit(pcg(h + 4u));
    let col = mix(vec3<f32>(0.62, 0.74, 1.0), vec3<f32>(1.0, 0.86, 0.7), tint);
    return col * bright;
}

// ---------------------------------------------------------------- the disk

// Temperature profile of a thin disk with no torque at the inner edge: T ~ x^(-3/4) (1 - x^(-1/2))^(1/4)
// with x = r / r_in, normalized to 1 at its maximum, x = 49/36.
fn profile(x: f32) -> f32 {
    if (x <= 1.0) { return 0.0; }
    return pow(x, -0.75) * pow(1.0 - inverseSqrt(x), 0.25) / 0.4880;
}

fn value_noise(psi: f32, s: f32, cells: f32, ds: f32, seed: u32) -> f32 {
    let x = psi / TAU * cells;
    let y = s / ds;
    let xi = floor(x);
    let yi = floor(y);
    let fx = x - xi;
    let fy = y - yi;
    let n = u32(cells);
    let x0 = u32(xi) % n;
    let x1 = (x0 + 1u) % n;
    let y0 = u32(i32(yi) + 4096);
    let a = unit(hash3(seed, x0, y0));
    let b = unit(hash3(seed, x1, y0));
    let c = unit(hash3(seed, x0, y0 + 1u));
    let d = unit(hash3(seed, x1, y0 + 1u));
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    return mix(mix(a, b, sx), mix(c, d, sx), sy);
}

// Brightness modulation of the disk at radius r and azimuth psi, carried round by the Keplerian rate.
fn pattern(r: f32, psi: f32, m: f32) -> f32 {
    let kind = P.misc.x;
    if (kind == 0u) { return 1.0; }
    let tau = P.size.w;
    let omega = sqrt(m / (r * r * r));
    var a = psi - omega * tau;
    a = a - TAU * floor(a / TAU);
    let s = log(r / P.hole.w);
    let contrast = P.size.z;
    if (kind == 1u) {
        let n = 0.65 * value_noise(a, s, 32.0, 0.18, P.misc.y) + 0.35 * value_noise(a, s, 64.0, 0.09, P.misc.y + 7u);
        return exp(contrast * 3.0 * (n - 0.5));
    }
    let arms = f32(2u + P.misc.y % 3u);
    let phase = TAU * unit(pcg(P.misc.y + 11u));
    return exp(contrast * 1.5 * cos(arms * a + 3.0 * s + phase));
}

fn blackbody(kelvin: f32) -> vec3<f32> {
    let x = 1024.0 * sqrt(clamp(kelvin, 0.0, 50000.0) / 50000.0);
    let i = min(u32(x), 1023u);
    let f = x - f32(i);
    return mix(BB[i].xyz, BB[i + 1u].xyz, f);
}

struct Disk {
    g: f32,
    r: f32,
    psi: f32,
    color: vec3<f32>,
}

fn disk_at(m: f32, r: f32, b: f32, phi_k: f32, e1: vec3<f32>, e2: vec3<f32>) -> Disk {
    var d: Disk;
    d.r = r;
    let z = P.dz.xyz;
    let hz = dot(cross(e1, e2), z);
    let omega = sqrt(m / (r * r * r));
    // the photon travels against the traced path, so its angular momentum about the axis is -b hz
    d.g = sqrt(max(1.0 - 3.0 * m / r, 0.0)) / (1.0 + omega * b * hz);
    let p = cos(phi_k) * e1 + sin(phi_k) * e2;
    d.psi = atan2(dot(p, P.dy.xyz), dot(p, P.dx.xyz));
    let t = P.dy.w * profile(r / P.hole.w);
    d.color = blackbody(d.g * t) * (P.dz.w * pattern(r, d.psi, m));
    return d;
}

// ---------------------------------------------------------------- one ray

struct Ray {
    color: vec3<f32>,
    dbg: vec4<f32>,
    dbg2: vec4<f32>,
}

fn shade(d: vec3<f32>, pixel_angle: f32) -> Ray {
    var out: Ray;
    out.color = vec3<f32>(0.0);
    out.dbg = vec4<f32>(0.0);
    out.dbg2 = vec4<f32>(0.0);
    let m = P.eye.w;
    let to_eye = P.eye.xyz - P.hole.xyz;
    let r_o = length(to_eye);
    let n = to_eye / r_o;
    let cosa = -dot(n, d);
    let perp = d + n * cosa;
    let sina = length(perp);
    var e2 = vec3<f32>(0.0);
    if (sina > 1e-6) {
        e2 = perp / sina;
    } else {
        var helper = vec3<f32>(1.0, 0.0, 0.0);
        if (abs(n.x) > 0.9) { helper = vec3<f32>(0.0, 1.0, 0.0); }
        e2 = normalize(cross(n, helper));
    }
    let b = max(r_o * sina / sqrt(1.0 - 2.0 * m / r_o), 1e-5);
    let has_disk = P.misc.x < 3u;
    let all = (P.misc.z & FLAG_ALL) != 0u;
    let phi0 = first_crossing(n, e2, P.dz.xyz);
    let t = trace(m, r_o, b, cosa > 0.0, phi0, P.hole.w, P.dx.w, all || !has_disk);
    let mode = u32(P.fwd.w);
    if (mode == 1u) {
        out.dbg = vec4<f32>(f32(t.kind), t.phi_inf, t.r.x, t.r.y);
        return out;
    }
    if (mode == 2u) {
        out.dbg = vec4<f32>(t.r.z, t.r.w, f32(t.n), phi0);
        return out;
    }
    if (mode == 3u) {
        out.dbg = vec4<f32>(0.0, -1.0, 0.0, -1.0);
        if (t.hit >= 0 && has_disk) {
            let rh = crossing_radius(t, t.hit);
            let e = disk_at(m, rh, b, phi0 + f32(t.hit) * PI, n, e2);
            out.dbg = vec4<f32>(e.g, e.r, e.psi, f32(t.hit));
        }
        return out;
    }
    if (has_disk && t.hit >= 0) {
        let rh = crossing_radius(t, t.hit);
        out.color = disk_at(m, rh, b, phi0 + f32(t.hit) * PI, n, e2).color;
        return out;
    }
    if (t.kind == 1u) {
        let dir = cos(t.phi_inf) * n + sin(t.phi_inf) * e2;
        out.color = stars(dir, pixel_angle);
    }
    return out;
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let px = floor(pos.xy);
    let focal = P.right.w;
    let half = P.size.xy * 0.5;
    let mode = u32(P.fwd.w);
    if (mode != 0u) {
        // one ray through the centre of the pixel
        let d = normalize(P.fwd.xyz * focal + P.right.xyz * (px.x + 0.5 - half.x) + P.down.xyz * (px.y + 0.5 - half.y));
        let r = shade(d, 1.0 / focal);
        return r.dbg;
    }
    let count = max(u32(P.down.w), 1u);
    let jitter0 = vec2<f32>(unit(hash3(u32(px.x), u32(px.y), 1u)), unit(hash3(u32(px.x), u32(px.y), 2u)));
    var sum = vec3<f32>(0.0);
    for (var s = 0u; s < count; s++) {
        var j = vec2<f32>(0.5);
        if (count > 1u) {
            j = fract(jitter0 + f32(s) * vec2<f32>(0.7548776662, 0.5698402910));
        }
        let d = normalize(P.fwd.xyz * focal + P.right.xyz * (px.x + j.x - half.x) + P.down.xyz * (px.y + j.y - half.y));
        sum += shade(d, 1.0 / focal).color;
    }
    return vec4<f32>(sum / f32(count), 1.0);
}
