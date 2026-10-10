//! The 3D renderer: forward+ PBR shading with shadows, image-based lighting,
//! transmission, Gaussian splats, depth of field and lens distortion. Its
//! output enters the compositor as a layer.
//!
//! Lights are binned into 16 px screen tiles on the CPU and each fragment
//! walks its tile's list. Shadows use one depth-array texture holding every
//! shadow view. The main pass is 4× multisampled; transmissive surfaces
//! refract a mip-mapped copy of the opaque result composited over the
//! compositor's backdrop; splats are radix-sorted on the GPU and drawn back
//! to front; blended surfaces follow, sorted back to front.

mod sort_cache;

use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::Zeroable;
use glam::{Mat4, Vec3};
use sr_3d::camera::CameraView;
use sr_3d::{AlphaMode, MaterialParams, Vertex};
use wgpu::util::DeviceExt;

use crate::resources::FORMAT;

const MSAA: u32 = 4;
const TILE: u32 = 16;
const MAX_PER_TILE: usize = 63;
const UNIFORM_ALIGN: u64 = 256;
const SHADOW_BUDGET: u64 = 512 << 20;
const IES_W: usize = 128;
const IES_ROWS: usize = 32;

/// A mesh on the GPU.
pub struct MeshGpu {
    pub vbuf: wgpu::Buffer,
    pub ibuf: wgpu::Buffer,
    pub count: u32,
    /// The vertices and indices on the CPU (the path tracer builds its BVH from them).
    pub cpu: Arc<(Vec<Vertex>, Vec<u32>)>,
    /// Object-space bounds.
    pub lo: Vec3,
    pub hi: Vec3,
}

/// A texture on the GPU (with mips).
pub struct TexGpu {
    pub view: wgpu::TextureView,
    pub key: u64,
    /// Size on the GPU (the source's, reduced to the device's limit).
    pub size: [u32; 2],
    /// Base-level RGBA8 pixels retained for ray tracing and displacement.
    pub rgba: Arc<[u8]>,
    pub srgb: bool,
    pub sampler: Option<sr_3d::sampling::TextureSampler>,
}

impl TexGpu {
    /// Samples the retained base level in linear colour using the texture sampler.
    pub fn sample(&self, uv: [f32; 2]) -> [f32; 4] {
        self.sampler.unwrap_or_default().sample(self.size, uv, |x, y| {
            let offset = (y as usize * self.size[0] as usize + x as usize) * 4;
            std::array::from_fn(|i| {
                let v = self.rgba[offset + i] as f32 / 255.0;
                if self.srgb && i < 3 {
                    sr_3d::material::srgb_to_linear(v)
                } else {
                    v
                }
            })
        })
    }
}

/// A prefiltered environment on the GPU.
pub struct EnvGpu {
    /// Prefiltered radiance for lighting (roughness by mip).
    pub view: wgpu::TextureView,
    /// The visible sky at the source's resolution.
    pub sky: wgpu::TextureView,
    pub mips: u32,
    pub sh: [[f32; 3]; 9],
}

/// Splats on the GPU (model space).
pub struct SplatGpu {
    pub buf: wgpu::Buffer,
    /// Spherical-harmonic coefficients (48 floats a splat), or one dummy entry.
    pub sh: wgpu::Buffer,
    pub sh_degree: u32,
    pub cpu: Arc<[[f32; 16]]>,
    pub cpu_sh: Arc<[[f32; 48]]>,
    pub n: u32,
    pub lo: Vec3,
    pub hi: Vec3,
}

/// Vertices of one draw.
pub enum MeshSrc {
    Cached(Arc<MeshGpu>),
    /// Deformed this frame (skinning, morphs): fresh vertices over the cached indices.
    Deformed(Vec<Vertex>, Arc<MeshGpu>),
}

impl MeshSrc {
    /// The vertices this draw uses (deformed ones when deformed) and the mesh's indices.
    pub fn cpu(&self) -> (&[Vertex], &[u32]) {
        match self {
            MeshSrc::Cached(m) => (&m.cpu.0, &m.cpu.1),
            MeshSrc::Deformed(v, m) => (v, &m.cpu.1),
        }
    }

    fn mesh(&self) -> &MeshGpu {
        match self {
            MeshSrc::Cached(m) | MeshSrc::Deformed(_, m) => m,
        }
    }

    fn bounds(&self) -> (Vec3, Vec3) {
        match self {
            Self::Cached(m) => (m.lo, m.hi),
            Self::Deformed(vertices, _) if vertices.is_empty() => (Vec3::ZERO, Vec3::ZERO),
            Self::Deformed(vertices, _) => {
                vertices.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), v| {
                    let p = Vec3::from_array(v.pos);
                    (lo.min(p), hi.max(p))
                })
            }
        }
    }
}

/// Texture slots: base colour, normal, metallic-roughness, occlusion, emissive, displacement.
pub type Maps = [Option<Arc<TexGpu>>; 6];

/// One mesh draw.
pub struct Draw3 {
    pub mesh: MeshSrc,
    pub model: Mat4,
    pub material: MaterialParams,
    pub maps: Maps,
    pub opacity: f32,
    pub cast_shadow: bool,
    pub receive_shadow: bool,
    /// A shadow catcher: draws no colour of its own, only the darkening the shadow casters cause (SREP shadow-catcher).
    pub shadow_catcher: bool,
}

impl Draw3 {
    /// Triangles of the mesh.
    pub fn mesh_triangles(&self) -> u64 {
        self.mesh.mesh().count as u64 / 3
    }

    /// Whether the mesh's bounds, placed by the draw's model matrix, reach into the clip volume of `view_proj`
    /// ([`box_in_view`]).
    pub fn in_view(&self, view_proj: Mat4) -> bool {
        box_in_view(view_proj * self.model, self.mesh.bounds())
    }

    /// The metallic the draw shades with: the material's factor, times the mean blue channel of its
    /// metallic-roughness map when it has one (at most 4,096 texels sampled); none for an unlit material, which
    /// reflects nothing, and for a shadow catcher, which draws no colour of its own.
    pub fn metallic(&self) -> Option<f32> {
        if self.material.unlit || self.shadow_catcher {
            return None;
        }
        let factor = self.material.metallic.clamp(0.0, 1.0);
        let Some(map) = &self.maps[2] else { return Some(factor) };
        let texels = map.rgba.len() / 4;
        if texels == 0 {
            return Some(factor);
        }
        let step = texels.div_ceil(4096);
        let (sum, n) =
            (0..texels).step_by(step).fold((0u64, 0u64), |(s, n), k| (s + map.rgba[4 * k + 2] as u64, n + 1));
        Some(factor * sum as f32 / (255.0 * n as f32))
    }
}

/// Whether a box, its eight corners mapped to clip space by `to_clip`, reaches into the clip volume (reverse-Z: depth
/// from 0 at the far plane to w at the near one): false only when every corner lies beyond the same plane of the volume.
/// This is the usual conservative frustum test: a box that misses the volume near one of its edges still counts as in.
pub fn box_in_view(to_clip: Mat4, (lo, hi): (Vec3, Vec3)) -> bool {
    let corners: [glam::Vec4; 8] = std::array::from_fn(|k| {
        let pick = |bit: usize, a: f32, b: f32| if k & bit == 0 { a } else { b };
        to_clip * Vec3::new(pick(1, lo.x, hi.x), pick(2, lo.y, hi.y), pick(4, lo.z, hi.z)).extend(1.0)
    });
    let beyond = |plane: fn(glam::Vec4) -> bool| corners.iter().all(|c| plane(*c));
    !(beyond(|c| c.x < -c.w)
        || beyond(|c| c.x > c.w)
        || beyond(|c| c.y < -c.w)
        || beyond(|c| c.y > c.w)
        || beyond(|c| c.z < 0.0)
        || beyond(|c| c.z > c.w))
}

/// The metallic at and above which a draw is reported when its 3D pass has no environment image. In this renderer's
/// shading (three.wgsl) a surface's diffuse response is albedo · (1 − metallic), and its specular reflectance at normal
/// incidence is f0 = mix(0.04, albedo, metallic) at the default ior of 1.5. The diffuse part shows the form under any
/// light; the specular part shows only what there is to reflect: a uniform colour from ambient light or from a dome
/// without an image, and highlights from punctual lights. The two parts are equal at metallic = (a − 0.04) / (2a − 0.04)
/// for an albedo a: 0.44 at a = 0.2, 0.48 at 0.5, 0.49 at 1. From 0.5 the reflection outweighs the diffuse response
/// whatever the albedo, so the surface reads mostly as the environment it does not have.
pub const METALLIC_NEEDS_ENVIRONMENT: f32 = 0.5;

/// Splats one storage binding of the device holds: 64 bytes each, 192 with spherical harmonics.
pub fn splat_capacity(limits: &wgpu::Limits, sh: bool) -> u64 {
    limits.max_storage_buffer_binding_size.min(limits.max_buffer_size) / if sh { 192 } else { 64 }
}

/// The note for a splat set of `total` of which the device holds `kept`.
pub fn splat_note(total: u64, kept: u64) -> Option<String> {
    (kept < total).then(|| {
        format!("{} of {total} Gaussian splats are not drawn: this device holds {kept} in one buffer", total - kept)
    })
}

/// A `w`×`h` image's size reduced, in proportion, to at most `cap` a side.
pub fn fit_texture(w: u32, h: u32, cap: u32) -> [u32; 2] {
    let m = w.max(h);
    if m <= cap {
        return [w, h];
    }
    [w, h].map(|v| ((v as u64 * cap as u64 / m as u64) as u32).max(1))
}

/// Which lights get shadow views: a view is a layer of one depth array, so once the device's
/// `max_layers` are taken the remaining lights light without shadows. A point light takes six
/// views, a directional one four cascades (one view under an orthographic camera), a spot one.
pub fn shadow_casters(lights: &[Light3], cascades: bool, max_layers: u32) -> Vec<bool> {
    let mut used = 0u32;
    lights
        .iter()
        .map(|l| {
            let views = match l.kind {
                _ if !l.cast_shadow => return false,
                LightKind::Ambient => return false,
                LightKind::Directional if cascades => 4,
                LightKind::Directional | LightKind::Spot => 1,
                _ => 6,
            };
            let fits = used + views <= max_layers;
            if fits {
                used += views;
            }
            fits
        })
        .collect()
}

/// The note for shadow-casting lights that [`shadow_casters`] leaves without shadows.
pub fn shadow_note(lights: &[Light3], cascades: bool, max_layers: u32) -> Option<String> {
    let cast = shadow_casters(lights, cascades, max_layers);
    let lost = lights.iter().zip(&cast).filter(|(l, c)| l.cast_shadow && l.kind != LightKind::Ambient && !**c).count();
    (lost > 0)
        .then(|| format!("{lost} light(s) cast no shadows: their shadow views exceed the {max_layers} this device has"))
}

/// Light kinds, as the shader numbers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightKind {
    Ambient = 0,
    Directional = 1,
    Point = 2,
    Spot = 3,
    Rect = 4,
    Disk = 5,
    Sphere = 6,
}

/// One light (dome lights become the environment).
#[derive(Clone, Debug)]
pub struct Light3 {
    pub kind: LightKind,
    pub pos: Vec3,
    /// Direction of travel (unit).
    pub dir: Vec3,
    /// Right axis (unit), for IES azimuth.
    pub right: Vec3,
    /// Linear colour × intensity × 2^exposure.
    pub color: Vec3,
    /// Scene units; 0 = unbounded.
    pub range: f32,
    pub falloff: f32,
    pub cos_outer: f32,
    pub cos_inner: f32,
    pub cast_shadow: bool,
    pub softness: f32,
    pub bias: f32,
    pub map_size: u32,
    /// width, height, radius.
    pub size: [f32; 3],
    pub ies: Option<Arc<Vec<f32>>>,
    pub affects_diffuse: bool,
    pub affects_specular: bool,
    /// Screen-space contact-shadow length in scene units (0: off).
    pub contact: f32,
}

/// The dome environment.
pub struct Env3 {
    pub env: Arc<EnvGpu>,
    pub intensity: f32,
    /// Environment → world rotation: the dome's yaw, pitch and roll.
    pub rotation: Mat4,
    pub visible: bool,
}

/// One splat cloud.
pub struct SplatDraw {
    pub gpu: Arc<SplatGpu>,
    pub model: Mat4,
    pub opacity: f32,
}

/// Depth of field.
#[derive(Clone, Copy, Debug)]
pub struct Dof {
    /// CoC px = scale · |1/focus − 1/depth|.
    pub coc_scale: f32,
    pub focus: f32,
    pub max_coc: f32,
    pub blades: u32,
}

/// Everything one 3D pass draws.
pub struct Scene3 {
    pub cam: CameraView,
    /// Maps the camera's frame clip space onto the target (offscreens); identity at the root.
    pub clip_fix: Mat4,
    pub size: [u32; 2],
    pub exposure: f32,
    pub dof: Option<Dof>,
    pub lens_k1: f32,
    pub draws: Vec<Draw3>,
    pub lights: Vec<Light3>,
    pub env: Option<Env3>,
    pub splats: Vec<SplatDraw>,
    /// Participating media in the same world and depth system as surface geometry.
    pub volumes: Vec<crate::volume::VolumeDraw>,
    /// Encode the output with the sRGB curve (working spaces that blend on encoded values).
    pub encode_srgb: bool,
    /// Screen-space ambient occlusion: radius (scene units) and intensity.
    pub ao: Option<[f32; 2]>,
    /// Screen-space reflections.
    pub ssr: bool,
    /// Path trace instead of rasterising.
    pub path: Option<crate::pathtrace::PathOpts>,
    /// Trace the geodesics of a black hole instead: nothing else of the scene is drawn.
    pub geodesic: Option<crate::geodesic::GeodesicScene>,
}

impl Scene3 {
    /// Identity of a self-contained raster pass for contrast counterfactuals.
    /// Hash resolved geometry and shader inputs, not frame time or node ids:
    /// shutter samples and simulated geometry can differ at the same time.
    /// External textures, media and backdrop-dependent transmission stay uncached.
    pub(crate) fn contrast_cache_key(&self) -> Option<[u8; 32]> {
        use sha2::{Digest, Sha256};
        let Self {
            cam,
            clip_fix,
            size,
            exposure,
            dof,
            lens_k1,
            draws,
            lights,
            env,
            splats,
            volumes,
            encode_srgb,
            ao,
            ssr,
            path,
            geodesic,
        } = self;
        if env.is_some() || !splats.is_empty() || !volumes.is_empty() || path.is_some() || geodesic.is_some() {
            return None;
        }
        let CameraView { view, proj, eye, focal_px, near, far, orthographic } = cam;
        let mut hash = Sha256::new();
        hash.update(b"contrast-raster-v1");
        hash.update(bytemuck::cast_slice(size));
        for matrix in [view, proj, clip_fix] {
            hash.update(bytemuck::cast_slice(&matrix.to_cols_array()));
        }
        hash.update(bytemuck::cast_slice(&[eye.x, eye.y, eye.z, *focal_px, *near, *far, *exposure, *lens_k1]));
        hash.update([*orthographic as u8, *encode_srgb as u8, *ssr as u8, dof.is_some() as u8, ao.is_some() as u8]);
        if let Some(Dof { coc_scale, focus, max_coc, blades }) = dof {
            hash.update(bytemuck::cast_slice(&[*coc_scale, *focus, *max_coc]));
            hash.update(blades.to_le_bytes());
        }
        if let Some(ao) = ao {
            hash.update(bytemuck::cast_slice(ao));
        }
        hash.update((draws.len() as u64).to_le_bytes());
        for Draw3 { mesh, model, material, maps, opacity, cast_shadow, receive_shadow, shadow_catcher } in draws {
            if maps.iter().any(Option::is_some) || material.transmission != 0.0 || material.foam_mix.is_some() {
                return None;
            }
            let (vertices, indices) = mesh.cpu();
            hash.update([matches!(mesh, MeshSrc::Deformed(..)) as u8]);
            hash.update((vertices.len() as u64).to_le_bytes());
            hash.update(bytemuck::cast_slice(vertices));
            hash.update((indices.len() as u64).to_le_bytes());
            hash.update(bytemuck::cast_slice(indices));
            let gpu = mesh.mesh();
            hash.update(gpu.count.to_le_bytes());
            hash.update(bytemuck::cast_slice(&[gpu.lo.x, gpu.lo.y, gpu.lo.z, gpu.hi.x, gpu.hi.y, gpu.hi.z]));
            hash.update(bytemuck::cast_slice(&model.to_cols_array()));
            hash.update(bytemuck::bytes_of(&material_u(material, maps)));
            hash.update(opacity.to_le_bytes());
            hash.update([*cast_shadow as u8, *receive_shadow as u8, *shadow_catcher as u8]);
        }
        hash.update((lights.len() as u64).to_le_bytes());
        for Light3 {
            kind,
            pos,
            dir,
            right,
            color,
            range,
            falloff,
            cos_outer,
            cos_inner,
            cast_shadow,
            softness,
            bias,
            map_size,
            size,
            ies,
            affects_diffuse,
            affects_specular,
            contact,
        } in lights
        {
            if ies.is_some() {
                return None;
            }
            hash.update((*kind as u32).to_le_bytes());
            hash.update(bytemuck::cast_slice(&[
                pos.x, pos.y, pos.z, dir.x, dir.y, dir.z, right.x, right.y, right.z, color.x, color.y, color.z, *range,
                *falloff, *cos_outer, *cos_inner, *softness, *bias, size[0], size[1], size[2], *contact,
            ]));
            hash.update(map_size.to_le_bytes());
            hash.update([*cast_shadow as u8, *affects_diffuse as u8, *affects_specular as u8]);
        }
        Some(hash.finalize().into())
    }
}

