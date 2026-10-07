//! The pieces that a body of cells is cut into, and which of them touch.
//!
//! The cells of a piece are KEYS OF THE OCCUPANCY that was cut (the object's own lattice, cells and not metres): a piece's cell is a cell
//! of the same grid, so a piece can be made into a body of cells without a change of frame.
//!
//! A [`Partition`] says which part of the body every cell is in (by seeds, by planes, or by labels that the caller gives), and
//! [`partition`] makes the pieces and the joints between them:
//!
//! * Every cell of the occupancy is in exactly one piece. A part of the rule (the cells that have one label) that is not connected is split
//!   into its components by faces ([`crate::occupancy::components`]), so every piece is connected by its six face neighbours.
//! * Pieces are numbered by their first cell in the order of the scan (z, then y, then x), and a piece lists its cells in that order, so the
//!   result is the same for the same cells in any order and on any number of threads.
//! * A joint ([`Edge`]) is the faces that two pieces share, `a < b`, sorted by `(a, b)`: how many, and in exact integers the sum of the
//!   doubled coordinates of their centres and of their normals, so that the area (`faces * cell_size^2`), the centroid
//!   (`face_sum / (2 * faces) * cell_size`) and the mean normal (`normal_sum / faces`) of a joint come from one division each.
//! * Voronoi in exact integers: the seeds and the cells are in doubled coordinates `u = 2 * key + 1` (twice the centre of a cell, in cells),
//!   a cell goes to the nearest seed by the squared distance in `i128`, and a tie goes to the seed of the lowest index. Seeds that are
//!   asked for by count come from `(seed, index)` alone, by splitmix64, with no global random state.
//! * The palette indices of the cells play no part: a partition by material is a [`Partition::Labels`] by the caller.

use crate::occupancy::{components, Occupancy};
use std::collections::{BTreeMap, HashMap};

/// A piece: its cells in the order of the scan, connected by faces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub cells: Vec<[i32; 3]>,
}

/// The joint between two pieces, `a < b`: the faces of cells that they share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub a: u32,
    pub b: u32,
    /// Faces of cells that the two pieces share.
    pub faces: u32,
    /// The sum of the centres of those faces in doubled coordinates: the face between the cell `k` and the cell `k + e` (a unit step on one
    /// axis) is at `2 k + 1` on the two other axes and at `2 k + 2` on that one.
    pub face_sum: [i64; 3],
    /// The sum of the unit normals of those faces, from the piece `a` to the piece `b`.
    pub normal_sum: [i32; 3],
}

/// The pieces and the joints between them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PieceGraph {
    pub pieces: Vec<Piece>,
    /// Sorted by `(a, b)`.
    pub edges: Vec<Edge>,
}

/// A plane over the doubled coordinates `u`: a cell is on its positive side if `normal . u - offset >= 0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plane {
    pub normal: [i64; 3],
    pub offset: i128,
}

