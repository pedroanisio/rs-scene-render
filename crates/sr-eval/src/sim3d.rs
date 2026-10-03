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
}

/// The 3D world of a document.
pub(crate) struct Phys3 {
    pub(crate) world: Option<World3>,
    pub(crate) bodies: Vec<Body3Node>,
    pub(crate) fractures: Vec<FractureNode>,
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

/// The collision shape of object `n` for `rigidBody` `b`, in the object's axes scaled by `s`.
fn shape_for(p: &Program, n: &FrameNode, b: &dyn Element, s: [f64; 3], problems: &mut Vec<String>) -> Shape3 {
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
            problems.push(format!("{}: {error}", n.id));
            return Shape3::Sphere(1.);
        }
    }
    let [sx, sy, sz] = s.map(f64::abs);
    let uniform = (sx - sy).abs() <= 1e-9 * sx.max(1.0) && (sx - sz).abs() <= 1e-9 * sx.max(1.0);
    let round_xz = (sx - sz).abs() <= 1e-9 * sx.max(1.0);
    // the object's triangles (object space, unscaled)
    let triangles = |problems: &mut Vec<String>| -> Option<Triangles> {
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
                    problems.push(format!("{}: rigidBody: {err}", n.id));
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
                    problems.push(format!("{}: rigidBody: {error}", n.id));
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
                        problems.push(format!("{}: rigidBody: {err}", n.id));
                        return None;
                    }
                }
            }
            "mesh" => {
                return match mesh_triangles(p, n) {
                    Ok(t) => Some(t),
                    Err(err) => {
                        problems.push(format!("{}: rigidBody: {err}", n.id));
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
        return match triangles(problems) {
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
    let hull = |problems: &mut Vec<String>| match triangles(problems) {
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
        "convex-hull" => hull(problems),
        "trimesh" | "decomposition" => match triangles(problems) {
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
            "globe" if text(e, "terrain").is_some() => match triangles(problems) {
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
            "text" | "clay" | "extrude" => match triangles(problems) {
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
            _ => hull(problems),
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

/// Ids of the 3D bodies in `g0`.
pub(crate) fn body_ids(g0: &FrameGraph) -> Vec<Arc<str>> {
    g0.nodes
        .iter()
        .filter(|n| n.kind == "object3D" && children(&*n.elem).iter().any(|c| c.element_name() == "rigidBody"))
        .map(|n| n.id.clone())
        .collect()
}

/// The 3D world of the document, or `None` without 3D bodies.
pub(crate) fn build(p: &Program, g0: &FrameGraph, cached: bool, problems: &mut Vec<String>) -> Option<Phys3> {
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
        let shape =
            if cached || sequence_budget.is_some() { Shape3::Sphere(1.0) } else { shape_for(p, n, c, scale, problems) };
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
            mass: num(c, "mass", 1.0),
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
        bodies.push(Body3Node { id: n.id.clone(), scale, windows, crater_surface, fragment: false, sequence_budget });
    }
    if bodies.is_empty() {
        return None;
    }
    let mut fractures = Vec::new();
    let mut events = Vec::new();
    let real_count = bodies.len();
    for source in 0..real_count {
        let n = &g0.nodes[index_of(g0, &bodies[source].id).expect("source")];
        let Some(config) =
            children(&*n.elem).into_iter().find_map(|c| c.as_any().downcast_ref::<sr_model::model::Fracture>())
        else {
            continue;
        };
        let prepare = || -> Result<_, String> {
            let start = ph.map_or(0., |p| p.start);
            let step = ph.map_or(1. / 120., |p| p.fixed_step.get());
            let at = num(config, "at", 0.).max(start);
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
                problems.push(format!("{}: {e}", n.id));
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
            body.scale = [1.; 3];
            bodies.push(body);
            fragments.push(sr_sim::physics3d::Fragment3 {
                body: index,
                offset: piece.offset,
                impulse: impulse.map(|v| v * piece.mass / specs[source].mass),
            });
            indices.push(index);
        }
        events.push(sr_sim::physics3d::Fracture3 {
            source,
            at: num(config, "at", 0.).max(ph.map_or(0., |p| p.start)),
            radial_impulse: num(config, "radialImpulse", 0.),
            fragments,
        });
        fractures.push(FractureNode { source, indices, geometry });
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
    if cached {
        return Some(Phys3 { world: None, bodies, fractures });
    }
    let [fw, fh] = p.size;
    let bounds = match ph.map(|p| p.bounds.to_string()).as_deref() {
        Some("frame") => Bounds3::Frame { w: fw, h: fh, d: fw.max(fh) },
        Some("floor") => Bounds3::Floor { y: fh },
        _ => Bounds3::None,
    };
    let spec = World3Spec {
        start: ph.map(|p| p.start).unwrap_or(0.0),
        step: ph.map(|p| p.fixed_step.get()).unwrap_or(1.0 / 120.0),
        gravity: [
            ph.map(|p| p.gravity_x).unwrap_or(0.0),
            ph.map(|p| p.gravity_y).unwrap_or(-9.80665),
            ph.map(|p| num(p, "gravityZ", 0.0)).unwrap_or(0.0),
        ],
        pixels_per_meter: ph.map(|p| p.pixels_per_meter.get()).unwrap_or(100.0),
        iterations: ph.map(|p| p.solver_iterations as usize).unwrap_or(8),
        bounds,
        bodies: specs,
        joints,
    };
    let world = match World3::new(spec).with_fractures(events) {
        Ok(world) => Some(world),
        Err(e) => {
            problems.push(e.to_string());
            None
        }
    };
    Some(Phys3 { world, bodies, fractures })
}

// ------------------------------------------------------------------ stepping

pub(crate) struct Driver<'a, 'b> {
    pub(crate) p: &'a Program,
    pub(crate) graphs: &'a mut Graphs<'b>,
    pub(crate) bodies: &'a [Body3Node],
    pub(crate) fields: &'a FieldSrc,
    pub(crate) statics: &'a [Field],
}

impl Driver3 for Driver<'_, '_> {
    fn collider(
        &mut self,
        t: f64,
        which: usize,
        revision: Option<u64>,
    ) -> Result<Option<sr_sim::physics3d::ColliderUpdate3>, String> {
        let body = &self.bodies[which];
        if let Some(budget) = body.sequence_budget {
            use std::hash::{Hash, Hasher};
            let graph = self.graphs.at(t);
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
        let Some((points, triangles)) = &body.crater_surface else { return Ok(None) };
        let graph = self.graphs.at(t);
        let Some(i) = index_of(&graph, &body.id) else { return Ok(None) };
        let Some(crater) = crate::crater::at(&graph.nodes[i])? else { return Ok(None) };
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
            .map(|&p| {
                crater.kernel.map(p, crater.progress).map(|m| std::array::from_fn(|i| m.position[i] * body.scale[i]))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(sr_sim::physics3d::ColliderUpdate3 {
            revision: current,
            vertices,
            triangles: triangles.clone(),
            max_bytes: crater.max_bytes,
        }))
    }

    fn enabled(&mut self, t: f64, which: usize) -> bool {
        enabled(&self.bodies[which], t)
    }

    fn kinematic(&mut self, t: f64, which: &[usize]) -> Vec<Pose3> {
        let g = self.graphs.at(t);
        which
            .iter()
            .map(|&k| index_of(&g, &self.bodies[k].id).map(|i| decompose(&world3(&g, i, 0)).0).unwrap_or_default())
            .collect()
    }
    fn fields(&mut self, t: f64) -> Vec<Field> {
        self.fields.at_step(t, self.graphs, self.statics)
    }
}

pub(crate) struct FractureNode {
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
pub(crate) fn apply(g: &mut FrameGraph, three: &Phys3, frame: &sr_sim::physics3d::Frame3) {
    use std::hash::{Hash, Hasher};
    for (b, pose) in three.bodies.iter().zip(&frame.bodies) {
        if b.fragment {
            continue;
        }
        let Some(i) = index_of(g, &b.id) else { continue };
        g.nodes[i].pose3 = Some(compose(pose, b.scale).to_cols_array());
    }
    for (k, fracture) in three.fractures.iter().enumerate() {
        if !frame.fractured.get(k).copied().unwrap_or(false) {
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
