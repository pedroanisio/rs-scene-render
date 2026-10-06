//! GPU fine stage of the vector rasteriser and paint conversion. The tile
//! encoder (`sr_vector::tile`) runs on the CPU; this module turns its
//! output into buffers, resolves its paints into the frame's paint table,
//! and dispatches `raster.wgsl` into a storage texture.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use sr_vector::scene::{GradientKind, Paint};
use sr_vector::tile::Encoded;

use crate::color::{self, Working};
use crate::paint::PaintTable;
use crate::resources::Tex;
use crate::types::{PaintDesc, Stop};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
struct RCmd {
    kind: u32,
    piece_off: u32,
    piece_count: u32,
    backdrop: u32,
    backdrop_val: f32,
    paint: u32,
    flags: u32,
    param: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, Default)]
struct RParams {
    tiles_x: u32,
    tiles_y: u32,
    width: u32,
    height: u32,
    active_count: u32,
    pad: [u32; 3],
}

/// A rasterisation job: encoded tiles with paints resolved, and its target.
pub struct RasterJob {
    pub target: Arc<Tex>,
    size: [u32; 2],
    tiles: [u32; 2],
    ranges: Vec<[u32; 2]>,
    cmds: Vec<RCmd>,
    pieces: Vec<[f32; 4]>,
    backdrops: Vec<f32>,
    /// No layer stack or gradient evaluation is needed by any command.
    solid_fills: bool,
    active_tiles: Option<Vec<u32>>,
}

/// An sRGB-encoded straight colour into the stored working representation.
pub fn srgb_to_working(w: &Working, c: [f64; 4]) -> [f64; 4] {
    let d = |v: f64| color::decode(sr_model::model::Transfer::Srgb, v);
    w.from_linear_srgb([d(c[0]), d(c[1]), d(c[2]), c[3]])
}

fn xf_desc(x: &sr_vector::Xf) -> ([f32; 4], [f32; 4]) {
    let [a, b, c, d, e, f] = x.0;
    ([a as f32, b as f32, c as f32, d as f32], [e as f32, f as f32, 0.0, 0.0])
}

/// Adds a vector paint to the frame's paint table and returns its index.
pub fn resolve_paint(t: &mut PaintTable, w: &Working, p: &Paint) -> u32 {
    match p {
        Paint::Solid { rgba, srgb } => {
            let stored = if *srgb { srgb_to_working(w, *rgba) } else { w.from_literal(*rgba) };
            t.stops.push(Stop { color: stored.map(|x| x as f32), offset: 0.0, midpoint: 0.5, pad: [0.0; 2] });
            t.paints.push(PaintDesc {
                kind: 0,
                stop_off: (t.stops.len() - 1) as u32,
                stop_count: 1,
                xform0: [1.0, 0.0, 0.0, 1.0],
                ..Default::default()
            });
        }
        Paint::Gradient(g) => {
            let off = t.stops.len() as u32;
            for (o, c) in &g.stops {
                t.stops.push(Stop {
                    color: srgb_to_working(w, *c).map(|x| x as f32),
                    offset: *o as f32,
                    midpoint: 0.5,
                    pad: [0.0; 2],
                });
            }
            let (xform0, xform1) = xf_desc(&g.to_gradient);
            let (kind, p0, p1) = match g.kind {
                GradientKind::Linear { a, b } => (1, [a.x, a.y, b.x, b.y], [0.0; 4]),
                GradientKind::Radial { c, r, f, fr } => (2, [c.x, c.y, r, 1.0], [f.x, f.y, fr, 0.0]),
                GradientKind::Conic { c, start } => (3, [c.x, c.y, start, 0.0], [0.0; 4]),
            };
            t.paints.push(PaintDesc {
                xform0,
                xform1,
                p0: p0.map(|v| v as f32),
                p1: p1.map(|v| v as f32),
                kind,
                spread: g.spread,
                // imported gradients interpolate in sRGB, as SVG and Lottie do
                space: 1,
                stop_off: off,
                stop_count: g.stops.len() as u32,
                ..Default::default()
            });
        }
        Paint::External { index, to_local } => {
            let mut d = t.paints.get(*index as usize).copied().unwrap_or_default();
            // compose: target pixel → paint-local → paint space
            let [a, b, c, dd] = d.xform0.map(|v| v as f64);
            let [e, f, _, _] = d.xform1.map(|v| v as f64);
            let inner = sr_vector::Xf([a, b, c, dd, e, f]).mul(to_local);
            (d.xform0, d.xform1) = xf_desc(&inner);
            t.paints.push(d);
        }
    }
    (t.paints.len() - 1) as u32
}

