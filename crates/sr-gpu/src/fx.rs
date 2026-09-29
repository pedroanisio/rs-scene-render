//! Effects, transitions and colour finishing: GPU passes over offscreen
//! textures. One WGSL module holds every built-in pass (`effects.wgsl`);
//! custom effects and transitions are GLSL compiled through naga. A
//! [`Builder`] turns effect elements into passes on pooled textures; the
//! renderer records them between the jobs that produce and consume them.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use sr_eval::Value;
use sr_model::element::Element;
use sr_model::model as m;

use crate::color::{self, Working};
use crate::resources::{Pool, Tex, FORMAT};
use crate::vector::Attrs;

/// The WGSL of every built-in pass.
pub const WGSL: &str =
    concat!(include_str!("transfer.wgsl"), "\n", include_str!("d24.wgsl"), "\n", include_str!("effects.wgsl"));

/// Gradient stops: (offset, straight stored working RGBA).
pub type Stops = Vec<(f64, [f64; 4])>;
/// Resolves a colour value to straight stored working RGBA.
pub type ColorFn<'a> = dyn Fn(&Value) -> Option<[f64; 4]> + 'a;
/// Resolves a paint value to gradient stops.
pub type GradientFn<'a> = dyn Fn(&Value) -> Option<Stops> + 'a;
/// A flow field filled in while the frame executes.
pub type FlowSlot = Arc<std::sync::OnceLock<wgpu::TextureView>>;

/// Uniform block of a pass (matches `Fx` in effects.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default, Debug, PartialEq)]
pub struct Params {
    pub v: [[f32; 4]; 8],
    pub i: [u32; 4],
}

/// A pass entry point.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Entry {
    Copy,
    Down,
    Up,
    Pre,
    Combine,
    Color,
    Warp,
    Line,
    Bokeh,
    Conv,
    Morph,
    Flow,
    Trans,
    /// A compiled GLSL shader, by source hash.
    Custom(u64),
}

impl Entry {
    fn name(self) -> &'static str {
        match self {
            Entry::Copy => "fs_copy",
            Entry::Down => "fs_down",
            Entry::Up => "fs_up",
            Entry::Pre => "fs_pre",
            Entry::Combine => "fs_combine",
            Entry::Color => "fs_color",
            Entry::Warp => "fs_warp",
            Entry::Line => "fs_line",
            Entry::Bokeh => "fs_bokeh",
            Entry::Conv => "fs_conv",
            Entry::Morph => "fs_morph",
            Entry::Flow => "fs_flow",
            Entry::Trans => "fs_trans",
            Entry::Custom(_) => "main",
        }
    }
}

/// Second input of a pass.
#[derive(Clone)]
pub enum Aux {
    None,
    Tex(Arc<Tex>),
    View(Arc<wgpu::TextureView>),
    /// An unfilterable flow field (binding 6), known once the frame's flow step ran.
    Flow(FlowSlot),
}

/// A 3D lookup table on the GPU.
pub struct Lut3 {
    pub tex: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: u32,
}

/// One pass: draw `entry` over `out` reading `src` (and the extras).
pub struct Pass {
    pub entry: Entry,
    pub params: Params,
    pub src: Arc<Tex>,
    pub aux: Aux,
    pub aux2: Option<Arc<Tex>>,
    pub lut: Option<Arc<Lut3>>,
    pub out: Arc<Tex>,
    /// Add into `out` instead of replacing it.
    pub additive: bool,
    /// Clear `out` first (additive accumulation starts from zero).
    pub clear: bool,
    /// A custom GLSL program with its own bindings (then `entry` and the fixed inputs are unused).
    pub custom: Option<Box<crate::shader::CustomBind>>,
}

impl Pass {
    /// A custom program writing `out`.
    pub fn custom(bind: crate::shader::CustomBind, out: Arc<Tex>) -> Pass {
        Pass {
            entry: Entry::Copy,
            params: Params::default(),
            src: out.clone(),
            aux: Aux::None,
            aux2: None,
            lut: None,
            out,
            additive: false,
            clear: true,
            custom: Some(Box::new(bind)),
        }
    }
}

/// Pipelines, samplers and caches.
pub struct FxEngine {
    pub(crate) device: Arc<wgpu::Device>,
    pub(crate) queue: Arc<wgpu::Queue>,
    bgl: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    pub(crate) module: wgpu::ShaderModule,
    pipes: HashMap<(Entry, bool), wgpu::RenderPipeline>,
    custom: HashMap<u64, Result<wgpu::ShaderModule, String>>,
    pub(crate) samp: wgpu::Sampler,
    dummy2: wgpu::TextureView,
    dummy3: wgpu::TextureView,
    luts: HashMap<String, Result<Arc<Lut3>, String>>,
    /// Custom GLSL programs by source hash (see `shader`).
    pub(crate) programs: HashMap<u64, Result<Arc<crate::shader::CustomPipe>, String>>,
    /// Persistent ISF buffers by effect instance (latest), their state at the start of the
    /// current frame, and checkpoints by frame.
    pub(crate) feedback: HashMap<String, crate::shader::Feedback>,
    pub(crate) feedback_frame: (i64, HashMap<String, crate::shader::Feedback>),
    pub(crate) checkpoints: std::collections::BTreeMap<i64, HashMap<String, crate::shader::Feedback>>,
    /// Images uploaded for shader samplers, by path.
    pub(crate) images: HashMap<std::path::PathBuf, Arc<Tex>>,
}

impl FxEngine {
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> FxEngine {
        let tex = |binding: u32, dim: wgpu::TextureViewDimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: dim,
                multisampled: false,
            },
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex(1, wgpu::TextureViewDimension::D2),
                tex(2, wgpu::TextureViewDimension::D2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex(4, wgpu::TextureViewDimension::D3),
                tex(5, wgpu::TextureViewDimension::D2),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("effects"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });
        let samp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fx"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let mk = |dim: wgpu::TextureDimension, vd: wgpu::TextureViewDimension| {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("fx dummy"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: dim,
                format: FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(vd), ..Default::default() })
        };
        let dummy2 = mk(wgpu::TextureDimension::D2, wgpu::TextureViewDimension::D2);
        let dummy3 = mk(wgpu::TextureDimension::D3, wgpu::TextureViewDimension::D3);
        FxEngine {
            device,
            queue,
            bgl,
            layout,
            module,
            pipes: HashMap::new(),
            custom: HashMap::new(),
            samp,
            dummy2,
            dummy3,
            luts: HashMap::new(),
            programs: HashMap::new(),
            feedback: HashMap::new(),
            feedback_frame: (i64::MIN, HashMap::new()),
            checkpoints: std::collections::BTreeMap::new(),
            images: HashMap::new(),
        }
    }

    fn pipeline(&mut self, entry: Entry, additive: bool) -> Option<&wgpu::RenderPipeline> {
        if !self.pipes.contains_key(&(entry, additive)) {
            let frag_module = match entry {
                Entry::Custom(h) => self.custom.get(&h)?.as_ref().ok()?,
                _ => &self.module,
            };
            let blend = additive.then_some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            });
            let p = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry.name()),
                layout: Some(&self.layout),
                vertex: wgpu::VertexState {
                    module: &self.module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: frag_module,
                    entry_point: Some(entry.name()),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
            self.pipes.insert((entry, additive), p);
        }
        self.pipes.get(&(entry, additive))
    }

    /// Records passes in order.
    pub fn record(&mut self, enc: &mut wgpu::CommandEncoder, passes: &[Pass]) -> usize {
        use wgpu::util::DeviceExt;
        let mut n = 0;
        for p in passes {
            if let Some(c) = &p.custom {
                self.record_custom(enc, c, &p.out);
                n += 1;
                continue;
            }
            let ub = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fx params"),
                contents: bytemuck::bytes_of(&p.params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let (aux_view, flow_view): (&wgpu::TextureView, &wgpu::TextureView) = match &p.aux {
                Aux::Tex(t) => (&t.view, &self.dummy2),
                Aux::View(v) => (v, &self.dummy2),
                Aux::Flow(slot) => (&self.dummy2, slot.get().unwrap_or(&self.dummy2)),
                Aux::None => (&self.dummy2, &self.dummy2),
            };
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fx"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: ub.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&p.src.view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(aux_view) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.samp) },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(
                            p.lut.as_ref().map(|l| &l.view).unwrap_or(&self.dummy3),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(
                            p.aux2.as_ref().map(|t| &t.view).unwrap_or(&self.dummy2),
                        ),
                    },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(flow_view) },
                ],
            });
            let Some(pipe) = self.pipeline(p.entry, p.additive) else { continue };
            let pipe = pipe.clone();
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fx"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &p.out.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: if p.clear || !p.additive {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&pipe);
            rp.set_bind_group(0, &bg, &[]);
            rp.draw(0..3, 0..1);
            n += 1;
        }
        n
    }

    /// Compiles a custom GLSL fragment shader (already wrapped); returns its entry.
    pub fn custom(&mut self, glsl: &str) -> Result<Entry, String> {
        let h = sr_eval::rng::hash_str(glsl);
        if !self.custom.contains_key(&h) {
            let r = check_glsl(glsl).map(|_| {
                self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("custom"),
                    source: wgpu::ShaderSource::Glsl {
                        shader: glsl.to_string().into(),
                        stage: wgpu::naga::ShaderStage::Fragment,
                        defines: &[],
                    },
                })
            });
            self.custom.insert(h, r);
        }
        self.custom[&h].as_ref().map(|_| Entry::Custom(h)).map_err(|e| e.clone())
    }

    /// A 3D LUT from a file (cached by path).
    pub fn lut(&mut self, path: &Path) -> Result<Arc<Lut3>, String> {
        let key = path.display().to_string();
        if !self.luts.contains_key(&key) {
            let r = load_lut(path).map(|(n, data)| Arc::new(self.upload_lut(n, &data)));
            self.luts.insert(key.clone(), r);
        }
        self.luts[&key].clone()
    }

    /// A 3D LUT made by `load` (cached by `key`, with its error).
    pub fn lut_with(
        &mut self,
        key: &str,
        load: impl FnOnce() -> Result<(u32, Vec<[f32; 3]>), String>,
    ) -> Result<Arc<Lut3>, String> {
        if !self.luts.contains_key(key) {
            let r = load().map(|(n, data)| Arc::new(self.upload_lut(n, &data)));
            self.luts.insert(key.to_string(), r);
        }
        self.luts[key].clone()
    }

    /// Uploads an n³ table (red fastest).
    pub fn upload_lut(&self, n: u32, data: &[[f32; 3]]) -> Lut3 {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lut"),
            size: wgpu::Extent3d { width: n, height: n, depth_or_array_layers: n },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let bytes: Vec<u8> = data
            .iter()
            .flat_map(|c| [c[0], c[1], c[2], 1.0].into_iter().flat_map(|v| half::f16::from_f32(v).to_le_bytes()))
            .collect();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(n * 8), rows_per_image: Some(n) },
            wgpu::Extent3d { width: n, height: n, depth_or_array_layers: n },
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D3),
            ..Default::default()
        });
        Lut3 { tex, view, size: n }
    }

    /// A 256×1 table texture (curves, gradient maps), premultiplied RGBA.
    pub fn table(&self, pool: &mut Pool, bgl1: &wgpu::BindGroupLayout, px: &[[f32; 4]; 256]) -> Arc<Tex> {
        let t = pool.get(&self.device, bgl1, [256, 1]);
        let bytes: Vec<u8> =
            px.iter().flat_map(|c| c.iter().flat_map(|v| half::f16::from_f32(*v).to_le_bytes())).collect();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &t.tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 8), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
        );
        t
    }
}

impl FxEngine {
    /// A texels.len()×1 data texture of f16 values (integers up to 2048 are exact).
    pub fn data_table(&self, pool: &mut Pool, bgl1: &wgpu::BindGroupLayout, texels: &[[f32; 4]]) -> Arc<Tex> {
        let w = texels.len().max(1) as u32;
        let t = pool.get(&self.device, bgl1, [w, 1]);
        let mut bytes: Vec<u8> =
            texels.iter().flat_map(|c| c.iter().flat_map(|v| half::f16::from_f32(*v).to_le_bytes())).collect();
        bytes.resize(w as usize * 8, 0);
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &t.tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 8), rows_per_image: Some(1) },
            wgpu::Extent3d { width: w, height: 1, depth_or_array_layers: 1 },
        );
        t
    }
}

/// Parses and validates GLSL with naga, returning its errors as text.
pub fn check_glsl(src: &str) -> Result<(), String> {
    use wgpu::naga;
    let mut fe = naga::front::glsl::Frontend::default();
    let module = fe
        .parse(&naga::front::glsl::Options::from(naga::ShaderStage::Fragment), src)
        .map_err(|e| e.emit_to_string(src))?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default())
        .validate(&module)
        .map_err(|e| e.emit_to_string(src))?;
    Ok(())
}

// ------------------------------------------------------------------ LUT files

fn bit_scale(d: Option<&str>) -> f32 {
    match d.unwrap_or("32f") {
        "8i" => 255.0,
        "10i" => 1023.0,
        "12i" => 4095.0,
        "16i" => 65535.0,
        _ => 1.0,
    }
}

enum Op {
    Lut1(Vec<[f32; 3]>),
    Lut3(u32, Vec<[f32; 3]>),
    Matrix([[f32; 4]; 3]),
    Range([f32; 4]),
    Cdl([f32; 3], [f32; 3], [f32; 3], f32),
}

