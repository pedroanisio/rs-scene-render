//! Rendering a Schwarzschild black hole with a thin accretion disk by tracing null geodesics.
//!
//! The units are G = c = 1: lengths are in the scene's units, the hole's radius is 2 `mass`, the
//! innermost stable circular orbit 6 `mass`, and the geometric time is a length. See
//! `geodesic.wgsl` for the method. The pass draws the whole frame (a hole, its disk and a star
//! field behind them), in tiles when asked, and nothing else of a 3D scene.

use glam::Vec3;
use wgpu::util::DeviceExt;

/// How the brightness of the disk varies with radius and azimuth, apart from its temperature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// A smooth disk.
    None,
    /// Seeded clumps carried round by the disk's Keplerian rotation.
    Clumps,
    /// Seeded logarithmic spiral arms carried round by the same rotation.
    Spiral,
}

/// A thin, opaque, Keplerian disk in the plane through the hole's centre perpendicular to `axis`.
#[derive(Clone, Copy, Debug)]
pub struct Disk {
    /// Inner radius; 6 `mass` is the innermost stable orbit.
    pub inner: f32,
    pub outer: f32,
    /// Highest local temperature of the disk (kelvin), reached at 49/36 of the inner radius.
    pub peak_kelvin: f32,
    /// Scale of the emitted radiance.
    pub intensity: f32,
    /// Strength of the pattern, 0 to 1.
    pub contrast: f32,
    pub pattern: Pattern,
    pub seed: u32,
    /// Direction of the spin: the disk turns counter-clockwise seen from where this points.
    pub axis: Vec3,
    /// Geometric time of the frame: the pattern turns by `sqrt(mass / r^3)` radians per unit of it.
    pub time: f32,
}

/// The frame to draw.
#[derive(Clone, Debug)]
pub struct GeodesicScene {
    pub size: [u32; 2],
    /// The observer, at rest at a distance of more than 3 `mass` from the hole.
    pub eye: Vec3,
    pub right: Vec3,
    /// Toward the bottom of the image.
    pub down: Vec3,
    pub forward: Vec3,
    pub focal_px: f32,
    pub hole: Vec3,
    pub mass: f32,
    pub disk: Option<Disk>,
    /// Rays per pixel, spread over it.
    pub samples: u32,
    pub star_seed: u32,
}

/// What a pass writes: the picture, or per-pixel numbers for checking the tracing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Picture,
    /// (outcome 0 captured / 1 escaped, escape angle, radius of crossing 0, of crossing 1), one ray
    /// through each pixel's centre, crossings of the disk's plane at any radius.
    Crossings,
    /// (radius of crossing 2, of crossing 3, crossings counted, angle of the first crossing).
    MoreCrossings,
    /// (g, radius, azimuth, order) of the first crossing inside the disk, or (0, -1, 0, -1).
    FirstHit,
}

const BLACKBODY_ENTRIES: usize = sr_volume::thermal::TABLE_INTERVALS + 1;

/// The pipeline of the pass.
pub struct GeodesicGpu {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    blackbody: wgpu::Buffer,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    eye: [f32; 4],
    hole: [f32; 4],
    right: [f32; 4],
    down: [f32; 4],
    fwd: [f32; 4],
    dx: [f32; 4],
    dy: [f32; 4],
    dz: [f32; 4],
    size: [f32; 4],
    misc: [u32; 4],
}

fn v4(v: Vec3, w: f32) -> [f32; 4] {
    [v.x, v.y, v.z, w]
}

/// Two unit vectors that with `axis` (made a unit vector) form a right-handed frame.
fn plane_axes(axis: Vec3) -> (Vec3, Vec3, Vec3) {
    let z = axis.normalize_or(Vec3::NEG_Y);
    let helper = if z.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let x = (helper - z * helper.dot(z)).normalize();
    (x, z.cross(x), z)
}

/// The most rows a pass draws at once; a taller frame is drawn in bands.
const BAND_ROWS: u32 = 128;

impl GeodesicGpu {
    pub fn new(d: &wgpu::Device, format: wgpu::TextureFormat) -> GeodesicGpu {
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("geodesic"),
            source: wgpu::ShaderSource::Wgsl(include_str!("geodesic.wgsl").into()),
        });
        let uniform = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("geodesic"),
            entries: &[uniform(0), uniform(1)],
        });
        let pipeline_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("geodesic"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("geodesic"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let table: Vec<[f32; 4]> = sr_volume::thermal::blackbody_table()
            .iter()
            .map(|rgb| [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, 0.0])
            .collect();
        debug_assert_eq!(table.len(), BLACKBODY_ENTRIES);
        let blackbody = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("geodesic-blackbody"),
            contents: bytemuck::cast_slice(&table),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        GeodesicGpu { pipeline, layout, blackbody }
    }

    /// Draws `scene` into `out`, a texture of the scene's size, in bands of at most `rows` rows
    /// (all of them when `None`): every pixel is computed from its own coordinates alone, so the
    /// bands change nothing but the time one submission takes.
    pub fn render(
        &self,
        d: &wgpu::Device,
        enc: &mut wgpu::CommandEncoder,
        scene: &GeodesicScene,
        output: Output,
        rows: Option<u32>,
        out: &wgpu::TextureView,
    ) {
        let (pattern, disk) = match &scene.disk {
            Some(disk) => (
                match disk.pattern {
                    Pattern::None => 0,
                    Pattern::Clumps => 1,
                    Pattern::Spiral => 2,
                },
                *disk,
            ),
            None => (
                3,
                Disk {
                    inner: 6.0 * scene.mass,
                    outer: 7.0 * scene.mass,
                    peak_kelvin: 0.0,
                    intensity: 0.0,
                    contrast: 0.0,
                    pattern: Pattern::None,
                    seed: 0,
                    axis: Vec3::NEG_Y,
                    time: 0.0,
                },
            ),
        };
        let (x, y, z) = plane_axes(disk.axis);
        let mode = match output {
            Output::Picture => 0.0,
            Output::Crossings => 1.0,
            Output::MoreCrossings => 2.0,
            Output::FirstHit => 3.0,
        };
        // the rays of every crossing are wanted for checks; the picture stops at the first in the disk
        let flags = u32::from(output != Output::Picture);
        let params = Params {
            eye: v4(scene.eye, scene.mass),
            hole: v4(scene.hole, disk.inner),
            right: v4(scene.right.normalize(), scene.focal_px),
            down: v4(scene.down.normalize(), scene.samples.max(1) as f32),
            fwd: v4(scene.forward.normalize(), mode),
            dx: v4(x, disk.outer),
            dy: v4(y, disk.peak_kelvin),
            dz: v4(z, disk.intensity),
            size: [scene.size[0] as f32, scene.size[1] as f32, disk.contrast, disk.time],
            misc: [pattern, disk.seed, flags, scene.star_seed],
        };
        let uniforms = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("geodesic-params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("geodesic"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.blackbody.as_entire_binding() },
            ],
        });
        let band = rows.unwrap_or(BAND_ROWS.max(scene.size[1])).max(1);
        let mut first = 0;
        let mut y0 = 0;
        while y0 < scene.size[1] {
            let h = band.min(scene.size[1] - y0);
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("geodesic"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: out,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if first == 0 { wgpu::LoadOp::Clear(wgpu::Color::BLACK) } else { wgpu::LoadOp::Load },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_scissor_rect(0, y0, scene.size[0], h);
            pass.draw(0..3, 0..1);
            drop(pass);
            first += 1;
            y0 += h;
        }
    }
}
