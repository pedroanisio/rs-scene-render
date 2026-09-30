//! Simulation in the evaluator: physics bodies, soft bodies,
//! force fields and particle emitters of a document, stepped by `sr-sim`
//! from `physics@start` and applied to each FrameGraph.
//!
//! Rigid bodies replace their node's world transform (descendants follow);
//! soft bodies hold their node at its start transform and carry a lattice
//! displacement that the renderer applies as a mesh warp; emitters carry
//! their live particles in frame space. Animated inputs (kinematic bodies,
//! animated fields, moving emitters, animated rates) come from base
//! evaluations at simulation-step times.

use std::collections::HashMap;
use std::sync::Arc;

use sr_model::element::{children, AttrValue, Element};
use sr_model::parse::ParseValue;
use sr_model::values::{Color, Paint};
use sr_sim::fields::{Field, FieldKind};
use sr_sim::particles::{Burst, EmitShape, Emitter, EmitterDriver, EmitterSpec, Walls};
use sr_sim::physics::{BodyKind, BodySpec, Bounds, Driver, JointKind, JointSpec, PxPose, Shape, World, WorldSpec};
use sr_sim::physics3d::Pose3;
use sr_sim::soft::{SoftKind, SoftSpec};

use crate::eval::{Affine, FrameGraph};
use crate::program::Program;
use crate::value::Value;

/// A soft body's lattice displacement in node space (rows × cols control points over `size`).
#[derive(Debug, Clone, PartialEq)]
pub struct SoftWarp {
    pub rows: usize,
    pub cols: usize,
    pub size: [f64; 2],
    pub offsets: Vec<[f64; 2]>,
}

/// Particle drawing style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParticleShape {
    #[default]
    Disc,
    Square,
    Sprite,
    Streak,
}

/// Live particles of one emitter at one frame, ready to draw (frame space).
#[derive(Debug, Clone, Default)]
pub struct ParticleFrame {
    pub pos: Vec<[f32; 2]>,
    pub vel: Vec<[f32; 2]>,
    /// Diameter in pixels.
    pub size: Vec<f32>,
    /// Degrees.
    pub rot: Vec<f32>,
    /// Position between `color0` and `color1`.
    pub color_t: Vec<f32>,
    pub alpha: Vec<f32>,
    /// Sprite-sheet cell.
    pub frame: Vec<u32>,
    pub shape: ParticleShape,
    pub color0: Option<Value>,
    pub color1: Option<Value>,
    /// Asset key of the sprite image.
    pub sprite: Option<Arc<str>>,
    pub cols: u32,
    pub rows: u32,
    /// Streak and trail length in seconds of motion.
    pub trail: f64,
    pub orient: bool,
}

// ------------------------------------------------------------------ attributes

pub(crate) fn num(e: &dyn Element, n: &str, d: f64) -> f64 {
    match e.get_attr(n) {
        Some(AttrValue::Num(v)) => v,
        Some(AttrValue::Length(l)) => l.value,
        Some(AttrValue::Bool(b)) => b as u8 as f64,
        _ => d,
    }
}

pub(crate) fn opt(e: &dyn Element, n: &str) -> Option<f64> {
    match e.get_attr(n) {
        Some(AttrValue::Num(v)) => Some(v),
        Some(AttrValue::Length(l)) => Some(l.value),
        _ => None,
    }
}

pub(crate) fn text(e: &dyn Element, n: &str) -> Option<String> {
    match e.get_attr(n) {
        Some(AttrValue::Str(s)) => Some(s),
        Some(AttrValue::Tokens(t)) => Some(t.join(" ")),
        Some(other) => Some(other.to_string()),
        None => None,
    }
}

fn flag(e: &dyn Element, n: &str, d: bool) -> bool {
    match e.get_attr(n) {
        Some(AttrValue::Bool(b)) => b,
        Some(AttrValue::Str(s)) => s == "true",
        _ => d,
    }
}

fn color_of(c: &Color) -> Value {
    match c {
        Color::Rgba(r) => Value::Color([r.r as f64, r.g as f64, r.b as f64, r.a as f64]),
        Color::Token(t) => Value::Str(format!("token:{t}").into()),
    }
}

pub(crate) fn color_attr(e: &dyn Element, props: &crate::eval::Props, n: &str) -> Option<Value> {
    if let Some(v) = props.get(n) {
        return Some(v.clone());
    }
    match e.get_attr(n)? {
        AttrValue::Color(c) | AttrValue::Paint(Paint::Color(c)) => Some(color_of(&c)),
        AttrValue::Str(s) => Color::parse_value(&s).ok().map(|c| color_of(&c)),
        _ => None,
    }
}

fn prop_num(g: &FrameGraph, key: &str, n: &str) -> Option<f64> {
    g.elements.iter().find(|e| &*e.key == key).and_then(|e| e.props.get(n)).and_then(Value::as_num)
}

fn node_prop(props: &crate::eval::Props, e: &dyn Element, n: &str, d: f64) -> f64 {
    props.get(n).and_then(Value::as_num).unwrap_or_else(|| num(e, n, d))
}

fn pose_of(w: &Affine, center: [f64; 2]) -> PxPose {
    let p = w.apply(center);
    PxPose { x: p[0], y: p[1], angle: w.0[1].atan2(w.0[0]).to_degrees() }
}

fn scales(w: &Affine) -> [f64; 2] {
    let [a, b, c, d, _, _] = w.0;
    let sx = (a * a + b * b).sqrt();
    let det = a * d - b * c;
    [sx, if sx > 0.0 { det / sx } else { (c * c + d * d).sqrt() }]
}

fn path_points(d: &str, tol: f64) -> Vec<Vec<[f64; 2]>> {
    crate::path::flatten(d, tol).unwrap_or_default()
}

/// Evenly spaced samples along polylines.
fn along(polys: &[Vec<[f64; 2]>], n: usize) -> Vec<[f64; 2]> {
    let total: f64 = polys
        .iter()
        .flat_map(|p| p.windows(2))
        .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
        .sum();
    if total <= 0.0 {
        return polys.iter().flatten().copied().take(n).collect();
    }
    let mut out = Vec::with_capacity(n);
    let step = total / n as f64;
    let mut next = step * 0.5;
    let mut acc = 0.0;
    for p in polys {
        for w in p.windows(2) {
            let l = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
            while next <= acc + l && out.len() < n {
                let u = if l > 0.0 { (next - acc) / l } else { 0.0 };
                out.push([w[0][0] + (w[1][0] - w[0][0]) * u, w[0][1] + (w[1][1] - w[0][1]) * u]);
                next += step;
            }
            acc += l;
        }
    }
    out
}

/// Pixels of an image asset whose alpha exceeds ½ (at most `limit`, evenly strided), in image pixels.
/// The local file of image asset `key`.
pub(crate) fn image_path(p: &Program, key: &str) -> Result<std::path::PathBuf, String> {
    let (doc, id) = p.assets.get(key).ok_or_else(|| format!("asset {key} not found"))?;
    let scene = if *doc == 0 { &p.scene } else { &p.includes.get(*doc as usize - 1).ok_or("include missing")?.1 };
    let a = scene
        .assets
        .as_ref()
        .and_then(|a| a.children.iter().find(|c| c.id() == Some(id.as_str())))
        .ok_or_else(|| format!("asset {id} not found"))?;
    let src = match a {
        sr_model::model::AssetsChild::Image(i) => i.src.clone(),
        _ => return Err(format!("asset {id} is not an image")),
    };
    let base = p.base_dirs.get(*doc as usize).cloned().unwrap_or_default();
    match sr_model::assets::resolve(&src, &base) {
        sr_model::assets::Resolved::Local(pth) => Ok(pth),
        sr_model::assets::Resolved::Remote(u) => Err(format!("remote image {u}")),
    }
}

