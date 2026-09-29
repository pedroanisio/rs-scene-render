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
pub(crate) struct Body3Node {
    pub(crate) id: Arc<str>,
    /// The object's scale at the start (x, y, z).
    scale: [f64; 3],
}

/// The 3D world of a document.
pub(crate) struct Phys3 {
    pub(crate) world: Option<World3>,
    pub(crate) bodies: Vec<Body3Node>,
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
                let pw = if g.nodes[j].kind == "object3D" { world3(g, j, depth + 1) } else { embed(&g.nodes[j].world) };
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
type Triangles = (Vec<[f64; 3]>, Vec<[u32; 3]>);

/// The local file of mesh asset `key` and its declared format.
fn mesh_path(p: &Program, key: &str) -> Result<(std::path::PathBuf, Option<String>), String> {
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
    let (path, fmt) = mesh_path(p, &key)?;
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
            let base = pts.len() as u32;
            pts.extend(prim.vertices.iter().map(|v| m.transform_point3(glam::Vec3::from(v.pos)).as_dvec3().to_array()));
            tris.extend(prim.indices.chunks_exact(3).map(|t| [base + t[0], base + t[1], base + t[2]]));
        }
    }
    if tris.is_empty() {
        return Err("the mesh has no triangles".into());
    }
    Ok((pts, tris))
}

fn prim_triangles(prim: &sr_3d::Primitive) -> Triangles {
    let pts = prim.vertices.iter().map(|v| v.pos.map(|c| c as f64)).collect();
    let tris = prim.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
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
    let [sx, sy, sz] = s.map(f64::abs);
    let uniform = (sx - sy).abs() <= 1e-9 * sx.max(1.0) && (sx - sz).abs() <= 1e-9 * sx.max(1.0);
    let round_xz = (sx - sz).abs() <= 1e-9 * sx.max(1.0);
    // the object's triangles (object space, unscaled)
    let triangles = |problems: &mut Vec<String>| -> Option<Triangles> {
        let prim = match kind.as_str() {
            "sphere" | "globe" => sr_3d::prim::sphere(r as f32, segs),
            "box" => sr_3d::prim::cuboid(w as f32, hh.unwrap_or(2.0 * r) as f32, depth as f32),
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
            "box" | "plane" => Shape3::Box(box_dims()),
            "sphere" | "globe" if uniform => Shape3::Sphere(r * sx),
            "cylinder" if round_xz => Shape3::Cylinder(tall * 0.5 * sy, r * sx),
            "cone" if round_xz => Shape3::Cone(tall * 0.5 * sy, r * sx),
            "capsule" if round_xz => {
                let total = hh.unwrap_or(4.0 * r).max(2.0 * r);
                Shape3::Capsule((total * 0.5 - r).max(0.0) * sy, r * sx)
            }
            "text" => Shape3::Box(box_dims()),
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
        let shape = if cached { Shape3::Sphere(1.0) } else { shape_for(p, n, c, scale, problems) };
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
        bodies.push(Body3Node { id: n.id.clone(), scale });
    }
    if bodies.is_empty() {
        return None;
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
        return Some(Phys3 { world: None, bodies });
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
    Some(Phys3 { world: Some(World3::new(spec)), bodies })
}

// ------------------------------------------------------------------ stepping

pub(crate) struct Driver<'a, 'b> {
    pub(crate) graphs: &'a mut Graphs<'b>,
    pub(crate) bodies: &'a [Body3Node],
    pub(crate) fields: &'a FieldSrc,
    pub(crate) statics: &'a [Field],
}

impl Driver3 for Driver<'_, '_> {
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

/// Sets each body's pose on its node.
pub(crate) fn apply(g: &mut FrameGraph, bodies: &[Body3Node], poses: &[Pose3]) {
    for (b, pose) in bodies.iter().zip(poses) {
        let Some(i) = index_of(g, &b.id) else { continue };
        g.nodes[i].pose3 = Some(compose(pose, b.scale).to_cols_array());
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
