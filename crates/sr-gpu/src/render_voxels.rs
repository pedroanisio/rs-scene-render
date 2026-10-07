//! Voxel objects (`object3D primitive="voxels"`) in the 3D pass: the surface of the cells of a `voxelAsset` as meshes that the raster
//! renderer and the path tracer draw like any other.
//!
//! The surface is made by [`sr_3d::voxel::surface`] (exposed faces merged into quads, kept between frames by revision and remeshed only
//! where the cells changed), and this module is what turns the object's attributes and the asset's palette into the table of classes the
//! mesher reads, the quads into one mesh for each group of cells that share a material, and the groups into draws.
//!
//! * **Material of a palette index**: the material that the object's `palette` names for it (the i-th id of the list), else the object's
//!   own `material`, else the colour and the material of the file; an index with none of the three is an error that names it.
//! * **Groups** are the draws: one for every document material in use, one for the file's indices that look alike but for their colour
//!   (the colour is in the vertices), and one for every distinct emissive colour and strength. A group's quads are one mesh.
//! * **Classes**: indices that look the same (the same document material, or the same colour and file material, or the same emissive
//!   colour and strength) are one class and merge into one quad; a class that lets light through is see-through, so that the faces
//!   between two cells of one glass are not made.
//! * **File materials** are mapped as the engine decides (the format has no photometric unit): diffuse is a rough dielectric, metal has
//!   its `_metal` and `_rough`, glass its `_trans` and index, emit an emission of `_emit` times the colour, blend its `_alpha` as an
//!   alpha-blended opacity; a `_type` the reader does not know is drawn as diffuse and said so.

use super::*;
use sr_3d::voxel::material::{Kind, Material};
use sr_3d::voxel::surface::{expand, Classes, SurfaceCache};
use sr_3d::{AlphaMode, MaterialParams};
use std::collections::BTreeMap;

/// What a group of cells is drawn with.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum GroupKey {
    /// A material of the document, by id: its own colour and maps.
    Document(String),
    /// A material of the file, by its numbers; the colour is the vertices'.
    File { kind: u8, numbers: [Option<u64>; 4] },
    /// An emissive colour and strength, as bits.
    Emit { colour: [u8; 4], strength: u64 },
}

/// The look of a palette index: the group it is drawn in and, when the colour is not the group's, the colour. Indices with an equal look
/// are one class.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Look {
    group: GroupKey,
    colour: Option<[u8; 4]>,
}

/// What a surface was last made from, so that a frame that changed none of it keeps its meshes.
#[derive(Default)]
pub(in crate::render) struct VoxelState {
    cache: SurfaceCache,
    signature: Option<u64>,
    groups: Vec<GroupDraw>,
}

struct GroupDraw {
    material: MaterialParams,
    maps: Maps,
    mesh: Arc<crate::three::MeshGpu>,
}

fn bits(v: Option<f64>) -> Option<u64> {
    v.map(f64::to_bits)
}

/// The group and the look of the palette index `index` of a file whose colour is `colour` and whose material is `material`.
fn file_look(material: Option<&Material>, colour: [u8; 4]) -> Look {
    let Some(m) = material else {
        return Look { group: GroupKey::File { kind: 0, numbers: [None; 4] }, colour: Some(colour) };
    };
    match &m.kind {
        Kind::Emit => {
            Look { group: GroupKey::Emit { colour, strength: m.emit.unwrap_or(1.0).to_bits() }, colour: None }
        }
        kind => {
            let code = match kind {
                Kind::Diffuse | Kind::Other(_) | Kind::Media => 0,
                Kind::Metal => 1,
                Kind::Glass => 2,
                Kind::Blend => 3,
                Kind::Emit => unreachable!(),
            };
            let numbers = match kind {
                Kind::Metal => [bits(m.metal), bits(m.rough), None, None],
                Kind::Glass => [bits(m.trans), bits(m.refractive_index()), bits(m.rough), None],
                Kind::Blend => [bits(m.alpha), None, None, None],
                _ => [bits(m.rough), None, None, None],
            };
            Look { group: GroupKey::File { kind: code, numbers }, colour: Some(colour) }
        }
    }
}

