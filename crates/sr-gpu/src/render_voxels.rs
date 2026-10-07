//! Voxel objects (`object3D primitive="voxels"`) in the 3D pass: the surface of the cells of a `voxelAsset` as meshes that the raster
//! renderer and the path tracer draw like any other.
//!
//! The surface is made by [`sr_3d::voxel::surface`] (exposed faces merged into quads, kept between frames by revision and remeshed only
//! where the cells changed), and this module is what turns the object's attributes and the asset's palette into the table of classes the
//! mesher reads, the quads into one mesh for each group of cells that share a material, and the groups into draws.
//!
//! A body of cells that a simulation cuts comes with the frame (`FrameNode::voxels`): its grid at the frame's revision, the bricks that each cut changed, and
//! the pieces that came away with their own grids and poses. Each of them is a surface of its own, kept between frames and brought up to date from the bricks
//! of the cuts since the revision it last read; an object that no simulation touches is drawn from the asset.
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
use sr_3d::{AlphaMode, MaterialParams, Occupancy};
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
    /// The quads of the surface held and their fingerprint, for the frame's statistics.
    quads: usize,
    hash: u64,
}

/// A mesh of the quads of one group. What it is drawn with is read from the group at each frame, so that a material that animates is
/// the frame's and not that of the frame that made the mesh.
struct GroupDraw {
    group: GroupKey,
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

/// The bricks that each cut of a body changed, by the revision it made.
type Steps<'a> = &'a [(u64, Vec<[i32; 3]>)];

/// A body of cells to draw: the object itself or a piece that came away from it.
struct Body<'a> {
    /// The piece's body; none for the object.
    id: Option<usize>,
    grid: &'a Occupancy,
    revision: u64,
    /// The bricks each cut changed, for a body that a simulation cuts; none for the asset's cells.
    steps: Option<Steps<'a>>,
    world: Mat4,
}

