//! A body of cells divided into pieces (the partition of `sr_3d::pieces`) that are bodies of cells for the rigid world's fracture: every cell
//! and every kilogram of the source is in one piece or in the dust, and the world given them breaks the source into them with the motion it had.
#![allow(clippy::needless_range_loop)]

use sr_3d::occupancy::Occupancy;
use sr_3d::pieces::{Partition, Plane};
use sr_eval::voxels::{fracture, Body, FracturePolicy, Fractured, Overflow};
use sr_sim::fields::Field;
use sr_sim::physics3d::{shape_mass_properties, *};
use std::collections::BTreeSet;

const SIZE: [f64; 3] = [0.25; 3];
const DENSITY: f64 = 2400.0;
const CELL_MASS: f64 = 2400.0 * 0.25 * 0.25 * 0.25;

/// Every piece is a body, up to `n` of them: what the first tests ask for.
fn keep(n: usize) -> FracturePolicy {
    FracturePolicy { min_cells: 1, max_fragments: n, overflow: Overflow::Error }
}

fn block(nx: i32, ny: i32, nz: i32) -> Occupancy {
    let mut cells = Vec::new();
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                cells.push(([i, j, k], 1 + ((i + j + k) % 3) as u8));
            }
        }
    }
    Occupancy::from_cells(cells).unwrap()
}

fn cells_of(shape: &Shape3) -> Vec<[i32; 3]> {
    let Shape3::Voxels { cells, .. } = shape else { panic!("a body of cells") };
    cells.clone()
}

/// The block of 12 by 6 by 6 cells cut by the planes x >= 2 and x >= 6 into columns of 72, 144 and 216 cells.
fn columns() -> (Occupancy, [Plane; 2]) {
    (block(12, 6, 6), [Plane { normal: [1, 0, 0], offset: 4 }, Plane { normal: [1, 0, 0], offset: 12 }])
}

fn cells_in(f: &Fractured) -> Vec<usize> {
    f.pieces.iter().map(|p| cells_of(&p.shape).len()).collect()
}

#[test]
fn every_cell_and_every_kilogram_of_the_source_is_in_exactly_one_piece() {
    let o = block(12, 6, 6);
    let f = fracture(&o, Partition::Voronoi { seeds: 5, seed: 7 }, &keep(16), SIZE, DENSITY, 1.0).unwrap();
    assert!(f.pieces.len() > 1, "{} pieces", f.pieces.len());
    let mut all: Vec<[i32; 3]> = f.pieces.iter().flat_map(|p| cells_of(&p.shape)).collect();
    assert_eq!(all.len(), o.count() as usize, "no cell twice or lost");
    all.sort_unstable();
    let mut wanted: Vec<[i32; 3]> = o.cells().collect();
    wanted.sort_unstable();
    assert_eq!(all, wanted);
    // a piece is its cells at the mass of a cell, the source all of them
    for p in &f.pieces {
        assert!((p.mass - cells_of(&p.shape).len() as f64 * CELL_MASS).abs() < 1e-9 * p.mass);
    }
    assert!((f.source.mass - o.count() as f64 * CELL_MASS).abs() < 1e-9 * f.source.mass);
    let total: f64 = f.pieces.iter().map(|p| p.mass).sum();
    assert!((total - f.source.mass).abs() < 1e-9 * total, "{total} against {}", f.source.mass);
    assert_eq!(cells_of(&f.source.shape).len(), o.count() as usize);
    assert!(f.dust.is_empty() && f.dust_body.is_none());
    assert_eq!(f.graph.pieces().len(), f.pieces.len());
    assert_eq!(f.piece_ids, (0..f.pieces.len()).collect::<Vec<_>>());
}

#[test]
fn the_pieces_and_their_joints_are_the_ones_a_hand_count_gives() {
    // the columns of 72, 144 and 216 cells: each joint is the 6 by 6 faces of the plane between two of them, and the first and the last do not touch
    let (o, planes) = columns();
    let f = fracture(&o, Partition::Planes(&planes), &keep(8), SIZE, DENSITY, 1.0).unwrap();
    assert_eq!(cells_in(&f), vec![72, 144, 216]);
    let joints: Vec<(u32, u32, u64)> = f.graph.edges().iter().map(|e| (e.a(), e.b(), e.total_faces())).collect();
    assert_eq!(joints, vec![(0, 1, 36), (1, 2, 36)]);
    // the pieces list their cells in the order of the scan: along x, then y, then z
    let first = cells_of(&f.pieces[0].shape);
    assert_eq!(first[..3], [[0, 0, 0], [1, 0, 0], [0, 1, 0]]);
}

