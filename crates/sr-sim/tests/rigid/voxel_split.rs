//! A body of cells that breaks at run time: the cells a cut destroys leave it, the pieces that separate take the slots made for them
//! with the motion they had, and the world replays the same whenever it is asked.
use sr_sim::{
    fields::Field,
    physics3d::{shape_mass_properties, *},
};

const SIZE: [f64; 3] = [0.25; 3];
const CELL_MASS: f64 = 2400.0 * 0.25 * 0.25 * 0.25;

/// A bar of 12 by 2 by 2 cells.
fn bar() -> Vec<[i32; 3]> {
    cells_of(0..12, 0..2, 0..2)
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

/// Cuts the bar at the time `at`: the slice of cells at x = 5 is destroyed, the part with x under 5 stays and the part over it is a piece.
struct Cutter {
    at: f64,
    pieces: usize,
    asked: Vec<Option<u64>>,
}

impl Cutter {
    fn new(at: f64) -> Self {
        Cutter { at, pieces: 1, asked: vec![] }
    }
}

impl Driver3 for Cutter {
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
        self.asked.push(revision);
        if parent != 0 || revision == Some(1) || t + 1e-9 < self.at {
            return Ok(None);
        }
        let mut pieces = vec![VoxelPiece3 { cells: cells_of(6..12, 0..2, 0..2), mass: 24.0 * CELL_MASS }];
        if self.pieces == 2 {
            // the far end in two: the second takes the cells of x from 9 on out of the first
            pieces = vec![
                VoxelPiece3 { cells: cells_of(6..9, 0..2, 0..2), mass: 12.0 * CELL_MASS },
                VoxelPiece3 { cells: cells_of(9..12, 0..2, 0..2), mass: 12.0 * CELL_MASS },
            ];
        }
        Ok(Some(VoxelCut3 {
            revision: 1,
            destroyed: cells_of(5..6, 0..2, 0..2),
            parent_mass: 20.0 * CELL_MASS,
            pieces,
        }))
    }
}

fn body(shape: Shape3, mass: f64, kind: BodyKind) -> Body3Spec {
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
        start: Pose3::default(),
    }
}

/// A world in space with the bar and `slots` placeholders for its pieces.
fn world(slots: usize, budget: Option<usize>) -> World3 {
    let mut parent = body(Shape3::Voxels { size: SIZE, cells: bar() }, 48.0 * CELL_MASS, BodyKind::Dynamic);
    parent.velocity = [1.0, 0.4, -0.3];
    parent.angular_velocity = [20.0, 10.0, 40.0];
    parent.start = Pose3 { pos: [0.3, -1.0, 0.2], rot: [0., 0., 0., 1.] };
    let mut bodies = vec![parent];
    for _ in 0..slots {
        bodies.push(body(Shape3::Voxels { size: SIZE, cells: vec![[0, 0, 0]] }, CELL_MASS, BodyKind::Dynamic));
    }
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
    let w = match budget {
        Some(b) => w.with_checkpoint_budget(b),
        None => w,
    };
    w.with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: (1..=slots).collect() }]).unwrap()
}