/// How a body is divided into parts, before the parts that are not connected are split.
pub enum Partition<'a> {
    /// `seeds` seeds drawn from `(seed, index)` in the box of the cells, then the nearest seed of each cell.
    Voronoi { seeds: u32, seed: u64 },
    /// These seeds, in doubled coordinates, then the nearest seed of each cell (a tie goes to the lowest index).
    VoronoiAt(&'a [[i64; 3]]),
    /// Up to 63 planes: the part of a cell is the mask of the planes it is on the positive side of.
    Planes(&'a [Plane]),
    /// Labels that the caller gives.
    Labels(&'a dyn Fn([i32; 3]) -> u32),
}

/// The most planes of a [`Partition::Planes`].
pub const MAX_PLANES: usize = 63;

const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// splitmix64 of `state`.
fn mix(state: u64) -> u64 {
    let mut z = state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The doubled coordinates of the seeds `0..count` drawn from `seed` in the box `lo..=hi` (doubled): the axis `a` of seed `s` is
/// `lo + splitmix64(seed + (3 s + a + 1) * golden) % (hi - lo + 1)`. (The remainder is slightly biased toward the low end, by at most
/// one part in 2^64 over the size of the box: it is a cut, not a sample.)
pub fn seeds_in(count: u32, seed: u64, lo: [i64; 3], hi: [i64; 3]) -> Vec<[i64; 3]> {
    (0..u64::from(count))
        .map(|s| {
            std::array::from_fn(|a| {
                let range = (hi[a] - lo[a] + 1) as u64;
                let h = mix(seed.wrapping_add((3 * s + a as u64 + 1).wrapping_mul(GOLDEN)));
                lo[a] + (h % range) as i64
            })
        })
        .collect()
}

fn doubled(c: [i32; 3]) -> [i64; 3] {
    c.map(|k| 2 * i64::from(k) + 1)
}

fn scan_key(c: &[i32; 3]) -> (i32, i32, i32) {
    (c[2], c[1], c[0])
}

/// The index of the seed nearest to `u`, the lowest index on a tie.
fn nearest(seeds: &[[i64; 3]], u: [i64; 3]) -> u32 {
    let mut best = (i128::MAX, 0u32);
    for (i, s) in seeds.iter().enumerate() {
        let d: i128 = (0..3).map(|a| i128::from(u[a] - s[a]).pow(2)).sum();
        if d < best.0 {
            best = (d, i as u32);
        }
    }
    best.1
}

/// Cuts the body `occupancy` by `rule` into pieces (at most `max_pieces`, an error that names the number otherwise) and finds the joints.
pub fn partition(occupancy: &Occupancy, rule: Partition, max_pieces: usize) -> Result<PieceGraph, String> {
    let cells: Vec<[i32; 3]> = occupancy.cells().collect();
    let Some((min, max)) = occupancy.bounds() else {
        return Err("a body with no cell has no pieces".into());
    };
    let labels: Vec<u64> = match rule {
        Partition::Labels(label) => cells.iter().map(|c| u64::from(label(*c))).collect(),
        Partition::Planes(planes) => {
            if planes.len() > MAX_PLANES {
                return Err(format!("{} planes, and a partition takes at most {MAX_PLANES}", planes.len()));
            }
            cells
                .iter()
                .map(|c| {
                    let u = doubled(*c);
                    planes.iter().enumerate().fold(0u64, |mask, (i, p)| {
                        let side: i128 =
                            (0..3).map(|a| i128::from(p.normal[a]) * i128::from(u[a])).sum::<i128>() - p.offset;
                        mask | (u64::from(side >= 0) << i)
                    })
                })
                .collect()
        }
        Partition::Voronoi { seeds, seed } => {
            if seeds == 0 {
                return Err("a Voronoi partition needs at least one seed".into());
            }
            let seeds = seeds_in(seeds, seed, doubled(min), doubled(max));
            cells.iter().map(|c| u64::from(nearest(&seeds, doubled(*c)))).collect()
        }
        Partition::VoronoiAt(seeds) => {
            if seeds.is_empty() {
                return Err("a Voronoi partition needs at least one seed".into());
            }
            cells.iter().map(|c| u64::from(nearest(seeds, doubled(*c)))).collect()
        }
    };
    // the cells of each label, then the components of each, then every component by its first cell in the scan
    let mut parts: BTreeMap<u64, Vec<[i32; 3]>> = BTreeMap::new();
    for (c, l) in cells.iter().zip(&labels) {
        parts.entry(*l).or_default().push(*c);
    }
    let mut pieces: Vec<Piece> =
        parts.values().flat_map(|part| components(part)).map(|cells| Piece { cells }).collect();
    if pieces.len() > max_pieces {
        return Err(format!("the partition makes {} pieces and the limit is {max_pieces} pieces", pieces.len()));
    }
    pieces.sort_by_key(|p| scan_key(&p.cells[0]));
    let mut piece_of: HashMap<[i32; 3], u32> = HashMap::with_capacity(cells.len());
    for (i, p) in pieces.iter().enumerate() {
        for c in &p.cells {
            piece_of.insert(*c, i as u32);
        }
    }
    // the joints: every face between a cell and its neighbour one step up an axis, once
    let mut joints: BTreeMap<(u32, u32), Edge> = BTreeMap::new();
    for (c, &mine) in &piece_of {
        for axis in 0..3 {
            let mut next = *c;
            next[axis] += 1;
            let Some(&theirs) = piece_of.get(&next) else { continue };
            if theirs == mine {
                continue;
            }
            let (a, b) = (mine.min(theirs), mine.max(theirs));
            let e = joints.entry((a, b)).or_insert(Edge { a, b, faces: 0, face_sum: [0; 3], normal_sum: [0; 3] });
            e.faces = e.faces.checked_add(1).ok_or("a joint has more faces than a u32 counts")?;
            let u = doubled(*c);
            for (k, sum) in e.face_sum.iter_mut().enumerate() {
                let centre = u[k] + i64::from(k == axis);
                *sum = sum.checked_add(centre).ok_or("the sum of the centres of the faces of a joint overflows")?;
            }
            // the unit normal from a to b: up the axis if the lower cell is in a, down it if it is in b
            e.normal_sum[axis] = e.normal_sum[axis]
                .checked_add(if mine == a { 1 } else { -1 })
                .ok_or("the sum of the normals of the faces of a joint overflows")?;
        }
    }
    Ok(PieceGraph { pieces, edges: joints.into_values().collect() })
}