/// Counters for statistics and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats3 {
    /// Objects prepared for at least one raster camera or shadow pass.
    pub prepared_objects: usize,
    /// Splat clouds whose depth order was computed this pass.
    pub splat_sorts: usize,
    /// Packed vertex/BVH buffer sets uploaded by the path tracer this pass.
    pub pt_geometry_uploads: usize,
    pub draws: usize,
    pub triangles: u64,
    pub shadow_views: usize,
    pub splats: u64,
    pub transmissive: usize,
    pub tile_overflow: bool,
    /// Path tracer CPU seconds: scene assembly, BVH construction and buffer packing.
    pub pt_assemble_seconds: f64,
    pub pt_bvh_seconds: f64,
    pub pt_pack_seconds: f64,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FrameU {
    view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    screen: [f32; 4],
    params: [f32; 4],
    params2: [f32; 4],
    dof: [f32; 4],
    post: [f32; 4],
    lens: [f32; 4],
    fx: [f32; 4],
    sh: [[f32; 4]; 9],
    env_rot: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LightU {
    pos: [f32; 4],
    dir: [f32; 4],
    color: [f32; 4],
    spot: [f32; 4],
    size: [f32; 4],
    flags: [f32; 4],
    right: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MaterialU {
    base_color: [f32; 4],
    emissive: [f32; 4],
    p0: [f32; 4],
    p1: [f32; 4],
    p2: [f32; 4],
    p3: [f32; 4],
    attenuation: [f32; 4],
    sheen: [f32; 4],
    specular_color: [f32; 4],
    irid: [f32; 4],
    aniso: [f32; 4],
    uv: [f32; 4],
    /// unevenness, feature size, seed (bits), unused
    finish: [f32; 4],
    sampling: [[u32; 4]; 6],
    borders: [[f32; 4]; 6],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ObjectU {
    model: [[f32; 4]; 4],
    normal: [[f32; 4]; 4],
    params: [f32; 4],
    spacing: [f32; 4],
    padding: [[f32; 4]; 6],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SortU {
    n: u32,
    blocks: u32,
    shift: u32,
    pad: u32,
    view: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SplatObjU {
    model: [[f32; 4]; 4],
    /// Scene space to the splats' own space (for view directions of their SH colour).
    model_inv: [[f32; 4]; 4],
    /// opacity, SH degree, unused ×2
    params: [f32; 4],
}

fn pad(v: &mut Vec<u8>, bytes: &[u8]) -> u32 {
    let off = v.len() as u32;
    v.extend_from_slice(bytes);
    v.resize(v.len().div_ceil(UNIFORM_ALIGN as usize) * UNIFORM_ALIGN as usize, 0);
    off
}

fn push_object(bytes: &mut Vec<u8>, object: &ObjectU, limit: u64) -> Result<u32, String> {
    let size = std::mem::size_of::<ObjectU>();
    let next = bytes.len().checked_add(size).ok_or("3D instance object size overflow")?;
    if next as u64 > limit || next > u32::MAX as usize {
        return Err("3D instance objects exceed device storage buffer limit".into());
    }
    let offset = bytes.len() as u32;
    bytes.extend_from_slice(bytemuck::bytes_of(object));
    Ok(offset)
}

fn material_u(m: &MaterialParams, maps: &Maps) -> MaterialU {
    let mut bits = 0u32;
    for (k, t) in maps.iter().enumerate() {
        if t.is_some() {
            bits |= 1 << k;
        }
    }
    let mode = match m.alpha_mode {
        AlphaMode::Opaque => 0.0,
        AlphaMode::Mask => 1.0,
        AlphaMode::Blend => 2.0,
    };
    MaterialU {
        base_color: m.base_color,
        emissive: [m.emissive[0], m.emissive[1], m.emissive[2], m.emissive_strength],
        p0: [m.metallic, m.roughness, m.opacity, m.alpha_cutoff],
        p1: [mode, m.double_sided as u32 as f32, m.unlit as u32 as f32, m.normal_scale],
        p2: [m.clearcoat, m.clearcoat_roughness, m.transmission, m.ior.max(1.0)],
        p3: [
            m.thickness,
            if m.attenuation_distance.is_finite() { m.attenuation_distance } else { 0.0 },
            m.dispersion,
            m.specular,
        ],
        attenuation: [m.attenuation_color[0], m.attenuation_color[1], m.attenuation_color[2], 1.0],
        sheen: [m.sheen_color[0], m.sheen_color[1], m.sheen_color[2], m.sheen_roughness],
        specular_color: [m.specular_color[0], m.specular_color[1], m.specular_color[2], 1.0],
        irid: [m.iridescence, m.iridescence_ior, m.iridescence_thickness, m.occlusion_strength],
        aniso: [m.anisotropy, m.anisotropy_rotation, m.displacement_scale, bits as f32],
        uv: [m.uv_scale[0], m.uv_scale[1], m.separate_uvs as u32 as f32, 0.0],
        finish: [m.unevenness, m.unevenness_scale.max(1e-6), f32::from_bits(m.unevenness_seed), 0.0],
        sampling: std::array::from_fn(|i| [maps[i].as_ref().and_then(|t| t.sampler).map_or(0, |s| s.flags()), 0, 0, 0]),
        borders: std::array::from_fn(|i| maps[i].as_ref().and_then(|t| t.sampler).map_or([0.0; 4], |s| s.border)),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PipeKey {
    cull: bool,
    blend: bool,
    plain: bool,
    global_lights: bool,
    light_signature: u32,
    alpha_mask: bool,
}

fn plain_material(draw: &Draw3) -> bool {
    let m = &draw.material;
    draw.maps.iter().all(Option::is_none)
        && m.foam_mix.is_none()
        && m.unevenness == 0.0
        && m.anisotropy.abs() <= 1e-3
        && m.clearcoat == 0.0
        && m.sheen_color == [0.0; 3]
        && m.iridescence == 0.0
        && m.transmission == 0.0
}

/// GPU state of the 3D renderer.
pub struct ThreeEngine {
    device: Arc<wgpu::Device>,
    /// Format of the view-depth and reflectance target of the screen-space pre-pass.
    gbuffer: wgpu::TextureFormat,
    /// Format of the resolved depth and the depth-of-field tile maps.
    scalar: wgpu::TextureFormat,
    targets: Arc<std::sync::Mutex<TargetPool>>,
    queue: Arc<wgpu::Queue>,
    main_mod: wgpu::ShaderModule,
    #[cfg(test)]
    specialize_plain: bool,
    #[cfg(test)]
    specialize_alpha_mask: bool,
    specialize_global_lights: bool,
    bgl_frame: wgpu::BindGroupLayout,
    bgl_mat: wgpu::BindGroupLayout,
    bgl_obj: wgpu::BindGroupLayout,
    bgl_splat: wgpu::BindGroupLayout,
    bgl_ssr: wgpu::BindGroupLayout,
    /// Path-tracing pipelines, built on first use.
    pt: Option<crate::pathtrace::PtGpu>,
    pt_geometry: crate::pathtrace::BuildCache,
    geodesic: Option<crate::geodesic::GeodesicGpu>,
    /// Depth and normal prepass pipelines (by culling).
    pre_pipes: [wgpu::RenderPipeline; 2],
    ssao_pipe: wgpu::RenderPipeline,
    ao_blur_pipe: wgpu::RenderPipeline,
    ssr_pipe: wgpu::RenderPipeline,
    bgl_post: wgpu::BindGroupLayout,
    bgl_depth: wgpu::BindGroupLayout,
    bgl_sort: wgpu::BindGroupLayout,
    main_layout: wgpu::PipelineLayout,
    pipes: HashMap<PipeKey, wgpu::RenderPipeline>,
    shadow_pipe: wgpu::RenderPipeline,
    dome_pipe: wgpu::RenderPipeline,
    splat_pipe: wgpu::RenderPipeline,
    blit_pipe: wgpu::RenderPipeline,
    under_pipe: wgpu::RenderPipeline,
    dof_pipe: wgpu::RenderPipeline,
    dof_kernel_pipe: Option<wgpu::ComputePipeline>,
    tile_max_pipe: wgpu::RenderPipeline,
    tile_dilate_pipe: wgpu::RenderPipeline,
    depth_pipe: wgpu::RenderPipeline,
    depth_coc_pipe: wgpu::RenderPipeline,
    sort_pipes: [wgpu::ComputePipeline; 4],
    software_scatter: Option<wgpu::ComputePipeline>,
    sorted_splats: sort_cache::Cache,
    /// Enables linear-work splat scattering on software adapters. Renderer sets
    /// this from the selected adapter; direct engine users can select it too.
    pub software_adapter: bool,
    repeat_smp: wgpu::Sampler,
    clamp_smp: wgpu::Sampler,
    cmp_smp: wgpu::Sampler,
    white: Arc<TexGpu>,
    brdf: wgpu::TextureView,
    black_env: wgpu::TextureView,
    mat_binds: HashMap<[u64; 6], wgpu::BindGroup>,
    /// Meshes keyed by the caller.
    pub meshes: HashMap<String, Arc<MeshGpu>>,
    /// The mesh of each parametric surface or heightfield node (SREP 70) with its content key: one upload per node,
    /// replaced when the sampled mesh changes, so an animated surface holds one mesh, not one per frame.
    pub param_meshes: HashMap<Arc<str>, (u64, Arc<MeshGpu>)>,
    /// Textures keyed by the caller.
    pub textures: HashMap<String, Arc<TexGpu>>,
    pub(crate) drape_order: std::collections::VecDeque<String>,
    /// Environments keyed by the caller.
    pub envs: HashMap<String, Arc<EnvGpu>>,
    /// Splats keyed by the caller.
    pub splat_cache: HashMap<String, Arc<SplatGpu>>,
    next_key: u64,
    ies_rows: HashMap<usize, usize>,
    /// Statistics of the last render.
    pub stats: Stats3,
    /// Whether raster and path-traced passes record timestamp queries.
    pub time_gpu: bool,
    /// Timestamps of the path-traced passes recorded since the caller last cleared them.
    pub pt_timers: Vec<crate::fx::Timer>,
    /// Timestamps of raster passes recorded since the caller last cleared them.
    pub raster_timers: Vec<crate::fx::Timer>,
}

fn raster_stamp<'a>(
    timer: &'a mut Option<crate::fx::Timer>,
    label: &str,
) -> Option<wgpu::RenderPassTimestampWrites<'a>> {
    let timer = timer.as_mut()?;
    let begin = timer.labelled_pair(label)?;
    Some(wgpu::RenderPassTimestampWrites {
        query_set: &timer.set,
        beginning_of_pass_write_index: Some(begin),
        end_of_pass_write_index: Some(begin + 1),
    })
}

fn tex_entry(
    binding: u32,
    vis: wgpu::ShaderStages,
    sample: wgpu::TextureSampleType,
    dim: wgpu::TextureViewDimension,
    ms: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis,
        ty: wgpu::BindingType::Texture { sample_type: sample, view_dimension: dim, multisampled: ms },
        count: None,
    }
}

fn buf_entry(
    binding: u32,
    vis: wgpu::ShaderStages,
    ty: wgpu::BufferBindingType,
    dynamic: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis,
        ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: dynamic, min_binding_size: None },
        count: None,
    }
}

fn smp_entry(binding: u32, vis: wgpu::ShaderStages, ty: wgpu::SamplerBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty: wgpu::BindingType::Sampler(ty), count: None }
}

const VS_FS: wgpu::ShaderStages = wgpu::ShaderStages::VERTEX_FRAGMENT;
const F: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: true };
const D2: wgpu::TextureViewDimension = wgpu::TextureViewDimension::D2;

fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRS: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4, 4 => Float32x2, 5 => Float32x2, 6 => Float32x2, 7 => Float32x2, 8 => Float32x4];
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRS,
    }
}

const PREMUL: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
};

/// Render targets of [`ThreeEngine::render`] kept between calls. Each is a large dedicated
/// allocation, and creating and dropping them every frame cost more CPU time than the rest of the
/// pass. A target is lent for one call and returned at its end; the encoder orders the
/// passes of successive calls, so reuse is safe, and targets unused for a while are dropped.
#[derive(Default)]
struct TargetPool {
    free: HashMap<TargetKey, Vec<(wgpu::Texture, u64)>>,
    lent: Vec<(TargetKey, wgpu::Texture)>,
    calls: u64,
}

type TargetKey = ([u32; 2], wgpu::TextureFormat, u32, u32, u32, &'static str);

/// Calls a pooled target may go unused before it is freed.
const POOL_KEEP: u64 = 64;

impl TargetPool {
    fn take(&mut self, device: &wgpu::Device, key: TargetKey) -> wgpu::Texture {
        let t = match self.free.get_mut(&key).and_then(|v| v.pop()) {
            Some((t, _)) => t,
            None => {
                let (size, format, mips, samples, layers, label) = key;
                texture(device, size, format, mips, samples, layers, label)
            }
        };
        self.lent.push((key, t.clone()));
        t
    }

    /// Returns the targets lent during this call and frees those unused for [`POOL_KEEP`] calls.
    fn end_call(&mut self) {
        self.calls += 1;
        let now = self.calls;
        for (k, t) in self.lent.drain(..) {
            self.free.entry(k).or_default().push((t, now));
        }
        self.free.retain(|_, v| {
            v.retain(|(_, used)| now - used <= POOL_KEEP);
            !v.is_empty()
        });
    }
}

fn texture(
    device: &wgpu::Device,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    mips: u32,
    samples: u32,
    layers: u32,
    label: &str,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: layers },
        mip_level_count: mips,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | if samples == 1 {
                wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC
            } else {
                wgpu::TextureUsages::empty()
            },
        view_formats: &[],
    })
}

fn half4(v: [f32; 4]) -> [u16; 4] {
    v.map(|x| half::f16::from_f32(x).to_bits())
}

impl ThreeEngine {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> ThreeEngine {
        ThreeEngine::new_with(device, queue, wgpu::TextureFormat::Rgba32Float, wgpu::TextureFormat::R32Float)
    }

