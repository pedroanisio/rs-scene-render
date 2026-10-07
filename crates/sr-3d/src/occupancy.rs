//! The occupancy of a voxel object: which cells of an integer lattice are filled, and with what.
//!
//! This is the contract between whoever fills a voxel object (an importer, a sequence, the removal of cells by a crater)
//! and whoever reads it (the rigid world, the mesher):
//!
//! * A cell is an integer key `[x, y, z]`; a cell's value is a palette index, `0` meaning empty and `1..=255` a
//!   material of the object's palette (the palette itself belongs to the asset, not to the grid).
//! * **Coordinates.** The cell `[i, j, k]` occupies `[i, i + 1) x [j, j + 1) x [k, k + 1)` of the object's space, in cells, in the object's
//!   own axes (the scene's: x right, y down, z away from the camera); an importer from another convention (MagicaVoxel is z up) turns
//!   its keys into these before it fills the grid. The size of a cell belongs to the object, not to the grid. The palette index
//!   is a `u8` by design (a `.vox` model has 255 colours): a label above 255 is the importer's error to report.
//! * Keys are within `[-2^30, 2^30)` ([`KEY_LIMIT`]): a cell outside is refused by every way of filling a grid, so that no neighbour, brick
//!   corner or sum of moments overflows (the moments hold for up to 2^32 cells).
//! * Cells are stored in sparse **bricks** of 8 by 8 by 8 cells, keyed by the key divided (flooring) by 8. Keys may
//!   be negative. A grid may have a limit on its bricks.
//! * The **scan order** of cells, which every ordering in this module and above it uses, is z, then y, then x (x runs fastest).
//! * The grid carries a **palette** of 256 RGBA colours beside the cells (index 0 is empty and its colour does not matter; the
//!   others start opaque white). Colours are not geometry: recolouring changes [`Occupancy::palette_revision`] and
//!   [`Occupancy::appearance_fingerprint`], and leaves the revision and [`Occupancy::fingerprint`] alone, so a collider, a mass or a
//!   component list computed from the cells stays valid and a mesh with colours does not.
//! * An importer that has the whole of a model builds it in bulk, from dense bricks ([`Occupancy::from_bricks`]) or from a list of
//!   cells with their palette indices ([`Occupancy::from_cells`]); the result has the same fingerprint as the same cells set one by
//!   one, and every brick of it is new to a reader at revision 0.
//! * The **revision** is a counter of this grid that changes when, and only when, a cell changes; the bricks that changed
//!   since a revision are told by [`Occupancy::changed_bricks_since`], in key order, a brick that was emptied included until it
//!   is compacted away. A revision compares states of one grid only (grids built differently may have the same one, and two copies
//!   of a grid that are edited differently reach the same one): whoever caches by revision keys it by the grid's
//!   [`Occupancy::lineage`] as well. The **fingerprint** is the hash of the content
//!   (cells and palette indices, in the order of the scan) and is equal for equal content, whatever its history.
//! * What is derived from the cells (mass, centre of mass, inertia, connected components) is computed from integers in a fixed
//!   order, so equal occupancy gives equal bits ([`Moments`], [`components`]).

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};

/// Cells to a side of a brick.
pub const BRICK: i32 = 8;

/// Every key of a cell is in `[-KEY_LIMIT, KEY_LIMIT)`: what the neighbours of a cell, the doubled coordinates of the moments and the key
/// of a brick times 8 need so as never to overflow. A cell outside it is an error to fill (and is empty to read).
pub const KEY_LIMIT: i32 = 1 << 30;

fn in_range(key: [i32; 3]) -> bool {
    key.iter().all(|k| (-KEY_LIMIT..KEY_LIMIT).contains(k))
}
const BRICK_CELLS: usize = 512;

#[derive(Clone, Debug)]
struct Brick {
    cells: Box<[u8; BRICK_CELLS]>,
    filled: u16,
    /// The revision of the grid after the last edit of this brick.
    changed: u64,
}

/// What a grid may hold. A cell, a brick or a byte over a limit is an error where it is set, never a truncation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Bricks that hold at least one cell.
    pub max_bricks: usize,
    /// Filled cells.
    pub max_cells: u64,
    /// Bytes of the bricks, by [`Occupancy::bytes`]: bricks times [`Occupancy::BRICK_BYTES`].
    pub max_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_bricks: usize::MAX, max_cells: u64::MAX, max_bytes: usize::MAX }
    }
}