fn alpha_points(p: &Program, key: &str, limit: usize) -> Result<(Vec<[f64; 2]>, [f64; 2]), String> {
    let path = image_path(p, key)?;
    let img = sr_media::still::open(&path)?.image.to_rgba8();
    let (w, h) = img.dimensions();
    let all: Vec<[f64; 2]> = img
        .enumerate_pixels()
        .filter(|(_, _, px)| px[3] > 127)
        .map(|(x, y, _)| [x as f64 + 0.5, y as f64 + 0.5])
        .collect();
    let stride = all.len().div_ceil(limit.max(1)).max(1);
    Ok((all.into_iter().step_by(stride).collect(), [w as f64, h as f64]))
}

/// Convex hull (monotone chain) of points.
fn hull(mut pts: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut lower: Vec<[f64; 2]> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<[f64; 2]> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

// ------------------------------------------------------------------ graphs at step times

pub(crate) struct Graphs<'a> {
    base: &'a dyn Fn(f64) -> FrameGraph,
    cache: Vec<(u64, Arc<FrameGraph>)>,
}

impl Graphs<'_> {
    pub(crate) fn at(&mut self, t: f64) -> Arc<FrameGraph> {
        let key = t.to_bits();
        if let Some((_, g)) = self.cache.iter().find(|(k, _)| *k == key) {
            return g.clone();
        }
        let g = Arc::new((self.base)(t));
        if self.cache.len() >= 4 {
            self.cache.remove(0);
        }
        self.cache.push((key, g.clone()));
        g
    }
}

pub(crate) fn index_of(g: &FrameGraph, id: &str) -> Option<usize> {
    g.nodes.iter().position(|n| &*n.id == id)
}

// ------------------------------------------------------------------ force fields

pub(crate) struct FieldSrc {
    /// (element key, static field in document units) per `<forceField>`.
    fields: Vec<(String, Field, f64, Option<f64>)>,
    pub(crate) animated: bool,
    /// physics@pixelsPerMeter.
    ppm: f64,
}

/// A field in document units (m/s² with +y up, radius in metres; positive radial strength
/// attracts) as the simulation's pixel-space field (px/s², +y down; positive radial repels).
fn to_pixels(mut f: Field, ppm: f64) -> Field {
    f.force = [f.force[0] * ppm, -f.force[1] * ppm];
    // +z toward the camera in the document's physics axes; scene z runs away from it
    f.force_z = -f.force_z * ppm;
    f.radius = f.radius.map(|r| r * ppm);
    f.strength *= match f.kind {
        FieldKind::Radial => -ppm,
        FieldKind::Drag => 1.0,
        _ => ppm,
    };
    f
}

impl FieldSrc {
    pub(crate) fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub(crate) fn at(&self, t: f64, g: Option<&FrameGraph>) -> Vec<Field> {
        self.fields
            .iter()
            .filter(|(_, _, s, e)| t >= *s && e.map(|e| t < e).unwrap_or(true))
            .map(|(key, f, _, _)| {
                let mut f = f.clone();
                if let Some(g) = g {
                    let get = |n: &str| prop_num(g, key, n);
                    f.pos = [get("x").unwrap_or(f.pos[0]), get("y").unwrap_or(f.pos[1])];
                    f.force = [get("forceX").unwrap_or(f.force[0]), get("forceY").unwrap_or(f.force[1])];
                    f.z = get("z").unwrap_or(f.z);
                    f.force_z = get("forceZ").unwrap_or(f.force_z);
                    f.strength = get("strength").unwrap_or(f.strength);
                    f.falloff = get("falloff").unwrap_or(f.falloff);
                    if let Some(r) = get("radius") {
                        f.radius = Some(r);
                    }
                }
                to_pixels(f, self.ppm)
            })
            .collect()
    }

    /// Fields at a simulation step: `statics` when nothing about them changes over time.
    pub(crate) fn at_step(&self, t: f64, graphs: &mut Graphs, statics: &[Field]) -> Vec<Field> {
        if self.animated {
            let g = graphs.at(t);
            self.at(t, Some(&g))
        } else if self.fields.iter().any(|(_, _, s, e)| *s > 0.0 || e.is_some()) {
            self.at(t, None)
        } else {
            statics.to_vec()
        }
    }

    pub(crate) fn named(&self, ids: &[String], t: f64, g: Option<&FrameGraph>) -> Vec<Field> {
        let all = self.at(t, g);
        let keys: Vec<&String> = self
            .fields
            .iter()
            .filter(|(_, _, s, e)| t >= *s && e.map(|e| t < e).unwrap_or(true))
            .map(|(k, ..)| k)
            .collect();
        all.into_iter().zip(keys).filter(|(_, k)| ids.iter().any(|i| i == *k)).map(|(f, _)| f).collect()
    }
}

fn build_fields(p: &Program) -> FieldSrc {
    let mut out = FieldSrc { fields: Vec::new(), animated: false, ppm: 100.0 };
    let Some(ph) = p.scene.physics.as_ref() else { return out };
    out.ppm = ph.pixels_per_meter.get();
    for c in &ph.children {
        let sr_model::model::PhysicsChild::ForceField(f) = c else { continue };
        let e: &dyn Element = f;
        let kind = match text(e, "type").as_deref() {
            Some("radial") => FieldKind::Radial,
            Some("vortex") => FieldKind::Vortex,
            Some("turbulence") => FieldKind::Turbulence,
            Some("drag") => FieldKind::Drag,
            Some("wind") => FieldKind::Wind,
            Some("attractor-path") => FieldKind::AttractorPath,
            _ => FieldKind::Directional,
        };
        let affects = text(e, "affects").unwrap_or_else(|| "all".into());
        let field = Field {
            kind,
            pos: [num(e, "x", 0.0), num(e, "y", 0.0)],
            force: [num(e, "forceX", 0.0), num(e, "forceY", 0.0)],
            strength: num(e, "strength", 0.0),
            falloff: num(e, "falloff", 0.0),
            radius: opt(e, "radius"),
            scale: num(e, "scale", 1.0),
            path: text(e, "path").map(|d| path_points(&d, 0.5).into_iter().flatten().collect()).unwrap_or_default(),
            seed: opt(e, "seed").map(|s| s as u64).unwrap_or_else(|| crate::rng::hash_str(&f.id)),
            bodies: affects != "particles",
            particles: affects != "bodies",
            z: num(e, "z", 0.0),
            force_z: num(e, "forceZ", 0.0),
        };
        if !f.children.is_empty() {
            out.animated = true;
        }
        out.fields.push((f.id.clone(), field, num(e, "start", 0.0), opt(e, "end")));
    }
    out
}

// ------------------------------------------------------------------ physics

struct BodyNode {
    id: Arc<str>,
    center: [f64; 2],
    scale: [f64; 2],
}

struct SoftNode {
    id: Arc<str>,
    rows: usize,
    cols: usize,
    size: [f64; 2],
    start_world: Affine,
    rest_local: Vec<[f64; 2]>,
}

/// Precomputed body poses and soft lattices per step, from `physics@cache`.
struct Cached {
    step: f64,
    start: f64,
    bodies: usize,
    soft_points: Vec<usize>,
    bodies3: usize,
    frames: Vec<Vec<f64>>,
}

impl Cached {
    fn row(&self, t: f64) -> &[f64] {
        let k = if t <= self.start { 0 } else { (((t - self.start) / self.step) + 1e-9).floor() as usize };
        &self.frames[k.min(self.frames.len().saturating_sub(1))]
    }

    /// 3D body poses: position then quaternion, after the 2D bodies and soft lattices.
    fn frame3(&self, t: f64) -> Vec<Pose3> {
        let f = self.row(t);
        let o = self.bodies * 3 + self.soft_points.iter().sum::<usize>() * 2;
        (0..self.bodies3)
            .map(|k| {
                let v = &f[o + 7 * k..o + 7 * k + 7];
                Pose3 { pos: [v[0], v[1], v[2]], rot: [v[3], v[4], v[5], v[6]] }
            })
            .collect()
    }

    fn frame(&self, t: f64) -> sr_sim::physics::Frame {
        let f = self.row(t);
        let mut out = sr_sim::physics::Frame::default();
        let mut i = 0;
        for _ in 0..self.bodies {
            out.bodies.push(PxPose { x: f[i], y: f[i + 1], angle: f[i + 2] });
            i += 3;
        }
        for &n in &self.soft_points {
            out.softs.push((0..n).map(|k| [f[i + 2 * k], f[i + 2 * k + 1]]).collect());
            i += 2 * n;
        }
        out
    }
}