    /// An engine whose float targets use formats the device renders to: view depth and
    /// reflectance in `gbuffer`, resolved depth and depth-of-field tiles in `scalar` (see
    /// `Gpu::gbuffer_depth` and `Gpu::scalar_target`: 32-bit float where the device can).
    pub fn new_with(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        gbuffer: wgpu::TextureFormat,
        scalar: wgpu::TextureFormat,
    ) -> ThreeEngine {
        let d = &*device;
        let module = |src: String, label: &str| {
            d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            })
        };
        let main_mod = module(main_src(), "three");
        let splat_mod = module(splat_src(), "three-splat");
        let post_mod = module(post_src(), "three-post");
        let depth_mod = module(depth_src(), "three-depth");
        let sort_mod = module(sort_src(), "three-sort");
        let ro = wgpu::BufferBindingType::Storage { read_only: true };
        let rw = wgpu::BufferBindingType::Storage { read_only: false };
        let uni = wgpu::BufferBindingType::Uniform;
        let filt = wgpu::SamplerBindingType::Filtering;
        let bgl = |entries: &[wgpu::BindGroupLayoutEntry], label: &str| {
            d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries })
        };
        let bgl_frame = bgl(
            &[
                buf_entry(0, VS_FS, uni, false),
                buf_entry(1, VS_FS, ro, false),
                buf_entry(2, VS_FS, ro, false),
                tex_entry(3, VS_FS, wgpu::TextureSampleType::Depth, wgpu::TextureViewDimension::D2Array, false),
                smp_entry(4, VS_FS, wgpu::SamplerBindingType::Comparison),
                buf_entry(5, VS_FS, ro, false),
                tex_entry(6, VS_FS, F, D2, false),
                smp_entry(7, VS_FS, filt),
                tex_entry(8, VS_FS, F, D2, false),
                tex_entry(9, VS_FS, F, D2, false),
                tex_entry(10, VS_FS, F, D2, false),
                smp_entry(11, VS_FS, filt),
                tex_entry(12, VS_FS, F, D2, false),
                tex_entry(13, VS_FS, wgpu::TextureSampleType::Float { filterable: false }, D2, false),
                tex_entry(14, VS_FS, F, D2, false),
            ],
            "three-frame",
        );
        let mut mat_entries = vec![buf_entry(0, VS_FS, uni, true)];
        for b in 1..=6 {
            mat_entries.push(tex_entry(b, VS_FS, F, D2, false));
        }
        mat_entries.push(smp_entry(7, VS_FS, filt));
        let bgl_mat = bgl(&mat_entries, "three-material");
        let bgl_obj = bgl(
            &[
                buf_entry(0, VS_FS, wgpu::BufferBindingType::Storage { read_only: true }, false),
                buf_entry(1, wgpu::ShaderStages::VERTEX, wgpu::BufferBindingType::Storage { read_only: true }, false),
            ],
            "three-object",
        );
        let bgl_splat = bgl(
            &[
                buf_entry(0, VS_FS, ro, false),
                buf_entry(1, VS_FS, ro, false),
                buf_entry(2, VS_FS, uni, false),
                buf_entry(3, VS_FS, ro, false),
            ],
            "three-splat",
        );
        let bgl_post = bgl(
            &[
                buf_entry(0, VS_FS, uni, false),
                tex_entry(1, VS_FS, F, D2, false),
                tex_entry(2, VS_FS, wgpu::TextureSampleType::Float { filterable: false }, D2, false),
                smp_entry(3, VS_FS, filt),
                tex_entry(4, VS_FS, wgpu::TextureSampleType::Float { filterable: false }, D2, false),
                buf_entry(5, VS_FS, uni, false),
            ],
            "three-post",
        );
        let bgl_depth = bgl(
            &[
                tex_entry(0, wgpu::ShaderStages::FRAGMENT, wgpu::TextureSampleType::Depth, D2, true),
                buf_entry(1, wgpu::ShaderStages::FRAGMENT, uni, false),
            ],
            "three-depth",
        );
        let cs = wgpu::ShaderStages::COMPUTE;
        let bgl_sort = bgl(
            &[
                buf_entry(0, cs, uni, true),
                buf_entry(1, cs, ro, false),
                buf_entry(2, cs, rw, false),
                buf_entry(3, cs, rw, false),
                buf_entry(4, cs, rw, false),
                buf_entry(5, cs, rw, false),
                buf_entry(6, cs, rw, false),
            ],
            "three-sort",
        );
        let layout = |bgls: &[&wgpu::BindGroupLayout]| {
            let v: Vec<Option<&wgpu::BindGroupLayout>> = bgls.iter().map(|b| Some(*b)).collect();
            d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &v,
                immediate_size: 0,
            })
        };
        let main_layout = layout(&[&bgl_frame, &bgl_mat, &bgl_obj]);
        // depth and normal prepass for the screen-space effects: view normal + roughness, view
        // depth + reflectance
        let pre_pipe = |cull: bool| {
            let _creation = crate::gpu::creation_lock();
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("three-prepass"),
                layout: Some(&main_layout),
                vertex: wgpu::VertexState {
                    module: &main_mod,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(vertex_layout())],
                },
                primitive: wgpu::PrimitiveState {
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: if cull { Some(wgpu::Face::Back) } else { None },
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &main_mod,
                    entry_point: Some("fs_prepass"),
                    compilation_options: Default::default(),
                    targets: &[Some(FORMAT.into()), Some(gbuffer.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pre_pipes = [pre_pipe(true), pre_pipe(false)];
        let shadow_pipe = {
            let _creation = crate::gpu::creation_lock();
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("three-shadow"),
                layout: Some(&main_layout),
                vertex: wgpu::VertexState {
                    module: &main_mod,
                    entry_point: Some("vs_shadow"),
                    compilation_options: Default::default(),
                    buffers: &[Some(vertex_layout())],
                },
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
                }),
                multisample: Default::default(),
                fragment: None,
                multiview_mask: None,
                cache: None,
            })
        };
        let ms = wgpu::MultisampleState { count: MSAA, ..Default::default() };
        let depth_state = |write: bool| wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(write),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: Default::default(),
            bias: Default::default(),
        };
        let target = |blend: Option<wgpu::BlendState>| {
            [Some(wgpu::ColorTargetState { format: FORMAT, blend, write_mask: wgpu::ColorWrites::ALL })]
        };
        let dome_pipe = {
            let _creation = crate::gpu::creation_lock();
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("three-dome"),
                layout: Some(&main_layout),
                vertex: wgpu::VertexState {
                    module: &main_mod,
                    entry_point: Some("vs_full"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: Some(depth_state(false)),
                multisample: ms,
                fragment: Some(wgpu::FragmentState {
                    module: &main_mod,
                    entry_point: Some("fs_dome"),
                    compilation_options: Default::default(),
                    targets: &target(None),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let splat_layout = layout(&[&bgl_frame, &bgl_splat]);
        let splat_pipe = {
            let _creation = crate::gpu::creation_lock();
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("three-splat"),
                layout: Some(&splat_layout),
                vertex: wgpu::VertexState {
                    module: &splat_mod,
                    entry_point: Some("vs_splat"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: Some(depth_state(false)),
                multisample: ms,
                fragment: Some(wgpu::FragmentState {
                    module: &splat_mod,
                    entry_point: Some("fs_splat"),
                    compilation_options: Default::default(),
                    targets: &target(Some(PREMUL)),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let post_layout = layout(&[&bgl_post]);
        let post = |entry: &str, format: wgpu::TextureFormat| {
            let _creation = crate::gpu::creation_lock();
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&post_layout),
                vertex: wgpu::VertexState {
                    module: &post_mod,
                    entry_point: Some("vs_post"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &post_mod,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let blit_pipe = post("fs_blit", FORMAT);
        let ssao_pipe = post("fs_ssao", FORMAT);
        let ao_blur_pipe = post("fs_ao_blur", FORMAT);
        let ssr_mod = module(ssr_src(), "three-ssr");
        let unf = wgpu::TextureSampleType::Float { filterable: false };
        let bgl_ssr = bgl(
            &[
                buf_entry(0, VS_FS, uni, false),
                tex_entry(1, VS_FS, F, D2, false),
                tex_entry(2, VS_FS, unf, D2, false),
                tex_entry(3, VS_FS, unf, D2, false),
                tex_entry(4, VS_FS, F, D2, false),
                smp_entry(5, VS_FS, filt),
                smp_entry(6, VS_FS, filt),
            ],
            "three-ssr",
        );
        let ssr_layout = layout(&[&bgl_ssr]);
        let ssr_pipe = {
            let _creation = crate::gpu::creation_lock();
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("three-ssr"),
                layout: Some(&ssr_layout),
                vertex: wgpu::VertexState {
                    module: &ssr_mod,
                    entry_point: Some("vs_ssr"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &ssr_mod,
                    entry_point: Some("fs_ssr"),
                    compilation_options: Default::default(),
                    targets: &[Some(FORMAT.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let under_pipe = post("fs_under", FORMAT);
        let dof_pipe = post("fs_dof", FORMAT);
        let tile_format = if scalar == wgpu::TextureFormat::R32Float
            && matches!(gbuffer, wgpu::TextureFormat::Rg32Float | wgpu::TextureFormat::Rgba32Float)
        {
            gbuffer
        } else {
            scalar
        };
        let tile_max_pipe = post("fs_tile_max", tile_format);
        let tile_dilate_pipe = post("fs_tile_dilate", tile_format);
        let depth_layout = layout(&[&bgl_depth]);
        let depth_pipeline = |entry: &str, target: wgpu::TextureFormat| {
            let _creation = crate::gpu::creation_lock();
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("three-depth-resolve"),
                layout: Some(&depth_layout),
                vertex: wgpu::VertexState {
                    module: &depth_mod,
                    entry_point: Some("vs_post"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &depth_mod,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(target.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let depth_pipe = depth_pipeline("fs_depth_resolve", scalar);
        let depth_coc_pipe = depth_pipeline("fs_depth_coc", gbuffer);
        let sort_layout = layout(&[&bgl_sort]);
        let sort_pipes = ["cs_keys", "cs_hist", "cs_scan", "cs_scatter"].map(|e| {
            let _creation = crate::gpu::creation_lock();
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(e),
                layout: Some(&sort_layout),
                module: &sort_mod,
                entry_point: Some(e),
                compilation_options: Default::default(),
                cache: None,
            })
        });
        let repeat_smp = d.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let clamp_smp = d.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let cmp_smp = d.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let mut eng = ThreeEngine {
            gbuffer,
            scalar,
            targets: Default::default(),
            white: Arc::new(TexGpu {
                view: texture(d, [1, 1], wgpu::TextureFormat::Rgba8Unorm, 1, 1, 1, "white")
                    .create_view(&Default::default()),
                key: 0,
                size: [1, 1],
                rgba: Arc::from([255u8; 4]),
                srgb: false,
                sampler: None,
            }),
            brdf: texture(d, [1, 1], FORMAT, 1, 1, 1, "brdf").create_view(&Default::default()),
            black_env: texture(d, [1, 1], FORMAT, 1, 1, 1, "black-env").create_view(&Default::default()),
            device: device.clone(),
            queue,
            main_mod,
            #[cfg(test)]
            specialize_plain: true,
            #[cfg(test)]
            specialize_alpha_mask: true,
            specialize_global_lights: true,
            bgl_frame,
            bgl_mat,
            bgl_obj,
            bgl_splat,
            bgl_ssr,
            pt: None,
            pt_geometry: Default::default(),
            geodesic: None,
            pre_pipes,
            ssao_pipe,
            ao_blur_pipe,
            ssr_pipe,
            bgl_post,
            bgl_depth,
            bgl_sort,
            main_layout,
            pipes: HashMap::new(),
            shadow_pipe,
            dome_pipe,
            splat_pipe,
            blit_pipe,
            under_pipe,
            dof_pipe,
            dof_kernel_pipe: None,
            tile_max_pipe,
            tile_dilate_pipe,
            depth_pipe,
            depth_coc_pipe,
            sort_pipes,
            software_scatter: None,
            sorted_splats: Default::default(),
            software_adapter: false,
            repeat_smp,
            clamp_smp,
            cmp_smp,
            mat_binds: HashMap::new(),
            meshes: HashMap::new(),
            param_meshes: HashMap::new(),
            textures: HashMap::new(),
            drape_order: Default::default(),
            envs: HashMap::new(),
            splat_cache: HashMap::new(),
            next_key: 1,
            ies_rows: HashMap::new(),
            stats: Stats3::default(),
            time_gpu: false,
            pt_timers: Vec::new(),
            raster_timers: Vec::new(),
        };
        // 1×1 white and the BRDF table
        let white = eng.upload_rgba8(1, 1, &[255, 255, 255, 255], false);
        eng.white = white;
        let n = 32;
        let lut: Vec<u16> = sr_3d::env::brdf_lut(n).iter().flat_map(|p| half4([p[0], p[1], 0.0, 1.0])).collect();
        let t = texture(&eng.device, [n as u32, n as u32], FORMAT, 1, 1, 1, "brdf");
        eng.write_tex(&t, 0, [n as u32, n as u32], bytemuck::cast_slice(&lut), 8);
        eng.brdf = t.create_view(&Default::default());
        eng
    }

    fn write_tex(&self, t: &wgpu::Texture, mip: u32, size: [u32; 2], data: &[u8], bpp: u32) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: t,
                mip_level: mip,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * bpp),
                rows_per_image: Some(size[1]),
            },
            wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        );
    }

    fn pipe(&mut self, key: PipeKey) -> &wgpu::RenderPipeline {
        let (d, m, l) = (&self.device, &self.main_mod, &self.main_layout);
        self.pipes.entry(key).or_insert_with(|| {
            {
                let _creation = crate::gpu::creation_lock();
                d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("three-main"),
                    layout: Some(l),
                    vertex: wgpu::VertexState {
                        module: m,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[Some(vertex_layout())],
                    },
                    // (b − a) × (c − a) points outward; after the projection's y flip those triangles wind counter-clockwise in NDC
                    primitive: wgpu::PrimitiveState {
                        front_face: wgpu::FrontFace::Ccw,
                        cull_mode: if key.cull { Some(wgpu::Face::Back) } else { None },
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: Some(!key.blend),
                        depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: wgpu::MultisampleState { count: MSAA, ..Default::default() },
                    fragment: Some(wgpu::FragmentState {
                        module: m,
                        entry_point: Some("fs_main"),
                        compilation_options: wgpu::PipelineCompilationOptions {
                            constants: &[
                                ("PLAIN_MATERIAL", key.plain as u32 as f64),
                                ("ALL_GLOBAL_LIGHTS", key.global_lights as u32 as f64),
                                ("LIGHT_SIGNATURE", key.light_signature as f64),
                                ("ALPHA_MASK", key.alpha_mask as u32 as f64),
                            ],
                            ..Default::default()
                        },
                        targets: &[Some(wgpu::ColorTargetState {
                            format: FORMAT,
                            blend: if key.blend { Some(PREMUL) } else { None },
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            }
        })
    }

    /// Uploads an RGBA8 image with a CPU-built mip chain, reduced to the device's texture
    /// limit (8192 at most, as 2D images are) when it is larger.
    pub fn upload_rgba8(&mut self, w: u32, h: u32, rgba: &[u8], srgb: bool) -> Arc<TexGpu> {
        let cap = self.device.limits().max_texture_dimension_2d.min(8192);
        let mut img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).unwrap_or_else(|| image::RgbaImage::new(w, h));
        let [w, h] = fit_texture(w.max(1), h.max(1), cap);
        if img.dimensions() != (w, h) {
            img = image::imageops::thumbnail(&img, w, h);
        }
        let rgba: Arc<[u8]> = img.as_raw().clone().into();
        let mips = 32 - w.max(h).max(1).leading_zeros();
        let format = if srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
        let t = texture(&self.device, [w, h], format, mips, 1, 1, "material-map");
        for level in 0..mips {
            let (lw, lh) = ((w >> level).max(1), (h >> level).max(1));
            if level > 0 {
                img = image::imageops::resize(&img, lw, lh, image::imageops::FilterType::Triangle);
            }
            self.write_tex(&t, level, [lw, lh], img.as_raw(), 4);
        }
        self.next_key += 1;
        Arc::new(TexGpu {
            view: t.create_view(&Default::default()),
            key: self.next_key,
            size: [w, h],
            rgba,
            srgb,
            sampler: None,
        })
    }

    /// Uploads a decoded material texture, retaining its explicit sampling contract.
    pub fn upload_texture(&mut self, texture: &sr_3d::Texture) -> Arc<TexGpu> {
        let mut gpu = self.upload_rgba8(texture.width, texture.height, &texture.rgba, texture.srgb);
        Arc::get_mut(&mut gpu).expect("new texture").sampler = texture.sampler;
        gpu
    }

    /// Uploads a mesh.
    pub fn upload_mesh(&self, vertices: &[Vertex], indices: &[u32]) -> Arc<MeshGpu> {
        self.upload_mesh_owned(vertices.to_vec(), indices.to_vec())
    }

    /// As [`Self::upload_mesh`], keeping the vectors as the host copy instead of copying them.
    pub fn upload_mesh_owned(&self, vertices: Vec<Vertex>, indices: Vec<u32>) -> Arc<MeshGpu> {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for v in &vertices {
            lo = lo.min(Vec3::from(v.pos));
            hi = hi.max(Vec3::from(v.pos));
        }
        if vertices.is_empty() {
            (lo, hi) = (Vec3::ZERO, Vec3::ZERO);
        }
        let vbuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh-v"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh-i"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Arc::new(MeshGpu { vbuf, ibuf, count: indices.len() as u32, cpu: Arc::new((vertices, indices)), lo, hi })
    }

    /// Prefilters and uploads an environment.
    pub fn upload_env(&self, eq: &sr_3d::env::Equirect) -> Arc<EnvGpu> {
        // lighting uses a 512-wide prefiltered chain; the visible sky keeps the source's resolution
        let base = eq.width.clamp(64, 512).next_power_of_two().min(512);
        let levels = (base.trailing_zeros() as usize).saturating_sub(2).clamp(2, 7);
        let pf = sr_3d::env::prefilter(eq, base, levels);
        let t = texture(&self.device, [pf.mips[0].0, pf.mips[0].1], FORMAT, levels as u32, 1, 1, "environment");
        for (k, (w, h, px)) in pf.mips.iter().enumerate() {
            let data: Vec<u16> = px.iter().flat_map(|p| half4(*p)).collect();
            self.write_tex(&t, k as u32, [*w, *h], bytemuck::cast_slice(&data), 8);
        }
        let cap = self.device.limits().max_texture_dimension_2d.min(8192);
        let resized;
        let src = if eq.width > cap || eq.height > cap {
            let w = cap.min(eq.width * cap / eq.height.max(1)).max(1);
            resized = eq.resized(w, (eq.height * w / eq.width).max(1));
            &resized
        } else {
            eq
        };
        let sky = texture(&self.device, [src.width, src.height], FORMAT, 1, 1, 1, "sky");
        let data: Vec<u16> = src.rgb.iter().flat_map(|p| half4([p[0], p[1], p[2], 1.0])).collect();
        self.write_tex(&sky, 0, [src.width, src.height], bytemuck::cast_slice(&data), 8);
        Arc::new(EnvGpu {
            view: t.create_view(&Default::default()),
            sky: sky.create_view(&Default::default()),
            mips: levels as u32,
            sh: pf.sh,
        })
    }

    /// Uploads splats: position and opacity, the covariance R S Sᵀ Rᵀ, colour. A set larger
    /// than one storage binding of the device is cut to what fits ([`splat_note`] reports it).
    pub fn upload_splats(&self, s: &sr_3d::Splats) -> Arc<SplatGpu> {
        let sh = s.sh_degree > 0 && s.sh.len() == s.len();
        self.upload_splats_within(s, splat_capacity(&self.device.limits(), sh).min(u32::MAX as u64) as usize)
    }

    /// [`upload_splats`](Self::upload_splats) of the first `cap` splats.
    pub fn upload_splats_within(&self, s: &sr_3d::Splats, cap: usize) -> Arc<SplatGpu> {
        let n = s.len().min(cap);
        let mut data: Vec<[f32; 16]> = Vec::with_capacity(n);
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for i in 0..n {
            let p = Vec3::from(s.pos[i]);
            lo = lo.min(p);
            hi = hi.max(p);
            let r = glam::Mat3::from_quat(glam::Quat::from_array(s.rot[i]));
            let m = r * glam::Mat3::from_diagonal(Vec3::from(s.scale[i]));
            let c = m * m.transpose();
            let col = s.color[i];
            data.push([
                p.x, p.y, p.z, col[3], c.x_axis.x, c.y_axis.x, c.z_axis.x, c.y_axis.y, c.z_axis.y, c.z_axis.z, 0.0,
                0.0, col[0], col[1], col[2], 1.0,
            ]);
        }
        if data.is_empty() {
            data.push([0.0; 16]);
            (lo, hi) = (Vec3::ZERO, Vec3::ZERO);
        }
        let buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("splats"),
            contents: bytemuck::cast_slice(&data),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let has_sh = s.sh_degree > 0 && s.sh.len() == s.len() && n > 0;
        let dummy = [[0.0f32; 48]];
        let sh = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("splat-sh"),
            contents: bytemuck::cast_slice(if has_sh { &s.sh[..n] } else { &dummy[..] }),
            usage: wgpu::BufferUsages::STORAGE,
        });
        Arc::new(SplatGpu {
            buf,
            sh,
            sh_degree: if has_sh { s.sh_degree.min(3) } else { 0 },
            n: n as u32,
            lo,
            hi,
            cpu: data.into(),
            cpu_sh: if has_sh { s.sh[..n].to_vec().into() } else { Arc::from([]) },
        })
    }

    fn mat_bind(&mut self, maps: &Maps, uniform: &wgpu::Buffer) -> [u64; 6] {
        let key = std::array::from_fn(|k| maps[k].as_ref().map(|t| t.key).unwrap_or(0));
        if !self.mat_binds.contains_key(&key) {
            let views: Vec<&wgpu::TextureView> =
                (0..6).map(|k| maps[k].as_ref().map(|t| &t.view).unwrap_or(&self.white.view)).collect();
            let mut entries = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: uniform,
                    offset: 0,
                    size: std::num::NonZeroU64::new(std::mem::size_of::<MaterialU>() as u64),
                }),
            }];
            for (k, v) in views.iter().enumerate() {
                entries.push(wgpu::BindGroupEntry {
                    binding: k as u32 + 1,
                    resource: wgpu::BindingResource::TextureView(v),
                });
            }
            entries
                .push(wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&self.repeat_smp) });
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("three-material"),
                layout: &self.bgl_mat,
                entries: &entries,
            });
            self.mat_binds.insert(key, bg);
        }
        key
    }
}

/// A shadow view: its matrix (standard depth, 0..1).
fn shadow_proj(fov_deg: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_deg.to_radians() * 0.5).tan();
    let (n, fa) = (near, far.max(near + 1.0));
    Mat4::from_cols_array(&[
        f,
        0.0,
        0.0,
        0.0,
        0.0,
        f,
        0.0,
        0.0,
        0.0,
        0.0,
        fa / (fa - n),
        1.0,
        0.0,
        0.0,
        -fa * n / (fa - n),
        0.0,
    ])
}

/// View matrix looking from `eye` along `dir` (camera convention: z forward, y down).
fn look(eye: Vec3, dir: Vec3) -> Mat4 {
    let f = dir.normalize_or(Vec3::Z);
    let hint = if f.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let r = hint.cross(f).normalize() * -1.0;
    let d = f.cross(r).normalize() * -1.0;
    Mat4::from_cols(r.extend(0.0), d.extend(0.0), f.extend(0.0), eye.extend(1.0)).inverse()
}

fn corners(lo: Vec3, hi: Vec3) -> [Vec3; 8] {
    std::array::from_fn(|k| {
        Vec3::new(
            if k & 1 == 0 { lo.x } else { hi.x },
            if k & 2 == 0 { lo.y } else { hi.y },
            if k & 4 == 0 { lo.z } else { hi.z },
        )
    })
}

/// Conservatively rejects object bounds outside a standard-depth clip volume.
fn bounds_outside_clip(lo: Vec3, hi: Vec3, model: Mat4, clip: Mat4) -> bool {
    if !lo.is_finite() || !hi.is_finite() || lo.cmpgt(hi).any() {
        return false;
    }
    let abs = |m: Mat4| Mat4::from_cols(m.x_axis.abs(), m.y_axis.abs(), m.z_axis.abs(), m.w_axis.abs());
    let (model_abs, clip_abs) = (abs(model), abs(clip));
    let mut outside = 0b11_1111u8;
    for corner in corners(lo, hi) {
        let local = corner.extend(1.0);
        // Match the vertex shader's two transforms. Bound cancellation error
        // with absolute products, rather than the potentially tiny result.
        let p = clip * (model * local);
        if !p.is_finite() {
            return false;
        }
        let magnitude = (clip_abs * (model_abs * local.abs())).max_element();
        let margin = 64.0 * f32::EPSILON * magnitude.max(1.0);
        let planes = [p.x + p.w, p.w - p.x, p.y + p.w, p.w - p.y, p.z, p.w - p.z];
        let mut bits = 0;
        for (k, distance) in planes.into_iter().enumerate() {
            if distance < -margin {
                bits |= 1 << k;
            }
        }
        outside &= bits;
        if outside == 0 {
            return false;
        }
    }
    outside != 0
}

/// Screen tile range touched by a light (inclusive), or `None` when it is off screen.
fn light_tiles(l: &Light3, vp: &Mat4, size: [u32; 2], tiles: [u32; 2]) -> Option<[u32; 4]> {
    let all = Some([0, 0, tiles[0] - 1, tiles[1] - 1]);
    if matches!(l.kind, LightKind::Ambient | LightKind::Directional) || l.range <= 0.0 {
        return all;
    }
    let r = l.range + l.size[0].max(l.size[1]).max(l.size[2]);
    let (mut lo, mut hi) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
    for c in corners(l.pos - Vec3::splat(r), l.pos + Vec3::splat(r)) {
        let q = *vp * c.extend(1.0);
        if q.w <= 1e-3 {
            return all;
        }
        let ndc = glam::Vec2::new(q.x / q.w, q.y / q.w);
        let px = glam::Vec2::new((ndc.x * 0.5 + 0.5) * size[0] as f32, (0.5 - ndc.y * 0.5) * size[1] as f32);
        lo = lo.min(px);
        hi = hi.max(px);
    }
    if hi.x < 0.0 || hi.y < 0.0 || lo.x >= size[0] as f32 || lo.y >= size[1] as f32 {
        return None;
    }
    let t = |v: f32, n: u32| ((v.max(0.0) as u32) / TILE).min(n - 1);
    Some([t(lo.x, tiles[0]), t(lo.y, tiles[1]), t(hi.x, tiles[0]), t(hi.y, tiles[1])])
}

impl ThreeEngine {
    fn ies_atlas(&mut self, lights: &[Light3]) -> (wgpu::TextureView, Vec<f32>) {
        let mut rows: Vec<Arc<Vec<f32>>> = Vec::new();
        let mut index = Vec::new();
        self.ies_rows.clear();
        for l in lights {
            match &l.ies {
                Some(t) => {
                    let key = Arc::as_ptr(t) as usize;
                    let row = *self.ies_rows.entry(key).or_insert_with(|| {
                        rows.push(t.clone());
                        rows.len() - 1
                    });
                    index.push(row as f32);
                }
                None => index.push(-1.0),
            }
        }
        let h = (rows.len().max(1) * IES_ROWS) as u32;
        let tex = texture(&self.device, [IES_W as u32, h], FORMAT, 1, 1, 1, "ies");
        let mut data = vec![0u16; IES_W * h as usize * 4];
        for (r, t) in rows.iter().enumerate() {
            for (k, v) in t.iter().enumerate().take(IES_W * IES_ROWS) {
                let o = (r * IES_W * IES_ROWS + k) * 4;
                data[o..o + 4].copy_from_slice(&half4([*v, *v, *v, 1.0]));
            }
        }
        self.write_tex(&tex, 0, [IES_W as u32, h], bytemuck::cast_slice(&data), 8);
        (tex.create_view(&Default::default()), index)
    }

    /// Bakes an IES profile into the table this engine samples.
    pub fn bake_ies(ies: &sr_3d::light::Ies) -> Arc<Vec<f32>> {
        Arc::new(ies.bake(IES_W, IES_ROWS))
    }

    /// Records the 3D pass into `enc`, writing premultiplied linear colour into `out`
    /// (same size as the scene). `backdrop` is what lies behind (for transmission).
    /// Invalid or over-budget volumes return an error before recording a pass.
    pub fn render(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        scene: &Scene3,
        backdrop: Option<&wgpu::TextureView>,
        out: &wgpu::TextureView,
    ) -> Result<(), String> {
        if let Some(geo) = &scene.geodesic {
            let pass = self.geodesic.get_or_insert_with(|| crate::geodesic::GeodesicGpu::new(&self.device, FORMAT));
            pass.render(&self.device, enc, geo, crate::geodesic::Output::Picture, None, out);
            self.stats = Stats3::default();
            return Ok(());
        }
        let limits = self.device.limits();
        let path = scene.path.or_else(|| {
            (!scene.volumes.is_empty()).then_some(crate::pathtrace::PathOpts { samples: 4, bounces: 2, denoise: true })
        });
        let note = path.and_then(|_| crate::pathtrace::limit_note(scene, &limits));
        if !scene.volumes.is_empty() {
            if let Some(note) = &note {
                return Err(note.clone());
            }
        }
        // a scene too large for the tracer's buffers is rasterised (the caller reports `limit_note`)
        if let Some(opts) = path.filter(|_| note.is_none()) {
            let (data, ticket) = self.pt_geometry.build_for_render(scene);
            if self.pt.is_none() {
                self.pt = Some(crate::pathtrace::PtGpu::new(&self.device, FORMAT));
            }
            let env_view = scene.env.as_ref().map(|e| &e.env.view).unwrap_or(&self.black_env);
            let inputs = crate::pathtrace::PtInputs {
                env: env_view,
                backdrop,
                black: &self.black_env,
                sampler: &self.repeat_smp,
            };
            let timing = crate::pathtrace::render_cached(
                self.pt.as_ref().expect("built"),
                &self.device,
                enc,
                scene,
                &data,
                opts,
                &inputs,
                out,
                self.time_gpu,
                ticket.as_ref().map(|t| t.id),
            );
            self.pt_timers.extend(timing.gpu);
            self.stats = Stats3 {
                draws: scene.draws.len(),
                triangles: scene.draws.iter().map(Draw3::mesh_triangles).sum::<u64>()
                    + scene.splats.iter().map(|s| u64::from(s.gpu.n)).sum::<u64>(),
                pt_assemble_seconds: data.timing.assemble_seconds,
                pt_bvh_seconds: data.timing.bvh_seconds,
                pt_pack_seconds: timing.pack_seconds,
                pt_geometry_uploads: timing.geometry_uploads,
                ..Default::default()
            };
            self.pt_geometry.recycle(data, ticket);
            return Ok(());
        }
        let d = self.device.clone();
        let size = [scene.size[0].max(1), scene.size[1].max(1)];
        let targets = self.targets.clone();
        let pool = |size: [u32; 2], format, mips, samples, layers, label| {
            targets.lock().unwrap_or_else(|e| e.into_inner()).take(&d, (size, format, mips, samples, layers, label))
        };

        let mut stats = Stats3::default();
        let vp = scene.clip_fix * scene.cam.view_proj();
        // ------------------------------------------------ lights, IES, shadow views
        let (ies_view, ies_rows) = self.ies_atlas(&scene.lights);
        // Skinning, morphs and terrain deformation can move vertices outside the
        // uploaded mesh. Compute current bounds once for lighting and depth sorting.
        let draw_bounds: Vec<_> = scene.draws.iter().map(|dr| dr.mesh.bounds()).collect();
        let (mut blo, mut bhi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for (dr, &(lo, hi)) in scene.draws.iter().zip(&draw_bounds) {
            for c in corners(lo, hi) {
                let w = dr.model.transform_point3(c);
                blo = blo.min(w);
                bhi = bhi.max(w);
            }
        }
        for sp in &scene.splats {
            for c in corners(sp.gpu.lo, sp.gpu.hi) {
                let w = sp.model.transform_point3(c);
                blo = blo.min(w);
                bhi = bhi.max(w);
            }
        }
        if blo.x > bhi.x {
            (blo, bhi) = (Vec3::ZERO, Vec3::ONE);
        }
        let extent = (bhi - blo).length().max(1.0);
        // cascaded shadow maps for directional lights: the view frustum out to the scene's
        // farthest point, split in depth (the practical split scheme: a blend of logarithmic
        // and uniform splits), each slice as world-space corners with its far distance
        const CASCADES: usize = 4;
        let cascades: Option<Vec<([Vec3; 8], f32)>> = (!scene.cam.orthographic).then(|| {
            let inv = scene.cam.view.inverse();
            let near = scene.cam.near.max(1e-3);
            let far = corners(blo, bhi)
                .iter()
                .map(|c| scene.cam.view.transform_point3(*c).z)
                .fold(near * 2.0, f32::max)
                .min(scene.cam.far);
            let (hw, hh) = (size[0] as f32 * 0.5 / scene.cam.focal_px, size[1] as f32 * 0.5 / scene.cam.focal_px);
            let split = |i: usize| {
                let f = i as f32 / CASCADES as f32;
                0.8 * near * (far / near).powf(f) + 0.2 * (near + (far - near) * f)
            };
            (0..CASCADES)
                .map(|c| {
                    let mut out = [Vec3::ZERO; 8];
                    for (k, z) in [split(c), split(c + 1)].into_iter().enumerate() {
                        for (j, (sx, sy)) in
                            [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)].into_iter().enumerate()
                        {
                            out[k * 4 + j] = inv.transform_point3(Vec3::new(sx * hw * z, sy * hh * z, z));
                        }
                    }
                    (out, split(c + 1))
                })
                .collect()
        });
        let casters = shadow_casters(&scene.lights, cascades.is_some(), limits.max_texture_array_layers);
        let mut shadow_mats: Vec<Mat4> = Vec::new();
        let mut map_size = 16u32;
        let mut lights_u: Vec<LightU> = Vec::with_capacity(scene.lights.len());
        for (li, l) in scene.lights.iter().enumerate() {
            let mut first = -1.0;
            let mut views = 1.0;
            // cascade split depths of a directional light (view z where cascades 0–2 end)
            let mut splits = [0.0f32; 3];
            if casters[li] {
                first = shadow_mats.len() as f32;
                map_size = map_size.max(l.map_size.clamp(16, 8192));
                match l.kind {
                    LightKind::Directional => {
                        let v = look(Vec3::ZERO, l.dir);
                        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                        for c in corners(blo, bhi) {
                            let q = v.transform_point3(c);
                            lo = lo.min(q);
                            hi = hi.max(q);
                        }
                        // one ortho view over light-space box [lo, hi], padded
                        let ortho_of = |lo: Vec3, hi: Vec3| {
                            let side = (hi.x - lo.x).max(hi.y - lo.y);
                            let pad = Vec3::new(side * 0.02, side * 0.02, extent * 0.02);
                            let (lo, hi) = (lo - pad, hi + pad);
                            Mat4::from_cols_array(&[
                                2.0 / (hi.x - lo.x),
                                0.0,
                                0.0,
                                0.0,
                                0.0,
                                2.0 / (hi.y - lo.y),
                                0.0,
                                0.0,
                                0.0,
                                0.0,
                                1.0 / (hi.z - lo.z),
                                0.0,
                                -(hi.x + lo.x) / (hi.x - lo.x),
                                -(hi.y + lo.y) / (hi.y - lo.y),
                                -lo.z / (hi.z - lo.z),
                                1.0,
                            ]) * v
                        };
                        match &cascades {
                            // each cascade covers its slice across the light (within the scene),
                            // and the whole scene along it, so casters outside the slice still shade it
                            Some(cs) => {
                                views = CASCADES as f32;
                                for (k, (slice, far)) in cs.iter().enumerate() {
                                    let (mut vlo, mut vhi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                                    for c in slice {
                                        let q = v.transform_point3(*c);
                                        vlo = vlo.min(q);
                                        vhi = vhi.max(q);
                                    }
                                    let (nlo, nhi) = (lo.max(vlo), hi.min(vhi));
                                    let (clo, chi) = if nlo.x < nhi.x && nlo.y < nhi.y {
                                        (Vec3::new(nlo.x, nlo.y, lo.z), Vec3::new(nhi.x, nhi.y, hi.z))
                                    } else {
                                        (lo, hi)
                                    };
                                    shadow_mats.push(ortho_of(clo, chi));
                                    if k < 3 {
                                        splits[k] = *far;
                                    }
                                }
                            }
                            None => shadow_mats.push(ortho_of(lo, hi)),
                        }
                    }
                    LightKind::Spot => {
                        let far = if l.range > 0.0 { l.range } else { (l.pos - (blo + bhi) * 0.5).length() + extent };
                        let fov = (2.0 * l.cos_outer.clamp(-1.0, 1.0).acos().to_degrees() + 4.0).min(170.0);
                        shadow_mats.push(shadow_proj(fov, 1.0, far) * look(l.pos, l.dir));
                    }
                    _ => {
                        let far = if l.range > 0.0 { l.range } else { (l.pos - (blo + bhi) * 0.5).length() + extent };
                        views = 6.0;
                        for dir in [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z] {
                            shadow_mats.push(shadow_proj(90.0, 1.0, far) * look(l.pos, dir));
                        }
                    }
                }
            }
            lights_u.push(LightU {
                pos: [l.pos.x, l.pos.y, l.pos.z, l.kind as u32 as f32],
                dir: [l.dir.x, l.dir.y, l.dir.z, l.range],
                color: [l.color.x, l.color.y, l.color.z, l.falloff],
                spot: [l.cos_outer, l.cos_inner, first, l.softness.max(1.0)],
                // directional lights have no size: their cascade splits ride in it
                size: if l.kind == LightKind::Directional && views > 1.5 {
                    [splits[0], splits[1], splits[2], ies_rows[li]]
                } else {
                    [l.size[0], l.size[1], l.size[2], ies_rows[li]]
                },
                flags: [l.affects_diffuse as u32 as f32, l.affects_specular as u32 as f32, l.bias, views],
                right: [l.right.x, l.right.y, l.right.z, l.contact.max(0.0)],
            });
        }
        if lights_u.is_empty() {
            lights_u.push(LightU::zeroed());
        }
        // shadow map budget
        let layers = shadow_mats.len().max(1) as u32;
        while map_size > 64 && (map_size as u64).pow(2) * 4 * layers as u64 > SHADOW_BUDGET {
            map_size /= 2;
        }
        stats.shadow_views = shadow_mats.len();
        let mut timer = self.time_gpu.then(|| crate::fx::Timer::new(&d, shadow_mats.len() as u32 + 32));
        // ------------------------------------------------ tiles
        let tiles = [size[0].div_ceil(TILE), size[1].div_ceil(TILE)];
        // Every global light appears in every tile in scene order. Keep the
        // original 63-light cap and overflow diagnostic without a repeated list.
        let global_lights = self.specialize_global_lights
            && scene.lights.iter().all(|l| matches!(l.kind, LightKind::Ambient | LightKind::Directional));
        // A small global-light list can preserve order while exposing each kind
        // and index to the shader compiler. Longer lists retain the tiled/general path.
        let light_signature = if global_lights && scene.lights.len() <= 4 {
            ((scene.lights.len() as u32) << 8)
                | scene
                    .lights
                    .iter()
                    .enumerate()
                    .fold(0, |mask, (k, light)| mask | ((matches!(light.kind, LightKind::Directional) as u32) << k))
        } else {
            u32::MAX
        };
        let tile_data = if global_lights {
            stats.tile_overflow = scene.lights.len() > MAX_PER_TILE;
            vec![0u32]
        } else {
            let mut tile_data = vec![0u32; (tiles[0] * tiles[1]) as usize * (MAX_PER_TILE + 1)];
            for (li, l) in scene.lights.iter().enumerate() {
                let Some([x0, y0, x1, y1]) = light_tiles(l, &vp, size, tiles) else { continue };
                for ty in y0..=y1 {
                    for tx in x0..=x1 {
                        let base = (ty * tiles[0] + tx) as usize * (MAX_PER_TILE + 1);
                        let n = tile_data[base] as usize;
                        if n < MAX_PER_TILE {
                            tile_data[base + 1 + n] = li as u32;
                            tile_data[base] += 1;
                        } else {
                            stats.tile_overflow = true;
                        }
                    }
                }
            }
            tile_data
        };
        // ------------------------------------------------ frame uniform
        let env = scene.env.as_ref();
        let mut sh = [[0.0f32; 4]; 9];
        if let Some(e) = env {
            for (dst, src) in sh.iter_mut().zip(&e.env.sh) {
                *dst = [src[0], src[1], src[2], 0.0];
            }
        }
        // Compute CoC only when both original depth and paired storage retain f32 precision.
        let precompute_coc = scene.dof.is_some()
            && self.scalar == wgpu::TextureFormat::R32Float
            && matches!(self.gbuffer, wgpu::TextureFormat::Rg32Float | wgpu::TextureFormat::Rgba32Float);
        let dof = scene.dof.unwrap_or(Dof { coc_scale: 0.0, focus: 1.0, max_coc: 0.0, blades: 0 });
        let frame = FrameU {
            view_proj: vp.to_cols_array_2d(),
            view: scene.cam.view.to_cols_array_2d(),
            inv_view_proj: vp.inverse().to_cols_array_2d(),
            eye: scene.cam.eye.extend(1.0).to_array(),
            screen: [size[0] as f32, size[1] as f32, 1.0 / size[0] as f32, 1.0 / size[1] as f32],
            params: [
                scene.exposure,
                scene.lights.len() as f32,
                tiles[0] as f32,
                env.map(|e| e.intensity).unwrap_or(0.0),
            ],
            params2: [
                0.0,
                env.map(|e| e.visible as u32 as f32).unwrap_or(0.0),
                env.is_some() as u32 as f32,
                env.map(|e| e.env.mips as f32).unwrap_or(1.0),
            ],
            dof: [dof.coc_scale, dof.focus.max(1e-3), dof.max_coc.min(64.0), dof.blades as f32],
            post: [scene.lens_k1, scene.dof.is_some() as u32 as f32, scene.cam.near, scene.cam.far],
            lens: [
                scene.clip_fix.x_axis.x * scene.cam.proj.x_axis.x * size[0] as f32 * 0.5,
                scene.encode_srgb as u32 as f32,
                scene.ao.is_some() as u32 as f32,
                0.0,
            ],
            fx: [
                scene.ao.map(|a| a[0]).unwrap_or(0.0),
                scene.ao.map(|a| a[1]).unwrap_or(0.0),
                scene.ssr as u32 as f32,
                precompute_coc as u32 as f32,
            ],
            sh,
            env_rot: env.map(|e| e.rotation.transpose()).unwrap_or(Mat4::IDENTITY).to_cols_array_2d(),
        };
        let buf = |data: &[u8], usage: wgpu::BufferUsages, label: &str| {
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: data, usage })
        };
        let frame_buf = buf(bytemuck::bytes_of(&frame), wgpu::BufferUsages::UNIFORM, "three-frame");
        let light_buf = buf(bytemuck::cast_slice(&lights_u), wgpu::BufferUsages::STORAGE, "three-lights");
        let tile_buf = buf(bytemuck::cast_slice(&tile_data), wgpu::BufferUsages::STORAGE, "three-tiles");
        let mats: Vec<[[f32; 4]; 4]> = if shadow_mats.is_empty() {
            vec![Mat4::IDENTITY.to_cols_array_2d()]
        } else {
            shadow_mats.iter().map(|m| m.to_cols_array_2d()).collect()
        };
        let smat_buf = buf(bytemuck::cast_slice(&mats), wgpu::BufferUsages::STORAGE, "three-shadow-mats");
        let shadow = pool([map_size, map_size], wgpu::TextureFormat::Depth32Float, 1, 1, layers, "three-shadows");
        let shadow_view = shadow.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        // ------------------------------------------------ draws: classify, uniforms
        #[derive(Clone, Copy, PartialEq)]
        enum Kind {
            Opaque,
            Transmissive,
            Blend,
        }
        let mut obj_bytes = Vec::new();
        let mut mat_bytes = Vec::new();
        let mut material_slots = HashMap::<Vec<u8>, u32>::new();
        struct Prep {
            kind: Kind,
            obj: u32,
            mat: u32,
            key: [u64; 6],
            vbuf: Option<wgpu::Buffer>,
            depth: f32,
            pipe: PipeKey,
            visible: bool,
            active: bool,
        }
        let mut preps: Vec<Prep> = Vec::new();
        for (dr, &(lo, hi)) in scene.draws.iter().zip(&draw_bounds) {
            let m = dr.mesh.mesh();
            stats.triangles += m.count as u64 / 3;
            let outside = |matrix| {
                matches!(&dr.mesh, MeshSrc::Cached(mesh)
                if dr.maps[5].is_none() && bounds_outside_clip(mesh.lo, mesh.hi, dr.model, matrix))
            };
            let visible = !outside(vp);
            let casts = dr.cast_shadow
                && !dr.shadow_catcher
                && dr.opacity > 0.0
                && shadow_mats.iter().any(|&matrix| !outside(matrix));
            if !visible && !casts {
                // Keep scene indices stable for batching; no uniforms, materials,
                // pipeline or bindings are prepared for this placeholder.
                preps.push(Prep {
                    kind: Kind::Opaque,
                    obj: 0,
                    mat: 0,
                    key: [0; 6],
                    vbuf: None,
                    depth: 0.0,
                    pipe: PipeKey {
                        cull: false,
                        blend: false,
                        plain: false,
                        global_lights: false,
                        light_signature: u32::MAX,
                        alpha_mask: true,
                    },
                    visible: false,
                    active: false,
                });
                continue;
            }
            stats.prepared_objects += 1;
            let o = ObjectU {
                model: dr.model.to_cols_array_2d(),
                normal: dr.model.inverse().transpose().to_cols_array_2d(),
                // the uniform part of the model matrix: scene units per unit of the mesh (for the unevenness noise)
                params: [
                    dr.opacity,
                    (dr.receive_shadow || dr.shadow_catcher) as u32 as f32,
                    dr.model.x_axis.truncate().length(),
                    dr.shadow_catcher as u32 as f32,
                ],
                spacing: [0.0; 4],
                padding: [[0.; 4]; 6],
            };
            let obj =
                push_object(&mut obj_bytes, &o, limits.max_storage_buffer_binding_size.min(limits.max_buffer_size))?;
            let value = material_u(&dr.material, &dr.maps);
            let bytes = bytemuck::bytes_of(&value);
            let mat = *material_slots.entry(bytes.to_vec()).or_insert_with(|| pad(&mut mat_bytes, bytes));
            let kind = if dr.material.transmission > 0.0 && !dr.material.unlit {
                Kind::Transmissive
            } else if dr.shadow_catcher
                || dr.material.alpha_mode == AlphaMode::Blend
                || dr.opacity < 1.0
                || dr.material.opacity < 1.0
            {
                Kind::Blend
            } else {
                Kind::Opaque
            };
            let plain = plain_material(dr);
            #[cfg(test)]
            let plain = plain && self.specialize_plain;
            // Only masked materials may discard fragments. Specializing this
            // branch leaves opaque depth rejection available to the driver.
            let alpha_mask = dr.material.alpha_mode == AlphaMode::Mask;
            #[cfg(test)]
            let alpha_mask = alpha_mask || !self.specialize_alpha_mask;
            let pipe = PipeKey {
                cull: !dr.material.double_sided,
                blend: kind == Kind::Blend,
                plain,
                global_lights,
                light_signature,
                alpha_mask,
            };
            self.pipe(pipe);
            let center = dr.model.transform_point3(lo * 0.5 + hi * 0.5);
            let vbuf = match &dr.mesh {
                MeshSrc::Deformed(vs, _) => Some(buf(bytemuck::cast_slice(vs), wgpu::BufferUsages::VERTEX, "deformed")),
                MeshSrc::Cached(_) => None,
            };
            preps.push(Prep {
                kind,
                obj,
                mat,
                key: [0; 6],
                vbuf,
                depth: scene.cam.depth_of(center),
                pipe,
                visible,
                active: true,
            });
        }
        stats.draws = preps.len();
        // shadow-pass object uniforms: one per (caster, view)
        let mut shadow_objs: Vec<(usize, u32, u32)> = Vec::new();
        for (v, &shadow_mat) in shadow_mats.iter().enumerate() {
            for (i, dr) in scene.draws.iter().enumerate() {
                if preps[i].active && dr.cast_shadow && !dr.shadow_catcher && dr.opacity > 0.0 {
                    // Cached undeformed bounds are authoritative only without
                    // shader displacement. Other casters keep the full path.
                    if let MeshSrc::Cached(mesh) = &dr.mesh {
                        if dr.maps[5].is_none() && bounds_outside_clip(mesh.lo, mesh.hi, dr.model, shadow_mat) {
                            continue;
                        }
                    }
                    let mut o: ObjectU = *bytemuck::from_bytes(
                        &obj_bytes[preps[i].obj as usize..preps[i].obj as usize + std::mem::size_of::<ObjectU>()],
                    );
                    o.spacing[3] = v as f32;
                    let offset = push_object(
                        &mut obj_bytes,
                        &o,
                        limits.max_storage_buffer_binding_size.min(limits.max_buffer_size),
                    )?;
                    shadow_objs.push((i, v as u32, offset));
                }
            }
        }
        if obj_bytes.is_empty() {
            pad(&mut obj_bytes, &[0u8; 256]);
        }
        if mat_bytes.is_empty() {
            // the dome still binds a material slot when nothing else is drawn
            pad(&mut mat_bytes, &[0u8; std::mem::size_of::<MaterialU>()]);
        }
        if obj_bytes.len() as u64 > limits.max_storage_buffer_binding_size
            || obj_bytes.len() as u64 > limits.max_buffer_size
        {
            return Err("3D instance objects exceed device storage buffer limit".into());
        }
        let obj_buf = buf(&obj_bytes, wgpu::BufferUsages::STORAGE, "three-objects");
        // one material buffer per render, like the object buffer: several 3D passes can be recorded before a
        // single submit, and rewriting a shared buffer with queue.write_buffer would hand every pass the data of
        // the last one. Bind groups reference this buffer, so the cache lives for this render only.
        let mat_buf = buf(&mat_bytes, wgpu::BufferUsages::UNIFORM, "three-materials");
        self.mat_binds.clear();
        for (i, dr) in scene.draws.iter().enumerate() {
            if preps[i].active {
                preps[i].key = self.mat_bind(&dr.maps, &mat_buf);
            }
        }
        if preps.iter().all(|p| !p.active) {
            let none: Maps = Default::default();
            self.mat_bind(&none, &mat_buf);
        }
        let mut order: Vec<usize> = (0..preps.len()).filter(|&i| preps[i].visible).collect();
        order.sort_by(|a, b| preps[*a].depth.total_cmp(&preps[*b].depth));
        let opaque: Vec<usize> = order.iter().copied().filter(|i| preps[*i].kind == Kind::Opaque).collect();
        let mut trans: Vec<usize> = order.iter().copied().filter(|i| preps[*i].kind == Kind::Transmissive).collect();
        let mut blended: Vec<usize> = order.iter().copied().filter(|i| preps[*i].kind == Kind::Blend).collect();
        stats.transmissive = trans.len();
        trans.reverse();
        blended.reverse();
        // Identity slots serve shadow/prepass draws. Sorted slots keep every
        // primary pass's existing order while allowing nonadjacent object storage.
        let mut object_indices: Vec<u32> = (0..(obj_bytes.len() / 256) as u32).collect();
        let mut batches = |list: &[usize]| {
            let mut runs: Vec<(usize, u32, u32)> = Vec::new();
            for &i in list {
                let merge = runs.last().is_some_and(|&(first, _, _)| {
                    preps[i].mat == preps[first].mat
                        && preps[i].key == preps[first].key
                        && preps[i].pipe == preps[first].pipe
                        && matches!((&scene.draws[i].mesh, &scene.draws[first].mesh),
                            (MeshSrc::Cached(a), MeshSrc::Cached(b)) if Arc::ptr_eq(a, b))
                });
                if merge {
                    runs.last_mut().unwrap().2 += 1;
                } else {
                    runs.push((i, object_indices.len() as u32, 1));
                }
                object_indices.push(preps[i].obj / 256);
            }
            runs
        };
        let opaque_runs = batches(&opaque);
        let trans_runs = batches(&trans);
        let blended_runs = batches(&blended);
        stats.draws = opaque_runs.len() + trans_runs.len() + blended_runs.len();
        let index_buf = buf(bytemuck::cast_slice(&object_indices), wgpu::BufferUsages::STORAGE, "three-object-order");
        let mut shadow_runs: Vec<(usize, u32, u32, u32)> = Vec::new();
        for &(i, view, offset) in &shadow_objs {
            let merge = shadow_runs.last().is_some_and(|&(first, v, off, count)| {
                view == v
                    && offset == off + count * 256
                    && preps[i].mat == preps[first].mat
                    && preps[i].key == preps[first].key
                    && matches!((&scene.draws[i].mesh, &scene.draws[first].mesh),
                        (MeshSrc::Cached(a), MeshSrc::Cached(b)) if Arc::ptr_eq(a, b))
            });
            if merge {
                shadow_runs.last_mut().unwrap().3 += 1;
            } else {
                shadow_runs.push((i, view, offset, 1));
            }
        }
        let obj_bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("three-object"),
            layout: &self.bgl_obj,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: obj_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: index_buf.as_entire_binding() },
            ],
        });
        // ------------------------------------------------ targets
        let color_ms = pool(size, FORMAT, 1, MSAA, 1, "three-color-ms").create_view(&Default::default());
        let depth_ms = pool(size, wgpu::TextureFormat::Depth32Float, 1, MSAA, 1, "three-depth-ms")
            .create_view(&Default::default());
        let resolved_t = pool(size, FORMAT, 1, 1, 1, "three-resolved");
        let resolved = resolved_t.create_view(&Default::default());
        let dummy_shadow = pool([1, 1], wgpu::TextureFormat::Depth32Float, 1, 1, 1, "three-no-shadows");
        let dummy_shadow_view = dummy_shadow.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        // the visible sky at the source's resolution, apart from the prefiltered chain
        let sky_view = env.map(|e| &e.env.sky).unwrap_or(&self.black_env);
        let no_tiles = pool([1, 1], self.scalar, 1, 1, 1, "three-no-tiles").create_view(&Default::default());
        let aperture = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("three-dof-aperture"),
            size: 128 * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        if scene.dof.is_some() {
            let pipe = self.dof_kernel_pipe.get_or_insert_with(|| {
                let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("three-dof-aperture"),
                    source: wgpu::ShaderSource::Wgsl(
                        format!("{TYPES}\n{}", include_str!("three_dof_kernel.wgsl")).into(),
                    ),
                });
                let _creation = crate::gpu::creation_lock();
                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("three-dof-aperture"),
                    layout: None,
                    module: &shader,
                    entry_point: Some("cs_aperture"),
                    compilation_options: Default::default(),
                    cache: None,
                })
            });
            let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("three-dof-aperture"),
                layout: &pipe.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: frame_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: aperture.as_entire_binding() },
                ],
            });
            let stamp = timer.as_mut().and_then(|timer| {
                let begin = timer.labelled_pair("three-dof-aperture")?;
                Some(wgpu::ComputePassTimestampWrites {
                    query_set: &timer.set,
                    beginning_of_pass_write_index: Some(begin),
                    end_of_pass_write_index: Some(begin + 1),
                })
            });
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("three-dof-aperture"),
                timestamp_writes: stamp,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(1, 4, 1);
        }
        let post_bind3 = |src: &wgpu::TextureView, aux: &wgpu::TextureView, tiles: &wgpu::TextureView| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("three-post"),
                layout: &self.bgl_post,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: frame_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(aux) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.clamp_smp) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(tiles) },
                    wgpu::BindGroupEntry { binding: 5, resource: aperture.as_entire_binding() },
                ],
            })
        };
        let post_bind = |src: &wgpu::TextureView, aux: &wgpu::TextureView| post_bind3(src, aux, &no_tiles);
        let post_pass = |enc: &mut wgpu::CommandEncoder,
                         pipe: &wgpu::RenderPipeline,
                         bind: &wgpu::BindGroup,
                         dst: &wgpu::TextureView,
                         stamp: Option<wgpu::RenderPassTimestampWrites<'_>>| {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("three-post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: dst,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: stamp,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(pipe);
            rp.set_bind_group(0, bind, &[]);
            rp.draw(0..3, 0..1);
        };
        // ------------------------------------------------ screen-space targets
        let prepass = scene.ao.is_some() || scene.ssr || scene.lights.iter().any(|l| l.contact > 0.0);
        let no_gbuffer = pool([1, 1], self.gbuffer, 1, 1, 1, "three-no-gbuffer").create_view(&Default::default());
        let (gb_n, gb_z, gb_d) = if prepass {
            (
                Some(pool(size, FORMAT, 1, 1, 1, "three-gb-normal").create_view(&Default::default())),
                Some(pool(size, self.gbuffer, 1, 1, 1, "three-gb-depth").create_view(&Default::default())),
                Some(
                    pool(size, wgpu::TextureFormat::Depth32Float, 1, 1, 1, "three-gb-zbuffer")
                        .create_view(&Default::default()),
                ),
            )
        } else {
            (None, None, None)
        };
        let ao_views = scene.ao.map(|_| {
            (
                pool(size, FORMAT, 1, 1, 1, "three-ao-raw").create_view(&Default::default()),
                pool(size, FORMAT, 1, 1, 1, "three-ao").create_view(&Default::default()),
            )
        });
        let ao_view = ao_views.as_ref().map(|a| &a.1).unwrap_or(&self.white.view);
        let gbz_view = gb_z.as_ref().unwrap_or(&no_gbuffer);
        let frame_bind_full = |scene_color: &wgpu::TextureView,
                               env_view: &wgpu::TextureView,
                               shadow_view: &wgpu::TextureView,
                               ao: &wgpu::TextureView,
                               gbz: &wgpu::TextureView| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("three-frame"),
                layout: &self.bgl_frame,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: frame_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: light_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: tile_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(shadow_view) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.cmp_smp) },
                    wgpu::BindGroupEntry { binding: 5, resource: smat_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(env_view) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&self.repeat_smp) },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&self.brdf) },
                    wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&ies_view) },
                    wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(scene_color) },
                    wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::Sampler(&self.clamp_smp) },
                    wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(ao) },
                    wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::TextureView(gbz) },
                    wgpu::BindGroupEntry { binding: 14, resource: wgpu::BindingResource::TextureView(sky_view) },
                ],
            })
        };
        // passes that write the occlusion or the prepass bind stand-ins for them
        let frame_bind_with =
            |scene_color: &wgpu::TextureView, env_view: &wgpu::TextureView, shadow_view: &wgpu::TextureView| {
                frame_bind_full(scene_color, env_view, shadow_view, &self.white.view, &no_gbuffer)
            };
        let frame_bind = |scene_color: &wgpu::TextureView, env_view: &wgpu::TextureView| {
            frame_bind_full(scene_color, env_view, &shadow_view, ao_view, gbz_view)
        };
        let env_view = env.map(|e| &e.env.view).unwrap_or(&self.black_env);
        let fb_plain = frame_bind(&self.white.view, env_view);
        // shadow passes write the shadow array, so they bind a frame group without it
        let fb_shadow = frame_bind_with(&self.white.view, env_view, &dummy_shadow_view);
        // ------------------------------------------------ shadow passes
        for v in 0..shadow_mats.len() {
            let layer = shadow.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: v as u32,
                array_layer_count: Some(1),
                ..Default::default()
            });
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("three-shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &layer,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: raster_stamp(&mut timer, "three-shadow"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.shadow_pipe);
            rp.set_bind_group(0, &fb_shadow, &[]);
            for &(i, vv, off, count) in &shadow_runs {
                if vv as usize != v {
                    continue;
                }
                let dr = &scene.draws[i];
                let m = dr.mesh.mesh();
                rp.set_bind_group(1, &self.mat_binds[&preps[i].key], &[preps[i].mat]);
                rp.set_bind_group(2, &obj_bind, &[]);
                rp.set_vertex_buffer(0, preps[i].vbuf.as_ref().unwrap_or(&m.vbuf).slice(..));
                rp.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                rp.draw_indexed(0..m.count, 0, off / 256..off / 256 + count);
            }
        }
        // ------------------------------------------------ opaque pass
        let draw_list = |rp: &mut wgpu::RenderPass, runs: &[(usize, u32, u32)], eng: &ThreeEngine| {
            for &(i, first, count) in runs {
                let m = scene.draws[i].mesh.mesh();
                rp.set_pipeline(&eng.pipes[&preps[i].pipe]);
                rp.set_bind_group(1, &eng.mat_binds[&preps[i].key], &[preps[i].mat]);
                rp.set_bind_group(2, &obj_bind, &[]);
                rp.set_vertex_buffer(0, preps[i].vbuf.as_ref().unwrap_or(&m.vbuf).slice(..));
                rp.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                rp.draw_indexed(0..m.count, 0, first..first + count);
            }
        };
        // ------------------------------------------------ depth and normal prepass, ambient occlusion
        if let (Some(gn), Some(gz), Some(gd)) = (&gb_n, &gb_z, &gb_d) {
            let fb_pre = frame_bind_with(&self.white.view, env_view, &shadow_view);
            {
                let clear = wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                };
                let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("three-prepass"),
                    color_attachments: &[
                        Some(wgpu::RenderPassColorAttachment {
                            view: gn,
                            resolve_target: None,
                            depth_slice: None,
                            ops: clear,
                        }),
                        Some(wgpu::RenderPassColorAttachment {
                            view: gz,
                            resolve_target: None,
                            depth_slice: None,
                            ops: clear,
                        }),
                    ],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: gd,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: raster_stamp(&mut timer, "three-prepass"),
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                rp.set_bind_group(0, &fb_pre, &[]);
                for &(i, first, count) in &opaque_runs {
                    let dr = &scene.draws[i];
                    let m = dr.mesh.mesh();
                    rp.set_pipeline(&self.pre_pipes[if preps[i].pipe.cull { 0 } else { 1 }]);
                    rp.set_bind_group(1, &self.mat_binds[&preps[i].key], &[preps[i].mat]);
                    rp.set_bind_group(2, &obj_bind, &[]);
                    rp.set_vertex_buffer(0, preps[i].vbuf.as_ref().unwrap_or(&m.vbuf).slice(..));
                    rp.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    rp.draw_indexed(0..m.count, 0, first..first + count);
                }
            }
            if let Some((raw, ao)) = &ao_views {
                post_pass(enc, &self.ssao_pipe, &post_bind(gn, gz), raw, raster_stamp(&mut timer, "three-ssao"));
                post_pass(enc, &self.ao_blur_pipe, &post_bind(raw, gz), ao, raster_stamp(&mut timer, "three-ao-blur"));
            }
        }
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("three-opaque"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_ms,
                    resolve_target: if trans.is_empty() { None } else { Some(&resolved) },
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_ms,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: raster_stamp(&mut timer, "three-opaque"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_bind_group(0, &fb_plain, &[]);
            if env.map(|e| e.visible).unwrap_or(false) {
                rp.set_pipeline(&self.dome_pipe);
                rp.set_bind_group(1, &self.mat_binds[&preps.first().map(|p| p.key).unwrap_or([0; 6])], &[0]);
                rp.set_bind_group(2, &obj_bind, &[]);
                rp.draw(0..3, 0..1);
            }
            draw_list(&mut rp, &opaque_runs, self);
        }
        // ------------------------------------------------ transmission: backdrop + opaque, mip chain
        let mut fb_trans = None;
        if !trans.is_empty() {
            let mips = 32 - size[0].max(size[1]).leading_zeros();
            let mips = mips.min(8);
            let sc = pool(size, FORMAT, mips, 1, 1, "three-scene-color");
            let lvl = |k: u32| {
                sc.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: k,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            };
            let black = pool([1, 1], FORMAT, 1, 1, 1, "three-black").create_view(&Default::default());
            let under = post_bind(&resolved, backdrop.unwrap_or(&black));
            post_pass(enc, &self.under_pipe, &under, &lvl(0), raster_stamp(&mut timer, "three-backdrop"));
            for k in 1..mips {
                let b = post_bind(&lvl(k - 1), &black);
                post_pass(enc, &self.blit_pipe, &b, &lvl(k), raster_stamp(&mut timer, "three-mip"));
            }
            fb_trans = Some(frame_bind(&sc.create_view(&Default::default()), env_view));
        }
        // ------------------------------------------------ splat sort
        if self.software_adapter && !scene.splats.is_empty() && self.software_scatter.is_none() {
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("three-sort-cpu"),
                source: wgpu::ShaderSource::Wgsl(sort_src().into()),
            });
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&self.bgl_sort)],
                immediate_size: 0,
            });
            let _creation = crate::gpu::creation_lock();
            self.software_scatter = Some(d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("cs_scatter_cpu"),
                layout: Some(&layout),
                module: &module,
                entry_point: Some("cs_scatter_cpu"),
                compilation_options: Default::default(),
                cache: None,
            }));
        }
        let mut splat_binds = Vec::new();
        for sp in &scene.splats {
            let n = sp.gpu.n.max(1);
            stats.splats += sp.gpu.n as u64;
            let transform = scene.cam.view * sp.model;
            let view = transform.to_cols_array_2d();
            let key = transform.to_cols_array().map(f32::to_bits);
            let va = if let Some(indices) = self.sorted_splats.get(&sp.gpu, key) {
                indices
            } else {
                stats.splat_sorts += 1;
                let blocks = n.div_ceil(256);
                let mut ubytes = Vec::new();
                for pass in 0..4u32 {
                    let u = SortU { n: sp.gpu.n, blocks, shift: pass * 8, pad: 0, view };
                    pad(&mut ubytes, bytemuck::bytes_of(&u));
                }
                let ubuf = buf(&ubytes, wgpu::BufferUsages::UNIFORM, "sort-params");
                let mk = |label: &str, len: u64| {
                    d.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(label),
                        size: len * 4,
                        usage: wgpu::BufferUsages::STORAGE,
                        mapped_at_creation: false,
                    })
                };
                let (ka, va, kb, vb) =
                    (mk("keys-a", n as u64), mk("vals-a", n as u64), mk("keys-b", n as u64), mk("vals-b", n as u64));
                let hist = mk("hist", 256 * blocks as u64);
                let bind = |ki: &wgpu::Buffer, vi: &wgpu::Buffer, ko: &wgpu::Buffer, vo: &wgpu::Buffer| {
                    d.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("sort"),
                        layout: &self.bgl_sort,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &ubuf,
                                    offset: 0,
                                    size: std::num::NonZeroU64::new(std::mem::size_of::<SortU>() as u64),
                                }),
                            },
                            wgpu::BindGroupEntry { binding: 1, resource: sp.gpu.buf.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 2, resource: ki.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 3, resource: vi.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 4, resource: ko.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 5, resource: vo.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 6, resource: hist.as_entire_binding() },
                        ],
                    })
                };
                let ab = bind(&ka, &va, &kb, &vb);
                let ba = bind(&kb, &vb, &ka, &va);
                let groups = [blocks.min(65535), blocks.div_ceil(65535)];
                {
                    let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("splat-sort"),
                        timestamp_writes: None,
                    });
                    cp.set_pipeline(&self.sort_pipes[0]);
                    cp.set_bind_group(0, &ab, &[0]);
                    cp.dispatch_workgroups(groups[0], groups[1], 1);
                    for pass in 0..4u32 {
                        let bg = if pass % 2 == 0 { &ab } else { &ba };
                        let off = pass * UNIFORM_ALIGN as u32;
                        cp.set_bind_group(0, bg, &[off]);
                        cp.set_pipeline(&self.sort_pipes[1]);
                        cp.dispatch_workgroups(groups[0], groups[1], 1);
                        cp.set_pipeline(&self.sort_pipes[2]);
                        cp.dispatch_workgroups(1, 1, 1);
                        cp.set_pipeline(if self.software_adapter {
                            self.software_scatter.as_ref().expect("software scatter compiled")
                        } else {
                            &self.sort_pipes[3]
                        });
                        cp.dispatch_workgroups(groups[0], groups[1], 1);
                    }
                }
                self.sorted_splats.insert(&sp.gpu, key, va.clone(), enc);
                va
            };
            // four passes end back in the "a" buffers
            let so = SplatObjU {
                model: sp.model.to_cols_array_2d(),
                model_inv: sp.model.inverse().to_cols_array_2d(),
                params: [sp.opacity, sp.gpu.sh_degree as f32, 0.0, 0.0],
            };
            let obuf = buf(bytemuck::bytes_of(&so), wgpu::BufferUsages::UNIFORM, "splat-object");
            let sb = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("splat"),
                layout: &self.bgl_splat,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: sp.gpu.buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: va.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: obuf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: sp.gpu.sh.as_entire_binding() },
                ],
            });
            splat_binds.push((sb, sp.gpu.n));
        }
        // ------------------------------------------------ transmissive, splats, blended
        let depth_t = pool(size, if precompute_coc { self.gbuffer } else { self.scalar }, 1, 1, 1, "three-depth");
        let depth_view = depth_t.create_view(&Default::default());
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("three-transparent"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_ms,
                    resolve_target: Some(&resolved),
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_ms,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: raster_stamp(&mut timer, "three-transparent"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(fb) = &fb_trans {
                rp.set_bind_group(0, fb, &[]);
                draw_list(&mut rp, &trans_runs, self);
            }
            rp.set_bind_group(0, &fb_plain, &[]);
            for (sb, n) in &splat_binds {
                rp.set_pipeline(&self.splat_pipe);
                rp.set_bind_group(1, sb, &[]);
                rp.draw(0..6, 0..*n);
            }
            draw_list(&mut rp, &blended_runs, self);
        }
        // ------------------------------------------------ depth resolve, post
        {
            let db = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("three-depth"),
                layout: &self.bgl_depth,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&depth_ms) },
                    wgpu::BindGroupEntry { binding: 1, resource: frame_buf.as_entire_binding() },
                ],
            });
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("three-depth-resolve"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &depth_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: raster_stamp(&mut timer, "three-depth-resolve"),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(if precompute_coc { &self.depth_coc_pipe } else { &self.depth_pipe });
            rp.set_bind_group(0, &db, &[]);
            rp.draw(0..3, 0..1);
        }
        // ------------------------------------------------ screen-space reflections
        let ssr_view = if let (true, Some(gn), Some(gz)) = (scene.ssr, &gb_n, &gb_z) {
            let v = pool(size, FORMAT, 1, 1, 1, "three-ssr").create_view(&Default::default());
            let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("three-ssr"),
                layout: &self.bgl_ssr,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: frame_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&resolved) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(gz) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(gn) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(env_view) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&self.clamp_smp) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&self.repeat_smp) },
                ],
            });
            post_pass(enc, &self.ssr_pipe, &bind, &v, raster_stamp(&mut timer, "three-ssr"));
            Some(v)
        } else {
            None
        };
        let resolved = ssr_view.as_ref().unwrap_or(&resolved);
        let pb = if scene.dof.is_some() {
            let tsize = [size[0].div_ceil(16), size[1].div_ceil(16)];
            let tile_format = if precompute_coc { self.gbuffer } else { self.scalar };
            let a = pool(tsize, tile_format, 1, 1, 1, "three-coc-tiles").create_view(&Default::default());
            let b = pool(tsize, tile_format, 1, 1, 1, "three-coc-dilated").create_view(&Default::default());
            post_pass(
                enc,
                &self.tile_max_pipe,
                &post_bind(resolved, &depth_view),
                &a,
                raster_stamp(&mut timer, "three-coc-tiles"),
            );
            post_pass(
                enc,
                &self.tile_dilate_pipe,
                &post_bind(resolved, &a),
                &b,
                raster_stamp(&mut timer, "three-coc-dilate"),
            );
            post_bind3(resolved, &depth_view, &b)
        } else {
            post_bind(resolved, &depth_view)
        };
        post_pass(enc, &self.dof_pipe, &pb, out, raster_stamp(&mut timer, "three-dof"));
        if let Some(timer) = timer {
            timer.resolve(enc);
            self.raster_timers.push(timer);
        }
        self.targets.lock().unwrap_or_else(|e| e.into_inner()).end_call();
        self.stats = stats;
        Ok(())
    }
}

