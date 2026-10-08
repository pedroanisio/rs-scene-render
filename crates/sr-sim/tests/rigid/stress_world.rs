//! A body of cells that breaks by stress, in the world: a cantilever of cubes welded to the world at its root with a weight on its tip. The oracles are the beam's: the
//! bending moment and the shear at each joint (the weight of what is beyond it and of the load), the section modulus of the joint, and the load that makes the
//! principal tension the strength.
#![allow(clippy::needless_range_loop)]

use sr_sim::fields::Field;
use sr_sim::physics3d::*;
use sr_sim::stress::JointSection;

pub const G: f64 = 9.80665;
pub const DENSITY: f64 = 2400.0;

pub struct Still;
impl Driver3 for Still {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
}

pub fn body(shape: Shape3, mass: f64, at: [f64; 3]) -> Body3Spec {
    Body3Spec {
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
        start: Pose3 { pos: at, rot: [0.0, 0.0, 0.0, 1.0] },
    }
}

/// The joint between the pieces `i` and `i + 1` of a row of cubes of `edge` metres along x, in the frame of the body (physics axes): a square of the edge at x = (i + 1) edge.
pub fn row_joint(i: u32, edge: f64) -> StressJoint3 {
    let x = f64::from(i + 1) * edge;
    let second = edge.powi(4) / 12.0;
    StressJoint3 {
        a: i,
        b: i + 1,
        section: JointSection {
            area: edge * edge,
            centroid: [x, -edge / 2.0, -edge / 2.0],
            second: [[0.0; 3], [0.0, second, 0.0], [0.0, 0.0, second]],
            normal: [1.0, 0.0, 0.0],
            lo: [x, -edge, -edge],
            hi: [x, 0.0, 0.0],
        },
    }
}

/// The principal tension of the joint after the piece `j` of a cantilever of `n` cubes of `edge` metres that carries a weight of `load` newtons on its tip piece:
/// the weight of the pieces beyond the joint and the load make its bending moment and its shear.
pub fn beam_principal(j: usize, n: usize, edge: f64, load: f64) -> f64 {
    let mass = DENSITY * edge.powi(3);
    let beyond = (n - 1 - j) as f64;
    let weight = mass * G * beyond;
    // the joint is at x = (j + 1) edge, the middle of the pieces beyond it at (j + 1 + beyond / 2) edge, and the load at the middle of the tip piece
    let joint = (j + 1) as f64 * edge;
    let moment = weight * (beyond / 2.0) * edge + load * (n as f64 - 0.5) * edge - load * joint;
    let shear = (weight + load) / (edge * edge);
    let sigma = moment / (edge.powi(3) / 6.0);
    sigma / 2.0 + (sigma * sigma / 4.0 + shear * shear).sqrt()
}

pub struct Cantilever {
    pub world: World3,
    pub n: usize,
}

/// A cantilever of `n` cubes of `edge` metres, welded to the world at its root, 5 m above nothing, with a block of `load` newtons of weight resting on its tip,
/// that breaks by stress at `strength` pascals. The bodies are the beam (0), the block (1) and the slots (2 and on, one for each piece after the first).
pub fn cantilever(n: usize, edge: f64, strength: f64, load: f64) -> Cantilever {
    cantilever_over(n, edge, strength, load, false)
}

/// The same, with a floor under it if `floor`: what falls when the root breaks lands on it.
pub fn cantilever_over(n: usize, edge: f64, strength: f64, load: f64, floor: bool) -> Cantilever {
    cantilever_with(n, edge, strength, load, floor, (0.0, 1e-6))
}

/// The same, with the block moving along x at `block.0` m/s and `block.1` metres over the top of the tip piece when it starts: a block that lands sliding drags the tip
/// with its friction.
pub fn cantilever_with(n: usize, edge: f64, strength: f64, load: f64, floor: bool, block: (f64, f64)) -> Cantilever {
    cantilever_mu(n, edge, strength, load, floor, block, (0.5, 0.5))
}

/// The same with the friction coefficients of the beam and of the block, which the world combines by the rule of Rapier's default for two colliders (their average).
pub fn cantilever_mu(
    n: usize,
    edge: f64,
    strength: f64,
    load: f64,
    floor: bool,
    block: (f64, f64),
    mu: (f64, f64),
) -> Cantilever {
    let cells: Vec<[i32; 3]> = (0..n as i32).map(|i| [i, 0, 0]).collect();
    let mass = DENSITY * edge.powi(3);
    let size = [edge; 3];
    let mut bodies = vec![
        body(Shape3::Voxels { size, cells: cells.clone() }, mass * n as f64, [0.0, -5.0, 0.0]),
        // the block rests on the top of the tip piece (the top is at y = -5 in the scene's axes, y down)
        {
            let mut b = body(
                Shape3::Box([edge / 2.0; 3]),
                load / G,
                [(n as f64 - 0.5) * edge, -5.0 - edge / 2.0 - block.1, edge / 2.0],
            );
            b.velocity = [block.0, 0.0, 0.0];
            b.friction = mu.1;
            b
        },
    ];
    bodies[0].friction = mu.0;
    for _ in 1..n {
        bodies.push(body(Shape3::Voxels { size, cells: vec![[0, 0, 0]] }, mass, [0.0, -5.0, 0.0]));
    }
    let weld = Joint3Spec {
        kind: Joint3Kind::Weld,
        a: 0,
        b: None,
        anchor: Some([0.0, -5.0 + edge / 2.0, edge / 2.0]),
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
    let pieces: Vec<StressPiece3> =
        (0..n as i32).map(|i| StressPiece3::from_cells(&[[i, 0, 0]], size, mass).unwrap()).collect();
    let joints: Vec<StressJoint3> = (0..n as u32 - 1).map(|i| row_joint(i, edge)).collect();
    let world = World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.0,
        step: 1.0 / 240.0,
        gravity: [0.0, -G, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: if floor { Bounds3::Floor { y: 0.0 } } else { Bounds3::None },
        joints: vec![weld],
        bodies,
    })
    .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: (2..2 + n - 1).collect() }])
    .unwrap()
    .with_stress(vec![Stress3 { parent: 0, strength, pieces, joints, min_cells: 1, overflow_to_dust: false }])
    .unwrap();
    Cantilever { world, n }
}

