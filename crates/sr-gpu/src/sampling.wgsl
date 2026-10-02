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
