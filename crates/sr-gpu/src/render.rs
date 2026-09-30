//! The compositor: FrameGraph → linear-light RGBA16F frame.
//!
//! Each frame is planned on the CPU into jobs (one per render target) and
//! executed in one command buffer:
//!
//! * **Isolation.** Instances, includes, and groups that are isolated,
//!   clipped, masked, matted, blended with a non-normal mode or carry
//!   effects render their children into an offscreen texture, which then
//!   composites as one layer. Offscreens of sized nodes live in the node's
//!   local space, so moving the node reuses the texture.
//! * **Caching.** Every offscreen and matte is keyed by a hash of its
//!   subtree's state; unchanged subtrees reuse last frame's texture. At the
//!   root, the longest run of leading draws that did not change since the
//!   previous frame is restored from a snapshot instead of redrawn.
//! * **Blending.** Normal and dissolve use fixed-function premultiplied
//!   over; the other 33 modes copy the backdrop under the layer's bounds and
//!   composite in the shader.
//! * **2.5D.** `threeD` nodes project through a perspective camera centred
//!   on the frame and sort back to front among consecutive 3D siblings.
//! * **Vector content.** Shapes, vector, SVG and Lottie assets become
//!   `sr_vector` scenes. Runs of simple shapes (normal blend, no mask,
//!   matte or 3D) rasterise together straight into the target's pixel
//!   space; other vector nodes rasterise in local space at their on-screen
//!   scale and composite like image layers. Deformed raster layers draw as
//!   a deformed grid mesh.

use std::collections::HashMap;
use std::sync::Arc;

use sr_eval::{Affine, FrameGraph, FrameNode, Program, Value};

use crate::fx;

#[path = "render_access.rs"]
mod render_access;
#[path = "render_fx.rs"]
mod render_fx;
#[path = "render_map3d.rs"]
mod render_map3d;
#[path = "render_three.rs"]
mod render_three;
pub use render_three::ViewOverride;
use sr_model::element::{AttrValue, Element};
use sr_model::model::{self as m, AssetsChild};

use crate::color::{self, Working};
use crate::gpu::Gpu;
use crate::paint::{token_table, PaintTable};
use crate::raster::{Raster, RasterJob};
use crate::resources::{self, Pool, Tex};
use crate::types::{self, flag, src, Draw, Gen, Globals, Mask, Vertex};
use crate::vector::Attrs;
use sr_vector::{Scene, Xf};

/// Counters for one rendered frame.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RenderStats {
    /// Draw calls.
    pub draws: usize,
    /// Render targets rendered (frame, offscreens, mattes, generators).
    pub targets: usize,
    /// Offscreens and mattes reused from the previous frame.
    pub cache_hits: usize,
    /// Leading root draws restored from the prefix snapshot.
    pub prefix_restored: usize,
    /// Backdrop copies for non-normal blend modes.
    pub backdrop_copies: usize,
    /// Nodes whose content belongs to later batches (asset or node kind).
    pub unsupported: Vec<String>,
    /// Media that could not be read (missing files, decode failures,
    /// image-sequence frames missing under `missingFrame="error"`).
    pub errors: Vec<String>,
    /// Video frames decoded and converted this frame.
    pub video_frames: usize,
    /// Seconds spent waiting for decoded video frames.
    pub decode_wait: f64,
    /// CPU seconds spent building vector geometry and encoding it into tiles.
    pub vector_seconds: f64,
    /// Effect, transition and finishing passes recorded.
    pub fx_passes: usize,
    /// Pixels of the effect targets whose chains ran this frame (a measure of effect work).
    pub effect_pixels: u64,
    /// Offscreen textures allocated this frame for effects, motion blur and time effects.
    pub textures_created: usize,
    /// Offscreen and cached textures destroyed this frame.
    pub textures_released: usize,
    /// CPU seconds evaluating the scene at other times (sub-frames for motion blur and time effects).
    pub subframe_seconds: f64,
    /// Sub-frames rendered for motion blur and time effects.
    pub subframes: usize,
    /// 3D mesh draws.
    pub objects3d: usize,
    /// 3D triangles drawn (all instances).
    pub triangles: u64,
    /// Gaussian splats drawn.
    pub splats: u64,
    /// WCAG contrast ratio of burned-in text against the backdrop behind it (node id or "captions").
    pub contrast: Vec<(String, f64)>,
    /// Text layers drawn inside an isolated group (offscreen), where the inline probe cannot
    /// see the final backdrop; delivery measures them by rendering the frame with and without
    /// the layer (`Renderer::contrast_with_without`).
    pub contrast_unprobed: Vec<String>,
}

struct Cmd {
    draw: u32,
    first_vertex: u32,
    /// Vertices drawn (6 for a quad, more for a deformed mesh).
    count: u32,
    src: Arc<Tex>,
    backdrop: Option<[u32; 4]>,
    matte: Option<Arc<Tex>>,
    hash: u64,
    /// Adjustment layer: snapshot the target and run passes before drawing.
    pre: Option<Box<AdjPre>>,
}

struct AdjPre {
    snapshot: Arc<Tex>,
    passes: Vec<fx::Pass>,
    /// A 3D pass rendered into its texture (the snapshot is its backdrop).
    three: Option<Box<(crate::three::Scene3, Arc<Tex>)>>,
}

struct Job {
    target: Arc<Tex>,
    clear: bool,
    cmds: Vec<Cmd>,
    root: bool,
    /// Effect passes recorded before the draws.
    fx: Vec<fx::Pass>,
    /// Optical flow between two finished textures, computed before `fx` (pixel motion blur).
    flow: Option<(Arc<Tex>, Arc<Tex>, fx::FlowSlot)>,
    /// Whether the job draws into `target` (effect-only jobs do not).
    draw: bool,
    /// Particles drawn into `target` before anything else.
    parts: Option<Box<crate::particles::ParticleJob>>,
}

impl Job {
    fn draws(target: Arc<Tex>, clear: bool, cmds: Vec<Cmd>, root: bool) -> Job {
        Job { target, clear, cmds, root, fx: Vec::new(), flow: None, draw: true, parts: None }
    }
}

struct GenJob {
    target: Arc<Tex>,
    bind: wgpu::BindGroup,
}

/// The `behind` blend code; a draw with it and no backdrop copy uses the fixed-function
/// destination-over pipeline (the project background).
const BLEND_UNDER: u32 = 34;

/// Blend modes the fixed-function blender computes exactly, so the shader needs no copy of the
/// backdrop: add and linear-dodge are dst + src on premultiplied colour, with normal alpha.
/// (Screen, multiply and plus-lighter clamp or multiply by the backdrop, which it cannot.)
pub(crate) fn fixed_function_blend(blend: u32) -> bool {
    matches!(blend, 2 | 17)
}
/// Adjustment layers: backdrop + (effect − backdrop) · coverage.
pub(crate) const BLEND_ADJUST: u32 = 35;

#[derive(Clone, Copy)]
struct Space {
    /// World → target pixels.
    xform: Affine,
    size: [u32; 2],
}

#[derive(Default)]
struct Plan {
    draws: Vec<Draw>,
    verts: Vec<Vertex>,
    masks: Vec<Mask>,
    edges: Vec<[f32; 4]>,
    paints: PaintTable,
    jobs: Vec<Job>,
    gens: Vec<GenJob>,
    rasters: Vec<RasterJob>,
    /// Pending runs of simple vector nodes, per command list.
    vbatches: Vec<VBatch>,
    stats: RenderStats,
    /// Pooled textures used by effect passes this frame.
    fx_temps: Vec<Arc<Tex>>,
    /// Colour finishing on the frame, and its result (copied back into the frame).
    post: Vec<fx::Pass>,
    post_out: Option<Arc<Tex>>,
    /// Contrast probes: node id, target rectangle, backdrop snapshot.
    probes: Vec<(String, [u32; 4], Arc<Tex>)>,
}

/// Simple vector nodes waiting to rasterise together into their target's space.
struct VBatch {
    /// Address of the command list they belong to.
    key: usize,
    space: Space,
    scene: Scene,
    hash: u64,
    first: String,
}

struct Prefix {
    hashes: Vec<u64>,
    tex: Arc<Tex>,
}

/// A rendered frame, resident on the GPU.
#[derive(Clone)]
pub struct Frame {
    /// The RGBA16F frame texture.
    pub texture: Arc<Tex>,
    /// Statistics.
    pub stats: RenderStats,
}

/// The compositor for one program.
pub struct Renderer {
    gpu: Gpu,
    over: wgpu::RenderPipeline,
    /// Premultiplied destination-over: the project background, drawn last beneath everything.
    under: wgpu::RenderPipeline,
    blend: wgpu::RenderPipeline,
    /// Add and linear-dodge through the blender (see `fixed_function_blend`).
    additive: wgpu::RenderPipeline,
    generator: wgpu::RenderPipeline,
    bgl0: wgpu::BindGroupLayout,
    bgl1: wgpu::BindGroupLayout,
    bgl2: wgpu::BindGroupLayout,
    bgl3: wgpu::BindGroupLayout,
    samp: wgpu::Sampler,
    samp_repeat: wgpu::Sampler,
    globals: wgpu::Buffer,
    dummy: Arc<Tex>,
    working: Working,
    tokens: HashMap<String, [f64; 4]>,
    images: HashMap<String, Option<Arc<Tex>>>,
    /// Image files whose embedded colour profile could not be honoured, and why.
    image_notes: HashMap<std::path::PathBuf, String>,
    /// The last uploaded picture of each grid simulation: (content key, texture).
    sim_textures: HashMap<Arc<str>, (u64, Arc<Tex>)>,
    generators: HashMap<u64, Arc<Tex>>,
    subtree: HashMap<String, (u64, Arc<Tex>)>,
    used: std::collections::HashSet<String>,
    /// The frame each cache key was last used in (frames counted by `cache_frame`).
    last_used: HashMap<String, u64>,
    /// Frames rendered, for cache ages.
    cache_frame: u64,
    prefix: Option<Prefix>,
    prev_root: Vec<u64>,
    pool: Pool,
    frame: Option<Arc<Tex>>,
    video: crate::video::VideoEngine,
    decoders: HashMap<String, Result<sr_media::VideoDecoder, String>>,
    video_frames: HashMap<String, Arc<Tex>>,
    raster: Raster,
    lotties: crate::vector::LottieCache,
    svgs: HashMap<(String, i32), Result<Arc<sr_vector::svg::Svg>, String>>,
    fx: fx::FxEngine,
    /// Nodes being drawn without their own opacity, blend, masks and matte (effect sources).
    bare: std::collections::HashSet<Arc<str>>,
    /// Nodes being drawn inside a transition.
    in_transition: std::collections::HashSet<Arc<str>>,
    /// Drawing a motion-blur sub-frame.
    sampling: bool,
    /// Pixel rectangle of the next frame-space quad (defaults to the whole target).
    frame_rect: Option<[f64; 4]>,
    /// Reuse effect results across frames (off with `SR_FX_NO_CACHE`, for measuring effect cost).
    fx_cache: bool,
    three: Option<Box<crate::three::ThreeEngine>>,
    three_assets: HashMap<String, Arc<Result<sr_3d::Asset, String>>>,
    mtlx: HashMap<String, Arc<Result<sr_3d::mtlx::MtlxMaterial, String>>>,
    ies: HashMap<String, Result<Arc<Vec<f32>>, String>>,
    /// Camera replacement for 360 faces and stereo eyes.
    pub view_override: Option<render_three::ViewOverride>,
    /// Quality tier overriding the document's `project@quality`.
    pub quality: Option<m::ProjectQuality>,
    /// Measure GPU time with timestamp queries (where the device has them); see `gpu_times`.
    pub time_gpu: bool,
    /// The last frame's timestamps.
    last_timer: Option<fx::Timer>,
    /// The tier of the frame being rendered.
    tier: Tier,
    sphere: Option<render_three::SpherePipe>,
    particles: Option<Box<crate::particles::ParticleEngine>>,
    /// Measure the contrast of burned-in text (`accessibility@contrastCheck`).
    pub contrast_probe: bool,
    grid: Option<(wgpu::RenderPipeline, wgpu::BindGroupLayout)>,
    /// Blurred glyph groups of the text layer being drawn (radius in node pixels, node-local scene).
    pending_blur: Vec<(f64, Scene)>,
    text: crate::text::TextCache,
    /// The current mesh key of each clay object (its old meshes are dropped when it changes).
    clay_keys: HashMap<Arc<str>, String>,
    glyph_tex: HashMap<u64, Option<Arc<Tex>>>,
    /// Burn only this caption track (an output's `burnCaptions`); otherwise tracks with mode burn or both.
    pub burn_captions: Option<String>,
    /// Burn no captions at all: an output with segments burns its own, in output time.
    pub captions_off: bool,
    /// Largest texture dimension used for images.
    pub max_texture: u32,
    /// Preferred asset representation (`proxy`, …); the asset's own `src` otherwise.
    pub representation: Option<String>,
    /// The audio mix for shader audio inputs (set by the delivery pipeline).
    pub audio: Option<Arc<crate::shader::AudioSignals>>,
    /// Whether the scene's shaders keep persistent ISF buffers (decided on the first render).
    persistent_isf: Option<bool>,
    /// Tiled vector paths reused across frames.
    tiles: sr_vector::tile::TileCache,
    /// The frame rendered last (ISF feedback replays on a seek).
    last_frame: Option<i64>,
    /// The project seed (seeded 64-bit hash draws default to it).
    pub(crate) seed: u64,
    /// The per-frame storage buffers, grown as needed and reused across frames with their bind group.
    frame_bufs: Option<FrameBufs>,
    /// The queue submission of the last frame (a pipelined caller waits on it, not on the whole queue).
    last_submit: Option<wgpu::SubmissionIndex>,
}

/// A grow-only GPU buffer reused across frames.
struct GrowBuf {
    buf: wgpu::Buffer,
    cap: u64,
}

/// The per-frame storage buffers and the bind group over them.
struct FrameBufs {
    draws: GrowBuf,
    masks: GrowBuf,
    edges: GrowBuf,
    paints: GrowBuf,
    stops: GrowBuf,
    verts: GrowBuf,
    bg0: wgpu::BindGroup,
}

/// What a quality tier (`project@quality`) trades for speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tier {
    /// Internal render scale: targets are this fraction of the document's size, drawn in its
    /// coordinates (effects, strokes and masks keep their document-unit sizes).
    pub scale: f64,
    /// Most motion-blur samples per frame.
    pub motion_blur_samples: usize,
    /// Most echo copies.
    pub echo_samples: usize,
    /// Most samples along a line (directional, radial and zoom blur, god rays) and lens-blur taps.
    pub line_samples: f64,
    /// Film grain and noise are drawn.
    pub grain: bool,
}

impl Tier {
    /// The tier of a quality: `final` as authored; `preview` at full size with at most 4
    /// motion-blur samples; `draft` at half size with at most 2 motion-blur samples, 4 echoes,
    /// 8 line samples, and no grain or noise.
    pub fn of(q: m::ProjectQuality) -> Tier {
        match q {
            m::ProjectQuality::Final => {
                Tier { scale: 1.0, motion_blur_samples: 256, echo_samples: 16, line_samples: 256.0, grain: true }
            }
            m::ProjectQuality::Preview => {
                Tier { scale: 1.0, motion_blur_samples: 4, echo_samples: 16, line_samples: 256.0, grain: true }
            }
            m::ProjectQuality::Draft => {
                Tier { scale: 0.5, motion_blur_samples: 2, echo_samples: 4, line_samples: 8.0, grain: false }
            }
        }
    }
}

/// Frames a cache entry may go unused before it is evicted.
const CACHE_KEEP: u64 = 4;

fn h(words: &[u64]) -> u64 {
    sr_eval::rng::hash(words)
}

/// Hash of the animated state of the elements outside the composition (paints, generators,
/// effects, lights): the same content as their JSON form, without formatting it.
fn elements_hash(elements: &[sr_eval::ElementState]) -> u64 {
    let mut f = Fnv(0xcbf2_9ce4_8422_2325);
    for e in elements {
        f.str(&e.key);
        f.str(e.element);
        f.u64(e.props.0.len() as u64);
        for (k, v) in &e.props.0 {
            f.str(k);
            f.value(v);
        }
    }
    h(&[f.0])
}