#[test]
fn the_load_in_each_joint_of_a_cantilever_at_rest_is_the_beam_s_to_the_exact_section_modulus() {
    let (n, edge, load) = (5, 0.4, 3000.0);
    let mut c = cantilever(n, edge, 1e12, load);
    let frame = c.world.frame_at(1.5, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    let levels = c.world.stress_levels(0);
    assert_eq!(levels.len(), n - 1);
    for (j, (joint, principal)) in levels.iter().enumerate() {
        assert_eq!(*joint as usize, j);
        let want = beam_principal(j, n, edge, load);
        // the solver holds a weld and a resting block to a few ten-thousandths of the load (4.5e-4 to 5.1e-4 measured in the four joints: the weld's give)
        assert!((principal / want - 1.0).abs() < 1e-3, "joint {j}: {principal} against {want}");
    }
}

/// The load (newtons) on the tip that makes the principal tension of the joint `j` of the beam the strength: the principal tension grows with the load, so this finds it
/// by halving, to the last digit that a double has.
pub fn breaking_load(j: usize, n: usize, edge: f64, strength: f64) -> f64 {
    let (mut lo, mut hi) = (0.0, 1.0);
    while beam_principal(j, n, edge, hi) < strength {
        hi *= 2.0;
    }
    for _ in 0..200 {
        let mid = (lo + hi) / 2.0;
        if beam_principal(j, n, edge, mid) < strength {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

fn broken_joints(c: &Cantilever) -> Vec<usize> {
    (0..c.n - 1).filter(|&j| c.world.stress_joint_broken(0, j) == Some(true)).collect()
}

#[test]
fn a_load_under_the_one_that_breaks_the_root_breaks_nothing_in_two_seconds_and_one_over_it_breaks_the_root_and_only_it()
{
    let (n, edge, strength) = (5, 0.4, 8.0e5);
    // the root joint (0) is the one with the greatest moment: this is the load that makes its principal tension the strength, and the others have room
    let target = breaking_load(0, n, edge, strength);
    for j in 1..n - 1 {
        assert!(breaking_load(j, n, edge, strength) > 1.3 * target, "joint {j}");
    }
    let mut under = cantilever(n, edge, strength, 0.995 * target);
    let frame = under.world.frame_at(2.0, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert_eq!(broken_joints(&under), Vec::<usize>::new(), "0.995 of the load that breaks it breaks something");
    assert_eq!(under.world.stress_pieces(0), Some(&[0u32, 1, 2, 3, 4][..]));
    let mut over = cantilever(n, edge, strength, 1.005 * target);
    let frame = over.world.frame_at(2.0, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    // the root joint (between the pieces 0 and 1) broke, the part that the weld holds (the piece 0) stayed the body, and the other four took a slot and fell
    assert_eq!(broken_joints(&over), vec![0]);
    assert_eq!(over.world.stress_pieces(0), Some(&[0u32][..]));
    assert_eq!(over.world.stress_pieces(2), Some(&[1u32, 2, 3, 4][..]));
    // and they fell: the piece is well below where it was (y is down in the scene's axes)
    assert!(
        frame.bodies[2].pos[1] > frame.bodies[0].pos[1] + 5.0,
        "{:?} {:?}",
        frame.bodies[2].pos,
        frame.bodies[0].pos
    );
}

/// A column of `n` cubes of `edge` metres, one above the next (the cell `[0, j, 0]` is the j-th from the top), as the pieces 0 to n - 1. With `hanging` it is welded to the
/// world by the top face of its first cube, 5 m above nothing; without, it stands on a floor, its bottom on it.
pub fn column(n: usize, edge: f64, strength: f64, hanging: bool) -> World3 {
    let cells: Vec<[i32; 3]> = (0..n as i32).map(|j| [0, j, 0]).collect();
    let mass = DENSITY * edge.powi(3);
    let size = [edge; 3];
    // the scene's y is down: the column's first cube is the top one, the origin of the lattice is its top corner
    let top = if hanging { -5.0 } else { -(n as f64) * edge };
    let mut bodies = vec![body(Shape3::Voxels { size, cells }, mass * n as f64, [0.0, top, 0.0])];
    for _ in 1..n {
        bodies.push(body(Shape3::Voxels { size, cells: vec![[0, 0, 0]] }, mass, [0.0, top, 0.0]));
    }
    let joints = if hanging {
        vec![Joint3Spec {
            kind: Joint3Kind::Weld,
            a: 0,
            b: None,
            anchor: Some([edge / 2.0, top, edge / 2.0]),
            axis: [0.0, 1.0, 0.0],
            rest_length: None,
            stiffness: None,
            damping: None,
            min: None,
            max: None,
            motor_speed: 0.0,
            max_force: None,
            break_force: None,
        }]
    } else {
        vec![]
    };
    let pieces: Vec<StressPiece3> =
        (0..n as i32).map(|j| StressPiece3::from_cells(&[[0, j, 0]], size, mass).unwrap()).collect();
    let joint = |j: u32| {
        let y = -f64::from(j + 1) * edge;
        let second = edge.powi(4) / 12.0;
        StressJoint3 {
            a: j,
            b: j + 1,
            section: JointSection {
                area: edge * edge,
                centroid: [edge / 2.0, y, -edge / 2.0],
                second: [[second, 0.0, 0.0], [0.0; 3], [0.0, 0.0, second]],
                normal: [0.0, -1.0, 0.0],
                lo: [0.0, y, -edge],
                hi: [edge, y, 0.0],
            },
        }
    };
    let joints_between: Vec<StressJoint3> = (0..n as u32 - 1).map(joint).collect();
    World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.0,
        step: 1.0 / 240.0,
        gravity: [0.0, -G, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: if hanging { Bounds3::None } else { Bounds3::Floor { y: 0.0 } },
        joints,
        bodies,
    })
    .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: (1..n).collect() }])
    .unwrap()
    .with_stress(vec![Stress3 {
        parent: 0,
        strength,
        pieces,
        joints: joints_between,
        min_cells: 1,
        overflow_to_dust: false,
    }])
    .unwrap()
}

#[test]
fn a_column_that_hangs_breaks_where_the_weight_below_a_joint_is_the_strength_and_a_column_that_stands_does_not_break_whatever_the_strength(
) {
    let (n, edge) = (6, 0.5f64);
    // the root joint carries the weight of the five cubes under it over one face: the strength that is that, a little over and a little under
    let tension = DENSITY * edge.powi(3) * G * 5.0 / (edge * edge);
    let mut safe = column(n, edge, 1.01 * tension, true);
    let frame = safe.frame_at(2.0, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert_eq!(
        safe.stress_pieces(0),
        Some(&(0..n as u32).collect::<Vec<_>>()[..]),
        "nothing broke at 1.01 of the strength that breaks it"
    );
    let mut weak = column(n, edge, 0.99 * tension, true);
    let frame = weak.frame_at(2.0, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    // the root joint broke, the first cube stayed with the weld and the five below fell as one body, and nothing else broke in the fall
    assert_eq!(weak.stress_joint_broken(0, 0), Some(true));
    assert!((1..n - 1).all(|j| weak.stress_joint_broken(0, j) == Some(false)));
    assert_eq!(weak.stress_pieces(0), Some(&[0u32][..]));
    assert_eq!(weak.stress_pieces(1), Some(&[1u32, 2, 3, 4, 5][..]));
    // a column that stands on a floor is in compression (59 kPa at its foot): with a strength of 100 pascals, which any pull or shear of the first steps of settling
    // is under (a few pascals) and the compression is far over, it does not break, in two seconds
    let mut standing = column(n, edge, 100.0, false);
    let frame = standing.frame_at(2.0, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert_eq!(
        standing.stress_pieces(0),
        Some(&(0..n as u32).collect::<Vec<_>>()[..]),
        "a column that stands is in compression and breaks nothing"
    );
}

use sr_sim::stress::balance::{momentum, spin_momentum, MassSum, Rigid};

/// A free beam of `n` cubes of `edge` metres (no weld, no gravity), the pieces in a row, spinning about z at `omega` rad/s about its own centre and moving
/// at `drift` m/s: what a body that breaks in flight is, with `slots` slots for the pieces. The parts are the bodies 0 (the beam) and 1 and on (the slots).
#[derive(Clone, Copy)]
pub struct Flyer {
    pub n: usize,
    pub edge: f64,
    pub strength: f64,
    pub omega: f64,
    pub drift: [f64; 3],
    pub slots: usize,
    pub min_cells: usize,
    pub dust: bool,
    /// Two walls (static boxes) at this distance from the middle on each side, along y, if any: what the halves of a beam that has broken in two fly into (a beam that spins about z
    /// sends its halves off along +y and -y).
    pub walls: Option<f64>,
}

impl Flyer {
    pub fn world(&self) -> World3 {
        let Flyer { n, edge, strength, omega, drift, slots, min_cells, dust, walls } = *self;
        let cells: Vec<[i32; 3]> = (0..n as i32).map(|i| [i, 0, 0]).collect();
        let mass = DENSITY * edge.powi(3);
        let size = [edge; 3];
        // the beam's frame: x along it; its centre of mass is at n edge / 2 along x, edge / 2 down y and edge / 2 along z: the body is placed so that the centre is at the origin
        let centre = [n as f64 * edge / 2.0, edge / 2.0, edge / 2.0];
        let mut beam = body(Shape3::Voxels { size, cells }, mass * n as f64, [-centre[0], -centre[1], -centre[2]]);
        beam.velocity = drift;
        // the spin is about the scene's z, in degrees a second
        beam.angular_velocity = [0.0, 0.0, omega.to_degrees()];
        let mut bodies = vec![beam];
        for _ in 0..slots {
            bodies.push(body(Shape3::Voxels { size, cells: vec![[0, 0, 0]] }, mass, [0.0; 3]));
        }
        if let Some(d) = walls {
            for side in [-1.0, 1.0] {
                let mut wall = body(Shape3::Box([3.0, 0.1, 3.0]), 1e6, [0.0, side * (d + 0.1), 0.0]);
                wall.kind = BodyKind::Static;
                bodies.push(wall);
            }
        }
        let pieces: Vec<StressPiece3> =
            (0..n as i32).map(|i| StressPiece3::from_cells(&[[i, 0, 0]], size, mass).unwrap()).collect();
        let joints: Vec<StressJoint3> = (0..n as u32 - 1).map(|i| row_joint(i, edge)).collect();
        World3::new(World3Spec {
            fix_internal_edges: false,
            start: 0.0,
            step: 1.0 / 240.0,
            gravity: [0.0; 3],
            pixels_per_meter: 1.0,
            iterations: 8,
            bounds: Bounds3::None,
            joints: vec![],
            bodies,
        })
        .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: (1..=slots).collect() }])
        .unwrap()
        .with_stress(vec![Stress3 { parent: 0, strength, pieces, joints, min_cells, overflow_to_dust: dust }])
        .unwrap()
    }
}

/// The mass sum of the pieces of a flyer that body `k` holds.
fn held_mass(w: &World3, k: usize, n: usize, edge: f64) -> Option<MassSum> {
    let held = w.stress_pieces(k)?;
    let mass = DENSITY * edge.powi(3);
    let size = [edge; 3];
    let mut sum = MassSum::default();
    for &i in held {
        let p = StressPiece3::from_cells(&[[i as i32, 0, 0]], size, mass).unwrap();
        sum.add(&MassSum { mass: p.mass, first: p.centre.map(|c| c * p.mass), second: p.second });
    }
    let _ = n;
    Some(sum)
}

/// The linear momentum, the angular momentum about the origin of the world and the kinetic energy of all the parts of a flyer in a frame.
fn invariants(w: &World3, frame: &Frame3, n: usize, edge: f64) -> ([f64; 3], [f64; 3], f64) {
    let (mut p, mut l, mut energy) = ([0.0; 3], [0.0; 3], 0.0);
    for k in 0..frame.bodies.len() {
        let Some(sum) = held_mass(w, k, n, edge) else { continue };
        if !frame.enabled[k] {
            continue;
        }
        // the frame is in the scene's axes: the physics' are x, -y, -z
        let flip = |v: [f64; 3]| [v[0], -v[1], -v[2]];
        let q = frame.bodies[k].rot;
        let q = [q[0], -q[1], -q[2], q[3]];
        let (x, y, z, wq) = (q[0], q[1], q[2], q[3]);
        let rotation = [
            [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * wq), 2.0 * (x * z + y * wq)],
            [2.0 * (x * y + z * wq), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * wq)],
            [2.0 * (x * z - y * wq), 2.0 * (y * z + x * wq), 1.0 - 2.0 * (x * x + y * y)],
        ];
        let rigid = Rigid {
            position: flip(frame.bodies[k].pos),
            rotation,
            linear: flip(frame.velocities[k].linear),
            angular: flip(frame.velocities[k].angular).map(f64::to_radians),
            centre: sum.centre(),
        };
        let mom = momentum(&sum, &rigid);
        let com = rigid.world(sum.centre());
        let spin = spin_momentum(&sum, &rigid);
        let orbital =
            [com[1] * mom[2] - com[2] * mom[1], com[2] * mom[0] - com[0] * mom[2], com[0] * mom[1] - com[1] * mom[0]];
        for a in 0..3 {
            p[a] += mom[a];
            l[a] += spin[a] + orbital[a];
        }
        let v = rigid.linear;
        energy += 0.5 * sum.mass * (v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
            + 0.5 * (0..3).map(|a| rigid.angular[a] * spin[a]).sum::<f64>();
    }
    (p, l, energy)
}

#[test]
fn a_body_that_breaks_in_flight_keeps_its_momentum_its_angular_momentum_and_its_energy() {
    let (n, edge, omega) = (5, 0.4f64, 12.0);
    // the joint between the pieces 1 and 2 and the one between 2 and 3 carry the pull of the two pieces on one side of each: m_S w^2 r over the area
    let mass = DENSITY * edge.powi(3);
    let tension = 2.0 * mass * omega * omega * 1.5 * edge / (edge * edge);
    let mut w = Flyer {
        n,
        edge,
        strength: 0.97 * tension,
        omega,
        drift: [3.0, -2.0, 1.0],
        slots: 4,
        min_cells: 1,
        dust: false,
        walls: None,
    }
    .world();
    let mut broke_at = None;
    let mut readings = Vec::new();
    for step in 1..=240u64 {
        let frame = w.frame_at(step as f64 / 240.0, &mut Still);
        assert!(frame.errors.is_empty(), "{:?}", frame.errors);
        readings.push((step, invariants(&w, &frame, n, edge)));
        if broke_at.is_none() && w.stress_pieces(0).is_some_and(|h| h.len() < n) {
            broke_at = Some(step);
        }
    }
    let at = broke_at.expect("the beam breaks: the stress at its middle joints is over the strength");
    // the break is at once: the two joints that are over strength at the end of one step, both gone at the start of the next
    assert_eq!(w.stress_pieces(0), Some(&[0u32, 1][..]));
    assert!(w.stress_joint_broken(0, 1) == Some(true) && w.stress_joint_broken(0, 2) == Some(true));
    assert_eq!(w.stress_pieces(1), Some(&[2u32][..]));
    assert_eq!(w.stress_pieces(2), Some(&[3u32, 4][..]));
    // momentum, angular momentum and energy: every step before and after the break is the one of the first
    let (p0, l0, e0) = readings[0].1;
    let mut checked_after = 0;
    for (step, (p, l, e)) in &readings {
        for a in 0..3 {
            assert!(
                (p[a] - p0[a]).abs() <= 1e-9 * p0.iter().map(|v| v.abs()).fold(1.0, f64::max),
                "step {step} momentum {a}: {p:?} against {p0:?}"
            );
            assert!(
                (l[a] - l0[a]).abs() <= 1e-9 * l0.iter().map(|v| v.abs()).fold(1.0, f64::max),
                "step {step} angular momentum {a}: {l:?} against {l0:?}"
            );
        }
        assert!((e - e0).abs() <= 1e-9 * e0, "step {step} energy {e} against {e0}");
        if *step > at {
            checked_after += 1;
        }
    }
    assert!(checked_after > 100, "{checked_after} steps after the break");
}

fn bits(frame: &Frame3) -> Vec<u64> {
    frame
        .bodies
        .iter()
        .flat_map(|b| b.pos.iter().chain(&b.rot).map(|v| v.to_bits()).collect::<Vec<_>>())
        .chain(
            frame
                .velocities
                .iter()
                .flat_map(|v| v.linear.iter().chain(&v.angular).map(|c| c.to_bits()).collect::<Vec<_>>()),
        )
        .collect()
}

#[test]
fn a_body_that_breaks_gives_the_same_bits_in_a_fresh_world_stepped_to_it_by_jumps_and_after_a_seek_back() {
    let (n, edge, strength) = (5, 0.4f64, 8.0e5);
    let load = 1.005 * breaking_load(0, n, edge, strength);
    let mut direct = cantilever(n, edge, strength, load);
    let want = bits(&direct.world.frame_at(2.0, &mut Still));
    let held: Vec<Option<Vec<u32>>> =
        (0..2 + n - 1).map(|k| direct.world.stress_pieces(k).map(<[u32]>::to_vec)).collect();
    assert!(held[2].is_some(), "the beam broke in this run");
    // the same world asked in jumps, then back to the middle and forward again
    let mut jumps = cantilever(n, edge, strength, load);
    for t in [0.3, 0.7, 0.9, 1.4, 2.0] {
        let _ = jumps.world.frame_at(t, &mut Still);
    }
    assert_eq!(bits(&jumps.world.frame_at(2.0, &mut Still)), want);
    let mut seek = cantilever(n, edge, strength, load);
    let _ = seek.world.frame_at(2.0, &mut Still);
    let early = seek.world.frame_at(0.5, &mut Still);
    assert!(early.errors.is_empty());
    assert_eq!(bits(&seek.world.frame_at(2.0, &mut Still)), want);
    // what each body holds is the same too
    for k in 0..2 + n - 1 {
        assert_eq!(seek.world.stress_pieces(k).map(<[u32]>::to_vec), held[k], "body {k}");
        assert_eq!(jumps.world.stress_pieces(k).map(<[u32]>::to_vec), held[k], "body {k}");
    }
    // and a fresh world is the same again
    let mut fresh = cantilever(n, edge, strength, load);
    assert_eq!(bits(&fresh.world.frame_at(2.0, &mut Still)), want);
}

#[test]
fn registering_a_body_that_breaks_by_stress_changes_no_bit_of_a_body_that_does_not_break() {
    let (n, edge) = (5, 0.4f64);
    let load = 3000.0;
    // the same cantilever with and without the registration (the strength is far over any stress): reading the stress is read only
    let mut with = cantilever(n, edge, 1e15, load);
    let a = bits(&with.world.frame_at(1.5, &mut Still));
    // the same bodies and joints with no with_stress: built the same way, by hand
    let mut plain = unregistered_cantilever(n, edge, load);
    assert_eq!(a, bits(&plain.frame_at(1.5, &mut Still)));
    // a body registered as one piece (no joint to break) is the body it was
    let mut one = column_of_one_piece(true);
    let mut plain = column_of_one_piece(false);
    for t in [0.2, 1.0, 1.7] {
        assert_eq!(bits(&one.frame_at(t, &mut Still)), bits(&plain.frame_at(t, &mut Still)), "at {t}");
    }
}

/// The cantilever of [`cantilever`] without the stress: the same world to the bit of its construction.
fn unregistered_cantilever(n: usize, edge: f64, load: f64) -> World3 {
    let cells: Vec<[i32; 3]> = (0..n as i32).map(|i| [i, 0, 0]).collect();
    let mass = DENSITY * edge.powi(3);
    let size = [edge; 3];
    let mut bodies = vec![
        body(Shape3::Voxels { size, cells }, mass * n as f64, [0.0, -5.0, 0.0]),
        body(Shape3::Box([edge / 2.0; 3]), load / G, [(n as f64 - 0.5) * edge, -5.0 - edge / 2.0 - 1e-6, edge / 2.0]),
    ];
    for _ in 1..n {
        bodies.push(body(Shape3::Voxels { size, cells: vec![[0, 0, 0]] }, mass, [0.0, -5.0, 0.0]));
    }
    let weld = Joint3Spec {
        kind: Joint3Kind::Weld,
        a: 0,
        b: None,
        anchor: Some([0.0, -5.0 + edge / 2.0, edge / 2.0]),
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
        gravity: [0.0, -G, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![weld],
        bodies,
    })
    .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: (2..2 + n - 1).collect() }])
    .unwrap()
}

