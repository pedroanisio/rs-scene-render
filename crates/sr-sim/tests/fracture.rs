use sr_sim::{fields::Field, physics3d::*};

struct Drive;
impl Driver3 for Drive {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
}
fn body(half: [f64; 3], mass: f64) -> Body3Spec {
    Body3Spec {
        kind: BodyKind::Dynamic,
        shape: Shape3::Box(half),
        mass,
        friction: 0.,
        restitution: 0.,
        linear_damping: 0.,
        angular_damping: 0.,
        velocity: [3., 0., 0.],
        angular_velocity: [0., 0., 90.],
        group: 0,
        collides_with: Some(vec![]),
        sensor: false,
        fixed_rotation: false,
        bullet: false,
        activate_at: 0.,
        start: Pose3::default(),
    }
}
fn spec() -> World3Spec {
    World3Spec {
        start: 0.,
        step: 0.01,
        gravity: [0.; 3],
        pixels_per_meter: 2.,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies: vec![body([2., 1., 1.], 4.), body([1.; 3], 2.), body([1.; 3], 2.)],
    }
}
fn event(at: f64) -> Fracture3 {
    Fracture3 {
        source: 0,
        at,
        radial_impulse: 0.,
        fragments: vec![
            Fragment3 { body: 1, offset: [-1., 0., 0.], impulse: [0.; 3] },
            Fragment3 { body: 2, offset: [1., 0., 0.], impulse: [0.; 3] },
        ],
    }
}

#[test]
fn radial_impulse_uses_release_orientation_and_mass_fraction() {
    let mut s = spec();
    for b in &mut s.bodies {
        b.velocity = [0.; 3];
    }
    let mut e = event(0.5);
    e.radial_impulse = 8.;
    let mut w = World3::new(s.clone()).with_fractures(vec![e]).unwrap();
    let mut baseline = World3::new(s).with_fractures(vec![event(0.5)]).unwrap();
    let f = w.frame_at(0.5, &mut Drive);
    let zero = baseline.frame_at(0.5, &mut Drive);
    for k in [1, 2] {
        let d: [f64; 3] = std::array::from_fn(|i| f.bodies[k].pos[i] - f.bodies[0].pos[i]);
        let len = d.iter().map(|v| v * v).sum::<f64>().sqrt();
        for (i, value) in d.iter().enumerate() {
            close(f.velocities[k].linear[i] - zero.velocities[k].linear[i], 2. * value / len);
        }
    }
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}

#[test]
fn fracture_activates_on_boundary_with_source_pose_velocity_and_spin() {
    let mut w = World3::new(spec()).with_fractures(vec![event(0.105)]).unwrap();
    let before = w.frame_at(0.10, &mut Drive);
    assert_eq!(before.enabled, [true, false, false]);
    assert_eq!(before.fractured, [false]);
    let at = w.frame_at(0.11, &mut Drive);
    assert!(at.errors.is_empty(), "{:?}", at.errors);
    assert_eq!(at.enabled, [false, true, true]);
    assert_eq!(at.fractured, [true]);
    let q = at.bodies[0].rot;
    let c = 1. - 2. * q[2] * q[2];
    let s = 2. * q[2] * q[3];
    for (i, sign) in [(1, -1.), (2, 1.)] {
        close(at.bodies[i].pos[0], at.bodies[0].pos[0] + sign * c);
        close(at.bodies[i].pos[1], at.bodies[0].pos[1] + sign * s);
        assert_eq!(at.bodies[i].rot, at.bodies[0].rot);
        close(at.velocities[i].linear[0], 3. - std::f64::consts::FRAC_PI_2 * sign * s);
        close(at.velocities[i].linear[1], std::f64::consts::FRAC_PI_2 * sign * c);
        close(at.velocities[i].angular[2], 90.);
    }
    assert_eq!(at, w.frame_at(0.11, &mut Drive), "sampling must not reapply fracture");
    let after = w.frame_at(0.2, &mut Drive);
    close((after.bodies[1].pos[0] + after.bodies[2].pos[0]) * 0.5, 0.6);
    close((after.bodies[1].pos[1] + after.bodies[2].pos[1]) * 0.5, 0.);
}

#[test]
fn fracture_impulse_uses_scene_units_once_and_replays_across_checkpoints() {
    let mut e = event(1.);
    e.fragments[0].impulse = [4., 0., 0.];
    e.fragments[1].impulse = [-4., 0., 0.];
    let mut s = spec();
    for b in &mut s.bodies {
        b.angular_velocity = [0.; 3];
    }
    let mut w = World3::new(s).with_fractures(vec![e]).unwrap();
    let first = w.frame_at(1., &mut Drive);
    close(first.velocities[1].linear[0], 5.);
    close(first.velocities[2].linear[0], 1.);
    let end = w.frame_at(4., &mut Drive);
    for t in [0., 0.99, 1., 2., 0.5] {
        let f = w.frame_at(t, &mut Drive);
        assert!(f.errors.is_empty());
        assert_eq!(f.fractured, [t >= 1.]);
        assert_eq!(end, w.frame_at(4., &mut Drive));
    }
    let mut w = w.with_checkpoint_budget(0);
    assert_eq!(first, w.frame_at(1., &mut Drive));
    assert_eq!(end, w.frame_at(4., &mut Drive));
}

