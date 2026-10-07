//! A body of cells fractured into pieces that are bodies of cells from the start (the fracture of the world, with voxel fragments): the
//! pieces move as their cells moved, and the momentum and the angular momentum of the whole are what they were.
#![allow(clippy::needless_range_loop)]

use sr_sim::{
    fields::Field,
    physics3d::{shape_mass_properties, *},
};

const SIZE: [f64; 3] = [0.25; 3];
const CELL_MASS: f64 = 2400.0 * 0.25 * 0.25 * 0.25;

struct Still;
impl Driver3 for Still {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
}

fn cells_of(xs: std::ops::Range<i32>, ys: std::ops::Range<i32>, zs: std::ops::Range<i32>) -> Vec<[i32; 3]> {
    let mut cells = Vec::new();
    for z in zs {
        for y in ys.clone() {
            for x in xs.clone() {
                cells.push([x, y, z]);
            }
        }
    }
    cells
}

fn body(cells: Vec<[i32; 3]>) -> Body3Spec {
    let mass = cells.len() as f64 * CELL_MASS;
    Body3Spec {
        kind: BodyKind::Dynamic,
        shape: Shape3::Voxels { size: SIZE, cells },
        mass,
        friction: 0.5,
        restitution: 0.0,
        linear_damping: 0.0,
        angular_damping: 0.0,
        velocity: [0.; 3],
        angular_velocity: [0.; 3],
        group: 0,
        collides_with: None,
        sensor: false,
        fixed_rotation: false,
        bullet: false,
        activate_at: 0.,
        start: Pose3 { pos: [0.3, -1.0, 0.2], rot: [0., 0., 0., 1.] },
    }
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn times(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| (0..3).map(|j| m[i][j] * v[j]).sum())
}