struct PhysicsRt {
    /// The 2D world (none without 2D bodies, or when cached).
    world: Option<World>,
    three: Option<crate::sim3d::Phys3>,
    cached: Option<Cached>,
    start: f64,
    step: f64,
    bodies: Vec<BodyNode>,
    softs: Vec<SoftNode>,
}

struct PDriver<'a, 'b> {
    graphs: &'a mut Graphs<'b>,
    bodies: &'a [BodyNode],
    fields: &'a FieldSrc,
    statics: Vec<Field>,
}

impl Driver for PDriver<'_, '_> {
    fn kinematic(&mut self, t: f64, which: &[usize]) -> Vec<PxPose> {
        let g = self.graphs.at(t);
        which
            .iter()
            .map(|&k| {
                let b = &self.bodies[k];
                index_of(&g, &b.id).map(|i| pose_of(&g.nodes[i].world, b.center)).unwrap_or_default()
            })
            .collect()
    }
    fn fields(&mut self, t: f64) -> Vec<Field> {
        self.fields.at_step(t, self.graphs, &self.statics)
    }
}

fn shape_for(
    p: &Program,
    n: &crate::eval::FrameNode,
    b: &dyn Element,
    size: [f64; 2],
    scale: [f64; 2],
    problems: &mut Vec<String>,
) -> Shape {
    let (w, h) = (size[0] * scale[0].abs(), size[1] * scale[1].abs());
    let centre = [size[0] * 0.5, size[1] * 0.5];
    let local = |q: [f64; 2]| [(q[0] - centre[0]) * scale[0].abs(), (q[1] - centre[1]) * scale[1].abs()];
    match text(b, "shape").as_deref().unwrap_or("box") {
        "circle" => Shape::Circle {
            r: opt(b, "radius").filter(|r| *r > 0.0).map(|r| r * scale[0].abs()).unwrap_or(w.min(h) * 0.5),
        },
        "capsule" => Shape::Capsule { w, h },
        "polygon" | "path" => match text(b, "path") {
            Some(d) => {
                let polys = path_points(&d, 0.5);
                let pts: Vec<[f64; 2]> = polys.into_iter().flatten().map(local).collect();
                if text(b, "shape").as_deref() == Some("polygon") {
                    Shape::Convex(pts)
                } else {
                    Shape::Outline(pts)
                }
            }
            None => {
                problems.push(format!("{}: rigidBody shape needs @path", n.id));
                Shape::Box { w, h }
            }
        },
        "convex-hull" => match n.asset.as_deref().map(|k| alpha_points(p, k, 20_000)) {
            Some(Ok((pts, _))) if pts.len() >= 3 => Shape::Convex(hull(pts).into_iter().map(local).collect()),
            Some(Err(e)) => {
                problems.push(format!("{}: convex hull: {e}", n.id));
                Shape::Box { w, h }
            }
            _ => Shape::Box { w, h },
        },
        _ => Shape::Box { w, h },
    }
}

fn build_physics(p: &Program, g0: &FrameGraph, fields: &FieldSrc, problems: &mut Vec<String>) -> Option<PhysicsRt> {
    let ph = p.scene.physics.as_ref();
    let mut bodies = Vec::new();
    let mut specs = Vec::new();
    let mut softs = Vec::new();
    let mut soft_specs = Vec::new();
    let step = ph.map(|p| p.fixed_step.get()).unwrap_or(1.0 / 120.0);
    let start = ph.map(|p| p.start).unwrap_or(0.0);
    for n in &g0.nodes {
        for c in children(&*n.elem) {
            match c.element_name() {
                "rigidBody" => {
                    let size = n.size.unwrap_or([0.0, 0.0]);
                    let centre = [size[0] * 0.5, size[1] * 0.5];
                    let sc = scales(&n.world);
                    let kind = match text(c, "type").as_deref() {
                        Some("static") => BodyKind::Static,
                        Some("kinematic") => BodyKind::Kinematic,
                        _ => BodyKind::Dynamic,
                    };
                    let collides_with = match text(c, "collidesWith").as_deref() {
                        None | Some("all") => None,
                        Some(list) => {
                            Some(list.split([' ', ',']).filter_map(|t| t.trim().parse::<u32>().ok()).collect())
                        }
                    };
                    specs.push(BodySpec {
                        kind,
                        shape: shape_for(p, n, c, size, sc, problems),
                        mass: num(c, "mass", 1.0),
                        friction: num(c, "friction", 0.5),
                        restitution: num(c, "restitution", 0.0),
                        linear_damping: num(c, "linearDamping", 0.01),
                        angular_damping: num(c, "angularDamping", 0.01),
                        velocity: [num(c, "velocityX", 0.0), num(c, "velocityY", 0.0)],
                        angular_velocity: num(c, "angularVelocity", 0.0),
                        group: num(c, "collisionGroup", 0.0) as u32,
                        collides_with,
                        sensor: flag(c, "sensor", false),
                        fixed_rotation: flag(c, "fixedRotation", false),
                        bullet: flag(c, "bullet", false),
                        activate_at: num(c, "activateAt", 0.0),
                        start: pose_of(&n.world, centre),
                    });
                    bodies.push(BodyNode { id: n.id.clone(), center: centre, scale: sc });
                }
                "softBody" => {
                    let size = n.size.unwrap_or([100.0, 100.0]);
                    let kind = match text(c, "kind").as_deref() {
                        Some("cloth") => SoftKind::Cloth,
                        Some("rope") => SoftKind::Rope,
                        _ => SoftKind::Jelly,
                    };
                    let (rows, cols) =
                        (num(c, "rows", 4.0).clamp(2.0, 16.0) as usize, num(c, "cols", 4.0).clamp(2.0, 16.0) as usize);
                    // jelly and cloth: a lattice of (rows + 1) × (cols + 1) points;
                    // a rope is one chain of `cols` points
                    let (rows, cols) = if kind == SoftKind::Rope { (rows, cols) } else { (rows + 1, cols + 1) };
                    let mut rest_local = Vec::with_capacity(rows * cols);
                    for i in 0..rows {
                        for j in 0..cols {
                            rest_local
                                .push([size[0] * j as f64 / (cols - 1) as f64, size[1] * i as f64 / (rows - 1) as f64]);
                        }
                    }
                    let pin = text(c, "pin").unwrap_or_else(|| "none".into());
                    let pinned: Vec<bool> = (0..rows * cols)
                        .map(|k| {
                            let (i, j) = (k / cols, k % cols);
                            match pin.as_str() {
                                "top" => i == 0,
                                "bottom" => i == rows - 1,
                                "left" => j == 0,
                                "right" => j == cols - 1,
                                "corners" => (i == 0 || i == rows - 1) && (j == 0 || j == cols - 1),
                                _ => false,
                            }
                        })
                        .collect();
                    soft_specs.push(SoftSpec {
                        kind,
                        rows,
                        cols,
                        rest: rest_local.iter().map(|q| n.world.apply(*q)).collect(),
                        mass: num(c, "mass", 1.0),
                        stiffness: num(c, "stiffness", 20.0),
                        damping: num(c, "damping", 0.1),
                        pressure: num(c, "pressure", 0.0),
                        pinned,
                        self_collision: flag(c, "selfCollision", false),
                    });
                    softs.push(SoftNode { id: n.id.clone(), rows, cols, size, start_world: n.world, rest_local });
                }
                _ => {}
            }
        }
    }
    let ids3 = crate::sim3d::body_ids(g0);
    if bodies.is_empty() && softs.is_empty() && ids3.is_empty() {
        return None;
    }
    let mut joints = Vec::new();
    if let Some(ph) = ph {
        for c in &ph.children {
            let sr_model::model::PhysicsChild::Constraint(k) = c else { continue };
            if ids3.iter().any(|i| **i == *k.a) {
                // a 3D joint
                continue;
            }
            let find = |id: &str| bodies.iter().position(|b| &*b.id == id);
            let Some(a) = find(&k.a) else {
                problems.push(format!("{}: constraint body {} has no rigidBody", k.id, k.a));
                continue;
            };
            let b = match &k.b {
                Some(id) => match find(id) {
                    Some(i) => Some(i),
                    None => {
                        problems.push(format!("{}: constraint body {id} has no rigidBody", k.id));
                        continue;
                    }
                },
                None => None,
            };
            let e: &dyn Element = k;
            let kind = match text(e, "type").as_deref() {
                Some("spring") => JointKind::Spring,
                Some("distance") => JointKind::Distance,
                Some("pin") => JointKind::Pin,
                Some("rope") => JointKind::Rope,
                Some("hinge") => JointKind::Hinge,
                Some("slider") => JointKind::Slider,
                Some("weld") => JointKind::Weld,
                _ => JointKind::Motor,
            };
            joints.push(JointSpec {
                kind,
                a,
                b,
                anchor: match (k.x, k.y) {
                    (Some(x), Some(y)) => Some([x, y]),
                    _ => None,
                },
                rest_length: k.rest_length,
                stiffness: k.stiffness,
                damping: k.damping,
                min_angle: k.min_angle,
                max_angle: k.max_angle,
                axis_angle: k.axis_angle,
                motor_speed: k.motor_speed,
                max_force: k.max_force.map(|v| v.get()),
                break_force: k.break_force.map(|v| v.get()),
            });
        }
    }
    let _ = fields;
    let [fw, fh] = p.size;
    let bounds = match ph.map(|p| p.bounds.to_string()).as_deref() {
        Some("frame") => Bounds::Frame { w: fw, h: fh },
        Some("floor") => Bounds::Floor { w: fw, h: fh },
        _ => Bounds::None,
    };
    let spec = WorldSpec {
        start,
        step,
        gravity: [ph.map(|p| p.gravity_x).unwrap_or(0.0), ph.map(|p| p.gravity_y).unwrap_or(-9.80665)],
        pixels_per_meter: ph.map(|p| p.pixels_per_meter.get()).unwrap_or(100.0),
        iterations: ph.map(|p| p.solver_iterations as usize).unwrap_or(8),
        bounds,
        bodies: specs,
        joints,
        softs: soft_specs,
    };
    // a verified cache replaces the simulation
    let mut cached = None;
    if let Some(uri) = ph.and_then(|p| p.cache.as_deref()) {
        let base = p.base_dirs.first().cloned().unwrap_or_default();
        if let sr_model::assets::Resolved::Local(path) = sr_model::assets::resolve(uri, &base) {
            if path.is_file() {
                match read_cache(&path, ph.and_then(|p| p.cache_sha256.as_ref()).map(|s| s.to_string())) {
                    Ok(c)
                        if c.bodies == bodies.len()
                            && c.bodies3 == ids3.len()
                            && c.soft_points == softs.iter().map(|s| s.rows * s.cols).collect::<Vec<_>>() =>
                    {
                        cached = Some(c)
                    }
                    Ok(_) => problems.push(format!(
                        "physics cache {}: does not match the document's bodies; simulating",
                        path.display()
                    )),
                    Err(e) => problems.push(format!("physics cache {}: {e}; simulating", path.display())),
                }
            }
        }
    }
    let has2 = !spec.bodies.is_empty() || !spec.softs.is_empty();
    let world = if cached.is_some() || !has2 { None } else { Some(World::new(spec)) };
    let three = crate::sim3d::build(p, g0, cached.is_some(), problems);
    Some(PhysicsRt { world, three, cached, start, step, bodies, softs })
}

