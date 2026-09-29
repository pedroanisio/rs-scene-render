//! Progressive path tracing of a 3D pass (camera `renderer="pathtrace"`).
//!
//! The same [`Scene3`] the rasteriser draws is flattened on the CPU into world-space triangles
//! (instances expanded, deformed vertices used), materials and lights, and a bounding volume
//! hierarchy is built over the triangles with the surface area heuristic (binned, 12 bins).
//! `pathtrace.wgsl` then traces `samples` paths per pixel in compute dispatches of a few
//! samples each: camera rays (thin lens for depth of field), next-event estimation toward the
//! analytic lights with shadow rays, a metallic-roughness BSDF (Lambert plus GGX, sampled by
//! lobe), smooth dielectric transmission, Russian roulette, and the dome environment and ambient
//! light for rays that leave the scene. Rays that leave through glass see the 2D layers behind
//! the pass where they cross the composition plane (z = 0). An edge-aware à-trous filter
//! (Dammertz et al. 2010) guided by first-hit normals and albedo optionally denoises the result.
//!
//! Not traced (reported): texture maps (their materials' factors apply), splats, IES profiles,
//! lens distortion, orthographic cameras and passes inside resized offscreens (these fall back
//! to the rasteriser).

use glam::{Mat4, Vec3};

use crate::three::{LightKind, Scene3};

/// Path-tracing options of a pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathOpts {
    pub samples: u32,
    pub bounces: u32,
    pub denoise: bool,
}

/// A material as the tracer reads it (16-byte rows).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PtMat {
    /// Linear base colour, opacity.
    pub base: [f32; 4],
    /// metallic, roughness, transmission, ior
    pub params: [f32; 4],
    /// emitted radiance, unlit flag
    pub emissive: [f32; 4],
    /// specular weight (KHR_materials_specular), unused ×3
    pub extra: [f32; 4],
}

/// A BVH node: bounds, and either (first triangle, count) for a leaf or (second child, 0) for an
/// interior node, whose first child is the second child's index − 1.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PtNode {
    pub lo: [f32; 3],
    pub a: u32,
    pub hi: [f32; 3],
    pub b: u32,
}

/// A light as the tracer reads it.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PtLight {
    /// position, kind (0 ambient, 1 directional, 2 point, 3 spot, 4 rect, 5 disk, 6 sphere)
    pub pos: [f32; 4],
    /// direction of travel, range (0 = unbounded)
    pub dir: [f32; 4],
    /// radiance or intensity, distance falloff exponent
    pub color: [f32; 4],
    /// cos outer, cos inner, width, height
    pub spot: [f32; 4],
    /// radius, casts shadows, unused ×2
    pub size: [f32; 4],
    /// right axis (rect lights), unused
    pub right: [f32; 4],
}

/// The flattened scene.
#[derive(Default)]
pub struct PtScene {
    /// World-space positions and normals of every triangle corner (three per triangle).
    pub pos: Vec<[f32; 4]>,
    pub nrm: Vec<[f32; 4]>,
    /// Material of each triangle.
    pub tri_mat: Vec<u32>,
    pub mats: Vec<PtMat>,
    pub nodes: Vec<PtNode>,
    pub lights: Vec<PtLight>,
    /// What the tracer leaves out, for the render notes.
    pub notes: Vec<String>,
}

fn srgb_luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// What the tracer leaves out of `scene`.
pub fn notes(scene: &Scene3) -> Vec<String> {
    let mut n = Vec::new();
    if scene.draws.iter().any(|d| d.maps.iter().any(|t| t.is_some())) {
        n.push("path tracing: texture maps are not traced; their materials' factors apply".into());
    }
    if !scene.splats.is_empty() {
        n.push("path tracing: Gaussian splats are not traced".into());
    }
    if scene.lights.iter().any(|l| l.ies.is_some()) {
        n.push("path tracing: IES profiles are not traced".into());
    }
    if scene.lens_k1 != 0.0 {
        n.push("path tracing: lens distortion is not traced".into());
    }
    n
}

