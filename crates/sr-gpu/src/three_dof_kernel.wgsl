// Aperture directions depend on the ring, sample index and blade count, not
// the output pixel. Keep the original f32 operations on the GPU; the gather
// still multiplies direction * radius * blade factor in that order.
@group(0) @binding(0) var<uniform> pfr: Frame;
@group(0) @binding(1) var<storage, read_write> aperture: array<vec4<f32>, 128>;

@compute @workgroup_size(32)
fn cs_aperture(@builtin(global_invocation_id) id: vec3<u32>) {
    let ring = id.y + 1u;
    let k = id.x;
    let cnt = ring * 8u;
    let index = id.y * 32u + k;
    if (k >= cnt) {
        aperture[index] = vec4(0.0);
        return;
    }
    let a = f32(k) / f32(cnt) * 2.0 * PI;
    let blades = pfr.dof.w;
    var sc = 1.0;
    if (blades >= 3.0) {
        let seg = 2.0 * PI / blades;
        let local = a - seg * floor(a / seg) - seg * 0.5;
        sc = cos(seg * 0.5) / cos(local);
    }
    aperture[index] = vec4(cos(a), sin(a), sc, 0.0);
}
