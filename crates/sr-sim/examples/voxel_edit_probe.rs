//! What it costs to edit the cells of a Rapier voxel collider, alone and while a checkpoint still holds the shape.
//!
//! `cargo run --profile ci -p sr-sim --example voxel_edit_probe`
//!
//! For floors of 1e4, 1e5 and 1e6 cells: the time to remove 1, 1 000 and 100 000 cells from a collider nobody else holds, and from one
//! that a copy (a checkpoint) holds, where the first edit has to copy the whole shape; and the resident memory a copy of the shape adds.

use rapier3d_f64::prelude::*;
use std::time::Instant;

fn resident_kib() -> u64 {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    statm.split_whitespace().nth(1).and_then(|p| p.parse::<u64>().ok()).unwrap_or(0) * 4
}

fn floor(side: i64) -> Vec<IVector> {
    let mut keys = Vec::with_capacity((side * side * 4) as usize);
    for z in 0..side {
        for y in 0..4 {
            for x in 0..side {
                keys.push(IVector::new(x, y, z));
            }
        }
    }
    keys
}

fn main() {
    for side in [50i64, 158, 500] {
        let keys = floor(side);
        let cells = keys.len();
        let collider = ColliderBuilder::voxels(Vector::new(0.1, 0.1, 0.1), &keys).build();
        for removed in [1usize, 1_000, 100_000] {
            if removed > cells / 2 {
                continue;
            }
            let victims = &keys[..removed];
            // unshared
            let mut alone = collider.clone();
            let started = Instant::now();
            {
                let voxels = alone.shape_mut().as_voxels_mut().expect("voxels");
                for k in victims {
                    voxels.set_voxel(*k, false);
                }
            }
            let unshared_after_copy = started.elapsed();
            // shared with a "checkpoint": the collider's shape is held by `snapshot` as well
            let snapshot = collider.clone();
            let mut shared = collider.clone();
            let started = Instant::now();
            {
                let voxels = shared.shape_mut().as_voxels_mut().expect("voxels");
                for k in victims {
                    voxels.set_voxel(*k, false);
                }
            }
            let with_copy = started.elapsed();
            // again on the same collider, now that it holds its own copy: the edit alone
            let started = Instant::now();
            {
                let voxels = shared.shape_mut().as_voxels_mut().expect("voxels");
                for k in &keys[removed..removed * 2] {
                    voxels.set_voxel(*k, false);
                }
            }
            let edit_alone = started.elapsed();
            drop(snapshot);
            println!(
                "{cells:>8} cells, remove {removed:>6}: edit with a copy held (copy + edit) {:>9.3} ms; the next {removed} edits alone {:>9.3} ms; (first edit of a fresh clone, as above, {:>9.3} ms)",
                with_copy.as_secs_f64() * 1e3,
                edit_alone.as_secs_f64() * 1e3,
                unshared_after_copy.as_secs_f64() * 1e3
            );
        }
        let before = resident_kib();
        let copy = {
            let mut c = collider.clone();
            c.shape_mut();
            c
        };
        let after = resident_kib();
        println!(
            "{cells:>8} cells: a private copy of the shape adds about {} KiB of resident memory",
            after.saturating_sub(before)
        );
        drop(copy);
    }
}