/// Cache files: version 2 adds 3D bodies (7 numbers each per frame); version 1 is still read.
const CACHE_MAGIC: &[u8; 8] = b"SRPHYS02";
const CACHE_MAGIC_V1: &[u8; 8] = b"SRPHYS01";

fn read_cache(path: &std::path::Path, sha: Option<String>) -> Result<Cached, String> {
    use sha2::Digest;
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    if let Some(want) = sha {
        let got: String = sha2::Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
        if !got.eq_ignore_ascii_case(want.trim()) {
            return Err(format!("SHA-256 {got} does not match cacheSha256 {want}"));
        }
    }
    let v2 = data.len() >= 8 && &data[..8] == CACHE_MAGIC;
    if data.len() < 40 || !(v2 || &data[..8] == CACHE_MAGIC_V1) {
        return Err("not a scene-render physics cache".into());
    }
    let u64_at = |o: usize| u64::from_le_bytes(data[o..o + 8].try_into().unwrap());
    let f64_at = |o: usize| f64::from_le_bytes(data[o..o + 8].try_into().unwrap());
    let (step, start, bodies, nsoft, nframes) =
        (f64_at(8), f64_at(16), u64_at(24) as usize, u64_at(32) as usize, 0usize);
    let _ = nframes;
    let mut o = 40;
    let mut soft_points = Vec::new();
    for _ in 0..nsoft {
        soft_points.push(u64_at(o) as usize);
        o += 8;
    }
    if data.len() < o + 16 {
        return Err("truncated".into());
    }
    let bodies3 = if v2 {
        o += 8;
        u64_at(o - 8) as usize
    } else {
        0
    };
    let frames_n = u64_at(o) as usize;
    o += 8;
    let per = bodies * 3 + soft_points.iter().sum::<usize>() * 2 + bodies3 * 7;
    if data.len() < o + frames_n * per * 8 {
        return Err("truncated".into());
    }
    let frames = (0..frames_n).map(|k| (0..per).map(|i| f64_at(o + (k * per + i) * 8)).collect()).collect();
    Ok(Cached { step, start, bodies, soft_points, bodies3, frames })
}

// ------------------------------------------------------------------ particles

struct EmitterRt {
    emitter: Emitter,
    node: u32,
    /// Replay source history when its composition-time mapping depends on the
    /// requested frame (loops, remaps or a held clock).
    replay_source: bool,
    size: f64,
    size_end: f64,
    size_curve: crate::curve::Ease,
    color_curve: crate::curve::Ease,
    opacity_end: f64,
    color0: Option<Value>,
    color1: Option<Value>,
    random_color: bool,
    shape: ParticleShape,
    sprite: Option<Arc<str>>,
    cols: u32,
    rows: u32,
    sprite_fps: f64,
    trail: f64,
    orient: bool,
    fields: Option<Vec<String>>,
    /// The emitter's program node when its world transform and rate never change (see
    /// `time_invariant`), and those values once read: stepping then needs no frame evaluation.
    invariant: Invariant,
}

/// An emitter's program node when its world transform and rate never change, and those values (world
/// transform, rate) once read.
type Invariant = Option<(u32, Option<([f64; 6], f64)>)>;

/// The force fields that act on a particle emitter, flock or fluid: `None` for every field of the scene
/// (no `forceFields`), else those it lists; none at all when `useForceFields` is false.
pub(crate) fn field_names(e: &dyn Element) -> Option<Vec<String>> {
    if matches!(text(e, "useForceFields").as_deref(), Some("false" | "0")) {
        return Some(Vec::new());
    }
    text(e, "forceFields").map(|s| s.split_whitespace().map(str::to_string).collect())
}

/// Whether node `n` has the same world transform and properties at every time it is drawn: neither it
/// nor any ancestor is animated, computed by an expression, linked, conditional, laid out, constrained,
/// posed by a skeleton or on a motion path.
fn time_invariant(p: &Program, n: u32) -> bool {
    let mut c = Some(n);
    while let Some(k) = c {
        let node = &p.nodes[k as usize];
        let moving_slot = node.slots.iter().any(|&s| {
            let sl = &p.slots[s as usize];
            !sl.channels.is_empty() || sl.expr.is_some() || sl.link.is_some() || sl.time_node.is_some()
        });
        let constrained = children(&*node.elem)
            .iter()
            .any(|c| matches!(c.element_name(), "transformConstraint" | "rigidBody" | "softBody"));
        if moving_slot
            || constrained
            || node.motion.is_some()
            || node.parent_link.is_some()
            || node.cond.is_some()
            || node.layout.is_some()
            || node.align.is_some()
            || node.fit.is_some()
            || matches!(node.name, "skeleton" | "bone")
        {
            return false;
        }
        c = node.parent;
    }
    true
}

