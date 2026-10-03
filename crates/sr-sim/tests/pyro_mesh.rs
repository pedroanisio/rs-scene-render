use sr_sim::pyro::{mesh::Mesh, Inputs, Shape, Simulation, Source, Spec};
use std::sync::Arc;

fn cube(radius: f64) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let vertices = [
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
    ]
    .map(|p| p.map(|v| v * radius))
    .to_vec();
    let faces = vec![
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [3, 7, 6],
        [3, 6, 2],
        [0, 4, 7],
        [0, 7, 3],
        [1, 2, 6],
        [1, 6, 5],
    ];
    (vertices, faces)
}

#[test]
fn deforming_mesh_uses_midpoint_region_and_material_boundary_velocity() {
    let (start, faces) = cube(1.0);
    // Stretch x, translate y: endpoint x is 3*x, y is y+2.
    // At the midpoint the material velocity is [x, 2, 0].
    let end: Vec<_> = start.iter().map(|p| [3.0 * p[0], p[1] + 2.0, p[2]]).collect();
    for reverse in [false, true] {
        let triangles: Vec<_> = faces.iter().map(|&[a, b, c]| if reverse { [a, c, b] } else { [a, b, c] }).collect();
        let mesh = Mesh::moving(&start, &end, &triangles, 1.0, 1 << 20).unwrap();
        assert!(mesh.boundary_velocity([1e308; 3]).is_err(), "unresolved distant queries must not substitute a vertex");
        assert!(mesh.contains([1.5, 1.0, 0.0]));
        assert!(!mesh.contains([2.5, 1.0, 0.0]));
        for p in [[2.0, 1.0, 0.0], [-2.0, 1.0, 0.0], [0.5, 2.0, 0.25]] {
            let v = mesh.boundary_velocity(p).unwrap();
            for (actual, expected) in v.into_iter().zip([p[0], 2.0, 0.0]) {
                assert!((actual - expected).abs() < 1e-12, "{p:?}: {v:?}");
            }
        }
    }
    assert!(Mesh::moving(&start, &end[..7], &faces, 1.0, 1 << 20).is_err());
    assert!(Mesh::moving(&start, &end, &faces, 0.0, 1 << 20).is_err());
    assert!(Mesh::moving(&start, &end, &faces, 1.0, 128).is_err());
    let collapsed: Vec<_> = start.iter().map(|p| [-p[0], p[1], p[2]]).collect();
    assert!(Mesh::moving(&start, &collapsed, &faces, 1.0, 1 << 20).is_err());
}

#[test]
fn deforming_obstacle_evaluates_velocity_at_mac_faces() {
    let (mid, faces) = cube(2.0);
    let start: Vec<_> = mid.iter().map(|p| [0.5 * p[0], p[1], p[2]]).collect();
    let end: Vec<_> = mid.iter().map(|p| [1.5 * p[0], p[1], p[2]]).collect();
    let mesh = Arc::new(Mesh::moving(&start, &end, &faces, 1.0, 1 << 20).unwrap());
    let mut sim = Simulation::new(Spec {
        cells: [8; 3],
        origin: [-4.0; 3],
        dt: 0.1,
        boundary: sr_sim::pyro::Boundary::Open,
        pressure_iterations: 500,
        ..Spec::default()
    })
    .unwrap();
    sim.step(&Inputs { obstacles: vec![sr_sim::pyro::Obstacle::stationary(Shape::Mesh(mesh))], ..Inputs::default() })
        .unwrap();
    // These are x faces between solid cells, near the top of the box.
    // Their nearest surface retains x, so v_x=x, not cell-centre x.
    for x in [-1.0, 0.0, 1.0] {
        let velocity = sim.state().velocity_at([x, 1.5, 0.5]);
        assert!((velocity[0] - x).abs() < 1e-12, "x={x} v={velocity:?}");
    }
}

