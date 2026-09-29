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
const SHADOW_BUDGET: u64 = 256 << 20;
const IES_W: usize = 128;
const IES_ROWS: usize = 32;

/// A mesh on the GPU.
pub struct MeshGpu {
    pub vbuf: wgpu::Buffer,
    pub ibuf: wgpu::Buffer,
    pub count: u32,
    /// Object-space bounds.
    pub lo: Vec3,
    pub hi: Vec3,
}

/// A texture on the GPU (with mips).
pub struct TexGpu {
    pub view: wgpu::TextureView,
    pub key: u64,
}

/// A prefiltered environment on the GPU.
pub struct EnvGpu {
    pub view: wgpu::TextureView,
    pub mips: u32,
    pub sh: [[f32; 3]; 9],
}

/// Splats on the GPU (model space).
pub struct SplatGpu {
    pub buf: wgpu::Buffer,
    /// Spherical-harmonic coefficients (48 floats a splat), or one dummy entry.
    pub sh: wgpu::Buffer,
    pub sh_degree: u32,
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
    fn mesh(&self) -> &MeshGpu {
        match self {
            MeshSrc::Cached(m) | MeshSrc::Deformed(_, m) => m,
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
    pub instances: u32,
}

impl Draw3 {
    /// Triangles of one instance.
    pub fn mesh_triangles(&self) -> u64 {
        self.mesh.mesh().count as u64 / 3
    }
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
}

/// The dome environment.
pub struct Env3 {
    pub env: Arc<EnvGpu>,
    pub intensity: f32,
    /// Radians about y.
    pub rotation: f32,
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
    /// Encode the output with the sRGB curve (working spaces that blend on encoded values).
    pub encode_srgb: bool,
}

/// Counters for statistics and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats3 {
    pub draws: usize,
    pub triangles: u64,
    pub shadow_views: usize,
    pub splats: u64,
    pub transmissive: usize,
    pub tile_overflow: bool,
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
    sh: [[f32; 4]; 9],
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
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ObjectU {
    model: [[f32; 4]; 4],
    normal: [[f32; 4]; 4],
    params: [f32; 4],
    spacing: [f32; 4],
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
        uv: [m.uv_scale[0], m.uv_scale[1], 0.0, 0.0],
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PipeKey {
    cull: bool,
    blend: bool,
    depth_write: bool,
}

/// GPU state of the 3D renderer.
pub struct ThreeEngine {
    device: Arc<wgpu::Device>,
    targets: Arc<std::sync::Mutex<TargetPool>>,
    queue: Arc<wgpu::Queue>,
    main_mod: wgpu::ShaderModule,
    bgl_frame: wgpu::BindGroupLayout,
    bgl_mat: wgpu::BindGroupLayout,
    bgl_obj: wgpu::BindGroupLayout,
    bgl_splat: wgpu::BindGroupLayout,
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
    tile_max_pipe: wgpu::RenderPipeline,
    tile_dilate_pipe: wgpu::RenderPipeline,
    depth_pipe: wgpu::RenderPipeline,
    sort_pipes: [wgpu::ComputePipeline; 4],
    repeat_smp: wgpu::Sampler,
    clamp_smp: wgpu::Sampler,
    cmp_smp: wgpu::Sampler,
    white: Arc<TexGpu>,
    brdf: wgpu::TextureView,
    black_env: wgpu::TextureView,
    mat_binds: HashMap<[u64; 6], wgpu::BindGroup>,
    /// Meshes keyed by the caller.
    pub meshes: HashMap<String, Arc<MeshGpu>>,
    /// Textures keyed by the caller.
    pub textures: HashMap<String, Arc<TexGpu>>,
    /// Environments keyed by the caller.
    pub envs: HashMap<String, Arc<EnvGpu>>,
    /// Splats keyed by the caller.
    pub splat_cache: HashMap<String, Arc<SplatGpu>>,
    next_key: u64,
    ies_rows: HashMap<usize, usize>,
    /// Statistics of the last render.
    pub stats: Stats3,
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
    const ATTRS: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4];
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
/// pass (gate 8). A target is lent for one call and returned at its end; the encoder orders the
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
            ],
            "three-frame",
        );
        let mut mat_entries = vec![buf_entry(0, VS_FS, uni, true)];
        for b in 1..=6 {
            mat_entries.push(tex_entry(b, VS_FS, F, D2, false));
        }
        mat_entries.push(smp_entry(7, VS_FS, filt));
        let bgl_mat = bgl(&mat_entries, "three-material");
        let bgl_obj = bgl(&[buf_entry(0, VS_FS, uni, true)], "three-object");
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
            ],
            "three-post",
        );
        let bgl_depth =
            bgl(&[tex_entry(0, wgpu::ShaderStages::FRAGMENT, wgpu::TextureSampleType::Depth, D2, true)], "three-depth");
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
        let shadow_pipe = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
        });
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
        let dome_pipe = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
        });
        let splat_layout = layout(&[&bgl_frame, &bgl_splat]);
        let splat_pipe = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
        });
        let post_layout = layout(&[&bgl_post]);
        let post = |entry: &str, format: wgpu::TextureFormat| {
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
        let under_pipe = post("fs_under", FORMAT);
        let dof_pipe = post("fs_dof", FORMAT);
        let tile_max_pipe = post("fs_tile_max", wgpu::TextureFormat::R32Float);
        let tile_dilate_pipe = post("fs_tile_dilate", wgpu::TextureFormat::R32Float);
        let depth_layout = layout(&[&bgl_depth]);
        let depth_pipe = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                entry_point: Some("fs_depth_resolve"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::R32Float.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sort_layout = layout(&[&bgl_sort]);
        let sort_pipes = ["cs_keys", "cs_hist", "cs_scan", "cs_scatter"].map(|e| {
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
            targets: Default::default(),
            white: Arc::new(TexGpu {
                view: texture(d, [1, 1], wgpu::TextureFormat::Rgba8Unorm, 1, 1, 1, "white")
                    .create_view(&Default::default()),
                key: 0,
            }),
            brdf: texture(d, [1, 1], FORMAT, 1, 1, 1, "brdf").create_view(&Default::default()),
            black_env: texture(d, [1, 1], FORMAT, 1, 1, 1, "black-env").create_view(&Default::default()),
            device: device.clone(),
            queue,
            main_mod,
            bgl_frame,
            bgl_mat,
            bgl_obj,
            bgl_splat,
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
            tile_max_pipe,
            tile_dilate_pipe,
            depth_pipe,
            sort_pipes,
            repeat_smp,
            clamp_smp,
            cmp_smp,
            mat_binds: HashMap::new(),
            meshes: HashMap::new(),
            textures: HashMap::new(),
            envs: HashMap::new(),
            splat_cache: HashMap::new(),
            next_key: 1,
            ies_rows: HashMap::new(),
            stats: Stats3::default(),
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
                    depth_write_enabled: Some(key.depth_write),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: MSAA, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: m,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend: if key.blend { Some(PREMUL) } else { None },
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    /// Uploads an RGBA8 image with a CPU-built mip chain.
    pub fn upload_rgba8(&mut self, w: u32, h: u32, rgba: &[u8], srgb: bool) -> Arc<TexGpu> {
        let mips = 32 - w.max(h).max(1).leading_zeros();
        let format = if srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm };
        let t = texture(&self.device, [w, h], format, mips, 1, 1, "material-map");
        let mut img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).unwrap_or_else(|| image::RgbaImage::new(w, h));
        for level in 0..mips {
            let (lw, lh) = ((w >> level).max(1), (h >> level).max(1));
            if level > 0 {
                img = image::imageops::resize(&img, lw, lh, image::imageops::FilterType::Triangle);
            }
            self.write_tex(&t, level, [lw, lh], img.as_raw(), 4);
        }
        self.next_key += 1;
        Arc::new(TexGpu { view: t.create_view(&Default::default()), key: self.next_key })
    }

    /// Uploads a mesh.
    pub fn upload_mesh(&self, vertices: &[Vertex], indices: &[u32]) -> Arc<MeshGpu> {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for v in vertices {
            lo = lo.min(Vec3::from(v.pos));
            hi = hi.max(Vec3::from(v.pos));
        }
        if vertices.is_empty() {
            (lo, hi) = (Vec3::ZERO, Vec3::ZERO);
        }
        let vbuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh-v"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh-i"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Arc::new(MeshGpu { vbuf, ibuf, count: indices.len() as u32, lo, hi })
    }

    /// Prefilters and uploads an environment.
    pub fn upload_env(&self, eq: &sr_3d::env::Equirect) -> Arc<EnvGpu> {
        let base = eq.width.clamp(64, 512).next_power_of_two().min(512);
        let levels = (base.trailing_zeros() as usize).saturating_sub(2).clamp(2, 7);
        let pf = sr_3d::env::prefilter(eq, base, levels);
        let t = texture(&self.device, [pf.mips[0].0, pf.mips[0].1], FORMAT, levels as u32, 1, 1, "environment");
        for (k, (w, h, px)) in pf.mips.iter().enumerate() {
            let data: Vec<u16> = px.iter().flat_map(|p| half4(*p)).collect();
            self.write_tex(&t, k as u32, [*w, *h], bytemuck::cast_slice(&data), 8);
        }
        Arc::new(EnvGpu { view: t.create_view(&Default::default()), mips: levels as u32, sh: pf.sh })
    }

    /// Uploads splats: position and opacity, the covariance R S Sᵀ Rᵀ, colour.
    pub fn upload_splats(&self, s: &sr_3d::Splats) -> Arc<SplatGpu> {
        let mut data: Vec<[f32; 16]> = Vec::with_capacity(s.len());
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for i in 0..s.len() {
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
        let has_sh = s.sh_degree > 0 && s.sh.len() == s.len() && !s.is_empty();
        let dummy = [[0.0f32; 48]];
        let sh = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("splat-sh"),
            contents: bytemuck::cast_slice(if has_sh { &s.sh[..] } else { &dummy[..] }),
            usage: wgpu::BufferUsages::STORAGE,
        });
        Arc::new(SplatGpu {
            buf,
            sh,
            sh_degree: if has_sh { s.sh_degree.min(3) } else { 0 },
            n: s.len() as u32,
            lo,
            hi,
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
    pub fn render(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        scene: &Scene3,
        backdrop: Option<&wgpu::TextureView>,
        out: &wgpu::TextureView,
    ) {
        let d = self.device.clone();
        let size = [scene.size[0].max(1), scene.size[1].max(1)];
        let targets = self.targets.clone();
        let pool = |size: [u32; 2], format, mips, samples, layers, label| {
            targets.lock().unwrap_or_else(|e| e.into_inner()).take(&d, (size, format, mips, samples, layers, label))
        };
        for cull in [true, false] {
            for blend in [false, true] {
                for write in [false, true] {
                    self.pipe(PipeKey { cull, blend, depth_write: write });
                }
            }
        }

        let mut stats = Stats3::default();
        let vp = scene.clip_fix * scene.cam.view_proj();
        // ------------------------------------------------ lights, IES, shadow views
        let (ies_view, ies_rows) = self.ies_atlas(&scene.lights);
        let (mut blo, mut bhi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for dr in &scene.draws {
            let m = dr.mesh.mesh();
            for c in corners(m.lo, m.hi) {
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
        let mut shadow_mats: Vec<Mat4> = Vec::new();
        let mut map_size = 16u32;
        let mut lights_u: Vec<LightU> = Vec::with_capacity(scene.lights.len());
        for (li, l) in scene.lights.iter().enumerate() {
            let mut first = -1.0;
            let mut views = 1.0;
            if l.cast_shadow && l.kind != LightKind::Ambient {
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
                        let pad = extent * 0.02;
                        let (lo, hi) = (lo - Vec3::splat(pad), hi + Vec3::splat(pad));
                        let ortho = Mat4::from_cols_array(&[
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
                        ]);
                        shadow_mats.push(ortho * v);
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
                size: [l.size[0], l.size[1], l.size[2], ies_rows[li]],
                flags: [l.affects_diffuse as u32 as f32, l.affects_specular as u32 as f32, l.bias, views],
                right: [l.right.x, l.right.y, l.right.z, 0.0],
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
        // ------------------------------------------------ tiles
        let tiles = [size[0].div_ceil(TILE), size[1].div_ceil(TILE)];
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
        // ------------------------------------------------ frame uniform
        let env = scene.env.as_ref();
        let mut sh = [[0.0f32; 4]; 9];
        if let Some(e) = env {
            for (dst, src) in sh.iter_mut().zip(&e.env.sh) {
                *dst = [src[0], src[1], src[2], 0.0];
            }
        }
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
                env.map(|e| e.rotation).unwrap_or(0.0),
                env.map(|e| e.visible as u32 as f32).unwrap_or(0.0),
                env.is_some() as u32 as f32,
                env.map(|e| e.env.mips as f32).unwrap_or(1.0),
            ],
            dof: [dof.coc_scale, dof.focus.max(1e-3), dof.max_coc.min(64.0), dof.blades as f32],
            post: [scene.lens_k1, scene.dof.is_some() as u32 as f32, scene.cam.near, scene.cam.far],
            lens: [
                scene.clip_fix.x_axis.x * scene.cam.proj.x_axis.x * size[0] as f32 * 0.5,
                scene.encode_srgb as u32 as f32,
                0.0,
                0.0,
            ],
            sh,
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
        struct Prep {
            kind: Kind,
            obj: u32,
            mat: u32,
            key: [u64; 6],
            vbuf: Option<wgpu::Buffer>,
            depth: f32,
            cull: bool,
        }
        let mut preps: Vec<Prep> = Vec::new();
        for dr in &scene.draws {
            let m = dr.mesh.mesh();
            let size3 = (m.hi - m.lo).max(Vec3::splat(1e-3));
            let n = dr.instances.max(1);
            let cols = (n as f32).sqrt().ceil().max(1.0);
            let o = ObjectU {
                model: dr.model.to_cols_array_2d(),
                normal: dr.model.inverse().transpose().to_cols_array_2d(),
                params: [dr.opacity, dr.receive_shadow as u32 as f32, n as f32, cols],
                spacing: [size3.x * 1.25, size3.y * 1.25, size3.z * 1.25, 0.0],
            };
            let obj = pad(&mut obj_bytes, bytemuck::bytes_of(&o));
            let mat = pad(&mut mat_bytes, bytemuck::bytes_of(&material_u(&dr.material, &dr.maps)));
            let kind = if dr.material.transmission > 0.0 && !dr.material.unlit {
                Kind::Transmissive
            } else if dr.material.alpha_mode == AlphaMode::Blend || dr.opacity < 1.0 || dr.material.opacity < 1.0 {
                Kind::Blend
            } else {
                Kind::Opaque
            };
            let center = dr.model.transform_point3((m.lo + m.hi) * 0.5);
            let vbuf = match &dr.mesh {
                MeshSrc::Deformed(vs, _) => Some(buf(bytemuck::cast_slice(vs), wgpu::BufferUsages::VERTEX, "deformed")),
                MeshSrc::Cached(_) => None,
            };
            stats.triangles += m.count as u64 / 3 * n as u64;
            preps.push(Prep {
                kind,
                obj,
                mat,
                key: [0; 6],
                vbuf,
                depth: scene.cam.depth_of(center),
                cull: !dr.material.double_sided,
            });
        }
        stats.draws = preps.len();
        stats.transmissive = preps.iter().filter(|p| p.kind == Kind::Transmissive).count();
        // shadow-pass object uniforms: one per (caster, view)
        let mut shadow_objs: Vec<(usize, u32, u32)> = Vec::new();
        for v in 0..shadow_mats.len() {
            for (i, dr) in scene.draws.iter().enumerate() {
                if dr.cast_shadow && dr.opacity > 0.0 {
                    let mut o: ObjectU = *bytemuck::from_bytes(
                        &obj_bytes[preps[i].obj as usize..preps[i].obj as usize + std::mem::size_of::<ObjectU>()],
                    );
                    o.spacing[3] = v as f32;
                    shadow_objs.push((i, v as u32, pad(&mut obj_bytes, bytemuck::bytes_of(&o))));
                }
            }
        }
        if obj_bytes.is_empty() {
            pad(&mut obj_bytes, &[0u8; 16]);
        }
        if mat_bytes.is_empty() {
            // the dome still binds a material slot when nothing else is drawn
            pad(&mut mat_bytes, &[0u8; std::mem::size_of::<MaterialU>()]);
        }
        let obj_buf = buf(&obj_bytes, wgpu::BufferUsages::UNIFORM, "three-objects");
        // one material buffer per render, like the object buffer: several 3D passes can be recorded before a
        // single submit, and rewriting a shared buffer with queue.write_buffer would hand every pass the data of
        // the last one. Bind groups reference this buffer, so the cache lives for this render only.
        let mat_buf = buf(&mat_bytes, wgpu::BufferUsages::UNIFORM, "three-materials");
        self.mat_binds.clear();
        for (i, dr) in scene.draws.iter().enumerate() {
            preps[i].key = self.mat_bind(&dr.maps, &mat_buf);
        }
        if preps.is_empty() {
            let none: Maps = Default::default();
            self.mat_bind(&none, &mat_buf);
        }
        let obj_bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("three-object"),
            layout: &self.bgl_obj,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &obj_buf,
                    offset: 0,
                    size: std::num::NonZeroU64::new(std::mem::size_of::<ObjectU>() as u64),
                }),
            }],
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
        let frame_bind_with =
            |scene_color: &wgpu::TextureView, env_view: &wgpu::TextureView, shadow_view: &wgpu::TextureView| {
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
                    ],
                })
            };
        let frame_bind = |scene_color: &wgpu::TextureView, env_view: &wgpu::TextureView| {
            frame_bind_with(scene_color, env_view, &shadow_view)
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
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.shadow_pipe);
            rp.set_bind_group(0, &fb_shadow, &[]);
            for &(i, vv, off) in &shadow_objs {
                if vv as usize != v {
                    continue;
                }
                let dr = &scene.draws[i];
                let m = dr.mesh.mesh();
                rp.set_bind_group(1, &self.mat_binds[&preps[i].key], &[preps[i].mat]);
                rp.set_bind_group(2, &obj_bind, &[off]);
                rp.set_vertex_buffer(0, preps[i].vbuf.as_ref().unwrap_or(&m.vbuf).slice(..));
                rp.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                rp.draw_indexed(0..m.count, 0, 0..dr.instances.max(1));
            }
        }
        // ------------------------------------------------ opaque pass
        let mut order: Vec<usize> = (0..preps.len()).collect();
        order.sort_by(|a, b| preps[*a].depth.total_cmp(&preps[*b].depth));
        let draw_list = |rp: &mut wgpu::RenderPass, list: &[usize], eng: &ThreeEngine, blend: bool| {
            for &i in list {
                let dr = &scene.draws[i];
                let m = dr.mesh.mesh();
                rp.set_pipeline(&eng.pipes[&PipeKey { cull: preps[i].cull, blend, depth_write: !blend }]);
                rp.set_bind_group(1, &eng.mat_binds[&preps[i].key], &[preps[i].mat]);
                rp.set_bind_group(2, &obj_bind, &[preps[i].obj]);
                rp.set_vertex_buffer(0, preps[i].vbuf.as_ref().unwrap_or(&m.vbuf).slice(..));
                rp.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                rp.draw_indexed(0..m.count, 0, 0..dr.instances.max(1));
            }
        };
        let opaque: Vec<usize> = order.iter().copied().filter(|i| preps[*i].kind == Kind::Opaque).collect();
        let mut trans: Vec<usize> = order.iter().copied().filter(|i| preps[*i].kind == Kind::Transmissive).collect();
        let mut blended: Vec<usize> = order.iter().copied().filter(|i| preps[*i].kind == Kind::Blend).collect();
        trans.reverse();
        blended.reverse();
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
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_bind_group(0, &fb_plain, &[]);
            if env.map(|e| e.visible).unwrap_or(false) {
                rp.set_pipeline(&self.dome_pipe);
                rp.set_bind_group(1, &self.mat_binds[&preps.first().map(|p| p.key).unwrap_or([0; 6])], &[0]);
                rp.set_bind_group(2, &obj_bind, &[0]);
                rp.draw(0..3, 0..1);
            }
            draw_list(&mut rp, &opaque, self, false);
        }
        let no_tiles =
            pool([1, 1], wgpu::TextureFormat::R32Float, 1, 1, 1, "three-no-tiles").create_view(&Default::default());
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
                ],
            })
        };
        let post_bind = |src: &wgpu::TextureView, aux: &wgpu::TextureView| post_bind3(src, aux, &no_tiles);
        let post_pass = |enc: &mut wgpu::CommandEncoder,
                         pipe: &wgpu::RenderPipeline,
                         bind: &wgpu::BindGroup,
                         dst: &wgpu::TextureView| {
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
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(pipe);
            rp.set_bind_group(0, bind, &[]);
            rp.draw(0..3, 0..1);
        };
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
            post_pass(enc, &self.under_pipe, &under, &lvl(0));
            for k in 1..mips {
                let b = post_bind(&lvl(k - 1), &black);
                post_pass(enc, &self.blit_pipe, &b, &lvl(k));
            }
            fb_trans = Some(frame_bind(&sc.create_view(&Default::default()), env_view));
        }
        // ------------------------------------------------ splat sort
        let mut splat_binds = Vec::new();
        for sp in &scene.splats {
            let n = sp.gpu.n.max(1);
            stats.splats += sp.gpu.n as u64;
            let blocks = n.div_ceil(256);
            let mut ubytes = Vec::new();
            for pass in 0..4u32 {
                let u = SortU {
                    n: sp.gpu.n,
                    blocks,
                    shift: pass * 8,
                    pad: 0,
                    view: (scene.cam.view * sp.model).to_cols_array_2d(),
                };
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
                    cp.set_pipeline(&self.sort_pipes[3]);
                    cp.dispatch_workgroups(groups[0], groups[1], 1);
                }
            }
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
        let depth_t = pool(size, wgpu::TextureFormat::R32Float, 1, 1, 1, "three-depth");
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
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(fb) = &fb_trans {
                rp.set_bind_group(0, fb, &[]);
                draw_list(&mut rp, &trans, self, false);
            }
            rp.set_bind_group(0, &fb_plain, &[]);
            for (sb, n) in &splat_binds {
                rp.set_pipeline(&self.splat_pipe);
                rp.set_bind_group(1, sb, &[]);
                rp.draw(0..6, 0..*n);
            }
            draw_list(&mut rp, &blended, self, true);
        }
        // ------------------------------------------------ depth resolve, post
        {
            let db = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("three-depth"),
                layout: &self.bgl_depth,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&depth_ms),
                }],
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
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.depth_pipe);
            rp.set_bind_group(0, &db, &[]);
            rp.draw(0..3, 0..1);
        }
        let pb = if scene.dof.is_some() {
            let tsize = [size[0].div_ceil(16), size[1].div_ceil(16)];
            let a =
                pool(tsize, wgpu::TextureFormat::R32Float, 1, 1, 1, "three-coc-tiles").create_view(&Default::default());
            let b = pool(tsize, wgpu::TextureFormat::R32Float, 1, 1, 1, "three-coc-dilated")
                .create_view(&Default::default());
            post_pass(enc, &self.tile_max_pipe, &post_bind(&resolved, &depth_view), &a);
            post_pass(enc, &self.tile_dilate_pipe, &post_bind(&resolved, &a), &b);
            post_bind3(&resolved, &depth_view, &b)
        } else {
            post_bind(&resolved, &depth_view)
        };
        post_pass(enc, &self.dof_pipe, &pb, out);
        self.targets.lock().unwrap_or_else(|e| e.into_inner()).end_call();
        self.stats = stats;
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
            for px in data[(y * row) as usize..(y * row + w * 8) as usize].chunks_exact(8) {
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
        self.render(&mut enc, scene, backdrop, &out.create_view(&Default::default()));
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
    format!("{TYPES}\n{BIND}\n{}", include_str!("three.wgsl"))
}

/// Splat drawing.
pub fn splat_src() -> String {
    format!("{TYPES}\n{BIND}\n{}", include_str!("three_splat.wgsl"))
}

/// Post passes: blits, backdrop composite, depth of field.
pub fn post_src() -> String {
    format!("{TYPES}\n{}", include_str!("three_post.wgsl"))
}

/// Multisampled depth resolve.
pub fn depth_src() -> String {
    include_str!("three_depth.wgsl").to_string()
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
