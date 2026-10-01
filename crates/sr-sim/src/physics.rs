//! Rigid bodies, joints and soft bodies on Rapier 2D (f64, enhanced
//! determinism), stepped at a fixed rate from a start time with a checkpoint
//! every simulated second.
//!
//! Inputs and outputs use document pixels with y down and angles in degrees
//! clockwise on screen; inside, Rapier works in metres with y up.

use std::collections::BTreeMap;

use rapier2d_f64::prelude::*;

use crate::fields::{self, Field};
use crate::soft::{SoftSpec, SoftState};

/// A pose in document space: position (px) and rotation (degrees, clockwise on screen).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PxPose {
    pub x: f64,
    pub y: f64,
    pub angle: f64,
}

/// Body motion type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    Static,
    Kinematic,
    Dynamic,
}

/// Collision shape in pixels, relative to the body's centre (screen axes).
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Box {
        w: f64,
        h: f64,
    },
    Circle {
        r: f64,
    },
    /// Capsule fitting a w × h box, along its longer side.
    Capsule {
        w: f64,
        h: f64,
    },
    Convex(Vec<[f64; 2]>),
    /// Convex parts of a concave outline.
    Compound(Vec<Vec<[f64; 2]>>),
    /// A closed outline decomposed into convex parts by Rapier.
    Outline(Vec<[f64; 2]>),
}

/// A rigid body.
#[derive(Clone, Debug)]
pub struct BodySpec {
    pub kind: BodyKind,
    pub shape: Shape,
    pub mass: f64,
    pub friction: f64,
    pub restitution: f64,
    pub linear_damping: f64,
    pub angular_damping: f64,
    /// px/s (y down).
    pub velocity: [f64; 2],
    /// Degrees per second, clockwise.
    pub angular_velocity: f64,
    pub group: u32,
    /// Groups it collides with; all when `None`.
    pub collides_with: Option<Vec<u32>>,
    pub sensor: bool,
    pub fixed_rotation: bool,
    pub bullet: bool,
    /// Follows its animation until this time, then simulates.
    pub activate_at: f64,
    /// Pose at the simulation start.
    pub start: PxPose,
}

/// Joint types of the schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointKind {
    Spring,
    Distance,
    Pin,
    Rope,
    Hinge,
    Slider,
    Weld,
    Motor,
}

/// A constraint between bodies `a` and `b` (or `a` and the world for pins).
#[derive(Clone, Debug)]
pub struct JointSpec {
    pub kind: JointKind,
    pub a: usize,
    pub b: Option<usize>,
    /// World anchor at the start (px); defaults to b's centre (or a's for pins).
    pub anchor: Option<[f64; 2]>,
    /// px.
    pub rest_length: Option<f64>,
    pub stiffness: Option<f64>,
    pub damping: Option<f64>,
    /// Degrees.
    pub min_angle: Option<f64>,
    pub max_angle: Option<f64>,
    pub axis_angle: f64,
    /// Degrees per second.
    pub motor_speed: f64,
    pub max_force: Option<f64>,
    pub break_force: Option<f64>,
}

/// World boundaries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bounds {
    None,
    /// Walls around a w × h frame.
    Frame {
        w: f64,
        h: f64,
    },
    /// A floor along the bottom of a frame of height h.
    Floor {
        w: f64,
        h: f64,
    },
}

/// Everything a world needs.
#[derive(Clone, Debug)]
pub struct WorldSpec {
    pub start: f64,
    pub step: f64,
    /// m/s², y up.
    pub gravity: [f64; 2],
    pub pixels_per_meter: f64,
    pub iterations: usize,
    pub bounds: Bounds,
    pub bodies: Vec<BodySpec>,
    pub joints: Vec<JointSpec>,
    /// Soft bodies with their rest lattice in pixels.
    pub softs: Vec<SoftSpec>,
}

/// Animated inputs, asked for at simulation-step times.
pub trait Driver {
    /// Poses of the bodies that follow animation at `t` (index, pose).
    fn kinematic(&mut self, t: f64, which: &[usize]) -> Vec<PxPose>;
    /// Force fields at `t`.
    fn fields(&mut self, t: f64) -> Vec<Field>;
}

/// Simulated state at one time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub bodies: Vec<PxPose>,
    /// Soft lattices in pixels, row-major.
    pub softs: Vec<Vec<[f64; 2]>>,
    /// Constraints removed by `breakForce`.
    pub broken: Vec<bool>,
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
    softs: Vec<SoftState>,
}