#[test]
fn the_pieces_do_not_depend_on_the_order_of_the_seeds_that_cut_the_body() {
    // seeds in doubled coordinates, found so that no cell of the block is at the same distance from two of them: the nearest one is the same in any
    // order, and the pieces are
    // numbered by their first cell and not by their seed
    let o = block(12, 6, 6);
    let seeds = [[3i64, 5, 0], [0, 0, 10], [17, 0, 6], [21, 3, 6]];
    let reversed = [seeds[3], seeds[2], seeds[1], seeds[0]];
    let shuffled = [seeds[1], seeds[3], seeds[0], seeds[2]];
    let a = fracture(&o, Partition::VoronoiAt(&seeds), &keep(8), SIZE, DENSITY, 1.0).unwrap();
    let b = fracture(&o, Partition::VoronoiAt(&reversed), &keep(8), SIZE, DENSITY, 1.0).unwrap();
    let c = fracture(&o, Partition::VoronoiAt(&shuffled), &keep(8), SIZE, DENSITY, 1.0).unwrap();
    assert!(a.pieces.len() >= 4, "{} pieces", a.pieces.len());
    assert_eq!(a, b);
    assert_eq!(a, c);
}

#[test]
fn what_cannot_be_broken_is_an_error_that_says_why() {
    let o = block(8, 4, 4);
    let many = fracture(&o, Partition::Voronoi { seeds: 12, seed: 3 }, &keep(3), SIZE, DENSITY, 1.0).unwrap_err();
    assert!(many.contains("maxFragments") && many.contains("slots"), "{many}");
    let empty = Occupancy::from_cells(Vec::<([i32; 3], u8)>::new()).unwrap();
    let e = fracture(&empty, Partition::Voronoi { seeds: 2, seed: 1 }, &keep(4), SIZE, DENSITY, 1.0).unwrap_err();
    assert!(e.contains("at least one cell"), "{e}");
    for (size, density, ppm) in [([0.0; 3], DENSITY, 1.0), (SIZE, 0.0, 1.0), (SIZE, DENSITY, f64::NAN)] {
        let e = fracture(&o, Partition::Voronoi { seeds: 2, seed: 1 }, &keep(4), size, density, ppm).unwrap_err();
        assert!(e.contains("positive"), "{e}");
    }
}

/// The source of a fracture as a body of the world, with the velocity and the spin that the tests give it.
fn source_spec(source: &Body, velocity: [f64; 3], spin: [f64; 3]) -> Body3Spec {
    Body3Spec { velocity, angular_velocity: spin, ..piece_spec(source) }
}

fn piece_spec(b: &Body) -> Body3Spec {
    Body3Spec {
        kind: BodyKind::Dynamic,
        shape: b.shape.clone(),
        mass: b.mass,
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

/// The world of a fracture made by [`fracture`]: the source, then its pieces as the fragments, at `at`.
fn world_of(f: &Fractured, velocity: [f64; 3], spin: [f64; 3], at: f64) -> Result<World3, FractureError> {
    world_at_scale(f, velocity, spin, at, 1.0)
}

/// The same in a world of `ppm` scene units to a metre (the velocities are in units a second).
fn world_at_scale(
    f: &Fractured,
    velocity: [f64; 3],
    spin: [f64; 3],
    at: f64,
    ppm: f64,
) -> Result<World3, FractureError> {
    let mut bodies = vec![source_spec(&f.source, velocity, spin)];
    bodies.extend(f.pieces.iter().map(piece_spec));
    let fragments = (1..=f.pieces.len()).map(|k| Fragment3 { body: k, offset: [0.0; 3], impulse: [0.0; 3] }).collect();
    World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.,
        step: 0.01,
        gravity: [0.; 3],
        pixels_per_meter: ppm,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies,
    })
    .with_fractures(vec![Fracture3 {
        source: 0,
        at,
        radial_impulse: 0.0,
        fragments,
        contact: None,
        dust: f.dust_body.clone(),
    }])
}

