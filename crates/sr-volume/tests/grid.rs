use glam::{DMat4, DVec3};
use sr_volume::{Error, SparseGrid, Transform};

fn grid(max_bricks: usize) -> SparseGrid {
    SparseGrid::new(Transform::identity(), 0.0, max_bricks).unwrap()
}

#[test]
fn sparse_negative_coordinates_and_pruning_preserve_background() {
    let mut g = grid(4);
    assert_eq!(g.brick_count(), 0);
    for p in [[-1, -1, -1], [0, 0, 0], [7, 7, 7], [8, 0, 0], [-9, 2, 1]] {
        g.set(p, 3.5).unwrap();
        assert_eq!(g.value(p), 3.5);
    }
    assert_eq!(g.brick_count(), 4);
    assert_eq!(g.value([-8, -8, -8]), 0.0);
    g.set([-1, -1, -1], 0.0).unwrap();
    assert_eq!(g.brick_count(), 3, "empty bricks must be reclaimed");
    g.set([i32::MAX, i32::MIN, 0], 2.0).unwrap();
    assert_eq!(g.value([i32::MAX, i32::MIN, 0]), 2.0);
}

#[test]
fn sampling_interpolates_across_bricks_and_affine_world_transform() {
    let matrix = DMat4::from_translation(DVec3::new(20.0, -3.0, 8.0))
        * DMat4::from_rotation_z(0.7)
        * DMat4::from_scale(DVec3::new(2.0, 3.0, 4.0));
    let tr = Transform::new(matrix.to_cols_array()).unwrap();
    let mut g = SparseGrid::new(tr, 0.0, 8).unwrap();
    for z in 7..=8 {
        for y in -1..=0 {
            for x in 7..=8 {
                g.set([x, y, z], (x + 2 * y + 4 * z) as f32).unwrap();
            }
        }
    }
    let p = [7.25, -0.5, 7.75];
    let want = 7.25 - 1.0 + 4.0 * 7.75;
    assert!((f64::from(g.sample_index(p)) - want).abs() < 1e-5);
    let w = matrix.transform_point3(DVec3::from(p)).to_array();
    assert!((f64::from(g.sample_world(w)) - want).abs() < 1e-5);
    assert_eq!(g.sample_index([1e100, 0.0, 0.0]), 0.0);
    assert_eq!(g.sample_index([f64::NAN, 0.0, 0.0]), 0.0);
}

#[test]
fn invalid_values_and_exhausted_budget_do_not_mutate_the_grid() {
    let mut g = grid(1);
    g.set([0, 0, 0], 1.0).unwrap();
    assert!(matches!(g.set([8, 0, 0], 1.0), Err(Error::Limit(_))));
    assert!(g.set([0, 0, 0], f32::NAN).is_err());
    assert!(g.set([0, 0, 0], f32::INFINITY).is_err());
    assert_eq!(g.value([0, 0, 0]), 1.0);
    assert_eq!(g.brick_count(), 1);
    g.set([8, 0, 0], 0.0).unwrap();
    g.set([0, 0, 0], 0.0).unwrap();
    g.set([8, 0, 0], -2.0).unwrap();
    assert_eq!(g.value([8, 0, 0]), -2.0);
}

#[test]
fn transforms_reject_nonfinite_singular_and_projective_matrices() {
    assert!(Transform::new([0.0; 16]).is_err());
    let mut m = DMat4::IDENTITY.to_cols_array();
    m[3] = 0.2;
    assert!(Transform::new(m).is_err());
    m = DMat4::IDENTITY.to_cols_array();
    m[12] = f64::INFINITY;
    assert!(Transform::new(m).is_err());
    assert!(SparseGrid::new(Transform::identity(), f32::NAN, 1).is_err());
}
