//! 3D assets for scene-render: everything the GPU renderer needs that can be
//! computed on the CPU.
//!
//! * **Models** import from glTF/GLB, OBJ, PLY, FBX (ufbx), and USDA/USDZ into
//!   one [`Model`]: a node hierarchy with triangle primitives, materials,
//!   textures, skins, morph targets, animations and material variants.
//! * **Splats** import from `.splat` and 3D Gaussian Splatting `.ply` files.
//! * **Primitives** build the schema's procedural shapes, including extruded
//!   paths and text, in scene units.
//! * **Lights, environments and cameras**: colour temperature, IES profiles,
//!   prefiltered HDRI environments, the physical camera and camera shake.
//!
//! Scene space is the composition's pixel space extended to 3D: x right,
//! y down, z away from the default camera. Imported Y-up assets measured in
//! metres map into it through [`Y_UP_METRES`] (100 px per metre).

// Keep numeric loops explicit; allow equivalent spellings across Clippy versions.
// lints some clippy versions lack: allowed where known
#![allow(unknown_lints, clippy::chunks_exact_to_as_chunks, clippy::manual_is_multiple_of)]

pub mod anim;
pub mod camera;
pub mod clay;
pub mod crater;
pub mod env;
pub mod fracture;
pub mod import;
pub mod light;
pub mod material;
pub mod mtlx;
mod mtlx_graph;
pub mod occupancy;
pub mod pieces;

pub mod prim;
pub mod sampling;
pub mod sequence;
pub mod terrain;
pub mod usdc;
pub mod voxel;

use glam::{Mat4, Quat, Vec3};

pub use material::{AlphaMode, FoamMix, MaterialParams};
pub use occupancy::Occupancy;

/// Y-up, metre-based assets (glTF, OBJ, PLY, FBX, USD after unit conversion)
/// to scene space: a 180° turn about x, 100 scene units per metre.
pub const Y_UP_METRES: Mat4 =
    Mat4::from_cols_array(&[100.0, 0.0, 0.0, 0.0, 0.0, -100.0, 0.0, 0.0, 0.0, 0.0, -100.0, 0.0, 0.0, 0.0, 0.0, 1.0]);

/// A vertex as the GPU reads it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// xyz tangent, w handedness.
    pub tangent: [f32; 4],
    /// Normal, metallic-roughness, occlusion and emissive UVs, when the material uses separate UVs.
    pub map_uv: [[f32; 2]; 4],
    /// Linear vertex colour, multiplied by the material base colour.
    pub color: [f32; 4],
}

impl Default for Vertex {
    fn default() -> Self {
        Self {
            pos: [0.0; 3],
            normal: [0.0; 3],
            uv: [0.0; 2],
            tangent: [0.0; 4],
            map_uv: [[0.0; 2]; 4],
            color: [1.0; 4],
        }
    }
}

/// Per-vertex morph target offsets.
#[derive(Clone, Debug, Default)]
pub struct MorphTarget {
    pub dpos: Vec<[f32; 3]>,
    pub dnormal: Vec<[f32; 3]>,
}

/// A triangle list with one material.
#[derive(Clone, Debug, Default)]
pub struct Primitive {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    /// Index into [`Model::materials`].
    pub material: Option<usize>,
    /// Imported UV sets, including sets selected by a material variant.
    pub tex_coords: std::collections::BTreeMap<u32, Vec<[f32; 2]>>,
    pub morphs: Vec<MorphTarget>,
    /// Up to four (joint, weight) pairs per vertex.
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    /// Material per variant name (KHR_materials_variants).
    pub variants: Vec<(String, usize)>,
}

/// Translation, rotation, scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trs {
    pub t: Vec3,
    pub r: Quat,
    pub s: Vec3,
}

impl Default for Trs {
    fn default() -> Trs {
        Trs { t: Vec3::ZERO, r: Quat::IDENTITY, s: Vec3::ONE }
    }
}

impl Trs {
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.s, self.r, self.t)
    }
}

/// A node of an imported model.
#[derive(Clone, Debug, Default)]
pub struct Node {
    pub name: String,
    pub parent: Option<usize>,
    pub local: Trs,
    /// Primitives drawn with this node's transform.
    pub primitives: Vec<usize>,
    pub skin: Option<usize>,
    /// Default morph weights of the node's mesh.
    pub weights: Vec<f32>,
    /// The names of the mesh's morph targets, in weight order (empty when the file names none).
    pub morph_names: Vec<String>,
}

/// Joints and inverse bind matrices.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skin {
    pub joints: Vec<usize>,
    pub inverse_bind: Vec<Mat4>,
}