/// Flattens `scene` into triangles, materials and lights, and builds the BVH.
pub fn build(scene: &Scene3) -> PtScene {
    let mut s = PtScene::default();
    let mut tris: Vec<[Vec3; 3]> = Vec::new();
    let mut norms: Vec<[Vec3; 3]> = Vec::new();
    let mut tmat: Vec<u32> = Vec::new();
    let mut textured = false;
    for dr in &scene.draws {
        let m = &dr.material;
        textured |= dr.maps.iter().any(|t| t.is_some());
        let unlit = m.unlit;
        let emissive = if unlit {
            [m.base_color[0], m.base_color[1], m.base_color[2]]
        } else {
            [
                m.emissive[0] * m.emissive_strength,
                m.emissive[1] * m.emissive_strength,
                m.emissive[2] * m.emissive_strength,
            ]
        };
        let opacity =
            if m.alpha_mode == sr_3d::AlphaMode::Blend { m.base_color[3] * m.opacity } else { 1.0 } * dr.opacity;
        let mi = s.mats.len() as u32;
        s.mats.push(PtMat {
            base: [m.base_color[0], m.base_color[1], m.base_color[2], opacity.clamp(0.0, 1.0)],
            params: [
                m.metallic.clamp(0.0, 1.0),
                m.roughness.clamp(0.02, 1.0),
                m.transmission.clamp(0.0, 1.0),
                m.ior.max(1.0),
            ],
            emissive: [emissive[0], emissive[1], emissive[2], unlit as u32 as f32],
            extra: [m.specular * srgb_luma(m.specular_color).max(0.0), 0.0, 0.0, 0.0],
        });
        let (verts, idx) = dr.mesh.cpu();
        let nmat = dr.model.inverse().transpose();
        // each copy of an instanced object is its own draw
        let model = dr.model;
        for t in idx.chunks_exact(3) {
            let p: [Vec3; 3] = std::array::from_fn(|c| model.transform_point3(Vec3::from(verts[t[c] as usize].pos)));
            let nn: [Vec3; 3] = std::array::from_fn(|c| {
                nmat.transform_vector3(Vec3::from(verts[t[c] as usize].normal)).normalize_or_zero()
            });
            tris.push(p);
            norms.push(nn);
            tmat.push(mi);
        }
    }
    let _ = textured;
    s.notes = notes(scene);
    // BVH over the triangles, then triangles in leaf order
    let order = bvh(&tris, &mut s.nodes);
    for &t in &order {
        for c in 0..3 {
            let p = tris[t][c];
            let q = norms[t][c];
            s.pos.push([p.x, p.y, p.z, 0.0]);
            s.nrm.push([q.x, q.y, q.z, 0.0]);
        }
        s.tri_mat.push(tmat[t]);
    }
    for l in &scene.lights {
        let kind = match l.kind {
            LightKind::Ambient => 0.0,
            LightKind::Directional => 1.0,
            LightKind::Point => 2.0,
            LightKind::Spot => 3.0,
            LightKind::Rect => 4.0,
            LightKind::Disk => 5.0,
            LightKind::Sphere => 6.0,
        };
        s.lights.push(PtLight {
            pos: [l.pos.x, l.pos.y, l.pos.z, kind],
            dir: [l.dir.x, l.dir.y, l.dir.z, l.range],
            color: [l.color.x, l.color.y, l.color.z, l.falloff],
            spot: [l.cos_outer, l.cos_inner, l.size[0], l.size[1]],
            size: [l.size[2], l.cast_shadow as u32 as f32, 0.0, 0.0],
            right: [l.right.x, l.right.y, l.right.z, 0.0],
        });
    }
    if s.lights.is_empty() {
        s.lights.push(PtLight {
            pos: [0.0, 0.0, 0.0, -1.0],
            dir: [0.0; 4],
            color: [0.0; 4],
            spot: [0.0; 4],
            size: [0.0; 4],
            right: [0.0; 4],
        });
    }
    if s.mats.is_empty() {
        s.mats.push(PtMat { base: [0.0; 4], params: [0.0; 4], emissive: [0.0; 4], extra: [0.0; 4] });
    }
    if s.pos.is_empty() {
        s.pos.push([0.0; 4]);
        s.nrm.push([0.0; 4]);
        s.tri_mat.push(0);
    }
    s
}