impl RasterJob {
    /// Resolves an encoded scene's paints and prepares its buffers.
    pub fn new(e: Encoded, target: Arc<Tex>, t: &mut PaintTable, w: &Working) -> RasterJob {
        let map: Vec<u32> = e.paints.iter().map(|p| resolve_paint(t, w, p)).collect();
        let solid_fills = e.cmds.iter().all(|c| c.kind == sr_vector::tile::kind::FILL)
            && map.iter().all(|&i| t.paints[i as usize].kind == 0);
        let cmds = e
            .cmds
            .iter()
            .map(|c| RCmd {
                kind: c.kind,
                piece_off: c.piece_off,
                piece_count: c.piece_count,
                backdrop: c.backdrop,
                backdrop_val: c.backdrop_val,
                paint: if c.kind == sr_vector::tile::kind::FILL {
                    map.get(c.paint as usize).copied().unwrap_or(0)
                } else {
                    0
                },
                flags: c.flags,
                param: c.param,
            })
            .collect();
        let nonempty = e.ranges.iter().filter(|r| r[1] != 0).count();
        let active_tiles = (nonempty * 2 < e.ranges.len())
            .then(|| e.ranges.iter().enumerate().filter_map(|(i, r)| (r[1] != 0).then_some(i as u32)).collect());
        RasterJob {
            target,
            size: e.size,
            tiles: e.tiles,
            ranges: e.ranges,
            cmds,
            pieces: e.pieces,
            backdrops: e.backdrops,
            solid_fills,
            active_tiles,
        }
    }
}

/// The fine-stage pipeline.
pub struct Raster {
    pipelines: [wgpu::ComputePipeline; 4],
    bgl: wgpu::BindGroupLayout,
}

fn storage<T: Pod>(d: &wgpu::Device, data: &[T], label: &str) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    let bytes: &[u8] = bytemuck::cast_slice(data);
    let mut v = bytes.to_vec();
    // an empty scene still binds one zeroed element of every record type
    if v.len() < 64 {
        v.resize(64, 0);
    }
    d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: &v,
        usage: wgpu::BufferUsages::STORAGE,
    })
}