/// Hash of each animated element outside the composition, by key, so a node's caches can
/// depend on the elements it uses rather than on all of them.
fn element_hashes(elements: &[sr_eval::ElementState]) -> HashMap<Arc<str>, u64> {
    elements
        .iter()
        .map(|e| {
            let mut f = Fnv(0xcbf2_9ce4_8422_2325);
            f.str(e.element);
            f.u64(e.props.0.len() as u64);
            for (k, v) in &e.props.0 {
                f.str(k);
                f.value(v);
            }
            (e.key.clone(), f.0)
        })
        .collect()
}

/// Hash of animated values and of a node's parts, field by field, without formatting them as text.
fn props_hash(props: &sr_eval::Props, parts: &[sr_eval::ElementState]) -> u64 {
    let mut f = Fnv(0xcbf2_9ce4_8422_2325);
    f.u64(props.0.len() as u64);
    for (k, v) in &props.0 {
        f.str(k);
        f.value(v);
    }
    f.u64(parts.len() as u64);
    for p in parts {
        f.str(&p.key);
        f.str(p.element);
        f.u64(p.props.0.len() as u64);
        for (k, v) in &p.props.0 {
            f.str(k);
            f.value(v);
        }
    }
    h(&[f.0])
}

/// Ids of paints a set of animated values refers to.
fn paint_refs(props: &sr_eval::Props, out: &mut std::collections::BTreeSet<Arc<str>>) {
    for (_, v) in &props.0 {
        if let Value::PaintRef(id) = v {
            out.insert(id.clone());
        }
    }
}

/// Every attribute the schema types as a paint.
const PAINT_ATTRS: &[&str] = &[
    "fill",
    "stroke",
    "color",
    "colorEnd",
    "colorLow",
    "colorHigh",
    "background",
    "paint",
    "strokeColor",
    "outline",
    "headFill",
    "highlight",
    "activeColor",
    "noData",
];

/// Ids of paints that element `e` and its descendants name in their attributes.
fn static_paint_refs(e: &dyn Element, out: &mut std::collections::BTreeSet<Arc<str>>) {
    e.visit(&mut |d| {
        for name in PAINT_ATTRS {
            if let Some(AttrValue::Paint(sr_model::values::Paint::Ref(r))) = d.get_attr(name) {
                out.insert(Arc::from(r.0.as_str()));
            }
        }
    });
}

/// FNV-1a over tagged, length-prefixed fields.
struct Fnv(u64);

impl Fnv {
    fn byte(&mut self, b: u8) {
        self.0 ^= b as u64;
        self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
    }
    fn u64(&mut self, v: u64) {
        v.to_le_bytes().into_iter().for_each(|b| self.byte(b));
    }
    fn str(&mut self, s: &str) {
        self.u64(s.len() as u64);
        s.bytes().for_each(|b| self.byte(b));
    }
    fn len(&mut self, l: &sr_model::values::Length) {
        self.u64(l.value.to_bits());
        self.byte(l.unit as u8);
    }
    fn value(&mut self, v: &sr_eval::Value) {
        use sr_eval::Value as V;
        match v {
            V::Num(n) => {
                self.byte(1);
                self.u64(n.to_bits());
            }
            V::Bool(b) => {
                self.byte(2);
                self.byte(*b as u8);
            }
            V::Str(s) => {
                self.byte(3);
                self.str(s);
            }
            V::Len(l) => {
                self.byte(4);
                self.len(l);
            }
            V::Pair(p) => {
                self.byte(5);
                self.len(&p[0]);
                self.len(&p[1]);
            }
            V::Color(c) => {
                self.byte(6);
                c.iter().for_each(|x| self.u64(x.to_bits()));
            }
            V::PaintRef(s) => {
                self.byte(7);
                self.str(s);
            }
            V::List(l) => {
                self.byte(8);
                self.u64(l.len() as u64);
                l.iter().for_each(|x| self.u64(x.to_bits()));
            }
        }
    }
}

fn hf(v: f64) -> u64 {
    v.to_bits()
}

fn affine_hash(a: &Affine) -> u64 {
    h(&a.0.map(hf))
}

/// `frameBlend` of a layer or its time remap: 0 none, 1 frame mix, 2 optical flow.
fn frame_blend(e: &dyn Element) -> u8 {
    let code = |v: Option<AttrValue>| match v {
        Some(AttrValue::Str(s)) if s == "frame-mix" => 1,
        Some(AttrValue::Str(s)) if s == "optical-flow" => 2,
        _ => 0,
    };
    let own = code(e.get_attr("frameBlend"));
    if own != 0 {
        return own;
    }
    sr_model::element::children(e)
        .iter()
        .filter(|c| c.element_name() == "timeRemap")
        .map(|c| code(c.get_attr("frameBlend")))
        .max()
        .unwrap_or(0)
}

fn blend_index(e: &dyn Element) -> u32 {
    match e.get_attr("blend") {
        Some(AttrValue::Str(s)) => m::Blend::ALL.iter().position(|b| b.as_str() == s).unwrap_or(0) as u32,
        _ => 0,
    }
}

fn to_clip(p: [f64; 2], size: [u32; 2]) -> [f32; 4] {
    [(2.0 * p[0] / size[0] as f64 - 1.0) as f32, (1.0 - 2.0 * p[1] / size[1] as f64) as f32, 0.0, 1.0]
}