fn lerp1(t: &[[f32; 3]], x: f32, c: usize) -> f32 {
    let n = t.len();
    if n == 0 {
        return x;
    }
    let f = x.clamp(0.0, 1.0) * (n - 1) as f32;
    let i = (f.floor() as usize).min(n - 1);
    let j = (i + 1).min(n - 1);
    t[i][c] + (t[j][c] - t[i][c]) * (f - i as f32)
}

fn lerp3(n: u32, t: &[[f32; 3]], x: [f32; 3]) -> [f32; 3] {
    let n1 = (n - 1) as f32;
    let f = x.map(|v| v.clamp(0.0, 1.0) * n1);
    let i = f.map(|v| (v.floor() as u32).min(n - 1));
    let j = i.map(|v| (v + 1).min(n - 1));
    let w = [f[0] - i[0] as f32, f[1] - i[1] as f32, f[2] - i[2] as f32];
    let at = |r: u32, g: u32, b: u32| t[(r + g * n + b * n * n) as usize];
    let mut o = [0.0; 3];
    for (dr, wr) in [(i[0], 1.0 - w[0]), (j[0], w[0])] {
        for (dg, wg) in [(i[1], 1.0 - w[1]), (j[1], w[1])] {
            for (db, wb) in [(i[2], 1.0 - w[2]), (j[2], w[2])] {
                let v = at(dr, dg, db);
                for c in 0..3 {
                    o[c] += v[c] * wr * wg * wb;
                }
            }
        }
    }
    o
}

fn apply_ops(ops: &[Op], mut c: [f32; 3]) -> [f32; 3] {
    for op in ops {
        c = match op {
            Op::Lut1(t) => [lerp1(t, c[0], 0), lerp1(t, c[1], 1), lerp1(t, c[2], 2)],
            Op::Lut3(n, t) => lerp3(*n, t, c),
            Op::Matrix(mx) => [0, 1, 2].map(|r| mx[r][0] * c[0] + mx[r][1] * c[1] + mx[r][2] * c[2] + mx[r][3]),
            Op::Range(r) => c.map(|v| (v - r[0]) / (r[1] - r[0]).max(1e-9) * (r[3] - r[2]) + r[2]),
            Op::Cdl(s, o, p, sat) => {
                let v = [0, 1, 2].map(|k| (c[k] * s[k] + o[k]).max(0.0).powf(p[k]));
                let l = 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
                v.map(|x| l + sat * (x - l))
            }
        };
    }
    c
}

fn nums(s: &str) -> Vec<f32> {
    s.split(|c: char| c.is_whitespace() || c == ',').filter_map(|t| t.parse().ok()).collect()
}

/// Loads a .cube, .3dl or .clf LUT as an n³ table (red fastest).
pub fn load_lut(path: &Path) -> Result<(u32, Vec<[f32; 3]>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let err = |m: &str| format!("{}: {m}", path.display());
    let ops: Vec<Op> = match ext.as_str() {
        "cube" => {
            let (mut n3, mut n1) = (0u32, 0u32);
            let (mut dmin, mut dmax) = ([0.0f32; 3], [1.0f32; 3]);
            let mut rows: Vec<[f32; 3]> = Vec::new();
            for l in text.lines() {
                let l = l.trim();
                if l.is_empty() || l.starts_with('#') || l.starts_with("TITLE") {
                    continue;
                }
                let mut it = l.split_whitespace();
                match it.next() {
                    Some("LUT_3D_SIZE") => {
                        n3 = it.next().and_then(|v| v.parse().ok()).ok_or_else(|| err("bad LUT_3D_SIZE"))?
                    }
                    Some("LUT_1D_SIZE") => {
                        n1 = it.next().and_then(|v| v.parse().ok()).ok_or_else(|| err("bad LUT_1D_SIZE"))?
                    }
                    Some("DOMAIN_MIN") => {
                        let v = nums(&l[10..]);
                        if v.len() == 3 {
                            dmin = [v[0], v[1], v[2]];
                        }
                    }
                    Some("DOMAIN_MAX") => {
                        let v = nums(&l[10..]);
                        if v.len() == 3 {
                            dmax = [v[0], v[1], v[2]];
                        }
                    }
                    Some(_) => {
                        let v = nums(l);
                        if v.len() == 3 {
                            rows.push([v[0], v[1], v[2]]);
                        }
                    }
                    None => {}
                }
            }
            let range = Op::Matrix([
                [1.0 / (dmax[0] - dmin[0]).max(1e-9), 0.0, 0.0, -dmin[0] / (dmax[0] - dmin[0]).max(1e-9)],
                [0.0, 1.0 / (dmax[1] - dmin[1]).max(1e-9), 0.0, -dmin[1] / (dmax[1] - dmin[1]).max(1e-9)],
                [0.0, 0.0, 1.0 / (dmax[2] - dmin[2]).max(1e-9), -dmin[2] / (dmax[2] - dmin[2]).max(1e-9)],
            ]);
            let unit = dmin == [0.0; 3] && dmax == [1.0; 3];
            let mut ops = if unit { Vec::new() } else { vec![range] };
            if n3 > 1 {
                if rows.len() != (n3 * n3 * n3) as usize {
                    return Err(err(&format!("expected {} rows, found {}", n3 * n3 * n3, rows.len())));
                }
                ops.push(Op::Lut3(n3, rows));
                ops
            } else if n1 > 1 {
                if rows.len() != n1 as usize {
                    return Err(err("row count does not match LUT_1D_SIZE"));
                }
                ops.push(Op::Lut1(rows));
                ops
            } else {
                return Err(err("no LUT_3D_SIZE or LUT_1D_SIZE"));
            }
        }
        "3dl" => {
            // optional shaper line, then n³ rows of integers with blue fastest
            let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#'));
            let first = nums(lines.next().ok_or_else(|| err("empty"))?);
            let mut rows: Vec<[f32; 3]> = Vec::new();
            if first.len() == 3 {
                rows.push([first[0], first[1], first[2]]);
            }
            for l in lines {
                let v = nums(l);
                if v.len() == 3 {
                    rows.push([v[0], v[1], v[2]]);
                }
            }
            let n = (rows.len() as f64).cbrt().round() as u32;
            if n < 2 || (n * n * n) as usize != rows.len() {
                return Err(err("row count is not a cube"));
            }
            let max = rows.iter().flat_map(|r| r.iter().copied()).fold(0.0f32, f32::max);
            let scale = [1023.0f32, 4095.0, 65535.0].into_iter().find(|s| max <= *s).unwrap_or(max.max(1.0));
            let mut t = vec![[0.0f32; 3]; rows.len()];
            for r in 0..n {
                for g in 0..n {
                    for b in 0..n {
                        let src = rows[(b + g * n + r * n * n) as usize];
                        t[(r + g * n + b * n * n) as usize] = src.map(|v| v / scale);
                    }
                }
            }
            vec![Op::Lut3(n, t)]
        }
        "clf" | "xml" => {
            let doc = roxmltree::Document::parse(&text).map_err(|e| err(&e.to_string()))?;
            let mut ops = Vec::new();
            for node in doc.root_element().children().filter(|c| c.is_element()) {
                let out_scale = bit_scale(node.attribute("outBitDepth"));
                let in_scale = bit_scale(node.attribute("inBitDepth"));
                let array = node.descendants().find(|c| c.has_tag_name("Array") || c.tag_name().name() == "Array");
                let arr = |a: roxmltree::Node| -> (Vec<u32>, Vec<f32>) {
                    (
                        a.attribute("dim")
                            .map(|d| d.split_whitespace().filter_map(|x| x.parse().ok()).collect())
                            .unwrap_or_default(),
                        nums(a.text().unwrap_or("")),
                    )
                };
                match node.tag_name().name() {
                    "LUT1D" => {
                        let (dim, v) = arr(array.ok_or_else(|| err("LUT1D without Array"))?);
                        let ch = dim.get(1).copied().unwrap_or(1) as usize;
                        let t: Vec<[f32; 3]> = v
                            .chunks(ch.max(1))
                            .map(|c| if ch >= 3 { [c[0], c[1], c[2]] } else { [c[0]; 3] }.map(|x| x / out_scale))
                            .collect();
                        if in_scale != 1.0 {
                            ops.push(Op::Range([0.0, 1.0, 0.0, 1.0]));
                        }
                        ops.push(Op::Lut1(t));
                    }
                    "LUT3D" => {
                        let (dim, v) = arr(array.ok_or_else(|| err("LUT3D without Array"))?);
                        let n = dim.first().copied().unwrap_or(0);
                        if n < 2 || v.len() != (n * n * n * 3) as usize {
                            return Err(err("LUT3D Array size does not match dim"));
                        }
                        let mut t = vec![[0.0f32; 3]; (n * n * n) as usize];
                        for r in 0..n {
                            for g in 0..n {
                                for b in 0..n {
                                    let k = ((b + g * n + r * n * n) * 3) as usize;
                                    t[(r + g * n + b * n * n) as usize] =
                                        [v[k], v[k + 1], v[k + 2]].map(|x| x / out_scale);
                                }
                            }
                        }
                        ops.push(Op::Lut3(n, t));
                    }
                    "Matrix" => {
                        let (dim, v) = arr(array.ok_or_else(|| err("Matrix without Array"))?);
                        let cols = dim.get(1).copied().unwrap_or(3) as usize;
                        let mut mx = [[0.0f32; 4]; 3];
                        for (r, row) in mx.iter_mut().enumerate() {
                            for (c, cell) in row.iter_mut().enumerate().take(cols.min(4)) {
                                *cell = v.get(r * cols + c).copied().unwrap_or(0.0) * in_scale / out_scale;
                            }
                        }
                        ops.push(Op::Matrix(mx));
                    }
                    "Range" => {
                        let g = |n: &str| {
                            node.children()
                                .find(|c| c.has_tag_name(n))
                                .and_then(|c| c.text())
                                .and_then(|t| t.trim().parse::<f32>().ok())
                        };
                        ops.push(Op::Range([
                            g("minInValue").unwrap_or(0.0) / in_scale,
                            g("maxInValue").unwrap_or(in_scale) / in_scale,
                            g("minOutValue").unwrap_or(0.0) / out_scale,
                            g("maxOutValue").unwrap_or(out_scale) / out_scale,
                        ]));
                    }
                    "ASC_CDL" => {
                        let g = |n: &str, d: f32| -> [f32; 3] {
                            node.descendants()
                                .find(|c| c.has_tag_name(n))
                                .and_then(|c| c.text())
                                .map(nums)
                                .filter(|v| v.len() >= 3)
                                .map(|v| [v[0], v[1], v[2]])
                                .unwrap_or([d; 3])
                        };
                        let sat = node
                            .descendants()
                            .find(|c| c.has_tag_name("Saturation"))
                            .and_then(|c| c.text())
                            .and_then(|t| t.trim().parse().ok())
                            .unwrap_or(1.0);
                        ops.push(Op::Cdl(g("Slope", 1.0), g("Offset", 0.0), g("Power", 1.0), sat));
                    }
                    "Description" | "InputDescriptor" | "OutputDescriptor" | "Info" => {}
                    other => return Err(err(&format!("unsupported CLF node <{other}>"))),
                }
            }
            ops
        }
        _ => return Err(err("LUTs must be .cube, .3dl or .clf")),
    };
    // a single 3D table uploads as is; anything else bakes to 33³
    if let [Op::Lut3(n, t)] = ops.as_slice() {
        return Ok((*n, t.clone()));
    }
    let n = 33u32;
    let mut t = Vec::with_capacity((n * n * n) as usize);
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                let c = [r, g, b].map(|v| v as f32 / (n - 1) as f32);
                t.push(apply_ops(&ops, c));
            }
        }
    }
    Ok((n, t))
}

// ------------------------------------------------------------------ building chains

/// Context of an effect chain on one offscreen.
pub struct Cx<'a> {
    /// Offscreen pixels per document pixel of the node.
    pub px: f64,
    /// Output pixels per document pixel (the render scale, without the node's own transform).
    pub scale: f64,
    /// Node-local direction → offscreen direction (2×2, column-major a, b, c, d).
    pub lin: [f64; 4],
    /// Node-local px → offscreen uv.
    pub to_uv: &'a dyn Fn([f64; 2]) -> [f64; 2],
    /// Default centre (uv).
    pub center: [f64; 2],
    /// The effect's input in offscreen pixels (x0, y0, x1, y1): the node's content box, or the whole
    /// offscreen (adjustment layers, unknown extents).
    pub content: Option<[f64; 4]>,
    pub time: f64,
    pub frame: i64,
    /// A colour value → straight stored working RGBA.
    pub color: &'a ColorFn<'a>,
    /// A paint value → gradient stops (offset, straight stored RGBA).
    pub gradient: &'a GradientFn<'a>,
    /// The second input (`source`), covering the offscreen.
    pub source: Option<Arc<Tex>>,
    /// Point lights: uv x, y, intensity, radius (uv).
    pub lights: Vec<[f32; 4]>,
    pub working: Working,
    pub base: &'a Path,
    /// Frames per second of the composition.
    pub fps: f64,
    /// Seconds since the node started.
    pub local_time: f64,
    /// Offscreen origin in frame pixels, GL orientation (bottom-left).
    pub offset: [f64; 2],
    /// Frame size in pixels.
    pub frame_size: [f64; 2],
    /// The node the effect runs on (keys persistent ISF buffers).
    pub node: String,
    /// Named sampler inputs (nodes, image assets) covering the offscreen, stored working premultiplied.
    pub named: HashMap<String, Arc<Tex>>,
    /// The audio mix, when the render has one (ISF audio inputs).
    pub audio: Option<Arc<crate::shader::AudioSignals>>,
    /// The project seed (seeded effects draw from the seeded 64-bit hash with per-element seeds derived from it).
    pub seed: u64,
}

