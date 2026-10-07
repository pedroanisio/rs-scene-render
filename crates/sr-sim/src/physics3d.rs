//! Rigid bodies and joints in 3D on Rapier 3D (f64, enhanced determinism),
//! stepped at a fixed rate from a start time with bounded replay checkpoints.
//! The 2D world ([`crate::physics`]) stays separate.
//!
//! Inputs and outputs use scene units (pixels) in scene axes (x right, y down,
//! z away from the camera), rotations as quaternions in those axes. Inside,
//! Rapier works in metres in a right-handed frame with y up and z toward the
//! camera: a scene point (x, y, z) is (x, −y, −z) / pixelsPerMeter.

use std::collections::BTreeMap;

use rapier3d_f64::prelude::*;

use crate::fields::{self, Field};

mod fracture;
pub use fracture::{Fracture3, FractureContact, FractureError, Fragment3};

/// A pose in scene space: position (px) and rotation (unit quaternion x, y, z, w, scene axes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose3 {
    pub pos: [f64; 3],
    pub rot: [f64; 4],
}

impl Default for Pose3 {
    fn default() -> Self {
        Pose3 { pos: [0.0; 3], rot: [0.0, 0.0, 0.0, 1.0] }
    }
}

pub use crate::physics::BodyKind;

/// Collision shape in pixels, relative to the body's centre, in the body's (scene) axes.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape3 {
    /// Half extents.
    Box([f64; 3]),
    Sphere(f64),
    /// Along the body's y axis: half height of the straight part, radius.
    Capsule(f64, f64),
    /// Along y: half height, radius.
    Cylinder(f64, f64),
    /// Along y (apex toward −y, up on screen): half height, base radius.
    Cone(f64, f64),
    /// The convex hull of points.
    Convex(Vec<[f64; 3]>),
    /// Exact triangles (for static and kinematic scenery).
    TriMesh(Vec<[f64; 3]>, Vec<[u32; 3]>),
    /// Convex parts of a closed mesh (V-HACD).
    Decomposition(Vec<[f64; 3]>, Vec<[u32; 3]>),
    /// A union of cells of a lattice, exactly: `size` is the size of a cell along the body's scene axes, and the cell with the
    /// integer key `[i, j, k]` fills the box from `[i, j, k] * size` to `[i + 1, j + 1, k + 1] * size` in the body's frame.
    /// Concave, and with the mass properties (centre of mass, inertia) of the cells.
    Voxels {
        size: [f64; 3],
        cells: Vec<[i32; 3]>,
    },
}

/// A rigid body.
#[derive(Clone, Debug)]
pub struct Body3Spec {
    pub kind: BodyKind,
    pub shape: Shape3,
    pub mass: f64,
    pub friction: f64,
    pub restitution: f64,
    pub linear_damping: f64,
    pub angular_damping: f64,
    /// px/s, scene axes.
    pub velocity: [f64; 3],
    /// Degrees per second about the scene axes.
    pub angular_velocity: [f64; 3],
    pub group: u32,
    pub collides_with: Option<Vec<u32>>,
    pub sensor: bool,
    pub fixed_rotation: bool,
    pub bullet: bool,
    pub activate_at: f64,
    pub start: Pose3,
}

/// Joint types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Joint3Kind {
    Spring,
    Distance,
    Pin,
    Rope,
    Hinge,
    Slider,
    Weld,
    Motor,
    /// A ball-and-socket joint.
    Ball,
}

/// A constraint between bodies `a` and `b` (or `a` and the world).
#[derive(Clone, Debug)]
pub struct Joint3Spec {
    pub kind: Joint3Kind,
    pub a: usize,
    pub b: Option<usize>,
    /// World anchor at the start (px, scene axes); b's centre (a's for pins) by default.
    pub anchor: Option<[f64; 3]>,
    /// Hinge, motor and slider axis (scene axes).
    pub axis: [f64; 3],
    pub rest_length: Option<f64>,
    pub stiffness: Option<f64>,
    pub damping: Option<f64>,
    /// Degrees (hinge, motor) or pixels (slider).
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Degrees per second.
    pub motor_speed: f64,
    pub max_force: Option<f64>,
    /// Newtons of linear constraint force that break the joint.
    pub break_force: Option<f64>,
}

/// World boundaries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bounds3 {
    None,
    /// A horizontal ground at scene y = `y` (the frame's bottom edge).
    Floor {
        y: f64,
    },
    /// A box round the frame: x ∈ [0, w], y ∈ [0, h], z ∈ [−d/2, d/2].
    Frame {
        w: f64,
        h: f64,
        d: f64,
    },
}

/// Everything a 3D world needs.
#[derive(Clone, Debug)]
pub struct World3Spec {
    pub start: f64,
    pub step: f64,
    /// m/s² in the physics frame (y up, z toward the camera).
    pub gravity: [f64; 3],
    pub pixels_per_meter: f64,
    pub iterations: usize,
    pub bounds: Bounds3,
    pub bodies: Vec<Body3Spec>,
    pub joints: Vec<Joint3Spec>,
    /// Fix the contacts that the edges between the triangles of a mesh make: the contact normal of a
    /// body on a flat mesh is the mesh's, whatever its tessellation, and a body that slides on it is not
    /// kicked sideways. The mesh is taken as two-sided. Off, a mesh is as it always was.
    pub fix_internal_edges: bool,
}

/// Animated inputs, asked for at simulation-step times.
pub trait Driver3 {
    /// A replacement triangle surface in the body's local scene axes. `revision`
    /// is the last successfully installed revision, including checkpoint restores.
    /// Return `None` when unchanged. Deforming dynamic bodies are unsupported.
    fn collider(&mut self, _t: f64, _which: usize, _revision: Option<u64>) -> Result<Option<ColliderUpdate3>, String> {
        Ok(None)
    }
    /// The surface of a deforming body, given the first impact the world has noticed on
    /// it, if any (see [`World3::with_impact_watches`]). The default ignores the impact
    /// and asks [`Driver3::collider`], so drivers that do not grow anything from impacts
    /// need not know about them.
    fn surface(
        &mut self,
        t: f64,
        which: usize,
        revision: Option<u64>,
        _impact: Option<&Impact3>,
    ) -> Result<Option<ColliderUpdate3>, String> {
        self.collider(t, which, revision)
    }
    /// The load on a dynamic body for the step that starts at `step`, at time `t`, in scene
    /// axes and units, or `None` for none. It may read the body's state at the start of the step,
    /// and must otherwise depend only on `step` and `body` and on records
    /// that no longer change, so that a replay applies the same load: a world restored to a
    /// checkpoint asks again for every step it re-takes. An error stops the step, which
    /// is then not taken; the default is no load, and a world that is never loaded is
    /// unchanged by this call.
    fn load(&mut self, _step: u64, _t: f64, _body: usize, _state: &BodyState) -> Result<Option<Load3>, String> {
        Ok(None)
    }
    /// Whether the body that made the impact of a watch (`source` against `owner`, see
    /// [`World3::with_impact_watches`]) is arrested at the start of step `step`, and if so at what
    /// deceleration, in scene units a second squared. `centre` is the body's centre of mass in the
    /// owner's frame, `impact` the first impact the world noticed. The world takes that much off the
    /// body's velocity relative to the owner along its direction, never more than stops it in the
    /// step, and takes the same share off its spin: it can only take energy out. It must depend only
    /// on its arguments and on records that no longer change. The default arrests nothing.
    fn capture(
        &mut self,
        _step: u64,
        _t: f64,
        _source: usize,
        _owner: usize,
        _centre: [f64; 3],
        _impact: &Impact3,
    ) -> Result<Option<f64>, String> {
        Ok(None)
    }
    /// Whether the body participates at composition time `t`. Invisible future bodies must
    /// not collide with bodies already in the world. The default keeps standalone worlds unchanged.
    fn enabled(&mut self, _t: f64, _which: usize) -> bool {
        true
    }

    /// Poses of the bodies that follow animation at `t`.
    fn kinematic(&mut self, t: f64, which: &[usize]) -> Vec<Pose3>;
    /// Force fields at `t` (pixel space, scene axes).
    fn fields(&mut self, t: f64) -> Vec<Field>;
}

pub struct ColliderUpdate3 {
    pub revision: u64,
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    /// Conservative ceiling for source data and collision acceleration storage.
    pub max_bytes: usize,
}

impl ColliderUpdate3 {
    /// Admission estimate shared with callers that must reject before tessellation.
    pub fn required_bytes(vertices: usize, triangles: usize) -> Option<usize> {
        vertices
            .checked_mul(96)
            .and_then(|v| triangles.checked_mul(524).and_then(|i| v.checked_add(i)))
            .and_then(|v| v.checked_add(4096))
    }
}

/// A body at the start of a step, as a load may need it: scene axes and units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyState {
    /// Pose of the body's origin.
    pub pose: Pose3,
    pub velocity: Velocity3,
    /// World position of the centre of mass.
    pub centre: [f64; 3],
    /// Whether the body takes part: a load on one that does not is ignored.
    pub enabled: bool,
}

/// A force and torque on a body, held for one step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Load3 {
    /// Mass times scene units per second squared, scene axes, acting at the centre of mass.
    pub force: [f64; 3],
    /// Mass times scene units squared per second squared, scene axes, about the centre of mass.
    pub torque: [f64; 3],
}

/// Rigid motion in scene axes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Velocity3 {
    /// Centre-of-mass velocity in scene units per second.
    pub linear: [f64; 3],
    /// Angular velocity in degrees per second about scene axes.
    pub angular: [f64; 3],
}

/// Simulated state at one time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame3 {
    pub bodies: Vec<Pose3>,
    pub velocities: Vec<Velocity3>,
    /// Participation after visibility and fracture activation.
    pub enabled: Vec<bool>,
    /// Fired fracture events, in registration order.
    pub fractured: Vec<bool>,
    /// Constraints removed by `breakForce` or source fracture.
    pub broken: Vec<bool>,
    /// The first impact of each watched pair, once a step has resolved it (see
    /// [`World3::with_impact_watches`]); in the order of the watches.
    pub impacts: Vec<Option<Impact3>>,
    /// Failed frames contain no poses; consumers must report these diagnostics.
    pub errors: Vec<String>,
}