/// 256 RGBA colours, which the palette indices of the cells of an [`Occupancy`] choose from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    colors: [[u8; 4]; 256],
}

impl Default for Palette {
    fn default() -> Self {
        let mut colors = [[255; 4]; 256];
        colors[0] = [0; 4];
        Self { colors }
    }
}

impl Palette {
    pub fn color(&self, index: u8) -> [u8; 4] {
        self.colors[usize::from(index)]
    }

    pub fn colors(&self) -> &[[u8; 4]; 256] {
        &self.colors
    }
}

static NEXT_LINEAGE: AtomicU64 = AtomicU64::new(1);

/// A sparse grid of filled cells, with a palette index in each.
#[derive(Debug)]
pub struct Occupancy {
    bricks: BTreeMap<[i32; 3], Brick>,
    filled: u64,
    revision: u64,
    limits: Limits,
    /// Bricks with at least one cell.
    live: usize,
    palette: Palette,
    palette_revision: u64,
    /// Which grid this is: its own for each grid built and each copy made, for the whole of its life.
    lineage: u64,
}

/// A copy has the cells, the palette and the revision of the grid it was made from and a lineage of its own: from then on the two are
/// edited separately, and a revision of one is not a revision of the other.
impl Clone for Occupancy {
    fn clone(&self) -> Self {
        Self {
            bricks: self.bricks.clone(),
            filled: self.filled,
            revision: self.revision,
            limits: self.limits,
            live: self.live,
            palette: self.palette.clone(),
            palette_revision: self.palette_revision,
            lineage: NEXT_LINEAGE.fetch_add(1, Ordering::Relaxed),
        }
    }
}

impl Default for Occupancy {
    fn default() -> Self {
        Self::new()
    }
}

fn split(key: [i32; 3]) -> ([i32; 3], usize) {
    let brick = key.map(|c| c.div_euclid(BRICK));
    let local = key.map(|c| c.rem_euclid(BRICK) as usize);
    (brick, local[0] + 8 * (local[1] + 8 * local[2]))
}

/// The key of the cell at `index` of the brick `brick`.
fn join(brick: [i32; 3], index: usize) -> [i32; 3] {
    [
        brick[0] * BRICK + (index % 8) as i32,
        brick[1] * BRICK + ((index / 8) % 8) as i32,
        brick[2] * BRICK + (index / 64) as i32,
    ]
}

fn scan_key(c: &[i32; 3]) -> (i32, i32, i32) {
    (c[2], c[1], c[0])
}

