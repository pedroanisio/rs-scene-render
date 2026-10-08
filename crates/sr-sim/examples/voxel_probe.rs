//! What a rigid body made of cells costs: the build of the world, the step, the checkpoint, and how still a block rests.
//!
//! `cargo run --profile ci -p sr-sim --example voxel_probe -- [floor-cells-per-side] [block-cells-per-side] [box-floor|voxel-floor] [tumble]`
//!
//! A static floor of `n` by `n` by 4 cells of 0.1 m (a plane of boxes when `box-floor` is given, which is the reference), and a
//! dynamic block of `m` cubed cells of 0.1 m dropped 0.2 m onto it at 1/60 s steps. Prints the time to build the world,
//! the mean time of a step while it falls and settles (the first 90 steps) and while it rests (the next 120), the bytes the
//! checkpoints hold, and the largest height change and speed of the block in the last 60 steps. With `tumble` the block starts
//! with a velocity and a spin, so that it keeps moving over the floor (a body that has come to rest is put to sleep and costs
//! nothing) and the step time that is printed is the one of 180 steps of that.

use sr_sim::{fields::Field, physics3d::*};
use std::time::Instant;

struct Still;
impl Driver3 for Still {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
}

fn cube(n: i32, thick: i32) -> Vec<[i32; 3]> {
    let (lo, hi) = (-n / 2, n - n / 2);
    let mut cells = Vec::with_capacity((n * n * thick) as usize);
    for z in lo..hi {
        for y in 0..thick {
            for x in lo..hi {
                cells.push([x, y, z]);
            }
        }
    }
    cells
}

fn body(kind: BodyKind, shape: Shape3, mass: f64, pos: [f64; 3]) -> Body3Spec {
    Body3Spec {
        kind,
        shape,
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
        start: Pose3 { pos, rot: [0., 0., 0., 1.] },
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let n: i32 = args.next().map_or(50, |v| v.parse().expect("floor cells per side"));
    let m: i32 = args.next().map_or(16, |v| v.parse().expect("block cells per side"));
    let box_floor = args.next().as_deref() == Some("box-floor");
    let tumble = args.next().as_deref() == Some("tumble");
    let size = [0.1; 3];
    let built = Instant::now();
    // the block: m cubed cells centred on its own origin, 2400 kg/m3, dropped from 0.2 m above the floor's top (y = 0, up is minus y)
    let half = 0.1 * f64::from(m) / 2.0;
    let block_cells: Vec<[i32; 3]> = {
        let mut c = Vec::new();
        for z in -m / 2..m - m / 2 {
            for y in -m / 2..m - m / 2 {
                for x in -m / 2..m - m / 2 {
                    c.push([x, y, z]);
                }
            }
        }
        c
    };
    let mass = 2400.0 * (0.1f64 * f64::from(m)).powi(3);
    let floor = if box_floor {
        None
    } else {
        Some(body(BodyKind::Static, Shape3::Voxels { size, cells: cube(n, 4) }, 1.0, [0.; 3]))
    };
    let mut bodies = Vec::new();
    bodies.extend(floor);
    let mut moving =
        body(BodyKind::Dynamic, Shape3::Voxels { size, cells: block_cells }, mass, [0., -(half + 0.2), 0.]);
    if tumble {
        moving.velocity = [3.0, 0.0, 1.0];
        moving.angular_velocity = [120.0, 0.0, 200.0];
    }
    bodies.push(moving);
    let mut world = World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.,
        step: 1. / 60.,
        gravity: [0., -9.80665, 0.],
        pixels_per_meter: 1.,
        iterations: 8,
        bounds: if box_floor { Bounds3::Floor { y: 0. } } else { Bounds3::None },
        joints: vec![],
        bodies,
    });
    let build = built.elapsed();
    let index = usize::from(!box_floor);
    let mut driver = Still;
    let mut run = |world: &mut World3, from: usize, to: usize| {
        let started = Instant::now();
        for k in from..to {
            let frame = world.frame_at(k as f64 / 60.0, &mut driver);
            assert!(frame.errors.is_empty(), "{:?}", frame.errors);
        }
        started.elapsed().as_secs_f64() * 1000.0 / (to - from) as f64
    };
    let falling = run(&mut world, 1, 91);
    let resting = run(&mut world, 91, 151);
    let mut heights = Vec::new();
    let mut speeds = Vec::new();
    for k in 151..211 {
        let frame = world.frame_at(k as f64 / 60.0, &mut Still);
        heights.push(frame.bodies[index].pos[1]);
        speeds.push(frame.velocities[index].linear.iter().map(|v| v * v).sum::<f64>().sqrt());
    }
    let spread = heights.iter().cloned().fold(f64::MIN, f64::max) - heights.iter().cloned().fold(f64::MAX, f64::min);
    let fastest = speeds.iter().cloned().fold(0.0, f64::max);
    println!(
        "{}floor {} ({} cells), block {} cells: build {:.1} ms; step {:.3} ms falling, {:.3} ms at rest; checkpoints {} bytes; rest y {:.6}, height spread {:.2e} m, top speed {:.2e} m/s",
        if tumble { "tumbling, " } else { "" },
        if box_floor { "box".to_string() } else { format!("{n}x{n}x4") },
        if box_floor { 1 } else { n * n * 4 },
        m * m * m,
        build.as_secs_f64() * 1000.0,
        falling,
        resting,
        world.checkpoint_bytes(),
        heights.last().copied().unwrap_or(f64::NAN),
        spread,
        fastest
    );
}
