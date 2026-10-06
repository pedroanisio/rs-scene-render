//! A sphere that meets a flat ground of triangles is pushed along the edges between them
//! unless the world is asked to fix them: with the option the contact normal is the ground's
//! and a body that slides on it is not kicked sideways; without it everything is as it was.

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

fn body(shape: Shape3, kind: BodyKind, at: [f64; 3], velocity: [f64; 3]) -> Body3Spec {
    Body3Spec {
        kind,
        shape,
        mass: 50.0,
        friction: 0.0,
        restitution: 0.0,
        linear_damping: 0.0,
        angular_damping: 0.0,
        velocity,
        angular_velocity: [0.0; 3],
        group: 0,
        collides_with: None,
        sensor: false,
        fixed_rotation: false,
        bullet: false,
        activate_at: 0.0,
        start: Pose3 { pos: at, ..Pose3::default() },
    }
}

/// A flat ground of `cells` x `cells` quads whose inner corners are moved about in its plane and whose
/// diagonals alternate, so that no two grounds of different `cells` agree.
fn ground(size: f64, cells: usize) -> Shape3 {
    let n = cells + 1;
    let step = size / cells as f64;
    let points = (0..n * n)
        .map(|k| {
            let (i, j) = (k % n, k / n);
            let inner = i > 0 && j > 0 && i < cells && j < cells;
            let shift = if inner { 0.3 * step * (((i * 7 + j * 13) % 11) as f64 / 11.0 - 0.5) } else { 0.0 };
            [i as f64 * step - size / 2.0 + shift, 0.0, j as f64 * step - size / 2.0 - shift]
        })
        .collect();
    let mut triangles = Vec::new();
    for z in 0..cells {
        for x in 0..cells {
            let a = (z * n + x) as u32;
            let (b, c, d) = (a + 1, a + n as u32, a + n as u32 + 1);
            if (x + z) % 2 == 0 {
                triangles.extend([[a, c, b], [b, c, d]]);
            } else {
                triangles.extend([[a, c, d], [a, d, b]]);
            }
        }
    }
    Shape3::TriMesh(points, triangles)
}

/// A sphere of radius 10 that arrives at the ground at y = 100 going `along` px/s along x and 200 down,
/// in a world that fixes the internal edges or not.
fn landing(cells: usize, along: f64, fix_internal_edges: bool) -> World3 {
    World3::new(World3Spec {
        start: 0.0,
        step: 0.005,
        gravity: [0.0, -9.81, 0.0],
        pixels_per_meter: 1.0,
        iterations: 8,
        bounds: Bounds3::None,
        joints: vec![],
        bodies: vec![
            body(Shape3::Sphere(10.0), BodyKind::Dynamic, [-80.0, 0.0, 0.0], [along, 200.0, 0.0]),
            body(ground(400.0, cells), BodyKind::Static, [0.0, 100.0, 0.0], [0.0; 3]),
        ],
        fix_internal_edges,
    })
    .with_contact_log(ContactLogConfig::new(256, 1 << 20))
}

/// The normal of the first contact, and how far the sphere has been kicked across its line of travel (z, px) after 1.5 s.
fn first_normal_and_kick(w: &mut World3) -> ([f64; 3], f64) {
    let frame = w.frame_at(1.5, &mut Still);
    assert!(frame.errors.is_empty(), "{:?}", frame.errors);
    let steps = w.progress().0;
    let first = (0..steps).find(|&s| !w.contacts_at(s).unwrap().is_empty()).expect("a contact");
    let contacts = w.contacts_at(first).unwrap();
    let total: f64 = contacts.iter().map(|c| c.impulse).sum();
    let mut normal = [0.0; 3];
    for c in contacts {
        for (sum, component) in normal.iter_mut().zip(c.normal) {
            *sum += c.impulse * component / total;
        }
    }
    (normal, w.frame_at(1.5, &mut Still).bodies[0].pos[2])
}

#[test]
fn with_the_option_the_normal_is_the_grounds_and_the_body_is_not_kicked_sideways() {
    for cells in [8, 13, 40] {
        let (normal, kick) = first_normal_and_kick(&mut landing(cells, 100.0, true));
        assert!(
            (normal[1] - 1.0).abs() < 1e-9 && normal[0].abs() < 1e-9 && normal[2].abs() < 1e-9,
            "{cells}: {normal:?}"
        );
        assert!(kick.abs() < 1e-6, "{cells} cells: kicked {kick} px sideways");
    }
}

#[test]
fn without_it_the_world_is_as_it_was_and_the_edges_tilt_the_normal() {
    let mut worst: f64 = 0.0;
    for cells in [8, 13, 40] {
        let (normal, _) = first_normal_and_kick(&mut landing(cells, 100.0, false));
        worst = worst.max(normal[0].hypot(normal[2]));
    }
    println!("EDGES tilt of the contact normal without the option, worst of three grounds: {worst}");
    assert!(worst > 1e-3, "the tilt the option is for: {worst}");
}