impl ThreeEngine {
    /// A render target for [`ThreeEngine::render`].
    pub fn target(&self, size: [u32; 2]) -> wgpu::Texture {
        texture(&self.device, size, FORMAT, 1, 1, 1, "three-out")
    }

    /// Reads back an RGBA16F texture (tests and tools).
    pub fn read(&self, t: &wgpu::Texture) -> Vec<[f32; 4]> {
        let (w, h) = (t.width(), t.height());
        let row = (w * 8).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("three-read"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: t,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let data = buf.slice(..).get_mapped_range().expect("mapped");
        let mut out = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for px in data[(y * row) as usize..(y * row + w * 8) as usize].as_chunks::<8>().0 {
                let c = |k: usize| half::f16::from_le_bytes([px[k], px[k + 1]]).to_f32();
                out.push([c(0), c(2), c(4), c(6)]);
            }
        }
        out
    }

    /// Records and submits one render (tests and tools).
    pub fn render_now(&mut self, scene: &Scene3, backdrop: Option<&wgpu::TextureView>) -> Vec<[f32; 4]> {
        let out = self.target(scene.size);
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.render(&mut enc, scene, backdrop, &out.create_view(&Default::default())).expect("valid 3D test scene");
        self.queue.submit([enc.finish()]);
        self.read(&out)
    }