/// A deterministic 2D world.
pub struct World {
    spec: WorldSpec,
    params: IntegrationParameters,
    pipeline: PhysicsPipeline,
    state: State,
    checkpoints: BTreeMap<u64, State>,
    steps_per_checkpoint: u64,
}

fn v(x: f64, y: f64) -> Vector {
    Vector::new(x, y)
}

/// The nearest surface point and outward normal (metres, y up) of the first enabled,
/// non-sensor collider containing `q`.
fn surface(colliders: &ColliderSet, q: Vector) -> Option<([f64; 2], [f64; 2])> {
    for (_, c) in colliders.iter() {
        if c.is_sensor() || !c.is_enabled() {
            continue;
        }
        let proj = c.shape().project_point(c.position(), q, false);
        if !proj.is_inside {
            continue;
        }
        let d = [proj.point.x - q.x, proj.point.y - q.y];
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-12);
        return Some(([proj.point.x, proj.point.y], [d[0] / l, d[1] / l]));
    }
    None
}

impl World {
    /// Metres (y up) of a pixel point.
    pub fn to_m(&self, p: [f64; 2]) -> [f64; 2] {
        [p[0] / self.spec.pixels_per_meter, -p[1] / self.spec.pixels_per_meter]
    }

    /// Pixels (y down) of a metre point.
    pub fn to_px(&self, p: [f64; 2]) -> [f64; 2] {
        [p[0] * self.spec.pixels_per_meter, -p[1] * self.spec.pixels_per_meter]
    }

