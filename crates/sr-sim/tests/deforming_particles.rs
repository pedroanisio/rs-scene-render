use sr_sim::particles3d::collider::Collider;

#[test]
fn moving_triangle_hits_stationary_sphere_between_clear_endpoints() {
    let start = [[-2., -1., -2.], [2., -1., -2.], [0., -1., 2.]];
    let end = start.map(|mut p| {
        p[1] += 2.;
        p
    });
    let collider = Collider::deforming(&start, &end, &[[0, 1, 2]], 0., 1., 1 << 20).unwrap();
    let hit = collider.sweep(0., 1., [0.; 3], [0.; 3], 0.1).unwrap().unwrap();
    assert!((hit.fraction - 0.45).abs() < 1e-8, "{hit:?}");
    assert!((hit.velocity[1] - 2.).abs() < 1e-8, "{hit:?}");
    assert!(hit.normal[1] > 0.999);
}

#[test]
fn deforming_triangle_reports_barycentric_surface_velocity() {
    let start = [[-2., -1., -2.], [2., -1., -2.], [0., -1., 2.]];
    let mut end = start;
    end[2][1] = 3.;
    let collider = Collider::deforming(&start, &end, &[[0, 1, 2]], 0., 1., 1 << 20).unwrap();
    let hit = collider.sweep(0., 1., [0.; 3], [0.; 3], 0.).unwrap().unwrap();
    assert!((hit.fraction - 0.5).abs() < 1e-8, "{hit:?}");
    assert!((hit.velocity[1] - 2.).abs() < 1e-8, "{hit:?}");
}

#[test]
fn deforming_surface_checks_memory_and_time_domain() {
    let p = [[-2., 0., -2.], [2., 0., -2.], [0., 0., 2.]];
    assert!(Collider::deforming(&p, &p, &[[0, 1, 2]], 0., 1., 1).is_err());
    assert!(Collider::deforming(&p, &p, &[[0, 1, 9]], 0., 1., 1 << 20).is_err());
    assert!(Collider::deforming(&p, &p, &[[0, 1, 2]], 0., 0., 1 << 20).is_err());
    let c = Collider::deforming(&p, &p, &[[0, 1, 2]], 1., 1., 1 << 20).unwrap();
    assert!(c.sweep(0., 1., [0., -1., 0.], [0., 1., 0.], 0.1).is_err());
    assert!(c.sweep(1., 1., [0., -0.1, 0.], [0., -1., 0.], 0.1).unwrap().is_none());
}

#[test]
fn polynomial_contacts_match_rigid_sweeps_across_faces_edges_and_vertices() {
    use sr_sim::{particles3d::collider::Geometry, physics3d::Pose3};
    let points = [[-2., 0., -2.], [2., 0., -2.], [0., 0., 2.]];
    let end = points.map(|mut p| {
        p[0] += 0.3;
        p[1] += 0.2;
        p
    });
    let deforming = Collider::deforming(&points, &end, &[[0, 1, 2]], 0., 1., 1 << 20).unwrap();
    let rigid = Geometry::new(&points, &[[0, 1, 2]], 1 << 20)
        .unwrap()
        .moving(Pose3::default(), [0.3, 0.2, 0.], [0.; 3], 0.)
        .unwrap();
    for x in -6..=6 {
        for z in -6..=6 {
            let from = [x as f64 * 0.4, -2., z as f64 * 0.4];
            let to = [from[0] + 0.6, 2., from[2] + 0.2];
            let a = rigid.sweep(0., 1., from, to, 0.17).unwrap();
            let b = deforming.sweep(0., 1., from, to, 0.17).unwrap();
            assert_eq!(a.is_some(), b.is_some(), "x={x} z={z}: {a:?} vs {b:?}");
            if let (Some(a), Some(b)) = (a, b) {
                assert!((a.fraction - b.fraction).abs() < 1e-7, "x={x} z={z}: {a:?} vs {b:?}");
            }
        }
    }
}

#[test]
fn deforming_contacts_preserve_subintervals_translation_and_scale() {
    for scale in [1e-6, 1., 1e6] {
        let offset = [17. * scale, -23. * scale, 31. * scale];
        let map = |p: [f64; 3]| std::array::from_fn(|i| p[i] * scale + offset[i]);
        let start = [[-2., -1., -2.], [2., -1., -2.], [0., -1., 2.]].map(map);
        let end = [[-2., 1., -2.], [2., 1., -2.], [0., 1., 2.]].map(map);
        let collider = Collider::deforming(&start, &end, &[[0, 1, 2]], 5., 2., 1 << 20).unwrap();
        let hit = collider.sweep(5.4, 1., offset, offset, 0.1 * scale).unwrap().unwrap();
        assert!((hit.fraction - 0.5).abs() < 1e-8, "{hit:?}");
        assert!((hit.velocity[1] / scale - 1.).abs() < 1e-8, "{hit:?}");
    }
}

#[test]
fn grazing_vertex_roots_are_found_without_turning_near_misses_into_hits() {
    let p = [[0., 0., 0.], [2., 0., 0.], [0., 0., 2.]];
    let c = Collider::deforming(&p, &p, &[[0, 1, 2]], 0., 1., 1 << 20).unwrap();
    let tangent = c.sweep(0., 1., [-1., -0.1, 0.], [1., -0.1, 0.], 0.1).unwrap().unwrap();
    assert!((tangent.fraction - 0.5).abs() < 1e-8, "{tangent:?}");
    assert!(c.sweep(0., 1., [-1., -0.101, 0.], [1., -0.101, 0.], 0.1).unwrap().is_none());
}

#[test]
fn swept_bvh_returns_the_earliest_triangle_independent_of_input_order() {
    let p = [[-2., 2., -2.], [2., 2., -2.], [0., 2., 2.], [-2., 0., -2.], [2., 0., -2.], [0., 0., 2.]];
    for triangles in [[[0, 1, 2], [3, 4, 5]], [[3, 4, 5], [0, 1, 2]]] {
        let c = Collider::deforming(&p, &p, &triangles, 0., 1., 1 << 20).unwrap();
        let hit = c.sweep(0., 1., [0., -2., 0.], [0., 3., 0.], 0.2).unwrap().unwrap();
        assert!((hit.fraction - 0.36).abs() < 1e-8, "{hit:?}");
    }
}

#[test]
fn fast_small_spheres_hit_a_vertex_before_their_centers_cross_it() {
    let p = [[0., 0., 0.], [2., 0., 0.], [0., 0., 2.]];
    let c = Collider::deforming(&p, &p, &[[0, 1, 2]], 0., 1., 1 << 20).unwrap();
    let hit = c.sweep(0., 1., [-100000., 0., 0.], [100000., 0., 0.], 0.001).unwrap().unwrap();
    assert!((hit.fraction - 0.499999995).abs() < 1e-11, "{hit:?}");
    assert!(hit.normal[0] < -0.999, "{hit:?}");
    assert!((hit.position[0] + 0.001).abs() < 1e-7, "{hit:?}");
    assert!(
        c.sweep(0., 1., [-100000., 0., 0.], [100000., 0., 0.], 1e-12).is_err(),
        "unresolvable radius must report precision failure"
    );
}