impl Renderer {
    /// Builds pipelines and resources for `program`.
    pub fn new(gpu: Gpu, program: &Program) -> Renderer {
        let d = &gpu.device;
        let storage = |b: u32| wgpu::BindGroupLayoutEntry {
            binding: b,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let tex = |b: u32| wgpu::BindGroupLayoutEntry {
            binding: b,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bgl0 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame"),
            entries: &[
                storage(0),
                storage(1),
                storage(2),
                storage(3),
                storage(4),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let bgl1 = resources::source_layout(d);
        let bgl2 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("backdrop+matte"),
            entries: &[tex(0), tex(1)],
        });
        let bgl3 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("generator"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("compositor"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(include_str!("common.wgsl"), include_str!("d24.wgsl"), include_str!("shaders.wgsl")).into(),
            ),
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("composite"),
            bind_group_layouts: &[Some(&bgl0), Some(&bgl1), Some(&bgl2), Some(&bgl3)],
            immediate_size: 0,
        });
        let vbuf = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x2, 2 => Float32x2],
        };
        let premul = wgpu::BlendState {
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
        let pipe =
            |fs: &str, blend: Option<wgpu::BlendState>, vs: &str, buffers: &[Option<wgpu::VertexBufferLayout>]| {
                d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(fs),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some(vs),
                        buffers,
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(fs),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: resources::FORMAT,
                            blend,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
            };
        let vbufs = [Some(vbuf)];
        let over = pipe("fs_over", Some(premul), "vs_main", &vbufs);
        let dst_over = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let under = pipe("fs_over", Some(wgpu::BlendState { color: dst_over, alpha: dst_over }), "vs_main", &vbufs);
        let blend = pipe("fs_blend", None, "vs_main", &vbufs);
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let additive = pipe("fs_over", Some(wgpu::BlendState { color: add, alpha: premul.alpha }), "vs_main", &vbufs);
        let generator = pipe("fs_generator", None, "vs_full", &[]);
        let sampler = |mode: wgpu::AddressMode| {
            d.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: mode,
                address_mode_v: mode,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            })
        };
        let samp = sampler(wgpu::AddressMode::ClampToEdge);
        let samp_repeat = sampler(wgpu::AddressMode::Repeat);
        let working = Working::of(&program.scene);
        let to_srgb = color::convert(working.space, m::ColorSpace::LinearSrgb);
        let from_srgb = color::convert(m::ColorSpace::LinearSrgb, working.space);
        let g = Globals {
            linear_light: working.linear as u32,
            // the project seed as a u64 (low, high) for seeded 64-bit hash draws
            seed: program.seed as u32,
            pad: [(program.seed >> 32) as u32, 0],
            to_srgb: types::mat3(&to_srgb),
            from_srgb: types::mat3(&from_srgb),
        };
        let globals = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue.write_buffer(&globals, 0, bytemuck::bytes_of(&g));
        let dummy = Arc::new(resources::create(d, &bgl1, [1, 1], 1, "dummy"));
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &dummy.tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[0u8; 8],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(8), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let max_texture = d.limits().max_texture_dimension_2d.min(8192);
        let (gpu_device, gpu_queue) = (gpu.device.clone(), gpu.queue.clone());
        let raster = Raster::new(&gpu.device);
        Renderer {
            seed: program.seed,
            tokens: token_table(&program.scene),
            gpu,
            over,
            under,
            additive,
            blend,
            generator,
            bgl0,
            bgl1,
            bgl2,
            bgl3,
            samp,
            samp_repeat,
            globals,
            dummy,
            working,
            images: HashMap::new(),
            image_notes: HashMap::new(),
            sim_textures: HashMap::new(),
            generators: HashMap::new(),
            subtree: HashMap::new(),
            used: Default::default(),
            last_used: HashMap::new(),
            cache_frame: 0,
            prefix: None,
            prev_root: Vec::new(),
            pool: Pool::default(),
            frame: None,
            fx: fx::FxEngine::new(gpu_device.clone(), gpu_queue.clone()),
            bare: Default::default(),
            in_transition: Default::default(),
            sampling: false,
            frame_rect: None,
            fx_cache: std::env::var_os("SR_FX_NO_CACHE").is_none(),
            three: None,
            three_assets: HashMap::new(),
            mtlx: HashMap::new(),
            ies: HashMap::new(),
            view_override: None,
            quality: None,
            time_gpu: false,
            last_timer: None,
            tier: Tier::of(m::ProjectQuality::Final),
            sphere: None,
            particles: None,
            contrast_probe: false,
            grid: None,
            pending_blur: Vec::new(),
            video: crate::video::VideoEngine::new(gpu_device, gpu_queue),
            decoders: HashMap::new(),
            video_frames: HashMap::new(),
            max_texture,
            representation: None,
            raster,
            lotties: HashMap::new(),
            svgs: HashMap::new(),
            text: Default::default(),
            clay_keys: HashMap::new(),
            glyph_tex: HashMap::new(),
            burn_captions: None,
            captions_off: false,
            audio: None,
            persistent_isf: None,
            tiles: Default::default(),
            last_frame: None,
            frame_bufs: None,
            last_submit: None,
        }
    }

    /// The working colour space of the frames.
    pub fn working(&self) -> Working {
        self.working
    }

    /// The device.
    /// GPU time of the last frame, with `time_gpu` on a device with timestamp queries; waits
    /// for that frame's work to complete.
    pub fn gpu_times(&self) -> Option<fx::GpuTimes> {
        let t = self.last_timer.as_ref()?;
        Some(t.read(&self.gpu.device, self.gpu.queue.get_timestamp_period()))
    }

    pub fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    // ---------------------------------------------------------- assets

    fn asset<'p>(&self, p: &'p Program, key: &str) -> Option<(&'p AssetsChild, usize)> {
        let (doc, id) = p.assets.get(key)?;
        let scene = if *doc == 0 { &p.scene } else { &p.includes.get(*doc as usize - 1)?.1 };
        let a = scene.assets.as_ref()?.children.iter().find(|c| c.id() == Some(id.as_str()))?;
        Some((a, *doc as usize))
    }

    fn image(
        &mut self,
        path: std::path::PathBuf,
        space: m::ColorSpace,
        transfer: m::Transfer,
        alpha: m::AlphaMode,
        embedded: bool,
    ) -> Option<Arc<Tex>> {
        let key = format!("{}|{space}|{transfer}|{alpha}|{embedded}", path.display());
        if let Some(t) = self.images.get(&key) {
            return t.clone();
        }
        let t = match resources::decode_image(&path, space, transfer, alpha, embedded, &self.working, self.max_texture)
        {
            Ok(d) => {
                if let Some(note) = &d.note {
                    self.image_notes.insert(path.clone(), note.clone());
                }
                Some(Arc::new(resources::upload(&self.gpu.device, &self.gpu.queue, &self.bgl1, &d, "image")))
            }
            Err(_) => None,
        };
        self.images.insert(key, t.clone());
        t
    }

    /// Source texture for a layer's asset at a source time.
    /// Source path, primaries and transfer of an asset, honouring the
    /// preferred representation.
    fn representation<'a>(
        &self,
        reps: &'a [m::Representation],
        src: &'a str,
        space: m::ColorSpace,
        transfer: m::Transfer,
    ) -> (&'a str, m::ColorSpace, m::Transfer) {
        match self.representation.as_deref().and_then(|name| reps.iter().find(|r| r.name == name)) {
            Some(r) => (r.src.as_str(), r.color_space.unwrap_or(space), r.transfer.unwrap_or(transfer)),
            None => (src, space, transfer),
        }
    }

    /// Draws a grid simulation's picture over the node's box.
    #[allow(clippy::too_many_arguments)]
    fn emit_sim_image(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        op: f64,
        blend: u32,
        seed: u32,
        cmds: &mut Vec<Cmd>,
        root_hash: u64,
    ) {
        let n = &ctx.g.nodes[i];
        let (Some(img), Some([bw, bh])) = (n.sim_image.clone(), n.size) else { return };
        let tex = match self.sim_textures.get(&n.id) {
            Some((k, t)) if *k == img.key => t.clone(),
            _ => {
                let working = self.working;
                let px: Vec<[f32; 4]> = img
                    .rgba
                    .iter()
                    .map(|p| {
                        let a = p[3] as f64;
                        if a <= 0.0 {
                            return [0.0; 4];
                        }
                        let c = working.from_linear_srgb([p[0] as f64 / a, p[1] as f64 / a, p[2] as f64 / a, a]);
                        [(c[0] * a) as f32, (c[1] * a) as f32, (c[2] * a) as f32, a as f32]
                    })
                    .collect();
                let d = resources::Decoded { levels: resources::mips(img.width, img.height, px), note: None };
                let t = Arc::new(resources::upload(&self.gpu.device, &self.gpu.queue, &self.bgl1, &d, "simulation"));
                self.sim_textures.insert(n.id.clone(), (img.key, t.clone()));
                t
            }
        };
        let d = Draw {
            opacity: op as f32,
            blend,
            src_kind: src::TEXTURE,
            seed,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            ..Default::default()
        };
        let hash = h(&[root_hash, sr_eval::rng::hash_str(&n.id), img.key, hf(op), 0x5157]);
        self.draw_cmd(
            plan,
            ctx,
            i,
            space,
            d,
            [0.0, 0.0, bw, bh],
            [0.0, 0.0, 1.0, 1.0],
            &n.world,
            tex,
            cmds,
            hash,
            false,
        );
    }

    fn solid_texture(&mut self, rgba: [f32; 4]) -> Arc<Tex> {
        let key = format!("solid:{rgba:?}");
        if let Some(Some(t)) = self.images.get(&key) {
            return t.clone();
        }
        let d = resources::Decoded { levels: vec![(1, 1, vec![rgba])], note: None };
        let t = Arc::new(resources::upload(&self.gpu.device, &self.gpu.queue, &self.bgl1, &d, "solid"));
        self.images.insert(key, Some(t.clone()));
        t
    }

    /// Source texture for a layer's asset at a source time.
    fn layer_source(&mut self, plan: &mut Plan, p: &Program, g: &FrameGraph, n: &FrameNode) -> Option<Arc<Tex>> {
        let key = n.asset.as_deref()?;
        let (a, doc) = self.asset(p, key)?;
        let base = p.base_dirs.get(doc).cloned().unwrap_or_default();
        match a {
            AssetsChild::Image(i) => {
                let (src, space, transfer) = self.representation(&i.representations, &i.src, i.color_space, i.transfer);
                match sr_model::assets::resolve(src, &base) {
                    sr_model::assets::Resolved::Local(path) => {
                        let t = self.image(
                            path.clone(),
                            space,
                            transfer,
                            i.alpha,
                            i.color_profile == sr_model::model::ColorProfile::Embedded,
                        );
                        if t.is_none() {
                            plan.stats.errors.push(format!("{}: cannot read image {}", n.id, path.display()));
                        }
                        if let Some(note) = self.image_notes.get(&path) {
                            plan.stats.unsupported.push(format!("{}: {}: {note}", n.id, path.display()));
                        }
                        t
                    }
                    sr_model::assets::Resolved::Remote(u) => {
                        plan.stats.errors.push(format!("{}: remote image {u} is not fetched while rendering", n.id));
                        None
                    }
                }
            }
            AssetsChild::ImageSequence(s) => {
                let t = n.source_time.unwrap_or(0.0).max(0.0);
                let count = ((s.last - s.first) / s.step as i64).max(0);
                let k = ((t * s.fps.as_f64() + 1e-9).floor() as i64).min(count);
                let hold = if s.missing_frame == m::MissingFrame::Hold { k } else { 0 };
                for back in 0..=hold {
                    let frame = s.first + (k - back) * s.step as i64;
                    let file = sr_model::assets::sequence_frame(&s.src, frame)?;
                    if let sr_model::assets::Resolved::Local(path) = sr_model::assets::resolve(&file, &base) {
                        if path.is_file() {
                            return self.image(
                                path,
                                s.color_space,
                                s.transfer,
                                s.alpha,
                                s.color_profile == sr_model::model::ColorProfile::Embedded,
                            );
                        }
                    }
                }
                let frame = s.first + k * s.step as i64;
                match s.missing_frame {
                    m::MissingFrame::Black => Some(self.solid_texture([0.0, 0.0, 0.0, 1.0])),
                    m::MissingFrame::Transparent | m::MissingFrame::Hold => None,
                    m::MissingFrame::Error => {
                        plan.stats
                            .errors
                            .push(format!("{}: frame {frame} of image sequence {} is missing", n.id, s.id));
                        None
                    }
                }
            }
            AssetsChild::Video(v) => {
                let (src, space, transfer) = self.representation(&v.representations, &v.src, v.color_space, v.transfer);
                let path = match sr_model::assets::resolve(src, &base) {
                    sr_model::assets::Resolved::Local(path) => path,
                    sr_model::assets::Resolved::Remote(u) => {
                        plan.stats.errors.push(format!("{}: remote video {u} is not fetched while rendering", n.id));
                        return None;
                    }
                };
                let blend = frame_blend(&*n.elem);
                let rotation = v.rotation;
                self.video_texture(plan, n, &path, v.fps.as_f64(), space, transfer, v.alpha, rotation, blend)
            }
            AssetsChild::Generator(gen) => Some(self.generator_texture(plan, p, g, gen)),
            AssetsChild::Generated(gm) if matches!(gm.kind.as_str(), "image" | "video") => {
                // generated media renders from its verified cache file
                let path = match sr_model::assets::resolve(&gm.cache, &base) {
                    sr_model::assets::Resolved::Local(path) => path,
                    sr_model::assets::Resolved::Remote(u) => {
                        plan.stats.errors.push(format!("{}: remote cache {u} is not fetched while rendering", n.id));
                        return None;
                    }
                };
                if gm.kind.as_str() == "image" {
                    let t = self.image(path.clone(), m::ColorSpace::Srgb, m::Transfer::Auto, m::AlphaMode::Auto, true);
                    if t.is_none() {
                        plan.stats.errors.push(format!("{}: cannot read generated image {}", n.id, path.display()));
                    }
                    t
                } else {
                    let fps = gm.fps.map(|f| f.as_f64()).unwrap_or(0.0);
                    let fps = if fps > 0.0 {
                        fps
                    } else {
                        sr_media::probe(&path).ok().and_then(|i| i.video).map(|v| v.fps).unwrap_or(25.0)
                    };
                    self.video_texture(
                        plan,
                        n,
                        &path,
                        fps,
                        m::ColorSpace::Srgb,
                        m::Transfer::Auto,
                        m::AlphaMode::Auto,
                        m::VideoAssetRotation::V0,
                        frame_blend(&*n.elem),
                    )
                }
            }
            other => {
                let batch = match other.element_name() {
                    "generated" => "audio only",
                    "mesh" => "mesh assets draw through object3D",
                    _ => "not drawable",
                };
                plan.stats.unsupported.push(format!("{}: {} asset ({batch})", n.id, other.element_name()));
                None
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn video_texture(
        &mut self,
        plan: &mut Plan,
        n: &FrameNode,
        path: &std::path::Path,
        fps: f64,
        space: m::ColorSpace,
        transfer: m::Transfer,
        alpha: m::AlphaMode,
        rotation: m::VideoAssetRotation,
        blend: u8,
    ) -> Option<Arc<Tex>> {
        let pkey = path.display().to_string();
        if !self.decoders.contains_key(&pkey) {
            let d = sr_media::VideoDecoder::open(path, Some(fps), 8).map_err(|e| e.to_string());
            self.decoders.insert(pkey.clone(), d);
        }
        let x = n.source_time.unwrap_or(0.0).max(0.0) * fps;
        let n0 = libm::floor(x + 1e-6) as i64;
        let f = (x - n0 as f64).clamp(0.0, 1.0);
        let frame_tex = |this: &mut Self, plan: &mut Plan, k: i64| -> Option<Arc<Tex>> {
            let fkey = format!("{pkey}#{k}");
            this.used.insert(format!("video:{fkey}"));
            if let Some(t) = this.video_frames.get(&fkey) {
                return Some(t.clone());
            }
            let dec = match this.decoders.get_mut(&pkey)? {
                Ok(d) => d,
                Err(e) => {
                    plan.stats.errors.push(format!("{}: {e}", n.id));
                    return None;
                }
            };
            let waited = std::time::Instant::now();
            let decoded = dec.frame(k);
            plan.stats.decode_wait += waited.elapsed().as_secs_f64();
            let frame = match decoded {
                Ok(f) => f,
                Err(e) => {
                    plan.stats.errors.push(format!("{}: {e}", n.id));
                    return None;
                }
            };
            let info = dec.info.clone();
            let quarter = match rotation {
                m::VideoAssetRotation::V0 => (info.rotation / 90) as u32,
                m::VideoAssetRotation::V90 => 1,
                m::VideoAssetRotation::V180 => 2,
                m::VideoAssetRotation::V270 => 3,
            };
            let transfer = if transfer == m::Transfer::Auto {
                crate::video::Interpretation::transfer_for(&info.transfer).unwrap_or(color::default_transfer(space))
            } else {
                transfer
            };
            let it = crate::video::Interpretation {
                space,
                transfer,
                alpha: if info.alpha { alpha } else { m::AlphaMode::None },
                quarter_turns: quarter,
                matrix: crate::video::Interpretation::matrix_for(&info.matrix, info.height),
                full_range: info.full_range || info.rgb,
            };
            let working = this.working;
            let t = Arc::new(this.video.convert(&frame, &it, &working, &this.bgl1));
            plan.stats.video_frames += 1;
            this.video_frames.insert(fkey, t.clone());
            Some(t)
        };
        let a = frame_tex(self, plan, n0)?;
        if blend == 0 || f < 1e-3 {
            return Some(a);
        }
        let b = frame_tex(self, plan, n0 + 1)?;
        let bkey = format!("{pkey}#{n0}~{}~{blend}", (f * 1e4).round());
        self.used.insert(format!("video:{bkey}"));
        if let Some(t) = self.video_frames.get(&bkey) {
            return Some(t.clone());
        }
        let t = Arc::new(if blend == 1 {
            self.video.mix(&a, &b, f as f32, &self.bgl1)
        } else {
            self.video.interpolate(&a, &b, f as f32, &self.bgl1)
        });
        self.video_frames.insert(bkey, t.clone());
        Some(t)
    }

    fn generator_texture(&mut self, plan: &mut Plan, p: &Program, g: &FrameGraph, gen: &m::GeneratorAsset) -> Arc<Tex> {
        let key = gen.id.as_str();
        let el = g.elements.iter().find(|e| &*e.key == key);
        let num = |name: &str, d: f64| el.and_then(|e| e.props.get(name)).and_then(Value::as_num).unwrap_or(d);
        let paint_of = |name: &str, fallback: &sr_model::values::Paint| -> Value {
            el.and_then(|e| e.props.get(name)).cloned().unwrap_or_else(|| crate::value_of_paint(fallback))
        };
        let (w, hgt) = ((gen.width as u32).min(self.max_texture), (gen.height as u32).min(self.max_texture));
        let pa = paint_of("paint", &gen.paint);
        let pb = paint_of("paint2", &gen.paint2);
        let tokens = self.tokens.clone();
        let tok = |t: &str| tokens.get(t).copied();
        let paint_index = |v: &Value, plan: &mut Plan| -> u32 {
            match v {
                Value::PaintRef(id) => {
                    if let Some(pc) =
                        p.scene.paints.as_ref().and_then(|ps| ps.children.iter().find(|c| c.id() == Some(id)))
                    {
                        if let Some(i) =
                            plan.paints.gradient(&self.working, pc, g, [0.0, 0.0, w as f64, hgt as f64], &tok)
                        {
                            return i;
                        }
                    }
                    plan.paints.solid(&self.working, [0.0; 4])
                }
                Value::Color(c) => plan.paints.solid(&self.working, *c),
                _ => plan.paints.solid(&self.working, [0.0; 4]),
            }
        };
        let (ia, ib) = (paint_index(&pa, plan), paint_index(&pb, plan));
        let kind = m::GeneratorAssetKind::ALL.iter().position(|k| *k == gen.kind).unwrap_or(0) as u32;
        // seeded 64-bit hash draws with generator seed
        let seed = sr_eval::rng::element_seed(p.seed, &gen.id, gen.seed, "generator");
        let evolution = num("evolution", gen.evolution);
        let frame = libm::floor(g.time * p.fps.as_f64() + 1e-6) as i64;
        let grain = (seed as i64)
            .wrapping_add(frame.wrapping_mul(7919))
            .wrapping_add((evolution * 1000.0).round_ties_even() as i64 * 104_729) as u64;
        let mut perm = [[0u32; 4]; 64];
        for (i, v) in sr_eval::rng::permutation(seed, 0, 256).into_iter().enumerate() {
            perm[i / 4][i % 4] = v as u32;
        }
        let u = Gen {
            size: [w as f32, hgt as f32],
            kind,
            octaves: num("octaves", gen.octaves as f64) as u32,
            scale: num("scale", gen.scale.get()) as f32,
            evolution: evolution as f32,
            contrast: num("contrast", gen.contrast) as f32,
            angle: num("angle", gen.angle) as f32,
            seed: seed as u32,
            paint_a: ia,
            paint_b: ib,
            seed_hi: (seed >> 32) as u32,
            grain: [grain as u32, (grain >> 32) as u32, 0, 0],
            perm,
        };
        let pd = |i: u32| {
            let d = plan.paints.paints[i as usize];
            let stops: Vec<u64> = plan.paints.stops[d.stop_off as usize..(d.stop_off + d.stop_count) as usize]
                .iter()
                .flat_map(|s| bytemuck::cast_slice::<_, u32>(&[*s]).iter().map(|x| *x as u64).collect::<Vec<_>>())
                .collect();
            h(&[
                h(bytemuck::cast_slice::<_, u32>(&[d]).iter().map(|x| *x as u64).collect::<Vec<_>>().as_slice()),
                h(&stops),
            ])
        };
        let hash = h(&[
            h(bytemuck::cast_slice::<_, u32>(&[u]).iter().map(|x| *x as u64).collect::<Vec<_>>().as_slice()),
            pd(ia),
            pd(ib),
        ]);
        self.used.insert(format!("gen:{hash}"));
        if let Some(t) = self.generators.get(&hash) {
            plan.stats.cache_hits += 1;
            return t.clone();
        }
        let target = Arc::new(resources::create(&self.gpu.device, &self.bgl1, [w, hgt], 1, "generator"));
        let ubuf = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gen"),
            size: std::mem::size_of::<Gen>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.gpu.queue.write_buffer(&ubuf, 0, bytemuck::bytes_of(&u));
        let bind = self.gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gen"),
            layout: &self.bgl3,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }],
        });
        self.generators.insert(hash, target.clone());
        plan.gens.push(GenJob { target: target.clone(), bind });
        target
    }

    // ---------------------------------------------------------- planning

    fn masks_of(&self, plan: &mut Plan, n: &FrameNode, bx: [f64; 2], extra_clip: Option<[f64; 2]>) -> (u32, u32) {
        let off = plan.masks.len() as u32;
        if self.bare.contains(&n.id) {
            return (off, 0);
        }
        let children: Vec<&dyn Element> = sr_model::element::children(&*n.elem);
        let mut k = 0usize;
        for c in children {
            if c.element_name() != "mask" {
                continue;
            }
            let Some(mk) = c.as_any().downcast_ref::<m::Mask>() else { continue };
            let key = format!("{}/mask[{k}]", n.id);
            k += 1;
            let part = n.parts.iter().find(|p| *p.key == key);
            let pv = |name: &str| part.and_then(|p| p.props.get(name));
            let len = |name: &str, d: sr_model::values::Length, extent: f64| -> f64 {
                match pv(name) {
                    Some(Value::Len(l)) => l.resolve(extent, bx[0], bx[1]),
                    Some(Value::Num(x)) => *x,
                    _ => d.resolve(extent, bx[0], bx[1]),
                }
            };
            let num = |name: &str, d: f64| pv(name).and_then(Value::as_num).unwrap_or(d);
            let size = n.size.unwrap_or(bx);
            let (x, y) = (len("x", mk.x, size[0]), len("y", mk.y, size[1]));
            let w = mk.width.map(|l| len("width", l, size[0])).unwrap_or(size[0]);
            let hh = mk.height.map(|l| len("height", l, size[1])).unwrap_or(size[1]);
            let mode = m::MaskMode::ALL.iter().position(|v| *v == mk.mode).unwrap_or(0) as u32;
            let mut mask = Mask {
                rect: [x as f32, y as f32, w as f32, hh as f32],
                kind: 0,
                mode,
                invert: mk.invert as u32,
                fill_rule: (mk.fill_rule == m::FillRule::Evenodd) as u32,
                feather: num("feather", mk.feather.get()) as f32,
                expansion: num("expansion", mk.expansion) as f32,
                radius: num("radius", mk.radius.get()) as f32,
                opacity: num("opacity", mk.opacity.get()) as f32,
                ..Default::default()
            };
            let polygon: Option<Vec<Vec<[f64; 2]>>> = match mk.r#type {
                m::MaskKind::Rect => None,
                m::MaskKind::Ellipse => {
                    mask.kind = 1;
                    None
                }
                m::MaskKind::RoundedRect => {
                    mask.kind = 2;
                    None
                }
                m::MaskKind::Polygon | m::MaskKind::Star => {
                    // vertices on the ellipse inscribed in the box, from the top,
                    // clockwise; a star's inner vertices on that ellipse scaled by `innerRadius` (0.5)
                    let pts = mk.points.max(3) as usize;
                    let (cx, cy, rx, ry) = (x + w / 2.0, y + hh / 2.0, w / 2.0, hh / 2.0);
                    let inner = mk.inner_radius.map(|v| v.get()).unwrap_or(0.5);
                    let star = mk.r#type == m::MaskKind::Star;
                    let count = if star { pts * 2 } else { pts };
                    Some(vec![(0..count)
                        .map(|i| {
                            let a = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::TAU / count as f64;
                            let k = if star && i % 2 == 1 { inner } else { 1.0 };
                            [cx + k * rx * libm::cos(a), cy + k * ry * libm::sin(a)]
                        })
                        .collect()])
                }
                m::MaskKind::Path => {
                    let d = match pv("path") {
                        Some(Value::Str(s)) => s.to_string(),
                        _ => mk.path.clone().unwrap_or_default(),
                    };
                    sr_eval::path::flatten(&d, 0.25).ok().map(|subs| {
                        subs.into_iter().map(|s| s.into_iter().map(|q| [q[0] + x, q[1] + y]).collect()).collect()
                    })
                }
            };
            if let Some(polys) = polygon {
                mask.kind = 3;
                mask.edge_off = plan.edges.len() as u32;
                for poly in polys {
                    for i in 0..poly.len() {
                        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                        plan.edges.push([a[0] as f32, a[1] as f32, b[0] as f32, b[1] as f32]);
                    }
                }
                mask.edge_count = plan.edges.len() as u32 - mask.edge_off;
            }
            plan.masks.push(mask);
        }
        if let Some([w, hh]) = extra_clip {
            // the clip box intersects whatever the masks produced
            plan.masks.push(Mask {
                rect: [0.0, 0.0, w as f32, hh as f32],
                mode: 0,
                opacity: 1.0,
                ..Default::default()
            });
        }
        (off, plan.masks.len() as u32 - off)
    }

    #[allow(clippy::too_many_arguments)]
    fn push_quad(
        &self,
        plan: &mut Plan,
        space: &Space,
        world: &Affine,
        three_d: Option<([f64; 3], [f64; 2])>,
        local: [f64; 4],
        uv: [f64; 4],
        proj: &glam::Mat4,
    ) -> ([u32; 4], u32) {
        let xf = space.xform.then(world);
        // grow the quad by about a pixel so the shader can antialias its edges
        let [a, b, c, d, _, _] = xf.0;
        let persp = if three_d.is_some() { 3.0 } else { 1.5 };
        let mx = persp / (a * a + b * b).sqrt().max(1e-9);
        let my = persp / (c * c + d * d).sqrt().max(1e-9);
        let (du, dv) = (
            (uv[2] - uv[0]) / (local[2] - local[0]).abs().max(1e-12),
            (uv[3] - uv[1]) / (local[3] - local[1]).abs().max(1e-12),
        );
        let (sx, sy) = ((local[2] - local[0]).signum(), (local[3] - local[1]).signum());
        let lx = [local[0] - mx * sx, local[1] - my * sy, local[2] + mx * sx, local[3] + my * sy];
        let ux = [uv[0] - du * mx, uv[1] - dv * my, uv[2] + du * mx, uv[3] + dv * my];
        let corners = [[lx[0], lx[1]], [lx[2], lx[1]], [lx[2], lx[3]], [lx[0], lx[3]]];
        let uvs = [[ux[0], ux[1]], [ux[2], ux[1]], [ux[2], ux[3]], [ux[0], ux[3]]];
        let mut clips = [[0f32; 4]; 4];
        let mut pix = [[0f64; 2]; 4];
        for i in 0..4 {
            let p = xf.apply(corners[i]);
            match three_d {
                None => {
                    clips[i] = to_clip(p, space.size);
                    pix[i] = p;
                }
                Some(([z, rx, ry], anchor)) => {
                    let a = xf.apply(anchor);
                    let (mut vx, mut vy, mut vz) = (p[0] - a[0], p[1] - a[1], 0.0);
                    // rotationX > 0 turns the top edge away (+z), rotationY > 0 the right edge
                    let (sx, cx) = (libm::sin(rx.to_radians()), libm::cos(rx.to_radians()));
                    let ny = vy * cx + vz * sx;
                    vz = -vy * sx + vz * cx;
                    vy = ny;
                    let (sy, cy) = (libm::sin(ry.to_radians()), libm::cos(ry.to_radians()));
                    let nx = vx * cy - vz * sy;
                    vz = vx * sy + vz * cy;
                    vx = nx;
                    let (px, py, pz) = (a[0] + vx, a[1] + vy, z + vz);
                    // through the frame camera (target px → frame → clip → target clip)
                    let c = *proj * glam::Vec4::new(px as f32, py as f32, pz as f32, 1.0);
                    // true clip coordinates: the rasteriser clips what lies behind the eye (w < 0 fails 0 ≤ z ≤ w)
                    clips[i] = [c.x, c.y, 0.0, c.w];
                    let wv = c.w.max(1e-3);
                    let (w, hh) = (space.size[0] as f64, space.size[1] as f64);
                    pix[i] = [((c.x / wv) as f64 * 0.5 + 0.5) * w, (0.5 - (c.y / wv) as f64 * 0.5) * hh];
                }
            }
        }
        let first = plan.verts.len() as u32;
        for i in [0usize, 1, 2, 0, 2, 3] {
            plan.verts.push(Vertex {
                clip: clips[i],
                uv: uvs[i].map(|v| v as f32),
                local: corners[i].map(|v| v as f32),
            });
        }
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for q in pix {
            b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])];
        }
        let clamp = |v: f64, hi: u32| v.clamp(0.0, hi as f64) as u32;
        let bounds = [
            clamp(b[0].floor() - 1.0, space.size[0]),
            clamp(b[1].floor() - 1.0, space.size[1]),
            clamp(b[2].ceil() + 1.0, space.size[0]),
            clamp(b[3].ceil() + 1.0, space.size[1]),
        ];
        (bounds, first)
    }

    /// State of one node that affects its own pixels, with its transform
    /// relative to the target space. Opacity is the node's own; ancestors
    /// contribute theirs through their own hashes.
    /// A quad covering the whole target whose local and texture coordinates
    /// extend those of the rectangle `local` → `uv` under `world`.
    fn push_full(
        &self,
        plan: &mut Plan,
        space: &Space,
        world: &Affine,
        local: [f64; 4],
        uv: [f64; 4],
    ) -> ([u32; 4], u32) {
        let inv = space.xform.then(world).inverse().unwrap_or(Affine::IDENTITY);
        let (w, hh) = (space.size[0] as f64, space.size[1] as f64);
        let px = [[0.0, 0.0], [w, 0.0], [w, hh], [0.0, hh]];
        let first = plan.verts.len() as u32;
        for i in [0usize, 1, 2, 0, 2, 3] {
            let l = inv.apply(px[i]);
            let u = [
                uv[0] + (l[0] - local[0]) / (local[2] - local[0]) * (uv[2] - uv[0]),
                uv[1] + (l[1] - local[1]) / (local[3] - local[1]) * (uv[3] - uv[1]),
            ];
            plan.verts.push(Vertex {
                clip: to_clip(px[i], space.size),
                uv: u.map(|v| v as f32),
                local: l.map(|v| v as f32),
            });
        }
        ([0, 0, space.size[0], space.size[1]], first)
    }

    /// A quad covering the target 1:1 (frame-space offscreens), with local
    /// coordinates in the space of `world` so masks apply.
    fn push_frame(&mut self, plan: &mut Plan, space: &Space, world: &Affine) -> ([u32; 4], u32) {
        let inv = space.xform.then(world).inverse().unwrap_or(Affine::IDENTITY);
        let (w, hh) = (space.size[0] as f64, space.size[1] as f64);
        let r = self.frame_rect.take().unwrap_or([0.0, 0.0, w, hh]);
        let px = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]];
        let uvs = [[0.0f32, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let first = plan.verts.len() as u32;
        for i in [0usize, 1, 2, 0, 2, 3] {
            let l = inv.apply(px[i]);
            plan.verts.push(Vertex { clip: to_clip(px[i], space.size), uv: uvs[i], local: l.map(|v| v as f32) });
        }
        let b = [
            r[0].max(0.0) as u32,
            r[1].max(0.0) as u32,
            (r[2].ceil() as u32).min(space.size[0]),
            (r[3].ceil() as u32).min(space.size[1]),
        ];
        (b, first)
    }

    fn node_hash(ctx: &Ctx, n: &FrameNode, rel: &Affine) -> u64 {
        let props = if n.props.0.is_empty() && n.parts.is_empty() { 0 } else { props_hash(&n.props, &n.parts) };
        let asset = n.asset.as_deref();
        let timed = asset.filter(|a| ctx.timed.contains(*a)).and(n.source_time).map(hf).unwrap_or(1);
        let generated = match asset.filter(|a| ctx.generated.contains(*a)) {
            Some(a) => Self::element_state(ctx, a),
            None => 0,
        };
        h(&[
            Self::effects_state(ctx, n),
            sr_eval::rng::hash_str(&n.id),
            Arc::as_ptr(&n.elem) as u64,
            affine_hash(rel),
            hf(n.opacity),
            timed,
            generated,
            n.draw as u64,
            n.three_d.map(|t| h(&t.map(hf))).unwrap_or(2),
            n.content.map(|c| h(&[h(&c.dest.map(hf)), h(&c.uv.map(hf))])).unwrap_or(3),
            n.size.map(|s| h(&s.map(hf))).unwrap_or(4),
            props,
            n.soft.as_ref().map(|s| h(&s.offsets.iter().flat_map(|o| o.map(hf)).collect::<Vec<u64>>())).unwrap_or(5),
            // simulated content changes without the node's own attributes changing
            n.sim_image.as_ref().map(|s| s.key).unwrap_or(6),
            n.particles
                .as_ref()
                .map(|p| {
                    h(&p.pos.iter().chain(&p.vel).flat_map(|q| q.map(|v| v.to_bits() as u64)).collect::<Vec<u64>>())
                })
                .unwrap_or(7),
        ])
    }

    /// Hash of a node and its subtree (and mattes) in a target space.
    fn subtree_hash(ctx: &Ctx, i: usize, to_space: &Affine) -> u64 {
        let n = &ctx.g.nodes[i];
        let rel = to_space.then(&n.world);
        let mut words = vec![Self::node_hash(ctx, n, &rel)];
        if let Some(mt) = n.matte {
            words.push(Self::subtree_hash(ctx, mt as usize, to_space));
            words.push(hf(ctx.g.nodes[mt as usize].world_opacity));
        }
        for &c in &ctx.kids[i] {
            words.push(Self::subtree_hash(ctx, c, to_space));
        }
        h(&words)
    }

    /// Hash of an isolated node's content: its children, not its own transform or opacity.
    /// The state of the node's effects: their animated values, and the time for effects that
    /// change with it (shaders see `iTime`). Without it a node whose only change is in an effect
    /// (an animated shader parameter) kept its cached pixels from an earlier frame.
    fn effects_state(ctx: &Ctx, n: &FrameNode) -> u64 {
        let ids = render_fx::effect_ids(&*n.elem);
        if ids.is_empty() {
            return 8;
        }
        let mut words = Vec::with_capacity(ids.len() * 2 + 1);
        let mut timed = false;
        for id in &ids {
            let Some(e) = render_fx::find_effect(ctx.p, id) else { continue };
            if e.r#type.as_str() == "posterize-time" {
                // it shows the node as it was at the start of its step: the step is its time
                let a = Attrs { e: e as &dyn Element, props: render_fx::element_props(ctx.g, id) };
                words.push(hf(render_fx::posterized(ctx.g.time, &a)));
            } else {
                timed |= render_fx::TIME_VARYING.contains(&e.r#type.as_str());
            }
            let props = render_fx::element_props(ctx.g, id).map(|p| props_hash(p, &[])).unwrap_or(9);
            words.extend([sr_eval::rng::hash_str(id), props]);
        }
        if timed {
            words.push(hf(ctx.g.time));
        }
        h(&words)
    }

    /// The asset element with id `id` in the main document.
    fn asset_element<'p>(p: &'p Program, id: &str) -> Option<&'p dyn Element> {
        let assets = p.scene.assets.as_ref()?;
        sr_model::element::children(assets).into_iter().find(|a| a.element_id() == Some(id))
    }

    /// Hash of one animated element and its animated descendants (`id/…` keys).
    fn element_state(ctx: &Ctx, id: &str) -> u64 {
        let mut words: Vec<u64> = ctx
            .el
            .iter()
            .filter(|(k, _)| &***k == id || (k.starts_with(id) && k[id.len()..].starts_with('/')))
            .map(|(k, v)| h(&[sr_eval::rng::hash_str(k), *v]))
            .collect();
        words.sort_unstable();
        h(&words)
    }

    /// Hash of the animated elements outside the composition that node `i`, its subtree and
    /// its mattes use (the paints they fill and stroke with, generator assets, and the paints
    /// and lights of their effects), and of the camera. A node's cached effect result depends
    /// on these and on nothing else outside its own subtree.
    fn deps_hash(ctx: &Ctx, i: usize) -> u64 {
        let mut ids = std::collections::BTreeSet::new();
        let mut stack = vec![i];
        while let Some(k) = stack.pop() {
            let n = &ctx.g.nodes[k];
            paint_refs(&n.props, &mut ids);
            // static references: the node's attributes and parts (spans, masks), and the asset it draws
            for name in PAINT_ATTRS {
                if let Some(AttrValue::Paint(sr_model::values::Paint::Ref(r))) = n.elem.get_attr(name) {
                    ids.insert(Arc::from(r.0.as_str()));
                }
            }
            for c in sr_model::element::children(&*n.elem) {
                static_paint_refs(c, &mut ids);
            }
            if let Some(a) = n.asset.as_deref().and_then(|a| Self::asset_element(ctx.p, a)) {
                static_paint_refs(a, &mut ids);
            }
            for part in &n.parts {
                paint_refs(&part.props, &mut ids);
            }
            if let Some(a) = n.asset.as_deref().filter(|a| ctx.generated.contains(*a)) {
                ids.insert(Arc::from(a));
            }
            for id in render_fx::effect_ids(&*n.elem) {
                if let Some(p) = render_fx::element_props(ctx.g, &id) {
                    paint_refs(p, &mut ids);
                }
                if let Some(e) = render_fx::find_effect(ctx.p, &id) {
                    let e: &dyn Element = e;
                    for name in ["lights", "paint", "source"] {
                        match e.get_attr(name) {
                            Some(AttrValue::Tokens(t)) => ids.extend(t.into_iter().map(Arc::from)),
                            Some(AttrValue::Str(s)) => {
                                ids.insert(Arc::from(s.trim_start_matches("url(#").trim_end_matches(')')));
                            }
                            Some(AttrValue::Paint(sr_model::values::Paint::Ref(r))) => {
                                ids.insert(Arc::from(r.0.as_str()));
                            }
                            _ => {}
                        }
                    }
                }
            }
            if let Some(mt) = n.matte {
                stack.push(mt as usize);
            }
            stack.extend(ctx.kids[k].iter().copied());
        }
        let mut words: Vec<u64> =
            ids.iter().map(|id| h(&[sr_eval::rng::hash_str(id), Self::element_state(ctx, id)])).collect();
        words.push(ctx.cam);
        h(&words)
    }

    /// Hash of how node `i` looks, apart from where it is: everything its drawing depends on
    /// except its own transform (its children are hashed relative to it). Equal hashes at two
    /// times mean one drawing, moved, is the other. `None` when its transform cannot be inverted.
    fn appearance_hash(ctx: &Ctx, i: usize) -> Option<u64> {
        const WHERE: &[&str] = &[
            "x",
            "y",
            "z",
            "anchorX",
            "anchorY",
            "anchor",
            "rotation",
            "scaleX",
            "scaleY",
            "scale",
            "skewX",
            "skewY",
            "skew",
            "zDepth",
            "rotationX",
            "rotationY",
        ];
        let n = &ctx.g.nodes[i];
        let local = n.world.inverse()?;
        let mut f = Fnv(0xcbf2_9ce4_8422_2325);
        for (k, v) in n.props.0.iter().filter(|(k, _)| !WHERE.contains(&&**k)) {
            f.str(k);
            f.value(v);
        }
        // a motion path moves the node; its other parts (masks, spans) change how it looks
        for p in n.parts.iter().filter(|p| p.element != "motionPath") {
            f.str(&p.key);
            for (k, v) in &p.props.0 {
                f.str(k);
                f.value(v);
            }
        }
        let asset = n.asset.as_deref();
        let mut words = vec![
            f.0,
            Self::effects_state(ctx, n),
            Self::deps_hash(ctx, i),
            sr_eval::rng::hash_str(&n.id),
            Arc::as_ptr(&n.elem) as u64,
            hf(n.world_opacity),
            asset.filter(|a| ctx.timed.contains(*a)).and(n.source_time).map(hf).unwrap_or(1),
            n.draw as u64,
            n.content.map(|c| h(&[h(&c.dest.map(hf)), h(&c.uv.map(hf))])).unwrap_or(3),
            n.size.map(|s| h(&s.map(hf))).unwrap_or(4),
            n.sim_image.as_ref().map(|s| s.key).unwrap_or(6),
            n.particles
                .as_ref()
                .map(|p| h(&p.pos.iter().flat_map(|q| q.map(|v| v.to_bits() as u64)).collect::<Vec<u64>>()))
                .unwrap_or(7),
            n.soft.as_ref().map(|s| h(&s.offsets.iter().flat_map(|o| o.map(hf)).collect::<Vec<u64>>())).unwrap_or(5),
        ];
        for &c in &ctx.kids[i] {
            words.push(Self::subtree_hash(ctx, c, &local));
        }
        Some(h(&words))
    }

    fn content_hash(ctx: &Ctx, i: usize, to_space: &Affine) -> u64 {
        let n = &ctx.g.nodes[i];
        let mut words = vec![
            sr_eval::rng::hash_str(&n.id),
            Arc::as_ptr(&n.elem) as u64,
            n.size.map(|s| h(&s.map(hf))).unwrap_or(4),
        ];
        for &c in &ctx.kids[i] {
            words.push(Self::subtree_hash(ctx, c, to_space));
        }
        h(&words)
    }

    fn isolated(n: &FrameNode, has_kids: bool, count_effects: bool) -> bool {
        if !has_kids {
            return false;
        }
        let e: &dyn Element = &*n.elem;
        if matches!(n.kind, "instance" | "include") {
            return true;
        }
        let masked = sr_model::element::children(e).iter().any(|c| c.element_name() == "mask");
        let effects = matches!(e.get_attr("effects"), Some(AttrValue::Tokens(t)) if !t.is_empty());
        matches!(e.get_attr("isolate"), Some(AttrValue::Bool(true)))
            || (effects && count_effects)
            || masked
            || n.clip
            || n.matte.is_some()
            || blend_index(e) != 0
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_cmd(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        mut d: Draw,
        local: [f64; 4],
        uv: [f64; 4],
        world: &Affine,
        srctex: Arc<Tex>,
        cmds: &mut Vec<Cmd>,
        hash: u64,
        frame_space: bool,
    ) {
        self.flush_vec(plan, cmds);
        let n = &ctx.g.nodes[i];
        if let Some(Value::Num(ink)) = n.props.get(render_access::CONTRAST_INK) {
            d.flags |= flag::CONTRAST_INK;
            d.color[0] = *ink as f32;
        }
        let three = n.three_d.map(|t| (t, n.anchor));
        // stencil modes act on the whole target: outside the layer the backdrop is cut away
        let stencil = matches!(d.blend, 29 | 30) && three.is_none();
        let (bounds, first_vertex) = if frame_space {
            self.push_frame(plan, space, world)
        } else if stencil {
            self.push_full(plan, space, world, local, uv)
        } else {
            self.push_quad(plan, space, world, three, local, uv, &self.proj25(ctx.g, ctx.p, space))
        };
        let mut matte = None;
        if let Some(mt) = n.matte.filter(|_| !self.bare.contains(&n.id)) {
            matte = Some(self.matte(plan, ctx, mt as usize, space));
            d.flags |= flag::MATTE;
            d.matte_mode = match n.elem.get_attr("matteMode") {
                Some(AttrValue::Str(s)) => m::MatteMode::ALL.iter().position(|v| v.as_str() == s).unwrap_or(0) as u32,
                _ => 0,
            };
        }
        d.target_size = [space.size[0] as f32, space.size[1] as f32];
        if !frame_space {
            d.box_rect = local.map(|v| v as f32);
            d.flags |= flag::EDGE;
        }
        let backdrop = (d.blend >= 2 && !fixed_function_blend(d.blend)).then_some(bounds);
        plan.draws.push(d);
        cmds.push(Cmd {
            draw: (plan.draws.len() - 1) as u32,
            first_vertex,
            count: 6,
            src: srctex,
            backdrop,
            matte,
            hash,
            pre: None,
        });
    }

    fn matte(&mut self, plan: &mut Plan, ctx: &Ctx, mt: usize, space: &Space) -> Arc<Tex> {
        let hash = h(&[
            Self::subtree_hash(ctx, mt, &space.xform),
            hf(ctx.g.nodes[mt].world_opacity),
            space.size[0] as u64,
            space.size[1] as u64,
            0x6d61,
        ]);
        let key = format!("matte:{}:{}x{}", ctx.g.nodes[mt].id, space.size[0], space.size[1]);
        self.used.insert(key.clone());
        if let Some((hh, t)) = self.subtree.get(&key) {
            if *hh == hash {
                plan.stats.cache_hits += 1;
                return t.clone();
            }
        }
        let target = match self.subtree.get(&key) {
            Some((_, t)) if t.size == space.size && Arc::strong_count(t) <= 2 => t.clone(),
            _ => Arc::new(resources::create(&self.gpu.device, &self.bgl1, space.size, 1, "matte")),
        };
        let mut cmds = Vec::new();
        self.emit(plan, ctx, mt, space, 1.0, &mut cmds, true, 0);
        self.flush_vec(plan, &mut cmds);
        plan.jobs.push(Job::draws(target.clone(), true, cmds, false));
        self.subtree.insert(key, (hash, target.clone()));
        target
    }

    /// Emits draw commands for node `i` and its subtree into `cmds`.
    #[allow(clippy::too_many_arguments)]
    fn emit(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        iso_op: f64,
        cmds: &mut Vec<Cmd>,
        force: bool,
        root_hash: u64,
    ) {
        let (g, kids) = (ctx.g, ctx.kids);
        let n = &g.nodes[i];
        if !n.draw && !force {
            return;
        }
        let e: &dyn Element = &*n.elem;
        let op = if iso_op > 0.0 { n.world_opacity / iso_op } else { 0.0 };
        if self.special(plan, ctx, i, space, iso_op, cmds, root_hash) {
            return;
        }
        let bare = self.bare.contains(&n.id);
        let blend = if bare { 0 } else { blend_index(e) };
        let seed = sr_eval::rng::hash_str(&n.id) as u32;
        let has_kids = !kids[i].is_empty();
        if Self::isolated(n, has_kids, !bare) {
            // fully transparent: nothing to draw, and rendering the content now would cache an empty
            // offscreen (children draw at world opacity / container opacity) under a hash that
            // leaves out the container's own opacity, so a later fade-in would never show it
            if n.world_opacity <= 0.0 {
                return;
            }
            let sized = n.size.filter(|s| s[0] >= 1.0 && s[1] >= 1.0 && s[0] <= 8192.0 && s[1] <= 8192.0);
            let (inner, quad_local) = match sized {
                Some([w, hh]) => {
                    let size = [w.ceil() as u32, hh.ceil() as u32];
                    let inv = n.world.inverse().unwrap_or(Affine::IDENTITY);
                    (Space { xform: inv, size }, [0.0, 0.0, size[0] as f64, size[1] as f64])
                }
                // no box: the offscreen shares the target's space and covers it 1:1
                None => (*space, [0.0, 0.0, space.size[0] as f64, space.size[1] as f64]),
            };
            let hash = h(&[Self::content_hash(ctx, i, &inner.xform), inner.size[0] as u64, inner.size[1] as u64]);
            let key = format!("iso:{}:{}x{}", n.id, inner.size[0], inner.size[1]);
            self.used.insert(key.clone());
            let tex = match self.subtree.get(&key) {
                Some((hh, t)) if *hh == hash => {
                    plan.stats.cache_hits += 1;
                    t.clone()
                }
                _ => {
                    let target = match self.subtree.get(&key) {
                        Some((_, t)) if Arc::strong_count(t) <= 2 => t.clone(),
                        _ => Arc::new(resources::create(&self.gpu.device, &self.bgl1, inner.size, 1, "isolated")),
                    };
                    let mut inner_cmds = Vec::new();
                    for c in Self::depth_sorted(g, &kids[i]) {
                        self.emit(plan, ctx, c, &inner, n.world_opacity, &mut inner_cmds, false, 0);
                    }
                    self.flush_vec(plan, &mut inner_cmds);
                    plan.jobs.push(Job::draws(target.clone(), true, inner_cmds, false));
                    self.subtree.insert(key, (hash, target.clone()));
                    target
                }
            };
            let clip = if n.clip { n.size } else { None };
            let mask_box = n.size.unwrap_or([space.size[0] as f64, space.size[1] as f64]);
            let (mask_off, mask_count) = self.masks_of(plan, n, mask_box, clip);
            let d = Draw {
                opacity: op as f32,
                blend,
                src_kind: src::TEXTURE,
                mask_off,
                mask_count,
                seed,
                uv_rect: [0.0, 0.0, 1.0, 1.0],
                ..Default::default()
            };
            let hash_cmd = h(&[root_hash, hash, affine_hash(&space.xform.then(&n.world)), hf(op), 1]);
            self.draw_cmd(
                plan,
                ctx,
                i,
                space,
                d,
                quad_local,
                [0.0, 0.0, 1.0, 1.0],
                &n.world,
                tex,
                cmds,
                hash_cmd,
                sized.is_none(),
            );
            return;
        }
        let probe = self.text_probe_start(plan, ctx, i, space, cmds, root_hash);
        self.dispatch_kind(plan, ctx, i, space, op, blend, seed, cmds, root_hash, iso_op);
        if let Some(at) = probe {
            self.text_probe_end(plan, ctx, i, space, cmds, at);
        }
        for c in Self::depth_sorted(g, &kids[i]) {
            self.emit(plan, ctx, c, space, iso_op, cmds, false, root_hash);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_kind(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        op: f64,
        blend: u32,
        seed: u32,
        cmds: &mut Vec<Cmd>,
        root_hash: u64,
        iso_op: f64,
    ) {
        let g = ctx.g;
        let n = &g.nodes[i];
        match n.kind {
            "layer" => {
                if let Some(content) = n.content {
                    let clock = std::time::Instant::now();
                    if let Some(built) = self.vector_layer_scene(plan, ctx, n, space, content) {
                        match built {
                            Ok((mut scene, bitmaps)) => {
                                let size = n
                                    .size
                                    .unwrap_or([content.dest[2] - content.dest[0], content.dest[3] - content.dest[1]]);
                                let tol = 0.05 / Xf(space.xform.then(&n.world).0).max_scale().max(1e-6);
                                let ds = crate::vector::deformers(n, g, size);
                                crate::vector::deform_scene(&mut scene, &ds, tol);
                                plan.stats.vector_seconds += clock.elapsed().as_secs_f64();
                                for b in bitmaps.iter().filter(|b| b.below) {
                                    self.draw_bitmap(plan, ctx, i, space, op, blend, seed, cmds, root_hash, b);
                                }
                                self.emit_vector(plan, ctx, i, space, op, blend, seed, cmds, root_hash, scene);
                                for (radius, part) in std::mem::take(&mut self.pending_blur) {
                                    self.blurred_part(plan, ctx, i, space, op, cmds, root_hash, radius, part);
                                }
                                // colour bitmap glyphs and map tiles as textured quads
                                for b in bitmaps.iter().filter(|b| !b.below) {
                                    self.draw_bitmap(plan, ctx, i, space, op, blend, seed, cmds, root_hash, b);
                                }
                            }
                            Err(e) => plan.stats.errors.push(format!("{}: {e}", n.id)),
                        }
                    } else if let Some(tex) = self.layer_source(plan, ctx.p, g, n) {
                        let bx = n.size.unwrap_or([content.dest[2], content.dest[3]]);
                        let (mask_off, mask_count) = self.masks_of(plan, n, bx, None);
                        let base_hash = h(&[
                            root_hash,
                            Self::node_hash(ctx, n, &space.xform.then(&n.world)),
                            hf(op),
                            Arc::as_ptr(&tex) as u64,
                        ]);
                        if let Some((bdest, buv)) = content.blur_fill {
                            let reduce = (tex.size[0].max(tex.size[1]) as f64 / bx[0].max(bx[1]).max(1.0)).max(1.0);
                            let lod = (libm::log2(reduce) + 5.0) as f32;
                            let d = Draw {
                                opacity: op as f32,
                                blend,
                                src_kind: src::TEXTURE_LOD,
                                lod,
                                mask_off,
                                mask_count,
                                seed,
                                uv_rect: buv.map(|v| v as f32),
                                ..Default::default()
                            };
                            self.draw_cmd(
                                plan,
                                ctx,
                                i,
                                space,
                                d,
                                bdest,
                                buv,
                                &n.world,
                                tex.clone(),
                                cmds,
                                h(&[base_hash, 7]),
                                false,
                            );
                        }
                        let d = Draw {
                            opacity: op as f32,
                            blend,
                            src_kind: src::TEXTURE,
                            mask_off,
                            mask_count,
                            seed,
                            uv_rect: content.uv.map(|v| v as f32),
                            ..Default::default()
                        };
                        self.draw_cmd(
                            plan,
                            ctx,
                            i,
                            space,
                            d,
                            content.dest,
                            content.uv,
                            &n.world,
                            tex,
                            cmds,
                            base_hash,
                            false,
                        );
                        let bx = n.size.unwrap_or([content.dest[2], content.dest[3]]);
                        let ds = crate::vector::deformers(n, g, bx);
                        if !ds.is_empty() && n.three_d.is_none() {
                            Self::meshify(plan, cmds, space, &n.world, content.dest, content.uv, &ds);
                        }
                    }
                }
            }
            "shape" => {
                let clock = std::time::Instant::now();
                let scale = Xf(space.xform.then(&n.world).0).max_scale().max(1e-6);
                let tol = 0.05 / scale;
                let built = {
                    let (p, g) = (ctx.p, ctx.g);
                    let mut pf = |v: &Value, b: [f64; 4]| self.vector_paint(plan, p, g, v, b, &n.id);
                    crate::vector::shape_scene(n, &mut pf, tol)
                };
                match built {
                    Ok(mut scene) => {
                        let size = n.size.unwrap_or([0.0, 0.0]);
                        let ds = crate::vector::deformers(n, g, size);
                        crate::vector::deform_scene(&mut scene, &ds, tol);
                        plan.stats.vector_seconds += clock.elapsed().as_secs_f64();
                        self.emit_vector(plan, ctx, i, space, op, blend, seed, cmds, root_hash, scene);
                        self.pattern_fills(plan, ctx, i, space, op, blend, seed, cmds, root_hash);
                    }
                    Err(e) => plan.stats.errors.push(format!("{}: {e}", n.id)),
                }
            }
            "particleEmitter" | "flock" => self.emit_particles(plan, ctx, i, space, op, cmds, root_hash),
            "fluid" | "slime" | "erosion" => self.emit_sim_image(plan, ctx, i, space, op, blend, seed, cmds, root_hash),
            "object3D" => self.three_run(plan, ctx, i, space, iso_op, cmds, root_hash),
            "adjustment" => self.adjust(plan, ctx, i, space, op, cmds, root_hash),
            _ => {}
        }
        let _ = iso_op;
    }

    /// Resolves a paint value for vector content over `box_rect` (x, y, w, h, node-local).
    fn vector_paint(
        &self,
        plan: &mut Plan,
        p: &Program,
        g: &FrameGraph,
        v: &Value,
        box_rect: [f64; 4],
        who: &str,
    ) -> Option<sr_vector::Paint> {
        use sr_vector::Paint;
        match v {
            Value::Color(c) => (c[3] > 0.0).then_some(Paint::Solid { rgba: *c, srgb: false }),
            Value::Str(s) if s.starts_with("token:") => {
                self.tokens.get(&s[6..]).map(|c| Paint::Solid { rgba: *c, srgb: false })
            }
            Value::PaintRef(id) => {
                let pc = p.scene.paints.as_ref().and_then(|ps| ps.children.iter().find(|c| c.id() == Some(id)))?;
                if matches!(pc, m::PaintsChild::Pattern(_)) {
                    plan.stats.unsupported.push(format!("{who}: pattern paint on vector content"));
                    return None;
                }
                let tokens = &self.tokens;
                let tok = |t: &str| tokens.get(t).copied();
                plan.paints
                    .gradient(&self.working, pc, g, box_rect, &tok)
                    .map(|index| Paint::External { index, to_local: Xf::IDENTITY })
            }
            _ => None,
        }
    }

    /// Scene of a layer whose asset is vector content (vector, SVG, Lottie), in node-local space.
    fn vector_layer_scene(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        n: &FrameNode,
        space: &Space,
        content: sr_eval::layout::Content,
    ) -> Option<Result<(Scene, Vec<sr_text::Bitmap>), String>> {
        let key = n.asset.as_deref()?;
        let (a, doc) = self.asset(ctx.p, key)?;
        let base = ctx.p.base_dirs.get(doc).cloned().unwrap_or_default();
        let (aw, ah) = match a {
            AssetsChild::Vector(v) => (v.width as f64, v.height as f64),
            AssetsChild::Lottie(l) => (l.width as f64, l.height as f64),
            AssetsChild::Text(x) => (x.width as f64, x.height as f64),
            AssetsChild::Chart(x) => (x.width as f64, x.height as f64),
            AssetsChild::Map(x) => (x.width as f64, x.height as f64),
            AssetsChild::Audiogram(x) => (x.width as f64, x.height as f64),
            AssetsChild::Code(x) => (x.width as f64, x.height as f64),
            AssetsChild::Formula(x) => (x.width as f64, x.height as f64),
            _ => return None,
        };
        let [x0, y0, x1, y1] = content.dest;
        let [u0, v0, u1, v1] = content.uv;
        let (du, dv) = ((u1 - u0).abs().max(1e-9) * (u1 - u0).signum(), (v1 - v0).abs().max(1e-9) * (v1 - v0).signum());
        let (sx, sy) = ((x1 - x0) / (du * aw), (y1 - y0) / (dv * ah));
        let to_local = Xf([sx, 0.0, 0.0, sy, x0 - u0 * aw * sx, y0 - v0 * ah * sy]);
        let scale = Xf(space.xform.then(&n.world).0).max_scale().max(1e-6) * to_local.max_scale().max(1e-9);
        let tol = 0.05 / scale;
        let read = |src: &str| -> Result<Vec<u8>, String> {
            match sr_model::assets::resolve(src, &base) {
                sr_model::assets::Resolved::Local(path) => {
                    std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
                }
                _ => Err(format!("{src}: only local files are supported")),
            }
        };
        let mut bitmaps = Vec::new();
        let scene = match a {
            AssetsChild::Text(_)
            | AssetsChild::Chart(_)
            | AssetsChild::Map(_)
            | AssetsChild::Audiogram(_)
            | AssetsChild::Code(_)
            | AssetsChild::Formula(_) => {
                let mut tc = std::mem::take(&mut self.text);
                let mut unsupported = Vec::new();
                let (p, g) = (ctx.p, ctx.g);
                let tokens = self.tokens.clone();
                let r = {
                    let mut pf = |val: &Value, b: [f64; 4]| self.vector_paint(plan, p, g, val, b, &n.id);
                    let mut cx = crate::text::Cx {
                        p,
                        g,
                        n,
                        base: base.clone(),
                        tol: tol / to_local.max_scale().max(1e-9),
                        paint: &mut pf,
                        tokens: &tokens,
                        unsupported: &mut unsupported,
                    };
                    crate::text::asset_drawing(&mut tc, &mut cx, key, a)
                };
                self.text = tc;
                for u in unsupported {
                    if !plan.stats.unsupported.contains(&u) {
                        plan.stats.unsupported.push(u);
                    }
                }
                match r? {
                    Ok(d) => {
                        bitmaps = d
                            .bitmaps
                            .iter()
                            .map(|b| sr_text::Bitmap { xf: to_local.mul(&b.xf), ..b.clone() })
                            .collect();
                        self.pending_blur = d
                            .blurred
                            .iter()
                            .map(|(r, sc)| (*r * to_local.max_scale(), sc.transformed(&to_local)))
                            .collect();
                        d.scene
                    }
                    Err(e) => return Some(Err(e)),
                }
            }
            AssetsChild::Vector(v) => {
                if v.shape.as_str() == "svg" {
                    let Some(src) = v.src.clone() else { return Some(Err("shape=\"svg\" needs @src".into())) };
                    let bucket = libm::floor(libm::log2(tol) * 2.0) as i32;
                    let ck = (key.to_string(), bucket);
                    if !self.svgs.contains_key(&ck) {
                        let tol_b = libm::exp2(bucket as f64 / 2.0);
                        let r = read(&src).and_then(|b| sr_vector::svg::load(&b, tol_b)).map(Arc::new);
                        self.svgs.retain(|k, _| k.0 != key);
                        self.svgs.insert(ck.clone(), r);
                    }
                    match &self.svgs[&ck] {
                        Ok(svg) => {
                            for sk in &svg.skipped {
                                plan.stats.unsupported.push(format!("{}: SVG {sk}", n.id));
                            }
                            let fit = Xf::scale(aw / svg.size[0].max(1e-9), ah / svg.size[1].max(1e-9));
                            svg.scene.transformed(&fit)
                        }
                        Err(e) => return Some(Err(e.clone())),
                    }
                } else {
                    let el = ctx.g.elements.iter().find(|e| *e.key == *v.id).map(|e| &e.props);
                    let attrs = crate::vector::Attrs { e: v, props: el };
                    let (p, g) = (ctx.p, ctx.g);
                    let mut pf = |val: &Value, b: [f64; 4]| self.vector_paint(plan, p, g, val, b, &n.id);
                    match crate::vector::vector_asset_scene(&attrs, &mut pf, tol / to_local.max_scale().max(1e-9)) {
                        Ok(s) => s,
                        Err(e) => return Some(Err(e)),
                    }
                }
            }
            AssetsChild::Lottie(l) => {
                if !self.lotties.contains_key(key) {
                    let slots: Vec<(String, String)> =
                        l.slots.iter().map(|s| (s.id.clone(), s.value.clone())).collect();
                    let r = read(&l.src)
                        .and_then(|b| sr_vector::lottie::Lottie::parse(&b, l.animation.as_deref(), &slots))
                        .map(Arc::new);
                    self.lotties.insert(key.to_string(), r);
                }
                match &self.lotties[key] {
                    Ok(lot) => {
                        for sk in &lot.skipped {
                            plan.stats.unsupported.push(format!("{}: Lottie {sk}", n.id));
                        }
                        let seg = l.segment.as_deref().and_then(|s| lot.segment(s));
                        let t = n.source_time.unwrap_or(n.local_time);
                        let fit = Xf::scale(aw / lot.size[0].max(1e-9), ah / lot.size[1].max(1e-9));
                        lot.render(lot.frame_at(t, seg), tol / (to_local.max_scale() * fit.max_scale()).max(1e-9))
                            .transformed(&fit)
                    }
                    Err(e) => return Some(Err(e.clone())),
                }
            }
            _ => return None,
        };
        let mut out = scene.transformed(&to_local);
        // cover and crop placements clip to the destination
        let full = u0.min(u1) <= 1e-9 && v0.min(v1) <= 1e-9 && u0.max(u1) >= 1.0 - 1e-9 && v0.max(v1) >= 1.0 - 1e-9;
        if !full {
            let clip = sr_vector::shapes::rect(x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs(), [0.0; 4])
                .flatten(tol);
            let mut wrapped = Scene::default();
            wrapped.cmds.push(sr_vector::Cmd::Push { mask_init: 0.0 });
            wrapped.extend(out);
            wrapped.cmds.push(sr_vector::Cmd::Mask {
                polys: clip,
                rule: sr_vector::FillRule::NonZero,
                op: sr_vector::MaskOp::Add,
                opacity: 1.0,
                invert: false,
            });
            wrapped.cmds.push(sr_vector::Cmd::Pop { opacity: 1.0 });
            out = wrapped;
        }
        Some(Ok((out, bitmaps)))
    }

    /// Texture of a bitmap glyph (decoded once).
    /// Draws a drawing's bitmap (a colour glyph or a map tile) as a textured quad in node space.
    #[allow(clippy::too_many_arguments)]
    fn draw_bitmap(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        op: f64,
        blend: u32,
        seed: u32,
        cmds: &mut Vec<Cmd>,
        root_hash: u64,
        b: &sr_text::Bitmap,
    ) {
        let Some(tex) = self.glyph_texture(b) else { return };
        let n = &ctx.g.nodes[i];
        let world = n.world.then(&Affine(b.xf.0));
        let d = Draw {
            opacity: (op * b.opacity) as f32,
            blend,
            src_kind: src::TEXTURE,
            seed,
            uv_rect: b.uv.map(|v| v as f32),
            ..Default::default()
        };
        let [x, y, w, hh] = b.rect;
        let hash = h(&[
            root_hash,
            b.key,
            affine_hash(&space.xform.then(&world)),
            hf(op * b.opacity),
            hf(b.uv[0]),
            hf(b.uv[1]),
            hf(b.uv[2]),
            hf(b.uv[3]),
        ]);
        self.draw_cmd(plan, ctx, i, space, d, [x, y, x + w, y + hh], b.uv, &world, tex, cmds, hash, false);
    }

    fn glyph_texture(&mut self, b: &sr_text::Bitmap) -> Option<Arc<Tex>> {
        if let Some(t) = self.glyph_tex.get(&b.key) {
            return t.clone();
        }
        let t = resources::decode_image_bytes(&b.png, &self.working, self.max_texture)
            .ok()
            .map(|d| Arc::new(resources::upload(&self.gpu.device, &self.gpu.queue, &self.bgl1, &d, "glyph")));
        self.glyph_tex.insert(b.key, t.clone());
        t
    }

    /// Draws a node's local-space vector scene: batched into the target when
    /// simple, otherwise rasterised at on-screen scale and composited.
    #[allow(clippy::too_many_arguments)]
    fn emit_vector(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        i: usize,
        space: &Space,
        op: f64,
        blend: u32,
        seed: u32,
        cmds: &mut Vec<Cmd>,
        root_hash: u64,
        scene: Scene,
    ) {
        let n = &ctx.g.nodes[i];
        let e: &dyn Element = &*n.elem;
        let masked = sr_model::element::children(e).iter().any(|c| c.element_name() == "mask");
        let ta = space.xform.then(&n.world);
        let content = h(&[
            Self::node_hash(ctx, n, &ta),
            hf(op),
            hf(n.local_time),
            hf(n.source_time.unwrap_or(0.0)),
            ctx.elements,
        ]);
        let simple = blend == 0
            && n.matte.is_none()
            && !n.is_matte
            && n.three_d.is_none()
            && !masked
            && !n.clip
            && n.props.get(render_access::CONTRAST_INK).is_none();
        if simple {
            let mut s = scene.transformed(&Xf(ta.0));
            if op < 1.0 {
                let mut w = Scene::default();
                w.cmds.push(sr_vector::Cmd::Push { mask_init: 1.0 });
                w.extend(s);
                w.cmds.push(sr_vector::Cmd::Pop { opacity: op });
                s = w;
            }
            let key = cmds as *const Vec<Cmd> as usize;
            match plan.vbatches.iter_mut().find(|b| b.key == key) {
                Some(b) => {
                    b.scene.extend(s);
                    b.hash = h(&[b.hash, content]);
                }
                None => plan.vbatches.push(VBatch {
                    key,
                    space: *space,
                    scene: s,
                    hash: h(&[content, space.size[0] as u64, space.size[1] as u64]),
                    first: n.id.to_string(),
                }),
            }
            return;
        }
        let b = scene.bounds();
        if b.is_empty() {
            return;
        }
        let max = self.max_texture.max(64) as f64;
        let mut s = Xf(ta.0).max_scale().max(1e-6);
        let span = (b.0[2] - b.0[0]).max(b.0[3] - b.0[1]).max(1e-9);
        if span * s + 2.0 > max {
            s = (max - 2.0) / span;
        }
        let (x0, y0) = (b.0[0] - 1.0 / s, b.0[1] - 1.0 / s);
        let tw = (((b.0[2] - b.0[0]) * s).ceil() as u32 + 2).max(1);
        let th = (((b.0[3] - b.0[1]) * s).ceil() as u32 + 2).max(1);
        let to_tex = Xf::scale(s, s).mul(&Xf::translate(-x0, -y0));
        let local = [x0, y0, x0 + tw as f64 / s, y0 + th as f64 / s];
        let key = format!("vec:{}:{tw}x{th}", n.id);
        self.used.insert(key.clone());
        let hash = h(&[content, tw as u64, th as u64, hf(s)]);
        let tex = match self.subtree.get(&key) {
            Some((hh, t)) if *hh == hash => {
                plan.stats.cache_hits += 1;
                t.clone()
            }
            _ => {
                let target = match self.subtree.get(&key) {
                    Some((_, t)) if Arc::strong_count(t) <= 2 => t.clone(),
                    _ => Arc::new(resources::create(&self.gpu.device, &self.bgl1, [tw, th], 1, "vector")),
                };
                let clock = std::time::Instant::now();
                let enc = sr_vector::tile::encode_cached(&scene.transformed(&to_tex), [tw, th], &mut self.tiles);
                if enc.flattened_layers > 0 {
                    plan.stats.unsupported.push(format!(
                        "{}: vector layers nested deeper than {} were flattened",
                        n.id,
                        sr_vector::tile::MAX_DEPTH
                    ));
                }
                plan.rasters.push(RasterJob::new(enc, target.clone(), &mut plan.paints, &self.working));
                plan.stats.vector_seconds += clock.elapsed().as_secs_f64();
                self.subtree.insert(key, (hash, target.clone()));
                target
            }
        };
        let bx = n.size.unwrap_or([local[2] - local[0], local[3] - local[1]]);
        let (mask_off, mask_count) = self.masks_of(plan, n, bx, if n.clip { n.size } else { None });
        let d = Draw {
            opacity: op as f32,
            blend,
            src_kind: src::TEXTURE,
            mask_off,
            mask_count,
            seed,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            ..Default::default()
        };
        self.draw_cmd(
            plan,
            ctx,
            i,
            space,
            d,
            local,
            [0.0, 0.0, 1.0, 1.0],
            &n.world,
            tex,
            cmds,
            h(&[root_hash, hash]),
            false,
        );
    }

    /// Burned-in captions over everything, in frame space.
    fn burn_captions(&mut self, plan: &mut Plan, ctx: &Ctx, space: &Space, cmds: &mut Vec<Cmd>) {
        if ctx.p.scene.captions.is_none() || self.captions_off {
            return;
        }
        let ink = ctx.g.elements.iter().find(|e| &*e.key == render_access::CONTRAST_INK).and_then(|e| {
            match e.props.get(render_access::CONTRAST_INK) {
                Some(Value::Num(v)) => Some(*v),
                _ => None,
            }
        });
        if ink.is_some() {
            self.flush_vec(plan, cmds);
        }
        let at_ink = cmds.len();
        let probe = self.contrast_probe.then(|| {
            self.flush_vec(plan, cmds);
            cmds.len()
        });
        let bounds = self.burn_captions_inner(plan, ctx, space, cmds);
        if let Some(ink) = ink {
            self.flush_vec(plan, cmds);
            for cmd in &mut cmds[at_ink..] {
                let d = &mut plan.draws[cmd.draw as usize];
                d.flags |= flag::CONTRAST_INK;
                d.color[0] = ink as f32;
                cmd.hash = h(&[cmd.hash, ink.to_bits(), 0x1ac]);
            }
        }
        if let (Some(at), Some(b)) = (probe, bounds) {
            self.flush_vec(plan, cmds);
            self.attach_probe(plan, space, cmds, at, "captions", b);
        }
    }

    fn burn_captions_inner(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        space: &Space,
        cmds: &mut Vec<Cmd>,
    ) -> Option<[f64; 4]> {
        let clock = std::time::Instant::now();
        let mut tc = std::mem::take(&mut self.text);
        let mut errors = Vec::new();
        let only = self.burn_captions.clone();
        let tokens = self.tokens.clone();
        let built = {
            let (p, g) = (ctx.p, ctx.g);
            let mut pf = |v: &Value, b: [f64; 4]| self.vector_paint(plan, p, g, v, b, "captions");
            crate::text::caption_scene(&mut tc, p, g, &mut pf, &tokens, only.as_deref(), &mut errors)
        };
        self.text = tc;
        for e in errors {
            if !plan.stats.errors.contains(&e) {
                plan.stats.errors.push(e);
            }
        }
        let (scene, hsh) = built?;
        let b = scene.bounds().0;
        plan.stats.vector_seconds += clock.elapsed().as_secs_f64();
        let key = cmds as *const Vec<Cmd> as usize;
        match plan.vbatches.iter_mut().find(|b| b.key == key) {
            Some(b) => {
                b.scene.extend(scene);
                b.hash = h(&[b.hash, hsh]);
            }
            None => plan.vbatches.push(VBatch {
                key,
                space: *space,
                scene,
                hash: h(&[hsh, space.size[0] as u64, space.size[1] as u64]),
                first: "captions".into(),
            }),
        }
        Some(b)
    }

    /// Rasterises the pending simple-vector batch of `cmds` and draws it.
    fn flush_vec(&mut self, plan: &mut Plan, cmds: &mut Vec<Cmd>) {
        let key = cmds as *const Vec<Cmd> as usize;
        let Some(pos) = plan.vbatches.iter().position(|b| b.key == key) else { return };
        let b = plan.vbatches.remove(pos);
        let size = b.space.size;
        let ck = format!("vbatch:{}:{}x{}", b.first, size[0], size[1]);
        self.used.insert(ck.clone());
        let tex = match self.subtree.get(&ck) {
            Some((hh, t)) if *hh == b.hash => {
                plan.stats.cache_hits += 1;
                t.clone()
            }
            _ => {
                let target = match self.subtree.get(&ck) {
                    Some((_, t)) if Arc::strong_count(t) <= 2 => t.clone(),
                    _ => Arc::new(resources::create(&self.gpu.device, &self.bgl1, size, 1, "vector batch")),
                };
                let clock = std::time::Instant::now();
                let enc = sr_vector::tile::encode_cached(&b.scene, size, &mut self.tiles);
                if enc.flattened_layers > 0 {
                    plan.stats.unsupported.push(format!(
                        "{}: vector layers nested deeper than {} were flattened",
                        b.first,
                        sr_vector::tile::MAX_DEPTH
                    ));
                }
                plan.rasters.push(RasterJob::new(enc, target.clone(), &mut plan.paints, &self.working));
                plan.stats.vector_seconds += clock.elapsed().as_secs_f64();
                self.subtree.insert(ck, (b.hash, target.clone()));
                target
            }
        };
        let (_, first_vertex) = self.push_frame(plan, &b.space, &Affine::IDENTITY);
        plan.draws.push(Draw {
            opacity: 1.0,
            blend: 0,
            src_kind: src::TEXTURE,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            target_size: [size[0] as f32, size[1] as f32],
            ..Default::default()
        });
        cmds.push(Cmd {
            draw: (plan.draws.len() - 1) as u32,
            first_vertex,
            count: 6,
            src: tex,
            backdrop: None,
            matte: None,
            hash: b.hash,
            pre: None,
        });
    }

    /// Replaces the last command's quad with a deformed grid mesh.
    fn meshify(
        plan: &mut Plan,
        cmds: &mut [Cmd],
        space: &Space,
        world: &Affine,
        local: [f64; 4],
        uv: [f64; 4],
        ds: &[sr_vector::deform::Deformer],
    ) {
        let Some(c) = cmds.last_mut() else { return };
        let (pos, uvs, idx) =
            sr_vector::deform::grid([local[0], local[1], local[2] - local[0], local[3] - local[1]], 32, 32);
        let m = space.xform.then(world);
        let first = plan.verts.len() as u32;
        let moved: Vec<[f64; 2]> = pos
            .iter()
            .map(|&q| {
                let d = sr_vector::deform::apply_all(ds, q);
                m.apply([d.x, d.y])
            })
            .collect();
        let mut bb = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for &k in &idx {
            let k = k as usize;
            let t = moved[k];
            bb = [bb[0].min(t[0]), bb[1].min(t[1]), bb[2].max(t[0]), bb[3].max(t[1])];
            let u = uvs[k];
            plan.verts.push(Vertex {
                clip: to_clip(t, space.size),
                uv: [(uv[0] + u.x * (uv[2] - uv[0])) as f32, (uv[1] + u.y * (uv[3] - uv[1])) as f32],
                local: [pos[k].x as f32, pos[k].y as f32],
            });
        }
        c.first_vertex = first;
        c.count = idx.len() as u32;
        if c.backdrop.is_some() {
            let (w, hh) = (space.size[0] as f64, space.size[1] as f64);
            let x0 = bb[0].floor().clamp(0.0, w) as u32;
            let y0 = bb[1].floor().clamp(0.0, hh) as u32;
            let x1 = bb[2].ceil().clamp(0.0, w) as u32;
            let y1 = bb[3].ceil().clamp(0.0, hh) as u32;
            c.backdrop = Some([x0, y0, x1, y1]);
        }
    }

    fn depth_sorted(g: &FrameGraph, kids: &[usize]) -> Vec<usize> {
        let mut out = kids.to_vec();
        let mut i = 0;
        while i < out.len() {
            if g.nodes[out[i]].three_d.is_none() {
                i += 1;
                continue;
            }
            let mut j = i;
            while j < out.len() && g.nodes[out[j]].three_d.is_some() {
                j += 1;
            }
            out[i..j].sort_by(|a, b| {
                let za = g.nodes[*a].three_d.unwrap()[0];
                let zb = g.nodes[*b].three_d.unwrap()[0];
                zb.partial_cmp(&za).unwrap_or(std::cmp::Ordering::Equal)
            });
            i = j;
        }
        out
    }

    // ---------------------------------------------------------- frame

    /// The project background. The composition composites on transparency and the background goes
    /// beneath everything last (as After Effects does), so `behind`, `subtract`, stencils and
    /// silhouettes act on the layers only.
    fn background(&mut self, plan: &mut Plan, ctx: &Ctx, space: &Space, cmds: &mut Vec<Cmd>) {
        let g = ctx.g;
        // the document's frame, in document units (the space may draw it at another scale)
        let (w, hh) = (g.size[0], g.size[1]);
        let tokens = self.tokens.clone();
        let tok = |t: &str| tokens.get(t).copied();
        let (kind, paint, tex) = match &g.background {
            Value::Color(c) => (src::PAINT, plan.paints.solid(&self.working, *c), self.dummy.clone()),
            Value::PaintRef(id) => {
                match ctx.p.scene.paints.as_ref().and_then(|ps| ps.children.iter().find(|c| c.id() == Some(id))) {
                    Some(m::PaintsChild::Pattern(pt)) => {
                        let asset = ctx
                            .p
                            .scene
                            .assets
                            .as_ref()
                            .and_then(|a| a.children.iter().find(|c| c.id() == Some(pt.asset.as_str())));
                        match asset {
                            Some(AssetsChild::Image(img)) => {
                                let path = match sr_model::assets::resolve(&img.src, &ctx.p.base_dirs[0]) {
                                    sr_model::assets::Resolved::Local(p) => Some(p),
                                    _ => None,
                                };
                                match path.and_then(|p| {
                                    self.image(
                                        p,
                                        img.color_space,
                                        img.transfer,
                                        img.alpha,
                                        img.color_profile == sr_model::model::ColorProfile::Embedded,
                                    )
                                }) {
                                    Some(t) => (
                                        src::PATTERN,
                                        plan.paints.pattern(pt, g, [img.width as f64, img.height as f64]),
                                        t,
                                    ),
                                    None => {
                                        (src::PAINT, plan.paints.solid(&self.working, [0.0; 4]), self.dummy.clone())
                                    }
                                }
                            }
                            _ => (src::PAINT, plan.paints.solid(&self.working, [0.0; 4]), self.dummy.clone()),
                        }
                    }
                    Some(pc) => match plan.paints.gradient(&self.working, pc, g, [0.0, 0.0, w, hh], &tok) {
                        Some(i) => (src::PAINT, i, self.dummy.clone()),
                        None => (src::PAINT, plan.paints.solid(&self.working, [0.0; 4]), self.dummy.clone()),
                    },
                    None => (src::PAINT, plan.paints.solid(&self.working, [0.0; 4]), self.dummy.clone()),
                }
            }
            _ => (src::PAINT, plan.paints.solid(&self.working, [0.0, 0.0, 0.0, 1.0]), self.dummy.clone()),
        };
        let (_, first_vertex) = self.push_quad(
            plan,
            space,
            &Affine::IDENTITY,
            None,
            [0.0, 0.0, w, hh],
            [0.0, 0.0, 1.0, 1.0],
            &glam::Mat4::IDENTITY,
        );
        plan.draws.push(Draw {
            opacity: 1.0,
            src_kind: kind,
            paint,
            target_size: [space.size[0] as f32, space.size[1] as f32],
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            blend: BLEND_UNDER,
            ..Default::default()
        });
        let hash = {
            let mut f = Fnv(0xcbf2_9ce4_8422_2325);
            f.value(&g.background);
            h(&[f.0, hf(g.size[0]), hf(g.size[1]), elements_hash(&g.elements)])
        };
        cmds.push(Cmd {
            draw: (plan.draws.len() - 1) as u32,
            first_vertex,
            count: 6,
            src: tex,
            backdrop: None,
            matte: None,
            hash,
            pre: None,
        });
    }

    /// Renders one FrameGraph into the frame texture.
    pub fn render(&mut self, g: &FrameGraph, p: &Program) -> Frame {
        self.render_with(g, p, None)
    }

    /// Renders a frame; `provider` evaluates the scene at other times for
    /// motion blur and time effects (without it they draw unblurred and are reported).
    ///
    /// Scenes whose shaders keep persistent ISF buffers replay the frames before `g.frame`
    /// (from the latest checkpoint) when it does not follow the previous render, so their
    /// feedback is the same whichever frame is rendered first.
    /// Offscreen textures held for reuse between frames.
    pub fn pooled_textures(&self) -> usize {
        self.pool.held()
    }

    pub fn render_with(
        &mut self,
        g: &FrameGraph,
        p: &Program,
        mut provider: Option<&mut dyn FnMut(f64) -> FrameGraph>,
    ) -> Frame {
        self.tiles.next_frame();
        let persistent =
            *self.persistent_isf.get_or_insert_with(|| crate::shader::has_persistent(p, &Self::base_dir(p)));
        let fps = {
            let f = &p.scene.project.fps;
            (f.num as f64 / f.den.max(1) as f64).max(1e-6)
        };
        let interval = fps.round().max(1.0) as i64;
        if persistent && self.last_frame != Some(g.frame) && self.last_frame != Some(g.frame - 1) {
            if let Some(pv) = provider.as_mut() {
                let start = self.fx.restore(g.frame - 1);
                for k in start..g.frame {
                    let gk = pv(k as f64 / fps);
                    let _ = self.render_graph(&gk, p, Some(&mut **pv));
                    self.fx.checkpoint(k, interval);
                }
            }
        }
        let out = self.render_graph(g, p, provider);
        self.last_frame = Some(g.frame);
        if persistent {
            self.fx.checkpoint(g.frame, interval);
        }
        out
    }

    fn render_graph(
        &mut self,
        g: &FrameGraph,
        p: &Program,
        provider: Option<&mut dyn FnMut(f64) -> FrameGraph>,
    ) -> Frame {
        if self.view_override.is_none() && p.scene.scene360.is_some() {
            return self.render_360(g, p, provider);
        }
        self.tier = Tier::of(self.quality.unwrap_or(p.scene.project.quality));
        let created_before = self.pool.created;
        let released_before = self.pool.released;
        let subs = provider.map(|pv| SubFrames {
            provider: std::cell::RefCell::new(pv),
            cache: Default::default(),
            seconds: Default::default(),
        });
        let scale = self.tier.scale;
        let size = [(g.size[0] * scale).round().max(1.0) as u32, (g.size[1] * scale).round().max(1.0) as u32];
        let mut plan = Plan::default();
        self.used.clear();
        let mut kids: Vec<Vec<usize>> = vec![Vec::new(); g.nodes.len()];
        let mut roots = Vec::new();
        for (i, n) in g.nodes.iter().enumerate() {
            match n.parent {
                Some(pi) => kids[pi as usize].push(i),
                None => roots.push(i),
            }
        }
        let frame = match &self.frame {
            Some(f) if f.size == size => f.clone(),
            _ => Arc::new(resources::create(&self.gpu.device, &self.bgl1, size, 1, "frame")),
        };
        self.frame = Some(frame.clone());
        let mut timed = std::collections::HashSet::new();
        let mut generated = std::collections::HashSet::new();
        for key in p.assets.keys() {
            match self.asset(p, key).map(|a| a.0) {
                Some(AssetsChild::Generator(_)) => {
                    generated.insert(key.to_string());
                }
                Some(AssetsChild::Image(_)) | None => {}
                Some(_) => {
                    timed.insert(key.to_string());
                }
            }
        }
        // the camera (with any 360 face override) changes every projected draw
        let cam_hash = {
            let (cv, _, _) = self.camera3(g, p, [g.size[0] as f32, g.size[1] as f32]);
            h(&cv.view_proj().to_cols_array().map(|v| v.to_bits() as u64))
        };
        let elements = h(&[elements_hash(&g.elements), cam_hash]);
        let el = element_hashes(&g.elements);
        let ctx = Ctx {
            g,
            p,
            kids: &kids,
            timed: &timed,
            generated: &generated,
            elements,
            el: &el,
            cam: cam_hash,
            sub: subs.as_ref().map(|s| s as &dyn SubSource),
        };
        for m in &g.problems {
            if !plan.stats.unsupported.contains(m) {
                plan.stats.unsupported.push(m.clone());
            }
        }
        // the document's coordinates at the tier's scale
        let space = Space { xform: Affine::scale(scale, scale), size };
        let mut cmds = Vec::new();
        for r in Self::depth_sorted(g, &roots) {
            let rh = h(&[Self::subtree_hash(&ctx, r, &Affine::IDENTITY), hf(g.nodes[r].world_opacity), cam_hash]);
            self.emit(&mut plan, &ctx, r, &space, 1.0, &mut cmds, false, rh);
        }
        self.burn_captions(&mut plan, &ctx, &space, &mut cmds);
        self.flush_vec(&mut plan, &mut cmds);
        self.background(&mut plan, &ctx, &space, &mut cmds);
        // contrast probes measure against what ends up behind the text: the background too
        let probe_bg = if plan.probes.is_empty() {
            None
        } else {
            let bg = self.temp(&mut plan, size);
            let mut bcmds = Vec::new();
            self.background(&mut plan, &ctx, &space, &mut bcmds);
            plan.jobs.push(Job::draws(bg.clone(), true, bcmds, false));
            Some(bg)
        };
        self.finishing(&mut plan, p, &frame);
        if let Some(s) = &subs {
            plan.stats.subframes = s.count();
            plan.stats.subframe_seconds = s.seconds.get();
        }
        // root prefix reuse
        let hashes: Vec<u64> = cmds.iter().enumerate().map(|(k, c)| h(&[c.hash, k as u64])).collect();
        if self.contrast_probe {
            // probed text needs its backdrop copied, so nothing is restored from the prefix snapshot
            self.prefix = None;
        }
        let restore = match &self.prefix {
            Some(pf)
                if pf.tex.size == size
                    && hashes.len() >= pf.hashes.len()
                    && hashes[..pf.hashes.len()] == pf.hashes[..] =>
            {
                pf.hashes.len()
            }
            _ => 0,
        };
        let stable = hashes.iter().zip(&self.prev_root).take_while(|(a, b)| a == b).count();
        let snapshot_at = (stable > restore).then_some(stable);
        if stable == 0 {
            self.prefix = None;
        }
        self.prev_root = hashes.clone();
        plan.jobs.push(Job::draws(frame.clone(), true, cmds, true));
        plan.stats.unsupported.sort();
        plan.stats.unsupported.dedup();
        let probes = std::mem::take(&mut plan.probes);
        let mut stats = self.execute(plan, restore, snapshot_at, &hashes);
        for (id, rect, snap) in probes {
            if let Some(ratio) = self.measure_contrast(&snap, probe_bg.as_deref(), &frame, rect) {
                stats.contrast.push((id, ratio));
            } else {
                // Equal colours do not establish that the text is absent. The delivery
                // fallback measures rendered coverage with contrasting probe inks.
                stats.contrast_unprobed.push(id);
            }
            self.pool.put(snap);
        }
        // evict cache entries unused for CACHE_KEEP frames (a posterized group leaves its children's
        // entries unused between steps); evicted offscreens go back to the pool rather than being
        // destroyed now, while the GPU may still be reading them
        let now = self.cache_frame;
        self.cache_frame += 1;
        for k in self.used.drain() {
            self.last_used.insert(k, now);
        }
        self.last_used.retain(|_, at| now - *at < CACHE_KEEP);
        let live = &self.last_used;
        let mut evicted = Vec::new();
        self.subtree.retain(|k, (_, t)| {
            let keep = live.contains_key(k);
            if !keep {
                evicted.push(t.clone());
            }
            keep
        });
        self.generators.retain(|k, t| {
            let keep = live.contains_key(&format!("gen:{k}"));
            if !keep {
                evicted.push(t.clone());
            }
            keep
        });
        self.video_frames.retain(|k, _| live.contains_key(&format!("video:{k}")));
        for t in evicted {
            self.pool.put(t);
        }
        self.pool.trim();
        stats.textures_created = self.pool.created - created_before;
        stats.textures_released = self.pool.released - released_before;
        Frame { texture: frame, stats }
    }

    /// Writes `data` into a grow-only buffer, reallocating (at least doubling) when it does not
    /// fit. Returns whether it was reallocated, in which case bind groups over it must be rebuilt.
    /// The write is queued, so it is ordered after the frames already submitted.
    fn upload<T: bytemuck::Pod + Default>(&self, slot: &mut Option<GrowBuf>, data: &[T], label: &str) -> bool {
        let one = [T::default()];
        let bytes: &[u8] = if data.is_empty() { bytemuck::cast_slice(&one) } else { bytemuck::cast_slice(data) };
        let need = bytes.len() as u64;
        let realloc = slot.as_ref().is_none_or(|b| b.cap < need);
        if realloc {
            let cap = need.max(slot.as_ref().map_or(0, |b| b.cap * 2)).div_ceil(256) * 256;
            let buf = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: cap,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::VERTEX,
                mapped_at_creation: false,
            });
            *slot = Some(GrowBuf { buf, cap });
        }
        self.gpu.queue.write_buffer(&slot.as_ref().expect("allocated").buf, 0, bytes);
        realloc
    }

    /// The queue submission of the last frame, to wait on without draining later work.
    pub fn last_submission(&self) -> Option<wgpu::SubmissionIndex> {
        self.last_submit.clone()
    }

    fn copy(enc: &mut wgpu::CommandEncoder, from: &Tex, to: &Tex, rect: [u32; 4]) {
        let (w, hh) = (rect[2].saturating_sub(rect[0]), rect[3].saturating_sub(rect[1]));
        if w == 0 || hh == 0 {
            return;
        }
        let o = wgpu::Origin3d { x: rect[0], y: rect[1], z: 0 };
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &from.tex,
                mip_level: 0,
                origin: o,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo { texture: &to.tex, mip_level: 0, origin: o, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: w, height: hh, depth_or_array_layers: 1 },
        );
    }

    fn execute(&mut self, mut plan: Plan, restore: usize, snapshot_at: Option<usize>, hashes: &[u64]) -> RenderStats {
        let d = self.gpu.device.clone();
        // the frame's storage buffers persist across frames; the bind group only when none grew
        let (bg0, verts, paints, stops) = {
            let (mut draws, mut masks, mut edges, mut paints, mut stops, mut verts, old) = match self.frame_bufs.take()
            {
                Some(f) => (
                    Some(f.draws),
                    Some(f.masks),
                    Some(f.edges),
                    Some(f.paints),
                    Some(f.stops),
                    Some(f.verts),
                    Some(f.bg0),
                ),
                None => (None, None, None, None, None, None, None),
            };
            let mut grew = self.upload(&mut draws, &plan.draws, "draws");
            grew |= self.upload(&mut masks, &plan.masks, "masks");
            grew |= self.upload(&mut edges, &plan.edges, "edges");
            grew |= self.upload(&mut paints, &plan.paints.paints, "paints");
            grew |= self.upload(&mut stops, &plan.paints.stops, "stops");
            self.upload(&mut verts, &plan.verts, "vertices");
            let (draws, masks, edges, paints, stops, verts) = (
                draws.expect("allocated"),
                masks.expect("allocated"),
                edges.expect("allocated"),
                paints.expect("allocated"),
                stops.expect("allocated"),
                verts.expect("allocated"),
            );
            let bg0 = match old {
                Some(bg) if !grew => bg,
                _ => d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("frame"),
                    layout: &self.bgl0,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: draws.buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: masks.buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: edges.buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 3, resource: paints.buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 4, resource: stops.buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&self.samp) },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: wgpu::BindingResource::Sampler(&self.samp_repeat),
                        },
                        wgpu::BindGroupEntry { binding: 7, resource: self.globals.as_entire_binding() },
                    ],
                }),
            };
            let (vbuf, pbuf, sbuf) = (verts.buf.clone(), paints.buf.clone(), stops.buf.clone());
            self.frame_bufs = Some(FrameBufs { draws, masks, edges, paints, stops, verts, bg0: bg0.clone() });
            (bg0, vbuf, pbuf, sbuf)
        };
        let bgl2 = self.bgl2.clone();
        let pair = |a: &wgpu::TextureView, b: &wgpu::TextureView| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("backdrop+matte"),
                layout: &bgl2,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(a) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(b) },
                ],
            })
        };
        let dummy_pair = pair(&self.dummy.view, &self.dummy.view);
        // one bind group per (backdrop, matte) texture pair per frame, not one per draw
        let mut pairs: HashMap<(usize, usize), wgpu::BindGroup> = HashMap::new();
        let dummy_gen = {
            let ub = d.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: std::mem::size_of::<Gen>() as u64,
                usage: wgpu::BufferUsages::UNIFORM,
                mapped_at_creation: false,
            });
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.bgl3,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() }],
            })
        };
        let mut enc = d.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        // GPU time: a begin/end pair per effect pass, and for the frame when the device can
        // write timestamps between passes
        self.last_timer = None;
        if self.time_gpu && self.gpu.timestamps {
            let passes: usize = plan
                .jobs
                .iter()
                .map(|j| {
                    j.fx.len() + j.cmds.iter().filter_map(|c| c.pre.as_ref()).map(|p| p.passes.len()).sum::<usize>()
                })
                .sum::<usize>()
                + plan.post.len();
            let mut t = fx::Timer::new(&d, passes as u32 + 1);
            if d.features().contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS) {
                if let Some(a) = t.pair() {
                    enc.write_timestamp(&t.set, a);
                    t.frame = Some(a);
                }
            }
            self.fx.timer = Some(t);
        }
        fn clear_pass<'e>(enc: &'e mut wgpu::CommandEncoder, t: &'e Tex, clear: bool) -> wgpu::RenderPass<'e> {
            enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: if clear { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) } else { wgpu::LoadOp::Load },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
        }
        for rj in &plan.rasters {
            self.raster.record(&d, &mut enc, rj, &paints, &stops, &self.globals);
            plan.stats.targets += 1;
        }
        for gj in &plan.gens {
            let mut pass = clear_pass(&mut enc, &gj.target, true);
            pass.set_pipeline(&self.generator);
            pass.set_bind_group(0, &bg0, &[]);
            pass.set_bind_group(1, &self.dummy.bind, &[]);
            pass.set_bind_group(2, &dummy_pair, &[]);
            pass.set_bind_group(3, &gj.bind, &[]);
            pass.draw(0..3, 0..1);
            plan.stats.targets += 1;
        }
        let jobs = std::mem::take(&mut plan.jobs);
        let mut scratch: HashMap<[u32; 2], Arc<Tex>> = HashMap::new();
        for job in &jobs {
            if let Some((a, b, slot)) = &job.flow {
                // the flow engine submits its own work: flush what is recorded so far first
                let done = std::mem::replace(
                    &mut enc,
                    d.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") }),
                );
                self.gpu.queue.submit([done.finish()]);
                let f = self.video.flow(a, b);
                let _ = slot.set(f.create_view(&Default::default()));
            }
            if !job.fx.is_empty() {
                plan.stats.fx_passes += self.fx.record(&mut enc, &job.fx);
            }
            if let Some(pj) = &job.parts {
                let g = &self.gpu;
                let eng = self
                    .particles
                    .get_or_insert_with(|| Box::new(crate::particles::ParticleEngine::new(g.device.clone(), &g.queue)));
                eng.record(&mut enc, pj, &job.target.view, job.target.size);
            }
            if !job.draw {
                continue;
            }
            plan.stats.targets += 1;
            let mut clear = job.clear;
            let mut i = 0;
            if job.root && restore > 0 {
                if let Some(pf) = &self.prefix {
                    Self::copy(&mut enc, &pf.tex, &job.target, [0, 0, job.target.size[0], job.target.size[1]]);
                    i = restore.min(job.cmds.len());
                    clear = false;
                    plan.stats.prefix_restored = i;
                }
            }
            if job.cmds.is_empty() || i >= job.cmds.len() {
                drop(clear_pass(&mut enc, &job.target, clear));
            }
            while i < job.cmds.len() {
                if job.root && snapshot_at == Some(i) {
                    self.snapshot(&mut enc, &job.target, &hashes[..i]);
                }
                if let Some(pre) = &job.cmds[i].pre {
                    if clear {
                        drop(clear_pass(&mut enc, &job.target, true));
                        clear = false;
                    }
                    Self::copy(&mut enc, &job.target, &pre.snapshot, [0, 0, job.target.size[0], job.target.size[1]]);
                    plan.stats.fx_passes += self.fx.record(&mut enc, &pre.passes);
                    if let (Some(three), Some(eng)) = (&pre.three, self.three.as_mut()) {
                        eng.render(&mut enc, &three.0, Some(&pre.snapshot.view), &three.1.view);
                    }
                }
                if let Some(b) = job.cmds[i].backdrop {
                    if clear {
                        drop(clear_pass(&mut enc, &job.target, true));
                        clear = false;
                    }
                    let s = scratch
                        .entry(job.target.size)
                        .or_insert_with(|| self.pool.get(&d, &self.bgl1, job.target.size))
                        .clone();
                    Self::copy(&mut enc, &job.target, &s, b);
                    plan.stats.backdrop_copies += 1;
                }
                let mut j = i + 1;
                while j < job.cmds.len()
                    && job.cmds[j].backdrop.is_none()
                    && job.cmds[j].pre.is_none()
                    && !(job.root && snapshot_at == Some(j))
                {
                    j += 1;
                }
                let groups: Vec<Option<wgpu::BindGroup>> = job.cmds[i..j]
                    .iter()
                    .map(|c| match (&c.backdrop, &c.matte) {
                        (None, None) => None,
                        (bd, mt) => {
                            let (bk, back) = match bd {
                                Some(_) => {
                                    let s = &scratch[&job.target.size];
                                    (Arc::as_ptr(s) as usize, &s.view)
                                }
                                None => (0, &self.dummy.view),
                            };
                            let (mk, mv) = match mt {
                                Some(t) => (Arc::as_ptr(t) as usize, &t.view),
                                None => (0, &self.dummy.view),
                            };
                            Some(pairs.entry((bk, mk)).or_insert_with(|| pair(back, mv)).clone())
                        }
                    })
                    .collect();
                {
                    let mut pass = clear_pass(&mut enc, &job.target, clear);
                    pass.set_vertex_buffer(0, verts.slice(..));
                    pass.set_bind_group(0, &bg0, &[]);
                    pass.set_bind_group(3, &dummy_gen, &[]);
                    for (k, c) in job.cmds[i..j].iter().enumerate() {
                        pass.set_pipeline(if c.backdrop.is_some() {
                            &self.blend
                        } else if plan.draws[c.draw as usize].blend == BLEND_UNDER {
                            &self.under
                        } else if fixed_function_blend(plan.draws[c.draw as usize].blend) {
                            &self.additive
                        } else {
                            &self.over
                        });
                        pass.set_bind_group(1, &c.src.bind, &[]);
                        pass.set_bind_group(2, groups[k].as_ref().unwrap_or(&dummy_pair), &[]);
                        pass.draw(c.first_vertex..c.first_vertex + c.count, c.draw..c.draw + 1);
                        plan.stats.draws += 1;
                    }
                }
                clear = false;
                i = j;
            }
            if job.root && snapshot_at == Some(job.cmds.len()) {
                self.snapshot(&mut enc, &job.target, &hashes[..job.cmds.len()]);
            }
        }
        if !plan.post.is_empty() {
            plan.stats.fx_passes += self.fx.record(&mut enc, &plan.post);
            if let (Some(out), Some(root)) = (&plan.post_out, jobs.iter().find(|j| j.root)) {
                Self::copy(&mut enc, out, &root.target, [0, 0, root.target.size[0], root.target.size[1]]);
            }
        }
        if let Some(t) = self.fx.timer.take() {
            if let Some(a) = t.frame {
                enc.write_timestamp(&t.set, a + 1);
            }
            t.resolve(&mut enc);
            self.last_timer = Some(t);
        }
        self.last_submit = Some(self.gpu.queue.submit([enc.finish()]));
        for (_, t) in scratch {
            self.pool.put(t);
        }
        drop(jobs);
        plan.post.clear();
        plan.post_out = None;
        for t in std::mem::take(&mut plan.fx_temps) {
            self.pool.put(t);
        }
        plan.stats
    }

    fn snapshot(&mut self, enc: &mut wgpu::CommandEncoder, target: &Tex, prefix: &[u64]) {
        let tex = match self.prefix.take() {
            Some(p) if p.tex.size == target.size => p.tex,
            _ => Arc::new(resources::create(&self.gpu.device, &self.bgl1, target.size, 1, "prefix")),
        };
        Self::copy(enc, target, &tex, [0, 0, target.size[0], target.size[1]]);
        self.prefix = Some(Prefix { hashes: prefix.to_vec(), tex });
    }

    /// Reads a texture back as premultiplied RGBA f32 in the working representation.
    pub fn read(&self, t: &Tex) -> Vec<[f32; 4]> {
        let (w, hh) = (t.size[0], t.size[1]);
        let row = (w * 8).div_ceil(256) * 256;
        let buf = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (row * hh) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.gpu.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &t.tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(hh) },
            },
            wgpu::Extent3d { width: w, height: hh, depth_or_array_layers: 1 },
        );
        self.gpu.queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.gpu.wait();
        let data = buf.slice(..).get_mapped_range().expect("mapped");
        let mut out = Vec::with_capacity((w * hh) as usize);
        for y in 0..hh {
            let line = &data[(y * row) as usize..(y * row + w * 8) as usize];
            for px in line.chunks_exact(8) {
                let c = |k: usize| half::f16::from_le_bytes([px[k], px[k + 1]]).to_f32();
                out.push([c(0), c(2), c(4), c(6)]);
            }
        }
        out
    }

    /// Display-referred 8-bit sRGB RGBA (straight alpha) of a frame.
    pub fn to_srgb8(&self, px: &[[f32; 4]]) -> Vec<u8> {
        let mut out = Vec::with_capacity(px.len() * 4);
        for p in px {
            let a = p[3].clamp(0.0, 1.0) as f64;
            let c = if a > 0.0 { [p[0] as f64 / a, p[1] as f64 / a, p[2] as f64 / a] } else { [0.0; 3] };
            let d = self.working.to_display_srgb(c);
            out.extend(d.map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8));
            out.push((a * 255.0).round() as u8);
        }
        out
    }
}