fn rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    // v' = v + 2 w (u x v) + 2 u x (u x v), u the vector part
    let u = [q[0], q[1], q[2]];
    let cross =
        |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let t = cross(u, v).map(|c| 2.0 * c);
    let t2 = cross(u, t);
    [v[0] + q[3] * t[0] + t2[0], v[1] + q[3] * t[1] + t2[1], v[2] + q[3] * t[2] + t2[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

/// The world position of the centre of mass of `cells` of the body that has the pose `pose`.
fn centre_of(pose: &Pose3, cells: Vec<[i32; 3]>) -> ([f64; 3], f64) {
    let n = cells.len() as f64;
    let props = shape_mass_properties(&Shape3::Voxels { size: SIZE, cells }, n * CELL_MASS, 1.0).unwrap();
    (add(pose.pos, rotate(pose.rot, props.centre)), props.mass)
}

#[test]
fn before_the_cut_only_the_bar_is_in_the_world_and_after_it_the_piece_has_the_motion_of_the_part_of_the_bar_it_was() {
    let mut w = world(2, None);
    let mut driver = Cutter::new(0.5);
    let early = w.frame_at(0.3, &mut driver);
    assert!(early.errors.is_empty(), "{:?}", early.errors);
    assert_eq!(early.enabled, vec![true, false, false], "the slots do not take part before they are used");
    let (pose, v) = (early.bodies[0], early.velocities[0].linear);
    let (_, m_old) = centre_of(&pose, bar());
    assert!((m_old - 48.0 * CELL_MASS).abs() < 1e-9);
    let late = w.frame_at(0.7, &mut driver);
    assert!(late.errors.is_empty(), "{:?}", late.errors);
    assert_eq!(late.enabled, vec![true, true, false], "the first slot is the piece, the second is still free");
    // the piece has the velocity that its cells had as part of the bar: v + w x (c - c_old), and the same spin
    let at_cut = w.frame_at(0.5, &mut driver);
    // a free body that is not a sphere changes its spin slowly (Euler's equations), so the spin is the one at the cut, which the cut leaves
    let spin = at_cut.velocities[0].angular;
    let omega = spin.map(f64::to_radians);
    let (piece_pose, piece_v, piece_spin) =
        (at_cut.bodies[1], at_cut.velocities[1].linear, at_cut.velocities[1].angular);
    assert_eq!(
        piece_pose.pos.map(f64::to_bits),
        at_cut.bodies[0].pos.map(f64::to_bits),
        "the piece starts in the frame of the bar"
    );
    // the centre of mass of the whole bar at the instant of the cut, which is where the body's frame is then
    let (c_old, _) = centre_of(&at_cut.bodies[0], bar());
    let (c_piece, m_piece) = centre_of(&at_cut.bodies[0], cells_of(6..12, 0..2, 0..2));
    let wanted = add(v, cross(omega, sub(c_piece, c_old)));
    for k in 0..3 {
        assert!((piece_v[k] - wanted[k]).abs() < 1e-12, "piece velocity {k}: {} against {}", piece_v[k], wanted[k]);
        assert!((piece_spin[k] - spin[k]).abs() < 1e-9, "piece spin {k}");
    }
    assert!((m_piece - 24.0 * CELL_MASS).abs() < 1e-9);
    // the part that stayed has the velocity of its own centre of mass, and its spin
    let (c_stay, m_stay) = centre_of(&at_cut.bodies[0], cells_of(0..5, 0..2, 0..2));
    let wanted = add(v, cross(omega, sub(c_stay, c_old)));
    let stay_v = at_cut.velocities[0].linear;
    for k in 0..3 {
        assert!((stay_v[k] - wanted[k]).abs() < 1e-12, "remaining velocity {k}: {} against {}", stay_v[k], wanted[k]);
    }
    assert!((m_stay - 20.0 * CELL_MASS).abs() < 1e-9);
    // the slice that was destroyed is not in either: with the velocity it had, momentum and angular momentum about the old centre are
    // what they were (the destroyed cells' share is the rigid motion of the bar)
    let (c_gone, m_gone) = centre_of(&at_cut.bodies[0], cells_of(5..6, 0..2, 0..2));
    let gone_v = add(v, cross(omega, sub(c_gone, c_old)));
    let momentum: [f64; 3] = [
        m_stay * stay_v[0] + m_piece * piece_v[0] + m_gone * gone_v[0],
        m_stay * stay_v[1] + m_piece * piece_v[1] + m_gone * gone_v[1],
        m_stay * stay_v[2] + m_piece * piece_v[2] + m_gone * gone_v[2],
    ];
    let before = scale(v, m_old);
    for k in 0..3 {
        assert!(
            (momentum[k] - before[k]).abs() < 1e-12 * m_old * 2.0,
            "momentum {k}: {} against {}",
            momentum[k],
            before[k]
        );
    }
    // and the cut happens once: the driver was asked again with its revision and said nothing
    assert!(driver.asked.contains(&Some(1)));
}

#[test]
fn the_pieces_take_the_slots_in_order_and_there_must_be_slots_for_them() {
    let mut w = world(2, None);
    let mut driver = Cutter { pieces: 2, ..Cutter::new(0.2) };
    let frame = w.frame_at(0.4, &mut driver);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert_eq!(frame.enabled, vec![true, true, true]);
    // the two pieces are the two ends of the far part, the first nearer the cut
    let (c1, _) = centre_of(&frame.bodies[1], cells_of(6..9, 0..2, 0..2));
    let (c2, _) = centre_of(&frame.bodies[2], cells_of(9..12, 0..2, 0..2));
    assert!(c1[0] < c2[0] + 1.0);
    // with one slot for two pieces the world says so, and does not take the step
    let mut short = world(1, None);
    let failed = short.frame_at(0.4, &mut Cutter { pieces: 2, ..Cutter::new(0.2) });
    assert!(failed.errors.iter().any(|e| e.contains("slot")), "{:?}", failed.errors);
}

#[test]
fn the_world_replays_the_cut_the_same_from_any_checkpoint_and_in_any_order_of_asking() {
    let times = [1.5, 0.3, 0.9, 1.2, 0.5, 1.5, 0.45, 2.0];
    let reference: Vec<_> = {
        let mut w = world(2, None);
        let mut driver = Cutter::new(0.5);
        times.iter().map(|t| w.frame_at(*t, &mut driver)).collect()
    };
    // a world that keeps next to no checkpoints replays from the start each time, and one asked in another order
    for budget in [Some(0), Some(4_000), None] {
        let mut w = world(2, budget);
        let mut driver = Cutter::new(0.5);
        for (t, want) in times.iter().zip(&reference) {
            let got = w.frame_at(*t, &mut driver);
            assert!(got.errors.is_empty(), "{:?}", got.errors);
            assert_eq!(got.enabled, want.enabled, "t = {t}");
            for k in 0..3 {
                assert_eq!(
                    got.bodies[k].pos.map(f64::to_bits),
                    want.bodies[k].pos.map(f64::to_bits),
                    "t = {t}, body {k}"
                );
                assert_eq!(
                    got.bodies[k].rot.map(f64::to_bits),
                    want.bodies[k].rot.map(f64::to_bits),
                    "t = {t}, body {k}"
                );
                assert_eq!(got.velocities[k], want.velocities[k], "t = {t}, body {k}");
            }
        }
    }
}

#[test]
fn a_body_of_cells_is_charged_to_the_checkpoints_by_its_cells() {
    // what an edited body keeps in the checkpoints taken before the edit is a private copy of its shape, about two bytes a cell: so the
    // charge of a checkpoint grows with the cells of the bodies, as that of a mesh does with its triangles
    let bytes = |n: i32| {
        let cells = cells_of(0..n, 0..20, 0..10);
        let mut w = World3::new(World3Spec {
            fix_internal_edges: false,
            start: 0.,
            step: 0.01,
            gravity: [0.; 3],
            pixels_per_meter: 1.,
            iterations: 8,
            bounds: Bounds3::None,
            joints: vec![],
            bodies: vec![body(Shape3::Voxels { size: SIZE, cells }, 1000.0, BodyKind::Dynamic)],
        });
        let frame = w.frame_at(3.5, &mut Cutter::new(99.0));
        assert!(frame.errors.is_empty(), "{:?}", frame.errors);
        (w.checkpoint_bytes(), w.progress().1)
    };
    let ((small, kept), (big, kept_big)) = (bytes(2), bytes(120));
    println!(
        "VOXEL SPLIT checkpoint bytes: {small} for 400 cells, {big} for 24 000 ({kept} and {kept_big} checkpoints)"
    );
    assert_eq!(kept, kept_big);
    let taken = kept - 1;
    assert!(big - small >= taken * 2 * (24_000 - 400), "{} against {}", big - small, taken * 2 * 23_600);
}

#[test]
fn a_split_that_names_a_body_that_is_not_made_of_cells_is_refused() {
    let w = World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.,
        step: 0.01,
        gravity: [0.; 3],
        pixels_per_meter: 1.,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies: vec![
            body(Shape3::Box([1.0; 3]), 1.0, BodyKind::Dynamic),
            body(Shape3::Voxels { size: SIZE, cells: vec![[0, 0, 0]] }, 1.0, BodyKind::Dynamic),
        ],
    });
    assert!(w.with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: vec![1] }]).is_err());
    let w = World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.,
        step: 0.01,
        gravity: [0.; 3],
        pixels_per_meter: 1.,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies: vec![
            body(Shape3::Voxels { size: SIZE, cells: bar() }, 1.0, BodyKind::Dynamic),
            body(Shape3::Voxels { size: SIZE, cells: vec![[0, 0, 0]] }, 1.0, BodyKind::Static),
        ],
    });
    assert!(w.with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: vec![1] }]).is_err(), "a slot is a dynamic body");
}