/// Preset values for attributes left at their schema defaults.
fn preset(name: &str) -> &'static [(&'static str, &'static str)] {
    match name {
        "smoke" => &[
            ("rate", "20"),
            ("lifetime", "4"),
            ("lifetimeVariance", "1"),
            ("speed", "40"),
            ("speedVariance", "15"),
            ("spread", "30"),
            ("gravityY", "-10"),
            ("drag", "0.3"),
            ("turbulence", "30"),
            ("size", "40"),
            ("sizeEnd", "140"),
            ("color", "#9A9A9A80"),
            ("opacityEnd", "0"),
        ],
        "sparks" => &[
            ("rate", "80"),
            ("lifetime", "0.8"),
            ("lifetimeVariance", "0.3"),
            ("speed", "400"),
            ("speedVariance", "150"),
            ("spread", "60"),
            ("gravityY", "600"),
            ("drag", "0.5"),
            ("size", "3"),
            ("sizeEnd", "1"),
            ("color", "#FFD27A"),
            ("colorEnd", "#FF4000"),
            ("opacityEnd", "0"),
            ("shape", "streak"),
            ("trail", "0.04"),
        ],
        "dust" => &[
            ("rate", "15"),
            ("lifetime", "6"),
            ("speed", "10"),
            ("speedVariance", "8"),
            ("spread", "360"),
            ("turbulence", "10"),
            ("size", "2"),
            ("sizeVariance", "0.5"),
            ("color", "#E8E0D0"),
            ("opacityEnd", "0"),
        ],
        "rain" => &[
            ("rate", "400"),
            ("lifetime", "1.2"),
            ("speed", "900"),
            ("speedVariance", "100"),
            ("direction", "100"),
            ("spread", "4"),
            ("gravityY", "800"),
            ("size", "2"),
            ("color", "#A0C0FFC0"),
            ("shape", "streak"),
            ("trail", "0.03"),
        ],
        "snow" => &[
            ("rate", "60"),
            ("lifetime", "8"),
            ("speed", "40"),
            ("speedVariance", "20"),
            ("direction", "90"),
            ("spread", "40"),
            ("turbulence", "20"),
            ("size", "4"),
            ("sizeVariance", "0.5"),
        ],
        "confetti" => &[
            ("rate", "120"),
            ("lifetime", "4"),
            ("speed", "500"),
            ("speedVariance", "200"),
            ("spread", "70"),
            ("gravityY", "500"),
            ("drag", "1"),
            ("size", "8"),
            ("shape", "square"),
            ("rotationVariance", "180"),
            ("angularVelocity", "360"),
            ("angularVelocityVariance", "360"),
            ("color", "#FF3366"),
            ("colorEnd", "#33CCFF"),
        ],
        "fire" => &[
            ("rate", "120"),
            ("lifetime", "1"),
            ("lifetimeVariance", "0.3"),
            ("speed", "120"),
            ("speedVariance", "40"),
            ("spread", "25"),
            ("gravityY", "-150"),
            ("turbulence", "40"),
            ("size", "24"),
            ("sizeEnd", "4"),
            ("color", "#FFB030"),
            ("colorEnd", "#FF2000"),
            ("opacityEnd", "0"),
        ],
        "bubbles" => &[
            ("rate", "10"),
            ("lifetime", "5"),
            ("speed", "60"),
            ("speedVariance", "20"),
            ("spread", "20"),
            ("turbulence", "15"),
            ("size", "12"),
            ("sizeVariance", "0.5"),
            ("color", "#BFE8FF80"),
        ],
        "bokeh" => &[
            ("rate", "5"),
            ("lifetime", "6"),
            ("speed", "10"),
            ("spread", "360"),
            ("size", "40"),
            ("sizeVariance", "0.6"),
            ("color", "#FFE8B060"),
            ("opacityEnd", "0"),
        ],
        "glitter" => &[
            ("rate", "60"),
            ("lifetime", "1.5"),
            ("speed", "30"),
            ("spread", "360"),
            ("size", "3"),
            ("sizeVariance", "0.5"),
            ("opacityEnd", "0"),
            ("angularVelocity", "720"),
            ("shape", "square"),
        ],
        _ => &[],
    }
}

/// Schema defaults of the attributes presets may fill.
fn schema_default(name: &str) -> Option<&'static str> {
    Some(match name {
        "rate" => "10",
        "lifetime" => "1",
        "lifetimeVariance"
        | "spread"
        | "gravityX"
        | "gravityY"
        | "drag"
        | "turbulence"
        | "sizeVariance"
        | "rotationVariance"
        | "angularVelocity"
        | "angularVelocityVariance"
        | "trail" => "0",
        "speed" => "100",
        "direction" => "-90",
        "size" => "4",
        "color" => "#FFFFFFFF",
        "shape" => "disc",
        "speedVariance" | "sizeEnd" | "colorEnd" | "opacityEnd" => "",
        _ => return None,
    })
}

struct Attr<'a> {
    e: &'a dyn Element,
    preset: &'static [(&'static str, &'static str)],
}

impl Attr<'_> {
    /// The preset's value when the document leaves `name` at its default.
    fn preset_of(&self, name: &str) -> Option<&'static str> {
        let (_, v) = self.preset.iter().find(|(n, _)| *n == name)?;
        let def = schema_default(name)?;
        let at_default = match self.e.get_attr(name) {
            None => true,
            Some(AttrValue::Num(x)) => def.parse::<f64>().map(|d| (d - x).abs() < 1e-12).unwrap_or(false),
            Some(AttrValue::Str(s)) => s == def,
            Some(AttrValue::Color(c)) | Some(AttrValue::Paint(Paint::Color(c))) => {
                matches!(c, Color::Rgba(r) if r.r >= 1.0 && r.g >= 1.0 && r.b >= 1.0 && r.a >= 1.0)
                    && def == "#FFFFFFFF"
            }
            _ => false,
        };
        at_default.then_some(*v)
    }
    fn num(&self, name: &str, d: f64) -> f64 {
        match self.preset_of(name) {
            Some(v) => v.parse().unwrap_or(d),
            None => num(self.e, name, d),
        }
    }
    fn opt(&self, name: &str) -> Option<f64> {
        match self.preset_of(name) {
            Some(v) => v.parse().ok(),
            None => opt(self.e, name),
        }
    }
    fn text(&self, name: &str) -> Option<String> {
        self.preset_of(name).map(str::to_string).or_else(|| text(self.e, name))
    }
    fn color(&self, props: &crate::eval::Props, name: &str) -> Option<Value> {
        if props.get(name).is_none() {
            if let Some(v) = self.preset_of(name) {
                return Color::parse_value(v).ok().map(|c| color_of(&c));
            }
        }
        color_attr(self.e, props, name)
    }
}

fn curve_of(e: &dyn Element, name: &str) -> crate::curve::Ease {
    let c = text(e, name)
        .and_then(|s| sr_model::model::Curve::parse_value(&s).ok())
        .unwrap_or(sr_model::model::Curve::Linear);
    crate::curve::resolve(c, &crate::curve::KeyParams::default())
}