#[test]
fn fracture_registration_rejects_ambiguous_or_nonconservative_inputs() {
    let mut cases = Vec::new();
    let mut e = event(1.);
    e.source = 3;
    cases.push(vec![e]);
    let mut e = event(1.);
    e.fragments[1].body = 1;
    cases.push(vec![e]);
    let mut e = event(1.);
    e.fragments[1].body = 0;
    cases.push(vec![e]);
    let mut e = event(1.);
    e.fragments[1].offset[0] = f64::NAN;
    cases.push(vec![e]);
    let mut e = event(1.);
    e.fragments[1].impulse[0] = f64::INFINITY;
    cases.push(vec![e]);
    cases.push(vec![event(f64::NAN)]);
    cases.push(vec![event(-1.)]);
    cases.push(vec![event(1.), event(2.)]);
    for events in cases {
        assert!(World3::new(spec()).with_fractures(events).is_err());
    }
    let mut s = spec();
    s.bodies[2].mass = 3.;
    assert!(World3::new(s).with_fractures(vec![event(1.)]).is_err());
    let mut s = spec();
    s.bodies[2].kind = BodyKind::Static;
    assert!(World3::new(s).with_fractures(vec![event(1.)]).is_err());
    let mut w = World3::new(spec());
    w.frame_at(0.1, &mut Drive);
    assert!(w.with_fractures(vec![event(1.)]).is_err());
}

