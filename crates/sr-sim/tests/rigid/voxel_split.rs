//! A body of cells that breaks at run time: the cells a cut destroys leave it, the pieces that separate take the slots made for them
//! with the motion they had, and the world replays the same whenever it is asked.
#![allow(clippy::needless_range_loop)]

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
    world_logging(slots, budget, None)
}

/// The same with a budget for the frames it keeps (none: the default); with 0 every frame asked for is computed, from a checkpoint.
fn world_logging(slots: usize, budget: Option<usize>, frames: Option<usize>) -> World3 {
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
    let w = match frames {
        Some(b) => w.with_frame_log_budget(b),
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

/// A driver that cuts as a [`Cutter`] does and remembers the pose and the centre of mass that each body had at the start of its last step.
struct Watch {
    inner: Cutter,
    seen: Vec<Option<(Pose3, [f64; 3])>>,
}

impl Driver3 for Watch {
    fn kinematic(&mut self, t: f64, which: &[usize]) -> Vec<Pose3> {
        self.inner.kinematic(t, which)
    }
    fn fields(&mut self, t: f64) -> Vec<Field> {
        self.inner.fields(t)
    }
    fn voxel_cut(
        &mut self,
        t: f64,
        parent: usize,
        revision: Option<u64>,
        i: Option<&Impact3>,
    ) -> Result<Option<VoxelCut3>, String> {
        self.inner.voxel_cut(t, parent, revision, i)
    }
    fn load(&mut self, _: u64, _: f64, body: usize, state: &BodyState) -> Result<Option<Load3>, String> {
        self.seen[body] = Some((state.pose, state.centre));
        Ok(None)
    }
}

/// The centre of mass of a body in its own frame, from its pose and the world position of its centre.
fn local_centre(pose: &Pose3, centre: [f64; 3]) -> [f64; 3] {
    let conjugate = [-pose.rot[0], -pose.rot[1], -pose.rot[2], pose.rot[3]];
    rotate(conjugate, sub(centre, pose.pos))
}

#[test]
fn the_pieces_take_the_slots_in_order_and_there_must_be_slots_for_them() {
    let mut w = world(2, None);
    let mut driver = Watch { inner: Cutter { pieces: 2, ..Cutter::new(0.2) }, seen: vec![None; 3] };
    let frame = w.frame_at(0.4, &mut driver);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert_eq!(frame.enabled, vec![true, true, true]);
    // what each slot holds, seen from the world: its centre of mass in its own frame is that of the cells of the piece that the cut gave it, in
    // order (the first piece, the cells x 6 to 8, in the first slot; the second, x 9 to 11, in the second)
    for (slot, cells) in [(1usize, cells_of(6..9, 0..2, 0..2)), (2, cells_of(9..12, 0..2, 0..2))] {
        let (pose, centre) = driver.seen[slot].expect("the slot has taken a step");
        let local = local_centre(&pose, centre);
        let wanted =
            shape_mass_properties(&Shape3::Voxels { size: SIZE, cells }, 12.0 * CELL_MASS, 1.0).unwrap().centre;
        for k in 0..3 {
            assert!((local[k] - wanted[k]).abs() < 1e-9, "slot {slot}, axis {k}: {} against {}", local[k], wanted[k]);
        }
    }
    // with one slot for two pieces the world says so, and does not take the step
    let mut short = world(1, None);
    let failed = short.frame_at(0.4, &mut Cutter { pieces: 2, ..Cutter::new(0.2) });
    assert!(failed.errors.iter().any(|e| e.contains("slot")), "{:?}", failed.errors);
}

#[test]
fn the_world_replays_the_cut_the_same_from_any_checkpoint_and_in_any_order_of_asking() {
    // the cut is at 1.5 s; the times go back and forth across it, and across the checkpoints taken each second
    let times = [3.5, 1.4, 2.2, 1.5, 1.6, 0.2, 3.0, 1.45, 1.51, 2.0];
    let reference: Vec<_> = {
        let mut w = world(2, None);
        let mut driver = Cutter::new(1.5);
        w.frame_at(3.5, &mut driver);
        times.iter().map(|t| w.frame_at(*t, &mut driver)).collect()
    };
    // worlds that keep no frames, so that every frame asked for is computed by replaying from a checkpoint: with the default budget for
    // checkpoints, with one that keeps next to none (only the first, which every world has) and with none
    for budget in [None, Some(4_000), Some(0)] {
        let mut w = world_logging(2, budget, Some(0));
        let mut driver = Cutter::new(1.5);
        let mut restores = 0;
        for (t, want) in times.iter().zip(&reference) {
            let got = w.frame_at(*t, &mut driver);
            same(&got, want, &format!("budget {budget:?}, t = {t}"));
            assert!(w.checkpoint_restores() >= restores, "the count does not go down");
            restores = w.checkpoint_restores();
        }
        // it did replay: frames that are before the world's state were taken from a checkpoint, and with the default budget there were
        // checkpoints other than the first to take them from
        let backward = times.windows(2).filter(|w| w[1] < w[0]).count() as u64;
        assert!(
            restores >= backward && restores > 0,
            "budget {budget:?}: {restores} restores for {backward} steps back"
        );
        if budget.is_none() {
            assert!(w.progress().1 >= 4, "{} checkpoints", w.progress().1);
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
    for budget in [None, Some(0)] {
        // no frames kept, so each is replayed from a checkpoint (counted)
        let mut replay = world_logging(2, budget, Some(0));
        let mut script = Script { cuts: vec![first_cut(0.5), driver.cuts[1].clone()] };
        for (t, want) in times.iter().zip(&wanted) {
            same(&replay.frame_at(*t, &mut script), want, &format!("budget {budget:?}, t = {t}"));
        }
        let backward = times.windows(2).filter(|w| w[1] < w[0]).count() as u64;
        assert!(
            replay.checkpoint_restores() >= backward,
            "{} restores for {backward} steps back",
            replay.checkpoint_restores()
        );
        // with the default budget a checkpoint other than the first was taken (each second), so the replays could start from one
        if budget.is_none() {
            assert!(replay.progress().1 > 1, "{} checkpoints", replay.progress().1);
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
    let make = |budget: Option<usize>, frames: Option<usize>| {
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
        let w = match frames {
            Some(b) => w.with_frame_log_budget(b),
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
    let mut w = make(None, None);
    let mut driver = script();
    // before the first cut the slot is not a body of the world and its split does nothing, whatever the driver would say
    let early = w.frame_at(0.4, &mut driver);
    assert!(early.errors.is_empty(), "{:?}", early.errors);
    assert_eq!(early.enabled, vec![true, false, false, false]);
    let mid = w.frame_at(0.9, &mut driver);
    assert_eq!(mid.enabled, vec![true, true, false, false]);
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
    // the replay of both levels is the same to the bit, from worlds that keep no frames and so replay from a checkpoint each time
    let times = [1.3, 0.45, 0.95, 1.0, 1.3, 0.2];
    let mut first = make(None, None);
    let mut d = script();
    let wanted: Vec<_> = times.iter().map(|t| first.frame_at(*t, &mut d)).collect();
    for budget in [None, Some(0)] {
        let mut replay = make(budget, Some(0));
        let mut d = script();
        for (t, want) in times.iter().zip(&wanted) {
            same(&replay.frame_at(*t, &mut d), want, &format!("budget {budget:?}, t = {t}"));
        }
        let backward = times.windows(2).filter(|w| w[1] < w[0]).count() as u64;
        assert!(
            replay.checkpoint_restores() >= backward,
            "{} restores for {backward} steps back",
            replay.checkpoint_restores()
        );
        // with the default budget a checkpoint other than the first was taken (each second), so the replays could start from one
        if budget.is_none() {
            assert!(replay.progress().1 > 1, "{} checkpoints", replay.progress().1);
        }
    }
}

/// Two frames are the same: the poses, the velocities and who takes part, to the bit.
fn same(a: &Frame3, b: &Frame3, why: &str) {
    assert!(a.errors.is_empty() && b.errors.is_empty(), "{why}: {:?} {:?}", a.errors, b.errors);
    assert_eq!(a.enabled, b.enabled, "{why}: who takes part");
    assert_eq!(a.bodies.len(), b.bodies.len());
    for k in 0..a.bodies.len() {
        assert_eq!(a.bodies[k].pos.map(f64::to_bits), b.bodies[k].pos.map(f64::to_bits), "{why}: position of body {k}");
        assert_eq!(a.bodies[k].rot.map(f64::to_bits), b.bodies[k].rot.map(f64::to_bits), "{why}: rotation of body {k}");
        assert_eq!(
            a.velocities[k].linear.map(f64::to_bits),
            b.velocities[k].linear.map(f64::to_bits),
            "{why}: velocity of body {k}"
        );
        assert_eq!(
            a.velocities[k].angular.map(f64::to_bits),
            b.velocities[k].angular.map(f64::to_bits),
            "{why}: spin of body {k}"
        );
    }
}

/// The frame at each of `steps` (around a cut), asked four ways, all equal to the bit: from a world run to 3 s whose log holds it (the reference),
/// from a fresh world asked for that time and nothing else, from a world asked for it, then for a later time, then for it again, and from a world
/// that keeps no frames and replays from a checkpoint (which has to have been taken and used). Returns the reference frames.
fn the_same_four_ways<D: Driver3>(
    make: &dyn Fn(Option<usize>) -> World3,
    driver: &dyn Fn() -> D,
    steps: &[u64],
) -> Vec<Frame3> {
    let at = |s: u64| s as f64 * 0.01;
    let mut reference = make(None);
    reference.frame_at(3.0, &mut driver());
    let wanted: Vec<Frame3> = steps.iter().map(|s| reference.frame_at(at(*s), &mut driver())).collect();
    for (i, s) in steps.iter().enumerate() {
        let t = at(*s);
        // (1) a world asked for this time and nothing else: nothing has been logged, the frame is computed
        let fresh = make(None).frame_at(t, &mut driver());
        same(&fresh, &wanted[i], &format!("fresh, step {s}"));
        // (2) asked, then asked later, then asked again: the second answer comes from what the later request logged
        let mut w = make(None);
        let mut d = driver();
        let first = w.frame_at(t, &mut d);
        w.frame_at(t + 0.2, &mut d);
        let again = w.frame_at(t, &mut d);
        same(&first, &wanted[i], &format!("first, step {s}"));
        same(&again, &wanted[i], &format!("after a later request, step {s}"));
        // (2b) asked many times: the answer does not change with how often it is asked
        let mut w = make(None);
        let mut d = driver();
        let times: Vec<Frame3> = (0..4).map(|_| w.frame_at(t, &mut d)).collect();
        for (k, f) in times.iter().enumerate() {
            same(f, &wanted[i], &format!("asked {} times, step {s}", k + 1));
        }
        let after = w.frame_at(t + 0.03, &mut d);
        let later = make(None).frame_at(t + 0.03, &mut driver());
        same(&after, &later, &format!("a later time after {s} was asked four times"));
        // (3) a world that keeps no frames, run to 3 s: the frame is computed from a checkpoint, which has to have been taken and used
        let mut replay = make(Some(0));
        let mut d = driver();
        replay.frame_at(3.0, &mut d);
        assert!(replay.progress().1 >= 3, "{} checkpoints", replay.progress().1);
        let restored = replay.checkpoint_restores();
        let got = replay.frame_at(t, &mut d);
        assert!(replay.checkpoint_restores() > restored, "step {s} was not replayed from a checkpoint");
        same(&got, &wanted[i], &format!("replayed, step {s}"));
    }
    wanted
}

#[test]
fn the_frame_at_every_step_across_a_cut_is_the_same_fresh_logged_asked_in_any_order_and_replayed_from_a_checkpoint() {
    // the cut is at 1.5 s, step 150; a checkpoint is taken each second, so the frames around it are replayed from the one at step 100
    let steps: Vec<u64> = (145..=156).collect();
    let wanted = the_same_four_ways(&|frames| world_logging(2, None, frames), &|| Cutter::new(1.5), &steps);
    assert!(
        wanted[0].enabled == vec![true, false, false] && wanted[11].enabled == vec![true, true, false],
        "the cut is inside the steps"
    );
}

#[test]
fn chained_splits_cut_the_slot_one_step_after_it_is_used_however_often_and_in_whatever_order_the_frames_are_asked() {
    // body 0 is the bar, 1 and 2 are its slots, 3 is the slot of the split whose parent is body 1; the child's cut is scripted at the same instant as
    // the parent's, so it has to wait for the step after: the piece is not a body in use before the step the parent is cut in is over
    let make = |frames: Option<usize>| {
        let mut parent = body(Shape3::Voxels { size: SIZE, cells: bar() }, 48.0 * CELL_MASS, BodyKind::Dynamic);
        parent.velocity = [1.0, 0.4, -0.3];
        parent.angular_velocity = [20.0, 10.0, 40.0];
        let mut bodies = vec![parent];
        for _ in 0..3 {
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
        let w = match frames {
            Some(b) => w.with_frame_log_budget(b),
            None => w,
        };
        w.with_voxel_splits(vec![
            VoxelSplit3 { parent: 0, slots: vec![1, 2] },
            VoxelSplit3 { parent: 1, slots: vec![3] },
        ])
        .unwrap()
    };
    let script = || Script {
        cuts: vec![
            first_cut(1.5),
            (
                1,
                1.5,
                None,
                VoxelCut3 {
                    revision: 1,
                    destroyed: cells_of(9..10, 0..2, 0..2),
                    parent_mass: 12.0 * CELL_MASS,
                    pieces: vec![VoxelPiece3 { cells: cells_of(10..12, 0..2, 0..2), mass: 8.0 * CELL_MASS }],
                },
            ),
        ],
    };
    let steps: Vec<u64> = (147..=153).collect();
    let wanted = the_same_four_ways(&make, &script, &steps);
    // the parent is cut at step 150 and the child, scripted for the same instant, in the step after
    let enabled: Vec<Vec<bool>> = wanted.iter().map(|f| f.enabled.clone()).collect();
    assert_eq!(enabled[2], vec![true, false, false, false], "step 149: nothing yet");
    assert_eq!(enabled[3], vec![true, true, false, false], "step 150: the parent is cut, the child waits");
    assert_eq!(enabled[4], vec![true, true, false, true], "step 151: the child is cut");
}

/// A driver that cuts as a [`Script`] does and pushes every body from a time on with a force and a torque, which is how a test sees the
/// mass and the inertia that the world gave a body: a = F / m, alpha = I^-1 tau. `closed` hides a body until a time.
struct Pusher {
    script: Script,
    from: f64,
    force: [f64; 3],
    torque: [f64; 3],
    closed: Vec<(usize, f64)>,
}

impl Driver3 for Pusher {
    fn kinematic(&mut self, t: f64, which: &[usize]) -> Vec<Pose3> {
        self.script.kinematic(t, which)
    }
    fn fields(&mut self, t: f64) -> Vec<Field> {
        self.script.fields(t)
    }
    fn voxel_cut(
        &mut self,
        t: f64,
        parent: usize,
        revision: Option<u64>,
        i: Option<&Impact3>,
    ) -> Result<Option<VoxelCut3>, String> {
        self.script.voxel_cut(t, parent, revision, i)
    }
    fn enabled(&mut self, t: f64, which: usize) -> bool {
        !self.closed.iter().any(|(k, until)| *k == which && t < *until)
    }
    fn load(&mut self, _step: u64, t: f64, _body: usize, _state: &BodyState) -> Result<Option<Load3>, String> {
        Ok((t + 1e-9 >= self.from).then_some(Load3 { force: self.force, torque: self.torque }))
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

/// A world of the bar at rest in space with `slots` slots.
fn resting(slots: usize) -> World3 {
    let mut parent = body(Shape3::Voxels { size: SIZE, cells: bar() }, 48.0 * CELL_MASS, BodyKind::Dynamic);
    parent.start = Pose3 { pos: [0.0, 0.0, 0.0], rot: [0., 0., 0., 1.] };
    let mut bodies = vec![parent];
    for _ in 0..slots {
        bodies.push(body(Shape3::Voxels { size: SIZE, cells: vec![[0, 0, 0]] }, CELL_MASS, BodyKind::Dynamic));
    }
    World3::new(World3Spec {
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
    .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: (1..=slots).collect() }])
    .unwrap()
}

/// What the world made of the mass and the inertia of the part of the bar that stays and of the piece: pushed from `from` s, a body speeds up
/// as F / m and spins up as I^-1 tau, with the m and I of its own cells. `stay` and `piece` are the cells the cut leaves in the parent and gives
/// the piece.
fn push_and_measure(
    closed: Vec<(usize, f64)>,
    from: f64,
    ask: (f64, f64),
    cut: (usize, f64, Option<u64>, VoxelCut3),
    stay: Vec<[i32; 3]>,
    piece: Vec<[i32; 3]>,
) {
    let (force, torque) = ([120.0, -40.0, 75.0], [30.0, 55.0, -45.0]);
    let mut w = resting(1);
    let mut d = Pusher { script: Script { cuts: vec![cut] }, from, force, torque, closed };
    let (a, b) = (w.frame_at(ask.0, &mut d), w.frame_at(ask.1, &mut d));
    assert!(a.errors.is_empty() && b.errors.is_empty(), "{:?} {:?}", a.errors, b.errors);
    let dt = ask.1 - ask.0;
    for (k, cells) in [(0usize, stay), (1, piece)] {
        let mass = cells.len() as f64 * CELL_MASS;
        let props = shape_mass_properties(&Shape3::Voxels { size: SIZE, cells }, mass, 1.0).unwrap();
        for i in 0..3 {
            let accel = (b.velocities[k].linear[i] - a.velocities[k].linear[i]) / dt;
            let wanted = force[i] / mass;
            assert!(
                (accel - wanted).abs() < 1e-9,
                "body {k}, axis {i}: acceleration {accel} against {wanted}: the mass is not {mass}"
            );
        }
        let inverse = inverse3(props.inertia);
        for i in 0..3 {
            let alpha = (b.velocities[k].angular[i] - a.velocities[k].angular[i]).to_radians() / dt;
            let wanted: f64 = (0..3).map(|j| inverse[i][j] * torque[j]).sum();
            assert!(
                (alpha - wanted).abs() < 2e-3 * wanted.abs().max(1e-3),
                "body {k}, axis {i}: spin-up {alpha} against {wanted}: the inertia is not that of its cells"
            );
        }
    }
}

/// A cut whose piece is an L, which has products of inertia, so that a swap or a rotation of the axes shows: the slice x = 5 is destroyed, and so is
/// the notch of the far end (x 9 to 11, y = 1), which leaves the cells x 6 to 11 with the notch out as the piece.
/// A cut as the script holds it: the parent, the time from which it applies, the revision it is made against, and the cut.
type Scripted = (usize, f64, Option<u64>, VoxelCut3);

fn the_l_cut(at: f64) -> (Scripted, Vec<[i32; 3]>, Vec<[i32; 3]>) {
    let notch = cells_of(9..12, 1..2, 0..2);
    let piece: Vec<[i32; 3]> = cells_of(6..12, 0..2, 0..2).into_iter().filter(|c| !notch.contains(c)).collect();
    let mut destroyed = cells_of(5..6, 0..2, 0..2);
    destroyed.extend(notch);
    let cut = VoxelCut3 {
        revision: 1,
        destroyed,
        parent_mass: 20.0 * CELL_MASS,
        pieces: vec![VoxelPiece3 { cells: piece.clone(), mass: piece.len() as f64 * CELL_MASS }],
    };
    ((0, at, None, cut), cells_of(0..5, 0..2, 0..2), piece)
}

#[test]
fn what_the_world_gives_the_part_that_stays_and_the_piece_is_the_mass_and_the_inertia_of_their_cells() {
    // pushed from 0.4 s, a tenth of a second after the cut, with the measurement over four steps
    push_and_measure(
        vec![],
        0.4,
        (0.41, 0.45),
        first_cut(0.3),
        cells_of(0..5, 0..2, 0..2),
        cells_of(6..12, 0..2, 0..2),
    );
    // and with a piece that is an L, whose tensor has products
    let (cut, stay, piece) = the_l_cut(0.3);
    push_and_measure(vec![], 0.4, (0.41, 0.45), cut, stay, piece);
}

#[test]
fn a_piece_whose_slot_is_hidden_when_it_is_cut_has_the_mass_and_the_inertia_of_its_cells_when_it_shows() {
    // the slot is hidden by the driver until 0.8 s, the cut is at 0.3 s: it is a body that the world has made and does not yet show
    push_and_measure(
        vec![(1, 0.8)],
        0.9,
        (0.91, 0.95),
        first_cut(0.3),
        cells_of(0..5, 0..2, 0..2),
        cells_of(6..12, 0..2, 0..2),
    );
    let (cut, stay, piece) = the_l_cut(0.3);
    push_and_measure(vec![(1, 0.8)], 0.9, (0.91, 0.95), cut, stay, piece);
}

#[test]
fn a_body_that_loses_all_its_cells_stays_out_of_the_world_and_its_pieces_are_the_bodies_that_are_left() {
    // the bar is cut in two and nothing stays: both halves are pieces and the bar is disabled for good
    let all = (
        0,
        0.5,
        None,
        VoxelCut3 {
            revision: 1,
            destroyed: vec![],
            parent_mass: 0.0,
            pieces: vec![
                VoxelPiece3 { cells: cells_of(0..6, 0..2, 0..2), mass: 24.0 * CELL_MASS },
                VoxelPiece3 { cells: cells_of(6..12, 0..2, 0..2), mass: 24.0 * CELL_MASS },
            ],
        },
    );
    let mut w = world(2, None);
    let mut d = Script { cuts: vec![all] };
    for t in [0.4, 0.5, 0.51, 0.6, 1.0, 2.5] {
        let frame = w.frame_at(t, &mut d);
        assert!(frame.errors.is_empty(), "{:?}", frame.errors);
        let wanted = if t < 0.5 - 1e-9 { vec![true, false, false] } else { vec![false, true, true] };
        assert_eq!(frame.enabled, wanted, "t = {t}: a body with nothing left comes back");
    }
    // and from a world that replays it
    let mut replay = world_logging(2, None, Some(0));
    let frame = replay.frame_at(2.5, &mut d);
    assert_eq!(frame.enabled, vec![false, true, true]);
    let back = replay.frame_at(0.7, &mut d);
    assert_eq!(back.enabled, vec![false, true, true]);
    assert!(replay.checkpoint_restores() >= 1);
}

/// Two bars with a split each: body 0 with slot 1 and body 2 with slot 3.
fn two_bars() -> World3 {
    let bar_body = || body(Shape3::Voxels { size: SIZE, cells: bar() }, 48.0 * CELL_MASS, BodyKind::Dynamic);
    let slot = || body(Shape3::Voxels { size: SIZE, cells: vec![[0, 0, 0]] }, CELL_MASS, BodyKind::Dynamic);
    let mut second = bar_body();
    second.start = Pose3 { pos: [10.0, 0.0, 0.0], rot: [0., 0., 0., 1.] };
    let mut slot2 = slot();
    slot2.start = second.start;
    World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.,
        step: 0.01,
        gravity: [0.; 3],
        pixels_per_meter: 1.,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies: vec![bar_body(), slot(), second, slot2],
    })
    .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: vec![1] }, VoxelSplit3 { parent: 2, slots: vec![3] }])
    .unwrap()
}

#[test]
fn a_step_whose_second_cut_cannot_be_installed_does_not_install_the_first() {
    let good = first_cut(0.5);
    // the second bar is cut by a cell it does not have
    let bad = (
        2,
        0.5,
        None,
        VoxelCut3 {
            revision: 1,
            destroyed: vec![[40, 0, 0]],
            parent_mass: 20.0 * CELL_MASS,
            pieces: vec![VoxelPiece3 { cells: cells_of(6..12, 0..2, 0..2), mass: 24.0 * CELL_MASS }],
        },
    );
    let mut w = two_bars();
    let failed = w.frame_at(0.7, &mut Script { cuts: vec![good.clone(), bad] });
    assert!(failed.errors.iter().any(|e| e.contains("cell")), "{:?}", failed.errors);
    // the world is as it was: asked by a driver that cuts nothing, the first bar has not been cut
    let after = w.frame_at(0.7, &mut Script { cuts: vec![] });
    assert!(after.errors.is_empty(), "{:?}", after.errors);
    assert_eq!(after.enabled, vec![true, false, true, false], "the cut that could have been installed was not");
    // and a driver that gets it right gets what a world that never failed gets, to the bit
    let right = (
        2,
        0.5,
        None,
        VoxelCut3 {
            revision: 1,
            destroyed: cells_of(5..6, 0..2, 0..2),
            parent_mass: 20.0 * CELL_MASS,
            pieces: vec![VoxelPiece3 { cells: cells_of(6..12, 0..2, 0..2), mass: 24.0 * CELL_MASS }],
        },
    );
    let mut clean = two_bars();
    let want = clean.frame_at(1.0, &mut Script { cuts: vec![good.clone(), right.clone()] });
    let got = w.frame_at(1.0, &mut Script { cuts: vec![good, right] });
    same(&got, &want, "after a failed step");
    assert_eq!(got.enabled, vec![true, true, true, true]);
}

#[test]
fn a_split_that_could_never_work_is_refused_when_it_is_registered() {
    let bar_body = || body(Shape3::Voxels { size: SIZE, cells: bar() }, 48.0 * CELL_MASS, BodyKind::Dynamic);
    let make = || {
        World3::new(World3Spec {
            fix_internal_edges: false,
            start: 0.,
            step: 0.01,
            gravity: [0.; 3],
            pixels_per_meter: 1.,
            iterations: 8,
            bounds: Bounds3::None,
            joints: vec![],
            bodies: vec![bar_body(), bar_body(), bar_body(), bar_body()],
        })
    };
    // two splits of one body: which would take the cut?
    assert!(make()
        .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: vec![1] }, VoxelSplit3 { parent: 0, slots: vec![2] }])
        .is_err());
    // a ring: 0 makes 1, which makes 0 again, and neither could ever be a slot in use first
    assert!(make()
        .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: vec![1] }, VoxelSplit3 { parent: 1, slots: vec![0] }])
        .is_err());
    // a longer ring
    assert!(make()
        .with_voxel_splits(vec![
            VoxelSplit3 { parent: 0, slots: vec![1] },
            VoxelSplit3 { parent: 1, slots: vec![2] },
            VoxelSplit3 { parent: 2, slots: vec![0] },
        ])
        .is_err());
    // a chain is fine, and so is a body that is both a slot and a parent
    assert!(make()
        .with_voxel_splits(vec![
            VoxelSplit3 { parent: 0, slots: vec![1, 2] },
            VoxelSplit3 { parent: 1, slots: vec![3] },
        ])
        .is_ok());
    // a body that a fracture will replace cannot also be cut, and one that is cut cannot be fractured, in whichever order they are registered
    let fracture = |source: usize, pieces: Vec<usize>| Fracture3 {
        source,
        at: 1.0,
        radial_impulse: 0.0,
        fragments: pieces.into_iter().map(|body| Fragment3 { body, offset: [0.0; 3], impulse: [0.0; 3] }).collect(),
        contact: None,
    };
    let fractured = make().with_fractures(vec![fracture(0, vec![1])]).unwrap();
    assert!(
        fractured.with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: vec![2] }]).is_err(),
        "a fracture's source as the parent of a split"
    );
    let fractured = make().with_fractures(vec![fracture(0, vec![1])]).unwrap();
    assert!(
        fractured.with_voxel_splits(vec![VoxelSplit3 { parent: 2, slots: vec![1] }]).is_err(),
        "a fracture's piece as the slot of a split"
    );
    let fractured = make().with_fractures(vec![fracture(0, vec![1])]).unwrap();
    assert!(
        fractured.with_voxel_splits(vec![VoxelSplit3 { parent: 1, slots: vec![2] }]).is_err(),
        "a fracture's piece as the parent of a split"
    );
    let split = || make().with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: vec![1] }]).unwrap();
    assert!(
        split().with_fractures(vec![fracture(0, vec![2])]).is_err(),
        "the parent of a split as the source of a fracture"
    );
    assert!(
        split().with_fractures(vec![fracture(2, vec![1])]).is_err(),
        "the slot of a split as a piece of a fracture"
    );
    assert!(
        split().with_fractures(vec![fracture(1, vec![2])]).is_err(),
        "the slot of a split as the source of a fracture"
    );
    assert!(split().with_fractures(vec![fracture(2, vec![3])]).is_ok(), "bodies that no split owns");
}

