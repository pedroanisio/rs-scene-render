//! sr-sim: rigid bodies, joints, fields, soft bodies and particles, with
//! bit-exact results whatever order times are requested in.

use sr_sim::fields::{Field, FieldKind};
use sr_sim::particles::{Burst, EmitShape, Emitter, EmitterDriver, EmitterSpec, Walls};
use sr_sim::physics::{BodyKind, BodySpec, Bounds, Driver, JointKind, JointSpec, PxPose, Shape, World, WorldSpec};
use sr_sim::soft::{SoftKind, SoftSpec};

struct Still(Vec<Field>);

impl Driver for Still {
    fn kinematic(&mut self, t: f64, which: &[usize]) -> Vec<PxPose> {
        // kinematic bodies slide right at 100 px/s
        which.iter().map(|_| PxPose { x: 100.0 + 100.0 * t, y: 100.0, angle: 0.0 }).collect()
    }
    fn fields(&mut self, _t: f64) -> Vec<Field> {
        self.0.clone()
    }
}

fn body(x: f64, y: f64) -> BodySpec {
    BodySpec {
        kind: BodyKind::Dynamic,
        shape: Shape::Box { w: 100.0, h: 100.0 },
        mass: 1.0,
        friction: 0.5,
        restitution: 0.0,
        linear_damping: 0.0,
        angular_damping: 0.0,
        velocity: [0.0, 0.0],
        angular_velocity: 0.0,
        group: 0,
        collides_with: None,
        sensor: false,
        fixed_rotation: false,
        bullet: false,
        activate_at: 0.0,
        start: PxPose { x, y, angle: 0.0 },
    }
}

fn world(bodies: Vec<BodySpec>, joints: Vec<JointSpec>, softs: Vec<SoftSpec>, bounds: Bounds) -> World {
    World::new(WorldSpec {
        start: 0.0,
        step: 1.0 / 120.0,
        gravity: [0.0, -9.80665],
        pixels_per_meter: 100.0,
        iterations: 8,
        bounds,
        bodies,
        joints,
        softs,
    })
}

#[test]
fn free_fall_and_rest_on_the_floor() {
    let mut w = world(vec![body(200.0, 100.0)], vec![], vec![], Bounds::None);
    let f = w.frame_at(1.0, &mut Still(vec![]));
    let fell = f.bodies[0].y - 100.0;
    assert!((fell - 490.3).abs() < 10.0, "½ g t² ≈ 490 px: {fell}");
    let mut w = world(vec![body(200.0, 100.0)], vec![], vec![], Bounds::Floor { w: 800.0, h: 600.0 });
    let f = w.frame_at(3.0, &mut Still(vec![]));
    assert!(
        (f.bodies[0].y - 550.0).abs() < 3.0 && f.bodies[0].angle.abs() < 1.0,
        "resting on the floor: {:?}",
        f.bodies[0]
    );
}

#[test]
fn any_order_of_times_gives_the_same_state() {
    let mk = || {
        let mut a = body(200.0, 100.0);
        a.angular_velocity = 90.0;
        a.velocity = [60.0, 0.0];
        let mut b = body(260.0, -80.0);
        b.shape = Shape::Circle { r: 40.0 };
        world(vec![a, b], vec![], vec![], Bounds::Frame { w: 800.0, h: 600.0 })
    };
    let mut w = mk();
    let late = w.frame_at(3.5, &mut Still(vec![]));
    let early = w.frame_at(1.2, &mut Still(vec![]));
    let again = w.frame_at(3.5, &mut Still(vec![]));
    assert_eq!(late, again, "replaying from a checkpoint is bit-exact");
    let fresh = mk().frame_at(1.2, &mut Still(vec![]));
    assert_eq!(early, fresh);
    let (steps, checkpoints) = w.progress();
    assert_eq!(steps, 420);
    assert!(checkpoints >= 4, "one checkpoint per simulated second: {checkpoints}");
}

#[test]
fn joints_hold_and_break() {
    // a pendulum hinged 150 px above its centre keeps that distance
    let pin = JointSpec {
        kind: JointKind::Pin,
        a: 0,
        b: None,
        anchor: Some([300.0, 50.0]),
        rest_length: None,
        stiffness: None,
        damping: None,
        min_angle: None,
        max_angle: None,
        axis_angle: 0.0,
        motor_speed: 0.0,
        max_force: None,
        break_force: None,
    };
    let mut b = body(450.0, 50.0);
    b.shape = Shape::Circle { r: 20.0 };
    let mut w = world(vec![b], vec![pin.clone()], vec![], Bounds::None);
    for t in [0.3, 0.9, 1.7] {
        let p = w.frame_at(t, &mut Still(vec![]));
        let d = ((p.bodies[0].x - 300.0).powi(2) + (p.bodies[0].y - 50.0).powi(2)).sqrt();
        assert!((d - 150.0).abs() < 2.0, "t={t}: {d}");
    }
    // a weld with a tiny break force lets go under gravity
    let weld = JointSpec { kind: JointKind::Weld, break_force: Some(0.01), ..pin };
    let mut w = world(vec![body(300.0, 100.0)], vec![weld], vec![], Bounds::None);
    let f = w.frame_at(0.5, &mut Still(vec![]));
    assert!(f.broken[0], "the weld broke");
    assert!(f.bodies[0].y > 150.0, "and the body fell: {}", f.bodies[0].y);
}

