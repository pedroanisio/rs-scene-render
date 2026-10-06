//! Video on the GPU: decoded planes become premultiplied working-space
//! textures (YUV matrix and range, chroma upsampling, rotation, input
//! transfer, primaries), and in-between source frames are synthesised by
//! frame mixing or DIS optical flow.

use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use sr_media::{PixelLayout, VideoFrame};
use sr_model::model::{AlphaMode, ColorSpace, Transfer};

use crate::color::{self, Working};
use crate::resources::{self, Tex};
use crate::types;

/// How to interpret a decoded stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interpretation {
    /// Primaries of the samples.
    pub space: ColorSpace,
    /// Transfer of the samples (resolved, not `auto`).
    pub transfer: Transfer,
    /// Alpha handling.
    pub alpha: AlphaMode,
    /// Clockwise quarter turns to display orientation.
    pub quarter_turns: u32,
    /// YUV matrix coefficients (Kr, Kb).
    pub matrix: (f32, f32),
    /// Full-range samples.
    pub full_range: bool,
}

impl Interpretation {
    /// Matrix coefficients from an FFmpeg `color_space` tag, defaulting by height.
    pub fn matrix_for(tag: &str, height: u32) -> (f32, f32) {
        match tag {
            "bt2020nc" | "bt2020c" | "bt2020_ncl" => (0.2627, 0.0593),
            "smpte170m" | "bt470bg" | "bt601" => (0.299, 0.114),
            "bt709" => (0.2126, 0.0722),
            _ if height >= 720 => (0.2126, 0.0722),
            _ => (0.299, 0.114),
        }
    }

    /// The transfer an FFmpeg `color_transfer` tag names, if it is one of the schema's.
    pub fn transfer_for(tag: &str) -> Option<Transfer> {
        Some(match tag {
            "smpte2084" => Transfer::Pq,
            "arib-std-b67" => Transfer::Hlg,
            "iec61966-2-1" => Transfer::Srgb,
            "bt709" | "bt1361e" | "smpte170m" | "bt2020-10" | "bt2020-12" => Transfer::Bt1886,
            "gamma22" => Transfer::Gamma22,
            "linear" => Transfer::Linear,
            _ => return None,
        })
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Conv {
    to_working: [[f32; 4]; 3],
    size_out: [u32; 2],
    coded: [u32; 2],
    chroma_shift: [u32; 2],
    packing: u32,
    rotation: u32,
    transfer: u32,
    alpha_mode: u32,
    full_range: u32,
    deep: u32,
    linear_light: u32,
    store_transfer: u32,
    kr: f32,
    kb: f32,
    pad: [u32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct FlowParams {
    size: [u32; 2],
    grid: [u32; 2],
    coarse: [u32; 2],
    t: f32,
    iterations: u32,
}

struct ConvPipe {
    bgl: wgpu::BindGroupLayout,
    pipe: wgpu::RenderPipeline,
}

/// Pipelines and plane textures for video processing.
pub struct VideoEngine {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    conv_f: ConvPipe,
    conv_u: ConvPipe,
    mix: wgpu::RenderPipeline,
    warp: wgpu::RenderPipeline,
    reduce: wgpu::ComputePipeline,
    patch: wgpu::ComputePipeline,
    densify: wgpu::ComputePipeline,
    bgl0: wgpu::BindGroupLayout,
    bgl1: wgpu::BindGroupLayout,
    bgl2: wgpu::BindGroupLayout,
    bgl3: wgpu::BindGroupLayout,
    samp: wgpu::Sampler,
    planes: HashMap<(u32, u32, PixelLayout), Vec<wgpu::Texture>>,
}

fn entry(binding: u32, vis: wgpu::ShaderStages, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty, count: None }
}

fn tex_ty(sample: wgpu::TextureSampleType) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: sample,
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}

fn uniform() -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

fn storage_tex(f: wgpu::TextureFormat) -> wgpu::BindingType {
    wgpu::BindingType::StorageTexture {
        access: wgpu::StorageTextureAccess::WriteOnly,
        format: f,
        view_dimension: wgpu::TextureViewDimension::D2,
    }
}

const F: wgpu::ShaderStages = wgpu::ShaderStages::FRAGMENT;
const C: wgpu::ShaderStages = wgpu::ShaderStages::COMPUTE;
const UNFILTERED: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: false };
const FILTERED: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: true };

