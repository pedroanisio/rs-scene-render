//! A body of cells fractured into pieces that are bodies of cells from the start (the fracture of the world, with voxel fragments): the
//! pieces move as their cells moved, and the momentum and the angular momentum of the whole are what they were.
#![allow(clippy::needless_range_loop)]

use sr_sim::{
    fields::Field,
    physics3d::{shape_mass_properties, *},
};

const SIZE: [f64; 3] = [0.25; 3];
const CELL_MASS: f64 = 2400.0 * 0.25 * 0.25 * 0.25;

struct Still;
impl Driver3 for Still {
    fn kinematic(&mut self, _: f64, which: &[usize]) -> Vec<Pose3> {
        vec![Pose3::default(); which.len()]
    }
    fn fields(&mut self, _: f64) -> Vec<Field> {
        vec![]
    }
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

fn body(cells: Vec<[i32; 3]>) -> Body3Spec {
    let mass = cells.len() as f64 * CELL_MASS;
    Body3Spec {
        kind: BodyKind::Dynamic,
        shape: Shape3::Voxels { size: SIZE, cells },
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
        start: Pose3 { pos: [0.3, -1.0, 0.2], rot: [0., 0., 0., 1.] },
    }
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn times(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| (0..3).map(|j| m[i][j] * v[j]).sum())
}

#[test]
fn a_body_of_cells_fractured_into_bodies_of_cells_keeps_its_momentum_and_its_angular_momentum_to_the_last_digits() {
    // a block of 8 by 4 by 4 cells in three pieces of unequal size and shape: an end of 3 columns, a middle in an L (one half height) and the rest
    let whole = cells_of(0..8, 0..4, 0..4);
    let end = cells_of(0..3, 0..4, 0..4);
    let step = cells_of(3..6, 0..2, 0..4);
    let rest: Vec<[i32; 3]> = whole.iter().copied().filter(|c| !end.contains(c) && !step.contains(c)).collect();
    let pieces = [end, step, rest];
    assert_eq!(pieces.iter().map(Vec::len).sum::<usize>(), whole.len());
    let mut source = body(whole.clone());
    source.velocity = [1.0, 0.4, -0.3];
    source.angular_velocity = [20.0, 10.0, 40.0];
    let mut bodies = vec![source];
    bodies.extend(pieces.iter().map(|p| body(p.clone())));
    let fragments = (1..=3).map(|k| Fragment3 { body: k, offset: [0.0; 3], impulse: [0.0; 3] }).collect();
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
    .with_fractures(vec![Fracture3 { source: 0, at: 0.0, radial_impulse: 0.0, fragments, contact: None }])
    .unwrap();
    // at the first instant the body is not rotated, so the tensors of its parts are in the axes of the world
    let frame = w.frame_at(0.0, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    assert_eq!(frame.enabled, vec![false, true, true, true], "the source gives way to its pieces");
    let props = |cells: &[[i32; 3]]| {
        shape_mass_properties(
            &Shape3::Voxels { size: SIZE, cells: cells.to_vec() },
            cells.len() as f64 * CELL_MASS,
            1.0,
        )
        .unwrap()
    };
    let total = props(&whole);
    let at = |p: &Pose3, c: [f64; 3]| [p.pos[0] + c[0], p.pos[1] + c[1], p.pos[2] + c[2]];
    let c0 = at(&frame.bodies[0], total.centre);
    let (v0, w0) = ([1.0, 0.4, -0.3], [20.0f64, 10.0, 40.0].map(f64::to_radians));
    // linear momentum: the pieces' sum is the whole's
    let mut p = [0.0; 3];
    let mut l = [0.0; 3];
    for (k, cells) in pieces.iter().enumerate() {
        let q = props(cells);
        let (v, spin) = (frame.velocities[k + 1].linear, frame.velocities[k + 1].angular.map(f64::to_radians));
        for i in 0..3 {
            p[i] += q.mass * v[i];
        }
        let d = [0, 1, 2].map(|i| at(&frame.bodies[k + 1], q.centre)[i] - c0[i]);
        // its own spin and its orbit about the centre of the whole
        let own = times(q.inertia, spin);
        let orbit = cross(d, [0, 1, 2].map(|i| q.mass * v[i]));
        for i in 0..3 {
            l[i] += own[i] + orbit[i];
        }
        // every piece has the spin of the body, and the velocity of the point of the body its centre of mass was
        let wanted = [0, 1, 2].map(|i| v0[i] + cross(w0, d)[i]);
        for i in 0..3 {
            assert!((spin[i] - w0[i]).abs() < 1e-12, "spin of piece {k}");
            assert!((v[i] - wanted[i]).abs() < 1e-12, "piece {k}, axis {i}: {} against {}", v[i], wanted[i]);
        }
    }
    let wanted_p = [0, 1, 2].map(|i| total.mass * v0[i]);
    let wanted_l = times(total.inertia, w0);
    for i in 0..3 {
        assert!((p[i] - wanted_p[i]).abs() < 1e-12 * total.mass, "momentum {i}: {} against {}", p[i], wanted_p[i]);
        assert!(
            (l[i] - wanted_l[i]).abs() < 1e-12 * wanted_l.iter().map(|c| c.abs()).fold(1.0, f64::max),
            "angular momentum {i}: {} against {}",
            l[i],
            wanted_l[i]
        );
    }
}
