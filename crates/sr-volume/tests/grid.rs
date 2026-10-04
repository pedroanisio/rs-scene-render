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

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// A brick of random samples, `background_share` percent of them equal to the background.
fn random_brick(state: &mut u64, background: f32, background_share: u64) -> Box<[f32; 512]> {
    let mut values = Box::new([background; 512]);
    for v in values.iter_mut() {
        if xorshift(state) % 100 >= background_share {
            *v = (xorshift(state) % 2000) as f32 * 0.25 - 100.0;
        }
    }
    values
}

fn same_grid(a: &SparseGrid, b: &SparseGrid, what: &str) {
    assert_eq!(a.brick_count(), b.brick_count(), "{what}: brick count");
    assert_eq!(a.bytes(), b.bytes(), "{what}: bytes");
    for ((ka, va), (kb, vb)) in a.bricks().zip(b.bricks()) {
        assert_eq!(ka, kb, "{what}: key");
        assert!(va.iter().zip(vb).all(|(p, q)| p.to_bits() == q.to_bits()), "{what}: values at {ka:?}");
    }
}

fn set_voxels(grid: &mut SparseGrid, key: [i32; 3], values: &[f32; 512]) {
    for (i, &v) in values.iter().enumerate() {
        let (x, y, z) = (i as i32 % 8, i as i32 / 8 % 8, i as i32 / 64);
        grid.set([key[0] * 8 + x, key[1] * 8 + y, key[2] * 8 + z], v).unwrap();
    }
}

#[test]
fn set_brick_equals_setting_every_voxel() {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for background in [0.0f32, 300.0, -0.0] {
        let (mut bulk, mut single) = (
            SparseGrid::new(Transform::identity(), background, 64).unwrap(),
            SparseGrid::new(Transform::identity(), background, 64).unwrap(),
        );
        let keys = [[0, 0, 0], [-1, 2, -3], [5, 5, 5], [i32::MAX / 8, 0, 0], [0, i32::MIN / 8, 1], [3, 0, 0]];
        // Share of background samples: none, some, nearly all, all.
        for (round, share) in [0u64, 40, 99, 100, 40].into_iter().enumerate() {
            for key in keys {
                let values = random_brick(&mut state, background, share);
                bulk.set_brick(key, &values).unwrap();
                set_voxels(&mut single, key, &values);
                same_grid(&bulk, &single, &format!("background {background} round {round} share {share} {key:?}"));
            }
        }
    }
}

#[test]
fn set_brick_obeys_the_budget_and_is_atomic() {
    let mut state = 7u64;
    let mut g = grid(1);
    let first = random_brick(&mut state, 0.0, 50);
    g.set_brick([0, 0, 0], &first).unwrap();
    // Replacing the existing brick is not a new brick.
    g.set_brick([0, 0, 0], &random_brick(&mut state, 0.0, 50)).unwrap();
    let before = {
        let mut copy = grid(1);
        for (key, values) in g.bricks() {
            copy.set_brick(key, values).unwrap();
        }
        copy
    };
    // A second brick exceeds the budget, an all-background brick does not.
    assert!(matches!(g.set_brick([1, 0, 0], &random_brick(&mut state, 0.0, 50)), Err(Error::Limit(_))));
    g.set_brick([1, 0, 0], &Box::new([0.0; 512])).unwrap();
    same_grid(&g, &before, "after the rejected brick");
    // Nonfinite samples are rejected without changing anything, even deep in the brick.
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut values = random_brick(&mut state, 0.0, 50);
        values[511] = bad;
        assert!(matches!(g.set_brick([0, 0, 0], &values), Err(Error::Invalid(_))));
        same_grid(&g, &before, "after the nonfinite brick");
    }
    // An all-background brick removes the brick it replaces.
    g.set_brick([0, 0, 0], &Box::new([0.0; 512])).unwrap();
    assert_eq!(g.brick_count(), 0);
}
