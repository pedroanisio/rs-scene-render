// ---------------------------------------------------------------- splats

struct Splat {
    // xyz position (world), opacity
    pos: vec4<f32>,
    // covariance upper triangle: xx, xy, xz, yy
    cov_a: vec4<f32>,
    // yz, zz, unused, unused
    cov_b: vec4<f32>,
    color: vec4<f32>,
};

@group(1) @binding(0) var<storage, read> splats: array<Splat>;
@group(1) @binding(1) var<storage, read> order: array<u32>;

struct SplatObject {
    model: mat4x4<f32>,
    // opacity, unused ×3
    params: vec4<f32>,
};

@group(1) @binding(2) var<uniform> sobj: SplatObject;

struct SOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    // offset from the centre in pixels
    @location(1) d: vec2<f32>,
    // inverse 2D covariance (a, b, c)
    @location(2) conic: vec3<f32>,
};

@vertex
fn vs_splat(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SOut {
    var o: SOut;
    let sp = splats[order[ii]];
    let world = sobj.model * vec4(sp.pos.xyz, 1.0);
    let v = fr.view * world;
    o.clip = vec4(0.0, 0.0, 2.0, 1.0);
    if (v.z <= fr.post.z) { return o; }
    // frustum cull with a guard band, as 3D Gaussian Splatting does: a splat just in front of the camera
    // but far to the side has a tiny depth, so its footprint would be huge and cover the whole frame
    let c0 = fr.view_proj * world;
    let ndc = c0.xy / c0.w;
    if (abs(ndc.x) > 1.3 || abs(ndc.y) > 1.3) { return o; }
    // project the 3D covariance: Σ' = J W Σ Wᵀ Jᵀ
    let cov = mat3x3<f32>(
        vec3(sp.cov_a.x, sp.cov_a.y, sp.cov_a.z),
        vec3(sp.cov_a.y, sp.cov_a.w, sp.cov_b.x),
        vec3(sp.cov_a.z, sp.cov_b.x, sp.cov_b.y));
    let m3 = mat3x3<f32>(sobj.model[0].xyz, sobj.model[1].xyz, sobj.model[2].xyz);
    let w3 = mat3x3<f32>(fr.view[0].xyz, fr.view[1].xyz, fr.view[2].xyz) * m3;
    let focal = fr.lens.x;
    let j = mat3x3<f32>(vec3(focal / v.z, 0.0, 0.0), vec3(0.0, focal / v.z, 0.0), vec3(-focal * v.x / (v.z * v.z), -focal * v.y / (v.z * v.z), 0.0));
    let t = j * w3;
    let c2 = t * cov * transpose(t);
    let a = c2[0][0] + 0.3;
    let b = c2[0][1];
    let c = c2[1][1] + 0.3;
    let det = a * c - b * b;
    if (det <= 0.0) { return o; }
    let mid = 0.5 * (a + c);
    let lam = mid + sqrt(max(mid * mid - det, 0.1));
    let radius = ceil(3.0 * sqrt(lam));
    let quad = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0));
    let q = quad[vi % 6u];
    let center = fr.view_proj * world;
    let off_px = q * radius;
    o.clip = center + vec4(off_px * vec2(2.0 * fr.screen.z, -2.0 * fr.screen.w) * center.w, 0.0, 0.0);
    o.d = off_px;
    o.conic = vec3(c, -b, a) / det;
    o.color = vec4(sp.color.rgb, sp.color.a * sobj.params.x);
    return o;
}

@fragment
fn fs_splat(i: SOut) -> @location(0) vec4<f32> {
    let p = -0.5 * (i.conic.x * i.d.x * i.d.x + i.conic.z * i.d.y * i.d.y) - i.conic.y * i.d.x * i.d.y;
    if (p > 0.0) { discard; }
    let a = min(0.99, i.color.a * exp(p));
    if (a < 1.0 / 255.0) { discard; }
    return vec4(i.color.rgb * a, a);
}