/// A body of three cells that tumbles on a floor, registered as one piece (with a joint of nothing to break) or not registered.
fn column_of_one_piece(registered: bool) -> World3 {
    let edge = 0.5f64;
    let cells: Vec<[i32; 3]> = (0..3).map(|i| [i, 0, 0]).collect();
    let mass = DENSITY * edge.powi(3) * 3.0;
    let size = [edge; 3];
    let mut b = body(Shape3::Voxels { size, cells: cells.clone() }, mass, [0.0, -2.0, 0.0]);
    b.angular_velocity = [30.0, 50.0, 70.0];
    let w = World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.0,
        step: 1.0 / 240.0,
        gravity: [0.0, -G, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::Floor { y: 0.0 },
        joints: vec![],
        bodies: vec![b, body(Shape3::Voxels { size, cells: vec![[0, 0, 0]] }, mass / 3.0, [0.0; 3])],
    })
    .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: vec![1] }])
    .unwrap();
    if !registered {
        return w;
    }
    let piece = StressPiece3::from_cells(&cells, size, mass).unwrap();
    w.with_stress(vec![Stress3 {
        parent: 0,
        strength: 1.0,
        pieces: vec![piece],
        joints: vec![],
        min_cells: 1,
        overflow_to_dust: false,
    }])
    .unwrap()
}

