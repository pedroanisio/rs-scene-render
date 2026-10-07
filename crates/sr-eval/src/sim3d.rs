//! Rigid bodies in 3D: `object3D` elements with a `<rigidBody>` child, stepped
//! by [`sr_sim::physics3d`] beside the 2D world and applied to each FrameGraph
//! as a pose that replaces the object's own transform (`FrameNode::pose3`).
//!
//! An object's start pose is its 3D world transform at `physics@start`, worked
//! out as the renderer does: T(x, y, z) · Rz · Ry · Rx · S in its 3D parent's
//! frame (`@parent` or a parent constraint), else in its 2D container's.
//! Collision shapes follow the primitive and the object's scale at the start.

use std::sync::Arc;

use glam::{DMat3, DMat4, DQuat, DVec3};
use sr_model::element::{children, AttrValue, Element};
use sr_sim::fields::Field;
use sr_sim::physics3d::{
    Body3Spec, BodyKind, Bounds3, Driver3, Joint3Kind, Joint3Spec, Pose3, Shape3, World3, World3Spec,
};

use crate::eval::{FrameGraph, FrameNode};
use crate::program::Program;
use crate::sim::{index_of, num, opt, text, FieldSrc, Graphs};

/// A simulated object.
#[derive(Clone)]
pub(crate) struct Body3Node {
    fragment: bool,
    sequence_budget: Option<usize>,
    pub(crate) id: Arc<str>,
    /// The object's scale at the start (x, y, z).
    scale: [f64; 3],
    /// Ancestor windows and clocks, from the composition down to this object.
    windows: Vec<(f64, Option<f64>, crate::program::Clock)>,
    /// Undeformed object-space surface; each substep maps this original mesh.
    crater_surface: Option<Triangles>,
    /// How a crater that grows from an impact on this body is made.
    crater: Option<Arc<crate::crater::CraterSource>>,
    /// An object of cells that a crater cuts: the cells, how it is cut and the bodies that take its pieces.
    voxels: Option<Arc<crate::voxel_cut::VoxelOwner>>,
    /// An object of cells whose collider is its cells: what its rigidBody says of them.
    pub(crate) cells: Option<Arc<crate::voxel_cut::CellsInfo>>,
    /// An object of cells that a fracture breaks into the bodies that take its pieces.
    cell_fracture: Option<Arc<crate::voxel_cut::CellFracture>>,
    /// A body that only takes a piece of an object of cells.
    slot: bool,
}

/// A crater element with a source: which watch of the world finds its impact, on which body.
#[derive(Clone, Debug)]
pub(crate) struct CraterLink {
    pub(crate) watch: usize,
    pub(crate) owner: usize,
    pub(crate) source: Arc<crate::crater::CraterSource>,
}

/// The 3D world of a document.
pub(crate) struct Phys3 {
    pub(crate) world: Option<World3>,
    pub(crate) bodies: Vec<Body3Node>,
    pub(crate) fractures: Vec<FractureNode>,
    /// Identity of the world's definition, when the caller asked for it.
    pub(crate) spec_digest: Option<[u8; 32]>,
    /// Each body's mass and shape, for loads that depend on them (the water's).
    pub(crate) hulls: Vec<crate::group::BodyHull>,
    /// The impacts the world watches for and the craters that grow from them.
    pub(crate) watches: Vec<sr_sim::physics3d::ImpactWatch>,
    pub(crate) links: Vec<CraterLink>,
    /// Per body: whether it is kinematic and when a dynamic one is released, so that the
    /// animation the world follows can be sampled without a world.
    follow: Vec<(bool, f64)>,
}

/// What `build` makes of the document's 3D bodies.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Plan3 {
    /// A world that will be stepped. `record` keeps the contacts it resolves; `digest` also
    /// hashes its definition (a baked cache stores that identity).
    Simulate { record: bool, digest: bool },
    /// No world: an old cache supplies the poses and shapes are not needed.
    Placeholder,
    /// No world either, but real shapes and the definition's identity, to check a cache that
    /// carries one.
    Verify,
}

fn attr(n: &FrameNode, name: &str, d: f64) -> f64 {
    n.props.get(name).and_then(crate::value::Value::as_num).unwrap_or_else(|| num(&*n.elem, name, d))
}

/// The object's own transform: T(x, y, z) · Rz · Ry · Rx · S.
fn local3(n: &FrameNode) -> DMat4 {
    let deg = |k: &str| attr(n, k, 0.0).to_radians();
    DMat4::from_translation(DVec3::new(attr(n, "x", 0.0), attr(n, "y", 0.0), attr(n, "z", 0.0)))
        * DMat4::from_rotation_z(deg("rotation"))
        * DMat4::from_rotation_y(deg("rotationY"))
        * DMat4::from_rotation_x(deg("rotationX"))
        * DMat4::from_scale(DVec3::new(attr(n, "scaleX", 1.0), attr(n, "scaleY", 1.0), attr(n, "scaleZ", 1.0)))
}

fn embed(a: &crate::eval::Affine) -> DMat4 {
    let [aa, b, c, d, e, f] = a.0;
    DMat4::from_cols_array(&[aa, b, 0.0, 0.0, c, d, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, e, f, 0.0, 1.0])
}

/// The 3D parent of an element and its influence: `@parent`, else a parent constraint's target.
fn parent3(e: &dyn Element) -> Option<(String, f64)> {
    if let Some(AttrValue::Str(pid)) = e.get_attr("parent") {
        return Some((pid.to_string(), 1.0));
    }
    children(e).into_iter().find_map(|c| {
        (c.element_name() == "transformConstraint" && text(c, "type").as_deref() == Some("parent"))
            .then(|| text(c, "target").map(|t| (t, num(c, "influence", 1.0).clamp(0.0, 1.0))))
            .flatten()
    })
}

fn toward(m: DMat4, w: f64) -> DMat4 {
    if w >= 1.0 {
        return m;
    }
    let (s, r, t) = m.to_scale_rotation_translation();
    DMat4::from_scale_rotation_translation(DVec3::ONE.lerp(s, w), DQuat::IDENTITY.slerp(r, w), t * w)
}

/// World matrix of node `i` (an object3D): its pose if simulated, else its transform in its parent's frame.
pub(crate) fn world3(g: &FrameGraph, i: usize, depth: u32) -> DMat4 {
    let n = &g.nodes[i];
    if let Some(m) = n.pose3 {
        return DMat4::from_cols_array(&m);
    }
    let own = local3(n);
    if depth < 32 {
        if let Some((pid, w)) = parent3(&*n.elem) {
            if let Some(j) = index_of(g, &pid) {
                let pw = if matches!(g.nodes[j].kind, "object3D" | "particles3D" | "ocean") {
                    world3(g, j, depth + 1)
                } else {
                    embed(&g.nodes[j].world)
                };
                return toward(pw, w) * own;
            }
        }
    }
    match n.parent {
        Some(p) => embed(&g.nodes[p as usize].world) * own,
        None => own,
    }
}

/// Splits a world matrix into a pose and a scale.
fn decompose(m: &DMat4) -> (Pose3, [f64; 3]) {
    let (c0, c1, c2) = (m.x_axis.truncate(), m.y_axis.truncate(), m.z_axis.truncate());
    let mut s = [c0.length(), c1.length(), c2.length()];
    if c0.cross(c1).dot(c2) < 0.0 {
        s[0] = -s[0];
    }
    let safe = |v: DVec3, k: f64| if k.abs() > 1e-12 { v / k } else { v };
    let r = DMat3::from_cols(safe(c0, s[0]), safe(c1, s[1]), safe(c2, s[2]));
    let q = DQuat::from_mat3(&r).normalize();
    let t = m.w_axis.truncate();
    (Pose3 { pos: t.to_array(), rot: q.to_array() }, s)
}

/// The object's matrix for a pose and scale: T · R · S.
fn compose(p: &Pose3, s: [f64; 3]) -> DMat4 {
    DMat4::from_scale_rotation_translation(DVec3::from(s), DQuat::from_array(p.rot).normalize(), DVec3::from(p.pos))
}

// ------------------------------------------------------------------ shapes

/// Points and triangles.
pub(crate) type Triangles = (Vec<[f64; 3]>, Vec<[u32; 3]>);

/// The local file of mesh asset `key` and its declared format.
pub(crate) fn mesh_path(p: &Program, key: &str) -> Result<(std::path::PathBuf, Option<String>), String> {
    let (doc, id) = p.assets.get(key).ok_or_else(|| format!("asset {key} not found"))?;
    let scene = if *doc == 0 { &p.scene } else { &p.includes.get(*doc as usize - 1).ok_or("include missing")?.1 };
    let a = scene
        .assets
        .as_ref()
        .and_then(|a| a.children.iter().find(|c| c.id() == Some(id.as_str())))
        .ok_or_else(|| format!("asset {id} not found"))?;
    let (src, fmt) = match a {
        sr_model::model::AssetsChild::Mesh(m) => (m.src.clone(), m.format.map(|f| f.to_string())),
        _ => return Err(format!("asset {id} is not a mesh")),
    };
    let base = p.base_dirs.get(*doc as usize).cloned().unwrap_or_default();
    match sr_model::assets::resolve(&src, &base) {
        sr_model::assets::Resolved::Local(pth) => Ok((pth, fmt)),
        sr_model::assets::Resolved::Remote(u) => Err(format!("remote mesh {u}")),
    }
}