#[derive(Clone)]
struct State {
    step: u64,
    islands: IslandManager,
    broad: BroadPhaseBvh,
    narrow: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    joints: ImpulseJointSet,
    multibody: MultibodyJointSet,
    soft_set: SoftBodySet,
    ccd: CCDSolver,
    handles: Vec<RigidBodyHandle>,
    joint_handles: Vec<Option<ImpulseJointHandle>>,
    active: Vec<bool>,
    collider_revisions: Vec<Option<u64>>,
    fractured: Vec<bool>,
    impacts: Vec<Option<Impact3>>,
}

struct Checkpoint {
    state: State,
    charge: usize,
}

/// One contact point resolved by the solver during a fixed step.
///
/// Units follow the rest of the world's output: positions in scene units, velocities
/// in scene units per second, scene axes, and the impulse in mass x scene units per
/// second (`pixels_per_meter` times the SI impulse), so that for a body of mass `m` the
/// impulse is `m` times the change of its velocity along the normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact3 {
    /// Index of the step that resolved the contact (the step starting at `start + step * dt`).
    pub step: u64,
    /// End of that step, the instant at which the resolved state is reported.
    pub time: f64,
    /// Indices into `World3Spec::bodies`, in ascending order; `None` is a world
    /// boundary slab (`Bounds3`), ordered after every body.
    pub bodies: [Option<usize>; 2],
    /// World-space contact point, halfway between the two surfaces at the start of the step.
    pub point: [f64; 3],
    /// Unit normal pointing from `bodies[0]` toward `bodies[1]`.
    pub normal: [f64; 3],
    /// Total impulse along the normal over the step; positive when the bodies push apart.
    pub impulse: f64,
    /// Velocity of `bodies[1]` relative to `bodies[0]` at the point, before the step.
    /// A negative component along `normal` means the bodies are approaching.
    pub relative_velocity: [f64; 3],
}

/// What the contact record keeps and how much of it: exceeding either limit is an error.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactLogConfig {
    /// Contact points one step may produce.
    pub max_per_step: usize,
    /// Bytes the retained record may use (`Contact3` storage plus per-step bookkeeping).
    pub max_bytes: usize,
    /// A pair of bodies whose contact points together push with an impulse (as in
    /// `Contact3::impulse`) not greater than this is not recorded; one that does is
    /// recorded with all its points. Zero keeps every pair that pushes; a body at rest has
    /// a small impulse every step, so a consumer interested in impacts raises it.
    pub min_impulse: f64,
}

impl ContactLogConfig {
    /// A record of every pushing contact, within the given limits.
    pub fn new(max_per_step: usize, max_bytes: usize) -> Self {
        Self { max_per_step, max_bytes, min_impulse: 0.0 }
    }
}

/// A pair of bodies whose first impact the world notices: `source` hitting `owner`,
/// indices into `World3Spec::bodies`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImpactWatch {
    pub source: usize,
    pub owner: usize,
    /// The pair's total normal impulse over one step (as in `Contact3::impulse`) must
    /// exceed this: a body at rest pushes with about its weight each step.
    pub min_impulse: f64,
}

/// The first impact of a watched pair, in the owner's own frame so that it holds wherever
/// the owner moves afterwards.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Impact3 {
    /// Index of the step that resolved the impact.
    pub step: u64,
    /// End of that step, when the impact is reported.
    pub time: f64,
    /// Total normal impulse over the pair's contacts in the step.
    pub impulse: f64,
    /// Where it hit: the contacts' mean, weighted by impulse, in the owner's frame (scene
    /// axes, scene units).
    pub point: [f64; 3],
    /// Unit normal from the owner toward the source, in the owner's frame.
    pub normal: [f64; 3],
    /// Speed of the source's approach along that normal before the step, scene units per
    /// second; positive.
    pub closing_speed: f64,
    /// Velocity of the source relative to the owner before the step, in the world's axes.
    pub relative_velocity: [f64; 3],
    /// The same velocity in the owner's frame, where `point` and `normal` are.
    pub owner_velocity: [f64; 3],
}

/// Rotates `v` by the inverse of the unit quaternion `q = [x, y, z, w]`.
fn rotate_back(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let (u, w) = ([-q[0], -q[1], -q[2]], q[3]);
    let cross =
        |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let t = cross(u, v).map(|c| 2.0 * c);
    let ut = cross(u, t);
    std::array::from_fn(|i| v[i] + w * t[i] + ut[i])
}

/// The impact of `watch` in one step, from that step's `contacts` and the owner's pose at
/// the start of it, or `None` if the pair did not push hard enough or was not approaching.
pub fn impact_in_step(watch: &ImpactWatch, contacts: &[Contact3], owner: &Pose3) -> Option<Impact3> {
    let pair = if watch.source < watch.owner {
        [Some(watch.source), Some(watch.owner)]
    } else {
        [Some(watch.owner), Some(watch.source)]
    };
    let mine: Vec<&Contact3> = contacts.iter().filter(|c| c.bodies == pair).collect();
    let first = mine.first()?;
    let impulse: f64 = mine.iter().map(|c| c.impulse).sum();
    if impulse.partial_cmp(&watch.min_impulse.max(0.0)) != Some(std::cmp::Ordering::Greater) {
        return None;
    }
    let mean = |pick: fn(&Contact3) -> [f64; 3]| -> [f64; 3] {
        let mut sum = [0.0; 3];
        for c in &mine {
            for (s, v) in sum.iter_mut().zip(pick(c)) {
                *s += c.impulse * v;
            }
        }
        sum.map(|s| s / impulse)
    };
    // normals and velocities are listed from the first body to the second; face them from
    // the owner to the source
    let sign = if watch.owner < watch.source { 1.0 } else { -1.0 };
    let normal = mean(|c| c.normal).map(|c| sign * c);
    let relative_velocity = mean(|c| c.relative_velocity).map(|c| sign * c);
    let length = normal.iter().map(|c| c * c).sum::<f64>().sqrt();
    if length.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return None;
    }
    let normal = normal.map(|c| c / length);
    let closing_speed = -relative_velocity.iter().zip(&normal).map(|(v, n)| v * n).sum::<f64>();
    if closing_speed.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return None;
    }
    let point = mean(|c| c.point);
    let from_owner: [f64; 3] = std::array::from_fn(|i| point[i] - owner.pos[i]);
    Some(Impact3 {
        step: first.step,
        time: first.time,
        impulse,
        point: rotate_back(owner.rot, from_owner),
        normal: rotate_back(owner.rot, normal),
        closing_speed,
        relative_velocity,
        owner_velocity: rotate_back(owner.rot, relative_velocity),
    })
}

/// The first impact of `watch` in a record of contacts, in step order, given the owner's
/// pose at the start of each step. This is what a recorded run, such as a baked cache,
/// holds, and it finds the impact the world found while it ran.
pub fn find_impact(watch: &ImpactWatch, contacts: &[Contact3], owner: impl Fn(u64) -> Pose3) -> Option<Impact3> {
    let mut from = 0;
    while from < contacts.len() {
        let step = contacts[from].step;
        let len = contacts[from..].iter().take_while(|c| c.step == step).count();
        if let Some(impact) = impact_in_step(watch, &contacts[from..from + len], &owner(step)) {
            return Some(impact);
        }
        from += len;
    }
    None
}

struct ContactLog {
    config: ContactLogConfig,
    steps: BTreeMap<u64, Vec<Contact3>>,
    bytes: usize,
}

/// Contact points of the watched pairs one step may produce while a pair is still to be noticed, unless the world is
/// told another limit ([`World3::with_watch_contacts_per_step`]).
const WATCH_CONTACTS_PER_STEP: usize = 4096;

/// Bookkeeping charged per retained step, on top of its contacts.
const CONTACT_STEP_BYTES: usize = 64;

/// Bytes the frame memory and the contact record may use together unless a caller says
/// otherwise. A frame costs about 110 bytes per body, so this holds a hundred bodies for
/// twelve and a half seconds at 240 steps a second, an order of magnitude below the
/// checkpoint budget.
const FRAME_LOG_BYTES: usize = 32 << 20;

/// Bookkeeping charged per retained frame, on top of its poses and flags.
const FRAME_ENTRY_BYTES: usize = 64;

/// The frames already simulated, by step: a request for one of them needs no checkpoint
/// restore and no replay.
struct FrameLog {
    budget: usize,
    frames: BTreeMap<u64, Frame3>,
    bytes: usize,
}

/// A body's motion at the start of a step, for relative velocities and contact points.
#[derive(Clone, Copy)]
struct Motion {
    pose: Pose,
    linvel: Vector,
    angvel: Vector,
    centre: Vector,
    /// Fixed or asleep: nothing was solved for it.
    idle: bool,
}

/// A deterministic 3D world.
pub struct World3 {
    spec: World3Spec,
    params: IntegrationParameters,
    pipeline: PhysicsPipeline,
    state: State,
    checkpoints: BTreeMap<u64, Checkpoint>,
    steps_per_checkpoint: u64,
    checkpoint_budget: usize,
    fractures: Vec<Fracture3>,
    fracture_sources: Vec<Option<usize>>,
    fragment_owners: Vec<Option<usize>>,
    contact_log: Option<ContactLog>,
    frame_log: FrameLog,
    watches: Vec<ImpactWatch>,
    watch_contacts_per_step: usize,
    /// Whether the shapes of a frame's deforming surfaces are built ahead, in parallel.
    prefetch: bool,
    /// Shapes built ahead and not yet taken by their step, by body and revision.
    prefetched: Vec<(usize, ColliderUpdate3, SharedShape)>,
    /// How many steps took a shape that had been built ahead.
    prefetch_hits: u64,
}

/// Whether a replacement surface can be built: within its memory budget, finite and with triangles that name its vertices.
fn validate_update(update: &ColliderUpdate3, ppm: f64) -> Result<(), String> {
    let bytes = ColliderUpdate3::required_bytes(update.vertices.len(), update.triangles.len())
        .ok_or("deforming collider memory overflow")?;
    if bytes > update.max_bytes {
        return Err("deforming collider memory budget exceeded".into());
    }
    if update.vertices.len() < 3
        || update.triangles.is_empty()
        || update.vertices.iter().flatten().any(|v| !v.is_finite() || !(v / ppm).is_finite())
        || update.triangles.iter().flatten().any(|&i| i as usize >= update.vertices.len())
    {
        return Err("invalid deforming collider vertices or triangles".into());
    }
    Ok(())
}