#[test]
fn kinematic_bodies_follow_and_fields_push() {
    let mut k = body(100.0, 100.0);
    k.kind = BodyKind::Kinematic;
    let mut w = world(vec![k], vec![], vec![], Bounds::None);
    let f = w.frame_at(1.0, &mut Still(vec![]));
    assert!((f.bodies[0].x - 200.0).abs() < 1.5 && (f.bodies[0].y - 100.0).abs() < 1e-6, "{:?}", f.bodies[0]);
    // a directional field of 200 px/s² to the right
    let field = Field {
        kind: FieldKind::Directional,
        pos: [0.0; 2],
        force: [200.0, 0.0],
        strength: 0.0,
        falloff: 0.0,
        radius: None,
        scale: 1.0,
        path: vec![],
        seed: 1,
        bodies: true,
        particles: true,
        z: 0.0,
        force_z: 0.0,
    };
    let mut w = world(vec![body(100.0, 100.0)], vec![], vec![], Bounds::None);
    let f = w.frame_at(1.0, &mut Still(vec![field]));
    assert!((f.bodies[0].x - 200.0).abs() < 5.0, "½ a t² = 100 px: {}", f.bodies[0].x);
    // radial repels, vortex turns
    let radial = Field {
        kind: FieldKind::Radial,
        pos: [0.0, 0.0],
        force: [0.0; 2],
        strength: 50.0,
        falloff: 0.0,
        radius: None,
        scale: 1.0,
        path: vec![],
        seed: 1,
        bodies: true,
        particles: true,
        z: 0.0,
        force_z: 0.0,
    };
    let a = radial.accel([10.0, 0.0], [0.0; 2], 0.0);
    assert!(a[0] > 49.0 && a[1].abs() < 1e-9);
    let vortex = Field { kind: FieldKind::Vortex, ..radial };
    let a = vortex.accel([10.0, 0.0], [0.0; 2], 0.0);
    assert!(a[0].abs() < 1e-9 && a[1] > 49.0);
}

fn lattice(x: f64, y: f64, w: f64, h: f64, rows: usize, cols: usize) -> Vec<[f64; 2]> {
    let mut v = Vec::new();
    for i in 0..rows {
        for j in 0..cols {
            v.push([x + w * j as f64 / (cols - 1) as f64, y + h * i as f64 / (rows - 1) as f64]);
        }
    }
    v
}

#[test]
fn soft_bodies_keep_shape_and_hang_from_pins() {
    let jelly = SoftSpec {
        kind: SoftKind::Jelly,
        rows: 4,
        cols: 4,
        rest: lattice(300.0, 200.0, 120.0, 120.0, 4, 4),
        mass: 1.0,
        stiffness: 400.0,
        damping: 0.3,
        pressure: 50.0,
        pinned: vec![false; 16],
        self_collision: false,
    };
    let mut w = world(vec![], vec![], vec![jelly], Bounds::Floor { w: 800.0, h: 600.0 });
    let f = w.frame_at(3.0, &mut Still(vec![]));
    let pts = &f.softs[0];
    let bottom = pts.iter().map(|p| p[1]).fold(f64::MIN, f64::max);
    let top = pts.iter().map(|p| p[1]).fold(f64::MAX, f64::min);
    assert!((bottom - 600.0).abs() < 5.0, "rests on the floor: {bottom}");
    assert!(bottom - top > 70.0, "keeps most of its height: {}", bottom - top);
    // cloth pinned along its top row sags below it
    let pinned: Vec<bool> = (0..25).map(|k| k < 5).collect();
    let cloth = SoftSpec {
        kind: SoftKind::Cloth,
        rows: 5,
        cols: 5,
        rest: lattice(200.0, 100.0, 200.0, 200.0, 5, 5),
        mass: 0.5,
        stiffness: 200.0,
        damping: 0.2,
        pressure: 0.0,
        pinned,
        self_collision: true,
    };
    let mut w = world(vec![], vec![], vec![cloth], Bounds::None);
    let f = w.frame_at(2.0, &mut Still(vec![]));
    assert_eq!(f.softs[0][2], [300.0, 100.0], "pins hold");
    assert!(f.softs[0][22][1] > 300.0, "the bottom stretches under gravity: {}", f.softs[0][22][1]);
    assert!(sr_sim::soft::substeps(1.0, 20.0, 16, 1.0 / 120.0) == 1);
    assert!(sr_sim::soft::substeps(1.0e-6, 1.0e6, 256, 1.0 / 120.0) > 4096);
}

