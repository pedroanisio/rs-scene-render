//! Delivery conversion on the GPU: the working-space frame becomes the
//! bytes an encoder reads (NV12, P010, RGBA8, RGBA16 or planar float), in
//! the output colour space, transfer, matrix and range, scaled to the
//! output size with letterboxing, then read back through a staging ring so
//! the next frame renders and converts while this one maps.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use sr_media::encode::InputFormat;
use sr_model::model::{ColorSpace, Transfer};

use crate::color::{self, Working};
use crate::resources::Tex;
use crate::types;

/// The colour of a deliverable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputColor {
    /// Primaries.
    pub space: ColorSpace,
    /// Transfer (resolved).
    pub transfer: Transfer,
    /// YUV matrix (Kr, Kb).
    pub matrix: (f32, f32),
    /// Full-range codes.
    pub full_range: bool,
}

impl OutputColor {
    /// Output colour of `space`/`transfer` attributes (`auto` resolves by space).
    pub fn new(space: ColorSpace, transfer: Transfer, full_range: bool) -> OutputColor {
        let transfer = color::resolve(space, transfer);
        let matrix = if space == ColorSpace::Rec2020 { (0.2627, 0.0593) } else { (0.2126, 0.0722) };
        OutputColor { space, transfer, matrix, full_range }
    }