#[derive(Clone, Copy)]
struct Ctx<'a> {
    g: &'a FrameGraph,
    p: &'a Program,
    kids: &'a [Vec<usize>],
    /// Asset keys whose pixels depend on source time.
    timed: &'a std::collections::HashSet<String>,
    /// Generator asset keys (their parameters live in `elements`).
    generated: &'a std::collections::HashSet<String>,
    /// Hash of all animated paint and generator values.
    elements: u64,
    /// Hash of each animated element outside the composition, by key.
    el: &'a HashMap<Arc<str>, u64>,
    /// Hash of the camera, which projects every 2.5D and 3D draw.
    cam: u64,
    /// The scene at other times (motion blur, time effects), when a provider was given.
    sub: Option<&'a dyn SubSource>,
}

impl<'a> Ctx<'a> {
    /// The same context over the scene at another time.
    fn at<'b>(&self, sg: &'b SubGraph) -> Ctx<'b>
    where
        'a: 'b,
    {
        Ctx {
            g: &sg.g,
            p: self.p,
            kids: &sg.kids,
            timed: self.timed,
            generated: self.generated,
            elements: sg.elements,
            el: &sg.el,
            cam: self.cam,
            sub: self.sub,
        }
    }
}

/// Source of sub-frame graphs.
pub(crate) trait SubSource {
    fn at(&self, t: f64) -> Arc<SubGraph>;
    fn count(&self) -> usize;
}

