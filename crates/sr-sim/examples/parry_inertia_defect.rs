//! The mismatch in Parry's diagonalisation of an inertia tensor that `physics3d::voxel_mass` works around.
//!
//! `cargo run --profile ci -p sr-sim --example parry_inertia_defect`
//!
//! The smallest case: the tensor of a block of 3 by 4 by 4 cubic cells (side 0.25 m, 2400 kg/m3, 1800 kg) about its centre is the diagonal
//! matrix (300, 234.375, 234.375). Handed to `MassProperties::with_inertia_matrix` and read back with `reconstruct_inertia_matrix`, it comes
//! back as (234.375, 300, 234.375): two different diagonal tensors (300, 234.375, 234.375) and (234.375, 300, 234.375) give the same principal
//! moments and the same frame, so one of them is wrong. The same tensor with the larger moment on y or on z, or with two equal larger moments, is
//! returned correctly.

use rapier3d_f64::prelude::*;

fn main() {
    for d in [
        [300.0, 234.375, 234.375],
        [234.375, 300.0, 234.375],
        [234.375, 234.375, 300.0],
        [800.0, 2000.0, 2000.0],
        [2000.0, 800.0, 2000.0],
    ] {
        let p = MassProperties::with_inertia_matrix(
            Vector::ZERO,
            1.0,
            Matrix::from_diagonal(Vector::new(d[0], d[1], d[2])),
        );
        let r = p.reconstruct_inertia_matrix();
        let (x, y, z) = (r.col(0)[0], r.col(1)[1], r.col(2)[2]);
        let right = (x - d[0]).abs() < 1e-9 && (y - d[1]).abs() < 1e-9 && (z - d[2]).abs() < 1e-9;
        println!(
            "diag {d:?}: principal {:?}, frame {:?}, read back ({x:.3}, {y:.3}, {z:.3}) {}",
            p.principal_inertia(),
            p.principal_inertia_local_frame,
            if right { "ok" } else { "WRONG" }
        );
    }
}
