// Sampler bits are defined by sr_3d::sampling::TextureSampler::flags.
fn address_index(x: i32, size: i32, mode: u32) -> i32 {
    if (mode == 1u) { return clamp(x, 0, size - 1); }
    if (mode == 2u) {
        let v = ((x % (size * 2)) + size * 2) % (size * 2);
        return select(size * 2 - 1 - v, v, v < size);
    }
    if (mode == 3u && (x < 0 || x >= size)) { return -1; }
    return ((x % size) + size) % size;
}
fn address_coord(v: f32, mode: u32) -> f32 {
    if (mode == 1u) { return clamp(v, 0.0, 1.0); }
    if (mode == 2u) { return v - floor(v / 2.0) * 2.0; }
    if (mode == 3u) { return clamp(v, -1.0, 2.0); }
    return fract(v);
}
fn sample_position(uv: vec2<f32>, size: vec2<f32>, flags: u32) -> vec2<f32> {
    return vec2(address_coord(uv.x, (flags >> 3u) & 3u), address_coord(uv.y, (flags >> 5u) & 3u)) * size - 0.5;
}
fn cubic_weights(t: f32) -> vec4<f32> {
    let t2 = t*t; let t3 = t2*t;
    return vec4(-0.5*t+t2-0.5*t3, 1.0-2.5*t2+1.5*t3, 0.5*t+2.0*t2-1.5*t3, -0.5*t2+0.5*t3);
}

// ---------------------------------------------------------------- SREP 71, shared by the raster and path-traced 3D passes

// The procedural sky (sr_3d::sky::Sky::radiance) at a unit direction of the dome's space, for the five vec4 of
// sr_3d::sky::Sky::uniforms: zenith (rgb, exponent), horizon (rgb, 1), ground, sun direction (xyz, cos of half its
// size), sun radiance.
fn sky_radiance(d: vec3<f32>, p: array<vec4<f32>, 5>) -> vec3<f32> {
    let s = -d.y;
    var far = p[0].rgb;
    var t = s;
    if (s < 0.0) { far = p[2].rgb; t = -s; }
    let w = select(pow(min(t, 1.0), p[0].w), 0.0, t <= 0.0);
    var c = p[1].rgb + (far - p[1].rgb) * w;
    if (any(p[4].rgb > vec3(0.0)) && dot(d, p[3].xyz) >= p[3].w) { c += p[4].rgb; }
    return c;
}

// The subsurface lobe (sr_3d::subsurface::lobe): the mean clamped cosine over the rings at the profile's quantiles
// (sr_3d::subsurface::QUANTILES, in units of the shape parameter d) on a surface of curvature 1 / radius.
const SSS_Q = array<f32, 16>(0.06383456, 0.20014349, 0.34958202, 0.51450014, 0.69787014, 0.90351087, 1.1364199, 1.4032811, 1.7132746, 2.0794415, 2.5211546, 3.0690293, 3.776038, 4.7477603, 6.25375, 9.535895);
fn sss_ring(c: f32, alpha: f32) -> f32 {
    let cc = clamp(c, -1.0, 1.0);
    let a = cc * cos(alpha);
    let b = sqrt(max(1.0 - cc * cc, 0.0)) * abs(sin(alpha));
    if (a >= b) { return a; }
    if (a <= -b) { return 0.0; }
    return (a * acos(clamp(-a / b, -1.0, 1.0)) + sqrt(max(b * b - a * a, 0.0))) * 0.318309886183791;
}
fn sss_lobe(c: f32, curvature: f32, d: f32) -> f32 {
    // a variable, which may be indexed by a value that is not constant
    var q = SSS_Q;
    var e = 0.0;
    for (var k = 0u; k < 16u; k++) {
        let half = min(q[k] * d * curvature * 0.5, 1.0);
        e += sss_ring(c, 2.0 * asin(half));
    }
    return e / 16.0;
}
fn sss_lobe3(c: f32, curvature: f32, d: vec3<f32>) -> vec3<f32> {
    return vec3(sss_lobe(c, curvature, d.x), sss_lobe(c, curvature, d.y), sss_lobe(c, curvature, d.z));
}