/// A flyer whose joints all break in the first step that reads them (a strength of a pascal): the pieces of the beam all come apart, the largest staying.
fn all_apart(slots: usize, min_cells: usize, dust: bool) -> World3 {
    let n = 5;
    // it spins, so that there is a load to read at once
    Flyer { n, edge: 0.4, strength: 1.0, omega: 12.0, drift: [0.0; 3], slots, min_cells, dust, walls: None }.world()
}

#[test]
fn every_piece_that_comes_loose_takes_a_slot_in_the_order_of_its_lowest_piece_and_the_one_that_stays_is_the_first_of_the_largest(
) {
    let mut w = all_apart(4, 1, false);
    let frame = w.frame_at(0.1, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    // five pieces of one cell: the first stays the body, and the others take the four slots in order
    assert_eq!(w.stress_pieces(0), Some(&[0u32][..]));
    for slot in 1..=4usize {
        assert_eq!(w.stress_pieces(slot), Some(&[slot as u32][..]), "slot {slot}");
    }
}

#[test]
fn loose_parts_of_fewer_cells_than_the_minimum_are_dust_and_hold_no_piece() {
    // a minimum of two cells: no loose part of one cell is a body, so the four others are dust and only the first piece is left
    let mut w = all_apart(4, 2, false);
    let frame = w.frame_at(0.1, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert_eq!(w.stress_pieces(0), Some(&[0u32][..]));
    for slot in 1..=4usize {
        assert_eq!(w.stress_pieces(slot), None, "slot {slot} holds nothing");
    }
}

#[test]
fn more_loose_parts_than_slots_is_an_error_that_names_both_numbers_or_the_smallest_are_dust() {
    // four loose parts and two slots
    let mut w = all_apart(2, 1, false);
    let frame = w.frame_at(0.1, &mut Still);
    assert!(
        frame.errors.iter().any(|e| e.contains("4 loose parts") && e.contains("2 slots") && e.contains("maxFragments")),
        "{:?}",
        frame.errors
    );
    // with the overflow to dust: the largest keep the slots, of equals the first, and the others are dust
    let mut w = all_apart(2, 1, true);
    let frame = w.frame_at(0.1, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert_eq!(w.stress_pieces(0), Some(&[0u32][..]));
    assert_eq!(w.stress_pieces(1), Some(&[1u32][..]));
    assert_eq!(w.stress_pieces(2), Some(&[2u32][..]));
}

#[test]
fn the_world_refuses_a_registration_that_is_not_a_body_of_pieces_it_can_read() {
    let size = [0.4; 3];
    let mass = DENSITY * 0.4f64.powi(3);
    let pieces: Vec<StressPiece3> =
        (0..5).map(|i| StressPiece3::from_cells(&[[i, 0, 0]], size, mass).unwrap()).collect();
    let joints: Vec<StressJoint3> = (0..4).map(|i| row_joint(i, 0.4)).collect();
    let good = Stress3 {
        parent: 0,
        strength: 1e6,
        pieces: pieces.clone(),
        joints: joints.clone(),
        min_cells: 1,
        overflow_to_dust: false,
    };
    // a world with the split and no stress yet
    let fresh = || {
        let mut bodies =
            vec![body(Shape3::Voxels { size, cells: (0..5).map(|i| [i, 0, 0]).collect() }, mass * 5.0, [0.0; 3])];
        for _ in 0..4 {
            bodies.push(body(Shape3::Voxels { size, cells: vec![[0, 0, 0]] }, mass, [0.0; 3]));
        }
        World3::new(World3Spec {
            fix_internal_edges: false,
            start: 0.0,
            step: 1.0 / 240.0,
            gravity: [0.0; 3],
            pixels_per_meter: 1.0,
            iterations: 8,
            bounds: Bounds3::None,
            joints: vec![],
            bodies,
        })
        .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: (1..=4).collect() }])
        .unwrap()
    };
    assert!(fresh().with_stress(vec![good.clone()]).is_ok());
    let bad = |what: &str, change: &dyn Fn(&mut Stress3)| {
        let mut s = good.clone();
        change(&mut s);
        let e = fresh().with_stress(vec![s]).err().unwrap_or_else(|| panic!("{what}: accepted"));
        assert!(e.to_string().contains("invalid stress fracture"), "{what}: {e}");
    };
    bad("a strength of zero", &|s| s.strength = 0.0);
    bad("a strength that is not a number", &|s| s.strength = f64::NAN);
    bad("no piece", &|s| s.pieces.clear());
    bad("a piece lost", &|s| {
        s.pieces.pop();
        s.joints.pop();
    });
    bad("a mass that is not the body's", &|s| s.pieces[2].mass *= 1.5);
    // the same total, and a piece heavier than its cells and another lighter: not one density
    bad("masses that add up and are not the cells'", &|s| {
        s.pieces[1].mass *= 1.25;
        s.pieces[3].mass -= s.pieces[1].mass * 0.2;
    });
    bad("a joint to itself", &|s| s.joints[1].b = s.joints[1].a);
    bad("a joint to a piece that is not there", &|s| s.joints[3].b = 9);
    bad("a joint twice", &|s| s.joints.push(row_joint(0, 0.4)));
    bad("a section with no area", &|s| s.joints[0].section.area = 0.0);
    bad("a body that is not a split's parent", &|s| s.parent = 1);
    // and twice for one body
    assert!(fresh().with_stress(vec![good.clone(), good]).is_err());
}