/// The seed of an effect's seeded 64-bit hash draws (`Params.rng`): the CRC-32
/// element seed of the project seed, the effect's id and @seed, with the seed's value as a
/// float print (`0.0` when absent, `7.0` for seed="7") as the purpose.
pub fn effect_seed(project: u64, e: &dyn Element, seed: Option<f64>) -> u64 {
    let v = seed.unwrap_or(0.0);
    let purpose = if v.fract() == 0.0 && v.abs() < 1e16 { format!("{v:.1}") } else { format!("{v}") };
    sr_eval::rng::element_seed(project, e.element_id().unwrap_or(""), seed.map(|v| v as u64), &purpose)
}

/// A named parameter: the effect's (animated) attribute, else a `<param>` child, else `default`.
fn param(e: &dyn Element, a: &Attrs, name: &str, default: f64) -> f64 {
    a.opt(name)
        .or_else(|| params_of(e).into_iter().find(|p| p.0 == name).and_then(|p| p.1.first().map(|v| *v as f64)))
        .unwrap_or(default)
}

/// Collects passes and the textures they use.
pub struct Builder<'a> {
    pub eng: &'a mut FxEngine,
    pub pool: &'a mut Pool,
    pub device: &'a wgpu::Device,
    pub bgl1: &'a wgpu::BindGroupLayout,
    pub passes: Vec<Pass>,
    pub temps: Vec<Arc<Tex>>,
    pub store: u32,
    pub problems: Vec<String>,
}

fn v4(a: [f64; 4]) -> [f32; 4] {
    a.map(|x| x as f32)
}

fn triple(s: Option<String>, d: f64) -> [f64; 3] {
    let v: Vec<f64> = s
        .as_deref()
        .map(|s| s.split(|c: char| c == ',' || c.is_whitespace()).filter_map(|t| t.parse().ok()).collect())
        .unwrap_or_default();
    match v.len() {
        0 => [d; 3],
        1 | 2 => [v[0]; 3],
        _ => [v[0], v[1], v[2]],
    }
}

/// Catmull-Rom through sorted control points, sampled at 256 positions.
fn curve_table(s: &str) -> Option<[f32; 256]> {
    let mut pts: Vec<(f64, f64)> = s
        .split_whitespace()
        .filter_map(|p| {
            let (x, y) = p.split_once(',')?;
            Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
        })
        .collect();
    if pts.len() < 2 {
        return None;
    }
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = [0.0f32; 256];
    for (k, o) in out.iter_mut().enumerate() {
        let x = k as f64 / 255.0;
        let i = pts.iter().rposition(|p| p.0 <= x).unwrap_or(0).min(pts.len() - 2);
        let (p1, p2) = (pts[i], pts[i + 1]);
        let p0 = if i > 0 { pts[i - 1] } else { (2.0 * p1.0 - p2.0, 2.0 * p1.1 - p2.1) };
        let p3 = if i + 2 < pts.len() { pts[i + 2] } else { (2.0 * p2.0 - p1.0, 2.0 * p2.1 - p1.1) };
        let t = ((x - p1.0) / (p2.0 - p1.0).max(1e-9)).clamp(0.0, 1.0);
        let t2 = t * t;
        let y = 0.5
            * ((2.0 * p1.1)
                + (-p0.1 + p2.1) * t
                + (2.0 * p0.1 - 5.0 * p1.1 + 4.0 * p2.1 - p3.1) * t2
                + (-p0.1 + 3.0 * p1.1 - 3.0 * p2.1 + p3.1) * t2 * t);
        *o = if x < pts[0].0 {
            pts[0].1 as f32
        } else if x > pts[pts.len() - 1].0 {
            pts[pts.len() - 1].1 as f32
        } else {
            y.clamp(0.0, 1.0) as f32
        };
    }
    Some(out)
}

/// Premultiplied 256-entry table of gradient stops.
fn gradient_table(stops: &[(f64, [f64; 4])]) -> [[f32; 4]; 256] {
    let mut out = [[0.0f32; 4]; 256];
    for (k, o) in out.iter_mut().enumerate() {
        let t = k as f64 / 255.0;
        let c = if stops.is_empty() {
            [t, t, t, 1.0]
        } else if t <= stops[0].0 {
            stops[0].1
        } else if t >= stops[stops.len() - 1].0 {
            stops[stops.len() - 1].1
        } else {
            let i = stops.iter().rposition(|s| s.0 <= t).unwrap_or(0).min(stops.len() - 2);
            let (a, b) = (stops[i], stops[i + 1]);
            let w = ((t - a.0) / (b.0 - a.0).max(1e-9)).clamp(0.0, 1.0);
            [0, 1, 2, 3].map(|j| a.1[j] + (b.1[j] - a.1[j]) * w)
        };
        *o = [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]].map(|v| v as f32);
    }
    out
}

