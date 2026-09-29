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
    pad: [u32; 3],
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
    samp: wgpu::Sampler,
    /// Parameters, rewritten before every submission (queue writes land after earlier work).
    params: wgpu::Buffer,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: c,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
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
            ],
        });
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
            samp,
            params,
            packed: None,
            ring: [None, None, None],
            head: 0,
            frame_index: 0,
        }
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
        let d = self.device.clone();
        let bytes = format.frame_bytes(size[0], size[1]);
        let words = bytes.div_ceil(4) as u32;
        let (fw, fh) = (frame.size[0] as f32, frame.size[1] as f32);
        let scale = (size[0] as f32 / fw).min(size[1] as f32 / fh);
        let offset = [(size[0] as f32 - fw * scale) / 2.0, (size[1] as f32 - fh * scale) / 2.0];
        let store = color::default_transfer(working.space);
        let p = Params {
            to_out: types::mat3(&color::convert(working.space, out.space)),
            fsize: frame.size,
            osize: size,
            offset,
            scale,
            format: match format {
                InputFormat::Nv12 => 0,
                InputFormat::P010 => 1,
                InputFormat::Rgba8 => 2,
                InputFormat::Rgba16 => 3,
                InputFormat::Gbrapf32 => 4,
            },
            transfer: color::transfer_id(out.transfer),
            full_range: out.full_range as u32,
            linear_light: working.linear as u32,
            store_transfer: color::transfer_id(if store == Transfer::Linear { Transfer::Srgb } else { store }),
            keep_alpha: keep_alpha as u32,
            kr: out.matrix.0,
            kb: out.matrix.1,
            words,
            seed: self.frame_index,
            pad: [0; 3],
        };
        self.frame_index = self.frame_index.wrapping_add(1);
        // The queue orders this write after every earlier submission, so the previous
        // frame's pack pass reads its own parameters before these land.
        self.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
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
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&frame.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.samp) },
                wgpu::BindGroupEntry { binding: 3, resource: storage.as_entire_binding() },
            ],
        });
        let mut enc = d.create_command_encoder(&Default::default());
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