impl Raster {
    /// Builds the pipeline.
    pub fn new(d: &wgpu::Device) -> Raster {
        let st = |b: u32| wgpu::BindGroupLayoutEntry {
            binding: b,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let un = |b: u32| wgpu::BindGroupLayoutEntry {
            binding: b,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let bgl = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raster"),
            entries: &[
                st(0),
                st(1),
                st(2),
                st(3),
                st(4),
                st(5),
                st(9),
                un(6),
                un(7),
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: crate::resources::FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("raster"),
            source: wgpu::ShaderSource::Wgsl(concat!(include_str!("common.wgsl"), include_str!("raster.wgsl")).into()),
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raster"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipelines = std::array::from_fn(|i| {
            let _creation = crate::gpu::creation_lock();
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("raster"),
                layout: Some(&layout),
                module: &module,
                entry_point: Some("raster_main"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[("SOLID_FILLS", (i & 1) as f64), ("SPARSE_TILES", (i >> 1) as f64)],
                    ..Default::default()
                },
                cache: None,
            })
        });
        Raster { pipelines, bgl }
    }

    /// Records a job into `enc`, reading the frame's paint buffers.
    pub fn record(
        &self,
        d: &wgpu::Device,
        enc: &mut wgpu::CommandEncoder,
        job: &RasterJob,
        paints: &wgpu::Buffer,
        stops: &wgpu::Buffer,
        globals: &wgpu::Buffer,
    ) {
        use wgpu::util::DeviceExt;
        let view =
            job.target.tex.create_view(&wgpu::TextureViewDescriptor { mip_level_count: Some(1), ..Default::default() });
        if let Some(active) = &job.active_tiles {
            // A clear covers empty tiles more cheaply than launching a pixel
            // invocation for each of them. Every populated tile is overwritten.
            drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear sparse raster"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
            }));
            if active.is_empty() {
                return;
            }
        }
        let params = RParams {
            tiles_x: job.tiles[0],
            tiles_y: job.tiles[1],
            width: job.size[0],
            height: job.size[1],
            active_count: job.active_tiles.as_ref().map_or(0, |a| a.len() as u32),
            pad: [0; 3],
        };
        let pb = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("raster params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let (c, r, p, b) = (
            storage(d, &job.cmds, "rcmds"),
            storage(d, &job.ranges, "ranges"),
            storage(d, &job.pieces, "pieces"),
            storage(d, &job.backdrops, "backdrops"),
        );
        let active = job.active_tiles.as_ref().map(|tiles| storage(d, tiles, "active tiles"));
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raster"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: c.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: r.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: p.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: b.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: paints.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: stops.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: pb.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 9, resource: active.as_ref().unwrap_or(&r).as_entire_binding() },
            ],
        });
        let mut pass =
            enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("raster"), timestamp_writes: None });
        pass.set_pipeline(&self.pipelines[job.solid_fills as usize + 2 * job.active_tiles.is_some() as usize]);
        pass.set_bind_group(0, &bind, &[]);
        if job.active_tiles.is_some() {
            let n = params.active_count;
            pass.dispatch_workgroups(n.min(65535), n.div_ceil(65535), 1);
        } else {
            pass.dispatch_workgroups(job.tiles[0], job.tiles[1], 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_fill_specialization_matches_general_rasterization_exactly() {
        let gpu = match crate::Gpu::new() {
            Ok(gpu) => gpu,
            Err(error) => {
                assert!(std::env::var("SR_REQUIRE_GPU").as_deref() != Ok("1"), "{error}");
                eprintln!("skipping GPU test: {error}");
                return;
            }
        };
        let d = &gpu.device;
        let layout = crate::resources::source_layout(d);
        // Partial edge tiles, an empty tile, fractional edge coverage, overlapping
        // translucent paints, and both winding rules exercise the shared arithmetic.
        let target = Arc::new(crate::resources::create(d, &layout, [31, 19], 1, "raster oracle"));
        let mut job = RasterJob {
            target: target.clone(),
            size: target.size,
            tiles: [2, 2],
            ranges: vec![[0, 3], [0, 0], [1, 2], [0, 3]],
            cmds: vec![
                RCmd { backdrop: u32::MAX, backdrop_val: 0.25, param: 0.75, ..Default::default() },
                RCmd { backdrop: 0, piece_count: 1, paint: 1, param: 0.5, ..Default::default() },
                RCmd { backdrop: u32::MAX, backdrop_val: 1.4, flags: 1, param: 0.9, ..Default::default() },
            ],
            pieces: vec![[0.3, 0.1, 12.7, 15.9]],
            backdrops: (0..16).map(|y| y as f32 / 32.0).collect(),
            solid_fills: false,
            active_tiles: None,
        };
        let paints = storage(d, &[PaintDesc::default(), PaintDesc { stop_off: 1, ..Default::default() }], "paints");
        let stops = storage(
            d,
            &[
                Stop { color: [0.1, 0.7, 0.3, 0.65], ..Default::default() },
                Stop { color: [0.9, 0.2, 0.4, 0.35], ..Default::default() },
            ],
            "stops",
        );
        let globals = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<crate::types::Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let document = sr_model::load_str(
            r#"<scene version="1.2"><project width="31" height="19" fps="1" duration="1"/><composition/></scene>"#,
            &sr_model::LoadOptions::default(),
        )
        .unwrap();
        let evaluator = sr_eval::Evaluator::new(&document, &sr_eval::EvalOptions::default()).unwrap();
        let reader = crate::Renderer::new(gpu.clone(), evaluator.program());
        let raster = Raster::new(d);
        let render = |job: &RasterJob| {
            let mut encoder = d.create_command_encoder(&Default::default());
            raster.record(d, &mut encoder, job, &paints, &stops, &globals);
            gpu.queue.submit([encoder.finish()]);
            reader.read(&job.target)
        };
        let general = render(&job);
        job.solid_fills = true;
        let specialized = render(&job);
        assert!(general.iter().any(|p| p[3] > 0.0));
        assert!(general.contains(&[0.0; 4]));
        assert_eq!(specialized, general);
        job.active_tiles = Some(vec![0, 2, 3]);
        assert_eq!(render(&job), general, "sparse solid tiles");
        job.solid_fills = false;
        assert_eq!(render(&job), general, "sparse general tiles");
        job.active_tiles = Some(Vec::new());
        assert!(render(&job).iter().all(|p| *p == [0.0; 4]), "empty jobs clear stale texels");
    }
}
