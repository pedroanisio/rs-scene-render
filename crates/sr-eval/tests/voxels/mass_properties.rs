//! What the rigid world makes of a body of cells against the exact mass properties of the cells (`sr_3d::occupancy`).
#![allow(clippy::needless_range_loop)]

use sr_3d::occupancy::Moments;
use sr_sim::physics3d::{shape_mass_properties, Shape3};

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