impl Builder<'_> {
    /// A pooled texture kept alive until the frame is submitted.
    pub fn tex(&mut self, size: [u32; 2]) -> Arc<Tex> {
        let t = self.pool.get(self.device, self.bgl1, [size[0].max(1), size[1].max(1)]);
        self.temps.push(t.clone());
        t
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &mut self,
        entry: Entry,
        i: [u32; 4],
        v: [[f32; 4]; 8],
        src: &Arc<Tex>,
        aux: Aux,
        aux2: Option<Arc<Tex>>,
        lut: Option<Arc<Lut3>>,
        size: [u32; 2],
    ) -> Arc<Tex> {
        let out = self.tex(size);
        let mut i = i;
        i[3] = self.store;
        self.passes.push(Pass {
            entry,
            params: Params { v, i },
            src: src.clone(),
            aux,
            aux2,
            lut,
            out: out.clone(),
            additive: false,
            clear: true,
            custom: None,
        });
        out
    }

    fn simple(&mut self, entry: Entry, op: u32, v: [[f32; 4]; 8], src: &Arc<Tex>, aux: Aux) -> Arc<Tex> {
        let size = src.size;
        self.run(entry, [op, 0, 0, 0], v, src, aux, None, None, size)
    }

    /// Dual-Kawase approximation of a Gaussian blur of standard deviation `sigma` texels.
    pub fn blur(&mut self, src: &Arc<Tex>, sigma: f64) -> Arc<Tex> {
        if sigma < 0.375 {
            return src.clone();
        }
        // σ of the pyramid ≈ 1.45 · offset · 2^levels (measured on an impulse);
        // pick the fewest levels keeping the offset ≤ 1.5 texels
        let x = sigma / 1.45;
        let levels = ((x / 1.5).log2().ceil() as i32).clamp(1, 10);
        let off = (x / 2f64.powi(levels)).clamp(0.2, 3.0) as f32;
        let mut chain = vec![src.clone()];
        let mut cur = src.clone();
        for _ in 0..levels {
            let size = [(cur.size[0] / 2).max(1), (cur.size[1] / 2).max(1)];
            let mut v = [[0.0; 4]; 8];
            v[0][0] = off;
            cur = self.run(Entry::Down, [0; 4], v, &cur, Aux::None, None, None, size);
            chain.push(cur.clone());
        }
        for k in (0..levels as usize).rev() {
            let size = chain[k].size;
            let mut v = [[0.0; 4]; 8];
            v[0][0] = off;
            cur = self.run(Entry::Up, [0; 4], v, &cur, Aux::None, None, None, size);
        }
        cur
    }

    fn combine(&mut self, mode: u32, s: &Arc<Tex>, p: &Arc<Tex>, k: f64, tint: [f64; 3], extra: f64) -> Arc<Tex> {
        let mut v = [[0.0; 4]; 8];
        v[0] = [k as f32, extra as f32, 0.0, 0.0];
        v[1] = [tint[0] as f32, tint[1] as f32, tint[2] as f32, 1.0];
        self.simple(Entry::Combine, mode, v, s, Aux::Tex(p.clone()))
    }

    /// Applies one effect element; returns the new image.
    pub fn effect(&mut self, e: &dyn Element, a: &Attrs, input: &Arc<Tex>, cx: &Cx) -> Result<Arc<Tex>, String> {
        let kind = a.str("type").unwrap_or_default();
        if a.num("enabled", 1.0) == 0.0 {
            return Ok(input.clone());
        }
        let mixv = a.num("mix", 1.0).clamp(0.0, 1.0);
        let out = self.effect_body(&kind, e, a, input, cx)?;
        if mixv < 1.0 - 1e-9 {
            return Ok(self.combine(0, input, &out, mixv, [1.0; 3], 0.0));
        }
        Ok(out)
    }

    fn effect_body(
        &mut self,
        kind: &str,
        e: &dyn Element,
        a: &Attrs,
        input: &Arc<Tex>,
        cx: &Cx,
    ) -> Result<Arc<Tex>, String> {
        let px = cx.px;
        let size = input.size;
        let (w, h) = (size[0] as f64, size[1] as f64);
        let r = a.num("radius", 4.0) * px;
        let intensity = a.num("intensity", 1.0);
        let threshold = a.num("threshold", 0.7);
        let amount = a.num("amount", 1.0);
        let sz = a.num("size", 1.0);
        let angle = a.num("angle", 0.0);
        let seed = a.opt("seed").unwrap_or(0.0) as f32 % 9973.0;
        let t = cx.time;
        let speed = a.num("speed", 1.0);
        let lin = |c: [f64; 4]| {
            let l = cx.working.to_linear([c[0], c[1], c[2]]);
            [l[0], l[1], l[2], c[3]]
        };
        let colour = |name: &str, d: [f64; 4]| a.paint(name).and_then(|v| (cx.color)(&v)).unwrap_or(d);
        let center = {
            let (cxp, cyp) = (a.opt("centerX"), a.opt("centerY"));
            match (cxp, cyp) {
                (None, None) => cx.center,
                _ => {
                    let d = cx.center;
                    let uv = (cx.to_uv)([cxp.unwrap_or(0.0), cyp.unwrap_or(0.0)]);
                    [if cxp.is_some() { uv[0] } else { d[0] }, if cyp.is_some() { uv[1] } else { d[1] }]
                }
            }
        };
        let dirv = |len: f64, ang: f64| -> [f32; 2] {
            let (dx, dy) = (ang.to_radians().cos() * len, ang.to_radians().sin() * len);
            let l = cx.lin;
            [(l[0] * dx + l[2] * dy) as f32, (l[1] * dx + l[3] * dy) as f32]
        };
        let offset_uv = || -> [f32; 2] {
            let (ox, oy) = (a.num("offsetX", 8.0), a.num("offsetY", 8.0));
            let l = cx.lin;
            [((l[0] * ox + l[2] * oy) / w) as f32, ((l[1] * ox + l[3] * oy) / h) as f32]
        };
        let big = |r: f64| if r <= 4.0 * px + 1e-9 { w.min(h) * 0.5 } else { r };
        let mut v = [[0.0f32; 4]; 8];
        // seeded ops read the effect's hash seed from i.yz
        let es = effect_seed(cx.seed, e, a.opt("seed"));
        let ids = |op: u32| [op, es as u32, (es >> 32) as u32, 0];
        let color_op = |b: &mut Self, op: u32, v: [[f32; 4]; 8], aux: Aux| {
            b.run(Entry::Color, ids(op), v, input, aux, None, None, input.size)
        };
        Ok(match kind {
            // separable Gaussian (σ = radius), edges transparent
            "blur" => self.gauss(input, r),
            "glow" | "bloom" | "halation" => {
                let tint = match kind {
                    "halation" => lin(colour("color", [1.0, 0.3, 0.12, 1.0])),
                    _ => lin(colour("color", [1.0; 4])),
                };
                // glow keeps the part of each pixel above `threshold` of its working-space luminance
                v[0] = [threshold as f32, if kind == "glow" { 0.0 } else { 0.5 }, 0.0, 0.0];
                v[1] = [tint[0] as f32, tint[1] as f32, tint[2] as f32, 1.0];
                let bright = self.simple(Entry::Pre, 0, v, input, Aux::None);
                // radius is the glow's standard deviation, as the blur effect's
                let blurred = self.blur(&bright, r.max(0.5));
                // the bloom is added over the content
                self.combine(1, input, &blurred, intensity, [1.0; 3], 0.0)
            }
            "drop-shadow" | "inner-shadow" | "inner-glow" => {
                let c = colour("color", if kind == "inner-glow" { [1.0, 1.0, 0.8, 1.0] } else { [0.0, 0.0, 0.0, 1.0] });
                let off = if kind == "inner-glow" { [0.0; 2] } else { offset_uv() };
                v[0] = [0.0, 0.0, off[0], off[1]];
                v[1] = v4(c);
                let pre = self.simple(Entry::Pre, if kind == "drop-shadow" { 1 } else { 2 }, v, input, Aux::None);
                // a drop shadow's radius is twice the standard deviation (CSS drop-shadow());
                // the inner styles' radius is the standard deviation, as the blur effect's
                let b = self.blur(&pre, if kind == "drop-shadow" { r * 0.5 } else { r });
                // a drop shadow goes behind the content by default; the inner styles
                // are drawn over it
                let mode = if kind == "drop-shadow" {
                    match a.str("compositeOriginal").as_deref() {
                        Some("on-top") => 3,
                        Some("none") => 4,
                        _ => 2,
                    }
                } else {
                    5
                };
                self.combine(mode, input, &b, intensity, [1.0; 3], 0.0)
            }
            "directional-blur" => {
                let d = dirv(r * 2.0, angle);
                v[0] = [d[0], d[1], a.num("samples", 16.0).clamp(1.0, 256.0) as f32, 0.0];
                self.simple(Entry::Line, 0, v, input, Aux::None)
            }
            "radial-blur" => {
                let spin = if angle != 0.0 { angle } else { amount * 10.0 };
                v[0] = [spin.to_radians() as f32, 0.0, a.num("samples", 16.0).clamp(1.0, 256.0) as f32, 0.0];
                v[2] = [center[0] as f32, center[1] as f32, 0.0, 0.0];
                self.simple(Entry::Line, 1, v, input, Aux::None)
            }
            "zoom-blur" => {
                v[0] = [(amount * 0.1) as f32, 0.0, a.num("samples", 16.0).clamp(1.0, 256.0) as f32, 0.0];
                v[2] = [center[0] as f32, center[1] as f32, 0.0, 0.0];
                self.simple(Entry::Line, 2, v, input, Aux::None)
            }
            "god-rays" => {
                v[0] = [threshold as f32, 0.2, 0.0, 0.0];
                v[1] = [1.0; 4];
                let bright = self.simple(Entry::Pre, 0, v, input, Aux::None);
                let mut v2 = [[0.0f32; 4]; 8];
                v2[0] =
                    [(amount * 0.5).clamp(0.0, 1.0) as f32, 0.0, a.num("samples", 16.0).clamp(4.0, 128.0) as f32, 0.97];
                v2[1] = [intensity as f32, 0.0, 0.0, 0.0];
                v2[2] = [center[0] as f32, center[1] as f32, 0.0, 0.0];
                self.simple(Entry::Line, 3, v2, input, Aux::Tex(bright))
            }
            "long-shadow" => {
                let len = if sz > 1.0 { sz * px } else { 60.0 * px };
                let n = len.clamp(1.0, 128.0);
                let d = dirv(len / n / px, angle);
                let c = colour("color", [0.0, 0.0, 0.0, 0.6]);
                v[0] = [d[0] * px as f32, d[1] * px as f32, n as f32, 0.0];
                v[1] = [c[0] as f32, c[1] as f32, c[2] as f32, (1.0 - c[3]) as f32];
                self.simple(Entry::Line, 4, v, input, Aux::None)
            }
            "lens-blur" | "tilt-shift" => {
                let blades = a.num("levels", 8.0);
                let rings = (a.num("samples", 16.0) / 3.0).sqrt().ceil().clamp(1.0, 6.0);
                v[0] = [
                    r as f32,
                    if blades >= 3.0 { blades as f32 } else { 0.0 },
                    angle.to_radians() as f32,
                    rings as f32,
                ];
                v[1] = [threshold as f32, (2.0 * amount) as f32, 0.0, 0.0];
                if kind == "tilt-shift" {
                    let band = if sz <= 1.0 { 0.1 } else { sz * px / h * 0.5 };
                    v[2] = [center[1] as f32, band as f32, angle.to_radians() as f32, 0.0];
                    v[0][2] = 0.0;
                }
                self.simple(Entry::Bokeh, (kind == "tilt-shift") as u32, v, input, Aux::None)
            }
            "color-grade" => {
                v[0] = [
                    a.num("saturation", 1.0) as f32,
                    a.num("contrast", 1.0) as f32,
                    a.num("brightness", 0.0) as f32,
                    0.0,
                ];
                color_op(self, 1, v, Aux::None)
            }
            "lift-gamma-gain" => {
                let (l, g, gn) = (triple(a.str("lift"), 0.0), triple(a.str("gamma"), 1.0), triple(a.str("gain"), 1.0));
                v[0] = [l[0] as f32, l[1] as f32, l[2] as f32, 0.0];
                v[1] = [g[0] as f32, g[1] as f32, g[2] as f32, 0.0];
                v[2] = [gn[0] as f32, gn[1] as f32, gn[2] as f32, 0.0];
                color_op(self, 2, v, Aux::None)
            }
            "cdl" => {
                let (s, o, p) =
                    (triple(a.str("slope"), 1.0), triple(a.str("offset"), 0.0), triple(a.str("power"), 1.0));
                v[0] = [s[0] as f32, s[1] as f32, s[2] as f32, 0.0];
                v[1] = [o[0] as f32, o[1] as f32, o[2] as f32, 0.0];
                v[2] = [p[0] as f32, p[1] as f32, p[2] as f32, 0.0];
                v[3] = [a.num("saturation", 1.0) as f32, 0.0, 0.0, 0.0];
                color_op(self, 3, v, Aux::None)
            }
            "lut" => {
                let src = a.str("src").ok_or("lut needs @src")?;
                let path = match sr_model::assets::resolve(&src, cx.base) {
                    sr_model::assets::Resolved::Local(p) => p,
                    _ => return Err(format!("{src}: only local LUT files are supported")),
                };
                let lut = self.eng.lut(&path)?;
                let space = a
                    .str("space")
                    .and_then(|s| m::ColorSpace::ALL.iter().copied().find(|c| c.as_str() == s))
                    .unwrap_or(cx.working.space);
                self.lut_pass(input, lut, space, 1.0, cx.working)
            }
            "curves" => {
                let tbl = curve_table(&a.str("curve").unwrap_or_default())
                    .ok_or("curves needs @curve with at least two \"x,y\" points")?;
                let ch = match a.str("channel").as_deref() {
                    Some("red") => 1,
                    Some("green") => 2,
                    Some("blue") => 3,
                    Some("alpha") => 4,
                    Some("luma") => 5,
                    _ => 0,
                };
                let mut px256 = [[0.0f32; 4]; 256];
                for (k, p) in px256.iter_mut().enumerate() {
                    let id = k as f32 / 255.0;
                    *p = [id, id, id, tbl[k]];
                    if (1..=3).contains(&ch) {
                        p[ch - 1] = tbl[k];
                    }
                }
                if ch == 4 {
                    self.problems.push("curves on the alpha channel apply to the master curve".into());
                }
                let t = self.eng.table(self.pool, self.bgl1, &px256);
                self.temps.push(t.clone());
                v[0] = [if ch == 4 { 0.0 } else { ch as f32 }, 0.0, 0.0, 0.0];
                color_op(self, 4, v, Aux::Tex(t))
            }
            "levels" => {
                let ch = match a.str("channel").as_deref() {
                    Some("red") => 1.0,
                    Some("green") => 2.0,
                    Some("blue") => 3.0,
                    _ => 0.0,
                };
                v[0] = [
                    a.num("inputBlack", 0.0) as f32,
                    a.num("inputWhite", 1.0) as f32,
                    triple(a.str("gamma"), 1.0)[0] as f32,
                    0.0,
                ];
                v[1] = [a.num("outputBlack", 0.0) as f32, a.num("outputWhite", 1.0) as f32, ch, 0.0];
                color_op(self, 5, v, Aux::None)
            }
            "white-balance" => {
                v[0] = [a.num("temperature", 0.0) as f32, a.num("tint", 0.0) as f32, 0.0, 0.0];
                color_op(self, 6, v, Aux::None)
            }
            "exposure" => {
                v[0] = [a.num("exposure", 0.0) as f32, 0.0, 0.0, 0.0];
                color_op(self, 7, v, Aux::None)
            }
            "hue-saturation" => {
                v[0] =
                    [a.num("hue", 0.0) as f32, a.num("saturation", 1.0) as f32, a.num("brightness", 0.0) as f32, 0.0];
                color_op(self, 8, v, Aux::None)
            }
            "tonemap" => {
                let op = ["aces", "agx", "filmic", "reinhard", "hable", "pq-to-sdr"]
                    .iter()
                    .position(|x| Some(*x) == a.str("tonemapper").as_deref())
                    .unwrap_or(0);
                v[0] = [a.num("exposure", 0.0) as f32, op as f32, 0.0, 0.0];
                color_op(self, 9, v, Aux::None)
            }
            "tint" | "tritone" | "color-overlay" | "fill" => {
                let c = lin(colour("color", if kind == "tint" { [1.0, 0.8, 0.6, 1.0] } else { [1.0, 0.5, 0.2, 1.0] }));
                let amt = if kind == "fill" {
                    c[3]
                } else {
                    amount.clamp(0.0, 1.0) * if kind == "color-overlay" { c[3] } else { 1.0 }
                };
                v[0] = [amt as f32, 0.0, 0.0, 0.0];
                if kind == "tritone" {
                    v[1] = [0.0, 0.0, 0.0, 1.0];
                    v[2] = v4(c);
                    v[3] = [1.0; 4];
                    color_op(self, 11, v, Aux::None)
                } else {
                    v[1] = v4(c);
                    color_op(self, if kind == "tint" { 10 } else { 18 }, v, Aux::None)
                }
            }
            "gradient-map" | "gradient-overlay" => {
                let stops = a
                    .paint("paint")
                    .and_then(|p| (cx.gradient)(&p))
                    .or_else(|| a.str("source").and_then(|s| (cx.gradient)(&Value::PaintRef(s.into()))))
                    .unwrap_or_default();
                let stops: Vec<(f64, [f64; 4])> = stops.into_iter().map(|(o, c)| (o, lin(c))).collect();
                if stops.is_empty() && kind == "gradient-overlay" {
                    return Err("gradient-overlay needs @paint (a gradient)".into());
                }
                let t = self.eng.table(self.pool, self.bgl1, &gradient_table(&stops));
                self.temps.push(t.clone());
                v[0] = [amount.clamp(0.0, 1.0) as f32, angle as f32, 0.0, 0.0];
                color_op(self, if kind == "gradient-map" { 12 } else { 19 }, v, Aux::Tex(t))
            }
            "grayscale" | "sepia" => {
                v[0] = [amount.clamp(0.0, 1.0) as f32, 0.0, 0.0, 0.0];
                color_op(self, if kind == "grayscale" { 13 } else { 14 }, v, Aux::None)
            }
            "invert" => {
                let ch = match a.str("channel").as_deref() {
                    Some("red") => 1.0,
                    Some("green") => 2.0,
                    Some("blue") => 3.0,
                    Some("luma") => 5.0,
                    _ => 0.0,
                };
                v[0] = [amount.clamp(0.0, 1.0) as f32, ch, 0.0, 0.0];
                color_op(self, 15, v, Aux::None)
            }
            "posterize" => {
                v[0] = [a.num("levels", 8.0).max(2.0) as f32, 0.0, 0.0, 0.0];
                color_op(self, 16, v, Aux::None)
            }
            "threshold" => {
                v[0] = [threshold as f32, 0.0, 0.0, 0.0];
                color_op(self, 17, v, Aux::None)
            }
            "selective-color" => {
                v[0] = [
                    a.num("hue", 0.0) as f32,
                    a.num("tolerance", 0.2) as f32,
                    a.num("saturation", 1.0) as f32,
                    a.num("brightness", 0.0) as f32,
                ];
                color_op(self, 20, v, Aux::None)
            }
            "film-grain" => {
                // grain: seeded-hash normals per sample, drawn from the
                // effect's seed with the frame as the channel, blurred by σ = (size − 1) / 2 px,
                // weighted by (4v(1 − v))^response per channel in linear light
                let sigma = ((sz * px - 1.0) / 2.0).max(0.0);
                let response = param(e, a, "response", 0.5).max(0.01);
                let strength = ["red", "green", "blue"].map(|c| param(e, a, c, 1.0) as f32);
                let frame = cx.frame.max(0) as f32;
                v[0] = [(0.05 * amount) as f32, 0.0, frame, 0.0];
                v[1] = [strength[0], strength[1], strength[2], response as f32];
                let ids = [21, es as u32, (es >> 32) as u32, 0];
                if sigma < 0.4 {
                    self.run(Entry::Color, ids, v, input, Aux::None, None, None, input.size)
                } else {
                    let field =
                        self.run(Entry::Color, [75, ids[1], ids[2], 0], v, input, Aux::None, None, None, input.size);
                    let blurred = self.gauss(&field, sigma);
                    v[0][3] = (2.0 * std::f64::consts::PI.sqrt() * sigma).max(1.0) as f32;
                    self.run(Entry::Color, ids, v, input, Aux::Tex(blurred), None, None, input.size)
                }
            }
            "noise" => {
                // seeded-hash uniforms in [-1, 1) per sample on the frame's channel, times amount, in display values
                let mono = a.str("channel").is_some_and(|c| c != "rgb") || a.num("saturation", 1.0) == 0.0;
                v[0] = [amount as f32, cx.frame.max(0) as f32, if mono { 1.0 } else { 3.0 }, 0.0];
                color_op(self, 22, v, Aux::None)
            }
            "spill-suppress" => {
                let k = lin(colour("keyColor", [0.0, 1.0, 0.0, 1.0]));
                v[0] = [a.num("spill", 0.5) as f32, 0.0, 0.0, 0.0];
                v[1] = v4(k);
                color_op(self, 23, v, Aux::None)
            }
            "vignette" => {
                // darkening 1 − amount · smoothstep(r₀, r₀ + softness, r), r the
                // distance from the centre over the half diagonal and r₀ = radius / half diagonal; absent
                // attributes take the schema's effect defaults (amount 1, radius 4, softness 0.1)
                let c = colour("color", [0.0, 0.0, 0.0, 1.0]);
                let r0 = r / (0.5 * w.hypot(h));
                let soft = a.num("softness", 0.1).max(0.0);
                v[0] = [(amount * c[3]) as f32, r0 as f32, soft as f32, 0.0];
                v[1] = v4(c);
                v[2] = [center[0] as f32, center[1] as f32, 0.0, 0.0];
                color_op(self, 64, v, Aux::None)
            }
            "letterbox" => {
                let c = colour("color", [0.0, 0.0, 0.0, 1.0]);
                v[0] = [if sz > 1.01 { sz } else { 2.39 } as f32, 0.0, 0.0, 0.0];
                v[1] = v4(c);
                color_op(self, 65, v, Aux::None)
            }
            "scanlines" => {
                let period = sz.max(1.0) * 4.0 * px;
                v[0] = [(amount * 0.5).clamp(0.0, 1.0) as f32, period as f32, (period * 0.5) as f32, 0.0];
                color_op(self, 66, v, Aux::None)
            }
            "light-leak" => {
                let c = colour("color", [1.0, 0.55, 0.2, 1.0]);
                v[0] = [intensity as f32, (t * speed) as f32, seed, 0.0];
                v[1] = v4(c);
                color_op(self, 67, v, Aux::None)
            }
            "light-sweep" => {
                let c = colour("color", [1.0; 4]);
                let pos = (t * speed * 0.5).rem_euclid(1.0) * 1.6 - 0.3;
                v[0] = [
                    intensity as f32 * 0.8,
                    pos as f32,
                    if sz > 1.0 { (sz * px / w) as f32 } else { 0.08 },
                    angle as f32 + 30.0,
                ];
                v[1] = v4(c);
                color_op(self, 68, v, Aux::None)
            }
            "lens-flare" => {
                let c = colour("color", [1.0, 0.9, 0.7, 1.0]);
                let src = if a.opt("centerX").is_some() || a.opt("centerY").is_some() { center } else { [0.25, 0.25] };
                v[0] = [intensity as f32, sz.max(0.1) as f32, 0.0, 0.0];
                v[1] = v4(c);
                v[2] = [src[0] as f32, src[1] as f32, 0.0, 0.0];
                color_op(self, 69, v, Aux::None)
            }
            "fractal-noise" => {
                v[0] = [
                    (if sz > 1.0 { sz } else { 64.0 } * px) as f32,
                    a.num("frequency", 1.0) as f32,
                    (t * speed) as f32,
                    seed,
                ];
                v[1] = [intensity as f32, amount.clamp(0.0, 1.0) as f32, 0.0, 0.0];
                color_op(self, 70, v, Aux::None)
            }
            "chroma-key" => {
                let k = lin(colour("keyColor", [0.0, 1.0, 0.0, 1.0]));
                v[0] = [a.num("tolerance", 0.2) as f32, a.num("softness", 0.1) as f32, a.num("spill", 0.5) as f32, 0.0];
                v[1] = v4(k);
                color_op(self, 71, v, Aux::None)
            }
            "luma-key" => {
                v[0] = [threshold as f32, a.num("softness", 0.1) as f32, (amount < 0.0) as u8 as f32, 0.0];
                color_op(self, 72, v, Aux::None)
            }
            "difference-key" => {
                let plate = cx.source.clone().ok_or("difference-key needs @source (the clean plate)")?;
                v[0] = [a.num("tolerance", 0.2) as f32, a.num("softness", 0.1) as f32, 0.0, 0.0];
                color_op(self, 73, v, Aux::Tex(plate))
            }
            "halftone" => {
                v[0] = [
                    (if sz >= 2.0 { sz } else { 8.0 } * px) as f32,
                    if angle != 0.0 { angle } else { 45.0 } as f32,
                    amount.clamp(0.0, 1.0) as f32,
                    0.0,
                ];
                color_op(self, 74, v, Aux::None)
            }
            "glitch" => {
                // glitch: seeded 64-bit hash draws of the effect's seed
                // on the frame's channel — rows shifted sideways, then blocks copied from the
                // input, channel delay and quantisation
                let amount_px = amount * px;
                if amount_px == 0.0 {
                    return Ok(input.clone());
                }
                let frame = cx.frame.max(0) as u64;
                let (wi, hi) = (input.size[0] as f64, input.size[1] as f64);
                let size_px = sz * px;
                let rows_h = (size_px.round_ties_even()).max(1.0);
                let count = (param(e, a, "blocks", 8.0).round_ties_even().max(1.0) as usize).min(256);
                let mut n = 0u64;
                let mut draw = || {
                    let u = sr_eval::rng::d24_unit(es, frame, n);
                    n += 1;
                    u
                };
                let mut texels = Vec::with_capacity(count * 3);
                let split = |v: f64| [(v as u32 >> 8) as f32, (v as u32 & 255) as f32];
                for _ in 0..count {
                    let bw = (0.03 + 0.17 * draw()) * wi;
                    let bw = bw.round_ties_even().min(wi).max(1.0);
                    let bh = (size_px * (1.0 + 3.0 * draw())).round_ties_even().min(hi).max(1.0);
                    let mut int = |hi_excl: f64| (hi_excl * draw()).floor();
                    let (x, y) = (int(wi - bw + 1.0), int(hi - bh + 1.0));
                    let (sx, sy) = (int(wi - bw + 1.0), int(hi - bh + 1.0));
                    let [a0, a1] = split(x);
                    let [b0, b1] = split(y);
                    let [c0, c1] = split(bw);
                    let [d0, d1] = split(bh);
                    let [e0, e1] = split(sx);
                    let [f0, f1] = split(sy);
                    texels.extend([[a0, a1, b0, b1], [c0, c1, d0, d1], [e0, e1, f0, f1]]);
                }
                let table = self.eng.data_table(self.pool, self.bgl1, &texels);
                self.temps.push(table.clone());
                let weight = amount.abs().min(1.0);
                v[0] = [amount_px as f32, rows_h as f32, weight as f32, count as f32];
                v[1] = [frame as f32, (hi / rows_h).ceil() as f32, 0.0, 0.0];
                let ids = [76, es as u32, (es >> 32) as u32, 0];
                let moved = self.run(Entry::Color, ids, v, input, Aux::Tex(table), None, None, input.size);
                v[0] = [
                    (amount_px * param(e, a, "channelDelay", 0.5)) as f32,
                    param(e, a, "quantization", 32.0).round_ties_even().max(2.0) as f32,
                    0.0,
                    0.0,
                ];
                self.run(Entry::Color, [77, 0, 0, 0], v, &moved, Aux::None, None, None, input.size)
            }
            "displacement-map"
            | "turbulent-displace"
            | "wave-warp"
            | "ripple"
            | "twirl"
            | "spherize"
            | "bulge"
            | "lens-distortion"
            | "heat-haze"
            | "mirror"
            | "kaleidoscope"
            | "tile"
            | "pixelate"
            | "mosaic"
            | "chromatic-aberration"
            | "rgb-split"
            | "vhs" => {
                v[2] = [center[0] as f32, center[1] as f32, 0.0, 0.0];
                v[7] = [t as f32, 0.0, 0.0, 0.0];
                let mut aux = Aux::None;
                let op = match kind {
                    "displacement-map" => {
                        aux = Aux::Tex(cx.source.clone().ok_or("displacement-map needs @source")?);
                        v[0] = [(amount * px) as f32, 0.0, 0.0, 0.0];
                        0
                    }
                    "turbulent-displace" => {
                        v[0] = [
                            (amount * px) as f32,
                            (if sz > 1.0 { sz } else { 64.0 } * px) as f32,
                            (t * speed) as f32,
                            seed,
                        ];
                        1
                    }
                    "wave-warp" => {
                        v[0] = [
                            (amount * px) as f32,
                            (if sz > 1.0 { sz } else { 100.0 } * px) as f32,
                            (t * speed * a.num("frequency", 1.0) * std::f64::consts::TAU) as f32,
                            angle as f32,
                        ];
                        2
                    }
                    "ripple" => {
                        v[0] = [
                            (amount * px) as f32,
                            (if sz > 1.0 { sz } else { 40.0 } * px) as f32,
                            (t * speed * std::f64::consts::TAU) as f32,
                            0.0,
                        ];
                        3
                    }
                    "twirl" => {
                        v[0] = [if angle != 0.0 { angle } else { amount * 90.0 } as f32, big(r) as f32, 0.0, 0.0];
                        4
                    }
                    "spherize" | "bulge" => {
                        v[0] = [amount as f32, big(r) as f32, 0.0, 0.0];
                        if kind == "spherize" {
                            5
                        } else {
                            6
                        }
                    }
                    "lens-distortion" => {
                        v[0] = [(amount * 0.2) as f32, 0.0, 0.0, 0.0];
                        7
                    }
                    "heat-haze" => {
                        v[0] = [
                            (amount * 4.0 * px) as f32,
                            (if sz > 1.0 { sz } else { 24.0 } * px) as f32,
                            (t * speed) as f32,
                            0.0,
                        ];
                        8
                    }
                    "mirror" => {
                        v[0] = [angle as f32, 0.0, 0.0, 0.0];
                        9
                    }
                    "kaleidoscope" => {
                        v[0] = [a.num("levels", 8.0).max(1.0) as f32, angle as f32, 0.0, 0.0];
                        10
                    }
                    "tile" => {
                        v[0] = [amount.max(1.0) as f32, (sz > 1.0) as u8 as f32, 0.0, 0.0];
                        11
                    }
                    "pixelate" | "mosaic" => {
                        v[0] = [(if sz > 1.0 { sz } else { 16.0 } * px) as f32, 0.0, 0.0, 0.0];
                        if kind == "pixelate" {
                            12
                        } else {
                            13
                        }
                    }
                    "chromatic-aberration" => {
                        // `amount` pixels at the input's farthest corner from its centre;
                        // the input is the node's content box (the frame for adjustment layers)
                        let b = cx.content.unwrap_or([0.0, 0.0, w, h]);
                        let c = match (a.opt("centerX"), a.opt("centerY")) {
                            (None, None) => [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5],
                            _ => [center[0] * w, center[1] * h],
                        };
                        let reach = (c[0] - b[0]).hypot(c[1] - b[1]).max((b[2] - c[0]).hypot(b[3] - c[1])).max(1.0);
                        v[0] = [(amount * px) as f32, 0.0, 0.0, 0.0];
                        v[1] = [c[0] as f32, c[1] as f32, reach as f32, 0.0];
                        14
                    }
                    "rgb-split" => {
                        let o = offset_uv();
                        v[0] = [o[0] * w as f32, o[1] * h as f32, 0.0, 0.0];
                        15
                    }
                    _ => {
                        v[0] = [amount.clamp(0.0, 2.0) as f32, t as f32, cx.frame.max(0) as f32, 0.0];
                        17
                    }
                };
                self.run(Entry::Warp, ids(op), v, input, aux, None, None, input.size)
            }
            "sharpen" => {
                v[0] = [amount as f32, 0.0, 0.0, 0.0];
                self.simple(Entry::Conv, 0, v, input, Aux::None)
            }
            "unsharp-mask" => {
                let b = self.blur(input, r);
                let th = if (0.7 - 1e-9..=0.7 + 1e-9).contains(&threshold) { 0.0 } else { threshold };
                self.combine(6, input, &b, amount, [1.0; 3], th)
            }
            "emboss" => {
                v[0] = [
                    angle.to_radians() as f32,
                    (a.num("relief", 0.0).max(1.0) * px) as f32,
                    amount.clamp(0.0, 1.0) as f32,
                    0.0,
                ];
                self.simple(Entry::Conv, 1, v, input, Aux::None)
            }
            "bevel" => {
                v[0] = [
                    if angle != 0.0 { angle } else { -45.0 }.to_radians() as f32,
                    (sz.max(2.0) * px) as f32,
                    (intensity * 0.5) as f32,
                    0.0,
                ];
                self.simple(Entry::Conv, 2, v, input, Aux::None)
            }
            "lighting" => {
                let c = lin(colour("color", [1.0; 4]));
                v[0] = [
                    if angle != 0.0 { angle } else { -45.0 }.to_radians() as f32,
                    a.num("relief", 0.0).max(0.2) as f32,
                    intensity.clamp(0.0, 1.0) as f32,
                    0.0,
                ];
                v[1] = v4(c);
                for (k, l) in cx.lights.iter().take(4).enumerate() {
                    v[3 + k] = *l;
                }
                self.simple(Entry::Conv, 3, v, input, Aux::None)
            }
            "stroke" | "outline" | "matte-choke" => {
                if kind == "matte-choke" {
                    let choke = amount * px;
                    v[0] = [choke.abs() as f32, (choke > 0.0) as u8 as f32, 0.0, 0.0];
                    let o = self.simple(Entry::Morph, 0, v, input, Aux::None);
                    let soft = a.num("softness", 0.1) * px * 4.0;
                    return Ok(if soft >= 1.0 { self.blur(&o, soft * 0.5) } else { o });
                }
                let c = colour("color", [1.0; 4]);
                let pos = match a.str("position").as_deref() {
                    Some("inside") => 1.0,
                    Some("center") => 2.0,
                    _ => 0.0,
                };
                v[0] = [(sz.max(1.0) * px) as f32, 0.0, 0.0, pos];
                v[1] = v4(c);
                self.simple(Entry::Morph, if kind == "stroke" { 1 } else { 2 }, v, input, Aux::None)
            }
            "shader" => return self.shader_effect(e, a, input, cx),
            other => return Err(format!("effect type {other} is not drawn")),
        })
    }

    /// A LUT pass in `space` (working ↔ LUT space through matrices and the space's transfer).
    pub fn lut_pass(
        &mut self,
        input: &Arc<Tex>,
        lut: Arc<Lut3>,
        space: m::ColorSpace,
        mixv: f64,
        working: Working,
    ) -> Arc<Tex> {
        let mm = color::convert(working.space, space);
        let mi = color::convert(space, working.space);
        let tf = color::transfer_id(color::resolve(space, m::Transfer::Auto));
        let mut v = [[0.0f32; 4]; 8];
        v[0] = [mixv as f32, lut.size as f32, 0.0, 0.0];
        for r in 0..3 {
            v[4 + r] = [mm[r][0] as f32, mm[r][1] as f32, mm[r][2] as f32, mi[0][r] as f32];
        }
        v[1] = [mi[1][0] as f32, mi[1][1] as f32, mi[1][2] as f32, 0.0];
        v[2] = [mi[2][0] as f32, mi[2][1] as f32, mi[2][2] as f32, 0.0];
        v[7] = [tf as f32, 0.0, 0.0, 0.0];
        let size = input.size;
        self.run(Entry::Color, [24, 0, 0, 0], v, input, Aux::None, None, Some(lut), size)
    }

    /// A LUT indexed by `input_transfer` of each working channel whose entries are code values
    /// of `output_space` and `output_transfer` (a display transform): encodes, looks up and
    /// decodes the result back to working colour.
    pub fn lut_pass_between(
        &mut self,
        input: &Arc<Tex>,
        lut: Arc<Lut3>,
        input_transfer: m::Transfer,
        output_space: m::ColorSpace,
        output_transfer: m::Transfer,
        working: Working,
    ) -> Arc<Tex> {
        let mi = color::convert(output_space, working.space);
        let tf = color::transfer_id(input_transfer);
        let tf_out = color::transfer_id(color::resolve(output_space, output_transfer));
        let mut v = [[0.0f32; 4]; 8];
        v[0] = [1.0, lut.size as f32, 0.0, 0.0];
        for r in 0..3 {
            let mut row = [0.0f32; 3];
            row[r] = 1.0;
            v[4 + r] = [row[0], row[1], row[2], mi[0][r] as f32];
        }
        v[1] = [mi[1][0] as f32, mi[1][1] as f32, mi[1][2] as f32, 0.0];
        v[2] = [mi[2][0] as f32, mi[2][1] as f32, mi[2][2] as f32, 0.0];
        v[7] = [tf as f32, 0.0, tf_out as f32 + 1.0, 0.0];
        let size = input.size;
        self.run(Entry::Color, [24, 0, 0, 0], v, input, Aux::None, None, Some(lut), size)
    }

    /// A per-pixel colour op on `input` (finishing: CDL looks, exposure, tone mapping).
    pub fn color(&mut self, input: &Arc<Tex>, op: u32, v: [[f32; 4]; 8]) -> Arc<Tex> {
        self.simple(Entry::Color, op, v, input, Aux::None)
    }

    /// Accumulates `src` into `acc` with weight `k` (motion blur, echo).
    pub fn accumulate(&mut self, acc: &Arc<Tex>, src: &Arc<Tex>, k: f64, first: bool) {
        let mut v = [[0.0f32; 4]; 8];
        v[0][0] = k as f32;
        self.passes.push(Pass {
            entry: Entry::Combine,
            params: Params { v, i: [8, 0, 0, self.store] },
            src: src.clone(),
            aux: Aux::Tex(src.clone()),
            aux2: None,
            lut: None,
            out: acc.clone(),
            additive: true,
            clear: first,
            custom: None,
        });
    }

    /// Flow-guided blur of `src` along `flow` (source texels per frame).
    pub fn flow_blur(
        &mut self,
        src: &Arc<Tex>,
        flow: Arc<std::sync::OnceLock<wgpu::TextureView>>,
        shutter: f64,
        samples: u32,
    ) -> Arc<Tex> {
        let mut v = [[0.0f32; 4]; 8];
        v[0] = [shutter as f32, samples.clamp(1, 64) as f32, 0.0, 0.0];
        self.simple(Entry::Flow, 0, v, src, Aux::Flow(flow))
    }

    /// A transition between two frame-sized images.
    #[allow(clippy::too_many_arguments)]
    pub fn transition(
        &mut self,
        kind: &str,
        from: &Arc<Tex>,
        to: &Arc<Tex>,
        luma: Option<Arc<Tex>>,
        p: f64,
        e: &dyn Element,
        a: &Attrs,
        color: [f64; 4],
        shader: Option<(String, &Path)>,
        velocity: f64,
        cx: Option<&Cx>,
    ) -> Result<Arc<Tex>, String> {
        // a missing side is a 1×1 transparent texture: the frame is the other side's size
        let size = if from.size[0] as u64 * from.size[1] as u64 >= to.size[0] as u64 * to.size[1] as u64 {
            from.size
        } else {
            to.size
        };
        if kind == "shader" {
            let (code, path) = shader.ok_or("transition type=\"shader\" needs @shader")?;
            let cx = cx.ok_or("shader transitions need a context")?;
            return self
                .shader_transition(&code, from, to, luma, p, velocity, e, a, cx)
                .map_err(|er| format!("{}: {er}", path.display()));
        }
        const KINDS: [&str; 34] = [
            "cut",
            "crossfade",
            "additive-dissolve",
            "dip-to-color",
            "wipe",
            "slide",
            "push",
            "cover",
            "reveal",
            "zoom-in",
            "zoom-out",
            "spin",
            "whip-pan",
            "circle-open",
            "circle-close",
            "iris",
            "clock-wipe",
            "radial-wipe",
            "barn-door",
            "blinds",
            "luma",
            "blur",
            "glitch",
            "pixelize",
            "flip",
            "cube",
            "page-curl",
            "film-roll",
            "stripe",
            "squash",
            "shuffle",
            "carousel",
            "light-leak",
            "morph",
        ];
        let k = KINDS.iter().position(|x| *x == kind).ok_or_else(|| format!("transition type {kind} is not drawn"))?;
        let dir = match a.str("direction").as_deref() {
            Some("right") => 1.0,
            Some("up") => 2.0,
            Some("down") => 3.0,
            Some("angle") => 4.0,
            _ => 0.0,
        };
        let mut v = [[0.0f32; 4]; 8];
        v[0] = [p as f32, a.num("softness", 0.1) as f32, a.num("angle", 0.0).to_radians() as f32, dir];
        v[1] = v4(color);
        let (fw, fh) = (size[0] as f64, size[1] as f64);
        let (has_a, has_b) = (from.size != [1, 1] || size == [1, 1], to.size != [1, 1] || size == [1, 1]);
        v[2] = [0.0, has_a as u32 as f32, has_b as u32 as f32, 0.0];
        v[3] = [0.0, 0.0, fw as f32, fh as f32];
        let x = TransitionExtras::new(kind, p, e, a, velocity, cx, [fw, fh], has_b, color);
        v[4..8].copy_from_slice(&x.v);
        let mut aux2 = luma.clone();
        // warp_affine pre-filters shrinking axes: σ = 0.45 / k along each axis with k < 0.6
        let pre = |b: &mut Self, t: &Arc<Tex>, k: [f64; 2]| {
            let s = k.map(|k| if k < 0.6 && k > 0.0 { 0.45 / k } else { 0.0 });
            if s[0] > 0.0 || s[1] > 0.0 {
                b.py_gaussian(t, s[0], s[1], false, false)
            } else {
                t.clone()
            }
        };
        let (from, to) = (&pre(self, from, x.shrink[0]), &pre(self, to, x.shrink[1]));
        match kind {
            "blur" => {
                // blur: both sides blurred (σ peaks at the cut), mixed by m
                let sigma = pparam(e, "amount", 0.03) * fw.max(fh) * (1.0 - (2.0 * p - 1.0).abs());
                let m = sstep(0.3, 0.7, p);
                let fa = if m < 1.0 { self.py_gaussian(from, sigma, sigma, true, true) } else { from.clone() };
                let fb = if m > 0.0 { self.py_gaussian(to, sigma, sigma, true, true) } else { to.clone() };
                v[0][0] = m as f32;
                return Ok(self.run(Entry::Trans, [1, 0, 0, 0], v, &fa, Aux::Tex(fb), None, None, size));
            }
            "pixelize" => {
                // mix, then average over blocks aligned with the frame centre
                let m = sstep(0.4, 0.6, p);
                v[0][0] = m as f32;
                let base = self.run(Entry::Trans, [1, 0, 0, 0], v, from, Aux::Tex(to.clone()), None, None, size);
                let bsf = 1.0 + (pparam(e, "amount", 0.05) * fw.max(fh) - 1.0) * (1.0 - (2.0 * p - 1.0).abs());
                let bs = bsf.round_ties_even() as i64;
                if bs <= 1 {
                    return Ok(base);
                }
                let (w, h) = (size[0] as i64, size[1] as i64);
                let (ox, oy) = ((-(w / 2)).rem_euclid(bs), (-(h / 2)).rem_euclid(bs));
                let blocks = [((ox + w + bs - 1) / bs) as u32, ((oy + h + bs - 1) / bs) as u32];
                v[4] = [bs as f32, ox as f32, oy as f32, 0.0];
                let avg = self.run(Entry::Trans, [34, 0, 0, 0], v, &base, Aux::None, None, None, blocks);
                return Ok(self.run(Entry::Trans, [35, 0, 0, 0], v, &avg, Aux::None, None, None, size));
            }
            "glitch" | "light-leak" if !x.table.is_empty() => {
                let t = self.eng.data_table(self.pool, self.bgl1, &x.table);
                self.temps.push(t.clone());
                aux2 = Some(t);
            }
            _ => {}
        }
        let has_luma = luma.is_some();
        Ok(self.run(Entry::Trans, [k as u32, has_luma as u32, 0, 0], v, from, Aux::Tex(to.clone()), aux2, None, size))
    }
}