impl Occupancy {
    /// An empty grid with no limit.
    pub fn new() -> Self {
        Self {
            bricks: BTreeMap::new(),
            filled: 0,
            revision: 0,
            limits: Limits { max_bricks: usize::MAX, max_cells: u64::MAX, max_bytes: usize::MAX },
            live: 0,
            palette: Palette::default(),
            palette_revision: 0,
            lineage: NEXT_LINEAGE.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// An empty grid that holds at most `max_bricks` bricks (of 512 cells); a cell in another brick is an error.
    pub fn with_limit(max_bricks: usize) -> Self {
        Self::with_limits(Limits { max_bricks, ..Limits::default() })
    }

    /// An empty grid with limits on its bricks, cells and bytes.
    pub fn with_limits(limits: Limits) -> Self {
        Self { limits, ..Self::new() }
    }

    /// What a brick costs, in bytes, for the limits and for [`Occupancy::bytes`]: its 512 palette indices and the bookkeeping of a node
    /// of the tree it is kept in. An estimate, the same for every brick.
    pub const BRICK_BYTES: usize = BRICK_CELLS + 48;

    /// The bytes of the bricks that hold something.
    pub fn bytes(&self) -> usize {
        self.live * Self::BRICK_BYTES
    }

    /// An upper bound on the bytes of a grid of `cells` filled cells whose box is `min` to `max` (inclusive keys), worked out before
    /// anything is allocated: no more bricks than cells, and no more than the box has.
    pub fn estimate_bytes(cells: u64, min: [i32; 3], max: [i32; 3]) -> u64 {
        let bricks: u128 = (0..3)
            .map(|a| (i64::from(max[a].div_euclid(BRICK)) - i64::from(min[a].div_euclid(BRICK)) + 1).max(0) as u128)
            .product();
        let bound = u128::from(cells).min(bricks) * Self::BRICK_BYTES as u128;
        u64::try_from(bound).unwrap_or(u64::MAX)
    }

    /// The smallest and largest keys of the filled cells (inclusive), exact for what is there now; none for an empty grid.
    pub fn bounds(&self) -> Option<([i32; 3], [i32; 3])> {
        let mut bounds: Option<([i32; 3], [i32; 3])> = None;
        for (key, cells) in self.bricks() {
            for (i, c) in cells.iter().enumerate() {
                if *c == 0 {
                    continue;
                }
                let cell = join(key, i);
                bounds = Some(match bounds {
                    None => (cell, cell),
                    Some((lo, hi)) => {
                        (std::array::from_fn(|a| lo[a].min(cell[a])), std::array::from_fn(|a| hi[a].max(cell[a])))
                    }
                });
            }
        }
        bounds
    }

    /// The size of the box of the filled cells, in cells along each axis; none for an empty grid.
    pub fn extent(&self) -> Option<[u64; 3]> {
        self.bounds().map(|(lo, hi)| std::array::from_fn(|a| (i64::from(hi[a]) - i64::from(lo[a]) + 1) as u64))
    }

    /// Whether one more cell, in a brick that holds nothing yet (`new_brick`) or in one that does, fits the limits.
    fn room(&self, new_brick: bool) -> Result<(), String> {
        if self.filled >= self.limits.max_cells {
            return Err(format!("the occupancy would hold more than {} cells", self.limits.max_cells));
        }
        if new_brick && self.live >= self.limits.max_bricks {
            return Err(format!("the occupancy would hold more than {} bricks", self.limits.max_bricks));
        }
        if new_brick && (self.live + 1).saturating_mul(Self::BRICK_BYTES) > self.limits.max_bytes {
            return Err(format!("the occupancy would hold more than {} bytes", self.limits.max_bytes));
        }
        Ok(())
    }

    /// A grid made of dense bricks (512 palette indices each, x fastest, then y, then z), in any order: a brick that is all empty is
    /// not kept, a brick given twice is an error. Every brick of it has changed since revision 0.
    pub fn from_bricks(bricks: impl IntoIterator<Item = ([i32; 3], [u8; BRICK_CELLS])>) -> Result<Self, String> {
        Self::from_bricks_with_limit(usize::MAX, bricks)
    }

    /// As [`Occupancy::from_bricks`], for a grid with a limit on its bricks.
    pub fn from_bricks_with_limit(
        max_bricks: usize,
        bricks: impl IntoIterator<Item = ([i32; 3], [u8; BRICK_CELLS])>,
    ) -> Result<Self, String> {
        Self::from_bricks_with_limits(Limits { max_bricks, ..Limits::default() }, bricks)
    }

    /// As [`Occupancy::from_bricks`], for a grid with limits: over one is an error, not a grid that was cut short.
    pub fn from_bricks_with_limits(
        limits: Limits,
        bricks: impl IntoIterator<Item = ([i32; 3], [u8; BRICK_CELLS])>,
    ) -> Result<Self, String> {
        let mut o = Self::with_limits(limits);
        let mut given = std::collections::BTreeSet::new();
        for (key, cells) in bricks {
            if !given.insert(key) {
                return Err(format!("the brick {key:?} is given twice"));
            }
            if key.iter().any(|k| !(-KEY_LIMIT / BRICK..KEY_LIMIT / BRICK).contains(k)) {
                return Err(format!("the brick {key:?} is outside the keys of an occupancy"));
            }
            let filled = cells.iter().filter(|c| **c != 0).count() as u16;
            if filled == 0 {
                continue;
            }
            // the cells of the brick come in at once: the brick's cost, then its cells against the limit on cells
            o.room(true)?;
            if o.filled + u64::from(filled) > o.limits.max_cells {
                return Err(format!("the occupancy would hold more than {} cells", o.limits.max_cells));
            }
            o.filled += u64::from(filled);
            o.live += 1;
            o.bricks.insert(key, Brick { cells: Box::new(cells), filled, changed: 1 });
        }
        o.revision = u64::from(!o.bricks.is_empty());
        Ok(o)
    }

    /// A grid made of cells with their palette indices (1 to 255), in any order; a cell given twice, or with the index 0, is an error.
    pub fn from_cells(cells: impl IntoIterator<Item = ([i32; 3], u8)>) -> Result<Self, String> {
        Self::from_cells_with_limit(usize::MAX, cells)
    }

    /// As [`Occupancy::from_cells`], for a grid with a limit on its bricks.
    pub fn from_cells_with_limit(
        max_bricks: usize,
        cells: impl IntoIterator<Item = ([i32; 3], u8)>,
    ) -> Result<Self, String> {
        Self::from_cells_with_limits(Limits { max_bricks, ..Limits::default() }, cells)
    }

    /// As [`Occupancy::from_cells`], for a grid with limits: over one is an error, not a grid that was cut short.
    pub fn from_cells_with_limits(
        limits: Limits,
        cells: impl IntoIterator<Item = ([i32; 3], u8)>,
    ) -> Result<Self, String> {
        let mut o = Self::with_limits(limits);
        for (key, palette) in cells {
            if palette == 0 {
                return Err(format!("the cell {key:?} is listed with the empty palette index"));
            }
            if !in_range(key) {
                return Err(format!("the cell {key:?} is outside the keys of an occupancy"));
            }
            let (brick_key, index) = split(key);
            let new_brick = o.bricks.get(&brick_key).is_none_or(|b| b.filled == 0);
            o.room(new_brick)?;
            let brick = o.bricks.entry(brick_key).or_insert_with(|| Brick {
                cells: Box::new([0; BRICK_CELLS]),
                filled: 0,
                changed: 1,
            });
            if brick.cells[index] != 0 {
                return Err(format!("the cell {key:?} is listed twice"));
            }
            brick.cells[index] = palette;
            brick.filled += 1;
            o.filled += 1;
            o.live += usize::from(new_brick);
        }
        o.revision = u64::from(!o.bricks.is_empty());
        Ok(o)
    }

    /// The palette of colours beside the cells.
    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// Changes whenever a colour of the palette does, and only then.
    pub fn palette_revision(&self) -> u64 {
        self.palette_revision
    }

    /// Sets one colour of the palette; true if that changed it. The cells and their revision are not touched.
    pub fn set_color(&mut self, index: u8, rgba: [u8; 4]) -> bool {
        if index == 0 || self.palette.colors[usize::from(index)] == rgba {
            return false;
        }
        self.palette.colors[usize::from(index)] = rgba;
        self.palette_revision += 1;
        true
    }

    /// Sets the whole palette (an importer's); true if that changed it.
    pub fn set_palette(&mut self, mut colors: [[u8; 4]; 256]) -> bool {
        // the empty index has no colour to set
        colors[0] = self.palette.colors[0];
        if self.palette.colors == colors {
            return false;
        }
        self.palette.colors = colors;
        self.palette_revision += 1;
        true
    }

    /// A hash of the content and of the colours: equal cells with equal palettes are equal, whatever the history. The key of what
    /// is made from both, a mesh with colours; for what is made from the cells alone use [`Occupancy::fingerprint`].
    pub fn appearance_fingerprint(&self) -> u64 {
        let mut h = self.fingerprint();
        for color in self.palette.colors {
            for byte in color {
                h ^= u64::from(byte);
                h = h.wrapping_mul(0x100000001b3);
            }
        }
        h ^= h >> 29;
        h.wrapping_mul(0xff51afd7ed558ccd)
    }

    /// The palette index of a cell; 0 for an empty one.
    pub fn get(&self, key: [i32; 3]) -> u8 {
        let (brick, index) = split(key);
        self.bricks.get(&brick).map_or(0, |b| b.cells[index])
    }

    /// Sets a cell; true if that changed it. A brick that holds nothing yet is made, and a cell filled, only within the limits.
    pub fn set(&mut self, key: [i32; 3], palette: u8) -> Result<bool, String> {
        if !in_range(key) {
            return if palette == 0 {
                Ok(false)
            } else {
                Err(format!("the cell {key:?} is outside the keys of an occupancy"))
            };
        }
        let (brick_key, index) = split(key);
        let present = self.bricks.get(&brick_key).map(|b| b.cells[index]);
        if present.unwrap_or(0) == palette {
            return Ok(false);
        }
        if present.unwrap_or(0) == 0 {
            // a cell that is filled where none was
            self.room(self.bricks.get(&brick_key).is_none_or(|b| b.filled == 0))?;
        }
        self.revision += 1;
        let revision = self.revision;
        let brick = self.bricks.entry(brick_key).or_insert_with(|| Brick {
            cells: Box::new([0; BRICK_CELLS]),
            filled: 0,
            changed: 0,
        });
        let before = brick.cells[index];
        brick.cells[index] = palette;
        brick.changed = revision;
        match (before, palette) {
            (0, _) => {
                brick.filled += 1;
                self.filled += 1;
                self.live += usize::from(brick.filled == 1);
            }
            (_, 0) => {
                brick.filled -= 1;
                self.filled -= 1;
                self.live -= usize::from(brick.filled == 0);
            }
            _ => {}
        }
        Ok(true)
    }

    /// How many cells are filled.
    pub fn count(&self) -> u64 {
        self.filled
    }

    /// Changes whenever a cell changes, and only then.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Which grid this is, for as long as it lives: every grid built, and every copy made of one, has a lineage of its own (a number
    /// for this process, never part of a hash, and so of no consequence for a result). A revision is a revision of one lineage; the
    /// state of a grid is told by the two together.
    pub fn lineage(&self) -> u64 {
        self.lineage
    }

    /// The bricks that hold something, in key order, with their 512 palette indices (x runs fastest, then y, then z).
    pub fn bricks(&self) -> impl Iterator<Item = ([i32; 3], &[u8; BRICK_CELLS])> {
        self.bricks.iter().filter(|(_, b)| b.filled > 0).map(|(k, b)| (*k, &*b.cells))
    }

    /// The keys of the bricks edited after `revision`, in key order; a brick that was emptied is among them until [`Occupancy::compact`].
    pub fn changed_bricks_since(&self, revision: u64) -> Vec<[i32; 3]> {
        self.bricks.iter().filter(|(_, b)| b.changed > revision).map(|(k, _)| *k).collect()
    }

    /// Forgets the emptied bricks last edited at or before `revision`: whoever read the grid up to then has been told. The mesher of the
    /// surface reads `changed_bricks_since` up to the revision it built, so that revision is the one to give here: a brick emptied after
    /// it stays until it has been read.
    pub fn compact(&mut self, revision: u64) {
        self.bricks.retain(|_, b| b.filled > 0 || b.changed > revision);
    }

    /// The filled cells in the order of the scan.
    pub fn cells(&self) -> impl Iterator<Item = [i32; 3]> {
        let mut cells: Vec<[i32; 3]> = self
            .bricks
            .iter()
            .flat_map(|(k, b)| (0..BRICK_CELLS).filter(move |i| b.cells[*i] != 0).map(move |i| join(*k, i)))
            .collect();
        cells.sort_by_key(scan_key);
        cells.into_iter()
    }

    /// A hash of the content, equal for equal content (the filled cells and their palette indices), whatever order and history made it.
    /// Not a cryptographic hash.
    pub fn fingerprint(&self) -> u64 {
        let mut h = 0xcbf29ce484222325u64;
        let mut eat = |byte: u8| {
            h ^= u64::from(byte);
            h = h.wrapping_mul(0x100000001b3);
        };
        for (key, cells) in self.bricks() {
            for c in key {
                c.to_le_bytes().into_iter().for_each(&mut eat);
            }
            cells.iter().copied().for_each(&mut eat);
        }
        // a final mix so that near contents are far apart
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51afd7ed558ccd);
        h ^ (h >> 33)
    }

    /// The connected components of the filled cells (see [`components`]).
    pub fn components(&self) -> Vec<Vec<[i32; 3]>> {
        components(&self.cells().collect::<Vec<_>>())
    }

    /// The exact moments of the filled cells (see [`Moments`]).
    pub fn moments(&self) -> Moments {
        Moments::of(self.cells())
    }
}

/// The moments of a set of cells, in integers: the count and the sums of the doubled coordinates `u = 2 key + 1` (twice the
/// centre of a cell in cells) and of their products, so that the centre of mass and the inertia are worked out from exact numbers
/// and a single division each, and are the same bits for any order of the cells. Keys within [`KEY_LIMIT`] (2^30) of the origin and at most
/// 2^32 cells keep every sum, and the products of two sums, inside an `i128` (the products reach 2^126).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Moments {
    n: u64,
    s: [i128; 3],
    /// xx, yy, zz, xy, xz, yz
    ss: [i128; 6],
}