#[test]
fn a_body_of_cells_fractured_into_bodies_of_cells_keeps_its_momentum_and_its_angular_momentum_to_the_last_digits() {
    // a block of 8 by 4 by 4 cells in three pieces of unequal size and shape: an end of 3 columns, a middle in an L (one half height) and the rest
    let whole = cells_of(0..8, 0..4, 0..4);
    let end = cells_of(0..3, 0..4, 0..4);
    let step = cells_of(3..6, 0..2, 0..4);
    let rest: Vec<[i32; 3]> = whole.iter().copied().filter(|c| !end.contains(c) && !step.contains(c)).collect();
    let pieces = [end, step, rest];
    assert_eq!(pieces.iter().map(Vec::len).sum::<usize>(), whole.len());
    let mut source = body(whole.clone());
    source.velocity = [1.0, 0.4, -0.3];
    source.angular_velocity = [20.0, 10.0, 40.0];
    let mut bodies = vec![source];
    bodies.extend(pieces.iter().map(|p| body(p.clone())));
    let fragments = (1..=3).map(|k| Fragment3 { body: k, offset: [0.0; 3], impulse: [0.0; 3] }).collect();
    let mut w = World3::new(World3Spec {
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
    .with_fractures(vec![Fracture3 { source: 0, at: 0.0, radial_impulse: 0.0, fragments, contact: None, dust: None }])
    .unwrap();
    // at the first instant the body is not rotated, so the tensors of its parts are in the axes of the world
    let frame = w.frame_at(0.0, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert_eq!(frame.enabled, vec![false, true, true, true], "the source gives way to its pieces");
    let props = |cells: &[[i32; 3]]| {
        shape_mass_properties(
            &Shape3::Voxels { size: SIZE, cells: cells.to_vec() },
            cells.len() as f64 * CELL_MASS,
            1.0,
        )
        .unwrap()
    };
    let total = props(&whole);
    let at = |p: &Pose3, c: [f64; 3]| [p.pos[0] + c[0], p.pos[1] + c[1], p.pos[2] + c[2]];
    let c0 = at(&frame.bodies[0], total.centre);
    let (v0, w0) = ([1.0, 0.4, -0.3], [20.0f64, 10.0, 40.0].map(f64::to_radians));
    // linear momentum: the pieces' sum is the whole's
    let mut p = [0.0; 3];
    let mut l = [0.0; 3];
    for (k, cells) in pieces.iter().enumerate() {
        let q = props(cells);
        let (v, spin) = (frame.velocities[k + 1].linear, frame.velocities[k + 1].angular.map(f64::to_radians));
        for i in 0..3 {
            p[i] += q.mass * v[i];
        }
        let d = [0, 1, 2].map(|i| at(&frame.bodies[k + 1], q.centre)[i] - c0[i]);
        // its own spin and its orbit about the centre of the whole
        let own = times(q.inertia, spin);
        let orbit = cross(d, [0, 1, 2].map(|i| q.mass * v[i]));
        for i in 0..3 {
            l[i] += own[i] + orbit[i];
        }
        // every piece has the spin of the body, and the velocity of the point of the body its centre of mass was
        let wanted = [0, 1, 2].map(|i| v0[i] + cross(w0, d)[i]);
        for i in 0..3 {
            assert!((spin[i] - w0[i]).abs() < 1e-12, "spin of piece {k}");
            assert!((v[i] - wanted[i]).abs() < 1e-12, "piece {k}, axis {i}: {} against {}", v[i], wanted[i]);
        }
    }
    let wanted_p = [0, 1, 2].map(|i| total.mass * v0[i]);
    let wanted_l = times(total.inertia, w0);
    for i in 0..3 {
        assert!((p[i] - wanted_p[i]).abs() < 1e-12 * total.mass, "momentum {i}: {} against {}", p[i], wanted_p[i]);
        assert!(
            (l[i] - wanted_l[i]).abs() < 1e-12 * wanted_l.iter().map(|c| c.abs()).fold(1.0, f64::max),
            "angular momentum {i}: {} against {}",
            l[i],
            wanted_l[i]
        );
    }
}

/// A block of 8 by 4 by 4 cells that breaks into a piece of 4 columns and one of 3, and one column of 16 cells that is dust.
fn dusty(gravity: f64, at: f64, dust_mass: f64) -> Result<World3, FractureError> {
    pushed_dusty(gravity, at, dust_mass, 0.0)
}

/// The same with the fragments pushed apart by `radial_impulse`.
fn pushed_dusty(gravity: f64, at: f64, dust_mass: f64, radial_impulse: f64) -> Result<World3, FractureError> {
    let whole = cells_of(0..8, 0..4, 0..4);
    let (a, b, dust) = (cells_of(0..4, 0..4, 0..4), cells_of(4..7, 0..4, 0..4), cells_of(7..8, 0..4, 0..4));
    let mut source = body(whole.clone());
    source.velocity = [1.0, 0.4, -0.3];
    source.angular_velocity = [20.0, 10.0, 40.0];
    let mut bodies = vec![source];
    bodies.extend([a, b].map(body));
    let fragments = (1..=2).map(|k| Fragment3 { body: k, offset: [0.0; 3], impulse: [0.0; 3] }).collect();
    let q = shape_mass_properties(&Shape3::Voxels { size: SIZE, cells: dust }, 16.0 * CELL_MASS, 1.0).unwrap();
    let dust = Dust3 { mass: dust_mass, centre: q.centre, inertia: q.inertia };
    World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.,
        step: 0.01,
        gravity: [0.0, gravity, 0.0],
        pixels_per_meter: 1.,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies,
    })
    .with_fractures(vec![Fracture3 { source: 0, at, radial_impulse, fragments, contact: None, dust: Some(dust) }])
}

#[test]
fn a_fracture_with_dust_takes_fragments_and_dust_that_sum_to_the_source_and_nothing_else() {
    let q = |n: f64| n * CELL_MASS;
    assert!(dusty(0.0, 0.0, q(16.0)).is_ok(), "48 + 64 + 16 cells are the 128 of the block");
    for wrong in [q(15.0), q(17.0), 0.0, f64::NAN, -q(16.0)] {
        assert!(dusty(0.0, 0.0, wrong).is_err(), "dust of {wrong} kg does not make the sum");
    }
}

#[test]
fn a_source_with_dust_weighs_what_its_cells_weigh_until_it_breaks() {
    // the same block with no fracture at all: under gravity and a push the two are the same body until the instant of the fracture
    let mut with = dusty(-9.80665, 2.0, 16.0 * CELL_MASS).unwrap();
    let mut alone = World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.,
        step: 0.01,
        gravity: [0.0, -9.80665, 0.0],
        pixels_per_meter: 1.,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies: {
            let mut source = body(cells_of(0..8, 0..4, 0..4));
            source.velocity = [1.0, 0.4, -0.3];
            source.angular_velocity = [20.0, 10.0, 40.0];
            vec![source]
        },
    });
    struct Push;
    impl Driver3 for Push {
        fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
            vec![Pose3::default(); which.len()]
        }
        fn fields(&mut self, _: f64) -> Vec<Field> {
            vec![]
        }
        fn load(&mut self, _: u64, t: f64, body: usize, _: &BodyState) -> Result<Option<Load3>, String> {
            Ok((body == 0 && t >= 0.5).then_some(Load3 { force: [30.0, 0.0, 0.0], torque: [5.0, 0.0, 12.0] }))
        }
    }
    let (a, b) = (with.frame_at(1.5, &mut Push), alone.frame_at(1.5, &mut Push));
    assert!(a.errors.is_empty() && b.errors.is_empty(), "{:?} {:?}", a.errors, b.errors);
    for i in 0..3 {
        assert!((a.bodies[0].pos[i] - b.bodies[0].pos[i]).abs() < 1e-9, "position {i}");
        assert!((a.velocities[0].linear[i] - b.velocities[0].linear[i]).abs() < 1e-9, "velocity {i}");
        assert!((a.velocities[0].angular[i] - b.velocities[0].angular[i]).abs() < 1e-9, "spin {i}");
    }
}

