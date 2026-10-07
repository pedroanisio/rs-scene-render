//! The tensors of a fracture whose pieces are meshes: the source has the sum of its pieces' tensors, and each piece its own, so that each spins up
//! as an exact box of that size does. A box is the case in which the tensors are diagonal and have repeated moments, the case in which a general
//! eigen solver (Parry's, behind `MassProperties::sum` and `from_trimesh`) puts the largest moment on the wrong axis.
use sr_sim::{fields::Field, physics3d::*};

/// The closed triangle mesh of a box of half sizes `h`, outward windings.
fn cuboid(h: [f64; 3]) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let points: Vec<[f64; 3]> =
        (0..8).map(|k| [0, 1, 2].map(|a| if (k >> a) & 1 == 1 { h[a] } else { -h[a] })).collect();
    let faces: [[u32; 3]; 12] = [
        [0, 2, 1],
        [1, 2, 3],
        [4, 5, 6],
        [5, 7, 6],
        [0, 1, 4],
        [1, 5, 4],
        [2, 6, 3],
        [3, 6, 7],
        [0, 4, 2],
        [2, 4, 6],
        [1, 3, 5],
        [3, 7, 5],
    ];
    (points, faces.to_vec())
}

/// The body of a box of half sizes `h` and `mass`, made as a box or as a closed mesh (a decomposition, which is what the pieces of a fracture are).
fn body(shape: Shape3, mass: f64, alone: bool) -> Body3Spec {
    Body3Spec {
        kind: BodyKind::Dynamic,
        shape,
        mass,
        friction: 0.0,
        restitution: 0.0,
        linear_damping: 0.0,
        angular_damping: 0.0,
        velocity: [0.; 3],
        angular_velocity: [0.; 3],
        group: 0,
        // the pieces touch each other face to face and are pushed alike: they collide with nothing, to see each as its own box
        collides_with: alone.then(Vec::new),
        sensor: false,
        fixed_rotation: false,
        bullet: false,
        activate_at: 0.,
        start: Pose3::default(),
    }
}

/// Pushes every body with a torque about x from 0.4 s on.
struct Torque(f64);
impl Driver3 for Torque {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
    fn load(&mut self, _: u64, t: f64, _: usize, _: &BodyState) -> Result<Option<Load3>, String> {
        Ok((t + 1e-9 >= 0.4).then_some(Load3 { force: [0.0; 3], torque: [self.0, 0.0, 0.0] }))
    }
}

fn world(bodies: Vec<Body3Spec>, at: f64, pieces: Vec<(usize, [f64; 3])>) -> World3 {
    let fragments = pieces.into_iter().map(|(body, offset)| Fragment3 { body, offset, impulse: [0.0; 3] }).collect();
    World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.,
        step: 0.01,
        gravity: [0.; 3],
        pixels_per_meter: 1.,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies,
    })
    .with_fractures(vec![Fracture3 { source: 0, at, radial_impulse: 0.0, fragments, contact: None }])
    .unwrap()
}

/// The spin-up about x of body `k` between 0.41 and 0.45 s under a torque of `torque` about x, in rad/s2.
fn spin_up(w: &mut World3, k: usize, torque: f64) -> f64 {
    let mut d = Torque(torque);
    let (a, b) = (w.frame_at(0.41, &mut d), w.frame_at(0.45, &mut d));
    assert!(a.errors.is_empty() && b.errors.is_empty(), "{:?}", a.errors);
    (b.velocities[k].angular[0] - a.velocities[k].angular[0]).to_radians() / 0.04
}

#[test]
fn the_source_of_a_fracture_into_meshes_spins_up_as_the_box_it_is_until_it_fractures() {
    // a box of 0.75 by 1 by 1 m and 1800 kg (about x: 1800 (1 + 1) / 12 = 300) in two halves along z, each a closed mesh of 0.75 by 1 by 0.5: the
    // sum of two diagonal tensors is diagonal with two equal moments, the smaller ones
    let (points, triangles) = cuboid([0.375, 0.5, 0.25]);
    let half = || Shape3::Decomposition(points.clone(), triangles.clone());
    let bodies =
        vec![body(Shape3::Box([0.375, 0.5, 0.5]), 1800.0, true), body(half(), 900.0, true), body(half(), 900.0, true)];
    let mut w = world(bodies, 2.0, vec![(1, [0.0, 0.0, -0.25]), (2, [0.0, 0.0, 0.25])]);
    let alpha = spin_up(&mut w, 0, 300.0);
    assert!((alpha - 1.0).abs() < 1e-9, "the source spins up at {alpha} rad/s2, the box of 300 kg m2 at 1");
}
