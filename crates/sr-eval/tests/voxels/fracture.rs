//! A body of cells divided into pieces (the partition of `sr_3d::pieces`) that are bodies of cells for the rigid world's fracture: every cell
//! and every kilogram of the source is in one piece, and the world given them breaks the source into them with the motion it had.
#![allow(clippy::needless_range_loop)]

use sr_3d::occupancy::Occupancy;
use sr_3d::pieces::{partition, Partition, Plane};
use sr_eval::voxels::{fracture, Fractured};
use sr_sim::fields::Field;
use sr_sim::physics3d::*;
use std::collections::BTreeSet;

const SIZE: [f64; 3] = [0.25; 3];
const DENSITY: f64 = 2400.0;
const CELL_MASS: f64 = 2400.0 * 0.25 * 0.25 * 0.25;

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

#[test]
fn every_cell_and_every_kilogram_of_the_source_is_in_exactly_one_piece() {
    let o = block(12, 6, 6);
    let f = fracture(&o, Partition::Voronoi { seeds: 5, seed: 7 }, 16, SIZE, DENSITY, 1.0).unwrap();
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
    // the pieces are the graph's, in its order, with its joints
    assert_eq!(f.graph, partition(&o, Partition::Voronoi { seeds: 5, seed: 7 }, 16).unwrap());
    assert_eq!(f.graph.pieces.len(), f.pieces.len());
    assert!(!f.graph.edges.is_empty());
}

#[test]
fn the_pieces_do_not_depend_on_the_order_the_cells_were_put_in() {
    let o = block(9, 5, 4);
    let reversed =
        Occupancy::from_cells(o.cells().collect::<Vec<_>>().into_iter().rev().map(|c| (c, o.get(c)))).unwrap();
    let rule = || Partition::Planes(&[Plane { normal: [1, 0, 0], offset: 9 }, Plane { normal: [0, 1, 0], offset: 5 }]);
    let a = fracture(&o, rule(), 8, SIZE, DENSITY, 1.0).unwrap();
    let b = fracture(&reversed, rule(), 8, SIZE, DENSITY, 1.0).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.pieces.len(), 4, "two planes cut the block into four");
}

#[test]
fn what_cannot_be_broken_is_an_error_that_says_why() {
    let o = block(8, 4, 4);
    let many = fracture(&o, Partition::Voronoi { seeds: 12, seed: 3 }, 3, SIZE, DENSITY, 1.0).unwrap_err();
    assert!(many.contains("piece"), "{many}");
    let empty = Occupancy::from_cells(Vec::<([i32; 3], u8)>::new()).unwrap();
    assert!(fracture(&empty, Partition::Voronoi { seeds: 2, seed: 1 }, 4, SIZE, DENSITY, 1.0).is_err());
    for (size, density, ppm) in [([0.0; 3], DENSITY, 1.0), (SIZE, 0.0, 1.0), (SIZE, DENSITY, f64::NAN)] {
        assert!(fracture(&o, Partition::Voronoi { seeds: 2, seed: 1 }, 4, size, density, ppm).is_err());
    }
}

/// The source at rest but for a velocity, nothing else in the world.
struct Still;
impl Driver3 for Still {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
}

fn spec(f: &Fractured) -> Body3Spec {
    let body = |b: &sr_eval::voxels::Body| Body3Spec {
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
        start: Pose3::default(),
    };
    let _ = f;
    Body3Spec { velocity: [1.0, 0.4, -0.3], ..body(&f.source) }
}

#[test]
fn the_world_given_the_pieces_breaks_the_source_into_them_with_the_velocity_it_had() {
    let o = block(10, 6, 6);
    let f = fracture(&o, Partition::Voronoi { seeds: 4, seed: 11 }, 8, SIZE, DENSITY, 1.0).unwrap();
    let mut bodies = vec![spec(&f)];
    for p in &f.pieces {
        let mut b = spec(&f);
        b.shape = p.shape.clone();
        b.mass = p.mass;
        b.velocity = [0.; 3];
        bodies.push(b);
    }
    let fragments = (1..=f.pieces.len()).map(|k| Fragment3 { body: k, offset: [0.0; 3], impulse: [0.0; 3] }).collect();
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
    .with_fractures(vec![Fracture3 { source: 0, at: 0.2, radial_impulse: 0.0, fragments, contact: None }])
    .unwrap();
    let frame = w.frame_at(0.3, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert_eq!(frame.fractured, vec![true]);
    let shown: BTreeSet<usize> = (0..frame.enabled.len()).filter(|k| frame.enabled[*k]).collect();
    assert_eq!(shown, (1..=f.pieces.len()).collect::<BTreeSet<_>>());
    // pieces of one body with no spin and no impulse all move as it moved
    for k in 1..=f.pieces.len() {
        for a in 0..3 {
            assert!((frame.velocities[k].linear[a] - [1.0, 0.4, -0.3][a]).abs() < 1e-9, "piece {k} axis {a}");
            assert!(frame.velocities[k].angular[a].abs() < 1e-9);
        }
    }
}