/// Triangles of a mesh asset in object space (the model's rest pose, in scene axes).
fn mesh_triangles(p: &Program, n: &FrameNode) -> Result<Triangles, String> {
    let key = n.asset.as_deref().map(str::to_string).or_else(|| text(&*n.elem, "mesh")).ok_or("no @mesh")?;
    mesh_asset_triangles(p, &key, usize::MAX)
}

/// Shared rest-pose geometry in scene axes. The flattened geometry and input
/// file are bounded before copying. Importer-internal/material allocations
/// retain the existing mesh importer's resource behavior.
pub(crate) fn mesh_asset_triangles(p: &Program, key: &str, max_bytes: usize) -> Result<Triangles, String> {
    let (path, fmt) = mesh_path(p, key)?;
    if std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?.len() > max_bytes as u64 {
        return Err(format!("{}: mesh file exceeds geometry budget", path.display()));
    }
    let model = match sr_3d::import::load(&path, fmt.as_deref())? {
        sr_3d::Asset::Model(m) => m,
        sr_3d::Asset::Splats(_) => return Err("splats have no surface to collide with".into()),
    };
    model_triangles(&model, max_bytes)
}

/// The triangles of a model as the renderer places them (its basis times the transform of each node, scene units, scene axes), with the
/// winding turned where that transform mirrors, bounded before they are copied.
pub(crate) fn model_triangles(model: &sr_3d::Model, max_bytes: usize) -> Result<Triangles, String> {
    let locals: Vec<_> = model.nodes.iter().map(|nd| nd.local).collect();
    let worlds = model.world_matrices(&locals);
    let (mut pts, mut tris) = (Vec::new(), Vec::new());
    for (k, nd) in model.nodes.iter().enumerate() {
        let m = model.basis * worlds[k];
        for &pi in &nd.primitives {
            let prim = &model.primitives[pi];
            let vertices = pts.len().checked_add(prim.vertices.len()).ok_or("mesh vertex count overflow")?;
            let triangles = tris.len().checked_add(prim.indices.len() / 3).ok_or("mesh triangle count overflow")?;
            let bytes = vertices
                .checked_mul(24)
                .and_then(|v| triangles.checked_mul(12).and_then(|t| v.checked_add(t)))
                .ok_or("mesh geometry size overflow")?;
            if bytes > max_bytes || vertices > u32::MAX as usize {
                return Err("flattened mesh exceeds geometry budget".into());
            }
            let base = pts.len() as u32;
            pts.extend(prim.vertices.iter().map(|v| m.transform_point3(glam::Vec3::from(v.pos)).as_dvec3().to_array()));
            tris.extend(prim.indices.as_chunks::<3>().0.iter().map(|t| {
                if m.determinant() < 0.0 {
                    [base + t[0], base + t[2], base + t[1]]
                } else {
                    [base + t[0], base + t[1], base + t[2]]
                }
            }));
        }
    }
    if tris.is_empty() {
        return Err("the mesh has no triangles".into());
    }
    Ok((pts, tris))
}

fn prim_triangles(prim: &sr_3d::Primitive) -> Triangles {
    let pts = prim.vertices.iter().map(|v| v.pos.map(|c| c as f64)).collect();
    let tris = prim.indices.as_chunks::<3>().0.iter().map(|t| [t[0], t[1], t[2]]).collect();
    (pts, tris)
}

/// The shape and the mass of an object of cells (primitive voxels) whose `rigidBody` `b` has the cells as its collider: the asset's cells, each
/// `cellSize` of the object (or of the asset, or 1) times the node's scale on a side, and the mass that its density gives them. The origin of
/// the body is the minimum corner of the box of cells, as the origin of the object is.
fn cells_body(
    p: &Program,
    n: &FrameNode,
    b: &dyn Element,
    s: [f64; 3],
    pixels_per_meter: f64,
) -> Result<(Shape3, f64, Arc<crate::voxel_asset::VoxelModel>, [f64; 3]), String> {
    let key = n.asset.as_deref().ok_or("the object names no voxel asset")?;
    let model = crate::voxel_asset::load(p, key)?;
    if s.iter().any(|c| !(c.is_finite() && *c > 0.0)) {
        return Err("a body of cells cannot be mirrored or have a scale that is not positive".into());
    }
    let cell = crate::voxel_asset::cell_size(opt(&*n.elem, "cellSize"), &model);
    let size = s.map(|c| c * cell);
    let density = num(b, "density", 0.0);
    let body = crate::voxels::body(&model.occupancy, size, density, pixels_per_meter)?;
    Ok((body.shape, body.mass, model, size))
}