/// The material parameters of a group of the file (the colour is the vertices').
fn file_params(kind: u8, numbers: &[Option<u64>; 4]) -> MaterialParams {
    let n = |i: usize| numbers[i].map(f64::from_bits);
    let mut p = MaterialParams { base_color: [1.0; 4], ..Default::default() };
    match kind {
        1 => {
            p.metallic = n(0).unwrap_or(1.0) as f32;
            p.roughness = n(1).unwrap_or(0.2) as f32;
        }
        2 => {
            p.transmission = n(0).unwrap_or(1.0) as f32;
            p.ior = n(1).unwrap_or(1.5) as f32;
            p.roughness = n(2).unwrap_or(0.05) as f32;
            p.double_sided = true;
        }
        3 => {
            p.alpha_mode = AlphaMode::Blend;
            p.base_color[3] = n(0).unwrap_or(1.0) as f32;
            p.roughness = 0.5;
        }
        _ => p.roughness = n(0).unwrap_or(0.8) as f32,
    }
    p
}

impl Renderer {
    /// The draws of an `object3D` of primitive `voxels`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn voxel_draws(
        &mut self,
        plan: &mut Plan,
        ctx: &Ctx,
        n: &sr_eval::FrameNode,
        world: Mat4,
        opacity: f32,
        (cast, receive, catcher): (bool, bool, bool),
        draws: &mut Vec<Draw3>,
    ) {
        let started = std::time::Instant::now();
        let a = attrs(n);
        let Some(key) = n.asset.as_ref() else {
            plan.stats.errors.push(format!("{}: a voxels object names no voxelAsset", n.id));
            return;
        };
        let model = match sr_eval::voxel_asset::load(ctx.p, key) {
            Ok(m) => m,
            Err(e) => {
                plan.stats.errors.push(format!("{}: {e}", n.id));
                return;
            }
        };
        let cell_size = a.opt("cellSize").or(model.cell_size).unwrap_or(1.0) as f32;
        let budget = (a.num("surfaceMemoryMiB", 128.0) as usize) << 20;
        // the material of each palette index in use
        let palette = a.str("palette");
        let tokens: Vec<String> = match palette.as_deref().map(str::trim) {
            Some("file") | None => Vec::new(),
            Some(list) => list.split_whitespace().map(str::to_string).collect(),
        };
        let from_file = match palette.as_deref().map(str::trim) {
            Some("file") => true,
            None => model.colours.is_some(),
            Some(_) => false,
        };
        let object_material = a.str("material");
        let mut used = [false; 256];
        for (_, cells) in model.occupancy.bricks() {
            for c in cells.iter() {
                used[usize::from(*c)] = true;
            }
        }
        let mut looks: BTreeMap<u8, Look> = BTreeMap::new();
        let mut document: BTreeMap<String, (MaterialParams, Maps)> = BTreeMap::new();
        for index in 1..=255u8 {
            if !used[usize::from(index)] {
                continue;
            }
            let named = tokens.get(usize::from(index) - 1).cloned().or_else(|| {
                if tokens.is_empty() || index as usize > tokens.len() {
                    object_material.clone()
                } else {
                    None
                }
            });
            let look = if let Some(id) = named {
                if !document.contains_key(&id) {
                    match self.document_material(plan, ctx, &id) {
                        Some(m) => {
                            document.insert(id.clone(), m);
                        }
                        None => {
                            plan.stats.errors.push(format!("{}: material {id} not found", n.id));
                            return;
                        }
                    }
                }
                Look { group: GroupKey::Document(id), colour: None }
            } else if from_file {
                let colour = model.occupancy.palette().color(index);
                if let Some(m) = model.materials.get(&index) {
                    let odd = match &m.kind {
                        Kind::Other(t) => Some(t.clone()),
                        Kind::Media => Some("_media".to_string()),
                        _ => None,
                    };
                    if let Some(t) = odd {
                        let note = format!(
                            "{}: the palette index {index} has the material type {t:?}, drawn as diffuse",
                            n.id
                        );
                        if !plan.stats.unsupported.contains(&note) {
                            plan.stats.unsupported.push(note);
                        }
                    }
                }
                file_look(model.materials.get(&index), colour)
            } else {
                plan.stats.errors.push(format!(
                    "{}: the palette index {index} has no material: the palette names none for it, the object has no material and the file has no colours",
                    n.id
                ));
                return;
            };
            looks.insert(index, look);
        }
        // classes: the lowest index of each look labels it; the see-through ones are those of glass and of blended or transmissive materials
        let mut label: BTreeMap<&Look, u8> = BTreeMap::new();
        for (index, look) in &looks {
            label.entry(look).or_insert(*index);
        }
        let see = |look: &Look| match &look.group {
            GroupKey::Document(id) => {
                document.get(id).is_some_and(|(p, _)| p.transmission > 0.0 || p.alpha_mode == AlphaMode::Blend)
            }
            GroupKey::File { kind, .. } => matches!(kind, 2 | 3),
            GroupKey::Emit { .. } => false,
        };
        let class_of = |i: u8| looks.get(&i).map_or(i, |l| label[l]);
        let see_through: Vec<u8> = label.iter().filter(|(l, _)| see(l)).map(|(_, i)| *i).collect();
        let classes = Classes::new(class_of, &see_through);
        // what the meshes depend on: the cells, the colours, the materials and how the object reads them
        let signature = h(&[
            model.occupancy.lineage(),
            model.occupancy.revision(),
            model.occupancy.palette_revision(),
            model.materials_fingerprint,
            sr_eval::rng::hash_str(&format!("{palette:?}|{object_material:?}|{looks:?}")),
        ]);
        let state = self.voxel_surfaces.entry(n.id.clone()).or_default();
        if state.signature != Some(signature) {
            let update = match state.cache.update(&model.occupancy, &classes, budget) {
                Ok(u) => u,
                Err(e) => {
                    state.signature = None;
                    state.groups.clear();
                    plan.stats.errors.push(format!("{}: {e}", n.id));
                    return;
                }
            };
            let _ = update;
            let quads = state.cache.quads();
            let mut by_group: BTreeMap<&GroupKey, Vec<sr_3d::voxel::surface::Quad>> = BTreeMap::new();
            let group_of_class: BTreeMap<u8, &GroupKey> = label.iter().map(|(l, i)| (*i, &l.group)).collect();
            let colour_of_class: BTreeMap<u8, Option<[u8; 4]>> = label.iter().map(|(l, i)| (*i, l.colour)).collect();
            for q in quads {
                if let Some(g) = group_of_class.get(&q.class) {
                    by_group.entry(g).or_default().push(q);
                }
            }
            let mut out = Vec::new();
            let limit = self.gpu.device.limits().max_buffer_size;
            for (group, quads) in by_group {
                let (material, maps) = match group {
                    GroupKey::Document(id) => document[id].clone(),
                    GroupKey::File { kind, numbers } => (file_params(*kind, numbers), Maps::default()),
                    GroupKey::Emit { colour, strength } => {
                        let c = self.literal_linear(colour.map(|v| f64::from(v) / 255.0));
                        let mut p = MaterialParams {
                            base_color: [c[0] as f32, c[1] as f32, c[2] as f32, 1.0],
                            ..Default::default()
                        };
                        p.emissive = [c[0] as f32, c[1] as f32, c[2] as f32];
                        p.emissive_strength = f64::from_bits(*strength) as f32;
                        (p, Maps::default())
                    }
                };
                let colour = |class: u8| -> [f32; 4] {
                    match colour_of_class.get(&class).copied().flatten() {
                        Some(c) => {
                            let l = self.literal_linear(c.map(|v| f64::from(v) / 255.0));
                            [l[0] as f32, l[1] as f32, l[2] as f32, l[3] as f32]
                        }
                        None => [1.0; 4],
                    }
                };
                let primitive = expand(&quads, colour);
                if primitive.vertices.len() as u64 * std::mem::size_of::<sr_3d::Vertex>() as u64 > limit
                    || primitive.indices.len() as u64 * 4 > limit
                {
                    plan.stats.errors.push(format!("{}: voxel surface exceeds device buffer limits", n.id));
                    return;
                }
                let mesh = self.three_engine().upload_mesh(&primitive.vertices, &primitive.indices);
                out.push(GroupDraw { material, maps, mesh });
            }
            let state = self.voxel_surfaces.get_mut(&n.id).expect("the state of this node");
            state.groups = out;
            state.signature = Some(signature);
        }
        let state = &self.voxel_surfaces[&n.id];
        let model = world * Mat4::from_scale(Vec3::splat(cell_size));
        for g in &state.groups {
            draws.push(Draw3 {
                mesh: MeshSrc::Cached(g.mesh.clone()),
                model,
                material: g.material.clone(),
                maps: g.maps.clone(),
                opacity,
                cast_shadow: cast,
                receive_shadow: receive,
                shadow_catcher: catcher,
            });
        }
        plan.stats.voxel_groups += state.groups.len();
        plan.stats.voxel_mesh_seconds += started.elapsed().as_secs_f64();
    }
}