fn build_emitter(p: &Program, n: &crate::eval::FrameNode, problems: &mut Vec<String>) -> EmitterRt {
    let e: &dyn Element = &*n.elem;
    let a = Attr { e, preset: text(e, "preset").map(|s| preset(&s)).unwrap_or(&[]) };
    let step = p.scene.physics.as_ref().map(|p| p.fixed_step.get()).unwrap_or(1.0 / 120.0);
    let node = p.nodes.iter().position(|pn| pn.id == n.id).expect("compiled emitter");
    let replay_source = !linear_emitter_clock(p, node as u32);
    let start = p.nodes[node].start;
    let end = p.nodes[node].end;
    let shape = match text(e, "emitterShape").as_deref() {
        Some("point") => EmitShape::Point,
        Some("ellipse") => EmitShape::Ellipse { w: num(e, "emitterWidth", 0.0), h: num(e, "emitterHeight", 0.0) },
        Some("line") => EmitShape::Line { w: num(e, "emitterWidth", 0.0) },
        Some("path") => {
            EmitShape::Points(text(e, "emitterPath").map(|d| along(&path_points(&d, 0.5), 1024)).unwrap_or_default())
        }
        Some("asset-alpha") => {
            let key = text(e, "emitterAsset").map(|id| {
                p.assets.keys().find(|k| k.rsplit('/').next() == Some(id.as_str())).map(|k| k.to_string()).unwrap_or(id)
            });
            match key.map(|k| alpha_points(p, &k, 4096)) {
                Some(Ok((pts, _))) => EmitShape::Points(pts),
                Some(Err(err)) => {
                    problems.push(format!("{}: emitterAsset: {err}", n.id));
                    EmitShape::Point
                }
                None => EmitShape::Point,
            }
        }
        _ => EmitShape::Rect { w: num(e, "emitterWidth", 0.0), h: num(e, "emitterHeight", 0.0) },
    };
    let bursts = children(e)
        .iter()
        .filter(|c| c.element_name() == "burst")
        .map(|c| Burst {
            time: start + num(*c, "time", 0.0),
            count: num(*c, "count", 1.0) as u64,
            repeat: num(*c, "repeat", 0.0) as u64,
            interval: num(*c, "interval", 1.0),
        })
        .collect();
    let bounds = p.scene.physics.as_ref().map(|p| p.bounds.to_string());
    let collide = flag(e, "collide", false);
    let walls = match (collide, bounds.as_deref()) {
        (true, Some("floor")) => Walls::Floor { h: p.size[1] },
        (true, Some("frame")) => Walls::Frame { w: p.size[0], h: p.size[1] },
        _ => Walls::None,
    };
    let seed = opt(e, "seed").map(|s| s as u64).unwrap_or_else(|| crate::rng::hash_str(&n.id) ^ p.seed);
    let size = a.num("size", 4.0);
    let spec = EmitterSpec {
        seed,
        start,
        end,
        preroll: num(e, "preroll", 0.0),
        step,
        lifetime: a.num("lifetime", 1.0),
        lifetime_variance: a.num("lifetimeVariance", 0.0),
        speed: a.num("speed", 100.0),
        speed_variance: a.opt("speedVariance").unwrap_or(0.0),
        direction: a.num("direction", -90.0),
        spread: a.num("spread", 0.0),
        gravity: [a.num("gravityX", 0.0), a.num("gravityY", 0.0)],
        drag: a.num("drag", 0.0),
        turbulence: a.num("turbulence", 0.0),
        turbulence_scale: num(e, "turbulenceScale", 100.0),
        rotation0: num(e, "rotation0", 0.0),
        rotation_variance: a.num("rotationVariance", 0.0),
        angular_velocity: a.num("angularVelocity", 0.0),
        angular_velocity_variance: a.num("angularVelocityVariance", 0.0),
        size_variance: a.num("sizeVariance", 0.0),
        max_particles: num(e, "maxParticles", 10_000.0).clamp(1.0, 10_000_000.0) as usize,
        shape,
        bursts,
        collide,
        bounce: num(e, "bounce", 0.3),
        walls,
    };
    let props = &n.props;
    let pshape = match a.text("shape").as_deref() {
        Some("square") => ParticleShape::Square,
        Some("sprite") => ParticleShape::Sprite,
        Some("streak") => ParticleShape::Streak,
        _ => ParticleShape::Disc,
    };
    let sprite = text(e, "sprite").map(|id| {
        let key: Arc<str> = p
            .assets
            .keys()
            .find(|k| k.rsplit('/').next() == Some(id.as_str()))
            .cloned()
            .unwrap_or_else(|| id.as_str().into());
        key
    });
    if pshape == ParticleShape::Sprite && sprite.is_none() {
        problems.push(format!("{}: shape=\"sprite\" without @sprite", n.id));
    }
    EmitterRt {
        emitter: Emitter::new(spec),
        node: node as u32,
        replay_source,
        size,
        size_end: a.opt("sizeEnd").unwrap_or(size),
        size_curve: curve_of(e, "sizeCurve"),
        color_curve: curve_of(e, "colorCurve"),
        opacity_end: a.opt("opacityEnd").unwrap_or(1.0),
        color0: a.color(props, "color"),
        color1: a.color(props, "colorEnd"),
        random_color: text(e, "preset").as_deref() == Some("confetti"),
        shape: pshape,
        sprite,
        cols: num(e, "spriteCols", 1.0).max(1.0) as u32,
        rows: num(e, "spriteRows", 1.0).max(1.0) as u32,
        sprite_fps: num(e, "spriteFps", 0.0),
        trail: a.num("trail", 0.0),
        orient: flag(e, "orientToVelocity", false),
        fields: field_names(e),
        invariant: p
            .nodes
            .iter()
            .position(|pn| pn.id == n.id)
            .map(|k| k as u32)
            .filter(|&k| time_invariant(p, k))
            .map(|k| (k, None)),
    }
}

fn linear_emitter_clock(p: &Program, n: u32) -> bool {
    use crate::program::Clock;
    let Some(parent) = p.nodes[n as usize].parent else { return true };
    if !linear_emitter_clock(p, parent) {
        return false;
    }
    match &p.nodes[parent as usize].clock {
        Clock::Same => true,
        Clock::Affine { scale, .. } => *scale != 0.0,
        Clock::Media(m) => m.remap.is_none() && m.freeze_at.is_none() && m.loops == 0 && m.rate != 0.0,
    }
}

/// Map source history back through every ancestor. A looping/remapped source
/// uses the branch nearest this output frame; overridden child clocks also let
/// us simulate source frames skipped by jumps, trims and freezes.
fn source_sample(p: &Program, node: u32, mut time: f64, at: f64) -> (f64, Vec<(u32, f64)>) {
    use crate::program::Clock;
    let mut clocks = Vec::new();
    let mut parent = p.nodes[node as usize].parent;
    while let Some(k) = parent {
        let n = &p.nodes[k as usize];
        let near = p.timeline_at(k, at);
        clocks.push((k, time));
        time = match &n.clock {
            Clock::Same => time,
            Clock::Affine { origin, offset, scale } if *scale != 0.0 => origin + offset + (time - origin) / scale,
            Clock::Affine { .. } => near,
            Clock::Media(m) => {
                let local = m.freeze_at.unwrap_or(near - m.start);
                if let Some(remap) = &m.remap {
                    m.start + remap.time_for_value(time, local)
                } else if m.rate != 0.0 {
                    let mut source = time - m.clip_in;
                    if m.reverse {
                        source = m.len.unwrap_or(0.0) - source;
                    }
                    if let Some(len) = m.len.filter(|len| *len > 0.0 && m.loops > 0) {
                        let cycle = (local * m.rate / len).floor().clamp(0.0, m.loops as f64);
                        source += cycle * len;
                    }
                    m.start + source / m.rate
                } else {
                    near
                }
            }
        };
        parent = n.parent;
    }
    (time, clocks)
}

struct EDriver<'a, 'b> {
    graphs: &'a mut Graphs<'b>,
    id: Arc<str>,
    fields: &'a FieldSrc,
    names: Option<&'a [String]>,
    physics: Option<&'a mut PhysicsRt>,
    p: &'a Program,
    invariant: &'a mut Invariant,
    /// The emitter's start: during its preroll it emits as it does at its start.
    start: f64,
    node: u32,
    at: f64,
}

