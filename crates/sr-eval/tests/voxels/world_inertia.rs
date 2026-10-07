//! The inertia that the world simulates for a body of cells against the exact one of its cells, observed from outside: pushed with a torque,
//! a body spins up as I^-1 tau. The expected values are the integer moments of `sr_3d::occupancy`, not what Rapier says of itself.
#![allow(clippy::needless_range_loop)]

use sr_3d::occupancy::Moments;
use sr_sim::fields::Field;
use sr_sim::physics3d::*;

const SIZE: [f64; 3] = [0.25; 3];
const DENSITY: f64 = 2400.0;
const CELL_MASS: f64 = 2400.0 * 0.25 * 0.25 * 0.25;

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
    Body3Spec {
        kind: BodyKind::Dynamic,
        mass: cells.len() as f64 * CELL_MASS,
        shape: Shape3::Voxels { size: SIZE, cells },
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
    }
}

/// Pushes every body with a torque from 0.4 s on, and cuts the block at 0.3 s when asked.
struct Push {
    torque: [f64; 3],
    cut: Option<VoxelCut3>,
}

impl Driver3 for Push {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
    fn voxel_cut(
        &mut self,
        t: f64,
        parent: usize,
        revision: Option<u64>,
        _: Option<&Impact3>,
    ) -> Result<Option<VoxelCut3>, String> {
        Ok(self.cut.clone().filter(|_| parent == 0 && revision.is_none() && t + 1e-9 >= 0.3))
    }
    fn load(&mut self, _: u64, t: f64, _: usize, _: &BodyState) -> Result<Option<Load3>, String> {
        Ok((t + 1e-9 >= 0.4).then_some(Load3 { force: [0.0; 3], torque: self.torque }))
    }
}

fn world(bodies: Vec<Body3Spec>, slots: usize) -> World3 {
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
    });
    if slots > 0 {
        w.with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: (1..=slots).collect() }]).unwrap()
    } else {
        w
    }
}

fn inverse3(m: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let c = |a: usize, b: usize| {
        let (r, s) = ([(a + 1) % 3, (a + 2) % 3], [(b + 1) % 3, (b + 2) % 3]);
        m[r[0]][s[0]] * m[r[1]][s[1]] - m[r[0]][s[1]] * m[r[1]][s[0]]
    };
    std::array::from_fn(|i| std::array::from_fn(|j| c(j, i) / det))
}

/// How fast a body of `cells` that is body `k` spins up under `torque` as the world sees it, against I^-1 tau of its cells, per axis.
fn assert_spin_up(frames: (&Frame3, &Frame3), dt: f64, k: usize, cells: &[[i32; 3]], torque: [f64; 3], what: &str) {
    let exact = Moments::of(cells.iter().copied()).properties(SIZE, DENSITY).unwrap();
    let inverse = inverse3(exact.inertia);
    for i in 0..3 {
        let alpha = (frames.1.velocities[k].angular[i] - frames.0.velocities[k].angular[i]).to_radians() / dt;
        let wanted: f64 = (0..3).map(|j| inverse[i][j] * torque[j]).sum();
        assert!(
            (alpha - wanted).abs() < 2e-3 * wanted.abs().max(1e-3),
            "{what}, axis {i}: spins up at {alpha} rad/s2, the cells say {wanted}"
        );
    }
}

#[test]
fn a_plate_and_a_block_spin_up_as_the_cells_say_about_each_of_their_axes() {
    // a plate thin along x with a hole, a block shorter along x than it is wide, a bar, and a plate thin along y: the diagonal tensors with two
    // equal moments in each orientation
    let shapes = [
        (
            "a plate thin along x",
            cells_of(0..1, 0..7, 0..7).into_iter().filter(|c| c[1] != 3 || c[2] != 3).collect::<Vec<_>>(),
        ),
        ("a block 3 by 4 by 4", cells_of(0..3, 0..4, 0..4)),
        ("a plate thin along y", cells_of(0..7, 0..1, 0..7)),
        ("a plate thin along z", cells_of(0..7, 0..7, 0..1)),
        ("a bar along x", cells_of(0..9, 0..2, 0..2)),
    ];
    for (what, cells) in shapes {
        for torque in [[40.0, 0.0, 0.0], [0.0, 40.0, 0.0], [0.0, 0.0, 40.0], [30.0, -20.0, 50.0]] {
            let mut w = world(vec![body(cells.clone())], 0);
            let mut d = Push { torque, cut: None };
            let (a, b) = (w.frame_at(0.41, &mut d), w.frame_at(0.45, &mut d));
            assert!(a.errors.is_empty() && b.errors.is_empty(), "{:?}", a.errors);
            assert_spin_up((&a, &b), 0.04, 0, &cells, torque, &format!("{what}, torque {torque:?}"));
        }
    }
}

