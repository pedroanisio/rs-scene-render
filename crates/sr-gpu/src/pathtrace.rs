//! Progressive path tracing of a 3D pass (camera `renderer="pathtrace"`).
//!
//! Repeated rigid meshes use shared object-space prototype BVHs and world-space instance bounds.
//! Unique, deformed and displaced geometry is flattened into world-space triangles. Both levels
//! use a binned (12 bins) surface-area hierarchy; materials and lights remain per draw.
//! `pathtrace.wgsl` then traces `samples` paths per pixel in compute dispatches of a few
//! samples each: camera rays (thin lens for depth of field), next-event estimation toward the
//! analytic lights with shadow rays, a metallic-roughness BSDF (Lambert plus GGX, sampled by
//! lobe), smooth dielectric transmission, Russian roulette, and the dome environment and ambient
//! light for rays that leave the scene. Rays that leave through glass see the 2D layers behind
//! the pass where they cross the composition plane (z = 0). An edge-aware à-trous filter
//! (Dammertz et al. 2010) guided by first-hit normals and albedo optionally denoises the result.
//!
//! Gaussian splats share the acceleration structure and composite along rays;
//! camera rays use the complete projection, including offscreen and lens transforms.

use glam::{Mat4, Vec3};

use crate::three::{Draw3, LightKind, MeshSrc, Scene3};
mod cache;
mod instances;
pub use cache::BuildCache;

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
    /// Specular weight, cast-shadow flag, receive-shadow flag, unused
    pub extra: [f32; 4],
    /// Offset, width, height and packed sRGB/sampler flags for each shared pixel map.
    pub maps: [[u32; 4]; 6],
    /// Alpha cutoff, alpha mode, normal scale, occlusion strength.
    pub texture_params: [f32; 4],
    pub borders: [[f32; 4]; 6],
    /// Absorption of the medium a refracting surface encloses: Beer-Lambert coefficient per scene
    /// unit for each colour channel (from the attenuation colour and distance), then the albedo of the foam
    /// mixed into the surface (0 when none).
    pub attenuation: [f32; 4],
}

/// Beer-Lambert coefficients per scene unit: the light left after `distance` units is `color`.
/// No distance (infinite or not positive) or a white colour means no absorption.
fn absorption(color: &[f32; 3], distance: f32) -> [f32; 4] {
    if !(distance.is_finite() && distance > 0.0) {
        return [0.0; 4];
    }
    let coefficient = |c: f32| (-(c.clamp(1e-4, 1.0).ln()) / distance).max(0.0);
    [coefficient(color[0]), coefficient(color[1]), coefficient(color[2]), 0.0]
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
    /// Radius, casts shadows, packed IES pixel offset, IES width.
    pub size: [f32; 4],
    /// Right axis (rect lights), light influence bits (1 diffuse, 2 specular)
    pub right: [f32; 4],
}

/// Packed triangles, shared prototypes and top-level instances.
#[derive(Default)]
pub struct PtScene {
    /// Triangle corners in world space or in their shared prototype space.
    pub pos: Vec<[f32; 4]>,
    pub nrm: Vec<[f32; 4]>,
    pub uv: Vec<[[f32; 2]; 6]>,
    pub pixels: Vec<u32>,
    pub colors: Vec<[f32; 4]>,
    pub splats: std::collections::HashMap<usize, [[f32; 4]; 24]>,
    /// Top-level instance records referencing shared local-space prototype BVHs.
    pub instances: std::collections::HashMap<usize, [[f32; 4]; 24]>,
    /// Material of each triangle.
    pub tri_mat: Vec<u32>,
    pub mats: Vec<PtMat>,
    pub nodes: Vec<PtNode>,
    pub lights: Vec<PtLight>,
    /// What the tracer leaves out, for the render notes.
    pub notes: Vec<String>,
    /// CPU time `build` spent, for statistics.
    pub timing: BuildTiming,
}

/// CPU seconds `build` spent, split so BVH construction shows apart from the rest.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuildTiming {
    /// The complete rigid geometry and its BVHs came from the previous build.
    pub geometry_reused: bool,
    /// Object-space prototype BVHs built or reused in this call.
    pub prototype_builds: usize,
    pub prototype_hits: usize,
    /// Everything but the BVHs: materials, textures, triangle expansion, prototypes, lights.
    pub assemble_seconds: f64,
    /// Construction of the top-level and prototype BVHs (`bvh`).
    pub bvh_seconds: f64,
}

/// What recording the tracer's passes cost beyond the scene build.
#[derive(Default)]
pub struct RenderTiming {
    /// CPU seconds packing and uploading the scene buffers before the first tile.
    pub pack_seconds: f64,
    /// Timestamps of the trace and denoise passes, when requested and supported.
    pub gpu: Option<crate::fx::Timer>,
}

fn srgb_luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// What the tracer leaves out of `scene`.
pub fn notes(scene: &Scene3) -> Vec<String> {
    let mut notes = Vec::new();
    // the tracer has no per-point shading hook: the procedural unevenness of a material (a clay finish) is raster-only
    if scene.draws.iter().any(|d| d.material.unevenness > 0.0 && !d.material.unlit) {
        notes.push("material unevenness is not applied by the path tracer".to_string());
    }
    if scene.draws.iter().any(|d| d.shadow_catcher) {
        notes.push("a shadow catcher is not drawn by the path tracer".to_string());
    }
    notes
}

/// Whether the tracer's scene buffers fit the device. Image working buffers are tiled;
/// geometry remains one binding. The error is the note reported on raster fallback.
pub fn fits(size: [u32; 2], primitives: u64, limits: &wgpu::Limits) -> Result<(), String> {
    let cap = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
    let mib = |b: u64| b.div_ceil(1 << 20);
    // Pixel indices and shader seeds are u32. Reject impossible frames before planning tiles.
    if u64::from(size[0].max(1)) * u64::from(size[1].max(1)) > u64::from(u32::MAX) {
        return Err("path tracing pixel indices exceed u32; rasterised instead".into());
    }
    if cap < MIN_TILE_BYTES {
        return Err("path tracing device cannot bind a denoising tile; rasterised instead".into());
    }
    // Each triangle, splat or instance occupies a 384-byte record.
    let corners = primitives.saturating_mul(384);
    if corners > cap {
        return Err(format!(
            "path tracing {primitives} stored primitives need a {} MiB buffer and this device binds at most {} MiB; rasterised instead",
            mib(corners),
            cap >> 20
        ));
    }
    Ok(())
}

/// Why `scene` cannot be path traced on a device with `limits`, if it cannot.
pub fn limit_note(scene: &Scene3, limits: &wgpu::Limits) -> Option<String> {
    if let Err(error) = crate::volume::validate(&scene.volumes) {
        return Some(error);
    }
    let lights = scene.lights.len().max(1) as u32;
    if let Err(error) = crate::volume::validate_light_grid(&scene.volumes, lights, scene.env.is_some(), limits) {
        return Some(error);
    }
    let triangles = instances::storage_primitives(scene);
    if let Err(note) = fits(scene.size, triangles, limits) {
        return Some(note);
    }
    if (scene.draws.len() as u64).saturating_mul(std::mem::size_of::<PtMat>() as u64)
        > limits.max_storage_buffer_binding_size.min(limits.max_buffer_size)
    {
        return Some("path tracing materials exceed device storage binding; rasterised instead".into());
    }
    let mut keys = std::collections::HashSet::new();
    let texture_bytes = scene
        .draws
        .iter()
        .flat_map(|d| d.maps.iter().flatten())
        .filter(|t| keys.insert(t.key))
        .fold(0u64, |n, t| n.saturating_add(t.rgba.len() as u64));
    let cap = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
    let ies_bytes = scene.lights.iter().filter_map(|l| l.ies.as_ref()).map(|p| p.len() as u64 * 4).sum::<u64>();
    let bytes = triangles
        .saturating_mul(384)
        .saturating_add(texture_bytes)
        .saturating_add(ies_bytes)
        .saturating_add(crate::volume::bytes(&scene.volumes))
        .saturating_add(128);
    if bytes > cap {
        return Some(format!(
            "path tracing geometry, textures and volumes require {} MiB, device permits {} MiB; rasterised instead",
            bytes.div_ceil(1 << 20),
            cap >> 20
        ));
    }
    None
}

