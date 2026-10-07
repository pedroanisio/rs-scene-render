//! A closed mesh cut into cells.
//!
//! The lattice is the scene's, aligned to multiples of the cell size in the mesh's own coordinates: the cell `[i, j, k]` is the box
//! from `(i, j, k)` to `(i + 1, j + 1, k + 1)` times the cell size, and it is filled (with the palette index 1) when its centre is
//! inside the mesh by the test that the colliders of the smoke use ([`sr_sim::pyro::mesh::Mesh`]: a closed, validated surface, the
//! nearest oriented surface decides), so a mesh that is not closed is the collider's error. The box of the lattice and the limits are
//! checked before any cell is looked at, and the rows of cells are cut in parallel and put together in the order of the scan, so
//! the result is the same on any number of threads.

use rayon::prelude::*;
use sr_3d::occupancy::{Limits, Occupancy};
use sr_sim::pyro::mesh::Mesh;

/// What the voxelizer accepts besides the limits of the grid.
#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    /// Cells of the box of the lattice that the mesh spans (the work is one test for each).
    pub max_box_cells: u64,
    /// Bytes of the region built from the mesh.
    pub max_mesh_bytes: usize,
}

impl Default for Bounds {
    fn default() -> Self {
        Self { max_box_cells: 1 << 28, max_mesh_bytes: 1 << 30 }
    }
}

/// The cells of the closed mesh `triangles` of `points`, on a lattice of `cell_size`.
pub fn from_triangles(
    points: &[[f64; 3]],
    triangles: &[[u32; 3]],
    cell_size: f64,
    limits: Limits,
    bounds: &Bounds,
) -> Result<Occupancy, String> {
    if !(cell_size.is_finite() && cell_size > 0.0) {
        return Err(format!("the size of a cell, {cell_size}, is not a positive number"));
    }
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for t in triangles {
        for &i in t {
            let p = points.get(i as usize).ok_or("a triangle names a vertex that is not there")?;
            for a in 0..3 {
                min[a] = min[a].min(p[a]);
                max[a] = max[a].max(p[a]);
            }
        }
    }
    if min.iter().chain(&max).any(|v| !v.is_finite()) {
        return Err("the mesh has no triangle with finite vertices".into());
    }
    // the cells that the box of the mesh touches
    let limit = f64::from(1u32 << 30);
    let range: Vec<(i64, i64)> = (0..3)
        .map(|a| {
            let (lo, hi) = ((min[a] / cell_size).floor(), (max[a] / cell_size).ceil());
            if lo.abs() > limit || hi.abs() > limit {
                return Err(format!("the mesh is too far from the origin for cells of {cell_size}"));
            }
            Ok((lo as i64, hi as i64))
        })
        .collect::<Result<_, _>>()?;
    let side = |a: usize| (range[a].1 - range[a].0).max(1) as u128;
    let box_cells = side(0) * side(1) * side(2);
    if box_cells > u128::from(bounds.max_box_cells) {
        return Err(format!(
            "the mesh spans {box_cells} cells at this size and the limit is {} cells",
            bounds.max_box_cells
        ));
    }
    let lo = [range[0].0 as i32, range[1].0 as i32, range[2].0 as i32];
    let hi = [(range[0].1 - 1) as i32, (range[1].1 - 1) as i32, (range[2].1 - 1) as i32];
    let estimate = Occupancy::estimate_bytes((box_cells as u64).min(limits.max_cells), lo, hi);
    if estimate > limits.max_bytes as u64 {
        return Err(format!(
            "the cells of this box may take {estimate} bytes and the limit is {} bytes",
            limits.max_bytes
        ));
    }
    let region = Mesh::new(points, triangles, bounds.max_mesh_bytes)
        .map_err(|e| format!("the mesh is not a closed surface that can be cut into cells: {e}"))?;
    let rows: Vec<(i32, i32)> = (lo[2]..=hi[2]).flat_map(|z| (lo[1]..=hi[1]).map(move |y| (y, z))).collect();
    let filled: Vec<Vec<i32>> = rows
        .par_iter()
        .map(|&(y, z)| {
            (lo[0]..=hi[0])
                .filter(|&x| {
                    let centre = [
                        (f64::from(x) + 0.5) * cell_size,
                        (f64::from(y) + 0.5) * cell_size,
                        (f64::from(z) + 0.5) * cell_size,
                    ];
                    region.contains(centre)
                })
                .collect()
        })
        .collect();
    let cells = rows.iter().zip(&filled).flat_map(|(&(y, z), xs)| xs.iter().map(move |&x| ([x, y, z], 1u8)));
    Occupancy::from_cells_with_limits(limits, cells)
}