/// The source at rest but for what it was given, nothing else in the world.
struct Still;
impl Driver3 for Still {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// The pieces of `f` in a world, at the first instant, with the source moving and spinning: each piece's centre of mass is where its cells' was and it
/// moves as that point of the body did, v + w x d, with the body's spin.
fn assert_pieces_move_as_the_body_did(f: &Fractured) {
    let (v0, w0) = ([1.0, 0.4, -0.3], [20.0, 10.0, 40.0]);
    let mut w = world_of(f, v0, w0, 0.0).unwrap();
    let frame = w.frame_at(0.0, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert_eq!(frame.fractured, vec![true]);
    let shown: BTreeSet<usize> = (0..frame.enabled.len()).filter(|k| frame.enabled[*k]).collect();
    assert_eq!(shown, (1..=f.pieces.len()).collect::<BTreeSet<_>>());
    let whole = shape_mass_properties(&f.source.shape, f.source.mass, 1.0).unwrap();
    let c0: [f64; 3] = std::array::from_fn(|i| frame.bodies[0].pos[i] + whole.centre[i]);
    for (k, piece) in f.pieces.iter().enumerate() {
        let q = shape_mass_properties(&piece.shape, piece.mass, 1.0).unwrap();
        let placed = &frame.bodies[k + 1];
        let com: [f64; 3] = std::array::from_fn(|i| placed.pos[i] + q.centre[i]);
        // the piece is placed at the source's origin: its cells are in the source's frame, so its centre of mass is where those cells' was
        assert!((com[0] - (0.3 + q.centre[0])).abs() < 1e-12, "piece {k} is put at the source's own place");
        let d: [f64; 3] = std::array::from_fn(|i| com[i] - c0[i]);
        let turn = cross(w0.map(f64::to_radians), d);
        for i in 0..3 {
            let wanted = v0[i] + turn[i];
            let got = frame.velocities[k + 1].linear[i];
            assert!((got - wanted).abs() < 1e-9, "piece {k}, axis {i}: {got} against {wanted}");
            assert!((frame.velocities[k + 1].angular[i] - w0[i]).abs() < 1e-9, "the spin of piece {k}");
        }
    }
}

#[test]
fn the_world_given_the_pieces_puts_each_where_its_cells_were_with_the_velocity_of_that_point_of_the_source() {
    let o = block(10, 6, 6);
    let f = fracture(&o, Partition::Voronoi { seeds: 4, seed: 11 }, &keep(8), SIZE, DENSITY, 1.0).unwrap();
    assert!(f.pieces.len() >= 3 && f.dust.is_empty());
    assert_pieces_move_as_the_body_did(&f);
}

#[test]
fn the_same_with_dust_the_pieces_still_move_as_the_body_did_about_the_centre_of_all_its_cells() {
    // the centre of mass of the source is that of all its cells, the dust's included, and the pieces' velocities are measured from it
    let (o, planes) = columns();
    let policy = FracturePolicy { min_cells: 100, max_fragments: 8, overflow: Overflow::Error };
    let f = fracture(&o, Partition::Planes(&planes), &policy, SIZE, DENSITY, 1.0).unwrap();
    assert_eq!(f.dust.len(), 72);
    assert_pieces_move_as_the_body_did(&f);
}

#[test]
fn the_pieces_with_fewer_cells_than_the_least_are_dust_that_leaves_with_the_fracture() {
    let (o, planes) = columns();
    let policy = FracturePolicy { min_cells: 100, max_fragments: 8, overflow: Overflow::Error };
    let f = fracture(&o, Partition::Planes(&planes), &policy, SIZE, DENSITY, 1.0).unwrap();
    assert_eq!(cells_in(&f), vec![144, 216]);
    assert_eq!(f.piece_ids, vec![1, 2]);
    assert_eq!(f.graph.pieces().len(), 3, "the graph has all the pieces");
    assert_eq!(f.dust.len(), 72);
    assert!(f.dust.iter().all(|c| c[0] < 2), "the dust is the column of 72 cells");
    // every cell is a body's, the dust's or nowhere else
    let mut every: Vec<[i32; 3]> = f.dust.clone();
    every.extend(f.pieces.iter().flat_map(|p| cells_of(&p.shape)));
    every.sort_unstable();
    let mut wanted: Vec<[i32; 3]> = o.cells().collect();
    wanted.sort_unstable();
    assert_eq!(every, wanted);
    // the source weighs all its cells, and the fragments and the dust sum to it
    assert_eq!(cells_of(&f.source.shape).len(), 432);
    assert!((f.source.mass - 432.0 * CELL_MASS).abs() < 1e-9 * f.source.mass);
    let dust = f.dust_body.as_ref().unwrap();
    let total: f64 = f.pieces.iter().map(|p| p.mass).sum::<f64>() + dust.mass;
    assert!((total - f.source.mass).abs() < 1e-9 * total, "{total} against {}", f.source.mass);
    assert_dust_is_the_column(dust, 1.0);
    // the dust's centre is that of its column (cells 0 and 1 of x, 6 by 6 of the others, a quarter of a metre each): at x = 0.25 m, y and z at 0.75 m
    for (got, wanted) in dust.centre.iter().zip([0.25, 0.75, 0.75]) {
        assert!((got - wanted).abs() < 1e-12, "{:?}", dust.centre);
    }
}

#[test]
fn of_too_many_pieces_the_smallest_are_dust_or_it_is_an_error_that_says_how_many() {
    let (o, planes) = columns();
    let dust = FracturePolicy { min_cells: 1, max_fragments: 2, overflow: Overflow::Dust };
    let f = fracture(&o, Partition::Planes(&planes), &dust, SIZE, DENSITY, 1.0).unwrap();
    assert_eq!(cells_in(&f), vec![144, 216], "the largest keep their slots, in the order of the partition");
    assert_eq!(f.dust.len(), 72);
    // of equal pieces the first ones keep them
    let halves = [Plane { normal: [1, 0, 0], offset: 8 }, Plane { normal: [1, 0, 0], offset: 16 }];
    let equal = fracture(&o, Partition::Planes(&halves), &dust, SIZE, DENSITY, 1.0).unwrap();
    assert_eq!(equal.piece_ids, vec![0, 1]);
    assert_eq!(equal.dust.len(), 144);
    let error = FracturePolicy { overflow: Overflow::Error, ..dust };
    let e = fracture(&o, Partition::Planes(&planes), &error, SIZE, DENSITY, 1.0).unwrap_err();
    assert_eq!(e, "a fracture makes 3 pieces and there are 2 slots for them (maxFragments)");
}

#[test]
fn a_fracture_in_which_no_piece_is_a_body_is_an_error_and_the_world_takes_the_pieces_and_the_dust() {
    let (o, planes) = columns();
    let none = FracturePolicy { min_cells: 1000, max_fragments: 8, overflow: Overflow::Dust };
    assert!(fracture(&o, Partition::Planes(&planes), &none, SIZE, DENSITY, 1.0).is_err());
    // the world is given the source with all its mass, the pieces as fragments and the dust: they sum to the source, and it takes them
    let policy = FracturePolicy { min_cells: 100, max_fragments: 8, overflow: Overflow::Error };
    let f = fracture(&o, Partition::Planes(&planes), &policy, SIZE, DENSITY, 1.0).unwrap();
    let w = world_of(&f, [0.0; 3], [0.0; 3], 0.2);
    assert!(w.is_ok(), "{:?}", w.err());
    // and without the dust that the pieces leave out, the sum is not the source's
    let without = Fractured { dust_body: None, ..f };
    assert!(world_of(&without, [0.0; 3], [0.0; 3], 0.2).is_err());
}

#[test]
fn a_world_with_a_fracture_made_here_is_the_same_to_the_bit_however_its_frames_are_asked() {
    // the pieces and the dust of a partition, the fracture fired at 0.3 s, the frames asked fresh, after a later one, and by a world that keeps none and
    // replays from a checkpoint (which has to have been taken and used)
    let (o, planes) = columns();
    let policy = FracturePolicy { min_cells: 100, max_fragments: 8, overflow: Overflow::Error };
    let f = fracture(&o, Partition::Planes(&planes), &policy, SIZE, DENSITY, 1.0).unwrap();
    let spin = [30.0, -20.0, 50.0];
    let make = || world_of(&f, [1.0, 0.4, -0.3], spin, 0.3).unwrap();
    let times = [0.29, 0.3, 0.31, 0.5];
    let reference: Vec<_> = {
        let mut w = make();
        times.iter().map(|t| w.frame_at(*t, &mut Still)).collect()
    };
    assert!(reference.iter().all(|fr| fr.errors.is_empty()), "{:?}", reference[1].errors);
    assert_eq!((reference[0].fractured.clone(), reference[1].fractured.clone()), (vec![false], vec![true]));
    for (k, t) in times.iter().enumerate() {
        // fresh, for that time alone
        assert_eq!(make().frame_at(*t, &mut Still), reference[k], "fresh, at {t}");
        // asked for a later time first, then again
        let mut later = make();
        later.frame_at(0.9, &mut Still);
        assert_eq!(later.frame_at(*t, &mut Still), reference[k], "after a later one, at {t}");
        assert_eq!(later.frame_at(*t, &mut Still), reference[k], "again, at {t}");
    }
    // a world that keeps no frames: every frame is replayed from a checkpoint
    let mut replay = make().with_frame_log_budget(0);
    replay.frame_at(150.0 * 0.01, &mut Still);
    for (k, t) in times.iter().enumerate().rev() {
        assert_eq!(replay.frame_at(*t, &mut Still), reference[k], "replayed, at {t}");
    }
    assert!(replay.checkpoint_restores() >= 1, "the replay went back to a checkpoint");
    // and what the dust took is the same figure from the world that fired it however it got there
    let mut fresh = make();
    fresh.frame_at(0.5, &mut Still);
    let wanted = fresh.fracture_lost(0).expect("the dust took something");
    replay.frame_at(0.5, &mut Still);
    assert_eq!(replay.fracture_lost(0), Some(wanted));
}

/// The dust of [`columns`] is a box of 2 by 6 by 6 cells of a quarter of a metre (0.5 by 1.5 by 1.5 m) of 72 cells at the mass of a cell: its tensor is
/// the box's, m (b^2 + c^2) / 12 about each axis, and not a number that the cells' own moments gave back to themselves; its centre is in the units of the
/// scene (`scale` of them to a metre).
fn assert_dust_is_the_column(dust: &Dust3, scale: f64) {
    let m = 72.0 * CELL_MASS;
    assert!((dust.mass - m).abs() < 1e-9 * m);
    for (got, wanted) in dust.centre.iter().zip([0.25, 0.75, 0.75]) {
        assert!((got - wanted * scale).abs() < 1e-9 * scale, "{:?}", dust.centre);
    }
    let diagonal = [
        m * (1.5f64.powi(2) + 1.5f64.powi(2)) / 12.0,
        m * (0.5f64.powi(2) + 1.5f64.powi(2)) / 12.0,
        m * (0.5f64.powi(2) + 1.5f64.powi(2)) / 12.0,
    ];
    for i in 0..3 {
        for j in 0..3 {
            let wanted = if i == j { diagonal[i] } else { 0.0 };
            assert!(
                (dust.inertia[i][j] - wanted).abs() < 1e-9 * diagonal[0],
                "inertia [{i}][{j}] is {} and the box's is {wanted}",
                dust.inertia[i][j]
            );
        }
    }
}

#[test]
fn the_dust_is_told_to_the_world_in_the_units_of_the_scene_with_the_tensor_of_its_cells() {
    // a metre is a hundred units: the cells are 25 units a side, the centre of the dust is in units, and its mass and its tensor are in kilograms and
    // metres whatever the scale
    let (o, planes) = columns();
    let policy = FracturePolicy { min_cells: 100, max_fragments: 8, overflow: Overflow::Error };
    let scene = |ppm: f64| {
        let units = [0.25 * ppm; 3];
        // the same cells on a lattice of the cell size in units
        fracture(&o, Partition::Planes(&planes), &policy, units, DENSITY, ppm).unwrap()
    };
    for ppm in [1.0, 100.0] {
        let f = scene(ppm);
        assert_dust_is_the_column(f.dust_body.as_ref().unwrap(), ppm);
        assert!(
            (f.source.mass - 432.0 * CELL_MASS).abs() < 1e-9 * f.source.mass,
            "the mass of a cell does not depend on the scale"
        );
    }
}

#[test]
fn what_the_dust_took_is_in_the_units_of_the_scene_whatever_its_scale() {
    // the same physical body (cells of a quarter of a metre, moving at the same metres a second) in a world of a hundred units to a metre: the
    // momentum is in kilograms units a second, a hundred times that of the world in metres, and the angular momentum in kilograms units squared a
    // second, ten thousand times
    let (o, planes) = columns();
    let policy = FracturePolicy { min_cells: 100, max_fragments: 8, overflow: Overflow::Error };
    let lost = |ppm: f64| {
        let f = fracture(&o, Partition::Planes(&planes), &policy, [0.25 * ppm; 3], DENSITY, ppm).unwrap();
        let mut w = world_at_scale(&f, [1.0 * ppm, 0.4 * ppm, -0.3 * ppm], [20.0, 10.0, 40.0], 0.0, ppm).unwrap();
        let frame = w.frame_at(0.0, &mut Still);
        assert!(frame.errors.is_empty(), "{:?}", frame.errors);
        w.fracture_lost(0).unwrap()
    };
    let (metres, units) = (lost(1.0), lost(100.0));
    for i in 0..3 {
        assert!(
            (units.momentum[i] - 100.0 * metres.momentum[i]).abs() < 1e-9 * (100.0 * metres.momentum[i]).abs().max(1.0),
            "momentum {i}"
        );
        assert!(
            (units.angular_momentum[i] - 1e4 * metres.angular_momentum[i]).abs()
                < 1e-9 * (1e4 * metres.angular_momentum[i]).abs().max(1.0),
            "angular momentum {i}: {} against {}",
            units.angular_momentum[i],
            1e4 * metres.angular_momentum[i]
        );
    }
    assert!(metres.momentum.iter().any(|c| c.abs() > 1.0));
}