/// A decoded texture.
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    /// RGBA8, straight alpha.
    pub rgba: Vec<u8>,
    /// Encoded with the sRGB transfer (colour maps) or linear (data maps).
    pub srgb: bool,
    /// Explicit base-level filtering; absent uses the renderer's mipmapped default.
    pub sampler: Option<sampling::TextureSampler>,
}

/// Texture slots of an imported material (indices into [`Model::textures`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaterialMaps {
    pub base_color: Option<usize>,
    pub normal: Option<usize>,
    pub metallic_roughness: Option<usize>,
    pub occlusion: Option<usize>,
    pub emissive: Option<usize>,
}

/// KHR_texture_transform: scale, then rotate counterclockwise, then translate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureTransform {
    pub tex_coord: u32,
    pub offset: [f32; 2],
    pub scale: [f32; 2],
    pub rotation: f32,
}

impl Default for TextureTransform {
    fn default() -> Self {
        Self { tex_coord: 0, offset: [0.0; 2], scale: [1.0; 2], rotation: 0.0 }
    }
}

impl TextureTransform {
    pub fn apply(&self, uv: [f32; 2]) -> [f32; 2] {
        let (sin, cos) = self.rotation.sin_cos();
        let [x, y] = [uv[0] * self.scale[0], uv[1] * self.scale[1]];
        [self.offset[0] + cos * x - sin * y, self.offset[1] + sin * x + cos * y]
    }
}

/// An imported material.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportedMaterial {
    pub name: String,
    pub params: MaterialParams,
    pub maps: MaterialMaps,
    /// Base colour, normal, metallic-roughness, occlusion and emissive, respectively.
    pub texture_transforms: [TextureTransform; 5],
}

impl ImportedMaterial {
    /// Bakes the active material's UV selection/transforms into an instance of
    /// the mesh. The original UV sets stay intact for material variants.
    pub fn apply_texture_coordinates(&self, primitive: &Primitive, vertices: &mut [Vertex]) {
        if !self.params.separate_uvs {
            return;
        }
        for (i, vertex) in vertices.iter_mut().enumerate() {
            let coords = self.texture_transforms.map(|transform| {
                let uv = primitive
                    .tex_coords
                    .get(&transform.tex_coord)
                    .and_then(|set| set.get(i))
                    .copied()
                    .unwrap_or(primitive.vertices[i].uv);
                transform.apply(uv)
            });
            vertex.uv = coords[0];
            vertex.map_uv.copy_from_slice(&coords[1..]);
        }
        if self.maps.normal.is_some() {
            // The normal map's UV parameterisation defines its tangent frame.
            for vertex in vertices.iter_mut() {
                vertex.uv = vertex.map_uv[0];
            }
            compute_tangents(vertices, &primitive.indices);
            for (i, vertex) in vertices.iter_mut().enumerate() {
                let t = self.texture_transforms[0];
                vertex.uv = t.apply(
                    primitive
                        .tex_coords
                        .get(&t.tex_coord)
                        .and_then(|set| set.get(i))
                        .copied()
                        .unwrap_or(primitive.vertices[i].uv),
                );
            }
        }
    }
}

/// A keyframed property of an animation channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    Translation,
    Rotation,
    Scale,
    Weights,
}

/// Keyframe interpolation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interp {
    Step,
    Linear,
    CubicSpline,
}

/// One animated property of one node.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub node: usize,
    pub path: Path,
    pub interp: Interp,
    pub times: Vec<f32>,
    /// Flattened values: 3 (T, S), 4 (R) or N (weights) per key; cubic keys carry in-tangent, value, out-tangent.
    pub values: Vec<f32>,
}

/// A named animation clip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Animation {
    pub name: String,
    pub channels: Vec<Channel>,
    pub duration: f32,
}

/// An imported model.
#[derive(Clone, Debug, Default)]
pub struct Model {
    pub nodes: Vec<Node>,
    pub primitives: Vec<Primitive>,
    pub materials: Vec<ImportedMaterial>,
    pub textures: Vec<Texture>,
    pub skins: Vec<Skin>,
    pub animations: Vec<Animation>,
    /// Source space to scene space.
    pub basis: Mat4,
    /// Problems that did not stop the import.
    pub warnings: Vec<String>,
}

impl Model {
    /// World matrices of every node in the model's own space (before `basis`).
    pub fn world_matrices(&self, locals: &[Trs]) -> Vec<Mat4> {
        let mut out = vec![Mat4::IDENTITY; self.nodes.len()];
        let mut done = vec![false; self.nodes.len()];
        fn visit(m: &Model, locals: &[Trs], i: usize, out: &mut [Mat4], done: &mut [bool]) {
            if done[i] {
                return;
            }
            let local = locals[i].matrix();
            out[i] = match m.nodes[i].parent {
                Some(p) => {
                    visit(m, locals, p, out, done);
                    out[p] * local
                }
                None => local,
            };
            done[i] = true;
        }
        for i in 0..self.nodes.len() {
            visit(self, locals, i, &mut out, &mut done);
        }
        out
    }