    pub fn new(spec: WorldSpec) -> World {
        let ppm = spec.pixels_per_meter.max(1e-9);
        let m = |p: [f64; 2]| v(p[0] / ppm, -p[1] / ppm);
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
            softs: Vec::new(),
        };
        for b in &spec.bodies {
            let follows = b.kind == BodyKind::Kinematic || (b.kind == BodyKind::Dynamic && b.activate_at > spec.start);
            let builder = match (b.kind, follows) {
                (BodyKind::Static, _) => RigidBodyBuilder::fixed(),
                (_, true) => RigidBodyBuilder::kinematic_position_based(),
                _ => RigidBodyBuilder::dynamic(),
            };
            let mut rb = builder
                .translation(m([b.start.x, b.start.y]))
                .rotation(-b.start.angle.to_radians())
                .linear_damping(b.linear_damping)
                .angular_damping(b.angular_damping)
                .ccd_enabled(b.bullet)
                .build();
            if b.fixed_rotation {
                rb.lock_rotations(true, false);
            }
            if !follows && b.kind == BodyKind::Dynamic {
                rb.set_linvel(v(b.velocity[0] / ppm, -b.velocity[1] / ppm), true);
                rb.set_angvel(-b.angular_velocity.to_radians(), true);
            }
            let h = st.bodies.insert(rb);
            let pts = |ps: &[[f64; 2]]| -> Vec<Vector> { ps.iter().map(|p| v(p[0] / ppm, -p[1] / ppm)).collect() };
            let mut parts: Vec<ColliderBuilder> = Vec::new();
            match &b.shape {
                Shape::Box { w, h } => {
                    parts.push(ColliderBuilder::cuboid((w * 0.5 / ppm).max(1e-4), (h * 0.5 / ppm).max(1e-4)))
                }
                Shape::Circle { r } => parts.push(ColliderBuilder::ball((r / ppm).max(1e-4))),
                Shape::Capsule { w, h } => {
                    let (long, short) = (w.max(*h) / ppm, w.min(*h) / ppm);
                    let r = (short * 0.5).max(1e-4);
                    let half = (long * 0.5 - r).max(0.0);
                    let c =
                        if h >= w { ColliderBuilder::capsule_y(half, r) } else { ColliderBuilder::capsule_x(half, r) };
                    parts.push(c);
                }
                Shape::Convex(ps) => {
                    if let Some(c) = ColliderBuilder::convex_hull(&pts(ps)) {
                        parts.push(c);
                    }
                }
                Shape::Compound(list) => {
                    for ps in list {
                        if let Some(c) = ColliderBuilder::convex_hull(&pts(ps)) {
                            parts.push(c);
                        }
                    }
                }
                Shape::Outline(ps) => {
                    if ps.len() >= 3 {
                        let vs = pts(ps);
                        let idx: Vec<[u32; 2]> =
                            (0..vs.len()).map(|k| [k as u32, ((k + 1) % vs.len()) as u32]).collect();
                        parts.push(ColliderBuilder::convex_decomposition(&vs, &idx));
                    }
                }
            }
            if parts.is_empty() {
                parts.push(ColliderBuilder::ball(0.01));
            }
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
            let n = parts.len() as f64;
            for c in parts {
                let c = c
                    .friction(b.friction)
                    .restitution(b.restitution)
                    .sensor(b.sensor)
                    .collision_groups(groups)
                    .mass(b.mass.max(1e-6) / n)
                    .build();
                st.colliders.insert_with_parent(c, h, &mut st.bodies);
            }
            st.handles.push(h);
            st.active.push(!follows);
        }
        // bounds as fixed walls
        let wall = |st: &mut State, cx: f64, cy: f64, hx: f64, hy: f64| {
            let h = st.bodies.insert(RigidBodyBuilder::fixed().translation(m([cx, cy])).build());
            st.colliders.insert_with_parent(
                // coefficient 0 under the Max rule: a contact takes the body's own friction and restitution
                ColliderBuilder::cuboid(hx / ppm, hy / ppm)
                    .friction(0.0)
                    .friction_combine_rule(CoefficientCombineRule::Max)
                    .restitution(0.0)
                    .restitution_combine_rule(CoefficientCombineRule::Max)
                    .build(),
                h,
                &mut st.bodies,
            );
        };
        match spec.bounds {
            Bounds::None => {}
            Bounds::Floor { w, h } => wall(&mut st, w * 0.5, h + 500.0, w * 50.0, 500.0),
            Bounds::Frame { w, h } => {
                wall(&mut st, w * 0.5, h + 500.0, w * 2.0, 500.0);
                wall(&mut st, w * 0.5, -500.0, w * 2.0, 500.0);
                wall(&mut st, -500.0, h * 0.5, 500.0, h * 2.0);
                wall(&mut st, w + 500.0, h * 0.5, 500.0, h * 2.0);
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
            let pa = st.bodies[ha].translation();
            let pb = st.bodies[hb].translation();
            let anchor_px = j.anchor.unwrap_or_else(|| match j.b {
                Some(b) => [spec.bodies[b].start.x, spec.bodies[b].start.y],
                None => [spec.bodies[j.a].start.x, spec.bodies[j.a].start.y],
            });
            let anchor = m(anchor_px);
            // local anchors: world anchor in each body's frame
            let local = |body: &RigidBody, w: Vector| body.position().inverse_transform_point(w);
            let (la, lb) = (local(&st.bodies[ha], anchor), local(&st.bodies[hb], anchor));
            let centre_dist = (pa - pb).length();
            let rest = j.rest_length.map(|r| r / ppm).unwrap_or(centre_dist);
            let mass = spec.bodies.get(j.a).map(|b| b.mass).unwrap_or(1.0).max(1e-3);
            let data: GenericJoint = match j.kind {
                JointKind::Spring => {
                    SpringJointBuilder::new(rest, j.stiffness.unwrap_or(10.0 * mass), j.damping.unwrap_or(0.5 * mass))
                        .local_anchor1(Vector::ZERO)
                        .local_anchor2(Vector::ZERO)
                        .build()
                        .into()
                }
                // a distance joint holds its length: a stiff spring
                JointKind::Distance => SpringJointBuilder::new(
                    rest,
                    j.stiffness.unwrap_or(1.0e4 * mass),
                    j.damping.unwrap_or(1.0e2 * mass),
                )
                .local_anchor1(Vector::ZERO)
                .local_anchor2(Vector::ZERO)
                .build()
                .into(),
                JointKind::Rope => {
                    RopeJointBuilder::new(rest).local_anchor1(Vector::ZERO).local_anchor2(Vector::ZERO).build().into()
                }
                JointKind::Pin => match j.stiffness {
                    Some(k) => SpringJointBuilder::new(0.0, k, j.damping.unwrap_or(0.5 * mass))
                        .local_anchor1(la)
                        .local_anchor2(lb)
                        .build()
                        .into(),
                    None => RevoluteJointBuilder::new().local_anchor1(la).local_anchor2(lb).build().into(),
                },
                JointKind::Hinge | JointKind::Motor => {
                    let mut r = RevoluteJointBuilder::new().local_anchor1(la).local_anchor2(lb);
                    if let (Some(lo), Some(hi)) = (j.min_angle, j.max_angle) {
                        r = r.limits([(-hi).to_radians(), (-lo).to_radians()]);
                    }
                    if j.kind == JointKind::Motor {
                        r = r.motor_velocity(-j.motor_speed.to_radians(), 1.0e3 * mass);
                        if let Some(f) = j.max_force {
                            r = r.motor_max_force(f);
                        }
                    }
                    r.build().into()
                }
                JointKind::Slider => {
                    let ang = -j.axis_angle.to_radians();
                    let mut s = PrismaticJointBuilder::new(v(ang.cos(), ang.sin())).local_anchor1(la).local_anchor2(lb);
                    if let (Some(lo), Some(hi)) = (j.min_angle, j.max_angle) {
                        // on a slider the limits are travel in pixels
                        s = s.limits([lo / ppm, hi / ppm]);
                    }
                    s.build().into()
                }
                JointKind::Weld => {
                    let rel = st.bodies[ha].position().inverse() * *st.bodies[hb].position();
                    FixedJointBuilder::new().local_frame1(rel).local_frame2(Pose::IDENTITY).build().into()
                }
            };
            let h = st.joints.insert(ha, hb, data, true);
            st.joint_handles.push(Some(h));
        }
        for s in &spec.softs {
            let mut sm = s.clone();
            sm.rest = s.rest.iter().map(|p| [p[0] / ppm, -p[1] / ppm]).collect();
            st.softs.push(SoftState::new(&sm));
        }
        let steps_per_checkpoint = ((1.0 / spec.step.max(1e-6)).round() as u64).max(1);
        let mut w = World {
            spec,
            params,
            pipeline: PhysicsPipeline::new(),
            state: st,
            checkpoints: BTreeMap::new(),
            steps_per_checkpoint,
        };
        w.checkpoints.insert(0, w.state.clone());
        w
    }