/// The shape of a deforming surface, in physics axes: the same one whichever thread builds it and whenever.
fn build_shape(
    vertices: &[[f64; 3]],
    triangles: Vec<[u32; 3]>,
    ppm: f64,
    fix_internal_edges: bool,
) -> Result<SharedShape, String> {
    let vertices = vertices.iter().map(|p| vec3(flip(*p).map(|c| c / ppm))).collect();
    let flags = if fix_internal_edges { TriMeshFlags::FIX_INTERNAL_EDGES_TWO_SIDED } else { TriMeshFlags::empty() };
    SharedShape::trimesh_with_flags(vertices, triangles, flags).map_err(|e| format!("invalid deforming collider: {e}"))
}

/// Scene axes ↔ physics axes: (x, y, z) ↔ (x, −y, −z), a half turn about x.
fn flip(v: [f64; 3]) -> [f64; 3] {
    [v[0], -v[1], -v[2]]
}

/// A quaternion between scene and physics axes (the half turn about x conjugates it).
fn flip_q(q: [f64; 4]) -> [f64; 4] {
    [q[0], -q[1], -q[2], q[3]]
}

fn vec3(v: [f64; 3]) -> Vector {
    Vector::new(v[0], v[1], v[2])
}

fn quat(q: [f64; 4]) -> Rotation {
    Rotation::from_xyzw(q[0], q[1], q[2], q[3]).normalize()
}

/// A local joint frame whose x axis is `axis` (the axis Rapier's revolute and prismatic joints use).
fn frame(anchor: Vector, axis: Vector) -> Pose {
    let a = if axis.length_squared() > 1e-18 { axis.normalize() } else { Vector::X };
    Pose::from_parts(anchor, Rotation::from_rotation_arc(Vector::X, a))
}

impl World3 {
    /// A world at its start.
    pub fn new(spec: World3Spec) -> World3 {
        let ppm = spec.pixels_per_meter.max(1e-9);
        let m = |p: [f64; 3]| vec3(flip(p).map(|c| c / ppm));
        let mut params = IntegrationParameters { dt: spec.step, ..Default::default() };
        params.num_solver_iterations = spec.iterations.max(1);
        let mut st = State {
            step: 0,
            islands: IslandManager::new(),
            broad: BroadPhaseBvh::new(),
            narrow: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            joints: ImpulseJointSet::new(),
            multibody: MultibodyJointSet::new(),
            soft_set: SoftBodySet::default(),
            ccd: CCDSolver::new(),
            handles: Vec::new(),
            joint_handles: Vec::new(),
            active: Vec::new(),
            collider_revisions: vec![None; spec.bodies.len()],
            fractured: Vec::new(),
            impacts: Vec::new(),
        };
        for b in &spec.bodies {
            let follows = b.kind == BodyKind::Kinematic || (b.kind == BodyKind::Dynamic && b.activate_at > spec.start);
            let builder = match (b.kind, follows) {
                (BodyKind::Static, _) => RigidBodyBuilder::fixed(),
                (_, true) => RigidBodyBuilder::kinematic_position_based(),
                _ => RigidBodyBuilder::dynamic(),
            };
            let mut builder = builder
                .pose(Pose::from_parts(m(b.start.pos), quat(flip_q(b.start.rot))))
                .linear_damping(b.linear_damping)
                .angular_damping(b.angular_damping)
                .ccd_enabled(b.bullet);
            if b.fixed_rotation {
                builder = builder.lock_rotations();
            }
            let mut rb = builder.build();
            if !follows && b.kind == BodyKind::Dynamic {
                rb.set_linvel(m(b.velocity), true);
                rb.set_angvel(vec3(flip(b.angular_velocity.map(f64::to_radians))), true);
            }
            let h = st.bodies.insert(rb);
            // shapes live in the body's frame: scene axes to physics axes, pixels to metres
            let pts = |ps: &[[f64; 3]]| -> Vec<Vector> { ps.iter().map(|p| vec3(flip(*p).map(|c| c / ppm))).collect() };
            let small = 1e-4;
            let collider = match &b.shape {
                Shape3::Box(h3) => Some(ColliderBuilder::cuboid(
                    (h3[0] / ppm).max(small),
                    (h3[1] / ppm).max(small),
                    (h3[2] / ppm).max(small),
                )),
                Shape3::Sphere(r) => Some(ColliderBuilder::ball((r / ppm).max(small))),
                Shape3::Capsule(hh, r) => Some(ColliderBuilder::capsule_y((hh / ppm).max(0.0), (r / ppm).max(small))),
                Shape3::Cylinder(hh, r) => Some(ColliderBuilder::cylinder((hh / ppm).max(small), (r / ppm).max(small))),
                // the scene's cone points up the screen (−y), which is +y in physics axes, as Rapier's
                Shape3::Cone(hh, r) => Some(ColliderBuilder::cone((hh / ppm).max(small), (r / ppm).max(small))),
                Shape3::Convex(ps) => ColliderBuilder::convex_hull(&pts(ps)),
                Shape3::TriMesh(ps, idx) if spec.fix_internal_edges => ColliderBuilder::trimesh_with_flags(
                    pts(ps),
                    flip_winding(idx),
                    TriMeshFlags::FIX_INTERNAL_EDGES_TWO_SIDED,
                )
                .ok(),
                Shape3::TriMesh(ps, idx) => ColliderBuilder::trimesh(pts(ps), flip_winding(idx)).ok(),
                Shape3::Decomposition(ps, idx) => {
                    Some(ColliderBuilder::convex_decomposition(&pts(ps), &flip_winding(idx)))
                }
                // the half turn about x that turns scene axes into physics axes maps the lattice onto itself: a cell
                // [i, j, k] is the cell [i, -j - 1, -k - 1] there, so no cell moves by half a cell
                Shape3::Voxels { size, cells } if !cells.is_empty() && size.iter().all(|s| *s > 0.0) => {
                    let keys: Vec<IVector> = cells
                        .iter()
                        .map(|c| IVector::new(i64::from(c[0]), -i64::from(c[1]) - 1, -i64::from(c[2]) - 1))
                        .collect();
                    Some(ColliderBuilder::voxels(vec3(size.map(|s| s / ppm)), &keys))
                }
                Shape3::Voxels { .. } => None,
            }
            .unwrap_or_else(|| ColliderBuilder::ball(0.01));
            let groups = {
                let member = Group::from_bits_truncate(1u32 << (b.group.min(31)));
                let filter = match &b.collides_with {
                    None => Group::ALL,
                    Some(list) => {
                        list.iter().fold(Group::NONE, |g, k| g | Group::from_bits_truncate(1u32 << (*k).min(31)))
                    }
                };
                InteractionGroups::new(member, filter, InteractionTestMode::And)
            };
            let c = collider
                .friction(b.friction)
                .restitution(b.restitution)
                .sensor(b.sensor)
                .collision_groups(groups)
                .mass(b.mass.max(1e-6))
                .build();
            st.colliders.insert_with_parent(c, h, &mut st.bodies);
            st.handles.push(h);
            st.active.push(!follows);
        }
        // bounds as fixed slabs, 5 m thick
        let slab = |st: &mut State, centre: [f64; 3], half: [f64; 3]| {
            let h = st.bodies.insert(RigidBodyBuilder::fixed().translation(m(centre)).build());
            st.colliders.insert_with_parent(
                // coefficient 0 under the Max rule: a contact takes the body's own friction and restitution
                ColliderBuilder::cuboid(half[0] / ppm, half[1] / ppm, half[2] / ppm)
                    .friction(0.0)
                    .friction_combine_rule(CoefficientCombineRule::Max)
                    .restitution(0.0)
                    .restitution_combine_rule(CoefficientCombineRule::Max)
                    .build(),
                h,
                &mut st.bodies,
            );
        };
        let t = 5.0 * ppm;
        match spec.bounds {
            Bounds3::None => {}
            Bounds3::Floor { y } => slab(&mut st, [0.0, y + t, 0.0], [1.0e4 * ppm, t, 1.0e4 * ppm]),
            Bounds3::Frame { w, h, d } => {
                let big = 2.0 * (w + h + d);
                slab(&mut st, [w * 0.5, h + t, 0.0], [big, t, big]);
                slab(&mut st, [w * 0.5, -t, 0.0], [big, t, big]);
                slab(&mut st, [-t, h * 0.5, 0.0], [t, big, big]);
                slab(&mut st, [w + t, h * 0.5, 0.0], [t, big, big]);
                slab(&mut st, [w * 0.5, h * 0.5, -d * 0.5 - t], [big, big, t]);
                slab(&mut st, [w * 0.5, h * 0.5, d * 0.5 + t], [big, big, t]);
            }
        }
        // joints
        let ground = st.bodies.insert(RigidBodyBuilder::fixed().build());
        for j in &spec.joints {
            let ha = st.handles.get(j.a).copied();
            let hb = match j.b {
                Some(b) => st.handles.get(b).copied(),
                None => Some(ground),
            };
            let (Some(ha), Some(hb)) = (ha, hb) else {
                st.joint_handles.push(None);
                continue;
            };
            let anchor_px = j.anchor.unwrap_or_else(|| match j.b {
                Some(b) => spec.bodies[b].start.pos,
                None => spec.bodies[j.a].start.pos,
            });
            let anchor = m(anchor_px);
            let (pa, pb) = (*st.bodies[ha].position(), *st.bodies[hb].position());
            let (la, lb) = (pa.inverse_transform_point(anchor), pb.inverse_transform_point(anchor));
            let axis_w = vec3(flip(j.axis));
            let (xa, xb) = (pa.rotation.inverse() * axis_w, pb.rotation.inverse() * axis_w);
            let centre_dist = (pa.translation - pb.translation).length();
            let rest = j.rest_length.map(|r| r / ppm).unwrap_or(centre_dist);
            let mass = spec.bodies.get(j.a).map(|b| b.mass).unwrap_or(1.0).max(1e-3);
            let data: GenericJoint = match j.kind {
                Joint3Kind::Spring => {
                    SpringJointBuilder::new(rest, j.stiffness.unwrap_or(10.0 * mass), j.damping.unwrap_or(0.5 * mass))
                        .local_anchor1(Vector::ZERO)
                        .local_anchor2(Vector::ZERO)
                        .build()
                        .into()
                }
                Joint3Kind::Distance => SpringJointBuilder::new(
                    rest,
                    j.stiffness.unwrap_or(1.0e4 * mass),
                    j.damping.unwrap_or(1.0e2 * mass),
                )
                .local_anchor1(Vector::ZERO)
                .local_anchor2(Vector::ZERO)
                .build()
                .into(),
                Joint3Kind::Rope => {
                    RopeJointBuilder::new(rest).local_anchor1(Vector::ZERO).local_anchor2(Vector::ZERO).build().into()
                }
                Joint3Kind::Pin | Joint3Kind::Ball => match (j.kind, j.stiffness) {
                    (Joint3Kind::Pin, Some(k)) => SpringJointBuilder::new(0.0, k, j.damping.unwrap_or(0.5 * mass))
                        .local_anchor1(la)
                        .local_anchor2(lb)
                        .build()
                        .into(),
                    _ => SphericalJointBuilder::new().local_anchor1(la).local_anchor2(lb).build().into(),
                },
                Joint3Kind::Hinge | Joint3Kind::Motor => {
                    let mut g = GenericJointBuilder::new(JointAxesMask::LOCKED_REVOLUTE_AXES)
                        .local_frame1(frame(la, xa))
                        .local_frame2(frame(lb, xb));
                    if let (Some(lo), Some(hi)) = (j.min, j.max) {
                        g = g.limits(JointAxis::AngX, [lo.to_radians(), hi.to_radians()]);
                    }
                    if j.kind == Joint3Kind::Motor {
                        g = g.motor_velocity(JointAxis::AngX, j.motor_speed.to_radians(), 1.0e3 * mass);
                        if let Some(f) = j.max_force {
                            g = g.motor_max_force(JointAxis::AngX, f);
                        }
                    }
                    g.build()
                }
                Joint3Kind::Slider => {
                    let mut g = GenericJointBuilder::new(JointAxesMask::LOCKED_PRISMATIC_AXES)
                        .local_frame1(frame(la, xa))
                        .local_frame2(frame(lb, xb));
                    if let (Some(lo), Some(hi)) = (j.min, j.max) {
                        g = g.limits(JointAxis::LinX, [lo / ppm, hi / ppm]);
                    }
                    g.build()
                }
                Joint3Kind::Weld => {
                    let rel = pa.inverse() * pb;
                    FixedJointBuilder::new().local_frame1(rel).local_frame2(Pose::IDENTITY).build().into()
                }
            };
            let h = st.joints.insert(ha, hb, data, true);
            st.joint_handles.push(Some(h));
        }
        let steps_per_checkpoint = ((1.0 / spec.step.max(1e-6)).round() as u64).max(1);
        let mut w = World3 {
            fracture_sources: vec![None; spec.bodies.len()],
            fragment_owners: vec![None; spec.bodies.len()],
            contact_log: None,
            frame_log: FrameLog { budget: FRAME_LOG_BYTES, frames: BTreeMap::new(), bytes: 0 },
            watches: Vec::new(),
            watch_contacts_per_step: WATCH_CONTACTS_PER_STEP,
            prefetch: true,
            prefetched: Vec::new(),
            prefetch_hits: 0,
            fractures: Vec::new(),
            spec,
            params,
            pipeline: PhysicsPipeline::new(),
            state: st,
            checkpoints: BTreeMap::new(),
            steps_per_checkpoint,
            checkpoint_budget: crate::timeline::BUDGET,
        };
        w.checkpoints.insert(0, Checkpoint { state: w.state.clone(), charge: 0 });
        w
    }