struct Triangle {
    p: [Vec3; 3],
    n: [Vec3; 3],
    uv: [[[f32; 2]; 6]; 3],
    colors: [[f32; 4]; 3],
}
fn triangle(dr: &Draw3, t: &[u32; 3], model: Mat4, nmat: Mat4) -> Triangle {
    let m = &dr.material;
    let verts = dr.mesh.cpu().0;
    let p = std::array::from_fn(|c| {
        let v = &verts[t[c] as usize];
        let mut pos = Vec3::from(v.pos);
        if let Some(map) = &dr.maps[5] {
            let uv = [v.uv[0] * m.uv_scale[0], v.uv[1] * m.uv_scale[1]];
            pos += Vec3::from(v.normal).normalize_or_zero() * map.sample(uv)[0] * m.displacement_scale;
        }
        model.transform_point3(pos)
    });
    let n =
        std::array::from_fn(|c| nmat.transform_vector3(Vec3::from(verts[t[c] as usize].normal)).normalize_or_zero());
    let uv = std::array::from_fn(|c| {
        let v = &verts[t[c] as usize];
        std::array::from_fn(|slot| {
            let uv = if m.separate_uvs && (1..5).contains(&slot) { v.map_uv[slot - 1] } else { v.uv };
            [uv[0] * m.uv_scale[0], uv[1] * m.uv_scale[1]]
        })
    });
    Triangle { p, n, uv, colors: std::array::from_fn(|i| verts[t[i] as usize].color) }
}

/// Builds world-space and shared prototype geometry, materials, lights and both BVH levels.
pub fn build(scene: &Scene3) -> PtScene {
    BuildCache::new(0).build(scene)
}