    /// Uploads an RGBA16F texture (tests and tools).
    pub fn upload_f16(&self, size: [u32; 2], px: &[[f32; 4]]) -> wgpu::Texture {
        let t = texture(&self.device, size, FORMAT, 1, 1, 1, "three-in");
        let data: Vec<u16> = px.iter().flat_map(|p| half4(*p)).collect();
        self.write_tex(&t, 0, size, bytemuck::cast_slice(&data), 8);
        t
    }
}

const TYPES: &str = include_str!("three_types.wgsl");
const BIND: &str = include_str!("three_bind.wgsl");

/// Main passes: meshes, shadows, dome background.
pub fn main_src() -> String {
    format!("{TYPES}\n{BIND}\n{}\n{}", include_str!("sampling.wgsl"), include_str!("three.wgsl"))
}

/// Splat drawing.
pub fn splat_src() -> String {
    format!("{TYPES}\n{BIND}\n{}", include_str!("three_splat.wgsl"))
}

/// Post passes: blits, backdrop composite, depth of field.
pub fn ssr_src() -> String {
    format!("{TYPES}\n{}", include_str!("three_ssr.wgsl"))
}

pub fn post_src() -> String {
    format!("{TYPES}\n{}\n{}", include_str!("three_coc.wgsl"), include_str!("three_post.wgsl"))
}