    /// Set the admission budget for optional replay copies, discarding existing
    /// optional checkpoints. Zero disables them. The mandatory initial/current
    /// states, input spec and solver scratch are outside this component budget.
    /// Charges estimate shape/BVH, contacts and world storage; they are not RSS.
    pub fn with_checkpoint_budget(mut self, bytes: usize) -> Self {
        self.checkpoint_budget = bytes;
        self.checkpoints.retain(|&k, _| k == 0);
        self.steps_per_checkpoint = ((1.0 / self.spec.step.max(1e-6)).round() as u64).max(1);
        self
    }

    /// Sum of admission charges for optional retained replay checkpoints.
    pub fn checkpoint_bytes(&self) -> usize {
        self.checkpoints.values().fold(0usize, |sum, cp| sum.saturating_add(cp.charge))
    }

    /// Set the bytes the frame memory may use together with the contact record, and
    /// discard the frames held. The contact record has priority: frames are a cache and
    /// give way when contacts grow, oldest first. Zero turns the memory off, and every
    /// request for an earlier time then restores a checkpoint and replays.
    pub fn with_frame_log_budget(mut self, bytes: usize) -> Self {
        self.frame_log = FrameLog { budget: bytes, frames: BTreeMap::new(), bytes: 0 };
        self
    }

    /// Whether the shapes of the deforming surfaces of the steps of a frame are built ahead, in parallel, once an impact has
    /// been noticed (the default), or each in its own step. The simulation is the same bit for bit either way.
    pub fn with_surface_prefetch(mut self, on: bool) -> Self {
        self.prefetch = on;
        self.prefetched.clear();
        self
    }

    /// How many steps have taken a surface built ahead.
    pub fn prefetch_hits(&self) -> u64 {
        self.prefetch_hits
    }

    /// Bytes charged for the frames held.
    pub fn frame_log_bytes(&self) -> usize {
        self.frame_log.bytes
    }

    /// Frames held.
    pub fn frame_log_len(&self) -> usize {
        self.frame_log.frames.len()
    }

    /// The budget the frame memory shares with the contact record.
    pub fn frame_log_budget(&self) -> usize {
        self.frame_log.budget
    }

    /// Keep the frame memory within what the contact record leaves of the budget,
    /// discarding the oldest steps first.
    fn trim_frames(&mut self) {
        let allowed = self.frame_log.budget.saturating_sub(self.contact_log_bytes());
        let log = &mut self.frame_log;
        while log.bytes > allowed {
            let Some((_, frame)) = log.frames.pop_first() else { break };
            log.bytes -= frame_bytes(&frame);
        }
    }

    /// Remember the frame of the current step, as `frame_at` reports it: after
    /// visibility and fracture activation and before the step starts.
    fn record_frame(&mut self) {
        if self.frame_log.budget == 0 {
            return;
        }
        let frame = self.snapshot();
        let bytes = frame_bytes(&frame);
        let log = &mut self.frame_log;
        if let Some(old) = log.frames.insert(self.state.step, frame) {
            log.bytes -= frame_bytes(&old);
        }
        log.bytes += bytes;
        self.trim_frames();
    }

    /// Record the contacts of every step from now on, as `config` says. Recording is
    /// off by default and changes nothing about the simulation. Discards any
    /// record held so far.
    pub fn with_contact_log(mut self, config: ContactLogConfig) -> Self {
        self.contact_log = Some(ContactLog { config, steps: BTreeMap::new(), bytes: 0 });
        self
    }

    /// Notice the first impact of each pair: `source` hitting `owner` with a pair impulse
    /// above the watch's threshold, while approaching. The impact is part of the world's
    /// state, so it is the same after any seek and in any fresh world; frames report it
    /// from the step after the one that resolved it (`Frame3::impacts`), and the owner's
    /// surface is asked about it (`Driver3::surface`). Watching changes nothing about the
    /// simulation. An owner watched against several sources is told the earliest of the impacts they have
    /// noticed. Must be set before the first step.
    pub fn with_impact_watches(mut self, watches: Vec<ImpactWatch>) -> Result<Self, String> {
        if self.state.step != 0 {
            return Err("impact watches must be set before the first step".into());
        }
        let bodies = self.spec.bodies.len();
        for w in &watches {
            if w.source >= bodies || w.owner >= bodies {
                return Err(format!("impact watch names body {} or {}, and the world has {bodies}", w.source, w.owner));
            }
            if w.source == w.owner {
                return Err(format!("impact watch of body {} against itself", w.source));
            }
            if !w.min_impulse.is_finite() {
                return Err("impact watch threshold must be finite".into());
            }
        }
        // a fracture by contact reads the impact of a watch against its own source
        for e in &self.fractures {
            if let Some(c) = e.contact {
                if watches.get(c.watch).is_none_or(|w| w.owner != e.source) {
                    return Err(format!("the fracture of body {} names no impact watch against it", e.source));
                }
            }
        }
        self.state.impacts = vec![None; watches.len()];
        for cp in self.checkpoints.values_mut() {
            cp.state.impacts = vec![None; watches.len()];
        }
        self.watches = watches;
        let budget = self.frame_log.budget;
        Ok(self.with_frame_log_budget(budget))
    }

    /// The most contact points of the watched pairs that one step may produce while one of them is still to be
    /// noticed (default 4096); more is an error. Only the pairs that are watched count. Must be set before the
    /// first step.
    pub fn with_watch_contacts_per_step(mut self, limit: usize) -> Result<Self, String> {
        if self.state.step != 0 {
            return Err("the contact limit of the watches must be set before the first step".into());
        }
        self.watch_contacts_per_step = limit;
        Ok(self)
    }

    /// The contacts resolved by step `step`, in a stable order: by body pair, then
    /// point, then impulse. `Some(&[])` for a step without contacts; `None` when
    /// recording is off, the step is not simulated yet in the current timeline (after a
    /// backward seek, steps are reported again as they are replayed), or the record
    /// was discarded.
    pub fn contacts_at(&self, step: u64) -> Option<&[Contact3]> {
        if step >= self.state.step {
            return None;
        }
        self.contact_log.as_ref()?.steps.get(&step).map(Vec::as_slice)
    }

    /// Bytes charged for the retained record.
    pub fn contact_log_bytes(&self) -> usize {
        self.contact_log.as_ref().map_or(0, |log| log.bytes)
    }

    /// Release the record of every step before `step`.
    pub fn discard_contacts_before(&mut self, step: u64) {
        if let Some(log) = &mut self.contact_log {
            let kept = log.steps.split_off(&step);
            log.steps = kept;
            log.bytes = log.steps.values().map(|v| contact_bytes(v.len())).sum();
        }
    }

