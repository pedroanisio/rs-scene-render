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
    // scene space → the splats' own space
    model_inv: mat4x4<f32>,
    // opacity, SH degree, unused ×2
    params: vec4<f32>,
};

@group(1) @binding(2) var<uniform> sobj: SplatObject;
// 16 RGB spherical-harmonic coefficients a splat (coefficient-major), when the degree is > 0
@group(1) @binding(3) var<storage, read> sh: array<f32>;

fn sh_c(i: u32, k: u32) -> vec3<f32> {
    let o = i * 48u + k * 3u;
    return vec3(sh[o], sh[o + 1u], sh[o + 2u]);
}

// View-dependent colour of 3D Gaussian Splatting (Kerbl et al. 2023, reference
// implementation's SH basis), in the capture's encoded colour; d is the unit direction from
// the camera to the splat in the splats' own space.
fn sh_color(i: u32, deg: u32, d: vec3<f32>) -> vec3<f32> {
    let c0 = 0.28209479177387814;
    var r = c0 * sh_c(i, 0u);
    if (deg >= 1u) {
        let c1 = 0.4886025119029199;
        r += -c1 * d.y * sh_c(i, 1u) + c1 * d.z * sh_c(i, 2u) - c1 * d.x * sh_c(i, 3u);
    }
    if (deg >= 2u) {
        let xx = d.x * d.x; let yy = d.y * d.y; let zz = d.z * d.z;
        r += 1.0925484305920792 * d.x * d.y * sh_c(i, 4u)
            - 1.0925484305920792 * d.y * d.z * sh_c(i, 5u)
            + 0.31539156525252005 * (2.0 * zz - xx - yy) * sh_c(i, 6u)
            - 1.0925484305920792 * d.x * d.z * sh_c(i, 7u)
            + 0.5462742152960396 * (xx - yy) * sh_c(i, 8u);
        if (deg >= 3u) {
            r += -0.5900435899266435 * d.y * (3.0 * xx - yy) * sh_c(i, 9u)
                + 2.890611442640554 * d.x * d.y * d.z * sh_c(i, 10u)
                - 0.4570457994644658 * d.y * (4.0 * zz - xx - yy) * sh_c(i, 11u)
                + 0.3731763325901154 * d.z * (2.0 * zz - 3.0 * xx - 3.0 * yy) * sh_c(i, 12u)
                - 0.4570457994644658 * d.x * (4.0 * zz - xx - yy) * sh_c(i, 13u)
                + 1.445305721320277 * d.z * (xx - yy) * sh_c(i, 14u)
                - 0.5900435899266435 * d.x * (xx - 3.0 * yy) * sh_c(i, 15u);
        }
    }
    return max(r + 0.5, vec3(0.0));
}

fn srgb_decode(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3(2.4));
    return select(hi, lo, c <= vec3(0.04045));
}

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
    var rgb = sp.color.rgb;
    let deg = u32(sobj.params.y);
    if (deg > 0u) {
        let cam = (sobj.model_inv * vec4(fr.eye.xyz, 1.0)).xyz;
        let dir = normalize(sp.pos.xyz - cam);
        rgb = srgb_decode(min(sh_color(order[ii], deg, dir), vec3(1.0)));
    }
    o.color = vec4(rgb, sp.color.a * sobj.params.x);
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

