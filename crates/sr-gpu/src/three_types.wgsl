// Forward+ PBR renderer, shadows, splats and camera post.
// Scene space: x right, y down, z away from the default camera; 100 units per metre.
// Depth is reverse-Z (near 1, far 0) in the main passes; shadow maps use standard depth.

const PI: f32 = 3.14159265;
const TILE: u32 = 16u;
const MAX_PER_TILE: u32 = 63u;

struct Frame {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    // w, h, 1/w, 1/h
    screen: vec4<f32>,
    // exposure multiplier, light count, tiles x, env intensity
    params: vec4<f32>,
    // unused, env visible, has env, env mip count
    params2: vec4<f32>,
    // coc scale, focus depth, max coc px, blades
    dof: vec4<f32>,
    // lens distortion k1, dof on, near, far
    post: vec4<f32>,
    // focal length (px), encode sRGB, ambient occlusion on, unused
    lens: vec4<f32>,
    // ambient-occlusion radius (scene units), intensity, screen-space reflections on, precomputed circle of confusion
    fx: vec4<f32>,
    sh: array<vec4<f32>, 9>,
    // world → environment rotation of the dome
    env_rot: mat4x4<f32>,
};

struct Light {
    // xyz position, w type (0 ambient, 1 directional, 2 point, 3 spot, 4 rect, 5 disk, 6 sphere)
    pos: vec4<f32>,
    // xyz direction of travel, w range (scene units, 0 = unbounded)
    dir: vec4<f32>,
    // linear rgb × intensity, w distance falloff exponent
    color: vec4<f32>,
    // cos outer, cos inner, first shadow view (−1 none), shadow softness (texels)
    spot: vec4<f32>,
    // width, height, radius (scene units; directional: the view depths where cascades 0–2 end), IES row (−1 none)
    size: vec4<f32>,
    // affects diffuse, affects specular, shadow bias, shadow views (1, 4 cascades or 6 cube faces)
    flags: vec4<f32>,
    // xyz right axis of the light (IES azimuth), w contact-shadow length (scene units, 0 = off)
    right: vec4<f32>,
};