const PAIRS: [(usize, usize); 6] = [(0, 0), (1, 1), (2, 2), (0, 1), (0, 2), (1, 2)];

impl Moments {
    pub fn of(cells: impl IntoIterator<Item = [i32; 3]>) -> Self {
        let mut m = Moments::default();
        for c in cells {
            let u = c.map(|k| 2 * i128::from(k) + 1);
            m.n += 1;
            for (sum, v) in m.s.iter_mut().zip(u) {
                *sum += v;
            }
            for (sum, (a, b)) in m.ss.iter_mut().zip(PAIRS) {
                *sum += u[a] * u[b];
            }
        }
        m
    }

    /// The moments of the union of two sets of cells with no cell in common.
    pub fn add(&mut self, other: &Moments) {
        self.n += other.n;
        for (sum, v) in self.s.iter_mut().zip(other.s) {
            *sum += v;
        }
        for (sum, v) in self.ss.iter_mut().zip(other.ss) {
            *sum += v;
        }
    }

    pub fn count(&self) -> u64 {
        self.n
    }

    /// Mass, centre of mass and inertia tensor about it, of cells of `size` (metres, along each axis of the lattice) of `density`
    /// (kilograms a cubic metre), in the axes of the lattice. The mass is the number of cells times the mass of one.
    pub fn properties(&self, size: [f64; 3], density: f64) -> Result<Properties, String> {
        if self.n == 0 {
            return Err("an object with no cells has no mass properties".into());
        }
        if !(size.iter().all(|s| s.is_finite() && *s > 0.0) && density.is_finite() && density > 0.0) {
            return Err("the cells need a positive size and density".into());
        }
        let n = self.n as f64;
        let cell_mass = density * size[0] * size[1] * size[2];
        let mass = n * cell_mass;
        let centre: [f64; 3] = std::array::from_fn(|a| size[a] * (self.s[a] as f64) / (2.0 * n));
        // the covariance of the doubled coordinates, exact in the numerator: (N sum(uv) - sum(u) sum(v)) / N^2
        let nn = i128::from(self.n);
        let covariance = |k: usize| {
            let (a, b) = PAIRS[k];
            (nn * self.ss[k] - self.s[a] * self.s[b]) as f64 / (n * n)
        };
        // variance of the centres of the cells along each axis and the products, in metres squared
        let var = |a: usize| size[a] * size[a] / 4.0 * covariance(a);
        let cross = |k: usize| {
            let (a, b) = PAIRS[k];
            size[a] * size[b] / 4.0 * covariance(k)
        };
        let mut inertia = [[0.0; 3]; 3];
        for (a, row) in inertia.iter_mut().enumerate() {
            let (b, c) = ((a + 1) % 3, (a + 2) % 3);
            // each cell about its own centre: m (d_b^2 + d_c^2) / 12; then the spread of the centres about the centre of mass
            let own = cell_mass / 12.0 * (size[b] * size[b] + size[c] * size[c]);
            row[a] = n * own + mass * (var(b) + var(c));
        }
        for (k, (a, b)) in PAIRS.iter().enumerate().skip(3) {
            let product = -mass * cross(k);
            inertia[*a][*b] = product;
            inertia[*b][*a] = product;
        }
        Ok(Properties { mass, centre, inertia })
    }
}