impl Renderer {
    /// What a group of cells is drawn with at this frame: the document's material as it is now, or the file's or the emission's numbers.
    fn group_material(
        &self,
        group: &GroupKey,
        document: &BTreeMap<String, (MaterialParams, Maps)>,
    ) -> (MaterialParams, Maps) {
        match group {
            GroupKey::Document(id) => document[id].clone(),
            GroupKey::File { kind, numbers } => (file_params(*kind, numbers), Maps::default()),
            GroupKey::Emit { colour, strength } => {
                let c = self.literal_linear(colour.map(|v| f64::from(v) / 255.0));
                let mut p =
                    MaterialParams { base_color: [c[0] as f32, c[1] as f32, c[2] as f32, 1.0], ..Default::default() };
                p.emissive = [c[0] as f32, c[1] as f32, c[2] as f32];
                p.emissive_strength = f64::from_bits(*strength) as f32;
                (p, Maps::default())
            }
        }
    }

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
        let cell_size = sr_eval::voxel_asset::cell_size(a.opt("cellSize"), &model) as f32;
        // measured (see the SREP): the raster shadow of cells smaller than 0.5 in the scene is displaced or lost. The size comes from the file
        // as well as from the document, so the renderer says it where the validator cannot
        let scale = (0..3).map(|c| world.col(c).truncate().length()).fold(f32::INFINITY, f32::min);
        let in_scene = (cell_size * scale * 1e4).round() / 1e4;
        if in_scene < 0.5 {
            let note = format!(
                "{}: cells of {in_scene} in the scene (the cell size times the scale of the object), below 0.5: the shadow that the raster renderer casts from them is displaced or lost, and the path tracer's holds down to 0.05",
                n.id
            );
            if !plan.stats.unsupported.contains(&note) {
                plan.stats.unsupported.push(note);
            }
        }
        let budget = (a.num("surfaceMemoryMiB", 128.0) as usize) << 20;
        // the bodies: what the simulation says when it cuts this object, else the asset's cells
        let mut bodies: Vec<Body> = Vec::new();
        match n.voxels.as_deref() {
            None => bodies.push(Body {
                id: None,
                grid: &model.occupancy,
                revision: model.occupancy.revision(),
                steps: None,
                world,
            }),
            Some(v) => {
                if v.enabled {
                    bodies.push(Body { id: None, grid: &v.grid, revision: v.revision, steps: Some(&v.steps), world });
                }
                for piece in v.pieces.iter().filter(|p| p.enabled) {
                    let pose = Mat4::from_cols_array(&piece.pose3.map(|c| c as f32));
                    bodies.push(Body {
                        id: Some(piece.body),
                        grid: &piece.grid,
                        revision: piece.revision,
                        steps: Some(&[]),
                        world: pose,
                    });
                }
            }
        }
        self.voxel_surfaces.retain(|(id, body), _| *id != n.id || bodies.iter().any(|b| b.id == *body));
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
        for body in &bodies {
            for (_, cells) in body.grid.bricks() {
                for c in cells.iter() {
                    used[usize::from(*c)] = true;
                }
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
        // the classes are what the surface is made from: the looks of the indices and which of them let light through, which a document
        // material that animates its transmission or alpha mode can change between frames
        let looks_hash = sr_eval::rng::hash_str(&format!("{palette:?}|{object_material:?}|{looks:?}|{see_through:?}"));
        let group_of_class: BTreeMap<u8, &GroupKey> = label.iter().map(|(l, i)| (*i, &l.group)).collect();
        let colour_of_class: BTreeMap<u8, Option<[u8; 4]>> = label.iter().map(|(l, i)| (*i, l.colour)).collect();
        let mut drawn_groups = 0;
        // the budget is the object's: each body takes what the ones before it left, in the order of the frame (the object, then its pieces)
        let mut used_quads = 0usize;
        for body in &bodies {
            let tag = body.id.map_or(0, |b| b as u64 + 1);
            // what the meshes depend on: the cells, the colours, the materials and how the object reads them
            let signature = h(&[
                model.occupancy.lineage(),
                tag,
                body.revision,
                model.occupancy.palette_revision(),
                model.materials_fingerprint,
                looks_hash,
            ]);
            let state_key = (n.id.clone(), body.id);
            let mut remeshed = (0, false);
            let left = budget.saturating_sub(used_quads * sr_3d::voxel::surface::BYTES_PER_QUAD);
            if self.voxel_surfaces.get(&state_key).and_then(|s| s.signature) != Some(signature) {
                let state = self.voxel_surfaces.entry(state_key.clone()).or_default();
                // the meshes that this rebuild replaces are released before the new ones are made (the budget counts one set of them)
                state.groups.clear();
                state.signature = None;
                let update = match body.steps {
                    None => state.cache.update(body.grid, &classes, left),
                    Some(steps) => state.cache.update_steps(
                        body.grid,
                        &classes,
                        left,
                        h(&[model.occupancy.lineage(), tag]),
                        body.revision,
                        steps,
                    ),
                };
                match update {
                    Ok(u) => remeshed = (u.remeshed, u.full),
                    Err(e) => {
                        state.signature = None;
                        state.groups.clear();
                        plan.stats.errors.push(format!("{}: {e}", n.id));
                        continue;
                    }
                }
                let quads = state.cache.quads();
                state.quads = quads.len();
                state.hash = sr_3d::voxel::surface::quads_hash(&quads);
                let mut by_group: BTreeMap<&GroupKey, Vec<sr_3d::voxel::surface::Quad>> = BTreeMap::new();
                for q in quads {
                    if let Some(g) = group_of_class.get(&q.class) {
                        by_group.entry(g).or_default().push(q);
                    }
                }
                let mut out = Vec::new();
                let mut failed = false;
                let limit = self.gpu.device.limits().max_buffer_size;
                for (group, quads) in by_group {
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
                        out.clear();
                        failed = true;
                        break;
                    }
                    let mesh = self.three_engine().upload_mesh_owned(primitive.vertices, primitive.indices);
                    out.push(GroupDraw { group: group.clone(), mesh });
                }
                let state = self.voxel_surfaces.get_mut(&state_key).expect("the state of this body");
                state.groups = out;
                // a surface that could not be drawn is made again at the next frame, and says why again
                state.signature = (!failed).then_some(signature);
            }
            // a surface made at an earlier frame, when the bodies before it held fewer quads, must fit what is left as well
            let state = self.voxel_surfaces.get_mut(&state_key).expect("the state of this body");
            if state.quads * sr_3d::voxel::surface::BYTES_PER_QUAD > left && !state.groups.is_empty() {
                state.groups.clear();
                state.signature = None;
                plan.stats.errors.push(format!(
                    "{}: voxel surface exceeds memory budget (surfaceMemoryMiB): the quads of the object and of its pieces cost {} bytes each at the peak and the budget of {budget} bytes admits {} quads in all",
                    n.id,
                    sr_3d::voxel::surface::BYTES_PER_QUAD,
                    budget / sr_3d::voxel::surface::BYTES_PER_QUAD
                ));
                continue;
            }
            if !state.groups.is_empty() {
                used_quads += state.quads;
            }
            let state = &self.voxel_surfaces[&state_key];
            let model_matrix = body.world * Mat4::from_scale(Vec3::splat(cell_size));
            for g in &state.groups {
                let (material, maps) = self.group_material(&g.group, &document);
                draws.push(Draw3 {
                    mesh: MeshSrc::Cached(g.mesh.clone()),
                    model: model_matrix,
                    material,
                    maps,
                    opacity,
                    cast_shadow: cast,
                    receive_shadow: receive,
                    shadow_catcher: catcher,
                });
            }
            drawn_groups += state.groups.len();
            plan.stats.voxel_surfaces.push(crate::render::VoxelSurfaceStat {
                id: n.id.to_string(),
                body: body.id,
                revision: body.revision,
                quads: state.quads,
                hash: state.hash,
                remeshed: remeshed.0,
                full: remeshed.1,
            });
        }
        plan.stats.voxel_groups += drawn_groups;
        plan.stats.voxel_mesh_seconds += started.elapsed().as_secs_f64();
    }
}