    /// Soft body specs in metres (as simulated).
    fn soft_m(&self, k: usize) -> SoftSpec {
        let ppm = self.spec.pixels_per_meter.max(1e-9);
        let mut s = self.spec.softs[k].clone();
        s.rest = s.rest.iter().map(|p| [p[0] / ppm, -p[1] / ppm]).collect();
        s
    }

    fn step_once(&mut self, driver: &mut dyn Driver) {
        let st = &mut self.state;
        let t = self.spec.start + st.step as f64 * self.spec.step;
        let ppm = self.spec.pixels_per_meter.max(1e-9);
        // activation and animated poses
        let mut follow = Vec::new();
        for (k, b) in self.spec.bodies.iter().enumerate() {
            if b.kind == BodyKind::Dynamic && !st.active[k] && t >= b.activate_at {
                let body = &mut st.bodies[st.handles[k]];
                body.set_body_type(RigidBodyType::Dynamic, true);
                body.set_linvel(v(b.velocity[0] / ppm, -b.velocity[1] / ppm), true);
                body.set_angvel(-b.angular_velocity.to_radians(), true);
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
                body.set_next_kinematic_translation(v(p.x / ppm, -p.y / ppm));
                body.set_next_kinematic_rotation(Rotation::new(-p.angle.to_radians()));
            }
        }
        // force fields on dynamic bodies
        let fields = driver.fields(t);
        for (k, _) in self.spec.bodies.iter().enumerate() {
            let body = &mut st.bodies[st.handles[k]];
            body.reset_forces(false);
            if !body.is_dynamic() || fields.is_empty() {
                continue;
            }
            let p = body.translation();
            let lv = body.linvel();
            let a = fields::total(&fields, [p.x * ppm, -p.y * ppm], [lv.x * ppm, -lv.y * ppm], t, false);
            let mass = body.mass();
            body.add_force(v(a[0] / ppm * mass, -a[1] / ppm * mass), true);
        }
        self.pipeline.step(
            v(self.spec.gravity[0], self.spec.gravity[1]),
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
        // breakable constraints
        for (k, j) in self.spec.joints.iter().enumerate() {
            let (Some(limit), Some(h)) = (j.break_force, st.joint_handles[k]) else { continue };
            let Some(joint) = st.joints.get(h) else { continue };
            let imp = joint.impulses;
            let f = (imp[0] * imp[0] + imp[1] * imp[1]).sqrt() / self.spec.step;
            if f > limit {
                st.joints.remove(h, true);
                st.joint_handles[k] = None;
            }
        }
        // soft bodies against the rigid world and the bounds
        if !st.softs.is_empty() {
            let g = self.spec.gravity;
            let bounds = self.spec.bounds;
            let colliders = &st.colliders;
            let collide = |p: [f64; 2]| -> Option<([f64; 2], [f64; 2])> {
                // bounds first (metres, y up; the frame spans y ∈ [−h, 0])
                match bounds {
                    Bounds::Floor { h, .. } | Bounds::Frame { h, .. } if p[1] < -h / ppm => {
                        return Some(([p[0], -h / ppm], [0.0, 1.0]))
                    }
                    _ => {}
                }
                if let Bounds::Frame { w, .. } = bounds {
                    if p[0] < 0.0 {
                        return Some(([0.0, p[1]], [1.0, 0.0]));
                    }
                    if p[0] > w / ppm {
                        return Some(([w / ppm, p[1]], [-1.0, 0.0]));
                    }
                    if p[1] > 0.0 {
                        return Some(([p[0], 0.0], [0.0, -1.0]));
                    }
                }
                // inside a solid: move to the surface along the shortest way out
                let (q, n) = surface(colliders, v(p[0], p[1]))?;
                Some(([q[0] + n[0] * 1e-4, q[1] + n[1] * 1e-4], n))
            };
            for k in 0..st.softs.len() {
                let spec = {
                    let mut s = self.spec.softs[k].clone();
                    s.rest = s.rest.iter().map(|p| [p[0] / ppm, -p[1] / ppm]).collect();
                    s
                };
                let accel = |p: [f64; 2], vel: [f64; 2]| -> [f64; 2] {
                    let a = fields::total(&fields, [p[0] * ppm, -p[1] * ppm], [vel[0] * ppm, -vel[1] * ppm], t, false);
                    [g[0] + a[0] / ppm, g[1] - a[1] / ppm]
                };
                st.softs[k].step(&spec, self.spec.step, &accel, &collide);
            }
        }
        st.step += 1;
        if st.step % self.steps_per_checkpoint == 0 && !self.checkpoints.contains_key(&st.step) {
            self.checkpoints.insert(st.step, st.clone());
        }
    }

    /// Steps before `t` (none before the start).
    pub fn step_index(&self, t: f64) -> u64 {
        if t <= self.spec.start {
            0
        } else {
            ((t - self.spec.start) / self.spec.step + 1e-9).floor() as u64
        }
    }

    /// The simulated state at `t`, replaying from the nearest checkpoint at or before it.
    pub fn frame_at(&mut self, t: f64, driver: &mut dyn Driver) -> Frame {
        let target = self.step_index(t);
        if self.state.step > target || target - self.state.step > self.steps_per_checkpoint {
            if let Some((_, cp)) = self.checkpoints.range(..=target).next_back() {
                if cp.step > self.state.step || self.state.step > target {
                    self.state = cp.clone();
                }
            }
        }
        while self.state.step < target {
            self.step_once(driver);
        }
        self.snapshot()
    }

    fn snapshot(&self) -> Frame {
        let ppm = self.spec.pixels_per_meter.max(1e-9);
        let st = &self.state;
        let bodies = st
            .handles
            .iter()
            .map(|h| {
                let b = &st.bodies[*h];
                let p = b.translation();
                PxPose { x: p.x * ppm, y: -p.y * ppm, angle: -b.rotation().angle().to_degrees() }
            })
            .collect();
        let softs = st
            .softs
            .iter()
            .enumerate()
            .map(|(k, s)| s.lattice(&self.soft_m(k)).iter().map(|p| [p[0] * ppm, -p[1] * ppm]).collect())
            .collect();
        Frame { bodies, softs, broken: st.joint_handles.iter().map(|h| h.is_none()).collect() }
    }

    /// Whether a pixel point lies inside a (non-sensor) collider at the current state, with the
    /// outward normal and the nearest surface point (pixels) when it does.
    pub fn hit(&self, p: [f64; 2]) -> Option<([f64; 2], [f64; 2])> {
        let ppm = self.spec.pixels_per_meter.max(1e-9);
        let (q, n) = surface(&self.state.colliders, v(p[0] / ppm, -p[1] / ppm))?;
        Some(([q[0] * ppm, -q[1] * ppm], [n[0], -n[1]]))
    }

    /// Steps simulated so far and checkpoints held (tests, statistics).
    pub fn progress(&self) -> (u64, usize) {
        (self.state.step, self.checkpoints.len())
    }
}