#[test]
fn the_fragments_and_what_the_dust_took_have_the_momentum_and_the_angular_momentum_of_the_source() {
    let mut w = dusty(0.0, 0.0, 16.0 * CELL_MASS).unwrap();
    let frame = w.frame_at(0.0, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    let lost = w.fracture_lost(0).expect("the dust took something");
    let props = |cells: Vec<[i32; 3]>| {
        let mass = cells.len() as f64 * CELL_MASS;
        shape_mass_properties(&Shape3::Voxels { size: SIZE, cells }, mass, 1.0).unwrap()
    };
    let (total, a, b) =
        (props(cells_of(0..8, 0..4, 0..4)), props(cells_of(0..4, 0..4, 0..4)), props(cells_of(4..7, 0..4, 0..4)));
    let at = |p: &Pose3, c: [f64; 3]| [p.pos[0] + c[0], p.pos[1] + c[1], p.pos[2] + c[2]];
    let c0 = at(&frame.bodies[0], total.centre);
    let (v0, w0) = ([1.0, 0.4, -0.3], [20.0f64, 10.0, 40.0].map(f64::to_radians));
    let (mut p, mut l) = (lost.momentum, lost.angular_momentum);
    for (k, q) in [(1, &a), (2, &b)] {
        let (v, spin) = (frame.velocities[k].linear, frame.velocities[k].angular.map(f64::to_radians));
        let d = [0, 1, 2].map(|i| at(&frame.bodies[k], q.centre)[i] - c0[i]);
        let own = times(q.inertia, spin);
        let orbit = cross(d, [0, 1, 2].map(|i| q.mass * v[i]));
        for i in 0..3 {
            p[i] += q.mass * v[i];
            l[i] += own[i] + orbit[i];
        }
    }
    let (wanted_p, wanted_l) = ([0, 1, 2].map(|i| total.mass * v0[i]), times(total.inertia, w0));
    let scale = wanted_l.iter().map(|c| c.abs()).fold(1.0, f64::max);
    for i in 0..3 {
        assert!((p[i] - wanted_p[i]).abs() < 1e-9 * total.mass, "momentum {i}: {} against {}", p[i], wanted_p[i]);
        assert!((l[i] - wanted_l[i]).abs() < 1e-9 * scale, "angular momentum {i}: {} against {}", l[i], wanted_l[i]);
    }
    // and the loss is not nothing: the dust was a column of 16 cells moving with the body
    assert!(lost.momentum.iter().map(|c| c * c).sum::<f64>().sqrt() > 1.0, "{lost:?}");
}

#[test]
fn what_the_dust_took_is_the_same_however_the_frame_is_asked_and_is_not_there_before_the_fracture() {
    let mut early = dusty(0.0, 0.5, 16.0 * CELL_MASS).unwrap();
    early.frame_at(0.4, &mut Still);
    assert_eq!(early.fracture_lost(0), None, "nothing is lost before it breaks");
    let mut fresh = dusty(0.0, 0.5, 16.0 * CELL_MASS).unwrap();
    fresh.frame_at(0.9, &mut Still);
    let wanted = fresh.fracture_lost(0).unwrap();
    // asked for later, then for earlier, by a world that keeps no frames and so replays from a checkpoint: the same figure, and none at all when it is
    // taken back to before the fracture (the figure is part of the state that a checkpoint restores)
    let mut later = dusty(0.0, 0.5, 16.0 * CELL_MASS).unwrap().with_frame_log_budget(0);
    later.frame_at(2.0, &mut Still);
    assert_eq!(later.checkpoint_restores(), 0);
    assert!(later.fracture_lost(0).is_some());
    later.frame_at(0.9, &mut Still);
    assert!(later.checkpoint_restores() > 0, "the world went back to a checkpoint and fired the fracture again");
    assert_eq!(later.fracture_lost(0), Some(wanted));
    later.frame_at(0.4, &mut Still);
    assert_eq!(later.fracture_lost(0), None, "taken back to before the fracture the world has not lost anything yet");
}

#[test]
fn the_push_that_separates_the_fragments_adds_no_momentum_to_them_and_the_dust_does_not_take_a_share_of_it() {
    // the pieces push each other, and the mean of the push is taken off over the fragments: with dust that is not the source's mass
    let mut w = pushed_dusty(0.0, 0.0, 16.0 * CELL_MASS, 400.0).unwrap();
    let frame = w.frame_at(0.0, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    let lost = w.fracture_lost(0).unwrap();
    let v0 = [1.0, 0.4, -0.3];
    let mut p = lost.momentum;
    for (k, cells) in [(1, 64.0), (2, 48.0)] {
        for i in 0..3 {
            p[i] += cells * CELL_MASS * frame.velocities[k].linear[i];
        }
    }
    for i in 0..3 {
        let wanted = 128.0 * CELL_MASS * v0[i];
        assert!((p[i] - wanted).abs() < 1e-9 * wanted.abs().max(1.0), "momentum {i}: {} against {wanted}", p[i]);
    }
    // and the pieces do move apart
    assert!((frame.velocities[1].linear[0] - frame.velocities[2].linear[0]).abs() > 0.1);
}

/// A source of four columns in a square (8 cells each, the second twice as tall), three of them fragments and the fourth, in the corner opposite
/// the first, dust: no three centres are in line with the dust, and the masses are unequal, so that the mean of a push does not point at the dust.
fn corners(radial_impulse: f64) -> (World3, Vec<Vec<[i32; 3]>>) {
    let (a, b, c, d) = (
        cells_of(0..2, 0..2, 0..2),
        cells_of(2..4, 0..2, 0..4),
        cells_of(0..2, 2..4, 0..2),
        cells_of(2..4, 2..4, 0..2),
    );
    let whole: Vec<[i32; 3]> = [&a, &b, &c, &d].into_iter().flatten().copied().collect();
    let mut source = body(whole.clone());
    source.velocity = [1.0, 0.4, -0.3];
    source.angular_velocity = [20.0, 10.0, 40.0];
    let mut bodies = vec![source];
    bodies.extend([a.clone(), b.clone(), c.clone()].map(body));
    let fragments = (1..=3).map(|k| Fragment3 { body: k, offset: [0.0; 3], impulse: [0.0; 3] }).collect();
    let q = shape_mass_properties(&Shape3::Voxels { size: SIZE, cells: d.clone() }, 8.0 * CELL_MASS, 1.0).unwrap();
    let dust = Dust3 { mass: 8.0 * CELL_MASS, centre: q.centre, inertia: q.inertia };
    let w = World3::new(World3Spec {
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
    .with_fractures(vec![Fracture3 { source: 0, at: 0.0, radial_impulse, fragments, contact: None, dust: Some(dust) }])
    .unwrap();
    (w, vec![a, b, c, whole])
}

#[test]
fn a_push_that_separates_the_fragments_keeps_the_angular_momentum_too_when_the_dust_is_off_their_line() {
    let (mut w, cells) = corners(400.0);
    let frame = w.frame_at(0.0, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    let lost = w.fracture_lost(0).unwrap();
    let props = |cells: &Vec<[i32; 3]>| {
        shape_mass_properties(&Shape3::Voxels { size: SIZE, cells: cells.clone() }, cells.len() as f64 * CELL_MASS, 1.0)
            .unwrap()
    };
    let at = |p: &Pose3, c: [f64; 3]| [p.pos[0] + c[0], p.pos[1] + c[1], p.pos[2] + c[2]];
    let total = props(&cells[3]);
    let c0 = at(&frame.bodies[0], total.centre);
    let (v0, w0) = ([1.0, 0.4, -0.3], [20.0f64, 10.0, 40.0].map(f64::to_radians));
    let (mut p, mut l) = (lost.momentum, lost.angular_momentum);
    for k in 1..=3 {
        let q = props(&cells[k - 1]);
        let (v, spin) = (frame.velocities[k].linear, frame.velocities[k].angular.map(f64::to_radians));
        let d = [0, 1, 2].map(|i| at(&frame.bodies[k], q.centre)[i] - c0[i]);
        let own = times(q.inertia, spin);
        let orbit = cross(d, [0, 1, 2].map(|i| q.mass * v[i]));
        for i in 0..3 {
            p[i] += q.mass * v[i];
            l[i] += own[i] + orbit[i];
        }
    }
    let (wanted_p, wanted_l) = ([0, 1, 2].map(|i| total.mass * v0[i]), times(total.inertia, w0));
    let scale = wanted_l.iter().map(|c| c.abs()).fold(1.0, f64::max);
    for i in 0..3 {
        assert!((p[i] - wanted_p[i]).abs() < 1e-9 * total.mass, "momentum {i}: {} against {}", p[i], wanted_p[i]);
        assert!((l[i] - wanted_l[i]).abs() < 1e-9 * scale, "angular momentum {i}: {} against {}", l[i], wanted_l[i]);
    }
    // and the fragments do fly apart
    assert!((frame.velocities[1].linear[0] - frame.velocities[2].linear[0]).abs() > 0.1);
}

#[test]
fn a_body_that_has_turned_before_it_breaks_gives_its_pieces_the_momentum_it_had() {
    // the fracture is not at the first instant: the source has moved and turned, its pieces were stepped as bodies that are out of the world (with
    // their colliders switched off by it), and what they have of the source is still its momentum, whatever the axis it was turning about
    for spin in [[0.0, 30.0, 0.0], [0.0, 0.0, 30.0], [20.0, 10.0, 40.0]] {
        let whole = cells_of(0..8, 0..4, 0..4);
        let end = cells_of(0..3, 0..4, 0..4);
        let step = cells_of(3..6, 0..2, 0..4);
        let rest: Vec<[i32; 3]> = whole.iter().copied().filter(|c| !end.contains(c) && !step.contains(c)).collect();
        let pieces = [end, step, rest];
        let mut source = body(whole.clone());
        source.velocity = [1.0, 0.4, -0.3];
        source.angular_velocity = spin;
        let mut bodies = vec![source];
        bodies.extend(pieces.iter().map(|p| body(p.clone())));
        let fragments = (1..=3).map(|k| Fragment3 { body: k, offset: [0.0; 3], impulse: [0.0; 3] }).collect();
        let mut w = World3::new(World3Spec {
            fix_internal_edges: false,
            start: 0.,
            step: 1.0 / 240.0,
            gravity: [0.; 3],
            pixels_per_meter: 1.,
            iterations: 8,
            bounds: Bounds3::None,
            joints: vec![],
            bodies,
        })
        .with_fractures(vec![Fracture3 {
            source: 0,
            at: 0.2,
            radial_impulse: 0.0,
            fragments,
            contact: None,
            dust: None,
        }])
        .unwrap();
        // just after the fracture, before the pieces have touched each other for long (the step after it)
        let frame = w.frame_at(0.2, &mut Still);
        assert!(frame.errors.is_empty() && frame.fractured == vec![true], "{:?}", frame.errors);
        let props = |cells: &Vec<[i32; 3]>| {
            shape_mass_properties(
                &Shape3::Voxels { size: SIZE, cells: cells.clone() },
                cells.len() as f64 * CELL_MASS,
                1.0,
            )
            .unwrap()
        };
        // the centre of mass of every piece in the world: the pose of the source (which they have) and its own centre in the source's frame; the
        // rotation is read from the velocity of a piece along the source's axes, so this checks the sum and not the geometry
        let total = props(&whole);
        let (mut p, mut m) = ([0.0; 3], 0.0);
        for (k, cells) in pieces.iter().enumerate() {
            let q = props(cells);
            m += q.mass;
            for i in 0..3 {
                p[i] += q.mass * frame.velocities[k + 1].linear[i];
            }
        }
        for (i, v) in [1.0, 0.4, -0.3].iter().enumerate() {
            assert!(
                (p[i] - total.mass * v).abs() < 1e-9 * total.mass,
                "spin {spin:?}, axis {i}: {} against {}",
                p[i],
                total.mass * v
            );
        }
        assert!((m - total.mass).abs() < 1e-9 * m);
    }
}
