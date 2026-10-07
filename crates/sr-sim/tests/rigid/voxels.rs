//! Rigid bodies made of cells: a block of cells rests where the box of the same size and mass rests, and what a
//! voxel body is made of reaches the collider unchanged (cell sizes, the scene's axes, the mass).
use sr_sim::{fields::Field, physics3d::*};

struct Still;
impl Driver3 for Still {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
}

/// Cells of a block of `dims` cells whose lattice origin is at its middle (even sizes), or at the middle of a cell.
fn block(dims: [i32; 3]) -> Vec<[i32; 3]> {
    let mut cells = Vec::new();
    for z in -dims[2] / 2..dims[2] - dims[2] / 2 {
        for y in -dims[1] / 2..dims[1] - dims[1] / 2 {
            for x in -dims[0] / 2..dims[0] - dims[0] / 2 {
                cells.push([x, y, z]);
            }
        }
    }
    cells
}

fn body(shape: Shape3, mass: f64, start: Pose3, velocity: [f64; 3], spin: [f64; 3]) -> Body3Spec {
    Body3Spec {
        kind: BodyKind::Dynamic,
        shape,
        mass,
        friction: 0.5,
        restitution: 0.0,
        linear_damping: 0.0,
        angular_damping: 0.0,
        velocity,
        angular_velocity: spin,
        group: 0,
        collides_with: None,
        sensor: false,
        fixed_rotation: false,
        bullet: false,
        activate_at: 0.,
        start,
    }
}

/// A world with a floor at scene y = 0 (y points down, so bodies are at negative y) and the given bodies; one scene unit is a metre.
fn world(step: f64, bodies: Vec<Body3Spec>) -> World3 {
    World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.,
        step,
        gravity: [0., -9.80665, 0.],
        pixels_per_meter: 1.,
        iterations: 8,
        bounds: Bounds3::Floor { y: 0. },
        joints: vec![],
        bodies,
    })
}

fn at(x: f64, y: f64, z: f64) -> Pose3 {
    Pose3 { pos: [x, y, z], rot: [0., 0., 0., 1.] }
}

#[test]
fn a_block_of_cells_rests_where_the_box_of_the_same_size_and_mass_rests() {
    // 6 by 4 by 6 cells of 0.5 by 0.25 by 0.5: a box of half extents 1.5, 0.5, 1.5, 2400 kg/m3
    let size = [0.5, 0.25, 0.5];
    let mass = 2400.0 * 3.0 * 1.0 * 3.0;
    for step in [1.0 / 60.0, 1.0 / 240.0] {
        let mut w = world(
            step,
            vec![
                body(Shape3::Voxels { size, cells: block([6, 4, 6]) }, mass, at(0., -3., 0.), [0.; 3], [0.; 3]),
                body(Shape3::Box([1.5, 0.5, 1.5]), mass, at(10., -3., 0.), [0.; 3], [0.; 3]),
            ],
        );
        let frame = w.frame_at(4.0, &mut Still);
        assert!(frame.errors.is_empty(), "{:?}", frame.errors);
        let (voxels, boxed) = (frame.bodies[0], frame.bodies[1]);
        println!(
            "VOXEL REST step {step:.4}: cells y {:.9} box y {:.9}, difference {:.2e}",
            voxels.pos[1],
            boxed.pos[1],
            voxels.pos[1] - boxed.pos[1]
        );
        // the centres are at the same height above the floor, whatever the shape of the collider
        assert!((voxels.pos[1] - boxed.pos[1]).abs() < 1e-6, "{} against {}", voxels.pos[1], boxed.pos[1]);
        // it rests on its face: the centre is half the height above the floor (the floor is at y = 0, up is minus y)
        assert!((voxels.pos[1] + 0.5).abs() < 0.01, "{}", voxels.pos[1]);
        // and at rest
        let speed = frame.velocities[0].linear.iter().map(|v| v * v).sum::<f64>().sqrt();
        assert!(speed < 1e-3, "{speed}");
        // flat on the floor: the tilt of the block is the angle of its quaternion; the box's is the reference
        let tilt = |q: [f64; 4]| 2.0 * (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt().asin();
        println!("VOXEL REST tilt of the cells {:.2e} rad, of the box {:.2e} rad", tilt(voxels.rot), tilt(boxed.rot));
        assert!(tilt(voxels.rot) < 1e-4 && tilt(boxed.rot) < 1e-4, "{:?} {:?}", voxels.rot, boxed.rot);
    }
}

#[test]
fn a_stack_of_blocks_of_cells_stands_where_a_stack_of_boxes_stands() {
    // three blocks of 8 by 8 by 8 cells of 0.25 m (2 m cubes, 2400 kg/m3), one on another, dropped with small gaps
    let size = [0.25; 3];
    let mass = 2400.0 * 8.0;
    let heights = [-1.0, -3.1, -5.2];
    let stack = |voxels: bool, x: f64| -> Vec<Body3Spec> {
        heights
            .iter()
            .map(|y| {
                let shape =
                    if voxels { Shape3::Voxels { size, cells: block([8, 8, 8]) } } else { Shape3::Box([1.0; 3]) };
                body(shape, mass, at(x, *y, 0.), [0.; 3], [0.; 3])
            })
            .collect()
    };
    for step in [1.0 / 60.0, 1.0 / 120.0] {
        let mut w = world(step, stack(true, 0.).into_iter().chain(stack(false, 10.)).collect());
        // a long time, so that a stack that slowly leans or creeps shows it
        let frame = w.frame_at(8.0, &mut Still);
        assert!(frame.errors.is_empty(), "{:?}", frame.errors);
        let mut worst = 0.0f64;
        for k in 0..3 {
            let (cells, boxed) = (frame.bodies[k], frame.bodies[3 + k]);
            println!(
                "VOXEL STACK step {step:.4} block {k}: cells at ({:.5}, {:.5}, {:.5}), box at ({:.5}, {:.5}, {:.5}) from x = 10",
                cells.pos[0], cells.pos[1], cells.pos[2], boxed.pos[0] - 10.0, boxed.pos[1], boxed.pos[2]
            );
            // standing: the boxes themselves drift a few millimetres sideways in the fall, so the cells are compared with them, not
            // with the column: each coordinate within 5 mm of the boxes' (measured: 2.4 mm sideways and 0.1 mm in height at the top, which is
            // the solver's sensitivity to the tiny differences of a contact made of many points against four)
            let off = [cells.pos[0] - (boxed.pos[0] - 10.0), cells.pos[1] - boxed.pos[1], cells.pos[2] - boxed.pos[2]];
            worst = worst.max(off.iter().fold(0.0f64, |m, d| m.max(d.abs())));
            assert!(off.iter().all(|d| d.abs() < 5e-3), "block {k} is {off:?} from the box's");
            assert!(frame.velocities[k].linear.iter().all(|v| v.abs() < 1e-3), "block {k} moves");
        }
        println!("VOXEL STACK step {step:.4}: largest difference of a coordinate from the box stack {worst:.2e} m; tops at {:.6} and {:.6}", frame.bodies[2].pos[1], frame.bodies[5].pos[1]);
    }
}