#[test]
fn a_thin_plate_cut_off_a_block_spins_up_as_its_cells_say() {
    // 6 by 7 by 7 cells; the slice x = 1 is destroyed, the block x 2 to 5 stays (the larger part) and the plate x = 0 is the piece in the slot
    let block = cells_of(0..6, 0..7, 0..7);
    let plate = cells_of(0..1, 0..7, 0..7);
    let stays = cells_of(2..6, 0..7, 0..7);
    let cut = VoxelCut3 {
        added: vec![],
        revision: 1,
        destroyed: cells_of(1..2, 0..7, 0..7),
        parent_mass: stays.len() as f64 * CELL_MASS,
        pieces: vec![VoxelPiece3 { cells: plate.clone(), mass: plate.len() as f64 * CELL_MASS }],
    };
    for torque in [[40.0, 0.0, 0.0], [0.0, 40.0, 0.0], [30.0, -20.0, 50.0]] {
        let mut w = world(vec![body(block.clone()), body(vec![[0, 0, 0]])], 1);
        let mut d = Push { torque, cut: Some(cut.clone()) };
        let (a, b) = (w.frame_at(0.41, &mut d), w.frame_at(0.45, &mut d));
        assert!(a.errors.is_empty() && b.errors.is_empty(), "{:?}", a.errors);
        assert_eq!(b.enabled, vec![true, true]);
        assert_spin_up((&a, &b), 0.04, 1, &plate, torque, &format!("the plate in its slot, torque {torque:?}"));
        assert_spin_up((&a, &b), 0.04, 0, &stays, torque, &format!("the block that stays, torque {torque:?}"));
    }
}

#[test]
fn a_body_of_cells_that_will_fracture_spins_up_as_its_cells_say_until_it_does() {
    // the tensor of the source is built from its fragments': the block of 3 by 4 by 4 cells whose tensor is diag(300, 234.375, 234.375), in two
    // halves cut along z so that the sum is exactly diagonal, which is the case in which a general eigen solver puts the 300 on the wrong axis
    let block = cells_of(0..3, 0..4, 0..4);
    let (low, high) = (cells_of(0..3, 0..4, 0..2), cells_of(0..3, 0..4, 2..4));
    for torque in [[40.0, 0.0, 0.0], [0.0, 40.0, 0.0], [30.0, -20.0, 50.0]] {
        let w = world(vec![body(block.clone()), body(low.clone()), body(high.clone())], 0);
        let fragments = (1..=2).map(|k| Fragment3 { body: k, offset: [0.0; 3], impulse: [0.0; 3] }).collect();
        let mut w = w
            .with_fractures(vec![Fracture3 {
                source: 0,
                at: 2.0,
                radial_impulse: 0.0,
                fragments,
                contact: None,
                dust: None,
            }])
            .unwrap();
        let mut d = Push { torque, cut: None };
        let (a, b) = (w.frame_at(0.41, &mut d), w.frame_at(0.45, &mut d));
        assert!(a.errors.is_empty() && b.errors.is_empty(), "{:?}", a.errors);
        assert_eq!(b.enabled, vec![true, false, false], "before the fracture the source is the body");
        assert_spin_up((&a, &b), 0.04, 0, &block, torque, &format!("the source of a fracture, torque {torque:?}"));
    }
}

/// A deterministic set of cells without symmetry: a few dozen cells of a box of 5 by 4 by 3, so that the tensor has all of its products.
fn lumpy(seed: u64) -> Vec<[i32; 3]> {
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (state >> 33) as i32
    };
    let mut cells: Vec<[i32; 3]> = (0..70).map(|_| [next() % 5, next() % 4, next() % 3]).collect();
    cells.sort();
    cells.dedup();
    cells
}

/// A torque that spins the body up at a rate of the order of half a radian a second squared about each axis, from the trace of its inertia: the
/// bodies of these tests are small and a torque that suits a bar of metres turns them far in the time measured.
fn gentle(cells: &[[i32; 3]], size: [f64; 3], pixels_per_meter: f64) -> [f64; 3] {
    let i = Moments::of(cells.iter().copied()).properties(size.map(|s| s / pixels_per_meter), DENSITY).unwrap().inertia;
    let scale = (i[0][0] + i[1][1] + i[2][2]) / 3.0 * pixels_per_meter * pixels_per_meter;
    [0.4 * scale, -0.3 * scale, 0.5 * scale]
}

