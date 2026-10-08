//! A body of cells as pieces joined at their shared faces, for the world to break by stress: the pieces have the mass, centre and second moment of their cells, the joints the
//! exact section of the faces that their pieces share in the axes of the physics and in metres, and a world given them breaks a hanging column at the load that the beam's
//! formula says.
#![allow(clippy::needless_range_loop)]

use sr_3d::occupancy::Occupancy;
use sr_3d::pieces::{Partition, Plane};
use sr_eval::voxels::{stress, StressConfig};
use sr_sim::fields::Field;
use sr_sim::physics3d::*;

const DENSITY: f64 = 2400.0;

struct Still;
impl Driver3 for Still {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
}

fn config(strength: f64) -> StressConfig {
    StressConfig { strength, min_cells: 1, overflow_to_dust: false }
}

fn occupancy(cells: &[[i32; 3]]) -> Occupancy {
    Occupancy::from_cells(cells.iter().map(|c| (*c, 1u8))).unwrap()
}

fn column(n: i32) -> Vec<[i32; 3]> {
    (0..n).map(|j| [0, j, 0]).collect()
}

#[test]
fn the_joints_of_a_column_are_the_squares_between_its_cells_in_the_axes_of_the_physics_and_in_metres() {
    // six cells of 50 scene units with 100 to the metre: cells of 0.5 m, the first at the top (y is down in the lattice)
    let o = occupancy(&column(6));
    let s = stress(&o, Partition::Labels(&|c| c[1] as u32), &config(1e6), 0, [50.0; 3], DENSITY, 100.0).unwrap();
    let e = 0.5f64;
    assert_eq!((s.pieces.len(), s.joints.len(), s.strength, s.parent), (6, 5, 1e6, 0));
    let mass = DENSITY * e.powi(3);
    for (i, p) in s.pieces.iter().enumerate() {
        assert_eq!(p.cells, vec![[0, i as i32, 0]]);
        assert!((p.mass - mass).abs() <= 1e-12 * mass);
        // the physics key of the cell is [0, -i - 1, -1]: its centre is half a cell in from the lower corner
        let want = [0.5 * e, (-(i as f64) - 0.5) * e, -0.5 * e];
        for a in 0..3 {
            assert!((p.centre[a] - want[a]).abs() < 1e-12, "piece {i} axis {a}: {:?} against {want:?}", p.centre);
        }
    }
    for (j, joint) in s.joints.iter().enumerate() {
        let y = -((j + 1) as f64) * e;
        let sec = &joint.section;
        assert_eq!((joint.a, joint.b), (j as u32, j as u32 + 1));
        assert!((sec.area - e * e).abs() < 1e-15);
        let want = [e / 2.0, y, -e / 2.0];
        for a in 0..3 {
            assert!((sec.centroid[a] - want[a]).abs() < 1e-12, "joint {j}: {:?}", sec.centroid);
        }
        // the face is across y: its normal from the upper cell to the lower is down in the lattice and, in the physics, down is -y
        assert_eq!(sec.normal, [0.0, -1.0, 0.0]);
        let second = e.powi(4) / 12.0;
        assert!(
            (sec.second[0][0] - second).abs() < 1e-15
                && (sec.second[2][2] - second).abs() < 1e-15
                && sec.second[1][1].abs() < 1e-18
        );
        assert!(sec.second[0][2].abs() < 1e-18 && sec.second[0][1].abs() < 1e-18 && sec.second[1][2].abs() < 1e-18);
        for a in 0..3 {
            assert!(
                (sec.lo[a] - [0.0, y, -e][a]).abs() < 1e-12 && (sec.hi[a] - [e, y, 0.0][a]).abs() < 1e-12,
                "joint {j}: {:?} {:?}",
                sec.lo,
                sec.hi
            );
        }
    }
}

/// A column of `n` cells hung by its first cell from the world, the strength given, as the world takes it.
fn hanging(n: i32, strength: f64) -> World3 {
    let edge = 0.5;
    let o = occupancy(&column(n));
    let s = stress(&o, Partition::Labels(&|c| c[1] as u32), &config(strength), 0, [edge; 3], DENSITY, 1.0).unwrap();
    let mass = DENSITY * edge.powi(3);
    let body = |shape: Shape3, mass: f64| Body3Spec {
        kind: BodyKind::Dynamic,
        shape,
        mass,
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
        start: Pose3 { pos: [0.0, -5.0, 0.0], rot: [0.0, 0.0, 0.0, 1.0] },
    };
    let mut bodies = vec![body(Shape3::Voxels { size: [edge; 3], cells: column(n) }, mass * f64::from(n))];
    for _ in 1..n {
        bodies.push(body(Shape3::Voxels { size: [edge; 3], cells: vec![[0, 0, 0]] }, mass));
    }
    let weld = Joint3Spec {
        kind: Joint3Kind::Weld,
        a: 0,
        b: None,
        anchor: Some([edge / 2.0, -5.0, edge / 2.0]),
        axis: [0.0, 1.0, 0.0],
        rest_length: None,
        stiffness: None,
        damping: None,
        min: None,
        max: None,
        motor_speed: 0.0,
        max_force: None,
        break_force: None,
    };
    World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.0,
        step: 1.0 / 240.0,
        gravity: [0.0, -9.80665, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![weld],
        bodies,
    })
    .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: (1..n as usize).collect() }])
    .unwrap()
    .with_stress(vec![s])
    .unwrap()
}

