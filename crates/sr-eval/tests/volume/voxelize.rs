//! A closed mesh cut into cells: the cells whose centres are inside it, on a lattice of multiples of the cell size, found with the
//! test that the colliders use. The oracles are counts and volumes worked out here with no code of the voxelizer: the cells of a box
//! by arithmetic, the cells of a sphere by a ray cast of this file's own, and the volume of the mesh by the divergence theorem.

use sr_3d::occupancy::Limits;
use sr_eval::voxel::{from_triangles, Bounds};

type Mesh = (Vec<[f64; 3]>, Vec<[u32; 3]>);

/// A box with its corner at `min` and sides `size`, twelve triangles, wound outward.
fn cuboid(min: [f64; 3], size: [f64; 3]) -> Mesh {
    let p = |x: usize, y: usize, z: usize| {
        [min[0] + size[0] * x as f64, min[1] + size[1] * y as f64, min[2] + size[2] * z as f64]
    };
    let vertices: Vec<[f64; 3]> = (0..8).map(|i| p(i & 1, i >> 1 & 1, i >> 2 & 1)).collect();
    let quads = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    let mut triangles = Vec::new();
    for q in quads {
        triangles.push([q[0], q[1], q[2]]);
        triangles.push([q[0], q[2], q[3]]);
    }
    (vertices, triangles)
}

/// A sphere of `rings` rings and `segments` segments about `centre`, as a closed polyhedron.
fn sphere(centre: [f64; 3], radius: f64, rings: usize, segments: usize) -> Mesh {
    let mut vertices = vec![[centre[0], centre[1] - radius, centre[2]]];
    for r in 1..rings {
        let theta = std::f64::consts::PI * r as f64 / rings as f64;
        for s in 0..segments {
            let phi = std::f64::consts::TAU * s as f64 / segments as f64;
            vertices.push([
                centre[0] + radius * theta.sin() * phi.cos(),
                centre[1] - radius * theta.cos(),
                centre[2] + radius * theta.sin() * phi.sin(),
            ]);
        }
    }
    vertices.push([centre[0], centre[1] + radius, centre[2]]);
    let ring = |r: usize, s: usize| (1 + (r - 1) * segments + s % segments) as u32;
    let (top, bottom) = (0u32, (vertices.len() - 1) as u32);
    let mut triangles = Vec::new();
    for s in 0..segments {
        triangles.push([top, ring(1, s + 1), ring(1, s)]);
        triangles.push([bottom, ring(rings - 1, s), ring(rings - 1, s + 1)]);
    }
    for r in 1..rings - 1 {
        for s in 0..segments {
            triangles.push([ring(r, s), ring(r, s + 1), ring(r + 1, s + 1)]);
            triangles.push([ring(r, s), ring(r + 1, s + 1), ring(r + 1, s)]);
        }
    }
    (vertices, triangles)
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Whether `p` is inside the closed mesh: the parity of the crossings of a ray in an unlikely direction (Moller-Trumbore).
fn inside(mesh: &Mesh, p: [f64; 3]) -> bool {
    let d = [0.5773502691896258, 0.3090169943749474, 0.754709580222772];
    let mut crossings = 0;
    for t in &mesh.1 {
        let (a, b, c) = (mesh.0[t[0] as usize], mesh.0[t[1] as usize], mesh.0[t[2] as usize]);
        let (e1, e2) = (sub(b, a), sub(c, a));
        let h = cross(d, e2);
        let det = dot(e1, h);
        if det.abs() < 1e-14 {
            continue;
        }
        let f = 1.0 / det;
        let s = sub(p, a);
        let u = f * dot(s, h);
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = cross(s, e1);
        let v = f * dot(d, q);
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        if f * dot(e2, q) > 0.0 {
            crossings += 1;
        }
    }
    crossings % 2 == 1
}

/// The volume and the area of a closed mesh.
fn volume_and_area(mesh: &Mesh) -> (f64, f64) {
    let (mut volume, mut area) = (0.0, 0.0);
    for t in &mesh.1 {
        let (a, b, c) = (mesh.0[t[0] as usize], mesh.0[t[1] as usize], mesh.0[t[2] as usize]);
        volume += dot(a, cross(b, c)) / 6.0;
        let n = cross(sub(b, a), sub(c, a));
        area += 0.5 * dot(n, n).sqrt();
    }
    (volume.abs(), area)
}

fn voxelize(mesh: &Mesh, cell: f64) -> Result<sr_3d::occupancy::Occupancy, String> {
    from_triangles(&mesh.0, &mesh.1, cell, Limits::default(), &Bounds::default())
}

#[test]
fn a_box_gives_exactly_the_cells_whose_centres_are_inside_it() {
    // 4 x 6 x 8 on a lattice of 1: every cell of the box
    let cells = voxelize(&cuboid([0.0; 3], [4.0, 6.0, 8.0]), 1.0).unwrap();
    assert_eq!(cells.count(), 4 * 6 * 8);
    assert_eq!(cells.bounds(), Some(([0, 0, 0], [3, 5, 7])));
    // a lattice of 0.75 that cuts it across: the count is, along each axis, the integers i with 0 < (i + 1/2) 0.75 < side
    let along = |side: f64, offset: f64| {
        (-20..40).filter(|&i| (i as f64 + 0.5) * 0.75 > offset && (i as f64 + 0.5) * 0.75 < offset + side).count()
    };
    let (min, size) = ([0.31, -1.07, 2.5], [4.0, 6.0, 8.0]);
    let cells = voxelize(&cuboid(min, size), 0.75).unwrap();
    assert_eq!(cells.count() as usize, along(size[0], min[0]) * along(size[1], min[1]) * along(size[2], min[2]));
    // every cell is index 1
    assert!(cells.cells().all(|c| cells.get(c) == 1));
}

#[test]
fn a_sphere_gives_the_lattice_points_inside_it_by_a_ray_cast_and_the_volume_to_half_a_surface_layer() {
    let mesh = sphere([0.37, -0.21, 0.52], 5.0, 24, 36);
    let cell = 0.5;
    let cells = voxelize(&mesh, cell).unwrap();
    let mut brute = 0u64;
    for k in -14..14 {
        for j in -14..14 {
            for i in -14..14 {
                let centre = [(i as f64 + 0.5) * cell, (j as f64 + 0.5) * cell, (k as f64 + 0.5) * cell];
                let want = inside(&mesh, centre);
                assert_eq!(cells.get([i, j, k]) != 0, want, "the cell {:?}", [i, j, k]);
                brute += u64::from(want);
            }
        }
    }
    assert_eq!(cells.count(), brute);
    let (volume, area) = volume_and_area(&mesh);
    let from_cells = cells.count() as f64 * cell.powi(3);
    assert!((from_cells - volume).abs() <= 0.5 * area * cell, "{from_cells} against {volume} (area {area})");
}

#[test]
fn every_cell_centre_agrees_with_the_test_of_the_colliders() {
    let mesh = sphere([0.1, 0.2, 0.3], 3.0, 16, 24);
    let region = sr_sim::pyro::mesh::Mesh::new(&mesh.0, &mesh.1, usize::MAX).unwrap();
    let cell = 0.4;
    let cells = voxelize(&mesh, cell).unwrap();
    for k in -10..10 {
        for j in -10..10 {
            for i in -10..10 {
                let centre = [(i as f64 + 0.5) * cell, (j as f64 + 0.5) * cell, (k as f64 + 0.5) * cell];
                assert_eq!(cells.get([i, j, k]) != 0, region.contains(centre), "the cell {:?}", [i, j, k]);
            }
        }
    }
}

#[test]
fn a_mesh_that_is_not_closed_is_the_error_of_the_colliders() {
    let (vertices, mut triangles) = cuboid([0.0; 3], [2.0; 3]);
    triangles.truncate(10);
    let error = from_triangles(&vertices, &triangles, 0.5, Limits::default(), &Bounds::default()).unwrap_err();
    assert!(error.contains("closed") || error.contains("edge"), "{error}");
}

#[test]
fn the_box_of_the_lattice_and_the_limits_are_checked_before_any_cell_is_looked_at() {
    let mesh = cuboid([0.0; 3], [4.0, 6.0, 8.0]);
    let bounds = |max_box_cells: u64| Bounds { max_box_cells, ..Bounds::default() };
    let error = from_triangles(&mesh.0, &mesh.1, 1.0, Limits::default(), &bounds(100)).unwrap_err();
    assert!(error.contains("192") && error.contains("100"), "{error}");
    assert!(from_triangles(&mesh.0, &mesh.1, 1.0, Limits::default(), &bounds(192)).is_ok());
    // more cells than the grid may hold
    let limits = Limits { max_cells: 191, ..Limits::default() };
    assert!(from_triangles(&mesh.0, &mesh.1, 1.0, limits, &Bounds::default()).unwrap_err().contains("cells"));
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(from_triangles(&mesh.0, &mesh.1, bad, Limits::default(), &Bounds::default()).is_err(), "{bad}");
    }
}

