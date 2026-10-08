//! A voxel model as an SRVOL cache (the engine's volume cache, `sr_volume`): one grid, `voxels`, whose value at the index
//! `[i, j, k]` is the palette index (1 to 255, exact in a float) of the cell `[i, j, k]` of the lattice, and 0 where there is none
//! (the background). The transform of the grid is the size of a cell: a scale, equal on the three axes, from the origin, and the
//! index `[i, j, k]` is the corner of the cell, so a cell fills `[i, i + 1)` of the grid's index space.
//!
//! An SRVOL cache has no palette, so a model that comes out of one has no colours: the materials come from the object that uses it.
//! A value that is not an integer from 0 to 255 is an error that names the cell and the value, never a number that wrapped.

use crate::occupancy::{Limits, Occupancy, BRICK};
use sr_volume::{CacheLimits, SparseGrid, Transform, Volume};

/// The name of the grid that holds the cells.
pub const GRID: &str = "voxels";

/// A model read from a cache.
#[derive(Debug)]
pub struct Import {
    pub occupancy: Occupancy,
    /// The scale of the grid, if it is the same on the three axes with no shear and no offset: the size of a cell that the cache says.
    pub cell_size: Option<f64>,
}

fn message(e: sr_volume::Error) -> String {
    e.to_string()
}

/// The bytes of a cache that holds `occupancy` with cells of `cell_size`: canonical, the same bytes for the same cells.
pub fn write(occupancy: &Occupancy, cell_size: f64) -> Result<Vec<u8>, String> {
    if !(cell_size.is_finite() && cell_size > 0.0) {
        return Err(format!("the size of a cell, {cell_size}, is not a positive number"));
    }
    let s = cell_size;
    let transform =
        Transform::new([s, 0.0, 0.0, 0.0, 0.0, s, 0.0, 0.0, 0.0, 0.0, s, 0.0, 0.0, 0.0, 0.0, 1.0]).map_err(message)?;
    let bricks = occupancy.bricks().count();
    let mut grid = SparseGrid::new(transform, 0.0, bricks.max(1)).map_err(message)?;
    for (key, cells) in occupancy.bricks() {
        let values: [f32; 512] = std::array::from_fn(|i| f32::from(cells[i]));
        grid.set_brick(key, &values).map_err(message)?;
    }
    let mut volume = Volume::new();
    volume.insert(GRID, grid).map_err(message)?;
    let mut out = Vec::new();
    volume.write(&mut out).map_err(message)?;
    Ok(out)
}

/// The model in the grid `grid` of a cache, within `limits` and the decoder's `cache` limits.
pub fn import(bytes: &[u8], grid: &str, limits: Limits, cache: CacheLimits) -> Result<Import, String> {
    let volume = Volume::read(bytes, cache).map_err(message)?;
    let source = volume.grid(grid).ok_or_else(|| {
        format!("the cache has no grid \"{grid}\" (the model is the grid \"{GRID}\" unless the asset says another)")
    })?;
    if source.background() != 0.0 {
        return Err(format!(
            "the grid \"{grid}\" has the background {}, and a model's is 0 (no cell)",
            source.background()
        ));
    }
    let mut bricks = Vec::new();
    for (key, values) in source.bricks() {
        let mut cells = [0u8; 512];
        for (i, v) in values.iter().enumerate() {
            if *v == 0.0 {
                continue;
            }
            let at = [
                key[0] * BRICK + (i % 8) as i32,
                key[1] * BRICK + (i / 8 % 8) as i32,
                key[2] * BRICK + (i / 64) as i32,
            ];
            if !(v.is_finite() && v.fract() == 0.0 && (1.0..=255.0).contains(v)) {
                return Err(format!(
                    "the cell {at:?} has the value {v}, and a palette index is a whole number from 1 to 255"
                ));
            }
            cells[i] = *v as u8;
        }
        bricks.push((key, cells));
    }
    let occupancy = Occupancy::from_bricks_with_limits(limits, bricks)?;
    Ok(Import { occupancy, cell_size: cell_size_of(source.transform()) })
}

fn cell_size_of(transform: Transform) -> Option<f64> {
    let c = transform.columns();
    let off_diagonal = [c[1], c[2], c[3], c[4], c[6], c[7], c[8], c[9], c[11], c[12], c[13], c[14]];
    (off_diagonal.iter().all(|v| *v == 0.0) && c[0] == c[5] && c[5] == c[10] && c[0] > 0.0).then_some(c[0])
}
