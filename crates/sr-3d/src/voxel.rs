//! Voxel models read from files: what an importer makes of a `.vox` file of MagicaVoxel, in the lattice, the axes and the
//! palette that the rest of the engine reads (see [`crate::occupancy`]).
//!
//! * The lattice is the scene's: a cell `[i, j, k]` occupies `[i, i + 1) x [j, j + 1) x [k, k + 1)` of the object's space in cells, in
//!   the axes of the scene (x right, y down, z away from the camera). MagicaVoxel is x right, y forward, z up, also right-handed, and
//!   its cell `[x, y, z]` becomes `[x, -z - 1, y]`: a proper rotation of the lattice (a half turn about x composed with a swap of y
//!   and z, determinant 1), so no face is mirrored and the cells stay exactly on the lattice.
//! * Every import is bounded before it allocates: the file by its size, the cells by their count (and by the count of what a scene
//!   graph places, one model as many times as it is used), the bytes by the estimate of the grid. A model over a limit is an error
//!   that names the number, never a model cut short.

use crate::occupancy::Occupancy;
use std::collections::BTreeMap;

pub mod default_palette;
pub mod material;
pub mod srvol;
pub mod surface;
pub mod vox;

/// What an importer makes of a file.
#[derive(Debug)]
pub struct Imported {
    /// The cells, with the palette beside them if the file has colours.
    pub occupancy: Occupancy,
    /// Where the palette of `occupancy` comes from: the file's own colours (palette index c, 1 to 255, is the file's colour c - 1) or, for a
    /// file with no RGBA chunk, the default palette of the format.
    pub colours: Colours,
    /// The properties of the materials the file names, by palette index, as the file spells them (`_type`, `_rough`, ...).
    pub materials: BTreeMap<u8, BTreeMap<String, String>>,
}

/// Where the colours of a model come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colours {
    /// The RGBA chunk of the file.
    File,
    /// The default palette of the format, which a file with no RGBA chunk has (see [`default_palette`]).
    Default,
}

/// The cell of the scene that a cell of MagicaVoxel's axes (x right, y forward, z up) is.
pub fn scene_cell(c: [i32; 3]) -> [i32; 3] {
    [c[0], -c[2] - 1, c[1]]
}

/// The frame of the cells of an object of `primitive="voxels"`, one place for the evaluator, the physics and the renderer.
///
/// The cells of a model have their keys from zero (the box of the occupied cells has its minimum corner at the key `[0, 0, 0]`; what was taken
/// away to put it there is the model's `origin_cells`, so the key in the file's own lattice is the key plus `origin_cells`). The origin of the
/// object is that corner, not the middle of the box. The axes are the scene's (x right, y DOWN, z away from the camera). The cell with the key
/// `[i, j, k]` fills `[i, i + 1) x [j, j + 1) x [k, k + 1)` cells, and a cell is `cell_size` object units on a side (scene units before the
/// object's own scale): its centre is `(key + 1/2) * cell_size`.
///
/// The centre of the cell `key`, in object units.
pub fn cell_to_object(key: [i32; 3], cell_size: f64) -> [f64; 3] {
    key.map(|k| (f64::from(k) + 0.5) * cell_size)
}

/// The key of the cell that holds the point `p` (object units): the floor of `p / cell_size` on each axis, where a point on a face between two cells is
/// in the one that has the face as its lower face. A point is on a face if `p / cell_size` is within 4 units in the last place (of the larger of its
/// own size and 1) of a whole number: the division of two doubles that stand for decimals is rounded (0.3 / 0.1 is 2.9999999999999996, and 0.3 is the face
/// between the cells 2 and 3 of cells of 0.1), as is the product of the key and the size that a caller gets a face by, and a floor alone would put the
/// point in the cell below. A point nearer to the face than that is taken to be on it; anything farther (a nanometre in a cell of a metre) is not.
/// None for a point that is not finite, or whose cell has a key outside those of an occupancy ([`crate::occupancy::KEY_LIMIT`]), or for a `cell_size` that is
/// not positive and finite.
pub fn object_to_cell(p: [f64; 3], cell_size: f64) -> Option<[i32; 3]> {
    if !(cell_size.is_finite() && cell_size > 0.0) {
        return None;
    }
    let limit = f64::from(crate::occupancy::KEY_LIMIT);
    let mut key = [0i32; 3];
    for a in 0..3 {
        let q = p[a] / cell_size;
        let whole = q.round();
        let k = if (q - whole).abs() <= 4.0 * f64::EPSILON * q.abs().max(1.0) { whole } else { q.floor() };
        if !(k.is_finite() && (-limit..limit).contains(&k)) {
            return None;
        }
        key[a] = k as i32;
    }
    Some(key)
}

/// The key of a cell in the file's own lattice (after the scene graph, in the scene's axes), from its key in the model and the model's `origin_cells`.
pub fn file_key(key: [i32; 3], origin_cells: [i64; 3]) -> [i64; 3] {
    std::array::from_fn(|a| i64::from(key[a]) + origin_cells[a])
}
