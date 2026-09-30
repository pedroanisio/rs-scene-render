//! Rigid bodies and joints in 3D on Rapier 3D (f64, enhanced determinism),
//! stepped at a fixed rate from a start time with a checkpoint every simulated
//! second, like the 2D world ([`crate::physics`]), which stays separate.
//!
//! Inputs and outputs use scene units (pixels) in scene axes (x right, y down,
//! z away from the camera), rotations as quaternions in those axes. Inside,
//! Rapier works in metres in a right-handed frame with y up and z toward the
//! camera: a scene point (x, y, z) is (x, −y, −z) / pixelsPerMeter.

use std::collections::BTreeMap;

use rapier3d_f64::prelude::*;

use crate::fields::{self, Field};

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
}

/// Animated inputs, asked for at simulation-step times.
pub trait Driver3 {
    /// Poses of the bodies that follow animation at `t`.
    fn kinematic(&mut self, t: f64, which: &[usize]) -> Vec<Pose3>;
    /// Force fields at `t` (pixel space, scene axes).
    fn fields(&mut self, t: f64) -> Vec<Field>;
}

/// Simulated state at one time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame3 {
    pub bodies: Vec<Pose3>,
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
}

/// A deterministic 3D world.
pub struct World3 {
    spec: World3Spec,
    params: IntegrationParameters,
    pipeline: PhysicsPipeline,
    state: State,
    checkpoints: BTreeMap<u64, State>,
    steps_per_checkpoint: u64,
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
                Shape3::TriMesh(ps, idx) => ColliderBuilder::trimesh(pts(ps), flip_winding(idx)).ok(),
                Shape3::Decomposition(ps, idx) => {
                    Some(ColliderBuilder::convex_decomposition(&pts(ps), &flip_winding(idx)))
                }
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

    fn step_once(&mut self, driver: &mut dyn Driver3) {
        let st = &mut self.state;
        let t = self.spec.start + st.step as f64 * self.spec.step;
        let ppm = self.spec.pixels_per_meter.max(1e-9);
        let mut follow = Vec::new();
        for (k, b) in self.spec.bodies.iter().enumerate() {
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
            if !body.is_dynamic() || fields.is_empty() {
                continue;
            }
            let p = flip(body.translation().to_array()).map(|c| c * ppm);
            let lv = flip(body.linvel().to_array()).map(|c| c * ppm);
            let a = fields::total3(&fields, p, lv, t);
            let mass = body.mass();
            body.add_force(vec3(flip(a).map(|c| c / ppm * mass)), true);
        }
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
    pub fn frame_at(&mut self, t: f64, driver: &mut dyn Driver3) -> Frame3 {
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
        Frame3 { bodies, broken: st.joint_handles.iter().map(|h| h.is_none()).collect() }
    }

    /// Steps simulated so far and checkpoints held.
    pub fn progress(&self) -> (u64, usize) {
        (self.state.step, self.checkpoints.len())
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
        let mut b = mk();
        for k in [40, 3, 27, 0, 15, 40, 8] {
            assert_eq!(b.frame_at(k as f64 * 0.1, &mut NoDrive), straight[k]);
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