fn build_inner(scene: &Scene3, cache: &mut BuildCache) -> PtScene {
    let started = std::time::Instant::now();
    let mut bvh_seconds = 0.0;
    let mut s = PtScene::default();
    let key = cache::SceneKey::of(scene);
    let reused = cache.reuse(key.as_ref());
    s.timing.geometry_reused = reused.is_some();
    let mut tris: Vec<[Vec3; 3]> = Vec::new();
    let mut norms: Vec<[Vec3; 3]> = Vec::new();
    let mut tmat: Vec<u32> = Vec::new();
    let mut texcoords = Vec::new();
    let mut colors: Vec<[[f32; 4]; 3]> = Vec::new();
    let mut texture_offsets = std::collections::HashMap::new();
    let repetitions = instances::repetitions(scene);
    let mut prototype_ids = std::collections::HashMap::new();
    let mut prototypes: Vec<std::sync::Arc<cache::Prototype>> = Vec::new();
    let mut instance_records = std::collections::HashMap::new();
    // a shadow catcher has no colour of its own and the tracer cannot evaluate its darkening: it is left out (see `notes`)
    for dr in scene.draws.iter().filter(|d| !d.shadow_catcher) {
        let m = &dr.material;
        let maps = std::array::from_fn(|slot| {
            dr.maps[slot].as_ref().map_or([0; 4], |texture| {
                let offset = *texture_offsets.entry(texture.key).or_insert_with(|| {
                    let offset = s.pixels.len() as u32;
                    s.pixels.extend(texture.rgba.as_chunks::<4>().0.iter().map(|p| u32::from_le_bytes(*p)));
                    offset
                });
                [
                    offset,
                    texture.size[0],
                    texture.size[1],
                    texture.srgb as u32 | texture.sampler.map_or(0, |s| s.flags()),
                ]
            })
        });
        let unlit = m.unlit;
        let emissive = [
            m.emissive[0] * m.emissive_strength,
            m.emissive[1] * m.emissive_strength,
            m.emissive[2] * m.emissive_strength,
        ];
        let opacity =
            if m.alpha_mode != sr_3d::AlphaMode::Opaque { m.base_color[3] * m.opacity } else { 1.0 } * dr.opacity;
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
            extra: [
                m.specular * srgb_luma(m.specular_color).max(0.0),
                dr.cast_shadow as u32 as f32,
                dr.receive_shadow as u32 as f32,
                // foam mixed into the surface (the water shader's hook): 1 + the foam's roughness, or 0
                m.foam_mix.map_or(0.0, |f| 1.0 + f.roughness.clamp(0.0, 1.0)),
            ],
            maps,
            borders: std::array::from_fn(|i| {
                dr.maps[i].as_ref().and_then(|t| t.sampler).map_or([0.0; 4], |s| s.border)
            }),
            texture_params: [
                m.alpha_cutoff,
                match m.alpha_mode {
                    sr_3d::AlphaMode::Opaque => 0.0,
                    sr_3d::AlphaMode::Mask => 1.0,
                    sr_3d::AlphaMode::Blend => 2.0,
                },
                m.normal_scale,
                m.occlusion_strength,
            ],
            attenuation: {
                // the fourth value is the albedo of foam mixed into the surface, when it is
                let mut a = absorption(&m.attenuation_color, m.attenuation_distance);
                a[3] = m.foam_mix.map_or(0.0, |f| f.albedo.clamp(0.0, 1.0));
                a
            },
        });
        if reused.is_some() {
            continue;
        }
        if let Some(key) = instances::key(dr).filter(|k| repetitions[k] > 1) {
            let prototype = *prototype_ids.entry(key).or_insert_with(|| {
                prototypes.push(cache.prototype(dr, &mut s.timing));
                prototypes.len() - 1
            });
            let MeshSrc::Cached(mesh) = &dr.mesh else { unreachable!() };
            instance_records.insert(tris.len(), instances::record(dr.model, prototype, mi));
            tris.push(instances::bounds(dr.model, mesh.lo, mesh.hi));
            norms.push([Vec3::ZERO; 3]);
            texcoords.push([[[0.; 2]; 6]; 3]);
            colors.push([[1.; 4]; 3]);
            tmat.push(u32::MAX - 1);
        } else {
            let normal = dr.model.inverse().transpose();
            for t in dr.mesh.cpu().1.as_chunks::<3>().0 {
                let t = triangle(dr, t, dr.model, normal);
                tris.push(t.p);
                norms.push(t.n);
                texcoords.push(t.uv);
                colors.push(t.colors);
                tmat.push(mi);
            }
        }
    }

    if s.pixels.is_empty() {
        s.pixels.push(u32::MAX);
    }
    s.notes = notes(scene);
    if let Some(geometry) = reused {
        geometry.apply(&mut s);
    } else {
        let mut splat_records = std::collections::HashMap::new();
        for cloud in &scene.splats {
            let linear = glam::Mat3::from_mat4(cloud.model);
            let local = glam::Mat3::from_mat4(cloud.model.inverse());
            for (index, data) in cloud.gpu.cpu.iter().take(cloud.gpu.n as usize).enumerate() {
                let center = cloud.model.transform_point3(Vec3::new(data[0], data[1], data[2]));
                let covariance = glam::Mat3::from_cols_array(&[
                    data[4], data[5], data[6], data[5], data[7], data[8], data[6], data[8], data[9],
                ]);
                let covariance = linear * covariance * linear.transpose();
                let inverse = covariance.inverse();
                if !inverse.is_finite() {
                    continue;
                }
                let extent = Vec3::from_array(
                    [covariance.x_axis.x, covariance.y_axis.y, covariance.z_axis.z].map(|v| v.max(0.0).sqrt()),
                ) * 3.0;
                let mut record = [[0.0; 4]; 24];
                record[0] = [center.x, center.y, center.z, f32::from_bits(u32::MAX)];
                for (row, axis) in inverse.to_cols_array_2d().into_iter().enumerate() {
                    record[row + 1] = [axis[0], axis[1], axis[2], 0.0];
                }
                record[4] = [data[12], data[13], data[14], data[3] * cloud.opacity];
                for (row, axis) in local.to_cols_array_2d().into_iter().enumerate() {
                    record[row + 5] = [axis[0], axis[1], axis[2], 0.0];
                }
                if let Some(sh) = cloud.gpu.cpu_sh.get(index) {
                    for (row, values) in sh.as_chunks::<4>().0.iter().enumerate() {
                        record[row + 8].copy_from_slice(values);
                    }
                    record[20][0] = cloud.gpu.sh_degree as f32;
                }
                splat_records.insert(tris.len(), record);
                tris.push([center - extent, center + extent, center]);
                norms.push([Vec3::ZERO; 3]);
                texcoords.push([[[0.0; 2]; 6]; 3]);
                colors.push([[1.0; 4]; 3]);
                tmat.push(u32::MAX);
            }
        }
        // Top-level BVH, then world triangles / splats / instance records in leaf order
        let clock = std::time::Instant::now();
        let order = bvh(&tris, &mut s.nodes);
        bvh_seconds += clock.elapsed().as_secs_f64();
        for (new_index, &t) in order.iter().enumerate() {
            if let Some(record) = instance_records.remove(&t) {
                s.instances.insert(new_index, record);
            }
            if let Some(record) = splat_records.remove(&t) {
                s.splats.insert(new_index, record);
            }
            for c in 0..3 {
                let p = tris[t][c];
                let q = norms[t][c];
                s.pos.push([p.x, p.y, p.z, 0.0]);
                s.nrm.push([q.x, q.y, q.z, 0.0]);
                s.uv.push(texcoords[t][c]);
                s.colors.push(colors[t][c]);
            }
            s.tri_mat.push(tmat[t]);
        }
        let mut roots = Vec::new();
        for prototype in &prototypes {
            let mut nodes = prototype.nodes.clone();
            let order = &prototype.order;
            let root = s.nodes.len() as u32;
            let first = s.tri_mat.len() as u32;
            for node in &mut nodes {
                node.a += if node.b == 0 { root } else { first };
            }
            roots.push(root);
            s.nodes.extend(nodes);
            for &index in order {
                let t = &prototype.triangles[index];
                for c in 0..3 {
                    s.pos.push(t.p[c].extend(0.).to_array());
                    s.nrm.push(t.n[c].extend(0.).to_array());
                    s.uv.push(t.uv[c]);
                    s.colors.push(t.colors[c]);
                }
                s.tri_mat.push(0);
            }
        }
        for record in s.instances.values_mut() {
            record[13][0] = f32::from_bits(roots[record[13][0].to_bits() as usize]);
        }
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
        let ies = l.ies.as_ref().map(|profile| {
            let offset = s.pixels.len() as u32;
            s.pixels.extend(profile.iter().map(|v| v.to_bits()));
            offset
        });
        s.lights.push(PtLight {
            pos: [l.pos.x, l.pos.y, l.pos.z, kind],
            dir: [l.dir.x, l.dir.y, l.dir.z, l.range],
            color: [l.color.x, l.color.y, l.color.z, l.falloff],
            spot: [l.cos_outer, l.cos_inner, l.size[0], l.size[1]],
            size: [
                l.size[2],
                l.cast_shadow as u32 as f32,
                f32::from_bits(ies.unwrap_or(0)),
                if ies.is_some() { 128.0 } else { 0.0 },
            ],
            right: [
                l.right.x,
                l.right.y,
                l.right.z,
                (l.affects_diffuse as u32 | ((l.affects_specular as u32) << 1)) as f32,
            ],
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
        s.mats.push(PtMat {
            base: [0.0; 4],
            params: [0.0; 4],
            emissive: [0.0; 4],
            extra: [0.0; 4],
            maps: [[0; 4]; 6],
            borders: [[0.0; 4]; 6],
            texture_params: [0.0; 4],
            attenuation: [0.0; 4],
        });
    }
    if s.pos.is_empty() {
        s.pos.extend([[0.0; 4]; 3]);
        s.nrm.extend([[0.0; 4]; 3]);
        s.tri_mat.push(0);
    }
    if !s.timing.geometry_reused {
        cache.remember(key, &s);
    }
    bvh_seconds += s.timing.bvh_seconds;
    s.timing.bvh_seconds = bvh_seconds;
    s.timing.assemble_seconds = (started.elapsed().as_secs_f64() - bvh_seconds).max(0.0);
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
    let mut stack = vec![(0usize, 0usize, n, 0u32)];
    nodes.push(PtNode::default());
    while let Some((ni, start, end, depth)) = stack.pop() {
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
        if count <= 4 || depth >= 30 {
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
        stack.push((right, mid, end, depth + 1));
        stack.push((left, start, mid, depth + 1));
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foam_alone_selects_the_foam_variant_and_never_the_water_one_and_every_variant_has_its_own_pipeline_slot() {
        let plain = sr_3d::MaterialParams::default();
        let glass = sr_3d::MaterialParams { transmission: 1.0, ..Default::default() };
        let mixed = sr_3d::MaterialParams {
            foam_mix: Some(sr_3d::FoamMix { albedo: 0.9, roughness: 0.8 }),
            ..Default::default()
        };
        let mixed_glass = sr_3d::MaterialParams { foam_mix: mixed.foam_mix, ..glass.clone() };
        assert_eq!(variant_of([&plain].into_iter()), (false, false));
        assert_eq!(variant_of([&plain, &glass].into_iter()), (true, false));
        assert_eq!(variant_of([&plain, &mixed].into_iter()), (false, true), "foam does not bring in the water variant");
        assert_eq!(variant_of([&glass, &mixed].into_iter()), (true, true));
        assert_eq!(variant_of([&mixed_glass].into_iter()), (true, true));
        // 5 shapes of scene (media, grid, lighting) times 3 variants: 15 distinct slots in 0..15
        let mut slots = std::collections::BTreeSet::new();
        for (media, grid, lighting) in
            [(false, false, false), (true, false, false), (true, false, true), (true, true, false), (true, true, true)]
        {
            for (water, foam) in [(true, false), (true, true), (false, true)] {
                slots.insert(variant_slot(media, grid, lighting, water, foam));
            }
        }
        assert_eq!(slots.len(), 15);
        assert_eq!((slots.first(), slots.last()), (Some(&0), Some(&14)));
    }

    #[test]
    fn a_variant_pipeline_is_given_water_only_for_water_and_foam_only_for_foam() {
        let plain = format!(
            "{}\n{}\n{}",
            include_str!("sampling.wgsl"),
            include_str!("pathtrace.wgsl"),
            include_str!("volume.wgsl")
        );
        let names = |c: Vec<(&str, f64)>| c.into_iter().map(|(n, v)| (n.to_string(), v)).collect::<Vec<_>>();
        // foam alone: the media constants, and neither WATER nor FOAM (the module has no FOAM override to give a value to)
        assert_eq!(
            names(variant_constants(false, false, false, true)).iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
            ["HAS_MEDIA", "MEDIUM_LIGHTING"]
        );
        // water: WATER on and FOAM off; water with foam: FOAM on
        assert!(variant_constants(true, false, true, false).contains(&("WATER", 1.0)));
        assert!(variant_constants(true, false, true, false).contains(&("FOAM", 0.0)));
        assert!(variant_constants(true, true, true, true).contains(&("FOAM", 1.0)));
        assert!(variant_constants(true, true, true, true).contains(&("MEDIUM_LIGHTING", 1.0)));
        // the text: nothing added for a scene with neither (the plain pipeline is not a variant), the refracted shadow rays only for
        // water, the hook only for foam
        assert_eq!(variant_source(&plain, false, false), plain);
        let foam_only = variant_source(&plain, false, true);
        assert!(foam_only.contains("pick < foam") && !foam_only.contains("fn light_through("));
        let water_only = variant_source(&plain, true, false);
        assert!(water_only.contains("fn light_through(") && !water_only.contains("pick < foam"));
        assert!(
            water_only.contains("override FOAM: bool = false;"),
            "the water shader declares the override the constant sets"
        );
        let both = variant_source(&plain, true, true);
        assert!(both.contains("fn light_through(") && both.contains("pick < foam"));
        assert!(
            !foam_only.contains("override FOAM"),
            "the foam-only shader has no override: a constant for it would be an error"
        );
    }

    #[test]
    fn the_foam_hook_changes_the_surface_at_the_hit_and_brings_in_no_refracted_shadow_ray() {
        let plain = format!(
            "{}\n{}\n{}",
            include_str!("sampling.wgsl"),
            include_str!("pathtrace.wgsl"),
            include_str!("volume.wgsl")
        );
        assert!(!plain.contains("foam_u"), "scenes without foam keep their shader text");
        for base in [plain.clone(), grid_source(), water_source(&plain)] {
            let foamy = foam_source(&base);
            assert!(foamy.contains("var s = surf_of(m);") && foamy.contains("if (pick < foam) {"));
            assert!(foamy.contains("foam_u = fract("), "the stratified number is set for every sample");
            assert!(foamy.contains("guide_albedo = mix(guide_albedo"), "the denoiser's guide is the mean albedo");
            // the light that gets through keeps the water's tint: the foam sample is the one that lets nothing through
            assert!(foamy.contains("s.trans = 0.0;") && !foamy.contains("s.albedo = mix("));
            assert_eq!(foamy.contains("fn light_through("), base.contains("fn light_through("));
        }
    }

    #[test]
    fn the_water_shader_adds_the_refracted_shadow_ray_and_the_plain_one_does_not_have_it() {
        let plain = format!(
            "{}\n{}\n{}",
            include_str!("sampling.wgsl"),
            include_str!("pathtrace.wgsl"),
            include_str!("volume.wgsl")
        );
        assert!(!plain.contains("light_through"), "scenes without transmissive materials keep their shader text");
        for base in [plain.clone(), grid_source()] {
            let water = water_source(&base);
            assert!(water.contains("fn light_through(") && water.contains("inside = entering;"));
            assert!(water.contains("thr *= eta * eta;") && water.contains("let probe = first_interface("));
            assert!(
                water.contains("fn water_transport(")
                    && water.contains("let seen = water_transport(o, d, hit.t, in_sigma);")
            );
            assert!(
                water.contains("if (!entering && dot(t, t) >= 1e-8) { cosi"),
                "the exit uses the cosine of the air side"
            );
            assert_eq!(
                water.matches("light_through(p, ng, n, l, ls.w, in_sigma, lt.size.y > 0.5 && m.extra.z > 0.5)").count(),
                1
            );
            assert!(water.contains("fn volume_transmittance("), "shared code is kept");
            assert!(
                water.contains("let c = dielectric_crossing(")
                    && water.contains("if (c.refracted) { inside = c.inside; }"),
                "the water shader takes whether the path is inside from a refraction only, not from a reflection"
            );
        }
    }

    /// The crossing of a dielectric surface is one function, called once from `radiance`, and its
    /// body is not repeated in the loop.
    #[test]
    fn the_crossing_of_a_dielectric_surface_is_a_function_radiance_calls() {
        let plain = include_str!("pathtrace.wgsl");
        assert_eq!(plain.matches("fn dielectric_crossing(").count(), 1);
        assert_eq!(plain.matches("dielectric_crossing(s, m, ").count(), 1, "called once");
        assert_eq!(plain.matches("refract(d, n, eta)").count(), 1, "the refraction is in the function only");
        let radiance = &plain[plain.find("fn radiance(").expect("radiance")..];
        assert!(!radiance.contains("refract("), "radiance does not refract itself");
    }

    #[test]
    fn the_grid_shader_replaces_the_exact_lighting_and_leaves_the_rest() {
        let source = grid_source();
        assert_eq!(source.matches("fn volume_incident(").count(), 1, "one volume_incident");
        assert!(source.contains("fn volume_incident_exact("), "the exact function stays for other media");
        // it is the plain function renamed, not a second copy that could drift from it
        assert!(
            !include_str!("volume_grid.wgsl").contains("fn volume_incident_exact("),
            "the grid file does not carry its own copy of the exact lighting"
        );
        let body = |text: &str, name: &str| {
            let start = text.find(name).expect("function");
            let end = start + text[start..].find("\n}\n").expect("end of function");
            text[start + name.len()..end].to_string()
        };
        assert_eq!(
            body(&source, "fn volume_incident_exact("),
            body(include_str!("volume.wgsl"), "fn volume_incident("),
            "the exact function is the plain one"
        );
        assert!(source.contains("tverts[base+13u].y>0.5,base)"), "the call that lights a domain passes the domain");
        assert!(!source.contains("//@exact-incident"), "the markers are removed");
        // everything outside the replaced span is the shader without grids
        let plain = include_str!("volume.wgsl");
        assert!(plain.matches("fn volume_incident(").count() == 1 && plain.contains("//@exact-incident-begin"));
        assert!(source.contains("fn volume_transmittance("), "shared code is kept");
    }

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
const DENOISE_PASSES: u32 = 5;
// A pass has radius 2 * step, with step = 1, 2, 4, 8, 16. The complete
// dependency footprint is 62 pixels on each side, not just the last pass's 32.
const DENOISE_HALO: u32 = 2 * ((1 << DENOISE_PASSES) - 1);
// Retain a 64×64 output core even at the smallest accepted budget. A one-pixel
// core would turn UHD into millions of largely redundant halo dispatches.
const MIN_TILE_BYTES: u64 = ((2 * DENOISE_HALO + 64) as u64).pow(2) * 32;
const DEFAULT_TILE_BYTES: u64 = 32 << 20;

#[derive(Clone, Copy, Debug)]
struct Tile {
    origin: [u32; 2],
    size: [u32; 2],
    output_origin: [u32; 2],
    output_size: [u32; 2],
}

/// Non-overlapping output rectangles with complete filter support around each one.
fn tiles(size: [u32; 2], bytes: u64, denoise: bool) -> Vec<Tile> {
    let pixels = bytes / 32;
    if u64::from(size[0]) * u64::from(size[1]) <= pixels {
        return vec![Tile { origin: [0; 2], size, output_origin: [0; 2], output_size: size }];
    }
    let halo = if denoise { DENOISE_HALO } else { 0 };
    let side = pixels.isqrt().min(u64::from(u32::MAX)) as u32;
    let stride = side.saturating_sub(2 * halo).max(1);
    let mut result = Vec::new();
    for y in (0..size[1]).step_by(stride as usize) {
        for x in (0..size[0]).step_by(stride as usize) {
            let origin = [x.saturating_sub(halo), y.saturating_sub(halo)];
            let end = [x.saturating_add(stride).min(size[0]), y.saturating_add(stride).min(size[1])];
            result.push(Tile {
                origin,
                size: std::array::from_fn(|i| end[i].saturating_add(halo).min(size[i]) - origin[i]),
                output_origin: [x, y],
                output_size: [end[0] - x, end[1] - y],
            });
        }
    }
    result
}
/// Spacing of per-dispatch parameter slots in the uniform buffer.
const SLOT: u64 = (std::mem::size_of::<Params>() as u64).div_ceil(256) * 256;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    cam_to_world: [[f32; 4]; 4],
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    env_rot: [[f32; 4]; 4],
    size: [f32; 4],
    cam: [f32; 4],
    env: [f32; 4],
    ambient: [f32; 4],
    misc: [f32; 4],
    out: [f32; 4],
    /// Global pixel origin (xy) and extent (zw) of the working tile.
    tile: [u32; 4],
    media: [u32; 4],
}

/// Compute pipelines of the shader variant with light grids.
struct GridPipelines {
    module: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    light: wgpu::ComputePipeline,
    dome: wgpu::ComputePipeline,
}

/// The shader with light grids: the exact in-scattering function and the call that lights a domain
/// are replaced, everything else is the shader without grids.
fn grid_source() -> String {
    const BEGIN: &str = "//@exact-incident-begin\n";
    const END: &str = "//@exact-incident-end\n";
    const CALL: &str = "volume_incident(point,-d,albedo.w,tverts[base+13u].y>0.5)";
    let volume = include_str!("volume.wgsl");
    let (a, b) = (volume.find(BEGIN).expect("incident begin marker"), volume.find(END).expect("incident end marker"));
    assert!(volume.matches(CALL).count() == 1, "one call lights a domain");
    // the plain function stays, renamed: the grid's `volume_incident` falls back to it for a domain without grids
    let exact = volume[a + BEGIN.len()..b].replacen("fn volume_incident(", "fn volume_incident_exact(", 1);
    let shared = format!("{}{}{}", &volume[..a], exact, &volume[b + END.len()..])
        .replace(CALL, "volume_incident(point,-d,albedo.w,tverts[base+13u].y>0.5,base)");
    format!(
        "{}\n{}\n{}\n{}",
        include_str!("sampling.wgsl"),
        include_str!("pathtrace.wgsl"),
        shared,
        include_str!("volume_grid.wgsl")
    )
}

/// The shader for scenes with a transmissive material: the shader it is given (with or without
/// light grids) plus the refracted shadow ray, the Fresnel term of a ray leaving the denser medium,
/// the scaling of radiance across an interface and the start of a camera under the water, which a
/// few small replacements hook in. Scenes
/// without such a material keep the text, and so the compiled code, they had.
fn water_source(base: &str) -> String {
    const SIGMA: &str = "    var in_sigma = vec3(0.0);\n";
    const ENTER: &str = "        if (WATER) { in_sigma = select(vec3(0.0), m.attenuation.rgb, entering); }\n";
    const CROSSED: &str = "            in_sigma = c.in_sigma;\n";
    const FRESNEL: &str = "    let cosi = clamp(dot(v, n), 0.0, 1.0);\n    let r0 = (1.0 - eta) / (1.0 + eta);\n    let fr = r0 * r0 + (1.0 - r0 * r0) * pow(1.0 - cosi, 5.0);\n    let t = refract(d, n, eta);\n";
    const FOG: &str = "        if (WATER && any(in_sigma > vec3(0.0))) { thr *= exp(-in_sigma * min(hit.t, 1e4)); }\n        if (HAS_MEDIA) {\n";
    const VISIBLE: &str = "            var visible = 1.0;\n            if (lt.size.y > 0.5 && m.extra.z > 0.5) { visible = visibility(p + ng * 1e-2, l, ls.w - 2e-2); }";
    for hook in [SIGMA, ENTER, CROSSED, FRESNEL, FOG, VISIBLE] {
        assert_eq!(base.matches(hook).count(), 1, "one place hooks in the refracted shadow ray: {hook}");
    }
    let hooked = base
        // Schlick's term takes the cosine of the side the light goes to when it leaves the denser medium
        .replace(
            FRESNEL,
            "    let t = refract(d, n, eta);\n\
             \x20   var cosi = clamp(dot(v, n), 0.0, 1.0);\n\
             \x20   if (!entering && dot(t, t) >= 1e-8) { cosi = clamp(-dot(normalize(t), n), 0.0, 1.0); }\n\
             \x20   let r0 = (1.0 - eta) / (1.0 + eta);\n\
             \x20   let fr = r0 * r0 + (1.0 - r0 * r0) * pow(1.0 - cosi, 5.0);\n",
        )
        // inside absorbing water the media's light is dimmed by the water in front of it, and the water
        // absorbs across the gaps; elsewhere the march is the plain one
        .replace(
            FOG,
            "        if (any(in_sigma > vec3(0.0))) {\n\
             \x20           let seen = water_transport(o, d, hit.t, in_sigma);\n\
             \x20           col += thr * seen.color;\n\
             \x20           thr *= seen.trans;\n\
             \x20           if (met == 0u) { alpha += (1.0 - alpha) * (1.0 - seen.open); }\n\
             \x20       } else if (HAS_MEDIA) {\n",
        )
        .replace(
            SIGMA,
            // a camera under the water starts inside the medium: the first transmissive surface
            // straight up (the scene's up is -y) is met from the inside
            &format!(
                "{SIGMA}    var inside = false;\n\
                 \x20   {{\n\
                 \x20       let probe = first_interface(o, vec3(0.0, -1.0, 0.0), 1e30);\n\
                 \x20       if (probe.found && !probe.entering) {{ inside = true; in_sigma = probe.sigma; }}\n\
                 \x20   }}\n"
            ),
        )
        .replace(
            ENTER,
            // radiance over eta^2 is the same on both sides of an interface: a path that goes
            // from a medium of index n1 into one of index n2 carries (n1 / n2)^2
            "        if (WATER) {\n\
             \x20           in_sigma = select(vec3(0.0), m.attenuation.rgb, entering);\n\
             \x20           inside = entering;\n\
             \x20           thr *= eta * eta;\n\
             \x20       }\n",
        )
        // the path leaves the crossing inside the medium or not
        .replace(CROSSED, &format!("{CROSSED}            if (c.refracted) {{ inside = c.inside; }}\n"))
        .replace(
            VISIBLE,
            &format!(
                "            if (WATER && inside) {{\n\
                 \x20               // a surface under water (or glass): the light reaches it refracted\n\
                 \x20               let seen = light_through(p, ng, n, l, ls.w, in_sigma, lt.size.y > 0.5 && m.extra.z > 0.5);\n\
                 \x20               var through = seen.vis;\n\
                 \x20               if (HAS_MEDIA && m.extra.z > 0.5) {{\n\
                 \x20                   through *= volume_transmittance(p + ng * 1e-2, seen.dir, seen.inside) * volume_transmittance(seen.q, seen.la, seen.outside);\n\
                 \x20               }}\n\
                 \x20               var c = thr * bsdf(s, n, v, seen.dir, light_lobes(lt)) * max(dot(n, seen.dir), 0.0) * rad * through;\n\
                 \x20               if (bounce > 0u) {{ c = min(c, vec3(20.0)); }}\n\
                 \x20               col += c;\n\
                 \x20               continue;\n\
                 \x20           }}\n{VISIBLE}"
            ),
        );
    format!("{hooked}\n{}", include_str!("pathtrace_water.wgsl"))
}

/// The shader for scenes where some surface has foam mixed into its material: the shader it is given plus the foam at the hit. A
/// surface's share of foam, the alpha of its vertex colour, is the share of its area that the foam covers: a sample is on the foam
/// with that probability (and then the surface is the foam's, a white diffuse one that lets nothing through), on the water otherwise
/// (and then the light that gets through keeps the water's tint), so the picture is the mean of the two, weighted by the share. The
/// albedo guide of the denoiser takes the mean albedo. Scenes without such a surface keep the text, and so the compiled code, they had.
fn foam_source(base: &str) -> String {
    const TRACE: &str = "        rng = pcg(global_pix * 9781u + pcg(si * 6271u + 1u));\n";
    const GUIDE: &str = "            guide[pix * 2u] += vec4(m.base.rgb, 1.0);\n";
    const SURFACE: &str = "        let s = surf_of(m);\n";
    for hook in [TRACE, GUIDE, SURFACE] {
        assert_eq!(base.matches(hook).count(), 1, "one place hooks in the foam: {hook}");
    }
    let hooked = base
        // the number that picks the lobe at the first hit of a sample: a sequence with a random offset for each pixel (a hash of it, so
        // that neighbours are not correlated), so the samples of a pixel are spread over the share instead of being drawn
        .replace(
            TRACE,
            &format!(
                "{TRACE}        foam_u = fract(f32(pcg(global_pix * 2654435761u + 40503u)) * (1.0 / 4294967296.0) + f32(si) * 0.6180339887);\n"
            ),
        )
        .replace(
            GUIDE,
            "            var guide_albedo = m.base.rgb;\n\
             \x20           if (m.extra.w > 0.5) { guide_albedo = mix(guide_albedo, vec3(m.attenuation.w), clamp(color.a, 0.0, 1.0)); }\n\
             \x20           guide[pix * 2u] += vec4(guide_albedo, 1.0);\n",
        )
        .replace(
            SURFACE,
            "        var s = surf_of(m);\n\
             \x20       if (m.extra.w > 0.5) {\n\
             \x20           let foam = clamp(color.a, 0.0, 1.0);\n\
             \x20           if (foam > 0.0) {\n\
             \x20               // the first hit of a sample takes the stratified number, the later ones a random one\n\
             \x20               var pick = foam_u;\n\
             \x20               if (met != 1u || !first) { pick = rnd(); }\n\
             \x20               if (pick < foam) {\n\
             \x20                   let r0 = (s.ior - 1.0) / (s.ior + 1.0);\n\
             \x20                   s.albedo = vec3(m.attenuation.w);\n\
             \x20                   s.metallic = 0.0;\n\
             \x20                   s.a = max((m.extra.w - 1.0) * (m.extra.w - 1.0), 1e-3);\n\
             \x20                   s.trans = 0.0;\n\
             \x20                   s.f0 = vec3(r0 * r0);\n\
             \x20                   s.specw = clamp(m.extra.x, 0.0, 1.0);\n\
             \x20               }\n\
             \x20           }\n\
             \x20       }\n",
        );
    format!("var<private> foam_u: f32 = 0.0;\n{hooked}")
}

/// The variant of the trace shader the materials of a scene need beyond the plain one: `(water, foam)`, where `water` is a
/// transmissive material (the refracted shadow rays) and `foam` a foam mix (the hook at the hit). Foam alone does not make
/// `water`: the refracted shadow rays cost several times the plain shader.
fn variant_of<'a>(materials: impl Iterator<Item = &'a sr_3d::MaterialParams>) -> (bool, bool) {
    materials.fold((false, false), |(water, foam), m| (water || m.transmission > 0.0, foam || m.foam_mix.is_some()))
}

/// The slot of the variant pipeline for a scene: five by media and grid lighting, times water, water with foam, or foam alone.
fn variant_slot(media: bool, grid: bool, lighting: bool, water: bool, foam: bool) -> usize {
    (match (media, grid) {
        (false, _) => 0,
        (true, false) => 1 + lighting as usize,
        (true, true) => 3 + lighting as usize,
    }) + 5 * if water { foam as usize } else { 2 }
}

/// The override constants of a variant pipeline: the media ones always, `WATER` only for a scene with a transmissive material and
/// `FOAM` only for one that also has foam (the water shader declares it; the foam-only shader has no such override, and a constant
/// for a name the module lacks is an error of the pipeline).
fn variant_constants(media: bool, lighting: bool, water: bool, foam: bool) -> Vec<(&'static str, f64)> {
    let mut constants = vec![("HAS_MEDIA", media as u8 as f64), ("MEDIUM_LIGHTING", lighting as u8 as f64)];
    if water {
        constants.extend([("WATER", 1.0), ("FOAM", foam as u8 as f64)]);
    }
    constants
}

/// The shader text of a variant: the base with the water additions when `water`, and the foam at the hit when `foam`.
fn variant_source(base: &str, water: bool, foam: bool) -> String {
    let source = if water { water_source(base) } else { base.to_string() };
    if foam {
        foam_source(&source)
    } else {
        source
    }
}

/// Pipelines of the path tracer (built on first use).
pub struct PtGpu {
    bgl0: wgpu::BindGroupLayout,
    bgl_pair: wgpu::BindGroupLayout,
    bgl_fin: wgpu::BindGroupLayout,
    trace: wgpu::ComputePipeline,
    trace_volume: [std::sync::OnceLock<wgpu::ComputePipeline>; 2],
    /// The same with the `WATER` constant for scenes with transmissive materials: no volumes, volumes
    /// (by whether their lighting needs albedo), and grid-lit volumes (the same).
    trace_water: [std::sync::OnceLock<wgpu::ComputePipeline>; 15],
    module: wgpu::ShaderModule,
    /// The variant of the shader with light grids, its group-1 layout and pipelines (built on first use).
    bgl_grid: wgpu::BindGroupLayout,
    grid: std::sync::OnceLock<GridPipelines>,
    trace_grid: [std::sync::OnceLock<wgpu::ComputePipeline>; 2],
    atrous: wgpu::ComputePipeline,
    output: wgpu::RenderPipeline,
    tile_bytes: u64,
}

impl PtGpu {
    pub fn new(d: &wgpu::Device, format: wgpu::TextureFormat) -> PtGpu {
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pathtrace"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}\n{}",
                    include_str!("sampling.wgsl"),
                    include_str!("pathtrace.wgsl"),
                    include_str!("volume.wgsl")
                )
                .into(),
            ),
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
        let bgl_grid = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pathtrace-light-grid"),
            entries: &[buf(0, rw, false)],
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
            let _creation = crate::gpu::creation_lock();
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
        let output = {
            let _creation = crate::gpu::creation_lock();
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
            })
        };
        PtGpu {
            bgl0,
            bgl_pair,
            bgl_fin,
            trace,
            trace_volume: std::array::from_fn(|_| std::sync::OnceLock::new()),
            trace_water: std::array::from_fn(|_| std::sync::OnceLock::new()),
            module,
            bgl_grid,
            grid: std::sync::OnceLock::new(),
            trace_grid: std::array::from_fn(|_| std::sync::OnceLock::new()),
            atrous,
            output,
            tile_bytes: DEFAULT_TILE_BYTES,
        }
    }

    /// The pipelines of the shader with light grids, compiled the first time a pass needs them.
    fn grid_pipelines(&self, d: &wgpu::Device) -> &GridPipelines {
        self.grid.get_or_init(|| {
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("pathtrace-light-grid"),
                source: wgpu::ShaderSource::Wgsl(grid_source().into()),
            });
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("pathtrace-light-grid"),
                bind_group_layouts: &[Some(&self.bgl0), Some(&self.bgl_grid)],
                immediate_size: 0,
            });
            let kernel = |entry: &str| {
                let _creation = crate::gpu::creation_lock();
                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            };
            let light = kernel("cs_light_grid");
            let dome = kernel("cs_dome_grid");
            GridPipelines { module, layout, light, dome }
        })
    }

    fn trace_pipeline(&self, d: &wgpu::Device, scene: &Scene3, grid: bool) -> &wgpu::ComputePipeline {
        let (water, foam) = variant_of(scene.draws.iter().map(|dr| &dr.material));
        if water || foam {
            return self.variant_pipeline(d, scene, grid, water, foam);
        }
        if scene.volumes.is_empty() {
            return &self.trace;
        }
        let lighting = scene.volumes.iter().any(|v| v.medium().optical().albedo.iter().any(|v| *v > 0.0));
        if grid {
            return self.trace_grid[lighting as usize].get_or_init(|| {
                let g = self.grid_pipelines(d);
                let _creation = crate::gpu::creation_lock();
                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("pathtrace-volumes-grid"),
                    layout: Some(&g.layout),
                    module: &g.module,
                    entry_point: Some("cs_trace"),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &[("HAS_MEDIA", 1.0), ("MEDIUM_LIGHTING", lighting as u8 as f64)],
                        ..Default::default()
                    },
                    cache: None,
                })
            });
        }
        self.trace_volume[lighting as usize].get_or_init(|| {
            let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("pathtrace-volumes"),
                bind_group_layouts: &[Some(&self.bgl0)],
                immediate_size: 0,
            });
            {
                let _creation = crate::gpu::creation_lock();
                d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("pathtrace-volumes"),
                    layout: Some(&layout),
                    module: &self.module,
                    entry_point: Some("cs_trace"),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &[("HAS_MEDIA", 1.0), ("MEDIUM_LIGHTING", lighting as u8 as f64)],
                        ..Default::default()
                    },
                    cache: None,
                })
            }
        })
    }

    /// The trace pipeline of a scene with transmissive materials (`water`: the shader with `WATER` set), with foam mixed into
    /// some surface's material (`foam`), or both. Scenes with neither keep the pipeline, and the speed, they had; foam alone does
    /// not bring in the refracted shadow rays, which cost several times the plain shader.
    fn variant_pipeline(
        &self,
        d: &wgpu::Device,
        scene: &Scene3,
        grid: bool,
        water: bool,
        foam: bool,
    ) -> &wgpu::ComputePipeline {
        let lighting = scene.volumes.iter().any(|v| v.medium().optical().albedo.iter().any(|v| *v > 0.0));
        let media = !scene.volumes.is_empty();
        let slot = variant_slot(media, grid, lighting, water, foam);
        self.trace_water[slot].get_or_init(|| {
            let constants = variant_constants(media, lighting, water, foam);
            let base = if media && grid {
                grid_source()
            } else {
                format!(
                    "{}\n{}\n{}",
                    include_str!("sampling.wgsl"),
                    include_str!("pathtrace.wgsl"),
                    include_str!("volume.wgsl")
                )
            };
            let source = variant_source(&base, water, foam);
            let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("pathtrace-variant"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let plain_layout;
            let layout = if media && grid {
                &self.grid_pipelines(d).layout
            } else {
                plain_layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("pathtrace-variant"),
                    bind_group_layouts: &[Some(&self.bgl0)],
                    immediate_size: 0,
                });
                &plain_layout
            };
            let _creation = crate::gpu::creation_lock();
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("pathtrace-variant"),
                layout: Some(layout),
                module: &module,
                entry_point: Some("cs_trace"),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                cache: None,
            })
        })
    }

    /// Sets the maximum size of each image storage buffer, additionally bounded by device
    /// limits. Useful for constraining memory and verifying that tiled output is identical.
    /// The budget must fit a 64×64 output core and its complete denoising halo
    /// (1,131,008 bytes), bounding the amount of repeated halo work.
    pub fn limit_buffers(&mut self, bytes: u64) -> Result<(), String> {
        if bytes < MIN_TILE_BYTES {
            return Err(format!("path tracing needs at least {MIN_TILE_BYTES} bytes per tile buffer"));
        }
        self.tile_bytes = bytes;
        Ok(())
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
    render_timed(pt, d, enc, scene, data, opts, inputs, out, false);
}