/// The pieces (one cell each) and the joints (one square face each) of a block of `nx` by `ny` by `nz` cubes of `edge` metres, in the lattice of the scene: the cell
/// `[i, j, k]` is the piece `i + nx (j + ny k)`, and the joints are in the body's frame (physics axes: y and z are the lattice's, negated).
pub fn grid_block(nx: usize, ny: usize, nz: usize, edge: f64) -> (Vec<StressPiece3>, Vec<StressJoint3>, Vec<[i32; 3]>) {
    let mass = DENSITY * edge.powi(3);
    let size = [edge; 3];
    let index = |i: usize, j: usize, k: usize| (i + nx * (j + ny * k)) as u32;
    let mut cells = Vec::new();
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                cells.push([i as i32, j as i32, k as i32]);
            }
        }
    }
    let pieces: Vec<StressPiece3> =
        cells.iter().map(|c| StressPiece3::from_cells(&[*c], size, mass).unwrap()).collect();
    let second = edge.powi(4) / 12.0;
    let mut joints = Vec::new();
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                let c = [(i as f64 + 0.5) * edge, -(j as f64 + 0.5) * edge, -(k as f64 + 0.5) * edge];
                for axis in 0..3 {
                    let (ni, nj, nk) = match axis {
                        0 => (i + 1, j, k),
                        1 => (i, j + 1, k),
                        _ => (i, j, k + 1),
                    };
                    if ni >= nx || nj >= ny || nk >= nz {
                        continue;
                    }
                    let mut centroid = c;
                    let mut normal = [0.0; 3];
                    // the face between the cell and its neighbour up the axis: half a cell on, along the axis' physics direction
                    let sign = if axis == 0 { 1.0 } else { -1.0 };
                    centroid[axis] += sign * edge / 2.0;
                    normal[axis] = sign;
                    let mut sec = [[0.0; 3]; 3];
                    let mut lo = centroid;
                    let mut hi = centroid;
                    for other in (0..3).filter(|o| *o != axis) {
                        sec[other][other] = second;
                        lo[other] -= edge / 2.0;
                        hi[other] += edge / 2.0;
                    }
                    joints.push(StressJoint3 {
                        a: index(i, j, k),
                        b: index(ni, nj, nk),
                        section: JointSection { area: edge * edge, centroid, second: sec, normal, lo, hi },
                    });
                }
            }
        }
    }
    (pieces, joints, cells)
}

/// A block of 8 by 8 by 4 cubes (256 pieces) of 0.25 m on a floor, hit on its top by a box of 50 kg at `speed` m/s, with the pieces registered or not.
/// The same with the slots and the split but the stress read only if `read`: what the world costs for being ready to cut, and what the reading adds to it.
pub fn struck_block_with(registered: bool, read: bool, strength: f64, speed: f64) -> World3 {
    struck_block_from(registered, read, strength, speed, false)
}

/// The same, struck from the top or, with `side`, from the side.
pub fn struck_block_from(registered: bool, read: bool, strength: f64, speed: f64, side: bool) -> World3 {
    let (nx, ny, nz, edge) = (8, 8, 4, 0.25f64);
    let (pieces, joints, cells) = grid_block(nx, ny, nz, edge);
    let size = [edge; 3];
    let mass = DENSITY * edge.powi(3) * cells.len() as f64;
    // the block's lattice origin is its top corner: y is down, so the block is 8 cubes (2 m) high above the floor of y = 0
    let mut bodies = vec![body(Shape3::Voxels { size, cells }, mass, [0.0, -(ny as f64) * edge, 0.0])];
    let striker = if side {
        // from the side, at the middle of the height: 1 m to the left of the block and moving along +x
        let mut b = body(Shape3::Box([0.3; 3]), 50.0, [-0.3 - 1.0, -(ny as f64) * edge / 2.0, 0.5]);
        b.velocity = [speed, 0.0, 0.0];
        b
    } else {
        let mut b = body(Shape3::Box([0.3; 3]), 50.0, [0.5, -(ny as f64) * edge - 0.3 - 1.0, -0.5]);
        b.velocity = [0.0, speed, 0.0];
        b
    };
    bodies.push(striker);
    let slots = if registered { 255 } else { 0 };
    for _ in 0..slots {
        bodies.push(body(Shape3::Voxels { size, cells: vec![[0, 0, 0]] }, DENSITY * edge.powi(3), [0.0; 3]));
    }
    let w = World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.0,
        step: 1.0 / 240.0,
        gravity: [0.0, -G, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::Floor { y: 0.0 },
        joints: vec![],
        bodies,
    });
    if !registered {
        return w;
    }
    let w = w.with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: (2..2 + slots).collect() }]).unwrap();
    if !read {
        return w;
    }
    w.with_stress(vec![Stress3 { parent: 0, strength, pieces, joints, min_cells: 1, overflow_to_dust: false }]).unwrap()
}

/// The block of `struck_block` with no striker, thrown tumbling at the floor: awake and in contact for the steps that follow.
pub fn tumbling_block(read: bool) -> World3 {
    let (nx, ny, nz, edge) = (8, 8, 4, 0.25f64);
    let (pieces, joints, cells) = grid_block(nx, ny, nz, edge);
    let size = [edge; 3];
    let mass = DENSITY * edge.powi(3) * cells.len() as f64;
    let mut b = body(Shape3::Voxels { size, cells }, mass, [0.0, -3.0, 0.0]);
    b.angular_velocity = [300.0, 150.0, 90.0];
    b.velocity = [2.0, 8.0, 0.0];
    let mut bodies = vec![b];
    for _ in 0..255 {
        bodies.push(body(Shape3::Voxels { size, cells: vec![[0, 0, 0]] }, DENSITY * edge.powi(3), [0.0; 3]));
    }
    let w = World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.0,
        step: 1.0 / 240.0,
        gravity: [0.0, -G, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::Floor { y: 0.0 },
        joints: vec![],
        bodies,
    })
    .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: (1..256).collect() }])
    .unwrap();
    if !read {
        return w;
    }
    w.with_stress(vec![Stress3 { parent: 0, strength: 1e15, pieces, joints, min_cells: 1, overflow_to_dust: false }])
        .unwrap()
}

#[test]
#[ignore = "a measurement"]
fn what_a_step_costs_with_256_pieces_when_the_block_tumbles_on_the_floor() {
    for (read, what) in [(false, "split, 255 slots"), (true, "split and stress read")] {
        let mut w = tumbling_block(read);
        let mut times = Vec::new();
        let mut awake = 0;
        for step in 1..=240u64 {
            let began = std::time::Instant::now();
            let frame = w.frame_at(step as f64 / 240.0, &mut Still);
            times.push(began.elapsed().as_secs_f64() * 1000.0);
            assert!(frame.errors.is_empty(), "{:?}", frame.errors);
            if !w.stress_levels(0).is_empty() {
                awake += 1;
            }
        }
        let worst = times.iter().copied().fold(0.0, f64::max);
        println!(
            "COST tumbling, {what}: worst {worst:.3} ms, mean {:.3} ms over {} steps ({awake} of them read)",
            times.iter().sum::<f64>() / times.len() as f64,
            times.len()
        );
    }
}