/// Binned SAH BVH. Returns the triangle order its leaves index.
fn bvh(tris: &[[Vec3; 3]], nodes: &mut Vec<PtNode>) -> Vec<usize> {
    let n = tris.len();
    let mut idx: Vec<usize> = (0..n).collect();
    nodes.clear();
    if n == 0 {
        nodes.push(PtNode { lo: [1.0; 3], a: 0, hi: [-1.0; 3], b: 1 });
        return idx;
    }
    let bounds: Vec<(Vec3, Vec3)> = tris.iter().map(|t| (t[0].min(t[1]).min(t[2]), t[0].max(t[1]).max(t[2]))).collect();
    let cent: Vec<Vec3> = bounds.iter().map(|(l, h)| (*l + *h) * 0.5).collect();
    fn area(lo: Vec3, hi: Vec3) -> f32 {
        let d = (hi - lo).max(Vec3::ZERO);
        2.0 * (d.x * d.y + d.y * d.z + d.z * d.x)
    }
    // (node index, start, end) to build; a node's second child index is patched when known
    let mut stack = vec![(0usize, 0usize, n)];
    nodes.push(PtNode::default());
    while let Some((ni, start, end)) = stack.pop() {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let (mut clo, mut chi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for &t in &idx[start..end] {
            lo = lo.min(bounds[t].0);
            hi = hi.max(bounds[t].1);
            clo = clo.min(cent[t]);
            chi = chi.max(cent[t]);
        }
        let count = end - start;
        let leaf = |nodes: &mut Vec<PtNode>| {
            nodes[ni] = PtNode { lo: lo.into(), a: start as u32, hi: hi.into(), b: count as u32 };
        };
        if count <= 4 {
            leaf(nodes);
            continue;
        }
        // best binned split over the three axes
        const BINS: usize = 12;
        let mut best: Option<(usize, f32, f32)> = None;
        let ext = chi - clo;
        for axis in 0..3 {
            if ext[axis] <= 1e-9 {
                continue;
            }
            let mut bin_n = [0usize; BINS];
            let mut bin_b = [(Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)); BINS];
            for &t in &idx[start..end] {
                let k = (((cent[t][axis] - clo[axis]) / ext[axis]) * BINS as f32).min(BINS as f32 - 1.0) as usize;
                bin_n[k] += 1;
                bin_b[k].0 = bin_b[k].0.min(bounds[t].0);
                bin_b[k].1 = bin_b[k].1.max(bounds[t].1);
            }
            for split in 1..BINS {
                let (mut ln, mut rn) = (0, 0);
                let (mut ll, mut lh, mut rl, mut rh) =
                    (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN), Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                for k in 0..split {
                    ln += bin_n[k];
                    ll = ll.min(bin_b[k].0);
                    lh = lh.max(bin_b[k].1);
                }
                for k in split..BINS {
                    rn += bin_n[k];
                    rl = rl.min(bin_b[k].0);
                    rh = rh.max(bin_b[k].1);
                }
                if ln == 0 || rn == 0 {
                    continue;
                }
                let cost = ln as f32 * area(ll, lh) + rn as f32 * area(rl, rh);
                if best.map(|b| cost < b.1).unwrap_or(true) {
                    best = Some((axis, cost, clo[axis] + ext[axis] * split as f32 / BINS as f32));
                }
            }
        }
        let Some((axis, cost, pos)) = best else {
            leaf(nodes);
            continue;
        };
        if cost >= count as f32 * area(lo, hi) && count <= 16 {
            leaf(nodes);
            continue;
        }
        let mid = {
            let s = &mut idx[start..end];
            let mut i = 0;
            for j in 0..s.len() {
                if cent[s[j]][axis] < pos {
                    s.swap(i, j);
                    i += 1;
                }
            }
            start + i
        };
        if mid == start || mid == end {
            leaf(nodes);
            continue;
        }
        // the children are allocated as a pair: an interior node keeps the second's index in
        // `a` (and `b` = 0); the first is `a` − 1
        let left = nodes.len();
        nodes.push(PtNode::default());
        let right = nodes.len();
        nodes.push(PtNode::default());
        nodes[ni] = PtNode { lo: lo.into(), a: right as u32, hi: hi.into(), b: 0 };
        stack.push((right, mid, end));
        stack.push((left, start, mid));
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Brute-force nearest hit versus BVH traversal on the CPU.
    fn hit(t: &[Vec3; 3], o: Vec3, d: Vec3) -> Option<f32> {
        let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
        let p = d.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-9 {
            return None;
        }
        let s = o - t[0];
        let u = s.dot(p) / det;
        let q = s.cross(e1);
        let v = d.dot(q) / det;
        let tt = e2.dot(q) / det;
        (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && tt > 1e-4).then_some(tt)
    }

    fn traverse(nodes: &[PtNode], tris: &[[Vec3; 3]], o: Vec3, d: Vec3) -> Option<f32> {
        let mut best: Option<f32> = None;
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            let nd = nodes[i];
            let (lo, hi) = (Vec3::from(nd.lo), Vec3::from(nd.hi));
            let inv = d.recip();
            let (t0, t1) = ((lo - o) * inv, (hi - o) * inv);
            let (tn, tf) = (t0.min(t1).max_element(), t0.max(t1).min_element());
            if tf < tn.max(0.0) || best.map(|b| tn > b).unwrap_or(false) {
                continue;
            }
            if nd.b > 0 {
                for t in &tris[nd.a as usize..(nd.a + nd.b) as usize] {
                    if let Some(h) = hit(t, o, d) {
                        best = Some(best.map_or(h, |b| b.min(h)));
                    }
                }
            } else {
                stack.push(nd.a as usize);
                stack.push(nd.a as usize - 1);
            }
        }
        best
    }

    #[test]
    fn bvh_traversal_finds_the_same_nearest_hits_as_brute_force() {
        let mut tris = Vec::new();
        let mut seed = 1u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
        };
        for _ in 0..3000 {
            let c = Vec3::new(rnd(), rnd(), rnd()) * 100.0;
            tris.push([c, c + Vec3::new(rnd(), rnd(), rnd()) * 6.0, c + Vec3::new(rnd(), rnd(), rnd()) * 6.0]);
        }
        let mut nodes = Vec::new();
        let order = bvh(&tris, &mut nodes);
        let sorted: Vec<[Vec3; 3]> = order.iter().map(|&k| tris[k]).collect();
        let mut hits = 0;
        for _ in 0..500 {
            let o = Vec3::new(rnd(), rnd(), rnd()) * 150.0;
            let d = Vec3::new(rnd(), rnd(), rnd()).normalize_or(Vec3::X);
            let brute = tris
                .iter()
                .filter_map(|t| hit(t, o, d))
                .fold(None, |b: Option<f32>, h| Some(b.map_or(h, |x| x.min(h))));
            let fast = traverse(&nodes, &sorted, o, d);
            assert_eq!(brute.is_some(), fast.is_some());
            if let (Some(a), Some(b)) = (brute, fast) {
                assert!((a - b).abs() < 1e-3);
                hits += 1;
            }
        }
        assert!(hits > 20, "{hits} rays hit");
        // leaves cover every triangle once
        let covered: usize = nodes.iter().filter(|n| n.b > 0).map(|n| n.b as usize).sum();
        assert_eq!(covered, tris.len());
    }
}