/// A driver that cuts as a [`Script`] does and loads every body with a torque that depends on where its centre of mass is, which is what a driver
/// that reads the state of the body would do: so a step in which a body is cut finds a different load if the load is asked before the cut and
/// if it is asked after.
struct Leaning {
    script: Script,
}

impl Driver3 for Leaning {
    fn kinematic(&mut self, t: f64, which: &[usize]) -> Vec<Pose3> {
        self.script.kinematic(t, which)
    }
    fn fields(&mut self, t: f64) -> Vec<Field> {
        self.script.fields(t)
    }
    fn voxel_cut(
        &mut self,
        t: f64,
        parent: usize,
        revision: Option<u64>,
        i: Option<&Impact3>,
    ) -> Result<Option<VoxelCut3>, String> {
        self.script.voxel_cut(t, parent, revision, i)
    }
    fn load(&mut self, _: u64, _: f64, _: usize, state: &BodyState) -> Result<Option<Load3>, String> {
        let c = state.centre;
        Ok(Some(Load3 {
            force: [3.0 * c[1], -2.0 * c[0], 5.0 * c[2]],
            torque: [7.0 * c[0], 11.0 * c[2], -13.0 * c[1]],
        }))
    }
}

#[test]
fn a_load_that_reads_the_state_of_a_body_finds_the_same_world_in_the_step_of_a_cut_however_the_step_is_reached() {
    // the cut is at 1.5 s; a load is asked at the start of each step, from the body as it is then
    let steps: Vec<u64> = (147..=153).collect();
    the_same_four_ways(
        &|frames| world_logging(2, None, frames),
        &|| Leaning { script: Script { cuts: vec![first_cut(1.5)] } },
        &steps,
    );
}