#[test]
#[ignore = "a measurement"]
fn what_a_step_costs_with_256_pieces_when_something_hits_the_block() {
    for (speed, label) in [(0.0, "the striker falling from 1 m"), (100.0, "struck at 100 m/s")] {
        for (registered, read, what) in [
            (false, false, "no split, no stress"),
            (true, false, "split, 255 slots"),
            (true, true, "split and stress read"),
        ] {
            let mut w = struck_block_with(registered, read, 1e15, speed);
            let mut times = Vec::new();
            for step in 1..=120u64 {
                let began = std::time::Instant::now();
                let frame = w.frame_at(step as f64 / 240.0, &mut Still);
                times.push((began.elapsed().as_secs_f64() * 1000.0, frame.errors.len()));
            }
            let worst = times.iter().map(|t| t.0).fold(0.0, f64::max);
            let mean = times.iter().map(|t| t.0).sum::<f64>() / times.len() as f64;
            println!(
                "COST {label}, {what}: worst step {worst:.3} ms, mean {mean:.3} ms (errors {})",
                times.iter().map(|t| t.1).sum::<usize>()
            );
        }
    }
}

#[test]
fn a_part_that_breaks_off_is_a_body_that_breaks_again_when_it_lands_and_its_pieces_take_slots_from_the_same_pool() {
    let (n, edge, strength) = (5, 0.4f64, 8.0e5);
    let load = 1.005 * breaking_load(0, n, edge, strength);
    let mut c = cantilever_over(n, edge, strength, load, true);
    let mut events: Vec<(u64, Vec<Option<Vec<u32>>>)> = Vec::new();
    for step in 1..=480u64 {
        let frame = c.world.frame_at(step as f64 / 240.0, &mut Still);
        assert!(frame.errors.is_empty(), "{:?}", frame.errors);
        let held: Vec<Option<Vec<u32>>> =
            (0..2 + n - 1).map(|k| c.world.stress_pieces(k).map(<[u32]>::to_vec)).collect();
        if events.last().is_none_or(|(_, last)| *last != held) {
            events.push((step, held));
        }
    }
    // the root breaks in the first steps (the load is over the strength at once), and the four pieces that fall are one body in the first slot (the body 2)
    assert_eq!(events.len(), 3, "{events:?}");
    assert_eq!(events[1].1, vec![Some(vec![0]), None, Some(vec![1, 2, 3, 4]), None, None, None]);
    // they fall five metres, in about a second, and when they land every joint of the part breaks together: the first piece stays the body that was the slot, and the
    // three others take the next three slots of the same pool, in order
    let (landed, held) = &events[2];
    assert!((200..260).contains(landed), "landed at step {landed}");
    assert_eq!(held, &vec![Some(vec![0]), None, Some(vec![1]), Some(vec![2]), Some(vec![3]), Some(vec![4])]);
}

#[test]
fn a_block_of_256_pieces_struck_at_100_m_s_breaks_into_pieces_that_are_all_accounted_for_with_nothing_that_is_not_a_number(
) {
    // a block of 2 m of 256 pieces hit on its side by 50 kg at 100 m/s, with a strength that this is far over: it breaks in the first steps
    let mut w = struck_block_from(true, true, 4.0e4, 100.0, true);
    let frame = w.frame_at(1.0, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert!(
        frame.bodies.iter().all(|b| b.pos.iter().chain(&b.rot).all(|v| v.is_finite()))
            && frame.velocities.iter().all(|v| v.linear.iter().chain(&v.angular).all(|c| c.is_finite())),
        "something is not a number"
    );
    // the bodies that hold pieces (the block and its slots) hold every piece once and none twice, whether the joints that broke left it in one body or in many
    let mut seen = vec![0u32; 256];
    let mut bodies = 0;
    for k in 0..2 + 255 {
        if k == 1 {
            continue;
        }
        if let Some(held) = w.stress_pieces(k) {
            bodies += 1;
            for &i in held {
                seen[i as usize] += 1;
            }
        }
    }
    let broken = (0..640).filter(|&j| w.stress_joint_broken(0, j) == Some(true)).count();
    // the strength of 4e4 Pa is 13% over what the block reads at rest (3.53e4), so the blow of the first steps breaks nearly all of it: how many joints and how many bodies is the
    // outcome of a run that is chaotic in its details (this one: 633 joints and 250 bodies, and it was 634 and 251 before the friction and the frame were corrected), so the test asks for
    // the bulk of it, and for the bodies to be fewer than the pieces only by what stayed joined
    assert!(broken >= 600 && (200..=256).contains(&bodies), "{broken} joints broken, {bodies} bodies that hold pieces");
    assert!(seen.iter().all(|c| *c == 1), "every piece is in exactly one body");
}

#[test]
fn a_block_at_rest_on_the_four_corners_that_the_solver_gives_it_reads_the_bending_of_a_beam_on_two_supports() {
    // the block of 2 m by 2 m by 1 m (256 cubes of 0.25 m) rests on the floor, and the solver holds it up at the four corners of its foot, a quarter of its weight at each
    // (98.1 N s a step against 392.3): the block is a deep beam of a span of 2 m on two supports, whose bending moment at the middle is W L / 8 and whose principal
    // tension there is 6 M / (b h^2) with b = 1 m and h = 2 m. The compression of its base, rho g h = 4.7e4 Pa, is another stress
    let (span, depth, height) = (2.0f64, 1.0f64, 2.0f64);
    let weight = DENSITY * span * depth * height * G;
    let bending = 6.0 * (weight * span / 8.0) / (depth * height * height);
    assert!((bending - 3.53e4).abs() < 1e2, "{bending}");
    let mut w = struck_block_with(true, true, 1e15, 0.0);
    let _ = w.frame_at(4.0 / 240.0, &mut Still);
    let top = w.stress_levels(0).iter().map(|(_, v)| *v).fold(0.0, f64::max);
    assert!((top / bending - 1.0).abs() < 0.03, "{top} against {bending}");
}

/// A bar of four cubes of 0.25 m on a floor, sliding along x at `speed` m/s, with the friction of `friction`, registered with a strength that nothing reaches.
pub fn sliding_bar(speed: f64, friction: f64) -> World3 {
    let (n, edge) = (4usize, 0.25f64);
    let cells: Vec<[i32; 3]> = (0..n as i32).map(|i| [i, 0, 0]).collect();
    let mass = DENSITY * edge.powi(3);
    let size = [edge; 3];
    let mut bar = body(Shape3::Voxels { size, cells }, mass * n as f64, [0.0, -edge, 0.0]);
    bar.velocity = [speed, 0.0, 0.0];
    bar.friction = friction;
    let mut bodies = vec![bar];
    for _ in 1..n {
        bodies.push(body(Shape3::Voxels { size, cells: vec![[0, 0, 0]] }, mass, [0.0; 3]));
    }
    let pieces: Vec<StressPiece3> =
        (0..n as i32).map(|i| StressPiece3::from_cells(&[[i, 0, 0]], size, mass).unwrap()).collect();
    let joints: Vec<StressJoint3> = (0..n as u32 - 1).map(|i| row_joint(i, edge)).collect();
    World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.0,
        step: 1.0 / 240.0,
        gravity: [0.0, -G, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::Floor { y: 0.0 },
        joints: vec![],
        bodies,
    })
    .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: (1..n).collect() }])
    .unwrap()
    .with_stress(vec![Stress3 { parent: 0, strength: 1e15, pieces, joints, min_cells: 1, overflow_to_dust: false }])
    .unwrap()
}

