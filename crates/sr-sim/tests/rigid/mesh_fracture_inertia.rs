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
    .with_fractures(vec![Fracture3 { source: 0, at, radial_impulse: 0.0, fragments, contact: None, dust: None }])
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

#[test]
fn the_pieces_of_a_fracture_that_are_plates_spin_up_as_the_plates_they_are() {
    // the same box in three plates thin along x (0.25 by 1 by 1 m, 600 kg: about x 600 (1 + 1) / 12 = 100, about y and z 53.125, the two smaller ones equal)
    let (points, triangles) = cuboid([0.125, 0.5, 0.5]);
    let plate = || Shape3::Decomposition(points.clone(), triangles.clone());
    let bodies = vec![
        body(Shape3::Box([0.375, 0.5, 0.5]), 1800.0, true),
        body(plate(), 600.0, true),
        body(plate(), 600.0, true),
        body(plate(), 600.0, true),
    ];
    let mut w = world(bodies, 0.3, vec![(1, [-0.25, 0.0, 0.0]), (2, [0.0; 3]), (3, [0.25, 0.0, 0.0])]);
    for k in 1..=3 {
        let alpha = spin_up(&mut w, k, 300.0);
        assert!(
            (alpha - 3.0).abs() < 1e-9,
            "plate {k} spins up at {alpha} rad/s2, a plate of 100 kg m2 under 300 at 3"
        );
    }
}

/// The three plates of the box above as the pieces of a fracture, whatever shape the caller makes of them.
fn plates_with(plate: impl Fn() -> Shape3) -> Result<World3, FractureError> {
    let bodies = vec![
        body(Shape3::Box([0.375, 0.5, 0.5]), 1800.0, true),
        body(plate(), 600.0, true),
        body(plate(), 600.0, true),
        body(plate(), 600.0, true),
    ];
    let pieces = [(1, [-0.25, 0.0, 0.0]), (2, [0.0; 3]), (3, [0.25, 0.0, 0.0])];
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
    .with_fractures(vec![Fracture3 {
        source: 0,
        at: 0.3,
        radial_impulse: 0.0,
        fragments,
        contact: None,
        dust: None,
    }])
}

#[test]
fn a_piece_that_is_a_convex_hull_spins_up_as_the_plate_it_is() {
    // the hull of the eight corners of a plate: its tensor comes from the same exact integrals as a mesh's, and not from Parry's hull routine, whose
    // eigen solver puts the largest moment of a plate on the wrong axis
    let (points, _) = cuboid([0.125, 0.5, 0.5]);
    let mut w = plates_with(|| Shape3::Convex(points.clone())).unwrap();
    for k in 1..=3 {
        let alpha = spin_up(&mut w, k, 300.0);
        assert!((alpha - 3.0).abs() < 1e-9, "hull {k} spins up at {alpha} rad/s2, a plate of 100 kg m2 under 300 at 3");
    }
}

#[test]
fn a_piece_whose_mesh_is_open_or_wound_both_ways_is_refused_and_one_with_unwelded_corners_is_not() {
    let (points, triangles) = cuboid([0.125, 0.5, 0.5]);
    // a face left out: no volume to speak of, and a tensor that depends on where the origin is
    let open: Vec<[u32; 3]> = triangles[2..].to_vec();
    assert!(plates_with(|| Shape3::Decomposition(points.clone(), open.clone())).is_err(), "an open mesh");
    // one triangle turned over: two faces that meet with the same winding
    let mut turned = triangles.clone();
    turned[3].swap(1, 2);
    assert!(plates_with(|| Shape3::Decomposition(points.clone(), turned.clone())).is_err(), "a mesh wound both ways");
    // every triangle with corners of its own (as a mesh that was cut apart has): the same closed surface
    let (mut loose, mut faces) = (Vec::new(), Vec::new());
    for t in &triangles {
        let base = loose.len() as u32;
        loose.extend(t.iter().map(|i| points[*i as usize]));
        faces.push([base, base + 1, base + 2]);
    }
    let mut w = plates_with(|| Shape3::Decomposition(loose.clone(), faces.clone())).unwrap();
    let alpha = spin_up(&mut w, 1, 300.0);
    assert!((alpha - 3.0).abs() < 1e-9, "{alpha}");
}

#[test]
fn a_piece_of_several_shells_wound_alike_is_taken_and_one_with_a_shell_turned_inward_is_refused() {
    // two disjoint boxes in one piece: a solid in two parts, taken; with the second turned inside out the integrals would be V1 - V2 with a centre and a
    // tensor that belong to no body, and a cavity (a shell wound inward) is refused with it: the pieces of a fracture of cells have none
    let (one, faces) = cuboid([0.125, 0.5, 0.5]);
    // (the second smaller: equal boxes would cancel to no volume at all and be refused for that)
    let two: Vec<[f64; 3]> = one.iter().map(|p| [p[0] * 0.3 + 0.5, p[1] * 0.3, p[2] * 0.3]).collect();
    let mut points = one.clone();
    points.extend(two);
    let shell = |turned: bool| {
        let mut triangles = faces.clone();
        triangles.extend(faces.iter().map(|t| {
            if turned {
                [t[0] + 8, t[2] + 8, t[1] + 8]
            } else {
                [t[0] + 8, t[1] + 8, t[2] + 8]
            }
        }));
        triangles
    };
    assert!(plates_with(|| Shape3::Decomposition(points.clone(), shell(false))).is_ok(), "two shells wound alike");
    let turned = plates_with(|| Shape3::Decomposition(points.clone(), shell(true)));
    assert!(turned.is_err(), "a shell turned inward");
}

#[test]
fn a_hull_that_is_not_a_box_has_the_tensor_of_the_same_solid_as_a_mesh() {
    // a tetrahedron with its corner off the origin, as a hull of four points and as the closed mesh of its four faces: the same solid, to the last digits
    let corners = vec![[0.1, 0.2, 0.3], [1.1, 0.2, 0.3], [0.1, 1.5, 0.3], [0.1, 0.2, 0.9]];
    let faces = vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];
    let spin = |shape: Shape3| spin_up(&mut plates_with(|| shape.clone()).unwrap(), 1, 300.0);
    let (hull, mesh) =
        (spin(Shape3::Convex(corners.clone())), spin(Shape3::Decomposition(corners.clone(), faces.clone())));
    assert!((hull - mesh).abs() < 1e-9 * mesh.abs(), "the hull spins up at {hull} rad/s2 and the mesh at {mesh}");
    assert!(mesh.is_finite() && mesh > 0.0);
}
