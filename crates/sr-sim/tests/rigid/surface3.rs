//! The normal of a body's surface where something hit it: the surface's own, not that of
//! whichever triangle's edge a contact happened to be reported against.

use sr_sim::physics3d::Shape3;
use sr_sim::surface::normal;

/// A grid of `n` x `n` cells over [-half, half]^2 in the plane z = 0, every second diagonal flipped
/// and the inner vertices moved about inside the plane, so that no two tessellations agree.
fn plane(n: usize, half: f64, wobble: f64) -> Shape3 {
    let mut points = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            let edge = i == 0 || j == 0 || i == n || j == n;
            let shift =
                if edge { 0.0 } else { wobble * (((i * 7 + j * 13) % 11) as f64 / 11.0 - 0.5) * 2.0 * half / n as f64 };
            points.push([
                -half + 2.0 * half * i as f64 / n as f64 + shift,
                -half + 2.0 * half * j as f64 / n as f64 - shift,
                0.0,
            ]);
        }
    }
    let at = |i: usize, j: usize| (j * (n + 1) + i) as u32;
    let mut triangles = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
            if (i + j) % 2 == 0 {
                triangles.extend([[a, b, c], [a, c, d]]);
            } else {
                triangles.extend([[a, b, d], [b, c, d]]);
            }
        }
    }
    Shape3::TriMesh(points, triangles)
}

fn along(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.iter().zip(&b).map(|(x, y)| x * y).sum()
}

#[test]
fn a_flat_surface_has_the_same_normal_whatever_its_triangles() {
    for (n, wobble) in [(2, 0.0), (7, 0.0), (16, 0.3), (160, 0.4)] {
        let shape = plane(n, 50.0, wobble);
        // inside triangles, on edges and at corners of the grid
        for point in [
            [0.3, 0.2, 0.1],
            [-12.5, 7.1, 0.0],
            [49.9, -49.9, 0.0],
            [50.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [20.0, 20.0, 3.0],
        ] {
            let found = normal(&shape, point).expect("a mesh has a normal");
            assert!((along(found, found) - 1.0).abs() < 1e-12, "unit: {found:?}");
            assert!(found[2].abs() > 1.0 - 1e-12, "{n} cells, at {point:?}: {found:?}");
        }
    }
}

/// A sphere of `radius` as `rings` x `sectors` quads and the winding of its outward normals.
fn globe(radius: f64, rings: usize, sectors: usize) -> Shape3 {
    let mut points = vec![[0.0, 0.0, radius]];
    for r in 1..rings {
        let polar = std::f64::consts::PI * r as f64 / rings as f64;
        for s in 0..sectors {
            let azimuth = 2.0 * std::f64::consts::PI * s as f64 / sectors as f64;
            points.push([
                radius * polar.sin() * azimuth.cos(),
                radius * polar.sin() * azimuth.sin(),
                radius * polar.cos(),
            ]);
        }
    }
    points.push([0.0, 0.0, -radius]);
    let ring = |r: usize, s: usize| (1 + (r - 1) * sectors + s % sectors) as u32;
    let mut triangles = Vec::new();
    for s in 0..sectors {
        triangles.push([0, ring(1, s), ring(1, s + 1)]);
        for r in 1..rings - 1 {
            triangles.push([ring(r, s), ring(r + 1, s), ring(r + 1, s + 1)]);
            triangles.push([ring(r, s), ring(r + 1, s + 1), ring(r, s + 1)]);
        }
        triangles.push([(points.len() - 1) as u32, ring(rings - 1, s + 1), ring(rings - 1, s)]);
    }
    Shape3::TriMesh(points, triangles)
}

#[test]
fn a_curved_surface_has_the_normal_that_is_local_to_the_point() {
    let radius = 50.0;
    let shape = globe(radius, 48, 96);
    for direction in [[0.2, 0.1, 0.97], [0.8, 0.1, 0.5], [-0.5, 0.7, 0.2], [0.1, -0.9, -0.4], [0.0, 0.0, 1.0]] {
        let length = along(direction, direction).sqrt();
        let unit = direction.map(|c| c / length);
        let point = unit.map(|c| c * radius);
        let found = normal(&shape, point).unwrap();
        let tilt = along(found, unit).abs().min(1.0).acos().to_degrees();
        assert!(tilt < 1.0, "{point:?}: {found:?} is {tilt} degrees from the radius");
    }
}

#[test]
fn simple_shapes_give_their_own_normals_and_others_none() {
    let sphere = Shape3::Sphere(10.0);
    let found = normal(&sphere, [6.0, 8.0, 0.0]).unwrap();
    assert!((found[0] - 0.6).abs() < 1e-12 && (found[1] - 0.8).abs() < 1e-12 && found[2].abs() < 1e-12);
    let cuboid = Shape3::Box([5.0, 2.0, 3.0]);
    assert_eq!(normal(&cuboid, [5.0, 0.5, -1.0]).unwrap(), [1.0, 0.0, 0.0]);
    assert_eq!(normal(&cuboid, [0.1, -2.0, 1.0]).unwrap(), [0.0, -1.0, 0.0]);
    assert_eq!(normal(&cuboid, [1.0, 1.0, 3.0]).unwrap(), [0.0, 0.0, 1.0]);
    assert!(normal(&Shape3::Capsule(1.0, 1.0), [1.0, 0.0, 0.0]).is_none());
    assert!(normal(&Shape3::TriMesh(vec![], vec![]), [0.0; 3]).is_none());
}

mod nearest_point {
    use super::plane;
    use sr_sim::physics3d::Shape3;
    use sr_sim::surface::nearest;

    #[test]
    fn a_point_a_little_inside_a_flat_mesh_is_brought_to_its_surface() {
        let flat = plane(6, 5.0, 0.0);
        let q = nearest(&flat, [1.3, -0.4, 0.27]).unwrap();
        assert!((q[0] - 1.3).abs() < 1e-12 && (q[1] + 0.4).abs() < 1e-12 && q[2].abs() < 1e-12, "{q:?}");
        // beyond the edge, the nearest point is on the edge
        let q = nearest(&flat, [9.0, 0.0, 1.0]).unwrap();
        assert!((q[0] - 5.0).abs() < 1e-12 && q[2].abs() < 1e-12, "{q:?}");
    }

    #[test]
    fn a_sphere_and_a_box_have_their_nearest_point_on_the_surface() {
        let q = nearest(&Shape3::Sphere(2.0), [0.0, 0.0, 1.5]).unwrap();
        assert!((q[2] - 2.0).abs() < 1e-12);
        assert!(nearest(&Shape3::Sphere(2.0), [0.0; 3]).is_none());
        let half = [1.0, 2.0, 3.0];
        // inside: the nearest face
        let q = nearest(&Shape3::Box(half), [0.9, 0.0, 0.0]).unwrap();
        assert_eq!(q, [1.0, 0.0, 0.0]);
        let q = nearest(&Shape3::Box(half), [-0.2, 1.95, 0.5]).unwrap();
        assert_eq!(q, [-0.2, 2.0, 0.5]);
        // outside: clamped
        let q = nearest(&Shape3::Box(half), [4.0, -0.5, 1.0]).unwrap();
        assert_eq!(q, [1.0, -0.5, 1.0]);
        assert!(nearest(&Shape3::Cone(1.0, 1.0), [0.0; 3]).is_none());
    }
}