    /// The motion of every body at the start of a step, indexed by arena slot.
    fn capture_motion(&self) -> Vec<Option<Motion>> {
        let st = &self.state;
        let mut motion = Vec::new();
        for (handle, body) in st.bodies.iter() {
            let slot = handle.into_raw_parts().0 as usize;
            if motion.len() <= slot {
                motion.resize(slot + 1, None);
            }
            motion[slot] = Some(Motion {
                pose: *body.position(),
                linvel: body.linvel(),
                angvel: body.angvel(),
                centre: body.center_of_mass(),
                idle: body.is_fixed() || body.is_sleeping(),
            });
        }
        motion
    }

    /// The contacts the step `step` resolved, in the documented order.
    fn collect_contacts(
        &self,
        step: u64,
        motion: &[Option<Motion>],
        min_impulse: f64,
        max_per_step: usize,
        only: Option<&[[usize; 2]]>,
    ) -> Result<Vec<Contact3>, String> {
        let st = &self.state;
        let ppm = self.spec.pixels_per_meter.max(1e-9);
        let mut index_of = vec![None; motion.len()];
        for (k, handle) in st.handles.iter().enumerate() {
            let slot = handle.into_raw_parts().0 as usize;
            if let Some(entry) = index_of.get_mut(slot) {
                *entry = Some(k);
            }
        }
        let time = self.spec.start + (step + 1) as f64 * self.spec.step;
        let mut contacts = Vec::new();
        for pair in st.narrow.contact_pairs() {
            let (c1, c2) = (&st.colliders[pair.collider1], &st.colliders[pair.collider2]);
            if c1.is_sensor() || c2.is_sensor() {
                continue;
            }
            let (Some(h1), Some(h2)) = (c1.parent(), c2.parent()) else { continue };
            let (slot1, slot2) = (h1.into_raw_parts().0 as usize, h2.into_raw_parts().0 as usize);
            let (Some(Some(m1)), Some(Some(m2))) = (motion.get(slot1), motion.get(slot2)) else { continue };
            // A pair that was idle before the step and still is after it was not solved: its
            // manifolds keep the impulses of the last step it was.
            let idle = |h: RigidBodyHandle| st.bodies.get(h).is_none_or(|b| b.is_fixed() || b.is_sleeping());
            if m1.idle && m2.idle && idle(h1) && idle(h2) {
                continue;
            }
            let (a1, a2) = (index_of.get(slot1).copied().flatten(), index_of.get(slot2).copied().flatten());
            // Order the pair by body index, boundary slabs last; flip the normal with it.
            let swap = a1.unwrap_or(usize::MAX) > a2.unwrap_or(usize::MAX);
            // the pairs that are looked for, when it is those only that matter: the others are not even read
            if let Some(only) = only {
                let key = [
                    a1.unwrap_or(usize::MAX).min(a2.unwrap_or(usize::MAX)),
                    a1.unwrap_or(usize::MAX).max(a2.unwrap_or(usize::MAX)),
                ];
                if !only.contains(&key) {
                    continue;
                }
            }
            let (first, second) = if swap { ((a2, m2), (a1, m1)) } else { ((a1, m1), (a2, m2)) };
            let lever = |c: &Collider| c.position_wrt_parent().copied().unwrap_or(Pose::IDENTITY);
            let (pose1, pose2) = (m1.pose * lever(c1), m2.pose * lever(c2));
            // The manifolds the solver saw: contact clusters for composite shapes (meshes,
            // compounds), the plain manifolds otherwise. Only those carry the impulses.
            let mut points = Vec::new();
            for manifold in pair.solver_manifolds() {
                for contact in &manifold.points {
                    let impulse = contact.data.impulse * ppm;
                    if impulse.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
                        continue;
                    }
                    let point = (pose1 * contact.local_p1 + pose2 * contact.local_p2) * 0.5;
                    let speed = |m: &Motion| m.linvel + m.angvel.cross(point - m.centre);
                    let relative = speed(second.1) - speed(first.1);
                    let normal = if swap { -manifold.data.normal } else { manifold.data.normal };
                    points.push(Contact3 {
                        step,
                        time,
                        bodies: [first.0, second.0],
                        point: flip(point.to_array()).map(|c| c * ppm),
                        normal: flip(normal.to_array()),
                        impulse,
                        relative_velocity: flip(relative.to_array()).map(|c| c * ppm),
                    });
                }
            }
            // a pair is recorded whole or not at all: its total impulse decides, not how
            // many points a mesh or a compound shape happens to split it over
            let total: f64 = points.iter().map(|c| c.impulse).sum();
            if total.partial_cmp(&min_impulse.max(0.0)) == Some(std::cmp::Ordering::Greater) {
                contacts.append(&mut points);
            }
            if contacts.len() > max_per_step {
                return Err(format!(
                    "{} contacts in one step exceed the contact log limit of {max_per_step}",
                    contacts.len()
                ));
            }
        }
        let key = |b: Option<usize>| b.unwrap_or(usize::MAX);
        contacts.sort_by(|x, y| {
            (key(x.bodies[0]), key(x.bodies[1]))
                .cmp(&(key(y.bodies[0]), key(y.bodies[1])))
                .then_with(|| {
                    x.point
                        .iter()
                        .zip(&y.point)
                        .map(|(p, q)| p.total_cmp(q))
                        .find(|o| o.is_ne())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| x.impulse.total_cmp(&y.impulse))
        });
        Ok(contacts)
    }

    /// Look for the impacts of the watched pairs that have not happened yet, in the step
    /// just solved.
    fn notice_impacts(&mut self, step: u64, motion: &[Option<Motion>]) -> Result<(), String> {
        let threshold = self
            .watches
            .iter()
            .zip(&self.state.impacts)
            .filter(|(_, found)| found.is_none())
            .map(|(w, _)| w.min_impulse)
            .fold(f64::INFINITY, f64::min);
        // only the pairs that are watched and not yet noticed can be an impact
        let pairs: Vec<[usize; 2]> = self
            .watches
            .iter()
            .zip(&self.state.impacts)
            .filter(|(_, found)| found.is_none())
            .map(|(w, _)| [w.source.min(w.owner), w.source.max(w.owner)])
            .collect();
        let contacts = self.collect_contacts(step, motion, threshold, self.watch_contacts_per_step, Some(&pairs))?;
        if contacts.is_empty() {
            return Ok(());
        }
        let ppm = self.spec.pixels_per_meter.max(1e-9);
        for k in 0..self.watches.len() {
            let watch = self.watches[k];
            if self.state.impacts[k].is_some() {
                continue;
            }
            let handle = self.state.handles[watch.owner];
            let Some(Some(owner)) = motion.get(handle.into_raw_parts().0 as usize) else { continue };
            // the owner's pose as a frame reports it: scene axes and units
            let q = owner.pose.rotation;
            let pose = Pose3 {
                pos: flip(owner.pose.translation.to_array()).map(|c| c * ppm),
                rot: flip_q([q.x, q.y, q.z, q.w]),
            };
            self.state.impacts[k] = impact_in_step(&watch, &contacts, &pose);
        }
        Ok(())
    }

    /// Store a step's contacts, replacing a replayed step's identical record, or
    /// fail without storing when the memory limit would be exceeded.
    fn store_contacts(&mut self, step: u64, contacts: Vec<Contact3>) -> Result<(), String> {
        let Some(log) = &mut self.contact_log else { return Ok(()) };
        let old = log.steps.get(&step).map_or(0, |v| contact_bytes(v.len()));
        let total = log.bytes - old + contact_bytes(contacts.len());
        if total > log.config.max_bytes {
            return Err(format!("contact log memory of {total} bytes exceeds the limit of {}", log.config.max_bytes));
        }
        log.bytes = total;
        log.steps.insert(step, contacts);
        self.trim_frames();
        Ok(())
    }

    /// Return to the nearest retained checkpoint at or before the current step
    /// (used when a step fails after the solver already ran).
    fn restore_nearest_checkpoint(&mut self) {
        if let Some((_, cp)) = self.checkpoints.range(..=self.state.step).next_back() {
            self.state = cp.state.clone();
        }
    }

    fn checkpoint_charge(&self) -> usize {
        let st = &self.state;
        let items = st.bodies.len().saturating_add(st.colliders.len()).saturating_add(self.spec.joints.len());
        let mut bytes = items.saturating_mul(8192).saturating_add(4096);
        for (_, collider) in st.colliders.iter() {
            bytes = bytes.saturating_add(shape_charge(collider.shape()));
        }
        for pair in st.narrow.contact_pairs() {
            bytes = bytes.saturating_add(4096);
            for manifold in pair.manifolds() {
                bytes = bytes.saturating_add(
                    manifold
                        .points
                        .capacity()
                        .saturating_add(manifold.data.solver_contacts.len())
                        .saturating_add(1)
                        .saturating_mul(1024),
                );
            }
        }
        bytes
    }

    fn save_checkpoint(&mut self) {
        let step = self.state.step;
        if step % self.steps_per_checkpoint != 0 || self.checkpoints.contains_key(&step) {
            return;
        }
        let charge = self.checkpoint_charge();
        if charge > self.checkpoint_budget {
            return;
        }
        // Admit before cloning. Thinning doubles temporal spacing, bounds the
        // count even for tiny worlds, and drops obsolete geometry Arcs first.
        while self.checkpoints.len() >= 65 || self.checkpoint_bytes() > self.checkpoint_budget - charge {
            self.steps_per_checkpoint = self.steps_per_checkpoint.saturating_mul(2);
            let every = self.steps_per_checkpoint;
            self.checkpoints.retain(|&k, _| k == 0 || k % every == 0);
            if step % every != 0 {
                return;
            }
        }
        self.checkpoints.insert(step, Checkpoint { state: self.state.clone(), charge });
    }