#[test]
fn the_cells_are_the_same_on_any_number_of_threads_and_every_time() {
    let mesh = sphere([0.37, -0.21, 0.52], 5.0, 24, 36);
    let run = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        pool.install(|| voxelize(&mesh, 0.5).unwrap())
    };
    let first = run(1);
    for threads in [2, 8] {
        let other = run(threads);
        assert_eq!(first.fingerprint(), other.fingerprint(), "{threads} threads");
        assert_eq!(first.revision(), other.revision());
    }
    assert_eq!(first.fingerprint(), voxelize(&mesh, 0.5).unwrap().fingerprint());
}

#[test]
fn a_mesh_is_cut_up_to_the_last_key_of_an_occupancy_and_no_further() {
    let edge = f64::from(sr_3d::occupancy::KEY_LIMIT);
    // the cells 2^30 - 2 and 2^30 - 1 of x are the last two
    let last = cuboid([edge - 2.0, 0.0, 0.0], [2.0, 1.0, 1.0]);
    let cut = voxelize(&last, 1.0).unwrap();
    assert_eq!(cut.cells().collect::<Vec<_>>(), vec![[(edge as i32) - 2, 0, 0], [(edge as i32) - 1, 0, 0]]);
    // one cell further is far from the origin, and so is the other side
    for min in [[edge - 1.0, 0.0, 0.0], [-edge - 1.0, 0.0, 0.0], [0.0, f64::from(i32::MAX), 0.0]] {
        let error = voxelize(&cuboid(min, [2.0, 1.0, 1.0]), 1.0).unwrap_err();
        assert!(error.contains("far"), "{min:?}: {error}");
    }
}