#[test]
fn cloth_shears_and_bends_more_easily_than_jelly() {
    // cloth shear springs are 0.15 k and bend springs 0.02 k; a cantilever pinned
    // along its left column droops further as cloth than as jelly with the same stiffness
    let droop = |kind: SoftKind| {
        let spec = SoftSpec {
            kind,
            rows: 5,
            cols: 5,
            rest: lattice(200.0, 100.0, 200.0, 200.0, 5, 5),
            mass: 0.5,
            stiffness: 300.0,
            damping: 0.3,
            pressure: 0.0,
            pinned: (0..25).map(|k| k % 5 == 0).collect(),
            self_collision: false,
        };
        let mut w = world(vec![], vec![], vec![spec], Bounds::None);
        let f = w.frame_at(2.0, &mut Still(vec![]));
        f.softs[0][4][1] - 100.0
    };
    let (cloth, jelly) = (droop(SoftKind::Cloth), droop(SoftKind::Jelly));
    assert!(cloth > jelly * 2.0 + 2.0, "cloth droops {cloth} px, jelly {jelly} px");
}

struct Here;

impl EmitterDriver for Here {
    fn origin(&mut self, _t: f64) -> [f64; 6] {
        [1.0, 0.0, 0.0, 1.0, 400.0, 300.0]
    }
    fn rate(&mut self, _t: f64) -> f64 {
        100.0
    }
    fn fields(&mut self, _t: f64) -> Vec<Field> {
        Vec::new()
    }
    fn hit(&mut self, _p: [f64; 2]) -> Option<([f64; 2], [f64; 2])> {
        None
    }
}

fn spec() -> EmitterSpec {
    EmitterSpec {
        seed: 7,
        start: 0.0,
        end: None,
        preroll: 0.0,
        step: 1.0 / 120.0,
        lifetime: 10.0,
        lifetime_variance: 0.0,
        speed: 100.0,
        speed_variance: 20.0,
        direction: -90.0,
        spread: 30.0,
        gravity: [0.0, 0.0],
        drag: 0.0,
        turbulence: 0.0,
        turbulence_scale: 100.0,
        rotation0: 0.0,
        rotation_variance: 0.0,
        angular_velocity: 0.0,
        angular_velocity_variance: 0.0,
        size_variance: 0.0,
        max_particles: 10_000,
        shape: EmitShape::Point,
        bursts: vec![],
        collide: false,
        bounce: 0.3,
        walls: Walls::None,
    }
}

#[test]
fn particles_emit_move_and_replay_exactly() {
    let mut e = Emitter::new(spec());
    let n = e.at(1.0, &mut Here).len();
    assert_eq!(n, 100, "100 per second");
    let p = e.at(1.0, &mut Here).clone();
    let up = p.y.iter().zip(&p.age).all(|(y, a)| *y <= 300.0 - 60.0 * a + 1e-6);
    assert!(up, "they travel up at 80–120 px/s");
    let late = e.at(3.3, &mut Here).clone();
    let _ = e.at(0.4, &mut Here);
    assert_eq!(e.at(3.3, &mut Here), &late, "replay is bit-exact");
    assert_eq!(Emitter::new(spec()).at(0.4, &mut Here).len(), 40);
    // bursts, the cap and a bouncing floor
    let mut s = spec();
    s.bursts = vec![Burst { time: 0.5, count: 30, repeat: 1, interval: 1.0 }];
    s.max_particles = 150;
    let mut e = Emitter::new(s);
    assert_eq!(e.at(0.51, &mut Here).len(), 50 + 30, "61 steps emit 50, plus the burst");
    assert_eq!(e.at(2.0, &mut Here).len(), 150, "capped");
    let mut s = spec();
    s.direction = 90.0;
    s.walls = Walls::Floor { h: 350.0 };
    let mut e = Emitter::new(s);
    assert!(e.at(3.0, &mut Here).y.iter().all(|y| *y <= 350.0 + 1e-9), "nothing falls through the floor");
    // preroll fills the system before it starts
    let mut s = spec();
    s.start = 1.0;
    s.preroll = 2.0;
    let mut e = Emitter::new(s);
    assert_eq!(e.at(1.0, &mut Here).len(), 200);
}