impl VideoEngine {
    /// Builds the pipelines.
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> VideoEngine {
        let d = &*device;
        let transfer = include_str!("transfer.wgsl");
        let conv = |tex_t: &str, scale: &str, sample: wgpu::TextureSampleType| {
            let src = format!(
                "{transfer}\n{}",
                include_str!("video.wgsl").replace("TEX_T", tex_t).replace("LOAD_SCALE", scale)
            );
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("video"),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            });
            let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("video planes"),
                entries: &[
                    entry(0, F, uniform()),
                    entry(1, F, tex_ty(sample)),
                    entry(2, F, tex_ty(sample)),
                    entry(3, F, tex_ty(sample)),
                ],
            });
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&bgl)],
                immediate_size: 0,
            });
            let pipe = full_pipeline(d, &module, &layout, "fs_convert");
            ConvPipe { bgl, pipe }
        };
        let conv_f = conv("f32", "1.0", FILTERED);
        let conv_u = conv("u32", "(1.0 / 65535.0)", wgpu::TextureSampleType::Uint);
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flow"),
            source: wgpu::ShaderSource::Wgsl(include_str!("flow.wgsl").into()),
        });
        let bgl0 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flow params"),
            entries: &[entry(0, C | F, uniform())],
        });
        let bgl1 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("reduce"),
            entries: &[
                entry(0, C, tex_ty(UNFILTERED)),
                entry(1, C, tex_ty(UNFILTERED)),
                entry(2, C, storage_tex(wgpu::TextureFormat::R32Float)),
                entry(3, C, storage_tex(wgpu::TextureFormat::R32Float)),
            ],
        });
        let bgl2 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("patches"),
            entries: &[
                entry(0, C, tex_ty(UNFILTERED)),
                entry(1, C, tex_ty(UNFILTERED)),
                entry(2, C, tex_ty(UNFILTERED)),
                entry(
                    3,
                    C,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(4, C, storage_tex(wgpu::TextureFormat::Rgba32Float)),
            ],
        });
        let bgl3 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("warp"),
            entries: &[
                entry(0, F, tex_ty(FILTERED)),
                entry(1, F, tex_ty(FILTERED)),
                entry(2, F, tex_ty(UNFILTERED)),
                entry(3, F, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            ],
        });
        let compute = |groups: &[Option<&wgpu::BindGroupLayout>], ep: &str| {
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: groups,
                immediate_size: 0,
            });
            {
                let _creation = crate::gpu::creation_lock();
                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(ep),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some(ep),
                    compilation_options: Default::default(),
                    cache: None,
                })
            }
        };
        let reduce = compute(&[Some(&bgl0), Some(&bgl1)], "cs_reduce");
        let patch = compute(&[Some(&bgl0), None, Some(&bgl2)], "cs_patch");
        let densify = compute(&[Some(&bgl0), None, Some(&bgl2)], "cs_densify");
        let frag_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl0), None, None, Some(&bgl3)],
            immediate_size: 0,
        });
        let mix = full_pipeline(d, &module, &frag_layout, "fs_mix");
        let warp = full_pipeline(d, &module, &frag_layout, "fs_warp");
        let samp = d.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        VideoEngine {
            device,
            queue,
            conv_f,
            conv_u,
            mix,
            warp,
            reduce,
            patch,
            densify,
            bgl0,
            bgl1,
            bgl2,
            bgl3,
            samp,
            planes: HashMap::new(),
        }
    }

    /// Uploads `frame` and converts it to a premultiplied working-space texture.
    pub fn convert(
        &mut self,
        frame: &VideoFrame,
        it: &Interpretation,
        working: &Working,
        src_layout: &wgpu::BindGroupLayout,
    ) -> Tex {
        let d = self.device.clone();
        let deep = matches!(frame.layout, PixelLayout::Yuv16(..) | PixelLayout::Rgba16);
        let rgba = matches!(frame.layout, PixelLayout::Rgba8 | PixelLayout::Rgba16);
        let fmt = match (rgba, deep) {
            (false, false) => wgpu::TextureFormat::R8Unorm,
            (false, true) => wgpu::TextureFormat::R16Uint,
            (true, false) => wgpu::TextureFormat::Rgba8Unorm,
            (true, true) => wgpu::TextureFormat::Rgba16Uint,
        };
        let key = (frame.width, frame.height, frame.layout);
        let planes = self.planes.entry(key).or_insert_with(|| {
            frame
                .planes
                .iter()
                .map(|p| {
                    d.create_texture(&wgpu::TextureDescriptor {
                        label: Some("video plane"),
                        size: wgpu::Extent3d { width: p.width, height: p.height, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: fmt,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    })
                })
                .collect()
        });
        let bpt = fmt.block_copy_size(None).unwrap_or(1);
        for (t, p) in planes.iter().zip(&frame.planes) {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: t,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &p.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(p.width * bpt),
                    rows_per_image: Some(p.height),
                },
                wgpu::Extent3d { width: p.width, height: p.height, depth_or_array_layers: 1 },
            );
        }
        let views: Vec<wgpu::TextureView> = planes.iter().map(|t| t.create_view(&Default::default())).collect();
        let (w, h) = if it.quarter_turns % 2 == 1 { (frame.height, frame.width) } else { (frame.width, frame.height) };
        let shift = match frame.layout {
            PixelLayout::Yuv8(x, y) | PixelLayout::Yuv16(x, y) => [x, y],
            _ => [0, 0],
        };
        let store = color::default_transfer(working.space);
        let u = Conv {
            to_working: types::mat3(&color::convert(it.space, working.space)),
            size_out: [w, h],
            coded: [frame.width, frame.height],
            chroma_shift: shift,
            packing: rgba as u32,
            rotation: it.quarter_turns % 4,
            transfer: color::transfer_id(it.transfer),
            alpha_mode: match it.alpha {
                AlphaMode::Premultiplied => 1,
                AlphaMode::None => 2,
                _ => 0,
            },
            full_range: it.full_range as u32,
            deep: deep as u32,
            linear_light: working.linear as u32,
            store_transfer: color::transfer_id(if store == Transfer::Linear { Transfer::Srgb } else { store }),
            kr: it.matrix.0,
            kb: it.matrix.1,
            pad: [0; 4],
        };
        let ubuf = uniform_buffer(&d, &self.queue, bytemuck::bytes_of(&u));
        let cp = if deep { &self.conv_u } else { &self.conv_f };
        let view = |i: usize| &views[i.min(views.len() - 1)];
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &cp.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view(0)) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(view(1)) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(view(2)) },
            ],
        });
        let out = resources::create(&d, src_layout, [w, h], 1, "video frame");
        let mut enc = d.create_command_encoder(&Default::default());
        {
            let mut pass = begin(&mut enc, &out.view);
            pass.set_pipeline(&cp.pipe);
            pass.set_bind_group(0, &bg, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([enc.finish()]);
        out
    }

    fn params(&self, p: FlowParams) -> wgpu::BindGroup {
        let b = uniform_buffer(&self.device, &self.queue, bytemuck::bytes_of(&p));
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.bgl0,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: b.as_entire_binding() }],
        })
    }

    fn blend_pass(
        &self,
        pipe: &wgpu::RenderPipeline,
        a: &Tex,
        b: &Tex,
        flow: &wgpu::TextureView,
        t: f32,
        src_layout: &wgpu::BindGroupLayout,
    ) -> Tex {
        let d = &self.device;
        let out = resources::create(d, src_layout, a.size, 1, "interpolated frame");
        let p0 = self.params(FlowParams { size: a.size, grid: [0; 2], coarse: [0; 2], t, iterations: 0 });
        let g3 = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.bgl3,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&a.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&b.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(flow) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.samp) },
            ],
        });
        let mut enc = d.create_command_encoder(&Default::default());
        {
            let mut pass = begin(&mut enc, &out.view);
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &p0, &[]);
            pass.set_bind_group(3, &g3, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([enc.finish()]);
        out
    }

    /// Linear blend of two frames: `(1 − t)·a + t·b`.
    pub fn mix(&self, a: &Tex, b: &Tex, t: f32, src_layout: &wgpu::BindGroupLayout) -> Tex {
        let dummy = self.flow_texture([1, 1]).create_view(&Default::default());
        self.blend_pass(&self.mix, a, b, &dummy, t, src_layout)
    }

    fn flow_texture(&self, size: [u32; 2]) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("flow"),
            size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    fn luma_texture(&self, size: [u32; 2]) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("luma"),
            size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        })
    }

    /// Dense flow from `a` to `b` at half resolution (pixels of that level).
    pub fn flow(&self, a: &Tex, b: &Tex) -> wgpu::Texture {
        let d = &self.device;
        // luma pyramid, level 0 at half resolution
        let mut sizes = vec![[a.size[0].div_ceil(2), a.size[1].div_ceil(2)]];
        while sizes.len() < 6 && sizes.last().unwrap().iter().all(|&s| s >= 32) {
            let l = *sizes.last().unwrap();
            sizes.push([l[0].div_ceil(2), l[1].div_ceil(2)]);
        }
        let la: Vec<wgpu::Texture> = sizes.iter().map(|s| self.luma_texture(*s)).collect();
        let lb: Vec<wgpu::Texture> = sizes.iter().map(|s| self.luma_texture(*s)).collect();
        let v = |t: &wgpu::Texture| t.create_view(&Default::default());
        let mut enc = d.create_command_encoder(&Default::default());
        for (l, &size) in sizes.iter().enumerate() {
            let (sa, sb) = if l == 0 { (a.view.clone(), b.view.clone()) } else { (v(&la[l - 1]), v(&lb[l - 1])) };
            let g1 = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.bgl1,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&sa) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&sb) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&v(&la[l])) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&v(&lb[l])) },
                ],
            });
            let p = self.params(FlowParams { size, grid: [0; 2], coarse: [0; 2], t: 0.0, iterations: 0 });
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.reduce);
            pass.set_bind_group(0, &p, &[]);
            pass.set_bind_group(1, &g1, &[]);
            pass.dispatch_workgroups(size[0].div_ceil(8), size[1].div_ceil(8), 1);
        }
        // coarse to fine
        let grid = |s: [u32; 2]| [(s[0].saturating_sub(8) / 4 + 1).max(1), (s[1].saturating_sub(8) / 4 + 1).max(1)];
        let g0 = grid(sizes[0]);
        let patches = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("patches"),
            size: (g0[0] * g0[1] * 8) as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let flows: Vec<wgpu::Texture> = sizes.iter().map(|s| self.flow_texture(*s)).collect();
        let none = self.flow_texture([1, 1]);
        for l in (0..sizes.len()).rev() {
            let size = sizes[l];
            let coarse = if l + 1 < sizes.len() { &flows[l + 1] } else { &none };
            let coarse_size = if l + 1 < sizes.len() { sizes[l + 1] } else { [0, 0] };
            let g2 = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.bgl2,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&v(&la[l])) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&v(&lb[l])) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&v(coarse)) },
                    wgpu::BindGroupEntry { binding: 3, resource: patches.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&v(&flows[l])) },
                ],
            });
            let gs = grid(size);
            let p = self.params(FlowParams { size, grid: gs, coarse: coarse_size, t: 0.0, iterations: 12 });
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_bind_group(0, &p, &[]);
            pass.set_bind_group(2, &g2, &[]);
            pass.set_pipeline(&self.patch);
            pass.dispatch_workgroups(gs[0].div_ceil(8), gs[1].div_ceil(8), 1);
            pass.set_pipeline(&self.densify);
            pass.dispatch_workgroups(size[0].div_ceil(8), size[1].div_ceil(8), 1);
        }
        self.queue.submit([enc.finish()]);
        flows.into_iter().next().unwrap()
    }

    /// Frame at phase `t` between `a` and `b`, warped along the optical flow.
    pub fn interpolate(&self, a: &Tex, b: &Tex, t: f32, src_layout: &wgpu::BindGroupLayout) -> Tex {
        let flow = self.flow(a, b);
        self.blend_pass(&self.warp, a, b, &flow.create_view(&Default::default()), t, src_layout)
    }

    /// Reads a flow texture back (x, y per pixel), for tests and diagnostics.
    pub fn read_flow(&self, t: &wgpu::Texture) -> (u32, u32, Vec<[f32; 2]>) {
        let (w, h) = (t.width(), t.height());
        let row = (w * 16).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
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
            for x in 0..w {
                let o = (y * row + x * 16) as usize;
                let f = |k: usize| f32::from_le_bytes([data[o + k], data[o + k + 1], data[o + k + 2], data[o + k + 3]]);
                out.push([f(0), f(4)]);
            }
        }
        (w, h, out)
    }
}

fn uniform_buffer(d: &wgpu::Device, q: &wgpu::Queue, bytes: &[u8]) -> wgpu::Buffer {
    let b = d.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    q.write_buffer(&b, 0, bytes);
    b
}

fn full_pipeline(
    d: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    fs: &str,
) -> wgpu::RenderPipeline {
    {
        let _creation = crate::gpu::creation_lock();
        d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(fs),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs_full"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some(fs),
                targets: &[Some(wgpu::ColorTargetState {
                    format: resources::FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        })
    }
}

fn begin<'e>(enc: &'e mut wgpu::CommandEncoder, view: &'e wgpu::TextureView) -> wgpu::RenderPass<'e> {
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}