#[test]
fn a_bar_that_slides_on_a_floor_has_contact_impulses_that_sum_to_the_momentum_it_changes() {
    let mass = DENSITY * 0.25f64.powi(3) * 4.0;
    let mut w = sliding_bar(3.0, 0.5);
    let mut slid = 0;
    let mut before = 3.0;
    for step in 1..=60u64 {
        let frame = w.frame_at(step as f64 / 240.0, &mut Still);
        assert!(frame.errors.is_empty(), "{:?}", frame.errors);
        let Some(balance) = w.stress_balance(0) else { continue };
        let speed = frame.velocities[0].linear[0];
        // the friction that the world read is what the bar lost in the step: m dv, which no part of the reading is fitted to (nothing else acts along the floor)
        let lost = mass * (speed - before);
        before = speed;
        assert!(
            (balance.friction[0] - lost).abs() < 2e-3 * mass * G / 240.0,
            "step {step}: friction {} against what the bar lost {lost}",
            balance.friction[0]
        );
        if speed < 1.0 {
            continue;
        }
        slid += 1;
        let weight = mass * G / 240.0;
        // the vertical: the weight and the normal impulses (whose sum is the step's total, whatever the number of sub-steps) leave nothing; across the sliding, nothing is left
        assert!(balance.force[1].abs() < 1e-6 * weight, "step {step}: vertical {} of {weight}", balance.force[1]);
        assert!(balance.force[2].abs() < 1e-6 * weight, "step {step}: across {} of {weight}", balance.force[2]);
        // along it, the friction takes what the body loses, mu m g dt, and the last sub-step's friction is 1/8 of the step's when it is steady
        assert!(balance.force[0].abs() < 1e-6 * weight, "step {step}: along {} of {weight}", balance.force[0]);
        assert!(
            (balance.friction_scale / 8.0 - 1.0).abs() < 0.02,
            "step {step}: the friction of the step is {} times the last sub-step's",
            balance.friction_scale
        );
        // and the moment: the friction (at the base, 0.125 m under the centre of mass: 0.5 * 6.13 * 0.125 = 0.38 N m s) and the normal impulses, at the points of the body that
        // they act on (not the middle of the two surfaces, which comes apart by what the contact slides: that left a fifth of the friction's moment), leave a thousandth of it
        let friction_moment = 0.5 * weight * 0.125;
        assert!(
            balance.moment[0].abs() < 1e-5 * weight && balance.moment[1].abs() < 1e-5 * weight,
            "step {step}: moment {:?}",
            balance.moment
        );
        assert!(
            balance.moment[2].abs() < 1e-3 * friction_moment,
            "step {step}: moment {:?} of {friction_moment}",
            balance.moment
        );
    }
    assert!(slid > 20, "{slid} steps of sliding");
}

#[test]
fn a_beam_that_spins_has_in_each_joint_the_pull_of_the_part_beyond_it_by_the_centripetal_load_and_nothing_across_it() {
    // six cubes of 0.4 m spinning at 12 rad/s about their centre of mass, in a step of 1/240 s (0.05 rad): the joint j holds the part up to it, whose centre is at a distance r from the
    // axis, with m_S w^2 r over the area, along the beam (the load is read at the middle of the step, in the frame of the body at the middle of it: a frame of the end made the
    // direction of the force turn half a step against the body, a shear of 2.5% of the pull and a bending moment that a slender beam turns into 30% of it)
    let (n, edge, omega) = (6, 0.4f64, 12.0);
    let mass = DENSITY * edge.powi(3);
    let mut w =
        Flyer { n, edge, strength: 1e15, omega, drift: [0.0; 3], slots: 5, min_cells: 1, dust: false, walls: None }
            .world();
    let frame = w.frame_at(1.0 / 240.0, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    // the shear is asserted apart (it enters the principal tension as shear^2 / pull, which a 2.5% shear changes by 6e-4 only): nothing across the beam, to 1e-3 of the pull, and no
    // bending, and no twist
    let readings = w.stress_readings(0);
    assert_eq!(readings.len(), n - 1);
    for (joint, r) in readings {
        assert!(
            r.shear < 1e-3 * r.normal && r.bending.abs() < 1e-3 * r.normal && r.torsion < 1e-3 * r.normal,
            "joint {joint}: {r:?}"
        );
    }
    let levels = w.stress_levels(0);
    assert_eq!(levels.len(), n - 1);
    for (j, (joint, principal)) in levels.iter().enumerate() {
        // the pieces up to the joint j are j + 1 of them, with their middle at (j + 1) edge / 2 from the end, and the whole beam's at n edge / 2
        let part = (j + 1) as f64;
        let r = (n as f64 / 2.0 - part / 2.0) * edge;
        let want = part * mass * omega * omega * r / (edge * edge);
        assert_eq!(*joint as usize, j);
        assert!((principal / want - 1.0).abs() < 2e-4, "joint {j}: {principal} against {want}");
    }
}

/// A beam of six cubes spinning at 12 rad/s breaks at its middle joint alone (a strength a little over what the joints on each side of it read) and its two halves fly
/// off along +y and -y into two walls, which they hit together, and every joint of each breaks: two bodies of the same family break in the same step, with `slots` slots
/// in all for the pieces (one is taken by the first break).
fn halves_into_walls(slots: usize, dust: bool) -> World3 {
    let (n, edge, omega) = (6, 0.4f64, 12.0);
    let mass = DENSITY * edge.powi(3);
    let strength = 4.25 * mass * omega * omega / edge;
    Flyer { n, edge, strength, omega, drift: [0.0; 3], slots, min_cells: 1, dust, walls: Some(0.9) }.world()
}

#[test]
fn two_bodies_of_a_family_that_break_in_the_same_step_share_the_pool_and_the_one_that_does_not_fit_is_an_error_or_dust()
{
    // with the overflow to dust, first: the two halves break in the same step, and what each holds is exact. Three slots: the first break takes one, and the two halves that hit the walls
    // together need two each and there are two left, which the body of the lower index takes: the rule is that the bodies of a family are served in the order of their index, the loose
    // parts of each by their lowest piece, and what does not fit is dust, the smallest first (the first of equals): not the largest of the whole family first
    let mut w = halves_into_walls(3, true);
    let (mut parent_apart, mut slot_apart) = (None, None);
    for step in 1..=240u64 {
        let frame = w.frame_at(step as f64 / 240.0, &mut Still);
        assert!(frame.errors.is_empty(), "step {step}: {:?}", frame.errors);
        if parent_apart.is_none() && w.stress_pieces(0).is_some_and(|h| h.len() == 1) {
            parent_apart = Some(step);
        }
        if slot_apart.is_none() && w.stress_pieces(1).is_some_and(|h| h.len() == 1) {
            slot_apart = Some(step);
        }
    }
    let step = parent_apart.expect("the first half breaks apart");
    assert_eq!(slot_apart, Some(step), "the two halves break apart in the same step");
    let held: Vec<Option<Vec<u32>>> = (0..4).map(|k| w.stress_pieces(k).map(<[u32]>::to_vec)).collect();
    // the parent keeps the first piece of its half, the pieces 1 and 2 of it take the two slots that are left, and the other half (the body 1) keeps its first piece, the pieces 4 and 5
    // of it being dust: no body holds them
    assert_eq!(held, vec![Some(vec![0]), Some(vec![3]), Some(vec![1]), Some(vec![2])]);
    let mut ids: Vec<u32> = held.iter().flatten().flatten().copied().collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids, vec![0, 1, 2, 3], "every piece that is held is held once, and the dust is held by none");
    // the same world with no overflow to dust is an error in that very step, that names both numbers, and not an index out of the pool
    let mut w = halves_into_walls(3, false);
    let mut errors = (0, Vec::new());
    for step in 1..=240u64 {
        let frame = w.frame_at(step as f64 / 240.0, &mut Still);
        if !frame.errors.is_empty() {
            errors = (step, frame.errors);
            break;
        }
    }
    assert!(
        errors
            .1
            .iter()
            .any(|e| e.contains("2 loose parts") && e.contains("0 slots free") && e.contains("maxFragments")),
        "{errors:?}"
    );
    assert_eq!(errors.0, step, "the error is in the step in which the halves break");
}

/// A driver that says that the body `which` does not take part from the time `from` on.
struct Absent {
    which: usize,
    from: f64,
}
impl Driver3 for Absent {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
    fn enabled(&mut self, t: f64, which: usize) -> bool {
        !(which == self.which && t >= self.from)
    }
}

