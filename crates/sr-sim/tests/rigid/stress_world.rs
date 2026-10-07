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
    let cells: Vec<[i32; 3]> = (0..n as i32).map(|i| [i, 0, 0]).collect();
    let mass = DENSITY * edge.powi(3);
    let size = [edge; 3];
    let mut bodies = vec![
        body(Shape3::Voxels { size, cells: cells.clone() }, mass * n as f64, [0.0, -5.0, 0.0]),
        // the block rests on the top of the tip piece (the top is at y = -5 in the scene's axes, y down)
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
        bounds: Bounds3::None,
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
        // the solver holds a weld and a resting block to a few ten-thousandths of the load
        assert!((principal / want - 1.0).abs() < 0.005, "joint {j}: {principal} against {want}");
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
}

impl Flyer {
    pub fn world(&self) -> World3 {
        let Flyer { n, edge, strength, omega, drift, slots, min_cells, dust } = *self;
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
                (p[a] - p0[a]).abs() <= 1e-9 * p0[0].abs().max(1.0) * 10.0,
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