/// The buffers of a pass that the light-grid kernels read, besides the grid itself.
struct GridInputs<'a> {
    verts: &'a wgpu::Buffer,
    mats: &'a wgpu::Buffer,
    nodes: &'a wgpu::Buffer,
    lights: &'a wgpu::Buffer,
    guide: &'a wgpu::Buffer,
    /// A buffer for the accumulation binding, which the kernels do not use.
    stand_in: &'a wgpu::Buffer,
}

/// Records the kernels that fill the light grids of `plan` into `group`, once before the tiles:
/// one dispatch for each scalar slot (a light's or a dome direction's) and, for isotropic media
/// under a dome, one for the pre-integrated radiance. `first` carries the pass's parameters, in
/// which only the slot differs from one dispatch to the next.
#[allow(clippy::too_many_arguments)]
fn build_light_grids(
    pt: &PtGpu,
    d: &wgpu::Device,
    enc: &mut wgpu::CommandEncoder,
    timer: &mut Option<crate::fx::Timer>,
    plan: &crate::volume::LightGridPlan,
    group: &wgpu::BindGroup,
    first: Params,
    buffers: &GridInputs,
    inputs: &PtInputs,
) {
    let params: Vec<Params> = (0..plan.scalar_slots)
        .map(|slot| Params { size: [first.size[0], first.size[1], slot as f32, 0.0], ..first })
        .collect();
    let mut bytes = vec![0u8; params.len().max(1) * SLOT as usize];
    for (k, sl) in params.iter().enumerate() {
        let b = bytemuck::bytes_of(sl);
        bytes[k * SLOT as usize..k * SLOT as usize + b.len()].copy_from_slice(b);
    }
    let ubuf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("pt-grid-params"),
        contents: &bytes,
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let entries = |acc: &wgpu::Buffer| {
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pathtrace-grid-build"),
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
                wgpu::BindGroupEntry { binding: 1, resource: buffers.verts.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: buffers.mats.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: buffers.nodes.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: buffers.lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: acc.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: buffers.guide.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(inputs.env) },
                wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::Sampler(inputs.sampler) },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: wgpu::BindingResource::TextureView(inputs.backdrop.unwrap_or(inputs.black)),
                },
            ],
        })
    };
    let build = entries(buffers.stand_in);
    let g = pt.grid_pipelines(d);
    let stamp = timer.as_mut().and_then(|t| t.labelled_pair("pathtrace light grid"));
    let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("pathtrace-light-grid"),
        timestamp_writes: stamp.zip(timer.as_ref()).map(|(a, t)| wgpu::ComputePassTimestampWrites {
            query_set: &t.set,
            beginning_of_pass_write_index: Some(a),
            end_of_pass_write_index: Some(a + 1),
        }),
    });
    cp.set_bind_group(1, group, &[]);
    cp.set_pipeline(&g.light);
    for slot in 0..plan.scalar_slots {
        cp.set_bind_group(0, &build, &[(u64::from(slot) * SLOT) as u32]);
        cp.dispatch_workgroups(plan.rows_per_slot.div_ceil(64), 1, 1);
    }
    if plan.radiance {
        cp.set_pipeline(&g.dome);
        cp.set_bind_group(0, &build, &[0]);
        cp.dispatch_workgroups(plan.node_count().div_ceil(64), 1, 1);
    }
}