// ---------------------------------------------------------------- GPU

use wgpu::util::DeviceExt;

/// Samples a trace dispatch takes.
const PER_DISPATCH: u32 = 4;
/// Spacing of per-dispatch parameter slots in the uniform buffer.
const SLOT: u64 = (std::mem::size_of::<Params>() as u64).div_ceil(256) * 256;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    cam_to_world: [[f32; 4]; 4],
    view_proj: [[f32; 4]; 4],
    env_rot: [[f32; 4]; 4],
    size: [f32; 4],
    cam: [f32; 4],
    env: [f32; 4],
    ambient: [f32; 4],
    misc: [f32; 4],
    out: [f32; 4],
}

/// Pipelines of the path tracer (built on first use).
pub struct PtGpu {
    bgl0: wgpu::BindGroupLayout,
    bgl_pair: wgpu::BindGroupLayout,
    bgl_fin: wgpu::BindGroupLayout,
    trace: wgpu::ComputePipeline,
    atrous: wgpu::ComputePipeline,
    output: wgpu::RenderPipeline,
}

impl PtGpu {
    pub fn new(d: &wgpu::Device, format: wgpu::TextureFormat) -> PtGpu {
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pathtrace"),
            source: wgpu::ShaderSource::Wgsl(include_str!("pathtrace.wgsl").into()),
        });
        let vis = wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT;
        let buf = |binding: u32, ty: wgpu::BufferBindingType, dynamic: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: vis,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: dynamic, min_binding_size: None },
            count: None,
        };
        let ro = wgpu::BufferBindingType::Storage { read_only: true };
        let rw = wgpu::BufferBindingType::Storage { read_only: false };
        let tex = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: vis,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bgl0 = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pathtrace"),
            entries: &[
                buf(0, wgpu::BufferBindingType::Uniform, true),
                buf(1, ro, false),
                buf(4, ro, false),
                buf(5, ro, false),
                buf(6, ro, false),
                buf(7, rw, false),
                buf(8, rw, false),
                tex(10),
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: vis,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex(12),
            ],
        });
        let bgl_pair = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pathtrace-denoise"),
            entries: &[buf(0, ro, false), buf(1, rw, false)],
        });
        let bgl_fin = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pathtrace-output"),
            entries: &[buf(0, ro, false)],
        });
        let layout = |groups: &[&wgpu::BindGroupLayout]| {
            let groups: Vec<Option<&wgpu::BindGroupLayout>> = groups.iter().map(|g| Some(*g)).collect();
            d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &groups,
                immediate_size: 0,
            })
        };
        let compute = |entry: &str, l: &wgpu::PipelineLayout| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(l),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let trace = compute("cs_trace", &layout(&[&bgl0]));
        let atrous = compute("cs_atrous", &layout(&[&bgl0, &bgl_pair]));
        let output = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pathtrace-output"),
            layout: Some(&layout(&[&bgl0, &bgl_fin])),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_out"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_out"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        PtGpu { bgl0, bgl_pair, bgl_fin, trace, atrous, output }
    }
}

