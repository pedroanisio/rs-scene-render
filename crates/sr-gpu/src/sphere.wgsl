// 360 reprojection: six cube faces (90° views from the viewport camera) into
// equirectangular, 3×2 cubemap, equi-angular cubemap (EAC) or 180° fisheye.
// Directions use the viewport camera's space: x right, y down, z forward.

const PI: f32 = 3.14159265;

struct Params {
    // face k maps face space → viewport camera space
    faces: array<mat4x4<f32>, 6>,
    // layout (0 equirect, 1 cubemap, 2 eac, 3 fisheye-180), unused ×3
    mode: vec4<f32>,
    // output region: x0, y0, w, h (pixels)
    region: vec4<f32>,
};

@group(0) @binding(0) var<uniform> pp: Params;
@group(0) @binding(1) var faces: texture_2d_array<f32>;
@group(0) @binding(2) var smp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VOut {
    var o: VOut;
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u)) * 2.0 - 1.0;
    o.pos = vec4(p, 0.0, 1.0);
    return o;
}

fn face_dir(k: u32, a: f32, b: f32) -> vec3<f32> {
    return normalize((pp.faces[k] * vec4(a, b, 1.0, 0.0)).xyz);
}

@fragment
fn fs_main(i: VOut) -> @location(0) vec4<f32> {
    let uv = (i.pos.xy - pp.region.xy) / pp.region.zw;
    let mode = u32(pp.mode.x);
    var d = vec3(0.0, 0.0, 1.0);
    if (mode == 0u) {
        let phi = (uv.x - 0.5) * 2.0 * PI;
        let th = uv.y * PI;
        d = vec3(sin(th) * sin(phi), -cos(th), sin(th) * cos(phi));
    } else if (mode == 1u || mode == 2u) {
        let col = min(u32(uv.x * 3.0), 2u);
        let row = min(u32(uv.y * 2.0), 1u);
        var a = fract(uv.x * 3.0) * 2.0 - 1.0;
        var b = fract(uv.y * 2.0) * 2.0 - 1.0;
        var k = 0u;
        if (mode == 1u) {
            // row 0: right, left, up; row 1: down, front, back
            k = array<u32, 6>(1u, 3u, 4u, 5u, 0u, 2u)[row * 3u + col];
        } else {
            // YouTube EAC: row 0 left, front, right; row 1 down, back, up (turned 90° clockwise)
            a = tan(a * PI * 0.25);
            b = tan(b * PI * 0.25);
            k = array<u32, 6>(3u, 0u, 1u, 5u, 2u, 4u)[row * 3u + col];
            if (row == 1u) {
                let t = a;
                a = b;
                b = -t;
            }
        }
        d = face_dir(k, a, b);
    } else {
        let side = min(pp.region.z, pp.region.w);
        let c = (i.pos.xy - pp.region.xy - pp.region.zw * 0.5) / (side * 0.5);
        let r = length(c);
        if (r > 1.0) { return vec4(0.0); }
        let th = r * PI * 0.5;
        let dir2 = select(vec2(0.0), c / r, r > 1e-6);
        d = vec3(sin(th) * dir2.x, sin(th) * dir2.y, cos(th));
    }
    // the face that sees d most directly
    var best = 0u;
    var bz = -2.0;
    for (var k = 0u; k < 6u; k++) {
        let l = (transpose(pp.faces[k]) * vec4(d, 0.0)).xyz;
        if (l.z > bz) {
            bz = l.z;
            best = k;
        }
    }
    let l = (transpose(pp.faces[best]) * vec4(d, 0.0)).xyz;
    let fuv = (l.xy / l.z) * 0.5 + 0.5;
    return textureSampleLevel(faces, smp, fuv, i32(best), 0.0);
}