#[test]
fn the_part_that_stays_after_a_cut_has_all_its_cells_when_they_end_at_the_edge_of_a_chunk_of_the_shape() {
    // cells of 0.1 m, 8 along x from key 680 to 687: Parry keeps its cells in chunks of 8 and its iterator over them is a half-open range of
    // floor(p / size), which for 688 * 0.1 / 0.1 comes out as 687.999...: the last column is the one it can lose
    let size = [0.1; 3];
    let cell_mass = 2400.0 * 0.1 * 0.1 * 0.1;
    let whole = cells_of(680..688, 0..2, 0..2);
    let (stay, piece, destroyed) =
        (cells_of(684..688, 0..2, 0..2), cells_of(680..682, 0..2, 0..2), cells_of(682..684, 0..2, 0..2));
    let mut parent =
        body(Shape3::Voxels { size, cells: whole.clone() }, whole.len() as f64 * cell_mass, BodyKind::Dynamic);
    parent.start = Pose3::default();
    let slot = body(Shape3::Voxels { size, cells: vec![[0, 0, 0]] }, cell_mass, BodyKind::Dynamic);
    let mut w = World3::new(World3Spec {
        fix_internal_edges: false,
        start: 0.,
        step: 0.01,
        gravity: [0.; 3],
        pixels_per_meter: 1.,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies: vec![parent, slot],
    })
    .with_voxel_splits(vec![VoxelSplit3 { parent: 0, slots: vec![1] }])
    .unwrap();
    let cut = VoxelCut3 {
        revision: 1,
        destroyed,
        parent_mass: stay.len() as f64 * cell_mass,
        pieces: vec![VoxelPiece3 { cells: piece.clone(), mass: piece.len() as f64 * cell_mass }],
    };
    // a torque small for a body of 16 cells of a tenth of a metre, so that it has hardly turned in the time measured
    let (force, torque) = ([0.0, 0.0, 0.0], [0.30, 0.55, -0.45]);
    let mut d = Pusher { script: Script { cuts: vec![(0, 0.3, None, cut)] }, from: 0.4, force, torque, closed: vec![] };
    let (a, b) = (w.frame_at(0.41, &mut d), w.frame_at(0.45, &mut d));
    assert!(a.errors.is_empty() && b.errors.is_empty(), "{:?}", a.errors);
    let props = shape_mass_properties(&Shape3::Voxels { size, cells: stay }, 16.0 * cell_mass, 1.0).unwrap();
    let inverse = inverse3(props.inertia);
    for i in 0..3 {
        let alpha = (b.velocities[0].angular[i] - a.velocities[0].angular[i]).to_radians() / 0.04;
        let wanted: f64 = (0..3).map(|j| inverse[i][j] * torque[j]).sum();
        assert!(
            (alpha - wanted).abs() < 2e-3 * wanted.abs().max(1e-3),
            "axis {i}: spin-up {alpha} against {wanted}: the part that stays has lost a cell"
        );
    }
}