impl EDriver<'_, '_> {
    fn sample_time(&self, t: f64) -> (f64, Vec<(u32, f64)>) {
        source_sample(self.p, self.node, t, self.at)
    }
    /// The emitter's world transform and rate at `t`: for an emitter that never changes, read once and then
    /// only checked against its time window, so a step does not evaluate the whole scene.
    fn state(&mut self, t: f64) -> ([f64; 6], f64) {
        const ABSENT: ([f64; 6], f64) = ([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 0.0);
        // the preroll runs before the emitter's start, where it is not yet in the frame: it emits as at its start
        let (t, clocks) = self.sample_time(t.max(self.start));
        if let Some((n, known)) = *self.invariant {
            let node = &self.p.nodes[n as usize];
            let source = clocks.first().map(|(_, t)| *t).unwrap_or(t);
            let present = if clocks.is_empty() {
                crate::eval::in_window(self.p, n, t)
            } else {
                source >= node.vis_start && node.vis_end.is_none_or(|end| source < end)
            };
            if !present {
                return ABSENT;
            }
            if let Some(v) = known {
                return v;
            }
        }
        let mut g = if clocks.is_empty() {
            self.graphs.at(t)
        } else {
            Arc::new(crate::eval::evaluate_with_clocks(self.p, t, &clocks))
        };
        if let Some(ph) = self.physics.as_deref_mut() {
            apply_physics(ph, Arc::make_mut(&mut g), self.graphs, self.fields, t);
        }
        let v = match index_of(&g, &self.id) {
            Some(i) => (g.nodes[i].world.0, node_prop(&g.nodes[i].props, &*g.nodes[i].elem, "rate", 10.0)),
            None => ABSENT,
        };
        if let Some((_, known)) = self.invariant.as_mut() {
            if index_of(&g, &self.id).is_some() {
                *known = Some(v);
            }
        }
        v
    }
}

impl EmitterDriver for EDriver<'_, '_> {
    fn origin(&mut self, t: f64) -> [f64; 6] {
        self.state(t).0
    }
    fn rate(&mut self, t: f64) -> f64 {
        self.state(t).1
    }
    fn fields(&mut self, t: f64) -> Vec<Field> {
        let t = self.sample_time(t).0;
        let g = if self.fields.animated { Some(self.graphs.at(t)) } else { None };
        match self.names {
            Some(ids) => self.fields.named(ids, t, g.as_deref()),
            None => self.fields.at(t, g.as_deref()),
        }
    }
    fn hit(&mut self, p: [f64; 2]) -> Option<([f64; 2], [f64; 2])> {
        self.physics.as_deref().and_then(|ph| ph.world.as_ref()).and_then(|w| w.hit(p))
    }
}

// ------------------------------------------------------------------ runtime

/// Simulation state of one evaluator.
#[derive(Default)]
pub struct Runtime {
    built: bool,
    fields: Option<FieldSrc>,
    physics: Option<PhysicsRt>,
    emitters: HashMap<Arc<str>, EmitterRt>,
    agents: crate::agents::Sims,
    /// Problems found while building (reported once).
    pub problems: Vec<String>,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime").field("built", &self.built).field("emitters", &self.emitters.len()).finish()
    }
}

/// Whether the program needs simulation at all.
pub fn needed(p: &Program) -> bool {
    p.scene.physics.is_some()
        || p.nodes.iter().any(|n| {
            n.name == "particleEmitter"
                || crate::agents::is_sim(n.name)
                || children(&*n.elem).iter().any(|c| matches!(c.element_name(), "rigidBody" | "softBody"))
        })
}

fn apply_physics(ph: &mut PhysicsRt, g: &mut FrameGraph, graphs: &mut Graphs<'_>, fields: &FieldSrc, t: f64) {
    if t >= ph.start {
        let (frame, frame3) = if let Some(c) = &ph.cached {
            (c.frame(t), c.frame3(t))
        } else {
            let statics = if fields.animated { Vec::new() } else { fields.at(ph.start, None) };
            let frame3 = match ph.three.as_mut() {
                Some(three) => {
                    let mut drv = crate::sim3d::Driver { graphs, bodies: &three.bodies, fields, statics: &statics };
                    three.world.as_mut().expect("world").frame_at(t, &mut drv).bodies
                }
                None => Vec::new(),
            };
            let frame = match ph.world.as_mut() {
                Some(w) => {
                    let mut drv = PDriver { graphs, bodies: &ph.bodies, fields, statics };
                    w.frame_at(t, &mut drv)
                }
                None => sr_sim::physics::Frame::default(),
            };
            (frame, frame3)
        };
        apply_bodies(g, &ph.bodies, &frame.bodies);
        if let Some(three) = &ph.three {
            crate::sim3d::apply(g, &three.bodies, &frame3);
        }
        for (k, s) in ph.softs.iter().enumerate() {
            let Some(i) = index_of(g, &s.id) else { continue };
            let Some(inv) = s.start_world.inverse() else { continue };
            let pts = &frame.softs[k];
            let offsets = pts.iter().zip(&s.rest_local).map(|(q, r)| {
                let l = inv.apply(*q);
                [l[0] - r[0], l[1] - r[1]]
            });
            let warp = SoftWarp { rows: s.rows, cols: s.cols, size: s.size, offsets: offsets.collect() };
            set_world(g, i, s.start_world);
            g.nodes[i].soft = Some(Arc::new(warp));
        }
    }
}

impl Runtime {
    /// Applies simulation to `g` (evaluated at `t`); `base` evaluates without simulation.
    pub fn apply(&mut self, p: &Program, g: &mut FrameGraph, t: f64, base: &dyn Fn(f64) -> FrameGraph) {
        let mut graphs = Graphs { base, cache: Vec::new() };
        if !self.built {
            self.built = true;
            let fields = build_fields(p);
            let start = p.scene.physics.as_ref().map(|p| p.start).unwrap_or(0.0);
            let g0 = if (g.time - start).abs() < 1e-12 { Arc::new(g.clone()) } else { graphs.at(start) };
            self.physics = build_physics(p, &g0, &fields, &mut self.problems);
            self.fields = Some(fields);
        }
        let fields = self.fields.as_ref().expect("built");
        // ---- physics
        if let Some(ph) = self.physics.as_mut() {
            apply_physics(ph, g, &mut graphs, fields, t);
        }
        // ---- particles
        let ids: Vec<usize> =
            g.nodes.iter().enumerate().filter(|(_, n)| n.kind == "particleEmitter").map(|(i, _)| i).collect();
        for i in ids {
            let id = g.nodes[i].id.clone();
            if self.emitters.get(&id).is_none_or(|rt| rt.replay_source) {
                // Nonlinear clocks can revisit the same source time on another
                // branch. Replay its history with this frame's branch mapping.
                let rt = build_emitter(p, &g.nodes[i], &mut self.problems);
                self.emitters.insert(id.clone(), rt);
            }
            let rt = self.emitters.get_mut(&id).expect("emitter");
            let names = rt.fields.clone();
            let mut drv = EDriver {
                graphs: &mut graphs,
                id: id.clone(),
                fields,
                names: names.as_deref(),
                physics: self.physics.as_mut(),
                p,
                start: rt.emitter.start(),
                node: rt.node,
                at: t,
                invariant: &mut rt.invariant,
            };
            let time = g.nodes[i].timeline_time;
            rt.emitter.at(time, &mut drv);
            g.nodes[i].particles = Some(Arc::new(render_frame(rt, rt.emitter.store())));
        }
        // ---- flocks and grid simulations
        self.agents.apply(p, g, &mut graphs, fields, &mut self.problems);
    }
}

fn render_frame(rt: &EmitterRt, s: &sr_sim::particles::Store) -> ParticleFrame {
    let n = s.len();
    let mut f = ParticleFrame {
        shape: rt.shape,
        color0: rt.color0.clone(),
        color1: rt.color1.clone(),
        sprite: rt.sprite.clone(),
        cols: rt.cols,
        rows: rt.rows,
        trail: rt.trail,
        orient: rt.orient,
        ..Default::default()
    };
    f.pos.reserve(n);
    for i in 0..n {
        let life = (s.age[i] / s.life[i]).clamp(0.0, 1.0);
        let su = rt.size_curve.apply(life);
        let cu = rt.color_curve.apply(life);
        f.pos.push([s.x[i] as f32, s.y[i] as f32]);
        f.vel.push([s.vx[i] as f32, s.vy[i] as f32]);
        f.size.push(((rt.size + (rt.size_end - rt.size) * su) * s.scale[i]).max(0.0) as f32);
        f.rot.push(if rt.orient { s.vy[i].atan2(s.vx[i]).to_degrees() as f32 } else { s.rot[i] as f32 });
        let ct = if rt.random_color { sr_sim::rng::unit(0xC0FE, s.id[i], 0) } else { cu };
        f.color_t.push(if rt.color1.is_some() { ct as f32 } else { 0.0 });
        f.alpha.push((1.0 + (rt.opacity_end - 1.0) * cu).clamp(0.0, 1.0) as f32);
        let cells = (rt.cols * rt.rows).max(1);
        let frame = if rt.sprite_fps > 0.0 { (s.age[i] * rt.sprite_fps) as u64 } else { s.id[i] };
        f.frame.push((frame % cells as u64) as u32);
    }
    f
}

