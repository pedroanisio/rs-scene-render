//! Particle drawing: instanced quads (disc, square, sprite-sheet cell,
//! velocity streak) rendered premultiplied into a target-space texture that
//! composites as the emitter's layer.

use std::collections::HashMap;
use std::sync::Arc;

use wgpu::util::DeviceExt;

use crate::resources::FORMAT;

/// One particle instance.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Inst {
    pub centre_half: [f32; 4],
    pub rot_shape: [f32; 4],
    pub color: [f32; 4],
    pub cell: [f32; 4],
}

/// A particle pass: instances in target pixels, with an optional sprite sheet (its asset id
/// and texture).
pub struct ParticleJob {
    pub insts: Vec<Inst>,
    pub sprite: Option<(String, wgpu::TextureView)>,
}

/// GPU state for particles.
pub struct ParticleEngine {
    device: Arc<wgpu::Device>,
    pipe: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    smp: wgpu::Sampler,
    white: wgpu::TextureView,
    /// Bind groups by sprite texture and target size. The texture, not the asset id: two
    /// documents' assets may share an id, and one asset decodes differently by colour settings.
    binds: HashMap<(Option<wgpu::TextureView>, [u32; 2]), (wgpu::BindGroup, wgpu::Buffer)>,
}

impl ParticleEngine {
    pub fn new(device: Arc<wgpu::Device>, queue: &wgpu::Queue) -> ParticleEngine {
        let d = &*device;
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particles"),
            source: wgpu::ShaderSource::Wgsl(include_str!("particles.wgsl").into()),
        });
        let vs_fs = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particles"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: vs_fs,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: vs_fs,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: vs_fs,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particles"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        const ATTRS: [wgpu::VertexAttribute; 4] =
            wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4];
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
        let pipe = {
            let _creation = crate::gpu::creation_lock();
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("particles"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Inst>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &ATTRS,
                    })],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend: Some(premul),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let smp = d.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let white_t = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("particle-white"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &white_t,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let white = white_t.create_view(&Default::default());
        ParticleEngine { device, pipe, bgl, smp, white, binds: HashMap::new() }
    }

    /// Records the particles of `job` into `target` (cleared first).
    pub fn record(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        job: &ParticleJob,
        target: &wgpu::TextureView,
        size: [u32; 2],
    ) {
        let d = self.device.clone();
        let key = (job.sprite.as_ref().map(|s| s.1.clone()), size);
        if !self.binds.contains_key(&key) {
            let buf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("particle-target"),
                contents: bytemuck::cast_slice(&[size[0] as f32, size[1] as f32, 0.0, 0.0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let view = job.sprite.as_ref().map(|s| &s.1).unwrap_or(&self.white);
            let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("particles"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.smp) },
                ],
            });
            if self.binds.len() > 64 {
                self.binds.clear();
            }
            self.binds.insert(key.clone(), (bg, buf));
        }
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("particles"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        if job.insts.is_empty() {
            return;
        }
        let ibuf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("particles"),
            contents: bytemuck::cast_slice(&job.insts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        rp.set_pipeline(&self.pipe);
        rp.set_bind_group(0, &self.binds[&key].0, &[]);
        rp.set_vertex_buffer(0, ibuf.slice(..));
        rp.draw(0..6, 0..job.insts.len() as u32);
    }
}