/// A driver that cuts several bodies at several times: each cut is for a parent, applies from a time and only to the state that has the
/// revision it was made against (so a second cut is the difference from the first).
struct Script {
    cuts: Vec<(usize, f64, Option<u64>, VoxelCut3)>,
}

impl Driver3 for Script {
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
        Ok(self
            .cuts
            .iter()
            .find(|(p, at, against, _)| *p == parent && t + 1e-9 >= *at && *against == revision)
            .map(|c| c.3.clone()))
    }
}

/// The first cut of the tests above, as a script entry: a slice at x = 5 destroyed, the part with x under 5 stays, the rest separates.
fn first_cut(at: f64) -> (usize, f64, Option<u64>, VoxelCut3) {
    (
        0,
        at,
        None,
        VoxelCut3 {
            revision: 1,
            destroyed: cells_of(5..6, 0..2, 0..2),
            parent_mass: 20.0 * CELL_MASS,
            pieces: vec![VoxelPiece3 { cells: cells_of(6..12, 0..2, 0..2), mass: 24.0 * CELL_MASS }],
        },
    )
}

/// The velocity of the point of a body at `c` that has the centre of mass `c0`, the velocity `v` of it and the spin `w` (rad/s).
fn point_velocity(v: [f64; 3], w: [f64; 3], c0: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    add(v, cross(w, sub(c, c0)))
}