/// Views and samplers the tracer reads that the 3D engine owns.
pub struct PtInputs<'a> {
    pub env: &'a wgpu::TextureView,
    pub backdrop: Option<&'a wgpu::TextureView>,
    pub black: &'a wgpu::TextureView,
    pub sampler: &'a wgpu::Sampler,
}

/// Traces `scene` into `out` (premultiplied, the rasteriser's output representation).
#[allow(clippy::too_many_arguments)]
pub fn render(
    pt: &PtGpu,
    d: &wgpu::Device,
    enc: &mut wgpu::CommandEncoder,
    scene: &Scene3,
    data: &PtScene,
    opts: PathOpts,
    inputs: &PtInputs,
    out: &wgpu::TextureView,
) {
    let size = [scene.size[0].max(1), scene.size[1].max(1)];
    let pixels = (size[0] * size[1]) as u64;
    let storage = |label: &str, bytes: &[u8]| {
        d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: wgpu::BufferUsages::STORAGE,
        })
    };
    let zeroed = |label: &str, len: u64| {
        d.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: len,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        })
    };
    // interleaved corners, the material index's bits in the first corner's position w
    let mut verts: Vec<[f32; 4]> = Vec::with_capacity(data.pos.len() * 2);
    for (c, (p, n)) in data.pos.iter().zip(&data.nrm).enumerate() {
        let w = if c % 3 == 0 { f32::from_bits(data.tri_mat[c / 3]) } else { 0.0 };
        verts.push([p[0], p[1], p[2], w]);
        verts.push(*n);
    }
    let tverts = storage("pt-verts", bytemuck::cast_slice(&verts));
    let mats = storage("pt-mats", bytemuck::cast_slice(&data.mats));
    let nodes = storage("pt-nodes", bytemuck::cast_slice(&data.nodes));
    let lights = storage("pt-lights", bytemuck::cast_slice(&data.lights));
    let accum = zeroed("pt-accum", pixels * 16);
    let guide = zeroed("pt-guides", pixels * 32);
    let stand_in = zeroed("pt-stand-in", 16);
    // parameter slots: trace dispatches, denoise passes, output
    let samples = opts.samples.max(1);
    let chunks = samples.div_ceil(PER_DISPATCH);
    const PASSES: u32 = 5;
    let ambient = scene.lights.iter().filter(|l| l.kind == LightKind::Ambient).fold(Vec3::ZERO, |a, l| a + l.color);
    let lens = scene.dof.map(|f| f.coc_scale / scene.cam.focal_px.max(1e-6) * 0.5).unwrap_or(0.0);
    let base = Params {
        cam_to_world: scene.cam.view.inverse().to_cols_array_2d(),
        view_proj: (scene.clip_fix * scene.cam.view_proj()).to_cols_array_2d(),
        // the dome's full orientation, as the rasteriser applies it
        env_rot: scene.env.as_ref().map(|e| e.rotation).unwrap_or(Mat4::IDENTITY).to_cols_array_2d(),
        size: [size[0] as f32, size[1] as f32, 0.0, 0.0],
        cam: [scene.cam.focal_px, lens, scene.dof.map(|f| f.focus).unwrap_or(1.0), opts.bounces as f32],
        env: [
            scene.env.as_ref().map(|e| if e.visible { 2.0 } else { 1.0 }).unwrap_or(0.0),
            scene.env.as_ref().map(|e| e.intensity).unwrap_or(0.0),
            0.0,
            (ambient.max_element() > 0.0) as u32 as f32,
        ],
        ambient: [ambient.x, ambient.y, ambient.z, data.lights.len() as f32],
        misc: [inputs.backdrop.is_some() as u32 as f32, samples as f32, 1.0, scene.exposure],
        out: [scene.encode_srgb as u32 as f32, 0.0, 0.0, 0.0],
    };
    let mut slots: Vec<Params> = Vec::new();
    for c in 0..chunks {
        let start = c * PER_DISPATCH;
        slots.push(Params {
            size: [base.size[0], base.size[1], start as f32, (samples - start).min(PER_DISPATCH) as f32],
            ..base
        });
    }
    for k in 0..PASSES {
        slots.push(Params {
            misc: [base.misc[0], base.misc[1], (1u32 << k) as f32, base.misc[3]],
            out: [base.out[0], 0.0, (k == 0) as u32 as f32, 0.0],
            ..base
        });
    }
    slots.push(Params { out: [base.out[0], opts.denoise as u32 as f32, 0.0, 0.0], ..base });
    let mut ubytes = vec![0u8; slots.len() * SLOT as usize];
    for (k, sl) in slots.iter().enumerate() {
        let b = bytemuck::bytes_of(sl);
        ubytes[k * SLOT as usize..k * SLOT as usize + b.len()].copy_from_slice(b);
    }
    let ubuf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("pt-params"),
        contents: &ubytes,
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let group0 = |acc: &wgpu::Buffer| {
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pathtrace"),
            layout: &pt.bgl0,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &ubuf,
                        offset: 0,
                        size: std::num::NonZeroU64::new(std::mem::size_of::<Params>() as u64),
                    }),
                },
                wgpu::BindGroupEntry { binding: 1, resource: tverts.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: mats.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: nodes.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: acc.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: guide.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(inputs.env) },
                wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::Sampler(inputs.sampler) },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: wgpu::BindingResource::TextureView(inputs.backdrop.unwrap_or(inputs.black)),
                },
            ],
        })
    };
    let g_trace = group0(&accum);
    // passes that read the accumulation bind a stand-in in its read-write slot
    let g_read = group0(&stand_in);
    let groups = [size[0].div_ceil(8), size[1].div_ceil(8)];
    {
        let mut cp =
            enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("pathtrace"), timestamp_writes: None });
        cp.set_pipeline(&pt.trace);
        for c in 0..chunks {
            cp.set_bind_group(0, &g_trace, &[(c as u64 * SLOT) as u32]);
            cp.dispatch_workgroups(groups[0], groups[1], 1);
        }
    }
    let mut fin = &accum;
    let (ping, pong) = (zeroed("pt-denoise-a", pixels * 16), zeroed("pt-denoise-b", pixels * 16));
    if opts.denoise {
        let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("pathtrace-denoise"),
            timestamp_writes: None,
        });
        cp.set_pipeline(&pt.atrous);
        for k in 0..PASSES {
            let (src, dst) = if k == 0 {
                (&accum, &ping)
            } else if k % 2 == 1 {
                (&ping, &pong)
            } else {
                (&pong, &ping)
            };
            let pair = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pathtrace-denoise"),
                layout: &pt.bgl_pair,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: src.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: dst.as_entire_binding() },
                ],
            });
            cp.set_bind_group(0, &g_read, &[((chunks + k) as u64 * SLOT) as u32]);
            cp.set_bind_group(1, &pair, &[]);
            cp.dispatch_workgroups(groups[0], groups[1], 1);
            fin = dst;
        }
    }
    let gfin = d.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pathtrace-output"),
        layout: &pt.bgl_fin,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: fin.as_entire_binding() }],
    });
    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("pathtrace-output"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: out,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    rp.set_pipeline(&pt.output);
    rp.set_bind_group(0, &g_read, &[((chunks + PASSES) as u64 * SLOT) as u32]);
    rp.set_bind_group(1, &gfin, &[]);
    rp.draw(0..3, 0..1);
}
