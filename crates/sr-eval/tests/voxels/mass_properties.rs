//! What the rigid world makes of a body of cells against the exact mass properties of the cells (`sr_3d::occupancy`).
#![allow(clippy::needless_range_loop)]

use sr_3d::occupancy::Moments;
use sr_sim::physics3d::{shape_mass_properties, Shape3};
use std::collections::BTreeSet;

/// An asymmetric union of cells: two blocks and a lone arm, with keys on both sides of the origin.
fn cells() -> Vec<[i32; 3]> {
    let mut cells = Vec::new();
    for z in -2..2 {
        for y in -3..1 {
            for x in -4..3 {
                cells.push([x, y, z]);
            }
        }
    }
    for z in 0..3 {
        for y in 1..6 {
            cells.push([2, y, z]);
        }
    }
    for x in 3..9 {
        cells.push([x, 0, -2]);
    }
    cells
}

#[test]
fn the_collider_of_a_body_of_cells_has_the_mass_properties_of_the_cells_in_the_scenes_axes() {
    let size = [0.5, 0.25, 0.4];
    let (mass, ppm) = (1234.5, 1.0);
    let exact =
        Moments::of(cells()).properties(size, mass / (cells().len() as f64 * size[0] * size[1] * size[2])).unwrap();
    let rapier = shape_mass_properties(&Shape3::Voxels { size, cells: cells() }, mass, ppm).unwrap();
    println!(
        "VOXEL MASS exact {:.9} rapier {:.9}; centre {:?} against {:?}",
        exact.mass, rapier.mass, exact.centre, rapier.centre
    );
    assert!((rapier.mass - mass).abs() < 1e-12 * mass);
    assert!((exact.mass - mass).abs() < 1e-12 * mass);
    for k in 0..3 {
        assert!(
            (rapier.centre[k] - exact.centre[k]).abs() < 1e-12,
            "centre {k}: {} against {}",
            rapier.centre[k],
            exact.centre[k]
        );
        for l in 0..3 {
            let scale = exact.inertia[k][k].max(exact.inertia[l][l]);
            assert!(
                (rapier.inertia[k][l] - exact.inertia[k][l]).abs() < 1e-12 * scale,
                "inertia [{k}][{l}]: {} against {}",
                rapier.inertia[k][l],
                exact.inertia[k][l]
            );
        }
    }
    // at another scale of the scene: lengths are scene units, a metre being `ppm` of them
    let ppm = 100.0;
    let scaled_size = size.map(|s| s * ppm);
    let rapier = shape_mass_properties(&Shape3::Voxels { size: scaled_size, cells: cells() }, mass, ppm).unwrap();
    let exact = Moments::of(cells())
        .properties(scaled_size, mass / (cells().len() as f64 * size[0] * size[1] * size[2] * ppm.powi(3)))
        .unwrap();
    for k in 0..3 {
        assert!((rapier.centre[k] - exact.centre[k]).abs() < 1e-9 * exact.centre[k].abs().max(1.0));
    }
    // the inertia is in kilograms scene units squared: every entry of the tensor, at this scale too
    for k in 0..3 {
        for l in 0..3 {
            let scale = exact.inertia[k][k].max(exact.inertia[l][l]);
            assert!(
                (rapier.inertia[k][l] - exact.inertia[k][l]).abs() < 1e-12 * scale,
                "at 100 a metre, inertia [{k}][{l}]: {} against {}",
                rapier.inertia[k][l],
                exact.inertia[k][l]
            );
        }
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

/// The mass properties that the world gives a body of these cells against the exact integer ones, in every entry of the tensor.
fn assert_exact(cells: &[[i32; 3]], size: [f64; 3], what: &str) {
    let density = 2400.0;
    let mass = cells.len() as f64 * density * size[0] * size[1] * size[2];
    let exact = Moments::of(cells.iter().copied()).properties(size, density).unwrap();
    let made = shape_mass_properties(&Shape3::Voxels { size, cells: cells.to_vec() }, mass, 1.0).unwrap();
    assert!((made.mass - exact.mass).abs() < 1e-12 * mass, "{what}: mass");
    for k in 0..3 {
        assert!(
            (made.centre[k] - exact.centre[k]).abs() < 1e-12 * exact.centre[k].abs().max(1.0),
            "{what}: centre {k}"
        );
        for l in 0..3 {
            let scale = exact.inertia[0][0].max(exact.inertia[1][1]).max(exact.inertia[2][2]);
            assert!(
                (made.inertia[k][l] - exact.inertia[k][l]).abs() < 1e-12 * scale,
                "{what}: inertia [{k}][{l}]: the world has {} and the cells have {}",
                made.inertia[k][l],
                exact.inertia[k][l]
            );
        }
    }
}

#[test]
fn the_mass_properties_are_exact_for_boxes_of_every_proportion_and_orientation_including_the_ones_with_two_equal_moments(
) {
    // boxes whose largest moment is on each axis, with two equal moments or none, at different places: the cases in which the tensor is diagonal
    // and has a repeated value are the ones in which a decomposition of it by an eigen solver may give the axes in the wrong order
    let dims = [
        [3, 4, 4],
        [4, 3, 4],
        [4, 4, 3],
        [2, 5, 5],
        [5, 2, 5],
        [5, 5, 2],
        [2, 3, 5],
        [3, 5, 2],
        [5, 2, 3],
        [6, 6, 1],
        [1, 6, 6],
        [6, 1, 6],
        [4, 4, 4],
        [1, 1, 1],
        [8, 1, 1],
        [1, 8, 1],
        [1, 1, 8],
    ];
    for (n, d) in dims.iter().enumerate() {
        for at in [[0, 0, 0], [-7, 3, -2], [100, -50, 25]] {
            let cells = cells_of(at[0]..at[0] + d[0], at[1]..at[1] + d[1], at[2]..at[2] + d[2]);
            assert_exact(&cells, [0.25; 3], &format!("box {d:?} at {at:?} ({n}), cubic cells"));
            assert_exact(&cells, [0.25, 0.5, 0.125], &format!("box {d:?} at {at:?} ({n}), cells of three sizes"));
        }
    }
}

#[test]
fn the_mass_properties_are_exact_for_random_sets_of_cells_and_for_symmetric_ones() {
    let mut state = 7u64;
    let mut next = move || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (state >> 33) as i32
    };
    for round in 0..40 {
        let cells: Vec<[i32; 3]> = (0..60 + round)
            .map(|_| [next() % 9 - 4, next() % 7 - 3, next() % 5 - 2])
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        assert_exact(&cells, [0.25; 3], &format!("random set {round}"));
        assert_exact(&cells, [0.25, 0.5, 0.125], &format!("random set {round}, cells of three sizes"));
    }
    // symmetric bodies, with the tensor diagonal: a cross, a plate with a hole, a hollow box
    let mut cross = cells_of(-3..4, 0..1, 0..1);
    cross.extend(cells_of(0..1, -3..4, 0..1));
    cross.sort();
    cross.dedup();
    assert_exact(&cross, [0.25; 3], "a cross");
    let plate: Vec<[i32; 3]> = cells_of(0..1, 0..7, 0..7).into_iter().filter(|c| c[1] != 3 || c[2] != 3).collect();
    assert_exact(&plate, [0.25; 3], "a plate with a hole, thin along x");
    let hollow: Vec<[i32; 3]> = cells_of(0..3, 0..6, 0..6)
        .into_iter()
        .filter(|c| !(c[0] == 1 && (1..5).contains(&c[1]) && (1..5).contains(&c[2])))
        .collect();
    assert_exact(&hollow, [0.25; 3], "a hollow box");
}