#[test]
fn a_body_that_the_driver_skips_does_not_ask_for_slots_so_it_cannot_abort_the_cuts_of_the_others() {
    // the two halves of halves_into_walls(3, false) break in the same step and the second does not fit (the error of the test above). With the second half out of the scene at that
    // step it asks for nothing, and the first half, which fits, is cut: the error would have been the second's, for a cut that is skipped
    let mut w = halves_into_walls(3, false);
    let mut step_of_break = None;
    for step in 1..=240u64 {
        let frame = w.frame_at(step as f64 / 240.0, &mut Still);
        if !frame.errors.is_empty() {
            step_of_break = Some(step);
            break;
        }
    }
    let step = step_of_break.expect("the halves break in the same step, and the second does not fit");
    let mut w = halves_into_walls(3, false);
    let mut driver = Absent { which: 1, from: (step as f64 - 0.5) / 240.0 };
    for s in 1..=step {
        let frame = w.frame_at(s as f64 / 240.0, &mut driver);
        assert!(frame.errors.is_empty(), "step {s}: {:?}", frame.errors);
    }
    // the first half is cut (its first piece stays and the others are loose), and the second is not
    assert_eq!(w.stress_pieces(0).map(<[u32]>::len), Some(1), "the first half is cut");
    assert!(w.stress_pieces(1).is_some_and(|h| h.len() > 1), "the skipped half is as it was: {:?}", w.stress_pieces(1));
}

#[test]
fn the_friction_that_a_block_landing_and_sliding_puts_on_a_welded_beam_is_what_the_block_loses_where_it_is_read_and_never_over_the_bound(
) {
    // a block of 3000 N lands on the tip of the welded cantilever from 2 cm with 1 m/s along the beam and slides: the beam is held by a joint of the world, so what the friction is
    // is not left to the balance of the body, and the only things that say what it is are the manifold's last sub-step vector and its normal impulses (the step's and the last
    // sub-step's). The block's horizontal momentum is lost to the beam's friction alone, so m dv of the block is the oracle, step by step
    let load = 3000.0;
    let mut c = cantilever_with(5, 0.4, 1e15, load, false, (1.0, 0.02));
    let mass = load / G;
    let mut before = 1.0;
    let (mut read, mut missed) = (0, 0);
    for step in 1..=60u64 {
        let frame = c.world.frame_at(step as f64 / 240.0, &mut Still);
        assert!(frame.errors.is_empty(), "{:?}", frame.errors);
        let v = frame.velocities[1].linear[0];
        let lost = -mass * (v - before);
        before = v;
        let b = c.world.stress_balance(0).expect("read");
        let friction = b.friction[0];
        // never more than the coefficient times the normal impulses (0.5 for these two bodies)
        assert!(
            friction.abs() <= 0.5 * b.normal * (1.0 + 1e-9),
            "step {step}: friction {friction} over 0.5 of the normal impulse {}",
            b.normal
        );
        if friction != 0.0 {
            read += 1;
            assert!(
                (friction - lost).abs() <= 0.3 * lost.abs().max(0.5),
                "step {step}: the friction read is {friction} and the block lost {lost}"
            );
        } else if lost.abs() > 0.5 {
            // the step in which the last sub-step has no friction vector (the block has bounced off, or is held): what the earlier sub-steps did is not given
            missed += 1;
        }
    }
    // it reads where it can (twenty steps of the block's sliding, 4% to 6% over what the block lost while it slides steadily, 23% under it in the step in which it stops), and
    // the steps it cannot read are the landing and a bounce
    assert!(read >= 15, "{read} steps read the friction");
    assert!(missed <= 4, "{missed} steps in which the block lost more than the world read");
}

#[test]
fn a_block_of_friction_0_8_on_a_beam_of_0_2_is_read_with_the_0_5_that_they_combine_to_in_every_step() {
    // the world combines two colliders by the average (Rapier's default rule; the Max rule is the boundary slabs'), so the friction between them is 0.5, and nothing that is read of a step is over
    // that times the normal impulse (the rule itself is checked on colliders, in the unit test of the module of the world's stress, because the bound only engages when the ratio of a step is not
    // to be trusted, and a landing of this block does not give one that is over it)
    let load = 3000.0;
    let mut c = cantilever_mu(5, 0.4, 1e15, load, false, (1.0, 0.02), (0.2, 0.8));
    let mut read = 0;
    for step in 1..=60u64 {
        let frame = c.world.frame_at(step as f64 / 240.0, &mut Still);
        assert!(frame.errors.is_empty(), "{:?}", frame.errors);
        let b = c.world.stress_balance(0).expect("read");
        assert!(
            b.friction[0].abs() <= 0.5 * b.normal * (1.0 + 1e-9),
            "step {step}: friction {} over the 0.5 of the normal impulse {}",
            b.friction[0],
            b.normal
        );
        read += usize::from(b.friction[0] != 0.0);
    }
    assert!(read >= 3, "{read} steps with a friction read");
}

#[test]
fn a_bar_spinning_flat_on_a_floor_is_slowed_by_a_twist_that_is_not_read_and_the_balance_says_so() {
    // a bar turning at 5 rad/s about the vertical on its floor: the friction under it is a twist of the manifold (the simplified friction solves it apart from the tangent vector), which
    // the world does not read: the frictions that are read add up to nothing along the floor, and the balance leaves the moment that slows the bar. Not a defect of the moment: the
    // stress of a bar that is slowed this way is not known, and the balance says it (the whole residual is the twist)
    let mut bar = sliding_bar(0.0, 0.5);
    let _ = &mut bar;
    let (n, edge) = (4usize, 0.25f64);
    let mass = DENSITY * edge.powi(3) * n as f64;
    let mut w = spinning_bar(5.0);
    let mut before = 5.0;
    let mut checked = 0;
    for step in 1..=30u64 {
        let frame = w.frame_at(step as f64 / 240.0, &mut Still);
        let b = w.stress_balance(0).expect("read");
        // angular velocity about the physics' y is the scene's -y
        let spin = frame.velocities[0].angular[1].to_radians().abs();
        let lost = before - spin;
        before = spin;
        if lost <= 0.0 {
            continue;
        }
        checked += 1;
        // the horizontal friction that was read is nothing: the bar's centre does not move
        assert!(
            b.friction[0].abs() < 1e-3 * mass * G / 240.0 && b.friction[2].abs() < 1e-3 * mass * G / 240.0,
            "step {step}: {:?}",
            b.friction
        );
        // and the moment about the vertical that the balance leaves is the spin that it lost times the moment of inertia (m (L^2 + w^2) / 12 about the vertical axis)
        let inertia = mass * (1.0f64 + 0.25 * 0.25) / 12.0;
        assert!(
            (b.moment[1].abs() - inertia * lost).abs() < 0.02 * inertia * lost,
            "step {step}: moment {:?} against {}",
            b.moment,
            inertia * lost
        );
    }
    assert!(checked > 10, "{checked} steps of a bar that slows");
}

/// The bar of `sliding_bar`, at rest, turning about the vertical at `omega` rad/s.
pub fn spinning_bar(omega: f64) -> World3 {
    let (n, edge) = (4usize, 0.25f64);
    let cells: Vec<[i32; 3]> = (0..n as i32).map(|i| [i, 0, 0]).collect();
    let mass = DENSITY * edge.powi(3);
    let size = [edge; 3];
    let mut bar = body(Shape3::Voxels { size, cells }, mass * n as f64, [0.0, -edge, 0.0]);
    bar.angular_velocity = [0.0, omega.to_degrees(), 0.0];
    bar.friction = 0.5;
    let mut bodies = vec![bar];
    for _ in 1..n {
        bodies.push(body(Shape3::Voxels { size, cells: vec![[0, 0, 0]] }, mass, [0.0; 3]));
    }
    let pieces: Vec<StressPiece3> =
        (0..n as i32).map(|i| StressPiece3::from_cells(&[[i, 0, 0]], size, mass).unwrap()).collect();
    let joints: Vec<StressJoint3> = (0..n as u32 - 1).map(|i| row_joint(i, edge)).collect();
    World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.0,
        step: 1.0 / 240.0,
        gravity: [0.0, -G, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::Floor { y: 0.0 },
        joints: vec![],
        bodies,
    })
    .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: (1..n).collect() }])
    .unwrap()
    .with_stress(vec![Stress3 { parent: 0, strength: 1e15, pieces, joints, min_cells: 1, overflow_to_dust: false }])
    .unwrap()
}