    /// Synchronize birth/departure before stepping and at sample boundaries. On birth, take
    /// the animated pose at that time rather than the transform at physics initialization.
    fn sync_visibility(&mut self, t: f64, driver: &mut dyn Driver3) {
        let ppm = self.spec.pixels_per_meter.max(1e-9);
        let mut born = false;
        for (k, spec) in self.spec.bodies.iter().enumerate() {
            let enabled = self.fracture_enabled(k) && driver.enabled(t, k);
            let handle = self.state.handles[k];
            if enabled == self.state.bodies[handle].is_enabled() {
                continue;
            }
            let body = &mut self.state.bodies[handle];
            body.set_enabled(enabled);
            if enabled {
                born = true;
                // A fragment keeps its inherited motion when a visibility
                // window reopens; its authored placeholder pose is never used.
                if self.fragment_owners[k].is_some() {
                    body.wake_up(true);
                    continue;
                }
                if let Some(pose) = driver.kinematic(t, &[k]).first() {
                    body.set_position(
                        Pose::from_parts(vec3(flip(pose.pos).map(|c| c / ppm)), quat(flip_q(pose.rot))),
                        true,
                    );
                }
                body.set_linvel(vec3(flip(spec.velocity).map(|c| c / ppm)), true);
                body.set_angvel(vec3(flip(spec.angular_velocity.map(f64::to_radians))), true);
            }
        }
        // Newly enabled scenery may overlap a sleeping island that had no contact edge
        // while the scenery was disabled. Wake it so narrow-phase contacts get solved.
        if born {
            for &handle in &self.state.handles {
                let body = &mut self.state.bodies[handle];
                if body.is_enabled() && body.is_dynamic() {
                    body.wake_up(true);
                }
            }
        }
    }

    /// The earliest impact any watch against body `k` has noticed (the first watch on a tie), which the owner is told.
    fn impact_of(&self, k: usize) -> Option<Impact3> {
        self.watches
            .iter()
            .enumerate()
            .filter(|(_, w)| w.owner == k)
            .filter_map(|(w, _)| self.state.impacts[w].map(|i| (i.step, w, i)))
            .min_by_key(|(step, w, _)| (*step, *w))
            .map(|(_, _, i)| i)
    }

    fn sync_colliders(&mut self, t: f64, driver: &mut dyn Driver3) -> Result<(), String> {
        let ppm = self.spec.pixels_per_meter.max(1e-9);
        // Validate every replacement before changing the world. Failure leaves
        // all collider revisions and shapes available for a deterministic retry.
        let mut replacements = Vec::new();
        for k in 0..self.spec.bodies.len() {
            if !self.fracture_enabled(k) || !driver.enabled(t, k) {
                continue;
            }
            let impact = self.impact_of(k);
            let Some(update) = driver.surface(t, k, self.state.collider_revisions[k], impact.as_ref())? else {
                continue;
            };
            if self.spec.bodies[k].kind == BodyKind::Dynamic {
                return Err("deforming colliders require static or kinematic bodies".into());
            }
            validate_update(&update, ppm)?;
            // a shape built ahead for this very surface is taken, and any other is built now
            let ahead = self
                .prefetched
                .iter()
                .position(|(body, built, _)| *body == k && built.revision == update.revision)
                .filter(|&at| {
                    self.prefetched[at].1.vertices == update.vertices
                        && self.prefetched[at].1.triangles == update.triangles
                });
            let shape = match ahead {
                Some(at) => {
                    self.prefetch_hits += 1;
                    self.prefetched.remove(at).2
                }
                None => build_shape(&update.vertices, update.triangles, ppm, self.spec.fix_internal_edges)?,
            };
            replacements.push((k, update.revision, shape));
        }
        let changed = !replacements.is_empty();
        for (k, revision, shape) in replacements {
            let handle = self.state.bodies[self.state.handles[k]].colliders()[0];
            self.state.colliders[handle].set_shape(shape);
            self.state.collider_revisions[k] = Some(revision);
        }
        // Excavation can remove support from a sleeping body; a raised rim can
        // meet a sleeping island without an existing contact edge.
        if changed {
            for &handle in &self.state.handles {
                let body = &mut self.state.bodies[handle];
                if body.is_enabled() && body.is_dynamic() {
                    body.wake_up(true);
                }
            }
        }
        Ok(())
    }

    /// Builds ahead, on several threads, the shapes of the deforming surfaces that the steps up to `target` will install.
    /// A surface is a function of its time once the impact that makes it is known, so what is built is what each step
    /// would build for itself; a step takes the shape whose surface it finds equal. Only the impacts noticed already are
    /// used (a frame in which one is noticed builds its shapes step by step), and any failure is left to the steps, which
    /// meet it in their own order.
    fn prefetch_surfaces(&mut self, target: u64, driver: &mut dyn Driver3) {
        use rayon::prelude::*;
        self.prefetched.clear();
        if !self.prefetch || self.watches.is_empty() || target < self.state.step + 2 {
            return;
        }
        let ppm = self.spec.pixels_per_meter.max(1e-9);
        let mut revisions = self.state.collider_revisions.clone();
        let mut updates: Vec<(usize, ColliderUpdate3)> = Vec::new();
        for step in self.state.step + 1..=target {
            // the surface of the step that ends at `t` is asked for at `t`
            let t = self.spec.start + step as f64 * self.spec.step;
            for (k, revision) in revisions.iter_mut().enumerate() {
                let Some(impact) = self.impact_of(k) else { continue };
                if !self.fracture_enabled(k) || !driver.enabled(t, k) || self.spec.bodies[k].kind == BodyKind::Dynamic {
                    continue;
                }
                match driver.surface(t, k, *revision, Some(&impact)) {
                    Ok(Some(update)) if validate_update(&update, ppm).is_ok() => {
                        *revision = Some(update.revision);
                        updates.push((k, update));
                    }
                    Ok(None) => {}
                    _ => return,
                }
            }
        }
        if updates.len() < 2 {
            return;
        }
        let fix = self.spec.fix_internal_edges;
        let built: Result<Vec<_>, String> = updates
            .into_par_iter()
            .map(|(k, update)| {
                let shape = build_shape(&update.vertices, update.triangles.clone(), ppm, fix)?;
                Ok((k, update, shape))
            })
            .collect();
        if let Ok(built) = built {
            self.prefetched = built;
        }
    }

    fn step_once(&mut self, driver: &mut dyn Driver3) -> Result<(), String> {
        // the loads first: a step that cannot get one is not taken, and nothing has changed
        let t = self.spec.start + self.state.step as f64 * self.spec.step;
        let mut loads = Vec::new();
        for (k, b) in self.spec.bodies.iter().enumerate() {
            if b.kind == BodyKind::Dynamic {
                if let Some(load) = driver.load(self.state.step, t, k, &self.body_state(k))? {
                    loads.push((k, load));
                }
            }
        }
        // the bodies that the driver arrests after their impact: a step it cannot answer is not taken
        let mut arrests = Vec::new();
        for w in 0..self.watches.len() {
            let watch = self.watches[w];
            let Some(impact) = self.state.impacts.get(w).copied().flatten() else { continue };
            let (source, owner) = (self.body_state(watch.source), self.body_state(watch.owner));
            let from_owner: [f64; 3] = std::array::from_fn(|i| source.centre[i] - owner.pose.pos[i]);
            let centre = rotate_back(owner.pose.rot, from_owner);
            if let Some(deceleration) =
                driver.capture(self.state.step, t, watch.source, watch.owner, centre, &impact)?
            {
                if !(deceleration.is_finite() && deceleration >= 0.0) {
                    return Err("a body is arrested with a deceleration that is not a number".into());
                }
                arrests.push((watch.source, watch.owner, deceleration));
            }
        }
        self.sync_colliders(t + self.spec.step, driver)?;
        self.sync_visibility(t, driver);
        self.apply_fractures(t, driver)?;
        self.record_frame();
        let st = &mut self.state;
        let ppm = self.spec.pixels_per_meter.max(1e-9);
        let mut follow = Vec::new();
        for (k, b) in self.spec.bodies.iter().enumerate() {
            if !st.bodies[st.handles[k]].is_enabled() {
                continue;
            }
            if b.kind == BodyKind::Dynamic && !st.active[k] && t >= b.activate_at {
                let body = &mut st.bodies[st.handles[k]];
                body.set_body_type(RigidBodyType::Dynamic, true);
                body.set_linvel(vec3(flip(b.velocity).map(|c| c / ppm)), true);
                body.set_angvel(vec3(flip(b.angular_velocity.map(f64::to_radians))), true);
                st.active[k] = true;
            }
            if b.kind == BodyKind::Kinematic || (b.kind == BodyKind::Dynamic && !st.active[k]) {
                follow.push(k);
            }
        }
        if !follow.is_empty() {
            let poses = driver.kinematic(t + self.spec.step, &follow);
            for (k, p) in follow.iter().zip(poses) {
                let body = &mut st.bodies[st.handles[*k]];
                body.set_next_kinematic_translation(vec3(flip(p.pos).map(|c| c / ppm)));
                body.set_next_kinematic_rotation(quat(flip_q(p.rot)));
            }
        }
        // force fields (pixels, scene axes) on dynamic bodies
        let fields = driver.fields(t);
        for k in 0..self.spec.bodies.len() {
            let body = &mut st.bodies[st.handles[k]];
            body.reset_forces(false);
            body.reset_torques(false);
            if !body.is_enabled() || !body.is_dynamic() || fields.is_empty() {
                continue;
            }
            let p = flip(body.translation().to_array()).map(|c| c * ppm);
            let lv = flip(body.linvel().to_array()).map(|c| c * ppm);
            let a = fields::total3(&fields, p, lv, t);
            let mass = body.mass();
            body.add_force(vec3(flip(a).map(|c| c / ppm * mass)), true);
        }
        for (k, load) in loads {
            let body = &mut st.bodies[st.handles[k]];
            let nothing = load.force.iter().chain(&load.torque).all(|c| *c == 0.0);
            if nothing || !body.is_enabled() || !body.is_dynamic() {
                continue;
            }
            body.add_force(vec3(flip(load.force).map(|c| c / ppm)), true);
            body.add_torque(vec3(flip(load.torque).map(|c| c / (ppm * ppm))), true);
        }
        for (source, owner, deceleration) in arrests {
            let (v, w, v_owner) = {
                let (body, other) = (&st.bodies[st.handles[source]], &st.bodies[st.handles[owner]]);
                (body.linvel(), body.angvel(), other.linvel())
            };
            let relative = v - v_owner;
            let speed = relative.length();
            // the share of the relative velocity that this step takes off: all of it if that stops the body
            let gone = deceleration / ppm * self.spec.step;
            let share = if speed > 0.0 { (gone / speed).min(1.0) } else { 1.0 };
            let body = &mut st.bodies[st.handles[source]];
            body.set_linvel(v - relative * share, true);
            body.set_angvel(w * (1.0 - share), true);
        }
        let pending = self.state.impacts.iter().any(Option::is_none);
        let motion = (self.contact_log.is_some() || pending).then(|| self.capture_motion());
        let st = &mut self.state;
        let g = self.spec.gravity;
        self.pipeline.step(
            Vector::new(g[0], g[1], g[2]),
            &self.params,
            &mut st.islands,
            &mut st.broad,
            &mut st.narrow,
            &mut st.bodies,
            &mut st.colliders,
            &mut st.joints,
            &mut st.multibody,
            &mut st.soft_set,
            &mut st.ccd,
            &(),
            &(),
        );
        for (k, j) in self.spec.joints.iter().enumerate() {
            let (Some(limit), Some(h)) = (j.break_force, st.joint_handles[k]) else { continue };
            let Some(joint) = st.joints.get(h) else { continue };
            // the impulse of the last solver substep (dt / iterations): force in newtons
            let imp = joint.impulses;
            let substep = self.spec.step / self.params.num_solver_iterations.max(1) as f64;
            let f = (imp[0] * imp[0] + imp[1] * imp[1] + imp[2] * imp[2]).sqrt() / substep;
            if f > limit {
                st.joints.remove(h, true);
                st.joint_handles[k] = None;
            }
        }
        if let Some(motion) = motion {
            let step = self.state.step;
            let mut noticed = Ok(());
            if let Some(log) = &self.contact_log {
                let config = log.config;
                noticed = self
                    .collect_contacts(step, &motion, config.min_impulse, config.max_per_step, None)
                    .and_then(|c| self.store_contacts(step, c));
            }
            if noticed.is_ok() && pending {
                noticed = self.notice_impacts(step, &motion);
            }
            if let Err(error) = noticed {
                // The solver already advanced the world: go back so a retry fails the same way.
                self.restore_nearest_checkpoint();
                return Err(error);
            }
        }
        self.state.step += 1;
        self.save_checkpoint();
        Ok(())
    }

