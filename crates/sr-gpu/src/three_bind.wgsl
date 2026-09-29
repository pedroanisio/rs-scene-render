@group(0) @binding(0) var<uniform> fr: Frame;
@group(0) @binding(1) var<storage, read> lights: array<Light>;
@group(0) @binding(2) var<storage, read> tiles: array<u32>;
@group(0) @binding(3) var shadow_tex: texture_depth_2d_array;
@group(0) @binding(4) var shadow_cmp: sampler_comparison;
@group(0) @binding(5) var<storage, read> shadow_mats: array<mat4x4<f32>>;
@group(0) @binding(6) var env_tex: texture_2d<f32>;
@group(0) @binding(7) var env_smp: sampler;
@group(0) @binding(8) var brdf_lut: texture_2d<f32>;
@group(0) @binding(9) var ies_tex: texture_2d<f32>;
@group(0) @binding(10) var scene_color: texture_2d<f32>;
@group(0) @binding(11) var clamp_smp: sampler;

// screen-space ambient occlusion (r) and the prepass's view depth (r) and reflectance (g)
@group(0) @binding(12) var ao_tex: texture_2d<f32>;
@group(0) @binding(13) var gb_depth: texture_2d<f32>;
// the visible sky at the source's resolution
@group(0) @binding(14) var sky_tex: texture_2d<f32>;