#[test]
fn a_body_with_all_the_products_of_inertia_spins_up_as_its_cells_say_so_that_every_rotation_of_the_diagonalisation_runs(
) {
    for seed in [1u64, 2, 3, 4] {
        let cells = lumpy(seed);
        let torque = gentle(&cells, SIZE, 1.0);
        let mut w = world(vec![body(cells.clone())], 0);
        let mut d = Push { torque, cut: None };
        let (a, b) = (w.frame_at(0.41, &mut d), w.frame_at(0.45, &mut d));
        assert!(a.errors.is_empty() && b.errors.is_empty(), "{:?}", a.errors);
        // the products are not zero (the test would be of a diagonal tensor): at least two of them are over a thousandth of the trace
        let inertia = Moments::of(cells.iter().copied()).properties(SIZE, DENSITY).unwrap().inertia;
        let trace = inertia[0][0] + inertia[1][1] + inertia[2][2];
        let big = [inertia[0][1], inertia[0][2], inertia[1][2]].iter().filter(|p| p.abs() > 1e-3 * trace).count();
        assert!(big >= 2, "seed {seed}: only {big} products of inertia: {inertia:?}");
        assert_spin_up((&a, &b), 0.04, 0, &cells, torque, &format!("a lumpy body, seed {seed}"));
    }
}

#[test]
fn the_pieces_of_a_fractured_body_spin_up_as_their_cells_say_each_with_its_own_inertia() {
    // a body of 8 by 4 by 4 cells fractured at 0.3 s into three pieces of unequal shape, which are pushed from 0.4 s
    let whole = cells_of(0..8, 0..4, 0..4);
    let end = cells_of(0..3, 0..4, 0..4);
    let step = cells_of(3..6, 0..2, 0..4);
    let rest: Vec<[i32; 3]> = whole.iter().copied().filter(|c| !end.contains(c) && !step.contains(c)).collect();
    let pieces = [end, step, rest];
    // the pieces touch each other face to face and are pushed differently, so that they would press on each other: they collide with nothing, to see
    // each spin up as its own cells say
    let alone = |cells: Vec<[i32; 3]>| Body3Spec { collides_with: Some(vec![]), ..body(cells) };
    let mut bodies = vec![alone(whole.clone())];
    bodies.extend(pieces.iter().map(|p| alone(p.clone())));
    let w = world(bodies, 0);
    let fragments = (1..=3).map(|k| Fragment3 { body: k, offset: [0.0; 3], impulse: [0.0; 3] }).collect();
    let mut w = w
        .with_fractures(vec![Fracture3 {
            source: 0,
            at: 0.3,
            radial_impulse: 0.0,
            fragments,
            contact: None,
            dust: None,
        }])
        .unwrap();
    // gentle for the smallest piece, whose inertia is a twentieth of the body's: the others are pushed less than that
    let torque = gentle(&whole, SIZE, 1.0).map(|t| t * 0.1);
    let mut d = Push { torque, cut: None };
    let (a, b) = (w.frame_at(0.41, &mut d), w.frame_at(0.45, &mut d));
    assert!(a.errors.is_empty() && b.errors.is_empty(), "{:?}", a.errors);
    assert_eq!(b.enabled, vec![false, true, true, true]);
    for (k, cells) in pieces.iter().enumerate() {
        assert_spin_up((&a, &b), 0.04, k + 1, cells, torque, &format!("fragment {k} of a fracture"));
    }
}

#[test]
fn the_scale_of_the_scene_and_cells_of_three_sizes_are_the_world_s_too() {
    // a metre is 100 scene units and the cells are 25 by 50 by 12.5 of them: lengths in the units of the scene, mass in kilograms, inertia in
    // kilograms of scene units squared, the force and the torque in the world's units
    let (ppm, size) = (100.0, [25.0, 50.0, 12.5]);
    let cells = lumpy(7);
    let mass = cells.len() as f64 * DENSITY * (size[0] / ppm) * (size[1] / ppm) * (size[2] / ppm);
    let mut spec = body(cells.clone());
    spec.shape = Shape3::Voxels { size, cells: cells.clone() };
    spec.mass = mass;
    let w = World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.,
        step: 0.01,
        gravity: [0.; 3],
        pixels_per_meter: ppm,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies: vec![spec],
    });
    let torque = gentle(&cells, size, ppm);
    let mut w = w;
    let mut d = Push { torque, cut: None };
    let (a, b) = (w.frame_at(0.41, &mut d), w.frame_at(0.45, &mut d));
    assert!(a.errors.is_empty() && b.errors.is_empty(), "{:?}", a.errors);
    // the exact tensor of the cells in metres, in kilograms of scene units squared
    let exact = Moments::of(cells.iter().copied()).properties(size.map(|s| s / ppm), DENSITY).unwrap();
    let inertia: [[f64; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| exact.inertia[i][j] * ppm * ppm));
    let inverse = inverse3(inertia);
    for i in 0..3 {
        let alpha = (b.velocities[0].angular[i] - a.velocities[0].angular[i]).to_radians() / 0.04;
        let wanted: f64 = (0..3).map(|j| inverse[i][j] * torque[j]).sum();
        assert!(
            (alpha - wanted).abs() < 2e-3 * wanted.abs().max(1e-3),
            "axis {i}: spins up at {alpha}, the cells say {wanted}"
        );
    }
}