    /// Steps before `t` (none before the start).
    pub fn step_index(&self, t: f64) -> u64 {
        if t <= self.spec.start {
            0
        } else {
            ((t - self.spec.start) / self.spec.step + 1e-9).floor() as u64
        }
    }

    /// The simulated state at `t`: the remembered frame when the step has been simulated
    /// and kept, otherwise replayed from the nearest checkpoint at or before it.
    pub fn frame_at(&mut self, t: f64, driver: &mut dyn Driver3) -> Frame3 {
        let target = self.step_index(t);
        if let Some(frame) = self.frame_log.frames.get(&target) {
            return frame.clone();
        }
        self.prefetched.clear();
        if self.state.step > target || target - self.state.step > self.steps_per_checkpoint {
            if let Some((_, cp)) = self.checkpoints.range(..=target).next_back() {
                if cp.state.step > self.state.step || self.state.step > target {
                    self.state = cp.state.clone();
                }
            }
        }
        self.prefetch_surfaces(target, driver);
        while self.state.step < target {
            if let Err(error) = self.step_once(driver) {
                self.prefetched.clear();
                return Frame3 { errors: vec![error], ..Default::default() };
            }
        }
        self.prefetched.clear();
        if let Err(error) = self.sync_colliders(self.spec.start + target as f64 * self.spec.step, driver) {
            return Frame3 { errors: vec![error], ..Default::default() };
        }
        self.sync_visibility(self.spec.start + target as f64 * self.spec.step, driver);
        if let Err(error) = self.apply_fractures(self.spec.start + target as f64 * self.spec.step, driver) {
            return Frame3 { errors: vec![error], ..Default::default() };
        }
        self.snapshot()
    }

    /// Body `k` as it is now.
    fn body_state(&self, k: usize) -> BodyState {
        let ppm = self.spec.pixels_per_meter.max(1e-9);
        let b = &self.state.bodies[self.state.handles[k]];
        let q = b.rotation();
        BodyState {
            pose: Pose3 { pos: flip(b.translation().to_array()).map(|c| c * ppm), rot: flip_q([q.x, q.y, q.z, q.w]) },
            velocity: Velocity3 {
                linear: flip(b.linvel().to_array()).map(|v| v * ppm),
                angular: flip(b.angvel().to_array()).map(f64::to_degrees),
            },
            centre: flip(b.center_of_mass().to_array()).map(|c| c * ppm),
            enabled: b.is_enabled(),
        }
    }

    fn snapshot(&self) -> Frame3 {
        let ppm = self.spec.pixels_per_meter.max(1e-9);
        let st = &self.state;
        let bodies = st
            .handles
            .iter()
            .map(|h| {
                let b = &st.bodies[*h];
                let q = b.rotation();
                Pose3 { pos: flip(b.translation().to_array()).map(|c| c * ppm), rot: flip_q([q.x, q.y, q.z, q.w]) }
            })
            .collect();
        let velocities = st
            .handles
            .iter()
            .map(|h| {
                let b = &st.bodies[*h];
                Velocity3 {
                    linear: flip(b.linvel().to_array()).map(|v| v * ppm),
                    angular: flip(b.angvel().to_array()).map(f64::to_degrees),
                }
            })
            .collect();
        Frame3 {
            bodies,
            velocities,
            enabled: st.handles.iter().map(|h| st.bodies[*h].is_enabled()).collect(),
            fractured: st.fractured.clone(),
            impacts: st.impacts.clone(),
            broken: st.joint_handles.iter().map(|h| h.is_none()).collect(),
            errors: Vec::new(),
        }
    }

    /// Steps simulated so far and checkpoints held.
    pub fn progress(&self) -> (u64, usize) {
        (self.state.step, self.checkpoints.len())
    }
}

/// The mass properties of a body of one shape and `mass`, as the world gives them to its collider, in the scene's units and axes:
/// the centre of mass in the body's frame and the inertia tensor about it (kilograms scene units squared), a metre being
/// `pixels_per_meter` scene units. Only for shapes of cells so far.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeMass {
    pub mass: f64,
    pub centre: [f64; 3],
    pub inertia: [[f64; 3]; 3],
}

pub fn shape_mass_properties(shape: &Shape3, mass: f64, pixels_per_meter: f64) -> Result<ShapeMass, String> {
    let Shape3::Voxels { size, cells } = shape else {
        return Err("mass properties are given only for bodies of cells".into());
    };
    if cells.is_empty()
        || !size.iter().all(|s| *s > 0.0)
        || mass.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
        || pixels_per_meter.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
    {
        return Err("a body of cells needs cells, a positive size, a positive mass and a positive scale".into());
    }
    let keys: Vec<IVector> =
        cells.iter().map(|c| IVector::new(i64::from(c[0]), -i64::from(c[1]) - 1, -i64::from(c[2]) - 1)).collect();
    let collider = ColliderBuilder::voxels(vec3(size.map(|s| s / pixels_per_meter)), &keys).mass(mass).build();
    let props = collider.mass_properties();
    let inertia = props.reconstruct_inertia_matrix();
    // physics axes to scene axes: a half turn about x, so a product of inertia with one of y and z (not both) changes sign
    let sign = [1.0, -1.0, -1.0];
    let scene: [[f64; 3]; 3] = std::array::from_fn(|a| {
        std::array::from_fn(|b| inertia.col(b)[a] * sign[a] * sign[b] * pixels_per_meter * pixels_per_meter)
    });
    let centre = props.local_com;
    Ok(ShapeMass {
        mass: props.mass(),
        centre: [centre.x * pixels_per_meter, -centre.y * pixels_per_meter, -centre.z * pixels_per_meter],
        inertia: scene,
    })
}

/// The volume a shape encloses, in the cube of the shape's own length unit. A mesh counts as
/// closed: its volume is the absolute sum of the signed volumes of the tetrahedra its
/// triangles make with the origin.
pub fn shape_volume(shape: &Shape3) -> Result<f64, String> {
    use std::f64::consts::PI;
    let volume = match shape {
        Shape3::Box(h) => 8.0 * h[0] * h[1] * h[2],
        Shape3::Sphere(r) => 4.0 / 3.0 * PI * r.powi(3),
        Shape3::Capsule(hh, r) => PI * r * r * 2.0 * hh + 4.0 / 3.0 * PI * r.powi(3),
        Shape3::Cylinder(hh, r) => PI * r * r * 2.0 * hh,
        Shape3::Cone(hh, r) => PI * r * r * 2.0 * hh / 3.0,
        Shape3::Convex(points) => {
            let points: Vec<_> = points.iter().map(|p| vec3(*p)).collect();
            let hull = SharedShape::convex_hull(&points).ok_or("the points have no convex hull")?;
            hull.mass_properties(1.0).mass()
        }
        Shape3::Voxels { size, cells } => {
            let mut unique = cells.clone();
            unique.sort_unstable();
            unique.dedup();
            unique.len() as f64 * size[0] * size[1] * size[2]
        }
        Shape3::TriMesh(points, triangles) | Shape3::Decomposition(points, triangles) => {
            let at = |i: u32| points.get(i as usize).copied().ok_or("a triangle names a missing vertex");
            let mut signed = 0.0;
            for t in triangles {
                let (a, b, c) = (at(t[0])?, at(t[1])?, at(t[2])?);
                let cross = [b[1] * c[2] - b[2] * c[1], b[2] * c[0] - b[0] * c[2], b[0] * c[1] - b[1] * c[0]];
                signed += (a[0] * cross[0] + a[1] * cross[1] + a[2] * cross[2]) / 6.0;
            }
            signed.abs()
        }
    };
    if volume.is_finite() && volume > 0.0 {
        Ok(volume)
    } else {
        Err("the shape encloses no volume".into())
    }
}

fn frame_bytes(frame: &Frame3) -> usize {
    let per_body = std::mem::size_of::<Pose3>() + std::mem::size_of::<Velocity3>() + 3;
    std::mem::size_of::<Frame3>()
        + frame.bodies.len() * per_body
        + frame.impacts.len() * std::mem::size_of::<Option<Impact3>>()
        + FRAME_ENTRY_BYTES
}

