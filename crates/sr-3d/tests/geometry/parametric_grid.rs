//! SREP 70 §3 and §4: a mesh sampled on a grid. Its triangles per cell, its normals (D_v × D_u, with the fallbacks),
//! the winding that makes a triangle face where its normals point, and the triangles dropped at non-finite vertices.

use glam::Vec3;
use sr_3d::prim::parametric_grid;

/// The grid (x, y, z)(u, v) at nu × nv samples over [u0, u1] × [v0, v1], open in both directions.
fn sample(
    nu: usize,
    nv: usize,
    [u0, u1]: [f64; 2],
    [v0, v1]: [f64; 2],
    f: impl Fn(f64, f64) -> [f64; 3],
) -> Vec<[f64; 3]> {
    let mut p = Vec::new();
    for j in 0..nv {
        for i in 0..nu {
            let u = u0 + i as f64 * (u1 - u0) / (nu - 1) as f64;
            let v = v0 + j as f64 * (v1 - v0) / (nv - 1) as f64;
            p.push(f(u, v));
        }
    }
    p
}

fn uvs(n: usize) -> Vec<[f32; 2]> {
    vec![[0.0, 0.0]; n]
}

/// The geometric normal (b − a) × (c − a) of every triangle, which the renderer treats as its front.
fn faces(m: &sr_3d::Primitive) -> Vec<Vec3> {
    m.indices
        .chunks_exact(3)
        .map(|t| {
            let p = |k: u32| Vec3::from(m.vertices[k as usize].pos);
            (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0])).normalize()
        })
        .collect()
}

#[test]
fn the_identity_plane_faces_the_camera() {
    // x = u, y = v, z = 0: the normal is −z, toward the implicit camera, like the plane primitive
    let p = sample(8, 8, [-50.0, 50.0], [-25.0, 25.0], |u, v| [u, v, 0.0]);
    let m = parametric_grid(&p, &uvs(64), 8, 8, [false, false]);
    assert_eq!(m.vertices.len(), 64);
    assert_eq!(m.indices.len(), 7 * 7 * 2 * 3, "two triangles per cell, no cell past an open border");
    for v in &m.vertices {
        assert!((Vec3::from(v.normal) - Vec3::NEG_Z).length() < 1e-6, "normal {:?}", v.normal);
    }
    for f in faces(&m) {
        assert!((f - Vec3::NEG_Z).length() < 1e-6, "a triangle winds away from the camera: {f}");
    }
}

#[test]
fn mirroring_u_turns_the_surface_away() {
    // x = −u flips D_u, so the normal is +z: a single-sided material culls it
    let p = sample(8, 8, [-50.0, 50.0], [-25.0, 25.0], |u, v| [-u, v, 0.0]);
    let m = parametric_grid(&p, &uvs(64), 8, 8, [false, false]);
    for v in &m.vertices {
        assert!((Vec3::from(v.normal) - Vec3::Z).length() < 1e-6, "normal {:?}", v.normal);
    }
    for f in faces(&m) {
        assert!((f - Vec3::Z).length() < 1e-6, "{f}");
    }
}

#[test]
fn a_flat_heightfield_faces_up() {
    // the heightfield of SREP 70 §4: u = z, v = x, vertex (x, −height, z); flat, its normal is (0, −1, 0)
    let p = sample(4, 4, [-25.0, 25.0], [-50.0, 50.0], |u, v| [v, -0.0, u]);
    let m = parametric_grid(&p, &uvs(16), 4, 4, [false, false]);
    for v in &m.vertices {
        assert!((Vec3::from(v.normal) - Vec3::NEG_Y).length() < 1e-6, "normal {:?}", v.normal);
    }
    for f in faces(&m) {
        assert!((f - Vec3::NEG_Y).length() < 1e-6, "{f}");
    }
}

#[test]
fn a_closed_direction_joins_its_last_samples_to_the_first() {
    // a cylinder around y: u closed (N steps over 2π), v open
    let n = 12;
    let mut p = Vec::new();
    for j in 0..3 {
        for i in 0..n {
            let a = i as f64 * std::f64::consts::TAU / n as f64;
            p.push([10.0 * a.cos(), j as f64 * 5.0, 10.0 * a.sin()]);
        }
    }
    let m = parametric_grid(&p, &uvs(3 * n), n, 3, [true, false]);
    assert_eq!(m.indices.len(), n * 2 * 2 * 3, "n cells around, 2 along");
    // every normal is horizontal and radial (inward or outward, one way for all)
    let radial: Vec<f32> = m
        .vertices
        .iter()
        .map(|v| {
            let r = Vec3::new(v.pos[0], 0.0, v.pos[2]).normalize();
            Vec3::from(v.normal).dot(r)
        })
        .collect();
    assert!(radial.iter().all(|d| (d.abs() - 1.0).abs() < 1e-3), "{radial:?}");
    assert!(radial.iter().all(|d| d.signum() == radial[0].signum()), "{radial:?}");
}

#[test]
fn a_degenerate_row_takes_the_normal_of_its_triangles() {
    // a cone: every vertex of the apex row is the same point, so D_v is zero there
    let n = 8;
    let mut p = Vec::new();
    for j in 0..3 {
        for i in 0..n {
            let a = i as f64 * std::f64::consts::TAU / n as f64;
            let r = 5.0 * j as f64;
            p.push([r * a.cos(), 10.0 * j as f64, r * a.sin()]);
        }
    }
    let m = parametric_grid(&p, &uvs(3 * n), n, 3, [true, false]);
    for v in &m.vertices {
        let nv = Vec3::from(v.normal);
        assert!(nv.is_finite() && (nv.length() - 1.0).abs() < 1e-4, "{:?}", v.normal);
    }
    let apex = m.vertices.iter().find(|v| Vec3::from(v.pos).length() < 1e-6).expect("the apex");
    // the apex triangles all lean the same way around the axis: their sum points along the axis
    let a = Vec3::from(apex.normal);
    assert!(a.y.abs() > 0.99, "apex normal {a}");
}

#[test]
fn triangles_at_non_finite_vertices_are_dropped() {
    let mut p = sample(5, 5, [0.0, 4.0], [0.0, 4.0], |u, v| [u, v, 0.0]);
    p[2 + 2 * 5] = [f64::NAN, 2.0, 0.0];
    let m = parametric_grid(&p, &uvs(25), 5, 5, [false, false]);
    // the centre vertex touches 6 of the 32 triangles of the 4 × 4 cells
    assert_eq!(m.indices.len(), (32 - 6) * 3);
    assert_eq!(m.vertices.len(), 24, "the dropped vertex is not kept");
    assert!(m.vertices.iter().all(|v| v.pos.iter().all(|c| c.is_finite())));
    for f in faces(&m) {
        assert!((f - Vec3::NEG_Z).length() < 1e-5, "{f}");
    }
}