#[test]
fn closed_mesh_contains_interior_surface_and_preserves_hollow_regions() {
    let (mut vertices, mut faces) = cube(2.0);
    let (inner, inner_faces) = cube(1.0);
    vertices.extend(inner);
    faces.extend(inner_faces.into_iter().map(|[a, b, c]| [a + 8, c + 8, b + 8]));
    for reverse in [false, true] {
        let faces: Vec<_> = faces.iter().map(|&[a, b, c]| if reverse { [a, c, b] } else { [a, b, c] }).collect();
        let mesh = Mesh::new(&vertices, &faces, 1 << 20).unwrap();
        assert!(!mesh.contains([0.0; 3]), "the inner component is a cavity");
        assert!(mesh.contains([1.5, 0.0, 0.0]));
        assert!(mesh.contains([2.0, 0.25, 0.25]));
        assert!(!mesh.contains([2.1, 0.0, 0.0]));
        assert!(!mesh.contains([f64::NAN, 0.0, 0.0]));
        assert!(mesh.bytes() <= 1 << 20);
    }
}

#[test]
fn mesh_validation_rejects_open_nonfinite_degenerate_and_oversized_inputs() {
    let (vertices, faces) = cube(1.0);
    assert!(Mesh::new(&vertices, &faces, 128).is_err());
    assert!(Mesh::new(&vertices, &faces[..11], 1 << 20).is_err());
    let mut bad = faces.clone();
    bad[0][0] = 99;
    assert!(Mesh::new(&vertices, &bad, 1 << 20).is_err());
    let mut bad = faces.clone();
    bad[0].swap(1, 2);
    assert!(Mesh::new(&vertices, &bad, 1 << 20).is_err());
    let mut bad = faces.clone();
    bad.push(faces[0]);
    assert!(Mesh::new(&vertices, &bad, 1 << 20).is_err());
    let mut bad = vertices.clone();
    bad[0][0] = f64::INFINITY;
    assert!(Mesh::new(&bad, &faces, 1 << 20).is_err());
    let flat: Vec<_> = vertices.iter().map(|p| [p[0], p[1], 0.0]).collect();
    assert!(Mesh::new(&flat, &faces, 1 << 20).is_err());
}

#[test]
fn mesh_emission_matches_the_same_analytic_box_in_all_three_axes() {
    let (vertices, faces) = cube(2.0);
    let vertices: Vec<_> = vertices.into_iter().map(|p| p.map(|v| v + 4.0)).collect();
    let mesh = Arc::new(Mesh::new(&vertices, &faces, 1 << 20).unwrap());
    let spec = Spec { cells: [8; 3], dt: 0.1, ..Spec::default() };
    let mut a = Simulation::new(spec.clone()).unwrap();
    let mut b = Simulation::new(spec).unwrap();
    for (sim, shape) in [(&mut a, Shape::Mesh(mesh)), (&mut b, Shape::Box { min: [2.0; 3], max: [6.0; 3] })] {
        sim.step(&Inputs {
            sources: vec![Source { shape, density_rate: 10.0, temperature_rate: 1000.0, ..Source::default() }],
            ..Inputs::default()
        })
        .unwrap();
    }
    assert_eq!(a.state().density(), b.state().density());
    assert_eq!(a.state().temperature(), b.state().temperature());
    assert_eq!(a.state().density().iter().filter(|&&v| v > 0.0).count(), 64);
}

#[test]
fn coincident_vertices_split_for_uvs_or_normals_do_not_open_the_volume() {
    let (vertices, faces) = cube(1.0);
    let mut split = Vec::new();
    let mut triangles = Vec::new();
    for face in faces {
        let start = split.len() as u32;
        split.extend(face.map(|i| vertices[i as usize]));
        triangles.push([start, start + 1, start + 2]);
    }
    let mesh = Mesh::new(&split, &triangles, 1 << 20).unwrap();
    assert!(mesh.contains([0.0; 3]));
    assert!(!mesh.contains([2.0, 0.0, 0.0]));
}

#[test]
fn unused_vertices_do_not_change_mesh_bounds_or_numerical_scale() {
    let (mut vertices, faces) = cube(1.0);
    vertices.push([1e20; 3]);
    let mesh = Mesh::new(&vertices, &faces, 1 << 20).unwrap();
    assert!(mesh.contains([0.0; 3]));
    assert!(!mesh.contains([2.0; 3]));
}