#[test]
fn a_second_cut_is_the_difference_from_the_first_and_the_next_piece_takes_the_next_slot() {
    // at 1.0 s the part that stayed (x 0..5) loses its slice at x = 2: the cells at x 0..2 separate, those at 3..5 stay
    let second = (
        0,
        1.0,
        Some(1),
        VoxelCut3 {
            revision: 2,
            destroyed: cells_of(2..3, 0..2, 0..2),
            parent_mass: 8.0 * CELL_MASS,
            pieces: vec![VoxelPiece3 { cells: cells_of(0..2, 0..2, 0..2), mass: 8.0 * CELL_MASS }],
        },
    );
    let mut w = world(2, None);
    let mut driver = Script { cuts: vec![first_cut(0.5), second.clone()] };
    let before = w.frame_at(0.9, &mut driver);
    assert!(before.errors.is_empty(), "{:?}", before.errors);
    assert_eq!(before.enabled, vec![true, true, false]);
    // the state of the remaining part just before its second cut, and just after
    // (a frame is the state at the start of a step, which a cut of that step has already changed: the world has to have taken the step
    // for the frame to show the cut, so it is asked past it first)
    w.frame_at(1.2, &mut driver);
    let at_cut = w.frame_at(1.0, &mut driver);
    assert!(at_cut.errors.is_empty(), "{:?}", at_cut.errors);
    assert_eq!(at_cut.enabled, vec![true, true, true], "the second piece took the second slot");
    let pose = at_cut.bodies[0];
    let omega = at_cut.velocities[0].angular.map(f64::to_radians);
    // before the cut the remaining part moved with the velocity of its centre of mass, which is that of the point of the body where it is
    let (c_before, m_before) = centre_of(&pose, cells_of(0..5, 0..2, 0..2));
    assert!((m_before - 20.0 * CELL_MASS).abs() < 1e-9);
    // the velocity of the part's centre of mass just before: the frame before the cut (the step at 0.99), moved on by nothing since there is no force
    let v_before = w.frame_at(0.99, &mut driver).velocities[0].linear;
    let (c_piece, _) = centre_of(&pose, cells_of(0..2, 0..2, 0..2));
    let wanted = point_velocity(v_before, omega, c_before, c_piece);
    let got = at_cut.velocities[2].linear;
    for k in 0..3 {
        assert!((got[k] - wanted[k]).abs() < 1e-9, "second piece, velocity {k}: {} against {}", got[k], wanted[k]);
        assert!((at_cut.velocities[2].angular[k] - at_cut.velocities[0].angular[k]).abs() < 1e-9);
    }
    // and the part that stays has the velocity of its own centre
    let (c_stay, _) = centre_of(&pose, cells_of(3..5, 0..2, 0..2));
    let wanted = point_velocity(v_before, omega, c_before, c_stay);
    let got = at_cut.velocities[0].linear;
    for k in 0..3 {
        assert!((got[k] - wanted[k]).abs() < 1e-9, "stays, velocity {k}: {} against {}", got[k], wanted[k]);
    }
    // a third cut has no slot: the world says so
    let third = (
        0,
        1.5,
        Some(2),
        VoxelCut3 {
            revision: 3,
            destroyed: cells_of(3..4, 0..2, 0..2),
            parent_mass: 4.0 * CELL_MASS,
            pieces: vec![VoxelPiece3 { cells: cells_of(4..5, 0..2, 0..2), mass: 4.0 * CELL_MASS }],
        },
    );
    let mut full = Script { cuts: vec![first_cut(0.5), second, third] };
    let mut short = world(2, None);
    let failed = short.frame_at(1.6, &mut full);
    assert!(failed.errors.iter().any(|e| e.contains("slot")), "{:?}", failed.errors);
    // and the replay from any checkpoint of the two cuts is the same to the bit
    let mut reference = world(2, None);
    let mut two = Script { cuts: vec![first_cut(0.5), driver.cuts[1].clone()] };
    let times = [1.4, 0.6, 1.0, 0.95, 1.4, 0.2, 1.1];
    let wanted: Vec<_> = times.iter().map(|t| reference.frame_at(*t, &mut two)).collect();
    for budget in [Some(0), Some(4_000)] {
        let mut replay = world(2, budget);
        let mut script = Script { cuts: vec![first_cut(0.5), driver.cuts[1].clone()] };
        for (t, want) in times.iter().zip(&wanted) {
            let got = replay.frame_at(*t, &mut script);
            assert!(got.errors.is_empty(), "{:?}", got.errors);
            for k in 0..3 {
                assert_eq!(
                    got.bodies[k].pos.map(f64::to_bits),
                    want.bodies[k].pos.map(f64::to_bits),
                    "t = {t}, body {k}"
                );
                assert_eq!(got.velocities[k], want.velocities[k], "t = {t}, body {k}");
            }
        }
    }
}