/// Mass properties in the axes of the lattice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Properties {
    pub mass: f64,
    pub centre: [f64; 3],
    /// About the centre of mass.
    pub inertia: [[f64; 3]; 3],
}

/// The principal moments of inertia, ascending, and the principal axes (`axes[k]` is the unit axis of `moments[k]`; a right-handed frame).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Principal {
    pub moments: [f64; 3],
    pub axes: [[f64; 3]; 3],
}

impl Properties {
    /// The principal moments and axes, by cyclic Jacobi rotations in a fixed order. A tensor that is already diagonal is not
    /// rotated, so equal moments keep the axes of the lattice; each axis is signed so that its largest component is positive, and
    /// the third is the cross product of the first two.
    pub fn principal(&self) -> Principal {
        let mut a = self.inertia;
        let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        for _ in 0..64 {
            let off = a[0][1].abs() + a[0][2].abs() + a[1][2].abs();
            let scale = a[0][0].abs() + a[1][1].abs() + a[2][2].abs();
            if off <= 1e-300 || off <= 1e-17 * scale {
                break;
            }
            for (p, q) in [(0, 1), (0, 2), (1, 2)] {
                if a[p][q] == 0.0 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
                let t = if theta >= 0.0 { 1.0 } else { -1.0 } / (theta.abs() + (theta * theta + 1.0).sqrt());
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for row in a.iter_mut() {
                    let (akp, akq) = (row[p], row[q]);
                    row[p] = c * akp - s * akq;
                    row[q] = s * akp + c * akq;
                }
                let (rp, rq) = (a[p], a[q]);
                a[p] = std::array::from_fn(|k| c * rp[k] - s * rq[k]);
                a[q] = std::array::from_fn(|k| s * rp[k] + c * rq[k]);
                for row in v.iter_mut() {
                    let (vkp, vkq) = (row[p], row[q]);
                    row[p] = c * vkp - s * vkq;
                    row[q] = s * vkp + c * vkq;
                }
            }
        }
        // eigenvectors are the columns of v; order by eigenvalue (ties keep the order of the axes)
        let mut order = [0usize, 1, 2];
        order.sort_by(|i, j| a[*i][*i].partial_cmp(&a[*j][*j]).unwrap_or(std::cmp::Ordering::Equal));
        let moments = order.map(|k| a[k][k]);
        let mut axes = order.map(|k| [v[0][k], v[1][k], v[2][k]]);
        for axis in axes.iter_mut().take(2) {
            let big = (0..3)
                .max_by(|i, j| axis[*i].abs().partial_cmp(&axis[*j].abs()).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap_or(0);
            if axis[big] < 0.0 {
                *axis = axis.map(|c| -c);
            }
        }
        axes[2] = [
            axes[0][1] * axes[1][2] - axes[0][2] * axes[1][1],
            axes[0][2] * axes[1][0] - axes[0][0] * axes[1][2],
            axes[0][0] * axes[1][1] - axes[0][1] * axes[1][0],
        ];
        Principal { moments, axes }
    }
}

/// The connected components of a set of cells, by faces (six neighbours): each lists its cells in the order of the scan, and
/// the components come in the order of their first cell in the scan. The result does not depend on the order of the input, and
/// a cell given twice counts once.
pub fn components(cells: &[[i32; 3]]) -> Vec<Vec<[i32; 3]>> {
    let mut sorted: Vec<[i32; 3]> = cells.to_vec();
    sorted.sort_by_key(scan_key);
    sorted.dedup();
    let index: HashMap<[i32; 3], usize> = sorted.iter().enumerate().map(|(i, c)| (*c, i)).collect();
    // union-find in which the smaller index is always the root, so a component's root is its first cell in the scan
    let mut parent: Vec<usize> = (0..sorted.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for (i, c) in sorted.iter().enumerate() {
        for axis in 0..3 {
            let mut next = *c;
            next[axis] += 1;
            if let Some(&j) = index.get(&next) {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[a.max(b)] = a.min(b);
                }
            }
        }
    }
    let mut slot: Vec<Option<usize>> = vec![None; sorted.len()];
    let mut parts: Vec<Vec<[i32; 3]>> = Vec::new();
    for (i, c) in sorted.iter().enumerate() {
        let root = find(&mut parent, i);
        let k = *slot[root].get_or_insert_with(|| {
            parts.push(Vec::new());
            parts.len() - 1
        });
        parts[k].push(*c);
    }
    parts
}