    /// FFmpeg colour tags: primaries, transfer, matrix.
    pub fn ffmpeg_tags(&self) -> (String, String, String) {
        let prim = match self.space {
            ColorSpace::DisplayP3 => "smpte432",
            ColorSpace::DciP3 => "smpte431",
            ColorSpace::Rec2020 => "bt2020",
            _ => "bt709",
        };
        let trc = match self.transfer {
            Transfer::Srgb => "iec61966-2-1",
            Transfer::Pq => "smpte2084",
            Transfer::Hlg => "arib-std-b67",
            Transfer::Linear => "linear",
            Transfer::Gamma22 => "gamma22",
            Transfer::Gamma26 => "unknown",
            _ => "bt709",
        };
        let mat = if self.space == ColorSpace::Rec2020 { "bt2020nc" } else { "bt709" };
        (prim.into(), trc.into(), mat.into())
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Params {
    to_out: [[f32; 4]; 3],
    fsize: [u32; 2],
    osize: [u32; 2],
    offset: [f32; 2],
    scale: f32,
    format: u32,
    transfer: u32,
    full_range: u32,
    linear_light: u32,
    store_transfer: u32,
    keep_alpha: u32,
    kr: f32,
    kb: f32,
    words: u32,
    seed: u32,
    mode: u32,
    bg_offset: [f32; 2],
    bg_scale: f32,
    overlay: u32,
    pad: [u32; 2],
}

/// How a frame is placed in an output of another shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placement {
    /// Scaled to fit inside, transparent (black when opaque) around it.
    Fit { focus: [f64; 2] },
    /// Scaled to cover, the excess cut away.
    Crop { focus: [f64; 2] },
    /// Scaled to fit inside, over a blurred copy scaled to cover.
    FitBlur { focus: [f64; 2] },
}

impl Default for Placement {
    fn default() -> Placement {
        Placement::Fit { focus: [0.5, 0.5] }
    }
}

impl Placement {
    /// Scale and offset (output pixels) of a `frame`-sized image placed in `size`: `cover` or fit, with
    /// the free space shared out by `focus` (0 puts the image's left or top edge on the output's).
    fn place(frame: [f32; 2], size: [u32; 2], cover: bool, focus: [f64; 2]) -> (f32, [f32; 2]) {
        let (sx, sy) = (size[0] as f32 / frame[0], size[1] as f32 / frame[1]);
        let scale = if cover { sx.max(sy) } else { sx.min(sy) };
        let free = [size[0] as f32 - frame[0] * scale, size[1] as f32 - frame[1] * scale];
        (scale, [free[0] * focus[0] as f32, free[1] * focus[1] as f32])
    }
}

/// A conversion in flight: its staging buffer (out of the ring until [`OutputStage::wait`]
/// returns it), the submission that fills it, and the flag its map callback sets.
pub struct Pending {
    slot: usize,
    buffer: wgpu::Buffer,
    bytes: usize,
    submitted: wgpu::SubmissionIndex,
    ready: Arc<std::sync::atomic::AtomicBool>,
}

/// Staging buffers in the ring: one mapping, one converting, one spare.
const RING: usize = 3;

/// The converter and its staging ring.
pub struct OutputStage {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipe: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    /// Placement into a working-space texture, without conversion (transitions between outputs' frames).
    place_pipe: wgpu::ComputePipeline,
    place_bgl: wgpu::BindGroupLayout,
    samp: wgpu::Sampler,
    /// Parameters, rewritten before every submission (queue writes land after earlier work).
    params: wgpu::Buffer,
    /// The fit-blur background: downsample and Gaussian passes, their direction uniforms (down,
    /// across, along), the two reduced textures they ping-pong between, and a 1×1 stand-in.
    blur_pipe: wgpu::ComputePipeline,
    blur_bgl: wgpu::BindGroupLayout,
    blur_dirs: [wgpu::Buffer; 3],
    blur_tex: Option<([u32; 2], [wgpu::TextureView; 2])>,
    none_view: wgpu::TextureView,
    /// Packed output, grown when a larger frame needs it.
    packed: Option<wgpu::Buffer>,
    /// Staging buffers by slot; a slot is empty while its buffer is out in a [`Pending`].
    ring: [Option<wgpu::Buffer>; RING],
    /// The slot the next submission takes.
    head: usize,
    frame_index: u32,
}

impl OutputStage {
    /// Builds the pipeline.
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> OutputStage {
        let d = &*device;
        let src = format!("{}\n{}", include_str!("transfer.wgsl"), include_str!("output.wgsl"));
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("output"),
            source: wgpu::ShaderSource::Wgsl(src.into()),
        });
        let c = wgpu::ShaderStages::COMPUTE;
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: c,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("output"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: c,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: c,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: c,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(4),
                texture(5),
            ],
        });
        let place_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("output place"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: c,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: c,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture(4),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: c,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let blur_module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("output blur"),
            source: wgpu::ShaderSource::Wgsl(include_str!("output_blur.wgsl").into()),
        });
        let blur_bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("output blur"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: c,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: c,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: c,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let blur_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&blur_bgl)],
            immediate_size: 0,
        });
        let blur_pipe = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("output blur"),
            layout: Some(&blur_layout),
            module: &blur_module,
            entry_point: Some("cs_blur"),
            compilation_options: Default::default(),
            cache: None,
        });
        let blur_dirs = [[0i32, 0, 0, 0], [1, 0, 0, 0], [0, 1, 0, 0]].map(|v| {
            let b = d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("output blur direction"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&b, 0, bytemuck::cast_slice(&v));
            b
        });
        let none_view = d
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("no blur"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipe = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pack"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("cs_pack"),
            compilation_options: Default::default(),
            cache: None,
        });
        let place_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&place_bgl)],
            immediate_size: 0,
        });
        let place_pipe = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("place"),
            layout: Some(&place_layout),
            module: &module,
            entry_point: Some("cs_place"),
            compilation_options: Default::default(),
            cache: None,
        });
        let samp = d.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let params = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("output params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        OutputStage {
            device,
            queue,
            pipe,
            bgl,
            place_pipe,
            place_bgl,
            samp,
            params,
            blur_pipe,
            blur_bgl,
            blur_dirs,
            blur_tex: None,
            none_view,
            packed: None,
            ring: [None, None, None],
            head: 0,
            frame_index: 0,
        }
    }

    /// Makes the next submitted frame frame number `frame` of the programme: it seeds that
    /// frame's dither, so a frame converts the same whichever frame a render started from.
    pub fn seek(&mut self, frame: u32) {
        self.frame_index = frame;
    }

    /// Starts converting `frame` to `size` in `format`; returns a pending readback.
    pub fn submit(
        &mut self,
        frame: &Tex,
        working: &Working,
        out: &OutputColor,
        format: InputFormat,
        size: [u32; 2],
        keep_alpha: bool,
    ) -> Pending {
        self.submit_placed(frame, working, out, format, size, keep_alpha, Placement::default(), None)
    }

    /// [`submit`](Self::submit), with the frame placed in the output by `placement` and an
    /// output-sized `overlay` (working representation, premultiplied) composited over it.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_placed(
        &mut self,
        frame: &Tex,
        working: &Working,
        out: &OutputColor,
        format: InputFormat,
        size: [u32; 2],
        keep_alpha: bool,
        placement: Placement,
        overlay: Option<&Tex>,
    ) -> Pending {
        let d = self.device.clone();
        let bytes = format.frame_bytes(size[0], size[1]);
        let words = bytes.div_ceil(4) as u32;
        let mut p = Self::params(frame, working, out, size, placement);
        p.format = match format {
            InputFormat::Nv12 => 0,
            InputFormat::P010 => 1,
            InputFormat::Rgba8 => 2,
            InputFormat::Rgba16 => 3,
            InputFormat::Gbrapf32 => 4,
        };
        p.keep_alpha = keep_alpha as u32;
        p.words = words;
        p.seed = self.frame_index;
        if let Some(o) = overlay {
            assert_eq!(o.size, size, "the overlay is output-sized");
            p.overlay = 1;
        }
        let mode = p.mode;
        self.frame_index = self.frame_index.wrapping_add(1);
        // The queue orders this write after every earlier submission, so the previous
        // frame's pack pass reads its own parameters before these land.
        self.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
        let mut enc = d.create_command_encoder(&Default::default());
        if mode == 1 {
            self.blur(&mut enc, frame);
        }
        let size_bytes = (words as u64) * 4;
        // The shader stops at `words`, so a packed buffer left over from a larger frame is fine.
        if self.packed.as_ref().is_none_or(|b| b.size() < size_bytes) {
            self.packed = Some(d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("packed"),
                size: size_bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }));
        }
        let storage = self.packed.as_ref().expect("packed buffer");
        // Round-robin through the ring. A slot is only empty while its buffer is out in a
        // `Pending` (still mapping, or dropped unwaited), so a buffer is never handed out
        // twice; an empty or wrongly sized slot gets a fresh buffer.
        let slot = self.head;
        self.head = (self.head + 1) % RING;
        let staging = match self.ring[slot].take() {
            Some(b) if b.size() == size_bytes => b,
            _ => d.create_buffer(&wgpu::BufferDescriptor {
                label: Some("staging"),
                size: size_bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        };
        let blurred = match (&self.blur_tex, mode) {
            (Some((_, v)), 1) => &v[0],
            _ => &self.none_view,
        };
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&frame.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.samp) },
                wgpu::BindGroupEntry { binding: 3, resource: storage.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(blurred) },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(overlay.map_or(&self.none_view, |o| &o.view)),
                },
            ],
        });
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.pipe);
            pass.set_bind_group(0, &bg, &[]);
            let groups = words.div_ceil(64);
            let (x, y) = if groups > 65535 { (65535, groups.div_ceil(65535)) } else { (groups, 1) };
            pass.dispatch_workgroups(x, y, 1);
        }
        enc.copy_buffer_to_buffer(storage, 0, &staging, 0, size_bytes);
        let submitted = self.queue.submit([enc.finish()]);
        let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let r = ready.clone();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |_| r.store(true, std::sync::atomic::Ordering::Release));
        Pending { slot, buffer: staging, bytes, submitted, ready }
    }

    /// Conversion parameters for `frame` placed in `size`; the caller sets the packing fields.
    fn params(frame: &Tex, working: &Working, out: &OutputColor, size: [u32; 2], placement: Placement) -> Params {
        let fsz = [frame.size[0] as f32, frame.size[1] as f32];
        let (mode, (scale, offset), (bg_scale, bg_offset)) = match placement {
            Placement::Fit { focus } => (0, Placement::place(fsz, size, false, focus), (1.0, [0.0; 2])),
            Placement::Crop { focus } => (0, Placement::place(fsz, size, true, focus), (1.0, [0.0; 2])),
            Placement::FitBlur { focus } => {
                (1, Placement::place(fsz, size, false, focus), Placement::place(fsz, size, true, focus))
            }
        };
        let store = color::default_transfer(working.space);
        Params {
            to_out: types::mat3(&color::convert(working.space, out.space)),
            fsize: frame.size,
            osize: size,
            offset,
            scale,
            format: 0,
            transfer: color::transfer_id(out.transfer),
            full_range: out.full_range as u32,
            linear_light: working.linear as u32,
            store_transfer: color::transfer_id(if store == Transfer::Linear { Transfer::Srgb } else { store }),
            keep_alpha: 0,
            kr: out.matrix.0,
            kb: out.matrix.1,
            words: 0,
            seed: 0,
            mode,
            bg_offset,
            bg_scale,
            overlay: 0,
            pad: [0; 2],
        }
    }

    /// Records the fit-blur background of `frame`: reduced 8× and blurred, down into [0], across into
    /// [1], down again into [0].
    fn blur(&mut self, enc: &mut wgpu::CommandEncoder, frame: &Tex) {
        let d = &*self.device;
        let small = [frame.size[0].div_ceil(8).max(1), frame.size[1].div_ceil(8).max(1)];
        if self.blur_tex.as_ref().is_none_or(|(s, _)| *s != small) {
            let make = || {
                d.create_texture(&wgpu::TextureDescriptor {
                    label: Some("fit-blur"),
                    size: wgpu::Extent3d { width: small[0], height: small[1], depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba16Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
            };
            self.blur_tex = Some((small, [make(), make()]));
        }
        let (_, views) = self.blur_tex.as_ref().expect("blur textures");
        let steps: [(&wgpu::TextureView, &wgpu::TextureView, usize); 3] =
            [(&frame.view, &views[0], 0), (&views[0], &views[1], 1), (&views[1], &views[0], 2)];
        for (src, dst, k) in steps {
            let g = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.blur_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(dst) },
                    wgpu::BindGroupEntry { binding: 2, resource: self.blur_dirs[k].as_entire_binding() },
                ],
            });
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.blur_pipe);
            pass.set_bind_group(0, &g, &[]);
            pass.dispatch_workgroups(small[0].div_ceil(8), small[1].div_ceil(8), 1);
        }
    }

    /// Places `frame` in `into` (the output's size) as `placement` does, in the working
    /// representation, without converting it: both sides of a transition between an output's
    /// segments are placed with their own focus before they are combined.
    pub fn place(&mut self, frame: &Tex, working: &Working, into: &Tex, placement: Placement) {
        let d = self.device.clone();
        let same = OutputColor::new(working.space, Transfer::Linear, true);
        let p = Self::params(frame, working, &same, into.size, placement);
        self.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
        let mut enc = d.create_command_encoder(&Default::default());
        if p.mode == 1 {
            self.blur(&mut enc, frame);
        }
        let blurred = match (&self.blur_tex, p.mode) {
            (Some((_, v)), 1) => &v[0],
            _ => &self.none_view,
        };
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.place_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&frame.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.samp) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(blurred) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&into.view) },
            ],
        });
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.place_pipe);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(into.size[0].div_ceil(8), into.size[1].div_ceil(8), 1);
        }
        self.queue.submit([enc.finish()]);
    }

    /// Waits for a conversion and returns its bytes; the staging buffer returns to the ring.
    ///
    /// Only this conversion's submission is waited for, so work submitted after it (the
    /// next frame's render and pack) keeps running on the GPU meanwhile.
    pub fn wait(&mut self, p: Pending) -> Vec<u8> {
        use std::sync::atomic::Ordering;
        let Pending { slot, buffer, bytes, submitted, ready } = p;
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: Some(submitted), timeout: None });
        while !ready.load(Ordering::Acquire) {
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        }
        let out = {
            let view = buffer.slice(..).get_mapped_range().expect("mapped");
            view[..bytes].to_vec()
        };
        buffer.unmap();
        self.ring[slot] = Some(buffer);
        out
    }
}
