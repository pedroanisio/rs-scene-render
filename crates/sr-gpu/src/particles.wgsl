// Particles: one instanced quad each, drawn premultiplied into a target-space texture.

struct Target {
    // width, height (px)
    size: vec4<f32>,
};

@group(0) @binding(0) var<uniform> tgt: Target;
@group(0) @binding(1) var sprite: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;

struct Inst {
    // centre (px), half extents (px)
    @location(0) centre_half: vec4<f32>,
    // rotation (rad), shape (0 disc, 1 square, 2 sprite, 3 streak), unused ×2
    @location(1) rot_shape: vec4<f32>,
    // premultiplied colour
    @location(2) color: vec4<f32>,
    // sprite cell: u0, v0, u1, v1
    @location(3) cell: vec4<f32>,
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
    // quad coordinates in [−1, 1]
    @location(0) q: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) shape: u32,
    // half extents in pixels (antialiasing width)
    @location(4) half: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Inst) -> VOut {
    let corners = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0));
    let c = corners[vi];
    // one extra pixel of margin for antialiasing
    let half = inst.centre_half.zw + vec2(1.0);
    let r = inst.rot_shape.x;
    let d = vec2(c.x * half.x, c.y * half.y);
    let p = inst.centre_half.xy + vec2(d.x * cos(r) - d.y * sin(r), d.x * sin(r) + d.y * cos(r));
    var o: VOut;
    o.pos = vec4(p.x / tgt.size.x * 2.0 - 1.0, 1.0 - p.y / tgt.size.y * 2.0, 0.0, 1.0);
    o.q = c * half / max(inst.centre_half.zw, vec2(1e-3));
    o.color = inst.color;
    let t = c * 0.5 + 0.5;
    o.uv = mix(inst.cell.xy, inst.cell.zw, t);
    o.shape = u32(inst.rot_shape.y);
    o.half = max(inst.centre_half.zw, vec2(1e-3));
    return o;
}

@fragment
fn fs_main(i: VOut) -> @location(0) vec4<f32> {
    var cover = 1.0;
    if (i.shape == 0u) {
        // disc: one pixel of edge
        let d = (length(i.q) - 1.0) * min(i.half.x, i.half.y);
        cover = clamp(0.5 - d, 0.0, 1.0);
    } else if (i.shape == 1u) {
        let d = (max(abs(i.q.x), abs(i.q.y)) - 1.0) * min(i.half.x, i.half.y);
        cover = clamp(0.5 - d, 0.0, 1.0);
    } else if (i.shape == 2u) {
        let inside = all(abs(i.q) <= vec2(1.0));
        return select(vec4(0.0), textureSampleLevel(sprite, smp, i.uv, 0.0) * i.color.a, inside);
    } else {
        // streak: a capsule along x, fading towards the tail (−x)
        let r = min(i.half.y, i.half.x);
        let lx = i.q.x * i.half.x;
        let ly = i.q.y * i.half.y;
        let core = max(i.half.x - r, 0.0);
        let dx = max(abs(lx) - core, 0.0);
        let d = length(vec2(dx, ly)) - r;
        cover = clamp(0.5 - d, 0.0, 1.0) * clamp(0.5 + 0.5 * i.q.x + 0.25, 0.0, 1.0);
    }
    return i.color * cover;
}