#[test]
fn a_world_given_the_body_by_the_builder_breaks_a_hanging_column_at_the_weight_under_the_root_over_its_area() {
    let (n, edge) = (6, 0.5f64);
    let tension = DENSITY * edge.powi(3) * 9.80665 * 5.0 / (edge * edge);
    let mut safe = hanging(n, 1.01 * tension);
    let frame = safe.frame_at(2.0, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert_eq!(safe.stress_pieces(0), Some(&(0..n as u32).collect::<Vec<_>>()[..]));
    let mut weak = hanging(n, 0.99 * tension);
    let frame = weak.frame_at(2.0, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert_eq!(weak.stress_pieces(0), Some(&[0u32][..]));
    assert_eq!(weak.stress_pieces(1), Some(&[1u32, 2, 3, 4, 5][..]));
}

#[test]
fn a_blob_cut_by_seeds_makes_pieces_that_the_world_takes_whole_and_joints_between_pieces_that_touch() {
    let mut cells = Vec::new();
    for k in 0..4 {
        for j in 0..5 {
            for i in 0..6 {
                if (i + j + k) % 7 != 3 {
                    cells.push([i, j, k]);
                }
            }
        }
    }
    let o = occupancy(&cells);
    let s = stress(&o, Partition::Voronoi { seeds: 9, seed: 5 }, &config(2e6), 0, [0.3; 3], DENSITY, 1.0).unwrap();
    // every cell once, every kilogram once, the joints a < b with a section that has an area
    let mut seen: Vec<[i32; 3]> = s.pieces.iter().flat_map(|p| p.cells.iter().copied()).collect();
    seen.sort_unstable();
    let mut want = cells.clone();
    want.sort_unstable();
    assert_eq!(seen, want);
    let total: f64 = s.pieces.iter().map(|p| p.mass).sum();
    assert!((total - cells.len() as f64 * DENSITY * 0.3f64.powi(3)).abs() < 1e-9 * total);
    assert!(!s.joints.is_empty() && s.joints.iter().all(|j| j.a < j.b && j.section.area > 0.0));
    // and a world takes it: the registration checks all of it
    let mass = total;
    let size = [0.3; 3];
    let body = |shape: Shape3, mass: f64| Body3Spec {
        kind: BodyKind::Dynamic,
        shape,
        mass,
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
        start: Pose3::default(),
    };
    let mut bodies = vec![body(Shape3::Voxels { size, cells: cells.clone() }, mass)];
    for _ in 0..s.pieces.len() {
        bodies.push(body(Shape3::Voxels { size, cells: vec![[0, 0, 0]] }, mass / 100.0));
    }
    let world = World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.0,
        step: 0.01,
        gravity: [0.0; 3],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies,
    })
    .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: (1..=s.pieces.len()).collect() }])
    .unwrap();
    assert!(world.with_stress(vec![s]).is_ok());
}

#[test]
fn a_core_inside_a_shell_has_a_joint_with_no_direction_and_planes_cut_a_bar_into_two() {
    // the middle cell of a 3 x 3 x 3 block is a piece of its own: its joint with the shell is a closed surface whose normals cancel, which is a normal of zero
    let block: Vec<[i32; 3]> = (0..27).map(|n| [n % 3, (n / 3) % 3, n / 9]).collect();
    let o = occupancy(&block);
    let s =
        stress(&o, Partition::Labels(&|c| u32::from(c == [1, 1, 1])), &config(1e6), 0, [1.0; 3], DENSITY, 1.0).unwrap();
    assert_eq!(s.joints.len(), 1);
    assert_eq!(s.joints[0].section.normal, [0.0; 3]);
    assert!((s.joints[0].section.area - 6.0).abs() < 1e-12);
    // a bar cut by one plane: two pieces and one joint across the plane
    let bar = occupancy(&(0..8).map(|i| [i, 0, 0]).collect::<Vec<_>>());
    let planes = [Plane { normal: [1, 0, 0], offset: 9 }];
    let s = stress(&bar, Partition::Planes(&planes), &config(1e6), 0, [1.0; 3], DENSITY, 1.0).unwrap();
    assert_eq!((s.pieces.len(), s.joints.len()), (2, 1));
}

#[test]
fn what_cannot_be_a_body_that_breaks_is_an_error() {
    let o = occupancy(&column(3));
    let label = Partition::Labels(&|c| c[1] as u32);
    for strength in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(
            stress(&o, Partition::Labels(&|c| c[1] as u32), &config(strength), 0, [1.0; 3], DENSITY, 1.0).is_err(),
            "{strength}"
        );
    }
    assert!(stress(&o, label, &config(1e6), 0, [0.0, 1.0, 1.0], DENSITY, 1.0).is_err());
    assert!(stress(&o, Partition::Labels(&|c| c[1] as u32), &config(1e6), 0, [1.0; 3], 0.0, 1.0).is_err());
    assert!(stress(&Occupancy::new(), Partition::Labels(&|_| 0), &config(1e6), 0, [1.0; 3], DENSITY, 1.0).is_err());
    // more pieces than a body that breaks by stress takes
    let long = occupancy(&(0..(MAX_STRESS_PIECES as i32 + 1)).map(|i| [i, 0, 0]).collect::<Vec<_>>());
    assert!(stress(&long, Partition::Labels(&|c| c[0] as u32), &config(1e6), 0, [1.0; 3], DENSITY, 1.0).is_err());
}