/// A driver that cuts as a [`Cutter`] does and remembers, for each body, the state it was asked to load at each step.
struct History {
    inner: Cutter,
    seen: Vec<Vec<(u64, BodyState)>>,
}

impl Driver3 for History {
    fn kinematic(&mut self, t: f64, which: &[usize]) -> Vec<Pose3> {
        self.inner.kinematic(t, which)
    }
    fn fields(&mut self, t: f64) -> Vec<Field> {
        self.inner.fields(t)
    }
    fn voxel_cut(
        &mut self,
        t: f64,
        parent: usize,
        revision: Option<u64>,
        i: Option<&Impact3>,
    ) -> Result<Option<VoxelCut3>, String> {
        self.inner.voxel_cut(t, parent, revision, i)
    }
    fn load(&mut self, step: u64, _: f64, body: usize, state: &BodyState) -> Result<Option<Load3>, String> {
        self.seen[body].push((step, *state));
        Ok(None)
    }
}

#[test]
fn a_piece_is_a_body_with_its_own_mass_and_centre_at_the_first_step_it_is_asked_about() {
    // a slot that has been stepped as a disabled body has its collider disabled by its parent until the pipeline runs again: the mass properties that
    // the cut gives it have to be those that the very next question about the body finds (its centre of mass), not the ones of an empty body
    let mut w = world(2, None);
    let mut d = History { inner: Cutter::new(0.2), seen: vec![vec![]; 3] };
    let frame = w.frame_at(0.3, &mut d);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    let first =
        d.seen[1].iter().find(|(step, state)| *step >= 20 && state.enabled).expect("the piece was asked about").1;
    let wanted = shape_mass_properties(
        &Shape3::Voxels { size: SIZE, cells: cells_of(6..12, 0..2, 0..2) },
        24.0 * CELL_MASS,
        1.0,
    )
    .unwrap()
    .centre;
    let local = local_centre(&first.pose, first.centre);
    for k in 0..3 {
        assert!(
            (local[k] - wanted[k]).abs() < 1e-9,
            "axis {k}: the centre of mass of the piece at its first step is {} and its cells' is {}",
            local[k],
            wanted[k]
        );
    }
}