/// Replaces body poses in transform-parent order, so a simulated child's own
/// pose wins over inherited movement, regardless of paint order.
fn apply_bodies(g: &mut FrameGraph, bodies: &[BodyNode], poses: &[PxPose]) {
    let mut ordered: Vec<_> =
        bodies.iter().zip(poses).filter_map(|(b, pose)| index_of(g, &b.id).map(|i| (i, b, pose))).collect();
    ordered.sort_by_key(|(i, _, _)| {
        let mut depth = 0;
        let mut parent = g.nodes[*i].transform_parent;
        while let Some(k) = parent {
            depth += 1;
            parent = g.nodes[k as usize].transform_parent;
        }
        depth
    });
    for (i, b, pose) in ordered {
        let [sx, sy] = b.scale;
        let (s, c) = pose.angle.to_radians().sin_cos();
        // T(pos) · R · S · T(−centre)
        let a = [c * sx, s * sx, -s * sy, c * sy];
        let e = pose.x - (a[0] * b.center[0] + a[2] * b.center[1]);
        let f = pose.y - (a[1] * b.center[0] + a[3] * b.center[1]);
        set_world(g, i, Affine([a[0], a[1], a[2], a[3], e, f]));
    }
}

/// Sets node `i`'s world transform and carries its descendants along.
fn set_world(g: &mut FrameGraph, i: usize, w: Affine) {
    let old = g.nodes[i].world;
    let Some(inv) = old.inverse() else {
        g.nodes[i].world = w;
        return;
    };
    // Affine::then composes right to left: a.then(&b) applies b first
    let delta = w.then(&inv);
    g.nodes[i].world = w;
    if let Some(pi) = g.nodes[i].transform_parent {
        if let Some(pinv) = g.nodes[pi as usize].world.inverse() {
            g.nodes[i].local = pinv.then(&w);
        }
    }
    // descendants: every node whose parent chain reaches i
    for j in 0..g.nodes.len() {
        let mut k = g.nodes[j].transform_parent;
        while let Some(pk) = k {
            if pk as usize == i {
                g.nodes[j].world = delta.then(&g.nodes[j].world);
                break;
            }
            k = g.nodes[pk as usize].transform_parent;
        }
    }
}

/// Simulates the document's physics from its start to `end` and returns the
/// cache file contents (`scene-render simulate`).
pub fn write_cache(p: &Program, end: f64, base: &dyn Fn(f64) -> FrameGraph) -> Result<Vec<u8>, String> {
    let mut problems = Vec::new();
    let fields = build_fields(p);
    let start = p.scene.physics.as_ref().map(|p| p.start).unwrap_or(0.0);
    let g0 = base(start);
    let mut ph = build_physics(p, &g0, &fields, &mut problems).ok_or("the document has no physics bodies")?;
    if ph.cached.is_some() {
        return Err("already cached".into());
    }
    let mut world = ph.world.take();
    let mut three = ph.three.take();
    let steps = (((end - start) / ph.step).ceil().max(0.0) as u64) + 1;
    let mut out = Vec::new();
    out.extend_from_slice(CACHE_MAGIC);
    out.extend_from_slice(&ph.step.to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&(ph.bodies.len() as u64).to_le_bytes());
    out.extend_from_slice(&(ph.softs.len() as u64).to_le_bytes());
    for s in &ph.softs {
        out.extend_from_slice(&((s.rows * s.cols) as u64).to_le_bytes());
    }
    out.extend_from_slice(&(three.as_ref().map(|t| t.bodies.len()).unwrap_or(0) as u64).to_le_bytes());
    out.extend_from_slice(&steps.to_le_bytes());
    let mut graphs = Graphs { base, cache: Vec::new() };
    let statics = if fields.animated { Vec::new() } else { fields.at(start, None) };
    for k in 0..steps {
        let t = start + k as f64 * ph.step;
        let f = match world.as_mut() {
            Some(w) => {
                let mut drv =
                    PDriver { graphs: &mut graphs, bodies: &ph.bodies, fields: &fields, statics: statics.clone() };
                w.frame_at(t + 1e-9, &mut drv)
            }
            None => sr_sim::physics::Frame::default(),
        };
        let f3 = match three.as_mut() {
            Some(th) => {
                let mut drv = crate::sim3d::Driver {
                    graphs: &mut graphs,
                    bodies: &th.bodies,
                    fields: &fields,
                    statics: &statics,
                };
                th.world.as_mut().expect("world").frame_at(t + 1e-9, &mut drv).bodies
            }
            None => Vec::new(),
        };
        for b in &f.bodies {
            for v in [b.x, b.y, b.angle] {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        for s in &f.softs {
            for q in s {
                out.extend_from_slice(&q[0].to_le_bytes());
                out.extend_from_slice(&q[1].to_le_bytes());
            }
        }
        for b in &f3 {
            for v in b.pos.iter().chain(&b.rot) {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(body: &str) -> crate::Evaluator {
        let xml = format!(
            r##"<scene version="1.1"><project width="64" height="36" fps="10" duration="10" background="#000000"/>
              <composition>{body}</composition></scene>"##
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
        crate::Evaluator::new(&doc, &Default::default()).unwrap()
    }

    fn index(p: &Program, id: &str) -> u32 {
        p.nodes.iter().position(|n| &*n.id == id).unwrap() as u32
    }

    #[test]
    fn emitters_that_never_change_are_recognised() {
        let ev = program(
            r#"<particleEmitter id="still" x="10" y="10" rate="20"/>
               <particleEmitter id="rated" x="10" y="10"><animate property="rate"><key time="0" value="1"/><key time="1" value="9"/></animate></particleEmitter>
               <group id="g"><animate property="x"><key time="0" value="0"/><key time="1" value="9"/></animate><particleEmitter id="carried" rate="20"/></group>
               <particleEmitter id="bound" rate="20"><transformConstraint type="copy-position" target="still"/></particleEmitter>"#,
        );
        let p = ev.program();
        assert!(time_invariant(p, index(p, "still")));
        assert!(!time_invariant(p, index(p, "rated")));
        assert!(!time_invariant(p, index(p, "carried")));
        assert!(!time_invariant(p, index(p, "bound")));
    }

    #[test]
    fn a_node_is_in_its_window_with_its_ancestors() {
        // windows on the node and its group, and a group whose clock runs at twice the speed: the check
        // agrees with the frame graph at every time
        let xml = r##"<scene version="1.1"><project width="64" height="36" fps="10" duration="10" background="#000000"/>
              <symbols><symbol id="s"><particleEmitter id="f" start="2" end="4" rate="20"/></symbol></symbols>
              <composition><group id="g" start="2" end="6"><particleEmitter id="e" start="3" end="5" rate="20"/></group>
              <instance id="h" symbol="s" start="1" speed="2"/></composition></scene>"##;
        let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
        let ev = crate::Evaluator::new(&doc, &Default::default()).unwrap();
        let p = ev.program();
        for id in ["e", "h/f"] {
            let n = index(p, id);
            let mut seen = [false, false];
            for k in 0..100 {
                let t = k as f64 * 0.1;
                let drawn = ev.evaluate(t).nodes.iter().any(|x| &*x.id == id);
                assert_eq!(crate::eval::in_window(p, n, t), drawn, "{id} at t = {t}");
                seen[drawn as usize] = true;
            }
            assert!(seen[0] && seen[1], "{id} is both in and out of its window");
        }
    }
}