    /// Axis-aligned bounds of all primitives in scene space at rest.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let locals: Vec<Trs> = self.nodes.iter().map(|n| n.local).collect();
        let world = self.world_matrices(&locals);
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for (i, n) in self.nodes.iter().enumerate() {
            let m = self.basis * world[i];
            for &p in &n.primitives {
                for v in &self.primitives[p].vertices {
                    let q = m.transform_point3(Vec3::from(v.pos));
                    lo = lo.min(q);
                    hi = hi.max(q);
                }
            }
        }
        if lo.x > hi.x {
            (Vec3::ZERO, Vec3::ZERO)
        } else {
            (lo, hi)
        }
    }
}

/// 3D Gaussian splats.
#[derive(Clone, Debug, Default)]
pub struct Splats {
    pub pos: Vec<[f32; 3]>,
    /// Standard deviations along the local axes.
    pub scale: Vec<[f32; 3]>,
    /// Unit quaternion (x, y, z, w).
    pub rot: Vec<[f32; 4]>,
    /// Linear sRGB colour and opacity (the view-independent colour).
    pub color: Vec<[f32; 4]>,
    /// Spherical-harmonic colour of 3D Gaussian Splatting captures, when present: 16 RGB
    /// coefficients per splat (coefficient-major; coefficient 0 is the DC term), of which the
    /// first (`sh_degree` + 1)² are used. Evaluated as 3DGS does: 0.5 + Σ Yₖ(d) cₖ in the
    /// capture's encoded colour, for the direction d from the camera in the splats' own frame.
    pub sh: Vec<[f32; 48]>,
    pub sh_degree: u32,
    /// Source space to scene space.
    pub basis: Mat4,
}

impl Splats {
    pub fn len(&self) -> usize {
        self.pos.len()
    }
    pub fn is_empty(&self) -> bool {
        self.pos.is_empty()
    }
}

/// A loaded mesh asset.
#[derive(Clone, Debug)]
pub enum Asset {
    Model(Model),
    Splats(Splats),
}

/// Recomputes smooth normals from triangles (area-weighted).
pub fn compute_normals(v: &mut [Vertex], idx: &[u32]) {
    let mut acc = vec![Vec3::ZERO; v.len()];
    for t in idx.chunks_exact(3) {
        let (a, b, c) =
            (Vec3::from(v[t[0] as usize].pos), Vec3::from(v[t[1] as usize].pos), Vec3::from(v[t[2] as usize].pos));
        let n = (b - a).cross(c - a);
        for &k in t {
            acc[k as usize] += n;
        }
    }
    for (vx, n) in v.iter_mut().zip(acc) {
        vx.normal = n.normalize_or_zero().into();
    }
}

/// Computes per-vertex tangents from UVs (per-triangle accumulation, Gram-Schmidt).
pub fn compute_tangents(v: &mut [Vertex], idx: &[u32]) {
    let mut tan = vec![Vec3::ZERO; v.len()];
    let mut bit = vec![Vec3::ZERO; v.len()];
    for t in idx.chunks_exact(3) {
        let (i0, i1, i2) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let (p0, p1, p2) = (Vec3::from(v[i0].pos), Vec3::from(v[i1].pos), Vec3::from(v[i2].pos));
        let (u0, u1, u2) = (glam::Vec2::from(v[i0].uv), glam::Vec2::from(v[i1].uv), glam::Vec2::from(v[i2].uv));
        let (e1, e2) = (p1 - p0, p2 - p0);
        let (d1, d2) = (u1 - u0, u2 - u0);
        let det = d1.x * d2.y - d2.x * d1.y;
        if det.abs() < 1e-12 {
            continue;
        }
        let r = 1.0 / det;
        let sdir = (e1 * d2.y - e2 * d1.y) * r;
        let tdir = (e2 * d1.x - e1 * d2.x) * r;
        for &k in &[i0, i1, i2] {
            tan[k] += sdir;
            bit[k] += tdir;
        }
    }
    for (i, vx) in v.iter_mut().enumerate() {
        let n = Vec3::from(vx.normal);
        let mut t = tan[i] - n * n.dot(tan[i]);
        if t.length_squared() < 1e-20 {
            t = n.any_orthonormal_vector();
        }
        let t = t.normalize();
        let w = if n.cross(t).dot(bit[i]) < 0.0 { -1.0 } else { 1.0 };
        vx.tangent = [t.x, t.y, t.z, w];
    }
}