impl Builder<'_> {
    /// The effect gaussian: separable, edges transparent;
    /// σ > 4 as three box passes of width ⌊√(4σ² + 1)⌋ (odd), else the
    /// kernel sampled over ±⌈3σ⌉.
    pub fn gauss(&mut self, src: &Arc<Tex>, sigma: f64) -> Arc<Tex> {
        if sigma < 0.3 {
            return src.clone();
        }
        if sigma > 4.0 {
            return self.py_gaussian(src, sigma, sigma, false, false);
        }
        let mut cur = src.clone();
        for axis in [0.0, 1.0] {
            let mut v = [[0.0f32; 4]; 8];
            v[4] = [sigma as f32, axis, 0.0, 0.0];
            cur = self.run(Entry::Trans, [39, 0, 0, 0], v, &cur, Aux::None, None, None, cur.size);
        }
        cur
    }

    /// The transition gaussian: three box passes per
    /// axis of width ⌊√(4σ² + 1)⌋ (odd), edges transparent or extended (`clamp`); an isotropic
    /// σ > 2.5 runs on a copy downsampled by f = min(8, max(2, ⌊σ / 2.5⌋)).
    pub fn py_gaussian(&mut self, src: &Arc<Tex>, sx: f64, sy: f64, clamp: bool, iso: bool) -> Arc<Tex> {
        if iso && sx > 2.5 && src.size[0].min(src.size[1]) > 64 {
            let f = (sx / 2.5).floor().clamp(2.0, 8.0);
            let fi = f as u32;
            let small = [src.size[0].div_ceil(fi), src.size[1].div_ceil(fi)];
            let mut v = [[0.0f32; 4]; 8];
            v[4] = [f as f32, 0.0, 0.0, 0.0];
            let down = self.run(Entry::Trans, [37, 0, 0, 0], v, src, Aux::None, None, None, small);
            let blurred = self.py_gaussian(&down, sx / f, sx / f, clamp, true);
            return self.run(Entry::Trans, [38, 0, 0, 0], v, &blurred, Aux::None, None, None, src.size);
        }
        let mut cur = src.clone();
        for (axis, sg) in [(0.0, sx), (1.0, sy)] {
            if sg < 0.4 {
                continue;
            }
            let mut w = ((12.0 * sg * sg / 3.0 + 1.0).sqrt() as i64).max(1);
            w += (w + 1) % 2;
            let mut v = [[0.0f32; 4]; 8];
            v[4] = [w as f32, axis, clamp as u32 as f32, 0.0];
            for _ in 0..3 {
                cur = self.run(Entry::Trans, [36, 0, 0, 0], v, &cur, Aux::None, None, None, cur.size);
            }
        }
        cur
    }
}

