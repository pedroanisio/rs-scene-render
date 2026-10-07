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

pub mod vox;

/// What an importer makes of a file.
#[derive(Debug)]
pub struct Imported {
    /// The cells, with the palette beside them if the file has colours.
    pub occupancy: Occupancy,
    /// Whether the file had colours, which are in the palette of `occupancy` (palette index c, 1 to 255, is the file's colour c - 1).
    pub colours: bool,
    /// The properties of the materials the file names, by palette index, as the file spells them (`_type`, `_rough`, ...).
    pub materials: BTreeMap<u8, BTreeMap<String, String>>,
}

/// The cell of the scene that a cell of MagicaVoxel's axes (x right, y forward, z up) is.
pub fn scene_cell(c: [i32; 3]) -> [i32; 3] {
    [c[0], -c[2] - 1, c[1]]
}