#[test]
fn a_slot_can_be_the_parent_of_another_split_once_it_has_been_used() {
    // body 0 is the bar, 1 and 2 are its slots; body 1 is also the parent of a split with the slot 3
    let mut parent = body(Shape3::Voxels { size: SIZE, cells: bar() }, 48.0 * CELL_MASS, BodyKind::Dynamic);
    parent.velocity = [1.0, 0.4, -0.3];
    parent.angular_velocity = [20.0, 10.0, 40.0];
    let mut bodies = vec![parent];
    for _ in 0..3 {
        bodies.push(body(Shape3::Voxels { size: SIZE, cells: vec![[0, 0, 0]] }, CELL_MASS, BodyKind::Dynamic));
    }
    let make = |budget: Option<usize>| {
        let w = World3::new(World3Spec {
            fix_internal_edges: false,
            start: 0.,
            step: 0.01,
            gravity: [0.; 3],
            pixels_per_meter: 1.,
            iterations: 8,
            bounds: Bounds3::None,
            joints: vec![],
            bodies: bodies.clone(),
        });
        let w = match budget {
            Some(b) => w.with_checkpoint_budget(b),
            None => w,
        };
        w.with_voxel_splits(vec![
            VoxelSplit3 { parent: 0, slots: vec![1, 2] },
            VoxelSplit3 { parent: 1, slots: vec![3] },
        ])
        .unwrap()
    };
    // the piece in slot 1 is the cells x 6..12; at 1.0 s it loses the slice x = 9 and the cells x 10..12 separate into slot 3
    let piece_cut = (
        1,
        1.0,
        None,
        VoxelCut3 {
            revision: 1,
            destroyed: cells_of(9..10, 0..2, 0..2),
            parent_mass: 12.0 * CELL_MASS,
            pieces: vec![VoxelPiece3 { cells: cells_of(10..12, 0..2, 0..2), mass: 8.0 * CELL_MASS }],
        },
    );
    let script = || Script { cuts: vec![first_cut(0.5), piece_cut.clone()] };
    let mut w = make(None);
    let mut driver = script();
    // before the first cut the slot is not a body of the world and its split does nothing, whatever the driver would say
    let early = w.frame_at(0.4, &mut driver);
    assert!(early.errors.is_empty(), "{:?}", early.errors);
    assert_eq!(early.enabled, vec![true, false, false, false]);
    let mid = w.frame_at(0.9, &mut driver);
    assert_eq!(mid.enabled, vec![true, true, false, false]);
    w.frame_at(1.2, &mut driver);
    let after = w.frame_at(1.0, &mut driver);
    assert!(after.errors.is_empty(), "{:?}", after.errors);
    assert_eq!(
        after.enabled,
        vec![true, true, false, true],
        "the piece of the piece took the slot of the second split"
    );
    // the part of the piece that separates moves as that part of the piece did: the piece's centre and spin
    let pose = after.bodies[1];
    let omega = after.velocities[1].angular.map(f64::to_radians);
    let (c_piece, _) = centre_of(&pose, cells_of(6..12, 0..2, 0..2));
    let (c_far, _) = centre_of(&pose, cells_of(10..12, 0..2, 0..2));
    let v_piece = w.frame_at(0.99, &mut driver).velocities[1].linear;
    let wanted = point_velocity(v_piece, omega, c_piece, c_far);
    let got = after.velocities[3].linear;
    for k in 0..3 {
        assert!((got[k] - wanted[k]).abs() < 1e-9, "velocity {k}: {} against {}", got[k], wanted[k]);
    }
    // the replay of both levels is the same to the bit
    let times = [1.3, 0.45, 0.95, 1.0, 1.3, 0.2];
    let mut first = make(None);
    let mut d = script();
    let wanted: Vec<_> = times.iter().map(|t| first.frame_at(*t, &mut d)).collect();
    for budget in [Some(0), Some(6_000)] {
        let mut replay = make(budget);
        let mut d = script();
        for (t, want) in times.iter().zip(&wanted) {
            let got = replay.frame_at(*t, &mut d);
            assert!(got.errors.is_empty(), "{:?}", got.errors);
            for k in 0..4 {
                assert_eq!(
                    got.bodies[k].pos.map(f64::to_bits),
                    want.bodies[k].pos.map(f64::to_bits),
                    "t = {t}, body {k}"
                );
                assert_eq!(got.velocities[k], want.velocities[k], "t = {t}, body {k}");
            }
        }
    }
}