/// Smoothstep with clamping (`transitions.sstep`).
fn sstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0).max(1e-9)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A transition's `<param name value>` (`transitions.param`).
fn pparam(e: &dyn Element, name: &str, default: f64) -> f64 {
    params_of(e).into_iter().find(|p| p.0 == name).and_then(|p| p.1.first().map(|v| *v as f64)).unwrap_or(default)
}

/// The per-type parameters of transitions, in v4..v7,
/// and the data table of the seeded types (in aux2).
struct TransitionExtras {
    v: [[f32; 4]; 4],
    table: Vec<[f32; 4]>,
    /// The column scales (x, y) of the affine warps of a and b (for their pre-filter).
    shrink: [[f64; 2]; 2],
}

impl TransitionExtras {
    #[allow(clippy::too_many_arguments)]
    fn new(
        kind: &str,
        p: f64,
        e: &dyn Element,
        a: &Attrs,
        velocity: f64,
        cx: Option<&Cx>,
        frame: [f64; 2],
        has_b: bool,
        color: [f64; 4],
    ) -> TransitionExtras {
        use std::f64::consts::PI;
        let mut v = [[0.0f32; 4]; 4];
        let mut table = Vec::new();
        let mut shrink = [[1.0; 2]; 2];
        let (w, h) = (frame[0], frame[1]);
        let dname = a.str("direction").unwrap_or_else(|| "left".into());
        let d = match dname.as_str() {
            "right" => [1.0, 0.0],
            "up" => [0.0, -1.0],
            "down" => [0.0, 1.0],
            "angle" => {
                let r = a.num("angle", 0.0).to_radians();
                [r.cos(), r.sin()]
            }
            _ => [-1.0, 0.0],
        };
        let extent = d[0].abs() * w + d[1].abs() * h;
        let fps = cx.map(|c| c.fps).unwrap_or(30.0).max(1e-9);
        let blur_on = !matches!(a.str("motionBlur").as_deref(), Some("false" | "0"));
        // a 180° shutter: the streak of a picture moving `travel` px per unit progress
        let shutter = |travel: f64| if blur_on { (velocity * travel).abs() * 0.5 / fps } else { 0.0 };
        let center = [pparam(e, "cx", 0.5) * w, pparam(e, "cy", 0.5) * h];
        // snapped to an axis; up/down run transposed; sign +1 when the incoming comes from the right
        let axis = if d[0].abs() >= d[1].abs() {
            if d[0] < 0.0 {
                "left"
            } else {
                "right"
            }
        } else if d[1] < 0.0 {
            "up"
        } else {
            "down"
        };
        let sign = if matches!(axis, "left" | "up") { 1.0 } else { -1.0 };
        let vertical = matches!(axis, "up" | "down");
        let outgoing_weight = |m: f64| if !has_b { 1.0 - m } else { 1.0 - sstep(0.4, 1.0, p) };
        match kind {
            "circle-open" | "iris" | "circle-close" | "clock-wipe" | "radial-wipe" | "luma" => {
                let corners = [[0.0, 0.0], [w, 0.0], [0.0, h], [w, h]];
                let maxr = corners.iter().map(|c| (c[0] - center[0]).hypot(c[1] - center[1])).fold(0.0f64, f64::max);
                v[0] = [center[0] as f32, center[1] as f32, maxr as f32, pparam(e, "invert", 0.0) as f32];
            }
            "slide" | "push" | "cover" | "reveal" => v[0] = [shutter(extent) as f32, extent as f32, 0.0, 0.0],
            "whip-pan" => {
                let k = 6.0f64;
                let t = |x: f64| x.tanh();
                let q = 0.5 + 0.5 * t(k * (p - 0.5)) / t(k / 2.0);
                let dq = 0.5 * k * (1.0 - t(k * (p - 0.5)).powi(2)) / t(k / 2.0);
                let dq0 = 0.5 * k * (1.0 - t(k / 2.0).powi(2)) / t(k / 2.0);
                let mut bl = extent * 0.12 * (dq - dq0).max(0.0);
                if !blur_on {
                    bl *= 0.25;
                }
                v[0] = [bl as f32, extent as f32, q as f32, 0.0];
            }
            "zoom-in" | "zoom-out" => {
                let amt = pparam(e, "amount", 3.0);
                let (a_end, b_start): (f64, f64) = if kind == "zoom-in" { (amt, 1.0 / 1.8) } else { (1.0 / amt, 1.8) };
                let m = sstep(0.2, 0.8, p);
                let (ka, kb) = (a_end.powf(p), b_start.powf(1.0 - p));
                v[0] = [center[0] as f32, center[1] as f32, ka as f32, kb as f32];
                v[1] = [m as f32, outgoing_weight(m) as f32, 0.0, 0.0];
                shrink = [[ka; 2], [kb; 2]];
            }
            "spin" => {
                let total = if dname == "angle" {
                    let t = a.num("angle", 180.0);
                    if t == 0.0 {
                        180.0
                    } else {
                        t
                    }
                } else {
                    180.0
                };
                let half = if matches!(dname.as_str(), "left" | "up") { -1.0 } else { 1.0 } * total / 2.0;
                let m = sstep(0.25, 0.75, p);
                v[0] = [
                    center[0] as f32,
                    center[1] as f32,
                    (half * p).to_radians() as f32,
                    (half * (p - 1.0)).to_radians() as f32,
                ];
                v[1] = [(1.0 + 0.6 * p) as f32, (0.4 + 0.6 * p) as f32, m as f32, outgoing_weight(m) as f32];
                shrink = [[1.0 + 0.6 * p; 2], [0.4 + 0.6 * p; 2]];
            }
            "squash" => {
                let c = [w / 2.0, h / 2.0];
                let lead = [c[0] + d[0] * extent / 2.0, c[1] + d[1] * extent / 2.0];
                let trail = [c[0] - d[0] * extent / 2.0, c[1] - d[1] * extent / 2.0];
                v[0] = [lead[0] as f32, lead[1] as f32, (1.0 - p) as f32, 0.0];
                v[1] = [trail[0] as f32, trail[1] as f32, p as f32, 0.0];
                // the columns of I + (k − 1) d dᵀ
                let col = |k: f64| {
                    let (a, b, c) =
                        (1.0 + (k - 1.0) * d[0] * d[0], (k - 1.0) * d[0] * d[1], 1.0 + (k - 1.0) * d[1] * d[1]);
                    [a.hypot(b), b.hypot(c)]
                };
                shrink = [col(1.0 - p), col(p)];
            }
            "shuffle" => {
                let o = (PI * p).sin() * 0.55 * extent;
                let s = sstep(0.15, 0.85, p);
                v[0] = [o as f32, (1.0 - 0.12 * s) as f32, (0.88 + 0.12 * s) as f32, (p < 0.5) as u32 as f32];
                shrink = [[1.0 - 0.12 * s; 2], [0.88 + 0.12 * s; 2]];
            }
            "blinds" => v[0] = [pparam(e, "count", 8.0).max(1.0).trunc() as f32, 0.0, 0.0, 0.0],
            "stripe" => {
                v[0] = [
                    pparam(e, "count", 10.0).max(1.0).trunc() as f32,
                    pparam(e, "stagger", 0.4).max(0.0) as f32,
                    0.0,
                    0.0,
                ]
            }
            "page-curl" => {
                let r = (pparam(e, "radius", 0.1) * extent).max(2.0);
                let c = extent - p * (extent + r);
                v[0] = [extent as f32, r as f32, c as f32, (0.45 * (PI * p).sin().max(0.0).sqrt()) as f32];
            }
            "flip" | "cube" | "carousel" => {
                let pw = if vertical { h } else { w };
                let cam = 2.0 * pw;
                let plane = |yaw: f64, x: f64, z: f64| {
                    [yaw.to_radians().cos() as f32, yaw.to_radians().sin() as f32, x as f32, z as f32]
                };
                let shade = |yaw: f64, k: f64| (1.0 - k * (1.0 - yaw.to_radians().cos().abs())) as f32;
                let facing = |yaw: f64, x: f64, z: f64| {
                    let (s, c) = (yaw.to_radians().sin(), yaw.to_radians().cos());
                    s * -x + -c * (-cam - z) > 1e-6
                };
                let mut flags = if vertical { 16u32 } else { 0 };
                match kind {
                    "flip" => {
                        let tz = (PI * p).sin() * 0.4 * pw;
                        let yaw = -sign * 180.0 * p;
                        let (yaw, is_b) = if p < 0.5 { (yaw, false) } else { (yaw + sign * 180.0, true) };
                        v[0] = plane(yaw, 0.0, tz);
                        v[2] = [shade(yaw, 0.35), 1.0, cam as f32, 0.0];
                        flags |= facing(yaw, 0.0, tz) as u32 | if is_b { 8 } else { 0 };
                    }
                    _ => {
                        let (step, rr, tzk, k) = if kind == "cube" {
                            (90.0, pw / 2.0, 0.35, 0.35)
                        } else {
                            let n = pparam(e, "panels", 6.0).trunc().max(3.0);
                            (360.0 / n, 1.15 * pw / (2.0 * (PI / n).tan()), 0.6, 0.5)
                        };
                        let tz = (PI * p).sin() * tzk * pw;
                        let pos = |yaw: f64| {
                            let r = yaw.to_radians();
                            (rr * r.sin(), rr - rr * r.cos() + tz)
                        };
                        let (ya, yb) = (-sign * step * p, sign * step * (1.0 - p));
                        let ((xa, za), (xb, zb)) = (pos(ya), pos(yb));
                        v[0] = plane(ya, xa, za);
                        v[1] = plane(yb, xb, zb);
                        v[2] = [shade(ya, k), shade(yb, k), cam as f32, 0.0];
                        flags |= facing(ya, xa, za) as u32 | (facing(yb, xb, zb) as u32) << 1;
                        if zb > za {
                            flags |= 4;
                        }
                    }
                }
                v[2][3] = flags as f32;
            }
            "film-roll" => {
                let pw = if vertical { h } else { w };
                let gap = param(e, a, "gap", 0.06).max(0.0);
                let length = pw * (1.0 + gap);
                let sh = if blur_on { shutter(length) / length.max(1.0) } else { 0.0 };
                let samples = if sh > 0.0 { ((sh * length).ceil()).clamp(1.0, 32.0) } else { 1.0 };
                v[0] = [
                    gap as f32,
                    param(e, a, "border", 0.07).clamp(0.0, 0.4) as f32,
                    param(e, a, "curvature", 0.85).clamp(0.0, 0.99) as f32,
                    param(e, a, "holes", 12.0).round_ties_even().max(2.0) as f32,
                ];
                v[1] = [sign as f32, vertical as u32 as f32, sh as f32, samples as f32];
            }
            "glitch" => {
                let k = (PI * p).sin().max(0.0).powf(0.8);
                v[0] = [-1.0, 0.0, 0.0, (p >= 0.5) as u32 as f32];
                if k >= 1e-3 {
                    if let Some(cx) = cx {
                        let seed = sr_eval::rng::element_seed(
                            cx.seed,
                            e.element_id().unwrap_or(""),
                            None,
                            "transition:glitch",
                        );
                        let frame = (cx.time * cx.fps).round_ties_even() as i64 as u64;
                        // Rng: one counter for uniform and normal draws
                        let n = std::cell::Cell::new(0u64);
                        let u = || {
                            let x = sr_eval::rng::d24_unit(seed, frame, n.get());
                            n.set(n.get() + 1);
                            x
                        };
                        let (wi, hi) = (w, h);
                        let bytes = |x: f64| [((x as u32 >> 8) & 255) as f32, (x as u32 & 255) as f32];
                        let nsl = (3.0 + 18.0 * k) as usize;
                        let mut slices = Vec::new();
                        for _ in 0..nsl {
                            let y0 = (hi * u()).floor();
                            let hh = ((0.005 + 0.085 * u()) * hi * (0.4 + k)).max(1.0).trunc();
                            let other = u() < 0.35 * k;
                            let g = sr_eval::rng::gaussian(seed, frame, n.get());
                            n.set(n.get() + 1);
                            let off = (g * 0.06 * wi * k).trunc();
                            let [a0, a1] = bytes(y0);
                            let [b0, b1] = bytes(hh);
                            let [c0, c1] = bytes(off.rem_euclid(wi));
                            slices.extend([[a0, a1, b0, b1], [c0, c1, 0.0, other as u32 as f32]]);
                        }
                        let nb = (8.0 * k) as usize;
                        let mut blocks = Vec::new();
                        for _ in 0..nb {
                            let bw = ((0.03 + 0.17 * u()) * wi).trunc();
                            let bh = ((0.01 + 0.05 * u()) * hi).trunc();
                            let int = |hi_excl: f64| (hi_excl * u()).floor();
                            let x0 = int((wi - bw).max(1.0));
                            let y0 = int((hi - bh).max(1.0));
                            let sx = int((wi - bw).max(1.0));
                            let sy = int((hi - bh).max(1.0));
                            let other = u() < 0.5;
                            let f = |x: f64| bytes(x);
                            let ([a0, a1], [b0, b1], [c0, c1], [d0, d1], [e0, e1], [f0, f1]) =
                                (f(x0), f(y0), f(bw), f(bh), f(sx), f(sy));
                            blocks.extend([
                                [a0, a1, b0, b1],
                                [c0, c1, d0, d1],
                                [e0, e1, f0, f1],
                                [0.0, other as u32 as f32, 0.0, 0.0],
                            ]);
                        }
                        let s = (0.012 * wi * k * if u() < 0.5 { 1.0 } else { -1.0 }).round_ties_even();
                        table.extend(slices);
                        table.extend(blocks);
                        v[0] = [nsl as f32, nb as f32, s as f32, (p >= 0.5) as u32 as f32];
                    }
                }
            }
            "light-leak" => {
                let m = sstep(0.35, 0.65, p);
                let k = (PI * p).sin().max(0.0).powf(1.2) * pparam(e, "amount", 1.3);
                v[3] = [m as f32, k as f32, 0.0, 0.0];
                if let (true, Some(cx)) = (k >= 1e-3, cx) {
                    let seed =
                        sr_eval::rng::element_seed(cx.seed, e.element_id().unwrap_or(""), None, "transition:leak");
                    let mut n = 0u64;
                    let mut uni = |lo: f64, hi: f64| {
                        let x = sr_eval::rng::d24_unit(seed, 0, n);
                        n += 1;
                        lo + (hi - lo) * x
                    };
                    // an explicit @color tints the blobs (the default #000000FF reads as none)
                    let explicit = color != cx.working.from_literal([0.0, 0.0, 0.0, 1.0]);
                    let palette = [[1.0, 0.56, 0.18], [1.0, 0.28, 0.12], [1.0, 0.86, 0.52]];
                    let dd = w.max(h);
                    for (i, col) in palette.iter().enumerate() {
                        let rgb = if explicit {
                            [color[0], color[1], color[2]]
                        } else {
                            let c = cx.working.from_literal([col[0], col[1], col[2], 1.0]);
                            [c[0], c[1], c[2]]
                        };
                        let (cx0, cy0) = (uni(0.15, 0.85) * w, uni(0.1, 0.9) * h);
                        let travel = uni(0.5, 0.9) * dd;
                        let bx = cx0 + d[0] * travel * (p - 0.5);
                        let by = cy0 + d[1] * travel * (p - 0.5) + uni(-0.1, 0.1) * h * (p - 0.5);
                        let sig = uni(0.18, 0.35) * dd;
                        let amp = uni(0.6, 1.0);
                        if i < 3 {
                            v[i] = [bx as f32, by as f32, sig as f32, amp as f32];
                        }
                        table.push([rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, 1.0]);
                    }
                }
            }
            _ => {}
        }
        TransitionExtras { v, table, shrink }
    }
}