struct Visibility;
impl Driver3 for Visibility {
    fn enabled(&mut self, t: f64, k: usize) -> bool {
        match k {
            0 => t >= 0.2,
            1 => t >= 0.25,
            _ => true,
        }
    }
    fn kinematic(&mut self, t: f64, which: &[usize]) -> Vec<Pose3> {
        which.iter().map(|_| Pose3 { pos: [10. + 2. * t, 0., 0.], ..Pose3::default() }).collect()
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
}
#[test]
fn hidden_sources_defer_fracture_and_hidden_pieces_preserve_inherited_state() {
    let mut s = spec();
    for b in &mut s.bodies {
        b.angular_velocity = [0.; 3];
    }
    let mut w = World3::new(s).with_fractures(vec![event(0.1)]).unwrap();
    let before = w.frame_at(0.19, &mut Visibility);
    assert_eq!(before.enabled, [false, false, false]);
    assert_eq!(before.fractured, [false]);
    let at = w.frame_at(0.2, &mut Visibility);
    assert_eq!(at.enabled, [false, false, true]);
    assert_eq!(at.fractured, [true]);
    close(at.bodies[1].pos[0], 9.4);
    let later = w.frame_at(0.25, &mut Visibility);
    assert_eq!(later.enabled, [false, true, true]);
    close(later.bodies[1].pos[0], 9.4);
    close(later.velocities[1].linear[0], 3.);
    assert_eq!(at, w.frame_at(0.2, &mut Visibility));
}

struct Kinematic;
impl Driver3 for Kinematic {
    fn kinematic(&mut self, t: f64, which: &[usize]) -> Vec<Pose3> {
        which.iter().map(|_| Pose3 { pos: [t * 7., t * 2., 0.], ..Pose3::default() }).collect()
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
}
#[test]
fn kinematic_source_transfers_actual_motion_instead_of_authored_initial_velocity() {
    let mut s = spec();
    s.bodies[0].kind = BodyKind::Kinematic;
    let mut w = World3::new(s).with_fractures(vec![event(0.3)]).unwrap();
    let at = w.frame_at(0.3, &mut Kinematic);
    for k in [1, 2] {
        close(at.velocities[k].linear[0], 7.);
        close(at.velocities[k].linear[1], 2.);
        close(at.velocities[k].angular[2], 0.);
    }
    let end = w.frame_at(0.4, &mut Kinematic);
    close(end.bodies[1].pos[0], 1.8);
    close(end.bodies[2].pos[0], 3.8);
    close(end.bodies[1].pos[1], 0.8);
}

#[test]
fn partition_preserves_linear_and_angular_momentum_at_release() {
    let mut s = spec();
    // Translate the material within the source frame: body origin != COM.
    let mut e = event(0.5);
    for p in &mut e.fragments {
        p.offset[0] += 5.;
    }
    // Spin about z stays on a principal axis and has a closed-form inertia.
    s.pixels_per_meter = 10.;
    let mut w = World3::new(s).with_fractures(vec![e]).unwrap();
    let at = w.frame_at(0.5, &mut Drive);
    let com = [(at.bodies[1].pos[0] + at.bodies[2].pos[0]) * 0.5, (at.bodies[1].pos[1] + at.bodies[2].pos[1]) * 0.5];
    let mut momentum = [0.; 2];
    let mut angular = 0.;
    for k in [1, 2] {
        let v = at.velocities[k].linear;
        momentum[0] += 2. * v[0];
        momentum[1] += 2. * v[1];
        let r = [at.bodies[k].pos[0] - com[0], at.bodies[k].pos[1] - com[1]];
        angular += 2. * (r[0] * v[1] - r[1] * v[0]);
        angular += (2. * (1. + 1.) / 3.) * at.velocities[k].angular[2].to_radians();
    }
    close(momentum[0], 12.);
    close(momentum[1], 0.);
    close(angular, (4. * (4. + 1.) / 3.) * std::f64::consts::FRAC_PI_2);
}

#[test]
fn retired_source_no_longer_blocks_a_later_body() {
    let mut s = spec();
    s.bodies[0].kind = BodyKind::Static;
    for b in &mut s.bodies {
        b.velocity = [0.; 3];
        b.angular_velocity = [0.; 3];
        b.collides_with = None;
    }
    let mut probe = body([0.1; 3], 1.);
    probe.start.pos = [-5., 0., 0.];
    probe.velocity = [10., 0., 0.];
    probe.angular_velocity = [0.; 3];
    probe.collides_with = None;
    s.bodies.push(probe);
    let mut e = event(0.1);
    for p in &mut e.fragments {
        p.impulse = [0., -200., 0.];
    }
    let mut w = World3::new(s).with_fractures(vec![e]).unwrap();
    let f = w.frame_at(0.8, &mut Drive);
    assert!(f.errors.is_empty());
    close(f.bodies[3].pos[0], 3.);
    close(f.velocities[3].linear[0], 10.);
    assert_eq!(f.enabled, [false, true, true, true]);
}

#[test]
fn fracture_detaches_source_joints_and_restores_them_on_backward_seek() {
    let mut s = spec();
    s.bodies[0].velocity = [0.; 3];
    s.bodies[0].angular_velocity = [0.; 3];
    s.joints.push(Joint3Spec {
        kind: Joint3Kind::Ball,
        a: 0,
        b: None,
        anchor: Some([0.; 3]),
        axis: [0., 1., 0.],
        rest_length: None,
        stiffness: None,
        damping: None,
        min: None,
        max: None,
        motor_speed: 0.,
        max_force: None,
        break_force: None,
    });
    let mut w = World3::new(s).with_fractures(vec![event(0.5)]).unwrap();
    assert_eq!(w.frame_at(0.49, &mut Drive).broken, [false]);
    assert_eq!(w.frame_at(0.5, &mut Drive).broken, [true]);
    assert_eq!(w.frame_at(0., &mut Drive).broken, [false]);
    assert_eq!(w.frame_at(0.5, &mut Drive).broken, [true]);
}

#[test]
fn release_overflow_does_not_partially_apply_an_event() {
    struct Hidden;
    impl Driver3 for Hidden {
        fn enabled(&mut self, _: f64, _: usize) -> bool {
            false
        }
        fn kinematic(&mut self, _: f64, _: &[usize]) -> Vec<Pose3> {
            vec![]
        }
        fn fields(&mut self, _: f64) -> Vec<Field> {
            vec![]
        }
    }
    let mut s = spec();
    s.bodies[0].mass = 2e-6;
    s.bodies[1].mass = 1e-6;
    s.bodies[2].mass = 1e-6;
    let mut e = event(1.);
    e.fragments[1].impulse = [f64::MAX, 0., 0.];
    let mut w = World3::new(s).with_fractures(vec![e]).unwrap();
    let f = w.frame_at(1., &mut Drive);
    assert!(f.bodies.is_empty());
    assert!(!f.errors.is_empty());
    // Inspect the failed boundary without rewinding through a checkpoint.
    // Hiding the source prevents another attempt but must expose unchanged
    // placeholder fragment poses and velocities, even for the first piece.
    let unchanged = w.frame_at(1., &mut Hidden);
    assert!(unchanged.errors.is_empty());
    assert_eq!(unchanged.fractured, [false]);
    for k in [1, 2] {
        assert_eq!(unchanged.bodies[k].pos, [0.; 3]);
        assert_eq!(unchanged.velocities[k].linear, [3., 0., 0.]);
    }
    let before = w.frame_at(0.99, &mut Drive);
    assert_eq!(before.enabled, [true, false, false]);
    assert_eq!(before.fractured, [false]);
    assert_eq!(f, w.frame_at(1., &mut Drive));
}

#[test]
fn rounding_never_fires_an_event_before_its_authored_time() {
    let mut w = World3::new(spec()).with_fractures(vec![event(0.1 + f64::EPSILON)]).unwrap();
    assert_eq!(w.frame_at(0.1, &mut Drive).fractured, [false]);
    assert_eq!(w.frame_at(0.11, &mut Drive).fractured, [true]);
}