fn contact_bytes(contacts: usize) -> usize {
    contacts.saturating_mul(std::mem::size_of::<Contact3>()).saturating_add(CONTACT_STEP_BYTES)
}

fn shape_charge(shape: &dyn rapier3d_f64::parry::shape::Shape) -> usize {
    if let Some(mesh) = shape.as_trimesh() {
        ColliderUpdate3::required_bytes(mesh.vertices().len(), mesh.indices().len()).unwrap_or(usize::MAX)
    } else if let Some(compound) = shape.as_compound() {
        compound
            .shapes()
            .iter()
            .fold(4096usize, |sum, (_, part)| sum.saturating_add(512).saturating_add(shape_charge(part.as_ref())))
    } else if let Some(poly) = shape.as_convex_polyhedron() {
        poly.points()
            .len()
            .saturating_add(poly.edges().len())
            .saturating_add(poly.faces().len())
            .saturating_mul(256)
            .saturating_add(4096)
    } else {
        4096
    }
}

/// Triangles keep facing outward after the half turn about x (it is a rotation: winding holds).
fn flip_winding(idx: &[[u32; 3]]) -> Vec<[u32; 3]> {
    idx.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoDrive;
    impl Driver3 for NoDrive {
        fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
            vec![Pose3::default(); which.len()]
        }
        fn fields(&mut self, _: f64) -> Vec<Field> {
            Vec::new()
        }
    }

    fn body(shape: Shape3, pos: [f64; 3]) -> Body3Spec {
        Body3Spec {
            kind: BodyKind::Dynamic,
            shape,
            mass: 1.0,
            friction: 0.5,
            restitution: 0.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
            group: 0,
            collides_with: None,
            sensor: false,
            fixed_rotation: false,
            bullet: false,
            activate_at: 0.0,
            start: Pose3 { pos, rot: [0.0, 0.0, 0.0, 1.0] },
        }
    }

    fn spec(bodies: Vec<Body3Spec>, bounds: Bounds3) -> World3Spec {
        World3Spec {
            fix_internal_edges: false,
            start: 0.0,
            step: 1.0 / 240.0,
            gravity: [0.0, -9.80665, 0.0],
            pixels_per_meter: 100.0,
            iterations: 8,
            bounds,
            bodies,
            joints: Vec::new(),
        }
    }

    #[test]
    fn free_fall_follows_half_g_t_squared() {
        let mut w = World3::new(spec(vec![body(Shape3::Sphere(10.0), [0.0, 0.0, 0.0])], Bounds3::None));
        for t in [0.25, 0.5, 1.0] {
            let f = w.frame_at(t, &mut NoDrive);
            let fallen = f.bodies[0].pos[1] / 100.0;
            let want = 0.5 * 9.80665 * t * t;
            // semi-implicit Euler lands within a step's worth of the analytic curve
            assert!((fallen - want).abs() < 9.80665 * t / 240.0 + 1e-9, "t={t}: {fallen} vs {want}");
        }
    }

    #[test]
    fn replay_in_any_order_is_bit_identical() {
        let mk = || {
            let mut b = vec![
                body(Shape3::Box([20.0, 20.0, 20.0]), [0.0, 0.0, 0.0]),
                body(Shape3::Sphere(15.0), [5.0, -60.0, 3.0]),
            ];
            b[1].angular_velocity = [30.0, 45.0, 10.0];
            World3::new(spec(b, Bounds3::Floor { y: 200.0 }))
        };
        let mut a = mk();
        let straight: Vec<Frame3> = (0..=40).map(|k| a.frame_at(k as f64 * 0.1, &mut NoDrive)).collect();
        for budget in [0, 64 << 10, crate::timeline::BUDGET] {
            let mut b = mk().with_checkpoint_budget(budget);
            for k in [40, 3, 27, 0, 15, 40, 8] {
                assert_eq!(b.frame_at(k as f64 * 0.1, &mut NoDrive), straight[k]);
                assert!(b.checkpoint_bytes() <= budget);
            }
        }
    }

    #[test]
    fn stacks_come_to_rest_on_the_floor() {
        let bodies =
            (0..3).map(|k| body(Shape3::Box([25.0, 25.0, 25.0]), [0.0, 100.0 - 60.0 * k as f64, 0.0])).collect();
        let mut w = World3::new(spec(bodies, Bounds3::Floor { y: 200.0 }));
        let f = w.frame_at(4.0, &mut NoDrive);
        // three 50-pixel cubes stacked on the floor at y = 200: centres at 175, 125, 75
        for (k, want) in [175.0, 125.0, 75.0].iter().enumerate() {
            let p = f.bodies[k].pos;
            assert!((p[1] - want).abs() < 2.0 && p[0].abs() < 2.0 && p[2].abs() < 2.0, "{k}: {p:?}");
        }
    }

    fn joint(kind: Joint3Kind, a: usize, b: Option<usize>, anchor: Option<[f64; 3]>, axis: [f64; 3]) -> Joint3Spec {
        Joint3Spec {
            kind,
            a,
            b,
            anchor,
            axis,
            rest_length: None,
            stiffness: None,
            damping: None,
            min: None,
            max: None,
            motor_speed: 0.0,
            max_force: None,
            break_force: None,
        }
    }

    #[test]
    fn a_pinned_pendulum_keeps_its_length() {
        // a ball 150 px to the side of its pin, pushed toward the camera: it swings in 3D
        let mut b = body(Shape3::Sphere(10.0), [150.0, 0.0, 0.0]);
        b.velocity = [0.0, 0.0, -80.0];
        let mut sp = spec(vec![b], Bounds3::None);
        sp.joints.push(joint(Joint3Kind::Pin, 0, None, Some([0.0, 0.0, 0.0]), [0.0, 0.0, 1.0]));
        let mut w = World3::new(sp);
        let mut moved = false;
        for k in 1..=30 {
            let p = w.frame_at(k as f64 * 0.1, &mut NoDrive).bodies[0].pos;
            let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            assert!((r - 150.0).abs() < 1.5, "t={}: length {r}", k as f64 * 0.1);
            moved |= p[2].abs() > 20.0;
        }
        assert!(moved, "the swing leaves the xy plane");
    }

    #[test]
    fn hinges_and_sliders_keep_to_their_axes() {
        // a bar hinged to the world about the scene z axis at its left end
        let bar = body(Shape3::Box([60.0, 10.0, 10.0]), [60.0, 0.0, 0.0]);
        let mut sp = spec(vec![bar], Bounds3::None);
        sp.joints.push(joint(Joint3Kind::Hinge, 0, None, Some([0.0, 0.0, 0.0]), [0.0, 0.0, 1.0]));
        let mut w = World3::new(sp);
        let f = w.frame_at(1.0, &mut NoDrive).bodies[0];
        // rotation only about z: the quaternion has no x or y part, and it has turned
        assert!(f.rot[0].abs() < 1e-6 && f.rot[1].abs() < 1e-6 && f.rot[2].abs() > 0.2, "{:?}", f.rot);
        assert!(f.pos[2].abs() < 1e-6);
        // a block on a slider along x, pushed diagonally: only x changes
        let mut blk = body(Shape3::Box([10.0, 10.0, 10.0]), [0.0, 0.0, 0.0]);
        blk.velocity = [100.0, 0.0, 100.0];
        let mut sp = spec(vec![blk], Bounds3::None);
        sp.gravity = [0.0; 3];
        sp.joints.push(joint(Joint3Kind::Slider, 0, None, Some([0.0, 0.0, 0.0]), [1.0, 0.0, 0.0]));
        let mut w = World3::new(sp);
        let p = w.frame_at(1.0, &mut NoDrive).bodies[0].pos;
        assert!(p[0] > 50.0 && p[1].abs() < 1e-6 && p[2].abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn meshes_collide_and_balls_bounce() {
        // a static triangle-mesh ground at y = 200 catches a falling sphere
        let ground = Body3Spec {
            kind: BodyKind::Static,
            ..body(
                Shape3::TriMesh(
                    vec![[-500.0, 0.0, -500.0], [500.0, 0.0, -500.0], [500.0, 0.0, 500.0], [-500.0, 0.0, 500.0]],
                    vec![[0, 1, 2], [0, 2, 3]],
                ),
                [0.0, 200.0, 0.0],
            )
        };
        let ball = body(Shape3::Sphere(20.0), [0.0, 0.0, 0.0]);
        let mut w = World3::new(spec(vec![ground.clone(), ball], Bounds3::None));
        let p = w.frame_at(3.0, &mut NoDrive).bodies[1].pos;
        assert!((p[1] - 180.0).abs() < 2.0, "{p:?}");
        // a bouncy ball rises again after it lands
        let mut bouncy = body(Shape3::Sphere(20.0), [0.0, 0.0, 0.0]);
        bouncy.restitution = 0.9;
        let mut g2 = ground;
        g2.restitution = 0.9;
        let mut w = World3::new(spec(vec![g2, bouncy], Bounds3::None));
        let ys: Vec<f64> = (0..=60).map(|k| w.frame_at(k as f64 * 0.025, &mut NoDrive).bodies[1].pos[1]).collect();
        let lowest = ys.iter().cloned().fold(f64::MIN, f64::max);
        let k = ys.iter().position(|y| *y == lowest).unwrap();
        assert!(lowest > 170.0 && ys[k..].iter().any(|y| *y < lowest - 40.0), "{ys:?}");
    }

    #[test]
    fn welds_break_under_load() {
        let heavy = |x: f64| {
            let mut b = body(Shape3::Box([20.0, 20.0, 20.0]), [x, 0.0, 0.0]);
            b.mass = 50.0;
            b
        };
        let mut anchor = body(Shape3::Box([20.0, 20.0, 20.0]), [0.0, 0.0, 0.0]);
        anchor.kind = BodyKind::Static;
        let mut sp = spec(vec![anchor, heavy(40.0)], Bounds3::None);
        let mut j = joint(Joint3Kind::Weld, 0, Some(1), None, [0.0, 0.0, 1.0]);
        j.break_force = Some(100.0);
        sp.joints.push(j);
        let mut w = World3::new(sp);
        let f = w.frame_at(1.0, &mut NoDrive);
        assert_eq!(f.broken, vec![true]);
        assert!(f.bodies[1].pos[1] > 100.0, "{:?}", f.bodies[1].pos);
    }
}