/// Multisampled depth resolve.
pub fn depth_src() -> String {
    format!("{TYPES}\n{}\n{}", include_str!("three_coc.wgsl"), include_str!("three_depth.wgsl"))
}

/// 360 reprojection.
pub fn sphere_src() -> String {
    include_str!("sphere.wgsl").to_string()
}

/// Splat radix sort.
pub fn sort_src() -> String {
    include_str!("three_sort.wgsl").to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_box_is_out_of_view_only_wholly_beyond_one_plane() {
        use super::*;
        let cam = sr_3d::camera::resolve(&sr_3d::camera::CameraParams::default(), 128.0, 128.0);
        let vp = cam.view_proj();
        let cube = |c: Vec3, r: f32| (c - Vec3::splat(r), c + Vec3::splat(r));
        assert!(box_in_view(vp, cube(Vec3::new(64.0, 64.0, 0.0), 10.0)), "at the frame's centre");
        assert!(!box_in_view(vp, cube(Vec3::new(64.0, 64.0, -1000.0), 10.0)), "behind the camera");
        assert!(!box_in_view(vp, cube(Vec3::new(5000.0, 64.0, 0.0), 10.0)), "far to the right");
        assert!(!box_in_view(vp, cube(Vec3::new(64.0, 64.0, 20000.0), 10.0)), "beyond the far plane");
        // straddling the right edge of the frame, and around the camera itself
        assert!(box_in_view(vp, cube(Vec3::new(130.0, 64.0, 0.0), 10.0)));
        assert!(box_in_view(vp, cube(cam.eye, 50.0)));
    }

    #[test]
    fn instance_object_admission_is_aligned_bounded_and_transactional() {
        use super::*;
        assert_eq!(std::mem::size_of::<ObjectU>(), 256, "WGSL storage array stride");
        let mut bytes = Vec::new();
        let object = ObjectU::zeroed();
        assert_eq!(push_object(&mut bytes, &object, 512).unwrap(), 0);
        assert_eq!(push_object(&mut bytes, &object, 512).unwrap(), 256);
        let before = bytes.clone();
        assert!(push_object(&mut bytes, &object, 512).is_err());
        assert_eq!(bytes, before, "reject before growing resident object storage");
    }

    #[test]
    fn alpha_discard_specialization_matches_general_shader() {
        use super::*;
        let gpu = match crate::gpu::test_gpu() {
            Ok(gpu) => gpu,
            Err(error) => {
                assert!(std::env::var("SR_REQUIRE_GPU").as_deref() != Ok("1"), "{error}");
                return;
            }
        };
        let mut engine =
            ThreeEngine::new_with(gpu.device.clone(), gpu.queue.clone(), gpu.gbuffer_depth, gpu.scalar_target);
        let sphere = sr_3d::prim::sphere(22.0, 24);
        let mesh = engine.upload_mesh(&sphere.vertices, &sphere.indices);
        let light = Light3 {
            kind: LightKind::Directional,
            pos: Vec3::new(20.0, 5.0, -80.0),
            dir: Vec3::new(0.3, 0.4, 1.0).normalize(),
            right: Vec3::X,
            color: Vec3::new(0.8, 0.6, 0.4),
            range: 0.0,
            falloff: 2.0,
            cos_outer: 0.0,
            cos_inner: 1.0,
            cast_shadow: false,
            softness: 0.0,
            bias: 0.001,
            map_size: 64,
            size: [0.0; 3],
            ies: None,
            affects_diffuse: true,
            affects_specular: true,
            contact: 0.0,
        };
        let mut ambient = light.clone();
        ambient.kind = LightKind::Ambient;
        ambient.color = Vec3::splat(0.25);
        let draw = Draw3 {
            mesh: MeshSrc::Cached(mesh),
            model: Mat4::from_translation(Vec3::new(48.0, 32.0, 0.0)),
            material: MaterialParams::default(),
            maps: Default::default(),
            opacity: 1.0,
            cast_shadow: false,
            receive_shadow: false,
            shadow_catcher: false,
        };
        let mut scene = Scene3 {
            cam: sr_3d::camera::resolve(&sr_3d::camera::CameraParams::default(), 96.0, 64.0),
            clip_fix: Mat4::IDENTITY,
            size: [96, 64],
            exposure: 0.83,
            dof: None,
            lens_k1: 0.0,
            draws: vec![draw],
            lights: vec![light, ambient],
            env: None,
            splats: Vec::new(),
            volumes: Vec::new(),
            encode_srgb: false,
            ao: None,
            ssr: false,
            path: None,
            geodesic: None,
        };
        let plane = sr_3d::prim::plane(100.0, 80.0, 1);
        scene.draws.push(Draw3 {
            mesh: MeshSrc::Cached(engine.upload_mesh(&plane.vertices, &plane.indices)),
            model: Mat4::from_translation(Vec3::new(48.0, 32.0, 28.0)),
            material: MaterialParams { base_color: [0.13, 0.63, 0.27, 1.0], double_sided: true, ..Default::default() },
            maps: Default::default(),
            opacity: 1.0,
            cast_shadow: false,
            receive_shadow: false,
            shadow_catcher: false,
        });
        let mask = engine.upload_rgba8(
            2,
            2,
            &[255, 255, 255, 0, 255, 128, 64, 255, 64, 128, 255, 255, 255, 255, 255, 0],
            false,
        );
        for mode in [AlphaMode::Opaque, AlphaMode::Mask, AlphaMode::Blend] {
            for alpha in [0.0, 0.49, 1.0] {
                for opacity in [0.4, 1.0] {
                    for mapped in [false, true] {
                        scene.draws[0].material = MaterialParams {
                            base_color: [0.73, 0.21, 1.5, alpha],
                            alpha_mode: mode,
                            roughness: 0.38,
                            metallic: 0.4,
                            double_sided: mapped,
                            ..Default::default()
                        };
                        scene.draws[0].maps[0] = mapped.then(|| mask.clone());
                        scene.draws[0].opacity = opacity;
                        engine.specialize_alpha_mask = true;
                        let actual = engine.render_now(&scene, None);
                        let stats = engine.stats;
                        engine.specialize_alpha_mask = false;
                        let expected = engine.render_now(&scene, None);
                        assert!(actual.iter().any(|p| p[3] > 0.01));
                        assert_eq!(
                            actual, expected,
                            "mode={mode:?}, alpha={alpha}, opacity={opacity}, mapped={mapped}"
                        );
                        assert_eq!(stats, engine.stats);
                    }
                }
            }
        }
        assert!(engine.pipes.keys().any(|key| key.alpha_mask));
        assert!(engine.pipes.keys().any(|key| !key.alpha_mask), "non-masked materials must omit discard");
    }

    #[test]
    fn plain_material_matches_general_shader() {
        use super::*;
        let gpu = match crate::gpu::test_gpu() {
            Ok(gpu) => gpu,
            Err(error) => {
                assert!(std::env::var("SR_REQUIRE_GPU").as_deref() != Ok("1"), "{error}");
                return;
            }
        };
        let mut engine =
            ThreeEngine::new_with(gpu.device.clone(), gpu.queue.clone(), gpu.gbuffer_depth, gpu.scalar_target);
        let mut rgba_reference = ThreeEngine::new_with(
            gpu.device.clone(),
            gpu.queue.clone(),
            wgpu::TextureFormat::Rgba32Float,
            gpu.scalar_target,
        );
        let sphere = sr_3d::prim::sphere(22.0, 24);
        let mesh = engine.upload_mesh(&sphere.vertices, &sphere.indices);
        let light = Light3 {
            kind: LightKind::Directional,
            pos: Vec3::new(20.0, 5.0, -80.0),
            dir: Vec3::new(0.3, 0.4, 1.0).normalize(),
            right: Vec3::X,
            color: Vec3::new(0.8, 0.6, 0.4),
            range: 0.0,
            falloff: 2.0,
            cos_outer: 0.0,
            cos_inner: 1.0,
            cast_shadow: false,
            softness: 0.0,
            bias: 0.001,
            map_size: 64,
            size: [0.0; 3],
            ies: None,
            affects_diffuse: true,
            affects_specular: true,
            contact: 0.0,
        };
        let mut ambient = light.clone();
        ambient.kind = LightKind::Ambient;
        ambient.color = Vec3::splat(0.25);
        let draw = Draw3 {
            mesh: MeshSrc::Cached(mesh),
            model: Mat4::from_translation(Vec3::new(48.0, 32.0, 0.0)),
            material: MaterialParams::default(),
            maps: Default::default(),
            opacity: 1.0,
            cast_shadow: false,
            receive_shadow: false,
            shadow_catcher: false,
        };
        let mut scene = Scene3 {
            cam: sr_3d::camera::resolve(&sr_3d::camera::CameraParams::default(), 96.0, 64.0),
            clip_fix: Mat4::IDENTITY,
            size: [96, 64],
            exposure: 0.83,
            dof: None,
            lens_k1: 0.0,
            draws: vec![draw],
            lights: vec![light, ambient],
            env: None,
            splats: Vec::new(),
            volumes: Vec::new(),
            encode_srgb: false,
            ao: None,
            ssr: false,
            path: None,
            geodesic: None,
        };
        for case in 0..24 {
            scene.draws[0].material = MaterialParams {
                base_color: [0.23, 0.71, 1.5, if case % 4 == 0 { 0.45 } else { 1.0 }],
                metallic: [0.0, 0.4, 1.0][case % 3],
                roughness: [0.03, 0.38, 0.95][case / 3 % 3],
                ior: if case % 2 == 0 { 1.5 } else { 2.3 },
                specular: if case % 3 == 0 { 0.0 } else { 0.75 },
                specular_color: [0.8, 0.3, 0.6],
                emissive: [0.05, 0.0, 0.01],
                unlit: case % 7 == 0,
                double_sided: case % 2 == 0,
                alpha_mode: if case % 4 == 0 { AlphaMode::Blend } else { AlphaMode::Opaque },
                ..Default::default()
            };
            scene.encode_srgb = case % 2 == 1;
            assert!(plain_material(&scene.draws[0]));
            engine.specialize_plain = true;
            engine.specialize_global_lights = true;
            let actual = engine.render_now(&scene, None);
            engine.specialize_global_lights = false;
            assert_eq!(actual, engine.render_now(&scene, None), "global lighting, material case {case}");
            engine.specialize_plain = false;
            let expected = engine.render_now(&scene, None);
            assert!(actual.iter().any(|p| p[3] > 0.01));
            assert_eq!(actual, expected, "material case {case}");
        }
        assert!(engine.pipes.keys().any(|key| key.plain));
        assert!(engine.pipes.keys().any(|key| !key.plain));
        // Exercise both G-buffer channels and the paired depth/CoC target,
        // including their interactions with transparent refraction and MSAA edges.
        let plane = sr_3d::prim::plane(100.0, 80.0, 1);
        scene.draws.push(Draw3 {
            mesh: MeshSrc::Cached(engine.upload_mesh(&plane.vertices, &plane.indices)),
            model: Mat4::from_translation(Vec3::new(48.0, 32.0, 28.0)),
            material: MaterialParams {
                base_color: [0.13, 0.63, 0.27, 1.0],
                double_sided: true,
                roughness: 0.2,
                metallic: 0.7,
                ..Default::default()
            },
            maps: Default::default(),
            opacity: 1.0,
            cast_shadow: false,
            receive_shadow: false,
            shadow_catcher: false,
        });
        engine.specialize_plain = true;
        for feature in 0..32 {
            scene.ao = (feature & 1 != 0).then_some([16.0, 1.0]);
            scene.ssr = feature & 2 != 0;
            scene.lights[0].contact = if feature & 4 != 0 { 32.0 } else { 0.0 };
            scene.dof = (feature & 8 != 0).then_some(Dof { coc_scale: 24000.0, focus: 100.0, max_coc: 9.1, blades: 7 });
            scene.draws[0].material = MaterialParams {
                base_color: [0.73, 0.21, 1.5, 1.0],
                roughness: 0.15,
                metallic: 0.4,
                transmission: if feature & 16 != 0 { 0.6 } else { 0.0 },
                ..Default::default()
            };
            let actual = engine.render_now(&scene, None);
            assert!(actual.iter().any(|p| p[0] > 0.01 && p[3] > 0.01));
            assert_eq!(actual, rgba_reference.render_now(&scene, None), "depth format: feature bits {feature}");
        }
        scene.draws[0].material = MaterialParams::default();
        assert!(plain_material(&scene.draws[0]));
        // Exact reference uses the original tiled list and local-light branches.
        // Include every supported small-light order, the tile cap, local-light
        // fallback and shadow paths. Both opaque and blended pipelines are checked.
        let template = scene.lights[0].clone();
        let scenarios = [(5usize, false, None), (63, false, None), (64, false, None), (3, true, None)]
            .into_iter()
            .chain((0..=4).flat_map(|count| (0..(1u32 << count)).map(move |kinds| (count, false, Some(kinds)))));
        for (count, mixed, kinds) in scenarios {
            scene.lights = (0..count)
                .map(|k| {
                    let mut light = template.clone();
                    light.kind = if mixed && k == 1 {
                        LightKind::Point
                    } else if kinds.map_or(k % 3 == 0, |mask| mask & (1 << k) == 0) {
                        LightKind::Ambient
                    } else {
                        LightKind::Directional
                    };
                    light.color = Vec3::new(0.03 * (k % 5 + 1) as f32, 0.07, 0.11);
                    light.pos = Vec3::new(30.0, 15.0, -50.0);
                    light.range = if mixed { 300.0 } else { 0.0 };
                    light.cast_shadow = k == 1;
                    light.contact = if k == 2 { 12.0 } else { 0.0 };
                    light
                })
                .collect();
            scene.draws[0].cast_shadow = true;
            scene.draws[0].receive_shadow = true;
            for transparent in [false, true] {
                scene.draws[0].material = MaterialParams {
                    base_color: [0.23, 0.71, 1.5, if transparent { 0.45 } else { 1.0 }],
                    roughness: 0.38,
                    metallic: 0.4,
                    alpha_mode: if transparent { AlphaMode::Blend } else { AlphaMode::Opaque },
                    ..Default::default()
                };
                engine.specialize_plain = true;
                engine.specialize_global_lights = true;
                let actual = engine.render_now(&scene, None);
                let actual_stats = engine.stats;
                engine.specialize_global_lights = false;
                let expected = engine.render_now(&scene, None);
                assert_eq!(
                    actual, expected,
                    "global count={count}, kinds={kinds:?}, mixed={mixed}, transparent={transparent}"
                );
                assert_eq!(actual_stats, engine.stats, "lighting statistics and overflow stay unchanged");
                assert_eq!(actual_stats.tile_overflow, count > MAX_PER_TILE);
            }
        }
        assert!(engine.pipes.keys().any(|key| key.global_lights));
        assert!(engine.pipes.keys().any(|key| !key.global_lights));
        let signatures: std::collections::HashSet<_> = engine
            .pipes
            .keys()
            .filter(|key| key.global_lights && key.light_signature != u32::MAX)
            .map(|key| key.light_signature)
            .collect();
        assert_eq!(signatures.len(), 31, "all light orders through four lights must activate specialization");
        scene.draws[0].material = MaterialParams::default();
        let m = &mut scene.draws[0].material;
        m.clearcoat = 0.5;
        assert!(!plain_material(&scene.draws[0]));
    }

    #[test]
    fn dof_matches_original_aperture_shader() {
        use super::*;
        let gpu = match crate::gpu::test_gpu() {
            Ok(gpu) => gpu,
            Err(error) => {
                assert!(std::env::var("SR_REQUIRE_GPU").as_deref() != Ok("1"), "{error}");
                return;
            }
        };
        let mut engine =
            ThreeEngine::new_with(gpu.device.clone(), gpu.queue.clone(), gpu.gbuffer_depth, gpu.scalar_target);
        let mut rgba_reference = ThreeEngine::new_with(
            gpu.device.clone(),
            gpu.queue.clone(),
            wgpu::TextureFormat::Rgba32Float,
            gpu.scalar_target,
        );
        // Compare the conversion with the original WGSL cast on the GPU, including
        // adjacent float values at powers of two, nonfinite values, and both axes
        // of the large-dimension fallback. Pixel coordinates select runtime inputs.
        let mut bits = vec![0u32, 0x80000000, 0x7f800000, 0xff800000, 0x7fc00000, 0xffc00000, 0x7f7fffff, 0xff7fffff];
        for exponent in -24..=31 {
            let power = 2.0f32.powi(exponent).to_bits();
            for value in [power - 1, power, power + 1] {
                bits.extend([value, value ^ 0x80000000]);
            }
        }
        let bit_literals = bits.iter().map(|b| format!("0x{b:08x}u")).collect::<Vec<_>>().join(",");
        let conversion_probe = format!(
            r#"
const CAST_BITS: array<u32, {count}> = array<u32, {count}>({bit_literals});
const CAST_DIMS: array<f32, 11> = array<f32, 11>(1.0, 2.0, 97.0, 8192.0, 16384.0, 8388607.0, 8388608.0, 8388609.0, 16777216.0, 2147483648.0, 4294967296.0);
@fragment
fn fs_conversion_probe(i: PostOut) -> @location(0) vec4<f32> {{
    let pixel = u32(i.pos.y) * 97u + u32(i.pos.x);
    let index = pixel % ({count}u * 11u);
    let value = bitcast<f32>(CAST_BITS[index % {count}u]);
    let dim = CAST_DIMS[index / {count}u];
    let dims = select(vec2(dim, 97.0), vec2(97.0, dim), pixel >= {count}u * 11u);
    let position = vec2(value, -value);
    let expected = vec2<i32>(clamp(position, vec2(0.0), dims - 1.0));
    let actual = clamped_texel(position, dims);
    return vec4(select(0.0, 1.0, all(actual == expected)));
}}
"#,
            count = bits.len(),
        );
        let shader = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("original-dof-oracle"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}\n{conversion_probe}",
                    post_src(),
                    include_str!("../tests/fixtures/three_dof_reference.wgsl")
                )
                .into(),
            ),
        });
        let layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&engine.bgl_post)],
            immediate_size: 0,
        });
        let dof_pipeline = |entry: &str| {
            let _creation = crate::gpu::creation_lock();
            gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("original-dof-oracle"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_post"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(FORMAT.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let mut reference = dof_pipeline("fs_dof_reference");
        let mut classification = dof_pipeline("fs_uniform_coc_classification");
        let mut conversion = dof_pipeline("fs_conversion_probe");
        let plane = sr_3d::prim::plane(25.0, 19.0, 1);
        let mesh = engine.upload_mesh(&plane.vertices, &plane.indices);
        let draws = [
            ([15.0, 18.0, -24.0], [0.9, 0.05, 0.2, 1.0]),
            ([49.0, 28.0, 40.0], [0.07, 0.83, 0.21, 1.0]),
            ([78.0, 42.0, 10.0], [0.04, 0.13, 1.7, 0.65]),
        ]
        .into_iter()
        .map(|(position, color)| Draw3 {
            mesh: MeshSrc::Cached(mesh.clone()),
            model: Mat4::from_translation(Vec3::from_array(position)),
            material: MaterialParams {
                base_color: color,
                unlit: true,
                double_sided: true,
                alpha_mode: if color[3] < 1.0 { AlphaMode::Blend } else { AlphaMode::Opaque },
                ..Default::default()
            },
            maps: Default::default(),
            opacity: 1.0,
            cast_shadow: false,
            receive_shadow: false,
            shadow_catcher: false,
        })
        .collect();
        let mut scene = Scene3 {
            cam: sr_3d::camera::resolve(&sr_3d::camera::CameraParams::default(), 97.0, 61.0),
            clip_fix: Mat4::IDENTITY,
            size: [97, 61],
            exposure: 0.73,
            dof: None,
            lens_k1: 0.0,
            draws,
            lights: Vec::new(),
            env: None,
            splats: Vec::new(),
            volumes: Vec::new(),
            encode_srgb: false,
            ao: None,
            ssr: false,
            path: None,
            geodesic: None,
        };
        scene.size = [97, 79];
        assert!(scene.size[0] as usize * scene.size[1] as usize >= bits.len() * 11 * 2);
        scene.dof = Some(Dof { coc_scale: 24000.0, focus: 100.0, max_coc: 9.1, blades: 7 });
        std::mem::swap(&mut engine.dof_pipe, &mut conversion);
        let mask = engine.render_now(&scene, None);
        std::mem::swap(&mut engine.dof_pipe, &mut conversion);
        for (pixel, color) in mask.iter().enumerate() {
            assert_eq!(*color, [1.0; 4], "clamped texture-coordinate conversion at probe pixel {pixel}");
        }
        scene.size = [97, 61];
        // Cross ring-count thresholds, circular and polygonal apertures, transparent
        // edges, HDR colour, lens distortion and encoded/linear working spaces.
        for blades in [0, 3, 7, 11] {
            for (case, max_coc) in [0.49, 0.51, 3.1, 6.1, 9.1, 64.0].into_iter().enumerate() {
                scene.dof = Some(Dof { coc_scale: 24000.0, focus: 100.0, max_coc, blades });
                scene.encode_srgb = case % 2 == 0;
                scene.lens_k1 = if case % 3 == 0 { 0.035 } else { 0.0 };
                let actual = engine.render_now(&scene, None);
                assert_eq!(
                    actual,
                    rgba_reference.render_now(&scene, None),
                    "depth format: blades={blades}, max_coc={max_coc}"
                );
                std::mem::swap(&mut engine.dof_pipe, &mut reference);
                let expected = engine.render_now(&scene, None);
                std::mem::swap(&mut engine.dof_pipe, &mut reference);
                assert!(actual.iter().any(|p| p[3] > 0.01), "the oracle must render visible geometry");
                assert_eq!(actual, expected, "blades={blades}, max_coc={max_coc}");
            }
        }

        // A full-screen constant-depth surface with varying HDR vertex colours
        // must activate the shortcut without making a wrong kernel look correct.
        let mut plane = sr_3d::prim::plane(400.0, 400.0, 64);
        for v in &mut plane.vertices {
            // A symmetric blur can preserve a linear ramp despite wrong taps.
            // Small alternating cells and asymmetric channel periods avoid that blind spot.
            let x = ((v.pos[0] + 200.0) / 6.25).round() as u32;
            let y = ((v.pos[1] + 200.0) / 6.25).round() as u32;
            v.color = [
                if (x + y).is_multiple_of(2) { 1.7 } else { 0.05 },
                if x.is_multiple_of(3) { 0.13 } else { 0.91 },
                if y.is_multiple_of(3) { 0.2 } else { 1.1 },
                1.0,
            ];
        }
        scene.draws = vec![Draw3 {
            mesh: MeshSrc::Cached(engine.upload_mesh(&plane.vertices, &plane.indices)),
            model: Mat4::from_translation(Vec3::new(48.5, 30.5, 0.0)),
            material: MaterialParams { unlit: true, double_sided: true, ..Default::default() },
            maps: Default::default(),
            opacity: 1.0,
            cast_shadow: false,
            receive_shadow: false,
            shadow_catcher: false,
        }];
        scene.dof = None;
        let pattern = engine.render_now(&scene, None);
        assert!(pattern.iter().any(|p| (p[0] - pattern[0][0]).abs() > 0.1), "nonlinear colour oracle is visible");
        let mut radii = vec![0.49, 0.51, 3.1, 6.1, 9.1, 64.0];
        // Neighbors of powers of two stress rounding in smoothstep's +/-1 edges.
        for radius in [1.0f32, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0] {
            radii.extend([f32::from_bits(radius.to_bits() - 1), radius, f32::from_bits(radius.to_bits() + 1)]);
        }
        for blades in [0, 3, 7, 11] {
            for (case, max_coc) in radii.iter().copied().enumerate() {
                scene.dof = Some(Dof { coc_scale: 48000.0, focus: 100.0, max_coc, blades });
                scene.encode_srgb = case % 2 == 0;
                scene.lens_k1 = if case % 3 == 0 { 0.035 } else { 0.0 };
                let actual = engine.render_now(&scene, None);
                assert_eq!(
                    actual,
                    rgba_reference.render_now(&scene, None),
                    "depth format: blades={blades}, max_coc={max_coc}"
                );
                std::mem::swap(&mut engine.dof_pipe, &mut reference);
                let expected = engine.render_now(&scene, None);
                std::mem::swap(&mut engine.dof_pipe, &mut reference);
                assert!(actual.iter().all(|p| p[3] > 0.99), "whole frame has geometry");
                assert_eq!(actual, expected, "uniform CoC: blades={blades}, max_coc={max_coc}");
            }
        }
        if gpu.scalar_target == wgpu::TextureFormat::R32Float
            && matches!(gpu.gbuffer_depth, wgpu::TextureFormat::Rg32Float | wgpu::TextureFormat::Rgba32Float)
        {
            std::mem::swap(&mut engine.dof_pipe, &mut classification);
            let mask = engine.render_now(&scene, None);
            std::mem::swap(&mut engine.dof_pipe, &mut classification);
            assert!(mask.iter().all(|p| p[0] > 0.99), "the uniform-CoC shortcut must be exercised");
            // A larger radius in the conservative border invalidates the proof,
            // even if it lies beyond the original maximum's tile neighborhood.
            scene.dof = Some(Dof { coc_scale: 24000.0, focus: 100.0, max_coc: 64.0, blades: 7 });
            scene.encode_srgb = false;
            scene.lens_k1 = 0.0;
            let edge = sr_3d::prim::plane(12.0, 200.0, 1);
            scene.draws.push(Draw3 {
                mesh: MeshSrc::Cached(engine.upload_mesh(&edge.vertices, &edge.indices)),
                model: Mat4::from_translation(Vec3::new(78.0, 30.5, -24.0)),
                material: MaterialParams {
                    base_color: [0.1, 0.9, 0.3, 1.0],
                    unlit: true,
                    double_sided: true,
                    ..Default::default()
                },
                maps: Default::default(),
                opacity: 1.0,
                cast_shadow: false,
                receive_shadow: false,
                shadow_catcher: false,
            });
            let actual = engine.render_now(&scene, None);
            std::mem::swap(&mut engine.dof_pipe, &mut reference);
            let expected = engine.render_now(&scene, None);
            std::mem::swap(&mut engine.dof_pipe, &mut reference);
            assert_eq!(actual, expected, "uniform-region boundary");
            assert_eq!(actual, rgba_reference.render_now(&scene, None), "depth format at uniform-region boundary");
            std::mem::swap(&mut engine.dof_pipe, &mut classification);
            let mask = engine.render_now(&scene, None);
            std::mem::swap(&mut engine.dof_pipe, &mut classification);
            assert_eq!(mask[30 * 97][0], 0.0, "expanded-neighborhood maximum participates in proof");
        }
    }

    #[test]
    fn raster_compiles_only_pipelines_used_by_the_scene() {
        use super::*;
        let gpu = match crate::gpu::test_gpu() {
            Ok(gpu) => gpu,
            Err(error) => {
                assert!(std::env::var("SR_REQUIRE_GPU").as_deref() != Ok("1"), "{error}");
                return;
            }
        };
        let mut engine = ThreeEngine::new(gpu.device.clone(), gpu.queue.clone());
        let mesh = sr_3d::prim::plane(16.0, 16.0, 1);
        let draw = Draw3 {
            mesh: MeshSrc::Cached(engine.upload_mesh(&mesh.vertices, &mesh.indices)),
            model: Mat4::from_translation(Vec3::new(16.0, 16.0, 0.0)),
            material: MaterialParams::default(),
            maps: Default::default(),
            opacity: 1.0,
            cast_shadow: false,
            receive_shadow: false,
            shadow_catcher: false,
        };
        let mut scene = Scene3 {
            cam: sr_3d::camera::resolve(&sr_3d::camera::CameraParams::default(), 32.0, 32.0),
            clip_fix: Mat4::IDENTITY,
            size: [32, 32],
            exposure: 1.0,
            dof: None,
            lens_k1: 0.0,
            draws: vec![draw],
            lights: Vec::new(),
            env: None,
            splats: Vec::new(),
            volumes: Vec::new(),
            encode_srgb: false,
            ao: None,
            ssr: false,
            path: None,
            geodesic: None,
        };
        assert_eq!(engine.render_now(&scene, None).len(), 32 * 32);
        assert_eq!(engine.pipes.len(), 1, "one material and blend/culling configuration");
        let MeshSrc::Cached(prototype) = &scene.draws[0].mesh else { unreachable!() };
        let prototype = prototype.clone();
        scene.draws = (0..128)
            .map(|i| Draw3 {
                mesh: MeshSrc::Cached(prototype.clone()),
                model: Mat4::from_translation(Vec3::new(
                    (i % 16) as f32 * 2.,
                    (i / 16) as f32 * 4.,
                    ((i * 37) % 128) as f32 / 128.,
                )),
                material: MaterialParams { unlit: true, ..Default::default() },
                maps: Default::default(),
                opacity: 1.,
                cast_shadow: false,
                receive_shadow: false,
                shadow_catcher: false,
            })
            .collect();
        for mode in 0..3 {
            for (i, draw) in scene.draws.iter_mut().enumerate() {
                draw.mesh = MeshSrc::Cached(prototype.clone());
                draw.opacity = if mode == 1 { 0.25 + (i % 3) as f32 * 0.25 } else { 1. };
                draw.material.base_color = if mode == 2 && i % 2 == 0 { [1., 0., 0., 1.] } else { [1.; 4] };
            }
            let batched = engine.render_now(&scene, None);
            assert_eq!(engine.stats.draws, if mode == 2 { 128 } else { 1 }, "compatible instance runs, mode {mode}");
            // Independent uploads force the ordinary single-instance path.
            for draw in &mut scene.draws {
                draw.mesh = MeshSrc::Cached(engine.upload_mesh(&mesh.vertices, &mesh.indices));
            }
            let individual = engine.render_now(&scene, None);
            assert_eq!(engine.stats.draws, 128);
            assert_eq!(batched, individual, "instance transforms/materials/opacity must preserve pixels, mode {mode}");
        }
        engine.pipes.clear();
        scene.draws.clear();
        engine.render_now(&scene, None);
        assert!(engine.pipes.is_empty(), "an empty scene needs no material pipeline");
    }

    #[test]
    fn shadow_bounds_cull_only_when_wholly_outside_one_clip_plane() {
        use super::*;
        let lo = Vec3::new(-0.25, -0.25, 0.25);
        let hi = Vec3::new(0.25, 0.25, 0.75);
        let outside = |shift| bounds_outside_clip(lo, hi, Mat4::from_translation(shift), Mat4::IDENTITY);
        assert!(!outside(Vec3::ZERO));
        for shift in [Vec3::X * 2.0, -Vec3::X * 2.0, Vec3::Y * 2.0, -Vec3::Y * 2.0, Vec3::Z, -Vec3::Z] {
            assert!(outside(shift), "wholly outside: {shift:?}");
        }
        for shift in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z * 0.5, -Vec3::Z * 0.5] {
            assert!(!outside(shift), "intersects: {shift:?}");
        }
        // Contact with a plane, including roundoff-sized excursions, stays visible.
        assert!(!outside(Vec3::X * (1.25 + f32::EPSILON)));
        assert!(!bounds_outside_clip(Vec3::splat(-2.0), Vec3::splat(2.0), Mat4::IDENTITY, Mat4::IDENTITY));
        let perspective = shadow_proj(70.0, 1.0, 100.0);
        assert!(!bounds_outside_clip(lo, hi, Mat4::from_translation(Vec3::Z), perspective));
        assert!(bounds_outside_clip(lo, hi, Mat4::from_translation(-Vec3::Z * 4.0), perspective));
        // Invalid or cancellation-heavy transforms are retained conservatively.
        assert!(!bounds_outside_clip(Vec3::splat(f32::NAN), hi, Mat4::IDENTITY, Mat4::IDENTITY));
        assert!(!bounds_outside_clip(lo, hi, Mat4::from_translation(Vec3::splat(f32::INFINITY)), perspective));
        let huge = Mat4::from_translation(Vec3::splat(1e20));
        assert!(!bounds_outside_clip(lo, hi, huge, huge.inverse()));
    }

    #[test]
    fn shaders_validate() {
        for (name, src) in [
            ("main", super::main_src()),
            ("splat", super::splat_src()),
            ("post", super::post_src()),
            ("depth", super::depth_src()),
            ("sort", super::sort_src()),
            ("sphere", super::sphere_src()),
        ] {
            let m = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(&src)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
                .validate(&m)
                .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(&src)));
        }
    }
}

#[cfg(test)]
mod splat_sort;