/// [`render`], also reporting the CPU packing time and, with `time_gpu`, timestamp queries
/// around each tile's trace and denoise passes (labels `pathtrace trace` / `pathtrace denoise`).
#[allow(clippy::too_many_arguments)]
pub fn render_timed(
    pt: &PtGpu,
    d: &wgpu::Device,
    enc: &mut wgpu::CommandEncoder,
    scene: &Scene3,
    data: &PtScene,
    opts: PathOpts,
    inputs: &PtInputs,
    out: &wgpu::TextureView,
    time_gpu: bool,
) -> RenderTiming {
    let started = std::time::Instant::now();
    let size = [scene.size[0].max(1), scene.size[1].max(1)];
    let limits = d.limits();
    let budget = pt.tile_bytes.min(limits.max_storage_buffer_binding_size).min(limits.max_buffer_size);
    let tiles = tiles(size, budget, opts.denoise);
    let pixels = tiles.iter().map(|t| u64::from(t.size[0]) * u64::from(t.size[1])).max().unwrap_or(1);
    let mut timer = time_gpu.then(|| crate::fx::Timer::new(d, 2 * tiles.len() as u32 + 1));
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
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };
    // interleaved corners, the material index's bits in the first corner's position w
    let mut verts: Vec<[f32; 4]> = Vec::with_capacity(data.pos.len() * 8);
    for (c, (p, n)) in data.pos.iter().zip(&data.nrm).enumerate() {
        let w = if c % 3 == 0 { f32::from_bits(data.tri_mat[c / 3]) } else { 0.0 };
        verts.push([p[0], p[1], p[2], w]);
        verts.push(*n);
        verts.push(data.colors.get(c).copied().unwrap_or([1.0; 4]));
        let uv = data.uv.get(c).copied().unwrap_or([[0.0; 2]; 6]);
        for pair in uv.as_chunks::<2>().0 {
            verts.push([pair[0][0], pair[0][1], pair[1][0], pair[1][1]]);
        }
        if c % 3 == 2 {
            verts.extend([[0.0; 4]; 6]);
        }
    }
    for (&index, record) in &data.splats {
        verts[index * 24..(index + 1) * 24].copy_from_slice(record);
    }
    for (&index, record) in &data.instances {
        verts[index * 24..(index + 1) * 24].copy_from_slice(record);
    }
    let offset = (verts.len() * 4) as u32;
    let mut materials = data.mats.clone();
    for material in &mut materials {
        for map in &mut material.maps {
            map[0] += offset;
        }
    }
    for pixels in data.pixels.chunks(4) {
        verts.push(std::array::from_fn(|i| f32::from_bits(pixels.get(i).copied().unwrap_or(0))));
    }
    let media = crate::volume::pack(&mut verts, &scene.volumes);
    // Light grids: callers check `limit_note` first, which reports the grid sizing errors.
    let grid_plan = crate::volume::plan_light_grid(&scene.volumes, data.lights.len() as u32, scene.env.is_some())
        .unwrap_or_else(|e| panic!("{e}; check pathtrace::limit_note before rendering"));
    let grid_buffer = grid_plan.as_ref().map(|plan| {
        let buffer = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pt-light-grid"),
            size: plan.bytes,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: true,
        });
        buffer
            .slice(..)
            .get_mapped_range_mut()
            .expect("mapped at creation")
            .copy_from_slice(bytemuck::cast_slice(&crate::volume::light_grid_buffer(plan)));
        buffer.unmap();
        buffer
    });
    let grid_group = grid_buffer.as_ref().map(|buffer| {
        d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pathtrace-light-grid"),
            layout: &pt.bgl_grid,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        })
    });
    let tverts = storage("pt-verts", bytemuck::cast_slice(&verts));
    let mats = storage("pt-mats", bytemuck::cast_slice(&materials));
    let nodes = storage("pt-nodes", bytemuck::cast_slice(&data.nodes));
    let mut lights = data.lights.clone();
    for light in &mut lights {
        if light.size[3] > 0.0 {
            light.size[2] = f32::from_bits(light.size[2].to_bits() + offset);
        }
    }
    let lights = storage("pt-lights", bytemuck::cast_slice(&lights));
    let accum = zeroed("pt-accum", pixels * 16);
    let guide = zeroed("pt-guides", pixels * 32);
    let stand_in = zeroed("pt-stand-in", 16);
    // parameter slots: trace dispatches, denoise passes, output
    let samples = opts.samples.max(1);
    let chunks = samples.div_ceil(PER_DISPATCH);
    const PASSES: u32 = DENOISE_PASSES;
    let ambient = scene.lights.iter().filter(|l| l.kind == LightKind::Ambient).fold(Vec3::ZERO, |a, l| a + l.color);
    let lens = scene.dof.map(|f| f.coc_scale / scene.cam.focal_px.max(1e-6) * 0.5).unwrap_or(0.0);
    // These buffers are reused by every tile in command order, bounding peak memory
    // independently of frame area. Clear sums and guides before tracing each tile.
    let ping = opts.denoise.then(|| zeroed("pt-denoise-a", pixels * 16));
    let pong = opts.denoise.then(|| zeroed("pt-denoise-b", pixels * 16));
    let pack_seconds = started.elapsed().as_secs_f64();
    let make_base = |tile: &Tile| Params {
        cam_to_world: scene.cam.view.inverse().to_cols_array_2d(),
        view_proj: (scene.clip_fix * scene.cam.view_proj()).to_cols_array_2d(),
        inv_view_proj: (scene.clip_fix * scene.cam.view_proj()).inverse().to_cols_array_2d(),
        // the dome's full orientation, as the rasteriser applies it
        env_rot: scene.env.as_ref().map(|e| e.rotation).unwrap_or(Mat4::IDENTITY).to_cols_array_2d(),
        size: [size[0] as f32, size[1] as f32, 0.0, 0.0],
        cam: [scene.lens_k1, lens, scene.dof.map(|f| f.focus).unwrap_or(1.0), opts.bounces as f32],
        env: [
            scene.env.as_ref().map(|e| if e.visible { 2.0 } else { 1.0 }).unwrap_or(0.0),
            scene.env.as_ref().map(|e| e.intensity).unwrap_or(0.0),
            scene.cam.orthographic as u32 as f32,
            (ambient.max_element() > 0.0) as u32 as f32,
        ],
        ambient: [ambient.x, ambient.y, ambient.z, data.lights.len() as f32],
        misc: [inputs.backdrop.is_some() as u32 as f32, samples as f32, 1.0, scene.exposure],
        out: [scene.encode_srgb as u32 as f32, 0.0, 0.0, 0.0],
        tile: [tile.origin[0], tile.origin[1], tile.size[0], tile.size[1]],
        media,
    };

    // the grids are built once, before the tiles
    if let (Some(plan), Some(group)) = (&grid_plan, &grid_group) {
        let buffers = GridInputs {
            verts: &tverts,
            mats: &mats,
            nodes: &nodes,
            lights: &lights,
            guide: &guide,
            stand_in: &stand_in,
        };
        build_light_grids(pt, d, enc, &mut timer, plan, group, make_base(&tiles[0]), &buffers, inputs);
    }
    for (tile_index, tile) in tiles.iter().enumerate() {
        enc.clear_buffer(&accum, 0, None);
        enc.clear_buffer(&guide, 0, None);
        let base = make_base(tile);
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
        let groups = [tile.size[0].div_ceil(8), tile.size[1].div_ceil(8)];
        {
            let stamp = timer.as_mut().and_then(|t| t.labelled_pair("pathtrace trace"));
            let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("pathtrace"),
                timestamp_writes: stamp.zip(timer.as_ref()).map(|(a, t)| wgpu::ComputePassTimestampWrites {
                    query_set: &t.set,
                    beginning_of_pass_write_index: Some(a),
                    end_of_pass_write_index: Some(a + 1),
                }),
            });
            cp.set_pipeline(pt.trace_pipeline(d, scene, grid_group.is_some()));
            if let Some(group) = &grid_group {
                cp.set_bind_group(1, group, &[]);
            }
            for c in 0..chunks {
                cp.set_bind_group(0, &g_trace, &[(c as u64 * SLOT) as u32]);
                cp.dispatch_workgroups(groups[0], groups[1], 1);
            }
        }
        let mut fin = &accum;
        if let (Some(ping), Some(pong)) = (&ping, &pong) {
            let stamp = timer.as_mut().and_then(|t| t.labelled_pair("pathtrace denoise"));
            let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("pathtrace-denoise"),
                timestamp_writes: stamp.zip(timer.as_ref()).map(|(a, t)| wgpu::ComputePassTimestampWrites {
                    query_set: &t.set,
                    beginning_of_pass_write_index: Some(a),
                    end_of_pass_write_index: Some(a + 1),
                }),
            });
            cp.set_pipeline(&pt.atrous);
            for k in 0..PASSES {
                let (src, dst) = if k == 0 {
                    (&accum, ping)
                } else if k % 2 == 1 {
                    (ping, pong)
                } else {
                    (pong, ping)
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
                ops: wgpu::Operations {
                    load: if tile_index == 0 {
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
        rp.set_pipeline(&pt.output);
        rp.set_scissor_rect(tile.output_origin[0], tile.output_origin[1], tile.output_size[0], tile.output_size[1]);
        rp.set_bind_group(0, &g_read, &[((chunks + PASSES) as u64 * SLOT) as u32]);
        rp.set_bind_group(1, &gfin, &[]);
        rp.draw(0..3, 0..1);
    }
    if let Some(t) = &timer {
        t.resolve(enc);
    }
    RenderTiming { pack_seconds, gpu: timer }
}