/// `<param name value>` children of an element.
pub fn params_of(e: &dyn Element) -> Vec<(String, Vec<f32>)> {
    sr_model::element::children(e)
        .into_iter()
        .filter(|c| crate::vector::is(*c, "param"))
        .filter_map(|c| {
            let a = Attrs { e: c, props: None };
            let name = a.str("name")?;
            let val = a.str("value").unwrap_or_default();
            Some((
                name,
                val.split(|ch: char| !(ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == 'e' || ch == 'E'))
                    .filter_map(|t| t.parse().ok())
                    .collect(),
            ))
        })
        .collect()
}

/// Wraps an effect (`vec4 effect(vec2 uv)` reading `getColor(uv)`) or a
/// gl-transitions shader (`vec4 transition(vec2 uv)` reading
/// `getFromColor`/`getToColor`, with `progress` and `ratio`) as a complete
/// Vulkan-GLSL fragment shader. Loose uniforms move into the pass's
/// parameter block (defaults from `// = value` comments, overridden by
/// `<param>`); colours reach user code as straight, display-encoded sRGB.
pub fn wrap_glsl(
    code: &str,
    transition: bool,
    params: &[(String, Vec<f32>)],
    builtins: [f32; 4],
) -> Result<(String, [[f32; 4]; 8]), String> {
    let mut v = [[0.0f32; 4]; 8];
    v[0] = builtins;
    let mut body = String::new();
    let mut defines = String::new();
    let mut slot = (2usize, 0usize); // v[2..8]
    for line in code.lines() {
        let t = line.trim_start();
        let is_uniform = t.starts_with("uniform ") && !t.contains("sampler");
        if !is_uniform {
            if t.starts_with("#version") || t.starts_with("precision ") {
                continue;
            }
            body.push_str(line);
            body.push('\n');
            continue;
        }
        let decl = t.trim_start_matches("uniform ").split(';').next().unwrap_or("").trim().to_string();
        let mut parts = decl.split_whitespace();
        let (Some(ty), Some(name)) = (parts.next(), parts.next()) else { continue };
        if matches!(name, "progress" | "ratio" | "time" | "resolution") {
            continue;
        }
        let n = match ty {
            "float" | "int" | "bool" => 1,
            "vec2" | "ivec2" => 2,
            "vec3" | "ivec3" => 3,
            "vec4" | "ivec4" => 4,
            other => return Err(format!("uniform type {other} is not supported in custom shaders")),
        };
        if slot.1 + n > 4 {
            slot = (slot.0 + 1, 0);
        }
        if slot.0 >= 8 {
            return Err("custom shaders take at most 24 floats of uniforms".into());
        }
        let default: Vec<f32> = line
            .split("//")
            .nth(1)
            .and_then(|c| c.split('=').nth(1))
            .map(|d| {
                // "vec2(1.0, -1.0)" → the numbers inside the parentheses
                let inner = match (d.find('('), d.rfind(')')) {
                    (Some(a), Some(b)) if b > a => &d[a + 1..b],
                    _ => d,
                };
                inner
                    .split(|ch: char| !(ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == 'e'))
                    .filter_map(|x| x.parse().ok())
                    .collect()
            })
            .unwrap_or_default();
        let given = params.iter().find(|p| p.0 == name).map(|p| p.1.clone()).unwrap_or(default);
        for k in 0..n {
            v[slot.0][slot.1 + k] = given.get(k).copied().or_else(|| given.first().copied()).unwrap_or(0.0);
        }
        let sw = ["x", "y", "z", "w"];
        let comps: String = (0..n).map(|k| sw[slot.1 + k]).collect();
        let access = format!("sr_fx.v[{}].{}", slot.0, comps);
        let expr = match ty {
            "int" => format!("int({access})"),
            "bool" => format!("({access} != 0.0)"),
            "ivec2" | "ivec3" | "ivec4" => format!("{ty}({access})"),
            _ => access,
        };
        defines.push_str(&format!("#define {name} ({expr})\n"));
        slot.1 += n;
    }
    let pre = r#"#version 450
layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 o_color;
layout(set = 0, binding = 0) uniform SrFx { vec4 v[8]; uvec4 i; } sr_fx;
layout(set = 0, binding = 1) uniform texture2D sr_src;
layout(set = 0, binding = 2) uniform texture2D sr_aux;
layout(set = 0, binding = 3) uniform sampler sr_smp;
#define progress (sr_fx.v[0].x)
#define time (sr_fx.v[0].x)
#define resolution (sr_fx.v[0].zw)
#define ratio (sr_fx.v[0].z / sr_fx.v[0].w)
float sr_enc1(float x) { x = max(x, 0.0); return x <= 0.0031308 ? 12.92 * x : 1.055 * pow(x, 1.0 / 2.4) - 0.055; }
float sr_dec1(float x) { return x <= 0.04045 ? x / 12.92 : pow((x + 0.055) / 1.055, 2.4); }
vec4 sr_to_user(vec4 c) { if (c.a <= 1e-6) return vec4(0.0); vec3 s = c.rgb / c.a; return vec4(sr_enc1(s.r), sr_enc1(s.g), sr_enc1(s.b), c.a); }
vec4 sr_from_user(vec4 c) { float a = clamp(c.a, 0.0, 1.0); return vec4(vec3(sr_dec1(c.r), sr_dec1(c.g), sr_dec1(c.b)) * a, a); }
vec4 getColor(vec2 uv) { return sr_to_user(texture(sampler2D(sr_src, sr_smp), vec2(uv.x, 1.0 - uv.y))); }
vec4 getFromColor(vec2 uv) { return getColor(uv); }
vec4 getToColor(vec2 uv) { return sr_to_user(texture(sampler2D(sr_aux, sr_smp), vec2(uv.x, 1.0 - uv.y))); }
"#;
    let main = if transition {
        "void main() { o_color = sr_from_user(transition(vec2(v_uv.x, 1.0 - v_uv.y))); }\n"
    } else {
        "void main() { o_color = sr_from_user(effect(vec2(v_uv.x, 1.0 - v_uv.y))); }\n"
    };
    let glsl = if transition {
        format!("{pre}{defines}{body}{main}").replace("#define time (sr_fx.v[0].x)\n", "")
    } else {
        format!("{pre}{defines}{body}{main}").replace("#define progress (sr_fx.v[0].x)\n", "")
    };
    Ok((glsl, v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::naga;

    fn tmp(name: &str, text: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sr-fx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn effects_wgsl_validates() {
        let module = naga::front::wgsl::parse_str(WGSL).unwrap_or_else(|e| panic!("{}", e.emit_to_string(WGSL)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(WGSL)));
        for entry in [
            "fs_copy",
            "fs_down",
            "fs_up",
            "fs_pre",
            "fs_combine",
            "fs_color",
            "fs_warp",
            "fs_line",
            "fs_bokeh",
            "fs_conv",
            "fs_morph",
            "fs_flow",
            "fs_trans",
        ] {
            assert!(module.entry_points.iter().any(|e| e.name == entry), "missing {entry}");
        }
    }

    #[test]
    fn custom_effect_wraps_and_validates() {
        let code = "uniform float amount; // = 0.5\nuniform vec3 tint; // = vec3(1.0, 0.5, 0.25)\nvec4 effect(vec2 uv) { vec4 c = getColor(uv); return vec4(mix(c.rgb, tint, amount), c.a); }";
        let (glsl, v) = wrap_glsl(code, false, &[("amount".into(), vec![0.75])], [1.0, 2.0, 64.0, 32.0]).unwrap();
        check_glsl(&glsl).unwrap();
        assert_eq!(v[2][0], 0.75, "param overrides the default");
        assert_eq!(&v[2][1..4], &[1.0, 0.5, 0.25], "vec3 default from the comment packs into .yzw");
    }

    #[test]
    fn gl_transition_wraps_and_validates() {
        // the gl-transitions "directional wipe" shape: loose uniforms with defaults, progress, ratio
        let code = "uniform vec2 direction; // = vec2(1.0, -1.0)\nuniform float smoothness; // = 0.5\nconst vec2 center = vec2(0.5, 0.5);\nvec4 transition (vec2 uv) {\n  vec2 v = normalize(direction);\n  v /= abs(v.x)+abs(v.y);\n  float d = v.x * center.x + v.y * center.y;\n  float m = (1.0-step(progress, 0.0)) * (1.0 - smoothstep(-smoothness, 0.0, v.x * uv.x + v.y * uv.y - (d-0.5+progress*(1.+smoothness))));\n  return mix(getFromColor(uv), getToColor(uv), m * ratio / ratio);\n}";
        let (glsl, v) = wrap_glsl(code, true, &[], [0.3, 0.0, 16.0, 9.0]).unwrap();
        check_glsl(&glsl).unwrap();
        assert_eq!(&v[2][..3], &[1.0, -1.0, 0.5]);
        assert!(wrap_glsl("uniform mat3 m;\nvec4 effect(vec2 uv){return getColor(uv);}", false, &[], [0.0; 4]).is_err());
        assert!(check_glsl("#version 450\nvoid main() { undefined_call(); }").is_err());
    }

    #[test]
    fn cube_luts_parse() {
        let mut text = String::from("TITLE \"t\"\nLUT_3D_SIZE 2\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    text.push_str(&format!("{} {} {}\n", 1 - r, g, b));
                }
            }
        }
        let (n, t) = load_lut(&tmp("inv.cube", &text)).unwrap();
        assert_eq!(n, 2);
        assert_eq!(t[0], [1.0, 0.0, 0.0]);
        assert_eq!(t[1], [0.0, 0.0, 0.0]);
        let (n1, t1) = load_lut(&tmp("half.cube", "LUT_1D_SIZE 2\n0 0 0\n0.5 0.5 0.5\n")).unwrap();
        assert_eq!(n1, 33);
        let last = t1[t1.len() - 1];
        assert!((last[0] - 0.5).abs() < 1e-6 && (t1[16][1] - 0.0).abs() < 1e-6);
        assert!(load_lut(&tmp("bad.cube", "LUT_3D_SIZE 3\n0 0 0\n")).is_err());
    }

    #[test]
    fn three_dl_parses_blue_fastest_and_scales() {
        let mut text = String::from("0 1023\n");
        for r in 0..2 {
            for g in 0..2 {
                for b in 0..2 {
                    text.push_str(&format!("{} {} {}\n", r * 1023, g * 1023, b * 1023));
                }
            }
        }
        let (n, t) = load_lut(&tmp("id.3dl", &text)).unwrap();
        assert_eq!(n, 2);
        // identity: the entry at (r=1, g=0, b=0) in red-fastest order is index 1
        assert_eq!(t[1], [1.0, 0.0, 0.0]);
        assert_eq!(t[4], [0.0, 0.0, 1.0]);
    }

    #[test]
    fn clf_process_lists_bake() {
        let clf = r#"<?xml version="1.0"?>
<ProcessList id="x" compCLFversion="3">
  <Matrix inBitDepth="32f" outBitDepth="32f"><Array dim="3 3">0.5 0 0 0 0.5 0 0 0 0.5</Array></Matrix>
  <ASC_CDL inBitDepth="32f" outBitDepth="32f" style="Fwd"><SOPNode><Slope>2 2 2</Slope><Offset>0 0 0</Offset><Power>1 1 1</Power></SOPNode><SatNode><Saturation>1</Saturation></SatNode></ASC_CDL>
  <LUT1D inBitDepth="32f" outBitDepth="10i"><Array dim="2 1">0 1023</Array></LUT1D>
</ProcessList>"#;
        let (n, t) = load_lut(&tmp("chain.clf", clf)).unwrap();
        assert_eq!(n, 33);
        // halve, double, identity 1D: the chain is the identity
        let mid = t[(16 + 16 * 33 + 16 * 33 * 33) as usize];
        assert!((mid[0] - 0.5).abs() < 1e-4, "{mid:?}");
        let unsupported = r#"<ProcessList id="x"><Log style="log10"/></ProcessList>"#;
        assert!(load_lut(&tmp("log.clf", unsupported)).unwrap_err().contains("Log"));
    }

    #[test]
    fn curves_and_gradients_sample() {
        let id = curve_table("0,0 1,1").unwrap();
        assert!((id[128] - 128.0 / 255.0).abs() < 1e-3);
        let s = curve_table("0,0 0.5,0.8 1,1").unwrap();
        assert!(s[128] > 0.75 && s[255] == 1.0);
        let g = gradient_table(&[(0.0, [1.0, 0.0, 0.0, 1.0]), (1.0, [0.0, 0.0, 1.0, 0.5])]);
        assert_eq!(g[0], [1.0, 0.0, 0.0, 1.0]);
        assert!((g[255][2] - 0.5).abs() < 1e-6 && (g[255][3] - 0.5).abs() < 1e-6, "premultiplied");
    }
}