/// The collision shape of object `n` for `rigidBody` `b`, in the object's axes scaled by `s`.
fn shape_for(
    p: &Program,
    n: &FrameNode,
    b: &dyn Element,
    s: [f64; 3],
    problems: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Shape3 {
    let e: &dyn Element = &*n.elem;
    let kind = text(e, "primitive").unwrap_or_else(|| "box".into());
    let r = attr(n, "radius", 50.0);
    let w = opt(e, "width").unwrap_or(2.0 * r);
    let hh = opt(e, "height");
    let depth = num(e, "depth", 10.0);
    let segs = num(e, "segments", 32.0).clamp(3.0, 512.0) as u32;
    let crater = children(e).into_iter().any(|c| c.element_name() == "crater");
    if crater {
        let admission = (|| -> Result<(), String> {
            let crater = crate::crater::at(n)?.ok_or("missing crater")?;
            let s = segs as usize;
            let grid = |u: usize, v: usize| ((u + 1) * (v + 1), 2 * u * v);
            let counts = match kind.as_str() {
                "plane" => {
                    let s = num(e, "segments", 32.).clamp(1., 1024.) as usize;
                    Some(grid(s, s))
                }
                "sphere" | "globe" => Some(grid(s, (s / 2).max(2))),
                "capsule" => Some(grid(s, 2 * (s / 2).max(2) + 1)),
                "torus" => Some(grid(s, (s / 2).max(3))),
                "box" => Some((24, 12)),
                "cylinder" => Some((4 * s + 6, 4 * s)),
                "cone" => Some((3 * s + 4, 3 * s)),
                _ => None,
            };
            if let Some((vertices, triangles)) = counts {
                let bytes = sr_sim::physics3d::ColliderUpdate3::required_bytes(vertices, triangles)
                    .ok_or("crater collider memory overflow")?;
                if bytes > crater.max_bytes {
                    return Err("crater collider exceeds memory budget".into());
                }
            }
            Ok(())
        })();
        if let Err(error) = admission {
            failures.push(format!("{}: {error}", n.id));
            return Shape3::Sphere(1.);
        }
    }
    let [sx, sy, sz] = s.map(f64::abs);
    let uniform = (sx - sy).abs() <= 1e-9 * sx.max(1.0) && (sx - sz).abs() <= 1e-9 * sx.max(1.0);
    let round_xz = (sx - sz).abs() <= 1e-9 * sx.max(1.0);
    // the object's triangles (object space, unscaled)
    let triangles = |problems: &mut Vec<String>, failures: &mut Vec<String>| -> Option<Triangles> {
        if kind == "globe" && text(e, "terrain").is_some() {
            let budget = (num(e, "terrainMemoryMiB", 128.) as usize).saturating_mul(1 << 20);
            let geometry = p
                .nodes
                .iter()
                .find(|node| node.id == n.id)
                .ok_or("globe collider missing".to_string())
                .and_then(|node| crate::terrain::collider_triangles(p, node, budget));
            return match geometry {
                Ok(t) => Some(t),
                Err(err) => {
                    failures.push(format!("{}: rigidBody: {err}", n.id));
                    None
                }
            };
        }
        let prim = match kind.as_str() {
            "clay" | "text" => match if kind == "clay" {
                crate::solid::clay_mesh(n, 256 << 20)
            } else {
                crate::solid::text_mesh(p, n, 256 << 20)
            } {
                Ok(mesh) => mesh,
                Err(error) => {
                    failures.push(format!("{}: rigidBody: {error}", n.id));
                    return None;
                }
            },
            "sphere" | "globe" => sr_3d::prim::sphere(r as f32, segs),
            "box" => sr_3d::prim::cuboid(w as f32, hh.unwrap_or(2.0 * r) as f32, depth as f32),
            "plane" if crater => sr_3d::prim::plane(
                w as f32,
                hh.unwrap_or(2.0 * r) as f32,
                num(e, "segments", 32.0).clamp(1.0, 1024.0) as u32,
            ),
            "plane" => sr_3d::prim::cuboid(w as f32, hh.unwrap_or(2.0 * r) as f32, 1.0),
            "cylinder" => sr_3d::prim::cylinder(r as f32, r as f32, hh.unwrap_or(2.0 * r) as f32, segs),
            "cone" => sr_3d::prim::cylinder(0.0, r as f32, hh.unwrap_or(2.0 * r) as f32, segs),
            "torus" => sr_3d::prim::torus(r as f32, hh.map(|h| h * 0.5).unwrap_or(0.35 * r).min(r) as f32, segs),
            "capsule" => sr_3d::prim::capsule(r as f32, hh.unwrap_or(4.0 * r).max(2.0 * r) as f32, segs),
            "extrude" => {
                let made = text(e, "path")
                    .ok_or_else(|| "extrude without @path".to_string())
                    .and_then(|d| sr_3d::prim::path_polygons(&d, 0.25))
                    .and_then(|polys| sr_3d::prim::extrude(&polys, depth as f32, num(e, "bevel", 0.0) as f32));
                match made {
                    Ok(p) => p,
                    Err(err) => {
                        failures.push(format!("{}: rigidBody: {err}", n.id));
                        return None;
                    }
                }
            }
            "mesh" => {
                return match mesh_triangles(p, n) {
                    Ok(t) => Some(t),
                    Err(err) => {
                        failures.push(format!("{}: rigidBody: {err}", n.id));
                        None
                    }
                }
            }
            other => {
                problems.push(format!(
                    "{}: rigidBody on a {other} object collides as a sphere of its radius; give it a shape",
                    n.id
                ));
                return None;
            }
        };
        Some(prim_triangles(&prim))
    };
    if crater {
        // Deformation requires the actual tessellated surface, even for shapes
        // normally represented by analytic collision primitives.
        return match triangles(problems, failures) {
            Some((points, mut triangles)) => {
                if s.iter().product::<f64>() < 0. {
                    for triangle in &mut triangles {
                        triangle.swap(1, 2);
                    }
                }
                Shape3::TriMesh(points.into_iter().map(|q| std::array::from_fn(|i| q[i] * s[i])).collect(), triangles)
            }
            None => Shape3::Sphere(r),
        };
    }
    let scaled =
        |(pts, tris): Triangles| (pts.into_iter().map(|q| [q[0] * sx, q[1] * sy, q[2] * sz]).collect::<Vec<_>>(), tris);
    let hull = |problems: &mut Vec<String>, failures: &mut Vec<String>| match triangles(problems, failures) {
        Some(t) => Shape3::Convex(scaled(t).0),
        None => Shape3::Sphere(r * sx.max(sy).max(sz)),
    };
    let box_dims = || match kind.as_str() {
        "plane" => [w * 0.5 * sx, hh.unwrap_or(2.0 * r) * 0.5 * sy, 0.5 * sz],
        "box" | "extrude" | "text" => [w * 0.5 * sx, hh.unwrap_or(2.0 * r) * 0.5 * sy, depth * 0.5 * sz],
        _ => [r * sx, hh.map(|h| h * 0.5).unwrap_or(r) * sy, r * sz],
    };
    let tall = hh.unwrap_or(2.0 * r);
    match text(b, "shape").as_deref().unwrap_or("auto") {
        "box" => Shape3::Box(box_dims()),
        "sphere" => Shape3::Sphere(r * sx.max(sy).max(sz)),
        "capsule" => {
            let total = hh.unwrap_or(4.0 * r).max(2.0 * r);
            Shape3::Capsule((total * 0.5 - r).max(0.0) * sy, r * sx.max(sz))
        }
        "cylinder" => Shape3::Cylinder(tall * 0.5 * sy, r * sx.max(sz)),
        "cone" => Shape3::Cone(tall * 0.5 * sy, r * sx.max(sz)),
        "convex-hull" => hull(problems, failures),
        "trimesh" | "decomposition" => match triangles(problems, failures) {
            Some(t) => {
                let (pts, tris) = scaled(t);
                if text(b, "shape").as_deref() == Some("trimesh") {
                    Shape3::TriMesh(pts, tris)
                } else {
                    Shape3::Decomposition(pts, tris)
                }
            }
            None => Shape3::Sphere(r * sx.max(sy).max(sz)),
        },
        // auto: the primitive's own form where one fits, else its convex hull
        _ => match kind.as_str() {
            "globe" if text(e, "terrain").is_some() => match triangles(problems, failures) {
                Some((points, mut triangles)) => {
                    let points = points.into_iter().map(|p| std::array::from_fn(|i| p[i] * s[i])).collect();
                    if s.iter().product::<f64>() < 0. {
                        for t in &mut triangles {
                            t.swap(1, 2);
                        }
                    }
                    Shape3::TriMesh(points, triangles)
                }
                None => Shape3::Sphere(r * sx.max(sy).max(sz)),
            },
            "box" | "plane" => Shape3::Box(box_dims()),
            "sphere" | "globe" if uniform => Shape3::Sphere(r * sx),
            "cylinder" if round_xz => Shape3::Cylinder(tall * 0.5 * sy, r * sx),
            "cone" if round_xz => Shape3::Cone(tall * 0.5 * sy, r * sx),
            "capsule" if round_xz => {
                let total = hh.unwrap_or(4.0 * r).max(2.0 * r);
                Shape3::Capsule((total * 0.5 - r).max(0.0) * sy, r * sx)
            }
            "text" | "clay" | "extrude" => match triangles(problems, failures) {
                Some((points, mut triangles)) => {
                    let points = points.into_iter().map(|p| std::array::from_fn(|i| p[i] * s[i])).collect();
                    if s.iter().product::<f64>() < 0. {
                        for triangle in &mut triangles {
                            triangle.swap(1, 2);
                        }
                    }
                    if matches!(text(b, "type").as_deref(), Some("static" | "kinematic")) {
                        Shape3::TriMesh(points, triangles)
                    } else {
                        Shape3::Decomposition(points, triangles)
                    }
                }
                None => Shape3::Sphere(r * sx.max(sy).max(sz)),
            },
            _ => hull(problems, failures),
        },
    }
}

// ------------------------------------------------------------------ building

fn flag(e: &dyn Element, n: &str) -> bool {
    match e.get_attr(n) {
        Some(AttrValue::Bool(b)) => b,
        Some(AttrValue::Str(s)) => s == "true",
        _ => false,
    }
}

/// Whether the document has a body of cells: an object of primitive voxels whose rigidBody has the cells as its collider.
pub(crate) fn has_bodies_of_cells(g0: &FrameGraph) -> bool {
    g0.nodes.iter().any(|n| {
        n.kind == "object3D"
            && children(&*n.elem).iter().any(|c| c.element_name() == "rigidBody" && is_body_of_cells(n, *c))
    })
}

/// Whether object `n` with the rigidBody `rigid` is a body of cells: an object of primitive voxels whose collider is its cells (the one predicate of the
/// build of the world and of the refusal of a physics cache).
fn is_body_of_cells(n: &FrameNode, rigid: &dyn Element) -> bool {
    text(&*n.elem, "primitive").as_deref() == Some("voxels")
        && matches!(text(rigid, "shape").as_deref(), None | Some("auto" | "voxels"))
}

/// Why a physics cache cannot hold a world with bodies of cells.
pub(crate) const NO_CACHE_OF_CELLS: &str = "a physics cache holds poses, velocities and contacts and has no place for the revisions and the cuts of bodies of cells (an object of primitive voxels whose rigidBody collider is its cells)";

/// Ids of the 3D bodies in `g0`.
pub(crate) fn body_ids(g0: &FrameGraph) -> Vec<Arc<str>> {
    g0.nodes
        .iter()
        .filter(|n| n.kind == "object3D" && children(&*n.elem).iter().any(|c| c.element_name() == "rigidBody"))
        .map(|n| n.id.clone())
        .collect()
}

/// The 3D world of the document, or `None` without 3D bodies.
pub(crate) fn build(
    p: &Program,
    g0: &FrameGraph,
    plan: Plan3,
    problems: &mut Vec<String>,
    failures: &mut Vec<String>,
) -> Option<Phys3> {
    let ph = p.scene.physics.as_ref();
    let mut bodies = Vec::new();
    let mut specs = Vec::new();
    for (i, n) in g0.nodes.iter().enumerate() {
        if n.kind != "object3D" {
            continue;
        }
        let Some(c) = children(&*n.elem).into_iter().find(|c| c.element_name() == "rigidBody") else { continue };
        let (start, scale) = decompose(&world3(g0, i, 0));
        let kind = match text(c, "type").as_deref() {
            Some("static") => BodyKind::Static,
            Some("kinematic") => BodyKind::Kinematic,
            _ => BodyKind::Dynamic,
        };
        let collides_with = match text(c, "collidesWith").as_deref() {
            None | Some("all") => None,
            Some(list) => Some(list.split([' ', ',']).filter_map(|t| t.trim().parse::<u32>().ok()).collect()),
        };
        let sequence_budget = children(&*n.elem)
            .into_iter()
            .find(|c| c.element_name() == "fracture")
            .filter(|_| {
                n.asset.as_deref().and_then(|key| p.assets.get(key)).is_some_and(|(doc, id)| {
                    let scene = if *doc == 0 { &p.scene } else { &p.includes[*doc as usize - 1].1 };
                    scene.assets.as_ref().is_some_and(|a| {
                        a.children
                            .iter()
                            .any(|a| matches!(a,sr_model::model::AssetsChild::MeshSequence(v) if &v.id==id))
                    })
                })
            })
            .map(|f| (num(f, "maxMemoryMiB", 256.) as usize).saturating_mul(1 << 20));
        // an object of cells whose collider is the cells: the shape and the mass are the cells'
        let of_cells = is_body_of_cells(n, c);
        let mut cells_mass = None;
        let mut cells_of_body = None;
        let shape = if matches!(plan, Plan3::Placeholder) || sequence_budget.is_some() {
            Shape3::Sphere(1.0)
        } else if of_cells {
            let ppm = ph.map_or(100.0, |p| p.pixels_per_meter.get());
            match cells_body(p, n, c, scale, ppm) {
                Ok((shape, mass, model, size)) => {
                    cells_mass = Some(mass);
                    cells_of_body = Some((model, size));
                    shape
                }
                Err(error) => {
                    failures.push(format!("{}: rigidBody: {error}", n.id));
                    Shape3::Sphere(1.0)
                }
            }
        } else {
            shape_for(p, n, c, scale, problems, failures)
        };
        let crater_surface = if children(&*n.elem).into_iter().any(|c| c.element_name() == "crater") {
            match &shape {
                Shape3::TriMesh(points, triangles) => {
                    Some((points.iter().map(|p| std::array::from_fn(|i| p[i] / scale[i])).collect(), triangles.clone()))
                }
                _ => None,
            }
        } else {
            None
        };
        specs.push(Body3Spec {
            kind,
            shape,
            mass: cells_mass.unwrap_or_else(|| num(c, "mass", 1.0)),
            friction: num(c, "friction", 0.5),
            restitution: num(c, "restitution", 0.0),
            linear_damping: num(c, "linearDamping", 0.01),
            angular_damping: num(c, "angularDamping", 0.01),
            velocity: [num(c, "velocityX", 0.0), num(c, "velocityY", 0.0), num(c, "velocityZ", 0.0)],
            angular_velocity: [
                num(c, "angularVelocityX", 0.0),
                num(c, "angularVelocityY", 0.0),
                num(c, "angularVelocityZ", 0.0),
            ],
            group: num(c, "collisionGroup", 0.0) as u32,
            collides_with,
            sensor: flag(c, "sensor"),
            fixed_rotation: flag(c, "fixedRotation"),
            bullet: flag(c, "bullet"),
            activate_at: num(c, "activateAt", 0.0),
            start,
        });
        let mut windows = Vec::new();
        let mut index = p.nodes.iter().position(|node| node.id == n.id);
        while let Some(k) = index {
            let node = &p.nodes[k];
            let (start, end) = if matches!(node.kind, crate::program::Kind::Plain) {
                (node.vis_start, node.vis_end)
            } else {
                (f64::NEG_INFINITY, None)
            };
            windows.push((start, end, node.clock.clone()));
            index = node.parent.map(|i| i as usize);
        }
        windows.reverse();
        // an object of cells with a crater is cut: how, and the slots that take its pieces (made below, after the bodies of the document)
        let has_crater = children(&*n.elem).into_iter().any(|c| c.element_name() == "crater");
        let cells = cells_of_body.map(|(model, size)| {
            let overflow = match text(c, "fragmentOverflow").as_deref() {
                Some("dust") => crate::voxels::Overflow::Dust,
                _ => crate::voxels::Overflow::Error,
            };
            let anchor = match text(c, "anchor").as_deref() {
                Some("largest") => crate::voxel_cut::Anchor::Largest,
                Some("base") => crate::voxel_cut::Anchor::Base,
                _ if kind == BodyKind::Dynamic => crate::voxel_cut::Anchor::Largest,
                _ => crate::voxel_cut::Anchor::Base,
            };
            Arc::new(crate::voxel_cut::CellsInfo {
                model,
                size,
                density: num(c, "density", 0.0),
                pixels_per_meter: ph.map_or(100.0, |p| p.pixels_per_meter.get()),
                min_cells: num(c, "fragmentMinCells", 1.0) as usize,
                max_fragments: num(c, "maxFragments", 64.0) as usize,
                overflow,
                anchor,
            })
        });
        let voxels =
            cells.as_ref().filter(|_| has_crater).map(|info| Arc::new(crate::voxel_cut::VoxelOwner::new(info)));
        bodies.push(Body3Node {
            id: n.id.clone(),
            scale,
            windows,
            crater_surface,
            fragment: false,
            sequence_budget,
            crater: None,
            voxels,
            cells,
            cell_fracture: None,
            slot: false,
        });
    }
    if bodies.is_empty() {
        return None;
    }
    let mut fractures = Vec::new();
    let mut events = Vec::new();
    // the fractures that fire on an impact: the event, the body that hits and the threshold of the impulse (if given)
    let mut by_contact: Vec<(usize, usize, Option<f64>)> = Vec::new();
    let real_count = bodies.len();
    for source in 0..real_count {
        let n = &g0.nodes[index_of(g0, &bodies[source].id).expect("source")];
        let Some(config) =
            children(&*n.elem).into_iter().find_map(|c| c.as_any().downcast_ref::<sr_model::model::Fracture>())
        else {
            continue;
        };
        // the body that breaks it, if it breaks on an impact: another dynamic body of the world
        let projectile = match text(config, "source") {
            None => None,
            Some(id) => {
                let found = bodies.iter().take(real_count).position(|b| *b.id == *id && !b.fragment);
                match found.filter(|&k| k != source && specs[k].kind == BodyKind::Dynamic) {
                    Some(k) => Some(k),
                    None => {
                        failures.push(format!("{}: fracture source {id} must be another dynamic 3D rigidBody", n.id));
                        continue;
                    }
                }
            }
        };
        // an object of cells breaks into the pieces of its partition, which are bodies of cells, and what is too small is dust
        if let Some(info) = bodies[source].cells.clone() {
            let cell = info.size[0] / bodies[source].scale[0];
            let made = (|| -> Result<_, String> {
                let planes;
                let label = |c: [i32; 3]| u32::from(info.model.occupancy.get(c));
                let rule = match text(config, "partition").as_deref() {
                    Some("planes") => {
                        planes = crate::voxel_cut::planes_of(&text(config, "planes").unwrap_or_default(), cell)?;
                        sr_3d::pieces::Partition::Planes(&planes)
                    }
                    Some("labels") => sr_3d::pieces::Partition::Labels(&label),
                    _ => {
                        sr_3d::pieces::Partition::Voronoi { seeds: num(config, "pieces", 8.) as u32, seed: config.seed }
                    }
                };
                crate::voxels::fracture(
                    &info.model.occupancy,
                    rule,
                    &crate::voxels::FracturePolicy {
                        min_cells: info.min_cells,
                        max_fragments: info.max_fragments,
                        overflow: info.overflow,
                    },
                    info.size,
                    info.density,
                    info.pixels_per_meter,
                )
            })();
            let broken = match made {
                Ok(broken) => broken,
                Err(error) => {
                    failures.push(format!("{}: {error}", n.id));
                    continue;
                }
            };
            let event = events.len();
            let impulse = [num(config, "impulseX", 0.), num(config, "impulseY", 0.), num(config, "impulseZ", 0.)];
            let mut fragments = Vec::new();
            let mut grids = Vec::new();
            for piece in &broken.pieces {
                let index = specs.len();
                let mut spec = specs[source].clone();
                spec.kind = BodyKind::Dynamic;
                spec.shape = piece.shape.clone();
                spec.mass = piece.mass;
                spec.velocity = [0.; 3];
                spec.angular_velocity = [0.; 3];
                spec.activate_at = ph.map_or(0., |p| p.start);
                specs.push(spec);
                let mut body = bodies[source].clone();
                body.sequence_budget = None;
                body.fragment = true;
                body.crater_surface = None;
                body.crater = None;
                body.voxels = None;
                body.cells = None;
                body.cell_fracture = None;
                body.scale = bodies[source].scale;
                bodies.push(body);
                fragments.push(sr_sim::physics3d::Fragment3 {
                    body: index,
                    offset: [0.; 3],
                    impulse: impulse.map(|v| v * piece.mass / specs[source].mass),
                });
                let Shape3::Voxels { cells, .. } = &piece.shape else { unreachable!("a piece is of cells") };
                let grid =
                    sr_3d::occupancy::Occupancy::from_cells(cells.iter().map(|c| (*c, info.model.occupancy.get(*c))));
                match grid {
                    Ok(grid) => grids.push((index, Arc::new(grid))),
                    Err(error) => failures.push(format!("{}: {error}", n.id)),
                }
            }
            if let Some(projectile) = projectile {
                by_contact.push((event, projectile, config.min_impulse.map(|v| v.get())));
            }
            events.push(sr_sim::physics3d::Fracture3 {
                source,
                at: num(config, "at", 0.).max(ph.map_or(0., |p| p.start)),
                radial_impulse: num(config, "radialImpulse", 0.),
                fragments,
                contact: projectile.map(|_| sr_sim::physics3d::FractureContact {
                    watch: usize::MAX,
                    energy_fraction: num(config, "energyFraction", 0.3),
                }),
                dust: broken.dust_body.clone(),
            });
            bodies[source].cell_fracture = Some(Arc::new(crate::voxel_cut::CellFracture {
                event,
                source: Arc::new(info.model.occupancy.clone()),
                pieces: grids,
            }));
            continue;
        }
        let prepare = || -> Result<_, String> {
            let start = ph.map_or(0., |p| p.start);
            let step = ph.map_or(1. / 120., |p| p.fixed_step.get());
            // a fracture on an impact is released by the impact, and the partition is the one of the document: taken
            // where the source first takes part
            let at = if projectile.is_some() { start } else { num(config, "at", 0.).max(start) };
            let mut boundary = ((at - start) / step).ceil();
            let mut release = start + boundary * step;
            let mut found = false;
            for _ in 0..1_000_000 {
                if release >= at && enabled(&bodies[source], release) {
                    found = true;
                    break;
                }
                boundary += 1.;
                release = start + boundary * step;
            }
            if !found {
                return Err("fracture source has no eligible boundary within seek work allowance".into());
            }
            let graph = crate::eval::evaluate_for_physics(p, release);
            let sample = &graph.nodes[index_of(&graph, &n.id).ok_or("fracture source missing at release")?];
            crate::fracture::prepare(p, sample, bodies[source].scale, config, specs[source].mass)
        };
        let geometry = match prepare() {
            Ok(g) => g,
            Err(e) => {
                failures.push(format!("{}: {e}", n.id));
                continue;
            }
        };
        let mut fragments = Vec::new();
        let mut indices = Vec::new();
        let impulse = [num(config, "impulseX", 0.), num(config, "impulseY", 0.), num(config, "impulseZ", 0.)];
        for piece in &geometry.pieces {
            let index = specs.len();
            let mut spec = specs[source].clone();
            spec.kind = BodyKind::Dynamic;
            spec.shape = piece.shape.clone();
            spec.mass = piece.mass;
            spec.velocity = [0.; 3];
            spec.angular_velocity = [0.; 3];
            spec.activate_at = ph.map_or(0., |p| p.start);
            specs.push(spec);
            let mut body = bodies[source].clone();
            body.sequence_budget = None;
            body.fragment = true;
            body.crater_surface = None;
            body.crater = None;
            body.voxels = None;
            body.cells = None;
            body.cell_fracture = None;
            body.scale = [1.; 3];
            bodies.push(body);
            fragments.push(sr_sim::physics3d::Fragment3 {
                body: index,
                offset: piece.offset,
                impulse: impulse.map(|v| v * piece.mass / specs[source].mass),
            });
            indices.push(index);
        }
        if let Some(projectile) = projectile {
            by_contact.push((events.len(), projectile, config.min_impulse.map(|v| v.get())));
        }
        events.push(sr_sim::physics3d::Fracture3 {
            source,
            at: num(config, "at", 0.).max(ph.map_or(0., |p| p.start)),
            radial_impulse: num(config, "radialImpulse", 0.),
            fragments,
            // the watch is the one that is made for it below, once the world's others are known
            contact: projectile.map(|_| sr_sim::physics3d::FractureContact {
                watch: usize::MAX,
                energy_fraction: num(config, "energyFraction", 0.3),
            }),
            dust: None,
        });
        fractures.push(FractureNode { event: events.len() - 1, source, indices, geometry });
    }
    // the slots that take the pieces of an object of cells that a crater cuts: bodies of one cell, dynamic and out of the world until a cut gives them a
    // piece, as many as the object says (`maxFragments`), after every other body of the world
    let mut splits = Vec::new();
    for owner in 0..real_count {
        let Some(voxels) = bodies[owner].voxels.clone() else { continue };
        let rock = voxels.settings().rock;
        let one = crate::voxels::body(&one_cell(), rock.size, rock.density, rock.pixels_per_meter);
        let Ok(one) = one else {
            failures
                .push(format!("{}: a body of cells needs a positive cell size, density and scale", bodies[owner].id));
            continue;
        };
        let mut slots = Vec::new();
        for _ in 0..rock.policy.max_fragments {
            let index = specs.len();
            let mut spec = specs[owner].clone();
            spec.kind = BodyKind::Dynamic;
            spec.shape = one.shape.clone();
            spec.mass = one.mass;
            spec.velocity = [0.; 3];
            spec.angular_velocity = [0.; 3];
            spec.activate_at = ph.map_or(0., |p| p.start);
            specs.push(spec);
            let mut body = bodies[owner].clone();
            body.sequence_budget = None;
            body.fragment = true;
            body.slot = true;
            body.crater_surface = None;
            body.crater = None;
            body.voxels = None;
            body.cells = None;
            body.cell_fracture = None;
            bodies.push(body);
            slots.push(index);
        }
        // the owner knows its slots, for the frames
        if let Some(own) = Arc::get_mut(bodies[owner].voxels.as_mut().expect("an owner")) {
            own.slots = slots.clone();
        } else {
            let mut fresh = crate::voxel_cut::VoxelOwner::new(bodies[owner].cells.as_ref().expect("cells"));
            fresh.slots = slots.clone();
            bodies[owner].voxels = Some(Arc::new(fresh));
        }
        splits.push(sr_sim::physics3d::VoxelSplit3 { parent: owner, slots });
    }
    let mut joints = Vec::new();
    for c in ph.map(|p| &p.children[..]).unwrap_or(&[]) {
        let sr_model::model::PhysicsChild::Constraint(k) = c else { continue };
        let find = |id: &str| bodies.iter().position(|b| &*b.id == id);
        let Some(a) = find(&k.a) else { continue };
        let b = match &k.b {
            Some(id) => match find(id) {
                Some(i) => Some(i),
                None => {
                    problems.push(format!("{}: constraint body {id} has no 3D rigidBody", k.id));
                    continue;
                }
            },
            None => None,
        };
        let e: &dyn Element = k;
        let t = text(e, "type").unwrap_or_default();
        let kind = match t.as_str() {
            "spring" => Joint3Kind::Spring,
            "distance" => Joint3Kind::Distance,
            "pin" => Joint3Kind::Pin,
            "rope" => Joint3Kind::Rope,
            "hinge" => Joint3Kind::Hinge,
            "slider" => Joint3Kind::Slider,
            "weld" => Joint3Kind::Weld,
            "ball" => Joint3Kind::Ball,
            _ => Joint3Kind::Motor,
        };
        let default_axis = if kind == Joint3Kind::Slider { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
        let axis = match (opt(e, "axisX"), opt(e, "axisY"), opt(e, "axisZ")) {
            (None, None, None) => default_axis,
            (x, y, z) => [x.unwrap_or(0.0), y.unwrap_or(0.0), z.unwrap_or(0.0)],
        };
        joints.push(Joint3Spec {
            kind,
            a,
            b,
            anchor: match (k.x, k.y) {
                (Some(x), Some(y)) => Some([x, y, opt(e, "z").unwrap_or(0.0)]),
                _ => None,
            },
            axis,
            rest_length: k.rest_length,
            stiffness: k.stiffness,
            damping: k.damping,
            min: k.min_angle,
            max: k.max_angle,
            motor_speed: k.motor_speed,
            max_force: k.max_force.map(|v| v.get()),
            break_force: k.break_force.map(|v| v.get()),
        });
    }
    // 2D joints name 2D bodies; one naming a 2D and a 3D body cannot join them
    for c in ph.map(|p| &p.children[..]).unwrap_or(&[]) {
        let sr_model::model::PhysicsChild::Constraint(k) = c else { continue };
        let is3 = |id: &str| bodies.iter().any(|b| &*b.id == id);
        if !is3(&k.a) && k.b.as_deref().is_some_and(is3) {
            problems.push(format!("{}: constraint joins a 2D body to a 3D one", k.id));
        }
    }
    let follow: Vec<(bool, f64)> = specs.iter().map(|b| (b.kind == BodyKind::Kinematic, b.activate_at)).collect();
    let hulls: Vec<crate::group::BodyHull> = bodies
        .iter()
        .zip(&specs)
        .map(|(node, spec)| {
            let hull = crate::group::BodyHull::new(node.id.clone(), spec.mass, &spec.shape);
            if spec.kind == BodyKind::Dynamic {
                hull
            } else {
                hull.fixed()
            }
        })
        .collect();
    let start = ph.map(|p| p.start).unwrap_or(0.0);
    let step = ph.map(|p| p.fixed_step.get()).unwrap_or(1.0 / 120.0);
    let gravity = [
        ph.map(|p| p.gravity_x).unwrap_or(0.0),
        ph.map(|p| p.gravity_y).unwrap_or(-9.80665),
        ph.map(|p| num(p, "gravityZ", 0.0)).unwrap_or(0.0),
    ];
    let pixels_per_meter = ph.map(|p| p.pixels_per_meter.get()).unwrap_or(100.0);
    let linked = link_craters(g0, &mut bodies, &specs, gravity, pixels_per_meter, step);
    let (mut watches, links) = match linked {
        Ok(found) => found,
        // an old cache cannot serve a crater from an impact, and says so itself
        Err(_) if matches!(plan, Plan3::Placeholder) => (Vec::new(), Vec::new()),
        Err(error) => {
            failures.push(error);
            (Vec::new(), Vec::new())
        }
    };
    // a fracture on an impact watches its projectile against its own body, after the craters' watches
    for (event, projectile, threshold) in by_contact {
        let owner = events[event].source;
        let min_impulse = threshold.unwrap_or_else(|| {
            crate::physcache::rest_threshold(specs[projectile].mass, gravity, pixels_per_meter, step)
        });
        if let Some(contact) = &mut events[event].contact {
            contact.watch = watches.len();
        }
        watches.push(sr_sim::physics3d::ImpactWatch { source: projectile, owner, min_impulse });
    }
    if matches!(plan, Plan3::Placeholder) {
        return Some(Phys3 { world: None, bodies, fractures, spec_digest: None, hulls, watches, links, follow });
    }
    let [fw, fh] = p.size;
    let bounds = match ph.map(|p| p.bounds.to_string()).as_deref() {
        Some("frame") => Bounds3::Frame { w: fw, h: fh, d: fw.max(fh) },
        Some("floor") => Bounds3::Floor { y: fh },
        _ => Bounds3::None,
    };
    let spec = World3Spec {
        fix_internal_edges: ph.is_some_and(|p| p.fix_internal_edges),
        start,
        step,
        gravity,
        pixels_per_meter,
        iterations: ph.map(|p| p.solver_iterations as usize).unwrap_or(8),
        bounds,
        bodies: specs,
        joints,
    };
    let spec_digest = match plan {
        Plan3::Verify | Plan3::Simulate { digest: true, .. } => {
            Some(crate::physcache::digest_world3(&spec, &events, &watches, &links, &splits))
        }
        _ => None,
    };
    let Plan3::Simulate { record, .. } = plan else {
        return Some(Phys3 { world: None, bodies, fractures, spec_digest, hulls, watches, links, follow });
    };
    let log = record.then(|| crate::physcache::contact_config(&spec, &watches));
    let world = World3::new(spec)
        .with_fractures(events)
        .map_err(|e| e.to_string())
        .and_then(|world| world.with_voxel_splits(splits).map_err(|e| e.to_string()))
        .and_then(|world| world.with_impact_watches(watches.clone()));
    let world = match world {
        Ok(world) => Some(match log {
            Some(config) => world.with_contact_log(config),
            None => world,
        }),
        Err(e) => {
            failures.push(e);
            None
        }
    };
    Some(Phys3 { world, bodies, fractures, spec_digest, hulls, watches, links, follow })
}

/// The craters that grow from impacts: for each `crater` element with a `source`, the watch
/// that finds the impact of that body on the crater's owner and how the crater is made.
/// Sets the owner's `crater`.
/// One cell, for the body of a slot.
fn one_cell() -> sr_3d::occupancy::Occupancy {
    sr_3d::occupancy::Occupancy::from_cells([([0, 0, 0], 1u8)]).expect("one cell")
}

fn link_craters(
    g0: &FrameGraph,
    bodies: &mut [Body3Node],
    specs: &[Body3Spec],
    gravity: [f64; 3],
    pixels_per_meter: f64,
    step: f64,
) -> Result<(Vec<sr_sim::physics3d::ImpactWatch>, Vec<CraterLink>), String> {
    let mut watches = Vec::new();
    let mut links = Vec::new();
    for owner in 0..bodies.len() {
        if bodies[owner].slot {
            continue;
        }
        let Some(node) = g0.nodes.iter().find(|n| n.id == bodies[owner].id) else { continue };
        let Some(element) =
            children(&*node.elem).into_iter().find(|c| c.element_name() == "crater" && text(*c, "source").is_some())
        else {
            continue;
        };
        let id = &bodies[owner].id;
        let source_id = text(element, "source").unwrap_or_default();
        let source = bodies
            .iter()
            .position(|b| *b.id == *source_id && !b.fragment)
            .ok_or_else(|| format!("{id}: crater source {source_id} has no 3D rigidBody"))?;
        if specs[source].kind != BodyKind::Dynamic {
            return Err(format!("{id}: crater source {source_id} must be a dynamic body"));
        }
        let scale = bodies[owner].scale;
        let uniform = scale.iter().all(|s| *s > 0.0 && (s - scale[0]).abs() <= 1e-9 * scale[0]);
        if !uniform {
            return Err(format!("{id}: a crater from an impact needs a uniformly scaled owner"));
        }
        let world_gravity = gravity.iter().map(|c| c * c).sum::<f64>().sqrt();
        let volume = sr_sim::physics3d::shape_volume(&specs[source].shape)
            .map_err(|e| format!("{source_id}: a crater source {e}"))?;
        let cfg = crate::crater::CraterSource::read(
            element,
            id,
            world_gravity,
            pixels_per_meter,
            (specs[source].mass, volume),
            scale[0],
            Arc::new(specs[owner].shape.clone()),
        )?;
        let cfg = Arc::new(cfg);
        let min_impulse = crate::physcache::rest_threshold(specs[source].mass, gravity, pixels_per_meter, step);
        links.push(CraterLink { watch: watches.len(), owner, source: cfg.clone() });
        watches.push(sr_sim::physics3d::ImpactWatch { source, owner, min_impulse });
        bodies[owner].crater = Some(cfg);
    }
    Ok((watches, links))
}

// ------------------------------------------------------------------ stepping

pub(crate) struct Driver<'a, 'b> {
    pub(crate) p: &'a Program,
    pub(crate) graphs: &'a mut Graphs<'b>,
    pub(crate) bodies: &'a [Body3Node],
    pub(crate) fields: &'a FieldSrc,
    pub(crate) statics: &'a [Field],
    /// The solvers this world exchanges with, whose outcomes load its bodies.
    pub(crate) group: Option<&'a crate::group::Group>,
}

impl Driver<'_, '_> {
    /// The composition's time when the world's is `t`: the world runs on the time of its group.
    fn composition(&self, t: f64) -> f64 {
        self.group.map_or(t, |g| g.clock().composition_time(t))
    }
}

impl Driver3 for Driver<'_, '_> {
    fn collider(
        &mut self,
        t: f64,
        which: usize,
        revision: Option<u64>,
    ) -> Result<Option<sr_sim::physics3d::ColliderUpdate3>, String> {
        let body = &self.bodies[which];
        let composition = self.composition(t);
        if let Some(budget) = body.sequence_budget {
            use std::hash::{Hash, Hasher};
            let graph = self.graphs.at(composition);
            let Some(i) = index_of(&graph, &body.id) else { return Ok(None) };
            let (sources, _) = crate::fracture::source::load(self.p, &graph.nodes[i], body.scale, budget)?;
            let mut vertices = Vec::new();
            let mut triangles = Vec::new();
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            for mesh in sources {
                let offset = vertices.len() as u32;
                for v in mesh.vertices {
                    v.pos.map(f32::to_bits).hash(&mut hash);
                    vertices.push(v.pos.map(f64::from));
                }
                for t in mesh.indices.as_chunks::<3>().0 {
                    triangles.push(t.map(|i| i + offset));
                }
            }
            triangles.hash(&mut hash);
            let key = hash.finish();
            if revision == Some(key) {
                return Ok(None);
            }
            return Ok(Some(sr_sim::physics3d::ColliderUpdate3 {
                revision: key,
                vertices,
                triangles,
                max_bytes: budget,
            }));
        }
        let Some(i) = ({
            let graph = self.graphs.at(composition);
            index_of(&graph, &body.id).map(|i| (graph, i))
        }) else {
            return Ok(None);
        };
        let Some(crater) = crate::crater::at(&i.0.nodes[i.1])? else { return Ok(None) };
        deformed_surface(body, &crater, revision)
    }

    fn surface(
        &mut self,
        t: f64,
        which: usize,
        revision: Option<u64>,
        impact: Option<&sr_sim::physics3d::Impact3>,
    ) -> Result<Option<sr_sim::physics3d::ColliderUpdate3>, String> {
        let body = &self.bodies[which];
        let composition = self.composition(t);
        let Some(source) = &body.crater else { return self.collider(t, which, revision) };
        // a crater from an impact has nothing to do until the impact has happened
        let Some(impact) = impact else { return Ok(None) };
        let grown = crate::crater::impact_crater(source, impact, t - impact.time)?;
        let graph = self.graphs.at(composition);
        let Some(i) = index_of(&graph, &body.id) else { return Ok(None) };
        let element = children(&*graph.nodes[i].elem)
            .into_iter()
            .find(|c| c.element_name() == "crater")
            .ok_or("missing crater")?;
        let crater = crate::crater::from_impact(element, &grown)?;
        deformed_surface(body, &crater, revision)
    }

    fn voxel_cut(
        &mut self,
        _t: f64,
        parent: usize,
        revision: Option<u64>,
        impact: Option<&sr_sim::physics3d::Impact3>,
    ) -> Result<Option<sr_sim::physics3d::VoxelCut3>, String> {
        let body = &self.bodies[parent];
        let (Some(owner), Some(source)) = (&body.voxels, &body.crater) else { return Ok(None) };
        // one crater is one watch is one impact is one cut: the revision is 1 whenever there is an impact, so a world that asks again finds it installed
        let (None, Some(impact)) = (revision, impact) else { return Ok(None) };
        let cut = owner.cut_for(source, impact, &body.id).map_err(|e| format!("{}: {e}", body.id))?;
        Ok(Some(cut.cut.cut.clone()))
    }

    fn load(
        &mut self,
        _step: u64,
        t: f64,
        which: usize,
        state: &sr_sim::physics3d::BodyState,
    ) -> Result<Option<sr_sim::physics3d::Load3>, String> {
        let Some(group) = self.group else { return Ok(None) };
        if !state.enabled {
            return Ok(None);
        }
        let composition = group.clock().composition_time(t);
        let graphs = &mut *self.graphs;
        group.load(t, which, state, &mut |ocean: &crate::group::GroupOcean, plane: Option<&crate::group::Plane>| {
            water_frame(&graphs.at(composition), ocean, plane)
        })
    }

    fn capture(
        &mut self,
        _step: u64,
        t: f64,
        _source: usize,
        owner: usize,
        centre: [f64; 3],
        impact: &sr_sim::physics3d::Impact3,
    ) -> Result<Option<f64>, String> {
        let Some(crater) = self.bodies[owner].crater.as_ref().filter(|c| c.capture) else { return Ok(None) };
        let grown = crate::crater::impact_crater(crater, impact, 0.0)?;
        let law = grown.law();
        // The window in which the body is arrested: the time that stops a body moving at the impact speed,
        // 2 d / U, which stops any slower one sooner, and a step. After it the body is the ground's: held
        // for good it would hover over the pit the ground opens under it, and a latch would be a state
        // of its own that a replay must restore, so the window is a function of the impact's instant.
        let speed = grown.speed();
        let step = self.p.scene.physics.as_ref().map_or(1.0 / 120.0, |ph| ph.fixed_step.get());
        if t >= impact.time + 2.0 * law.depth / speed + step {
            return Ok(None);
        }
        // only inside the rim radius of the crater, in the owner's frame, scene units
        let from = centre.iter().zip(&impact.point).map(|(c, p)| (c - p) * (c - p)).sum::<f64>().sqrt();
        if from >= law.rim_radius * crater.pixels_per_meter {
            return Ok(None);
        }
        // the mean force of a penetration as deep as the crater: the kinetic energy over the depth, as a
        // deceleration U^2 / (2 d), here in scene units a second squared
        Ok(Some(speed * speed / (2.0 * law.depth) * crater.pixels_per_meter))
    }

    fn enabled(&mut self, t: f64, which: usize) -> bool {
        enabled(&self.bodies[which], self.composition(t))
    }

    fn kinematic(&mut self, t: f64, which: &[usize]) -> Vec<Pose3> {
        let composition = self.composition(t);
        let g = self.graphs.at(composition);
        which
            .iter()
            .map(|&k| index_of(&g, &self.bodies[k].id).map(|i| decompose(&world3(&g, i, 0)).0).unwrap_or_default())
            .collect()
    }
    fn fields(&mut self, t: f64) -> Vec<Field> {
        self.fields.at_step(t, self.graphs, self.statics)
    }
}

/// The owner's surface with `crater` applied, or `None` when it is the one already installed.
fn deformed_surface(
    body: &Body3Node,
    crater: &crate::crater::Deformation,
    revision: Option<u64>,
) -> Result<Option<sr_sim::physics3d::ColliderUpdate3>, String> {
    let Some((points, triangles)) = &body.crater_surface else { return Ok(None) };
    let current = crater.progress.to_bits();
    if revision == Some(current) {
        return Ok(None);
    }
    let bytes = sr_sim::physics3d::ColliderUpdate3::required_bytes(points.len(), triangles.len())
        .ok_or("crater collider memory overflow")?;
    if bytes > crater.max_bytes {
        return Err(format!("{}: crater collider exceeds memory budget", body.id));
    }
    let vertices = points
        .iter()
        .map(|&p| crater.kernel.map(p, crater.progress).map(|m| std::array::from_fn(|i| m.position[i] * body.scale[i])))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(sr_sim::physics3d::ColliderUpdate3 {
        revision: current,
        vertices,
        triangles: triangles.clone(),
        max_bytes: crater.max_bytes,
    }))
}

/// The ocean's water in the world at one instant: its surface as `y = offset + slope x + slope z`,
/// the rest level or `plane` (in the ocean's own axes) when one is given, and where the ocean's
/// own x and z axes point.
fn water_frame(
    graph: &FrameGraph,
    ocean: &crate::group::GroupOcean,
    plane: Option<&crate::group::Plane>,
) -> Result<crate::group::WaterFrame, String> {
    let i = index_of(graph, &ocean.id).ok_or_else(|| format!("{}: the ocean is not in the frame", ocean.id))?;
    let m = world3(graph, i, 0);
    let level = |x: f64, z: f64| match plane {
        None => ocean.water_level,
        Some(p) => p.level + p.slope[0] * (x - p.centroid[0]) + p.slope[1] * (z - p.centroid[1]),
    };
    let at = |x: f64, z: f64| m.transform_point3(DVec3::new(x, level(x, z), z));
    let (p0, px, pz) = (at(0.0, 0.0), at(1.0, 0.0), at(0.0, 1.0));
    let (u, v) = (px - p0, pz - p0);
    // y = a + b x + c z through the three points
    let det = u.x * v.z - u.z * v.x;
    if det.abs() < 1e-9 || !det.is_finite() {
        return Err(format!("{}: a vertical ocean has no surface to float on", ocean.id));
    }
    let b = (u.y * v.z - u.z * v.y) / det;
    let c = (u.x * v.y - u.y * v.x) / det;
    let axes = [m.transform_vector3(DVec3::X).to_array(), m.transform_vector3(DVec3::Z).to_array()];
    Ok(crate::group::WaterFrame {
        surface: sr_sim::hydrostatics::Surface { offset: p0.y - b * p0.x - c * p0.z, slope: [b, c] },
        axes,
    })
}

impl Phys3 {
    /// Feeds `id` what the document gives the world at each of `steps` steps: which bodies
    /// take part, the poses of those that follow animation, the force fields and the
    /// revision of every deforming surface. This is the world's input, sampled on a
    /// schedule that does not depend on the world's own state.
    pub(crate) fn sample_inputs(
        &self,
        drv: &mut Driver<'_, '_>,
        start: f64,
        step: f64,
        steps: u64,
        id: &mut crate::physcache::Identity,
    ) -> Result<(), String> {
        let deforming: Vec<usize> = self
            .bodies
            .iter()
            .enumerate()
            .filter(|(_, b)| b.sequence_budget.is_some() || b.crater_surface.is_some())
            .map(|(i, _)| i)
            .collect();
        let mut revisions = vec![None; self.bodies.len()];
        for k in 0..steps {
            let t = start + k as f64 * step;
            let visible: Vec<bool> = (0..self.bodies.len()).map(|i| drv.enabled(t, i)).collect();
            id.value("enabled", &visible);
            let follow: Vec<usize> = self
                .follow
                .iter()
                .enumerate()
                .filter(|(_, (kinematic, release))| *kinematic || t < release + step)
                .map(|(i, _)| i)
                .collect();
            if !follow.is_empty() {
                id.value("poses", &drv.kinematic(t + step, &follow));
            }
            id.value("fields", &drv.fields(t));
            for &i in &deforming {
                if let Some(update) = drv.collider(t + step, i, revisions[i])? {
                    revisions[i] = Some(update.revision);
                    id.value("surface", &(k, i, update.revision, update.vertices.len(), update.triangles.len()));
                }
            }
        }
        Ok(())
    }
}

pub(crate) struct FractureNode {
    /// The place of the fracture among the world's events (a fracture of cells has one too, and no node here).
    event: usize,
    source: usize,
    indices: Vec<usize>,
    geometry: Arc<crate::fracture::Geometry>,
}

fn enabled(body: &Body3Node, mut t: f64) -> bool {
    for (start, end, clock) in &body.windows {
        if t < *start || end.is_some_and(|end| t >= end) {
            return false;
        }
        t = crate::eval::clock_map(clock, t);
    }
    true
}

/// Sets source poses and the active fracture snapshot, preserving piece visibility.
/// The cells of object `k` and of its pieces at a frame, as the world says (`voxel_revision`) and the asset and the cut make them.
fn voxel_state(
    world: &World3,
    three: &Phys3,
    frame: &sr_sim::physics3d::Frame3,
    k: usize,
    owner: &crate::voxel_cut::VoxelOwner,
    _time: f64,
) -> Result<crate::voxel_cut::SimVoxels, String> {
    let revision =
        frame.voxel_revision.get(k).copied().flatten().ok_or("a body of cells has no revision at this frame")?;
    // the cut of the impact, when there has been one: its rim has the palette indices of the cells it heaped
    let link = three.links.iter().find(|l| l.owner == k);
    let cut = match link.and_then(|l| frame.impacts.get(l.watch).copied().flatten().map(|i| (l, i))) {
        Some((link, impact)) if revision > 0 => Some(owner.cut_for(&link.source, &impact, &three.bodies[k].id)?),
        _ => None,
    };
    let cells_of = |body: usize, rev: u64| {
        world.voxel_cells_at(body, rev).ok_or_else(|| format!("the world no longer has the cells of revision {rev}"))
    };
    let grid = owner.grid_of(k, revision, &cells_of(k, revision)?, cut.as_deref())?;
    let changed_bricks =
        if revision == 0 { Vec::new() } else { world.voxel_bricks_changed(k, 0, revision).unwrap_or_default() };
    let mut pieces = Vec::new();
    for &slot in &owner.slots {
        let Some(rev) = frame.voxel_revision.get(slot).copied().flatten() else { continue };
        let grid = owner.grid_of(slot, rev, &cells_of(slot, rev)?, cut.as_deref())?;
        pieces.push(crate::voxel_cut::SimVoxelPiece {
            body: slot,
            enabled: frame.enabled.get(slot).copied().unwrap_or(false),
            revision: rev,
            grid,
            pose3: compose(&frame.bodies[slot], three.bodies[k].scale).to_cols_array(),
        });
    }
    let steps = (1..=revision)
        .map_while(|r| world.voxel_bricks_changed(k, r - 1, r).map(|bricks| (r, bricks)))
        .collect::<Vec<_>>();
    let steps = if steps.len() as u64 == revision { steps } else { Vec::new() };
    let enabled = frame.enabled.get(k).copied().unwrap_or(true);
    let rock = owner.settings().rock;
    let thrown = cut.as_ref().map(|c| {
        // the loose parts too small to be bodies leave the ground as well, at rest in its frame (a moving owner's motion is not given them)
        let metres = rock.size.map(|s| s / rock.pixels_per_meter);
        let rim: std::collections::BTreeMap<[i32; 3], u8> = c.excavation.rim.iter().copied().collect();
        let dust = c.cut.dust.iter().map(|cell| crate::voxel_crater::Thrown {
            cell: *cell,
            palette: match owner.model.occupancy.get(*cell) {
                0 => rim.get(cell).copied().unwrap_or(1),
                index => index,
            },
            position: std::array::from_fn(|a| (f64::from(cell[a]) + 0.5) * metres[a]),
            velocity: [0.0; 3],
        });
        Arc::new(crate::voxel_cut::ThrownCells {
            cells: c.excavation.thrown.iter().cloned().chain(dust).collect(),
            mass: rock.density * (rock.size[0] * rock.size[1] * rock.size[2]) / rock.pixels_per_meter.powi(3),
        })
    });
    Ok(crate::voxel_cut::SimVoxels { enabled, revision, grid, changed_bricks, steps, thrown, pieces })
}

pub(crate) fn apply(g: &mut FrameGraph, three: &Phys3, frame: &sr_sim::physics3d::Frame3) {
    use std::hash::{Hash, Hasher};
    for (b, pose) in three.bodies.iter().zip(&frame.bodies) {
        if b.fragment {
            continue;
        }
        let Some(i) = index_of(g, &b.id) else { continue };
        g.nodes[i].pose3 = Some(compose(pose, b.scale).to_cols_array());
    }
    for link in &three.links {
        let Some(Some(impact)) = frame.impacts.get(link.watch) else { continue };
        let Some(i) = index_of(g, &three.bodies[link.owner].id) else { continue };
        // the crater of an object of cells is made along the axis of the surface of its cells, and from the speed along it
        let aligned = match &three.bodies[link.owner].voxels {
            Some(owner) => owner.aligned(&link.source, impact),
            None => Ok(*impact),
        };
        let aligned = match aligned {
            Ok(impact) => impact,
            Err(error) => {
                g.fail(format!("{}: {error}", three.bodies[link.owner].id));
                continue;
            }
        };
        match crate::crater::impact_crater(&link.source, &aligned, g.time - aligned.time) {
            Ok(grown) => g.nodes[i].crater_impact = Some(Arc::new(grown)),
            Err(error) => g.fail(format!("{}: {error}", three.bodies[link.owner].id)),
        }
    }
    // an object of cells that breaks into pieces: whole until its time, and then its pieces
    for (k, b) in three.bodies.iter().enumerate() {
        let Some(broken) = &b.cell_fracture else { continue };
        let Some(i) = index_of(g, &b.id) else { continue };
        let fired = frame.fractured.get(broken.event).copied().unwrap_or(false);
        let pieces = if fired {
            broken
                .pieces
                .iter()
                .map(|(body, grid)| crate::voxel_cut::SimVoxelPiece {
                    body: *body,
                    enabled: frame.enabled.get(*body).copied().unwrap_or(false),
                    revision: 0,
                    grid: grid.clone(),
                    pose3: compose(&frame.bodies[*body], b.scale).to_cols_array(),
                })
                .collect()
        } else {
            Vec::new()
        };
        g.nodes[i].voxels = Some(Arc::new(crate::voxel_cut::SimVoxels {
            enabled: frame.enabled.get(k).copied().unwrap_or(true),
            revision: 0,
            grid: broken.source.clone(),
            changed_bricks: Vec::new(),
            steps: Vec::new(),
            thrown: None,
            pieces,
        }));
    }
    // the objects of cells that can be cut: how many cuts each has had, its cells, and the pieces that have come away
    if let Some(world) = three.world.as_ref() {
        for (k, b) in three.bodies.iter().enumerate() {
            let Some(owner) = &b.voxels else { continue };
            let Some(i) = index_of(g, &b.id) else { continue };
            match voxel_state(world, three, frame, k, owner, g.time) {
                Ok(state) => g.nodes[i].voxels = Some(Arc::new(state)),
                Err(error) => g.fail(format!("{}: {error}", b.id)),
            }
        }
    }
    for fracture in &three.fractures {
        if !frame.fractured.get(fracture.event).copied().unwrap_or(false) {
            continue;
        }
        let Some(i) = index_of(g, &three.bodies[fracture.source].id) else { continue };
        let poses: Vec<_> = fracture.indices.iter().map(|&i| frame.bodies[i]).collect();
        let enabled: Vec<_> = fracture.indices.iter().map(|&i| frame.enabled[i]).collect();
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        fracture.geometry.key.hash(&mut hash);
        enabled.hash(&mut hash);
        for pose in &poses {
            for v in pose.pos.iter().chain(&pose.rot) {
                v.to_bits().hash(&mut hash);
            }
        }
        g.nodes[i].fracture = Some(Arc::new(crate::fracture::SimFracture {
            key: hash.finish(),
            geometry: fracture.geometry.clone(),
            poses,
            enabled,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poses_round_trip_through_matrices() {
        let m = DMat4::from_translation(DVec3::new(10.0, -20.0, 30.0))
            * DMat4::from_rotation_z(0.3)
            * DMat4::from_rotation_y(-1.1)
            * DMat4::from_rotation_x(0.7)
            * DMat4::from_scale(DVec3::new(2.0, 3.0, 0.5));
        let (p, s) = decompose(&m);
        let back = compose(&p, s);
        for (a, b) in m.to_cols_array().iter().zip(back.to_cols_array()) {
            assert!((a - b).abs() < 1e-9, "{m:?} vs {back:?}");
        }
    }
}