/// A sub-frame FrameGraph with its child lists and id index.
pub(crate) struct SubGraph {
    g: FrameGraph,
    kids: Vec<Vec<usize>>,
    index: HashMap<Arc<str>, usize>,
    elements: u64,
    el: HashMap<Arc<str>, u64>,
}

/// Evaluates and caches the scene at other times during one frame.
pub(crate) struct SubFrames<'a> {
    provider: std::cell::RefCell<&'a mut dyn FnMut(f64) -> FrameGraph>,
    cache: std::cell::RefCell<HashMap<u64, Arc<SubGraph>>>,
    /// Seconds spent in the provider.
    seconds: std::cell::Cell<f64>,
}

impl SubSource for SubFrames<'_> {
    fn count(&self) -> usize {
        self.cache.borrow().len()
    }
    fn at(&self, t: f64) -> Arc<SubGraph> {
        let key = (t * 1e6).round() as i64 as u64;
        if let Some(s) = self.cache.borrow().get(&key) {
            return s.clone();
        }
        let started = std::time::Instant::now();
        let g = (self.provider.borrow_mut())(t);
        self.seconds.set(self.seconds.get() + started.elapsed().as_secs_f64());
        let mut kids = vec![Vec::new(); g.nodes.len()];
        let mut index = HashMap::new();
        for (i, n) in g.nodes.iter().enumerate() {
            if let Some(pi) = n.parent {
                kids[pi as usize].push(i);
            }
            index.insert(n.id.clone(), i);
        }
        let elements = elements_hash(&g.elements);
        let el = element_hashes(&g.elements);
        let s = Arc::new(SubGraph { g, kids, index, elements, el });
        self.cache.borrow_mut().insert(key, s.clone());
        s
    }
}
