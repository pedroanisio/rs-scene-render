//! The pieces that a body of cells is cut into, and which of them touch.
//!
//! This module has two users that were written apart and share it: the fracture of a body of cells into rigid pieces (which makes bodies of
//! the pieces) and the fracture by stress (which breaks the joints between pieces by their area). It is a pure function of an occupancy and a
//! rule, with no state and no global random number, and what it returns cannot be edited into something that breaks its own rules (the
//! fields are private).
//!
//! The cells of a piece are KEYS OF THE OCCUPANCY that was cut (the object's own lattice, cells and not metres): a piece's cell is a cell of
//! the same grid, so a piece can be made into a body of cells without a change of frame. A body with no cell has no pieces, and is an error.
//!
//! A [`Partition`] says which part of the body every cell is in (by seeds, by planes, or by labels that the caller gives), and
//! [`partition`] makes the pieces and the joints between them:
//!
//! * Every cell of the occupancy is in exactly one piece. A part of the rule (the cells that have one label) that is not connected is split
//!   into its components by faces ([`crate::occupancy::components`]), so every piece is connected by its six face neighbours.
//! * Pieces are numbered by their first cell in the order of the scan (z, then y, then x), and a piece lists its cells in that order, so the
//!   result is the same for the same cells in any order and on any number of threads. Nothing here depends on the order of a hash: where the
//!   order of work matters (the first of two overflows to be reported) it is the order of the scan.
//! * A piece has the exact [`Moments`] of its cells, once, so that its mass, centre and inertia are one division each away.
//! * A joint ([`Edge`]) is the faces that two pieces share, `a < b`, sorted by `(a, b)`: how many of them are normal to each axis (so that the
//!   area is right on a lattice whose cells are not cubes), and in exact integers the sum of the doubled coordinates of their centres, by
//!   axis, and of their normals. [`Edge::area`] and [`Edge::centroid`] are the area and the centre of the joint, worked out from them.
//! * [`sections`] gives a joint what a load needs of it: the [`Section`] of its faces (area, centre, exact second moments about the centre, the box that holds
//!   them, the mean normal), as exact integer sums taken from the first face and converted once.
//! * [`PieceGraph::piece_of`] says which piece a cell is in.
//! * Voronoi in exact integers: the seeds and the cells are in doubled coordinates `u = 2 * key + 1` (twice the centre of a cell, in cells),
//!   a cell goes to the nearest seed by the squared distance in `i128`, and a tie goes to the seed of the lowest index. Seeds that are
//!   asked for by count come from `(seed, index)` alone, by splitmix64, with no global random state. Seeds are within
//!   [`MAX_SEED_COORD`] of the origin and at most [`MAX_SEEDS`], and a partition makes `cells * seeds` distances.
//! * The palette indices of the cells play no part: a partition by material is a [`Partition::Labels`] by the caller.
//!
//! What it does not do: it does not merge or cut pieces again, it does not give the centre of the two pieces of a joint (they are in their
//! moments), and a joint's area takes the size of a cell as an argument and has no material.

use crate::occupancy::{components, Moments, Occupancy};
use std::collections::BTreeMap;

/// The most planes of a [`Partition::Planes`].
pub const MAX_PLANES: usize = 63;
/// The most seeds of a [`Partition::Voronoi`] or a [`Partition::VoronoiAt`]: the cost is a distance for every cell and every seed.
pub const MAX_SEEDS: u32 = 4096;
/// The farthest a seed may be from the origin on an axis, in doubled coordinates (cells are within 2^31 of it, so no distance overflows).
pub const MAX_SEED_COORD: i64 = 1 << 40;

/// A piece: its cells in the order of the scan, connected by faces, and their exact moments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    cells: Vec<[i32; 3]>,
    moments: Moments,
}

impl Piece {
    /// The cells, keys of the occupancy that was cut, in the order of the scan (z, y, x).
    pub fn cells(&self) -> &[[i32; 3]] {
        &self.cells
    }

    /// The exact moments of the cells.
    pub fn moments(&self) -> Moments {
        self.moments
    }
}

/// The joint between two pieces, `a < b`: the faces of cells that they share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    a: u32,
    b: u32,
    faces: [u32; 3],
    face_sum: [[i64; 3]; 3],
    normal_sum: [i32; 3],
}

impl Edge {
    /// An edge from its parts (for a consumer that builds one by other means; [`partition`] makes its own). The pieces are `a < b`: an edge from a
    /// piece to itself or from a higher to a lower is an error.
    pub fn new(a: u32, b: u32, faces: [u32; 3], face_sum: [[i64; 3]; 3], normal_sum: [i32; 3]) -> Result<Edge, String> {
        if a >= b {
            return Err(format!("an edge is from a piece to a higher one, and {a} to {b} is not"));
        }
        Ok(Edge { a, b, faces, face_sum, normal_sum })
    }

    pub fn a(&self) -> u32 {
        self.a
    }

    pub fn b(&self) -> u32 {
        self.b
    }

    /// Faces of cells that the two pieces share, by the axis that they are normal to.
    pub fn faces(&self) -> [u32; 3] {
        self.faces
    }

    /// All the faces.
    pub fn total_faces(&self) -> u64 {
        self.faces.iter().map(|f| u64::from(*f)).sum()
    }

    /// The sum of the centres of the faces, in doubled coordinates, by the axis that they are normal to: the face between the cell `k` and the cell
    /// `k + e` (a unit step on one axis) is at `2 k + 1` on the two other axes and at `2 k + 2` on that one.
    pub fn face_sum(&self) -> [[i64; 3]; 3] {
        self.face_sum
    }

    /// The sum of the unit normals of the faces, from the piece `a` to the piece `b`. Faces on the two sides of a piece that is wrapped round
    /// the other cancel here, so this is not an area: [`Edge::faces`] is.
    pub fn normal_sum(&self) -> [i32; 3] {
        self.normal_sum
    }

    /// The area of the joint for cells of `size` (along the axes of the lattice): each face is the product of the sizes of the two other axes.
    pub fn area(&self, size: [f64; 3]) -> f64 {
        (0..3).map(|k| f64::from(self.faces[k]) * face_area(size, k)).sum()
    }

    /// The centre of the joint for cells of `size`, in the same units as `size` and from the corner of the lattice: the mean of the centres of
    /// the faces weighted by their areas (each sum is an integer that is converted once to a float, so the relative error is that of a float
    /// whatever the sum), or none for a joint with no area (an edge built by hand with no faces, or a size of zero).
    pub fn centroid(&self, size: [f64; 3]) -> Option<[f64; 3]> {
        let area = self.area(size);
        if !(area.is_finite() && area > 0.0) {
            return None;
        }
        Some(std::array::from_fn(|j| {
            // the doubled coordinates make the centres, so half the sum is the sum of the centres in cells
            let moment: f64 = (0..3).map(|k| face_area(size, k) * self.face_sum[k][j] as f64 * 0.5 * size[j]).sum();
            moment / area
        }))
    }
}

fn face_area(size: [f64; 3], axis: usize) -> f64 {
    size[(axis + 1) % 3] * size[(axis + 2) % 3]
}

/// The pieces and the joints between them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PieceGraph {
    pieces: Vec<Piece>,
    edges: Vec<Edge>,
    /// Every cell and the piece it is in, in the order of the scan.
    owner: Vec<([i32; 3], u32)>,
}

impl PieceGraph {
    /// The pieces, numbered by their first cell in the order of the scan.
    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    /// The joints, sorted by `(a, b)`.
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// The piece that a cell is in, if it is a cell of the body.
    pub fn piece_of(&self, cell: [i32; 3]) -> Option<u32> {
        self.owner.binary_search_by_key(&scan_key(&cell), |(c, _)| scan_key(c)).ok().map(|i| self.owner[i].1)
    }
}

/// The section of a joint: the faces that two pieces share, as an area that carries a load: its area, its centre, the second moments of its faces about
/// the centre (what a bending stress needs of it) and the box that holds the faces. Lengths are in the units of the `size` that [`sections`] was given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Section {
    /// The sum of the areas of the faces ([`Edge::area`]).
    pub area: f64,
    /// The mean of the centres of the faces, weighted by their areas ([`Edge::centroid`]), from the corner of the lattice.
    pub centroid: [f64; 3],
    /// The integral of `r r^T` over the faces, `r` the position from the centroid: for a flat joint normal to an axis, the two second moments of its plane
    /// and their product, and zero for the axis across. The faces of a joint that is not flat add up as they are, each with the second moment of its own
    /// rectangle (its size squared over twelve along the two axes that it spans).
    pub second: [[f64; 3]; 3],
    /// The unit normal from the piece `a` to the piece `b`: the mean of the normals of the faces weighted by their areas, none if they cancel.
    pub normal: Option<[f64; 3]>,
    /// The corners of the box that holds all the faces, lowest and highest on each axis.
    pub lo: [f64; 3],
    pub hi: [f64; 3],
}

#[derive(Clone, Copy)]
struct SectionSums {
    /// The doubled centre of the first face, which the sums are taken from (so that their size is that of the joint and not of its distance from the origin).
    anchor: Option<[i64; 3]>,
    n: [u64; 3],
    /// By the axis that the face is normal to: the sum of the offsets from the anchor, and of their products, in doubled coordinates.
    s1: [[i128; 3]; 3],
    s2: [[[i128; 3]; 3]; 3],
    lo: [i64; 3],
    hi: [i64; 3],
}

/// The [`Section`] of every joint of `graph`, in the order of [`PieceGraph::edges`], for cells of `size` along the axes of the lattice. The sums are exact
/// integers (the products of the doubled centres of the faces, taken from the first face of the joint) that are converted to floats once, so a joint far from
/// the origin has the moments of one near it. `occupancy` is the body that `graph` cuts: an error if it is not, or if `size` is not positive and finite.
pub fn sections(graph: &PieceGraph, occupancy: &Occupancy, size: [f64; 3]) -> Result<Vec<Section>, String> {
    if !size.iter().all(|c| c.is_finite() && *c > 0.0) {
        return Err(format!("a section needs a positive finite cell size, and {size:?} is not"));
    }
    if graph.owner.len() as u64 != occupancy.count() {
        return Err("the graph does not cut this body: it has another number of cells".into());
    }
    let empty = SectionSums {
        anchor: None,
        n: [0; 3],
        s1: [[0; 3]; 3],
        s2: [[[0; 3]; 3]; 3],
        lo: [i64::MAX; 3],
        hi: [i64::MIN; 3],
    };
    let mut sums = vec![empty; graph.edges.len()];
    for c in occupancy.cells() {
        let Some(mine) = graph.piece_of(c) else {
            return Err(format!("the cell {c:?} of the body is in no piece of the graph"));
        };
        for axis in 0..3 {
            let mut next = c;
            next[axis] += 1;
            let Some(theirs) = graph.piece_of(next) else { continue };
            if theirs == mine {
                continue;
            }
            let key = (mine.min(theirs), mine.max(theirs));
            let Ok(at) = graph.edges.binary_search_by_key(&key, |e| (e.a, e.b)) else {
                return Err(format!("the faces of {c:?} make a joint {key:?} that the graph does not have"));
            };
            let acc = &mut sums[at];
            let mut u = doubled(c);
            u[axis] += 1;
            let anchor = *acc.anchor.get_or_insert(u);
            let d: [i128; 3] = std::array::from_fn(|j| i128::from(u[j]) - i128::from(anchor[j]));
            acc.n[axis] += 1;
            for j in 0..3 {
                acc.s1[axis][j] += d[j];
                for l in 0..3 {
                    acc.s2[axis][j][l] += d[j] * d[l];
                }
                // the corners of a face: its plane on the axis it is normal to, and a unit either side of its centre on the two others
                let reach = i64::from(j != axis);
                acc.lo[j] = acc.lo[j].min(u[j] - reach);
                acc.hi[j] = acc.hi[j].max(u[j] + reach);
            }
        }
    }
    let area_of = |k: usize| size[(k + 1) % 3] * size[(k + 2) % 3];
    let mut out = Vec::with_capacity(sums.len());
    for (edge, acc) in graph.edges.iter().zip(&sums) {
        for k in 0..3 {
            if u64::from(edge.faces[k]) != acc.n[k] {
                return Err(format!(
                    "the joint {}-{} has {} faces normal to axis {k} and the cells have {}",
                    edge.a, edge.b, edge.faces[k], acc.n[k]
                ));
            }
        }
        let Some(anchor) = acc.anchor else { return Err(format!("the joint {}-{} has no face", edge.a, edge.b)) };
        let area: f64 = (0..3).map(|k| acc.n[k] as f64 * area_of(k)).sum();
        // from the anchor, in length: a doubled coordinate is half a cell
        let half = size.map(|c| c * 0.5);
        let first: [f64; 3] =
            std::array::from_fn(|j| (0..3).map(|k| area_of(k) * acc.s1[k][j] as f64).sum::<f64>() * half[j]);
        let centre: [f64; 3] = first.map(|v| v / area);
        let mut second = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                let mut sum = 0.0;
                for k in 0..3 {
                    sum += area_of(k) * (acc.s2[k][i][j] as f64 * half[i] * half[j]);
                    if i == j && i != k {
                        // the faces' own spread: a rectangle of this size along the axis has size^2 / 12 about its centre
                        sum += area_of(k) * acc.n[k] as f64 * size[i] * size[i] / 12.0;
                    }
                }
                second[i][j] = sum - area * centre[i] * centre[j];
            }
        }
        let weighted: [f64; 3] = std::array::from_fn(|k| f64::from(edge.normal_sum[k]) * area_of(k));
        let norm = weighted.iter().map(|v| v * v).sum::<f64>().sqrt();
        let normal = (norm > 0.0).then(|| weighted.map(|v| v / norm));
        out.push(Section {
            area,
            centroid: std::array::from_fn(|j| anchor[j] as f64 * half[j] + centre[j]),
            second,
            normal,
            lo: std::array::from_fn(|j| acc.lo[j] as f64 * half[j]),
            hi: std::array::from_fn(|j| acc.hi[j] as f64 * half[j]),
        });
    }
    Ok(out)
}

/// A plane over the doubled coordinates `u`: a cell is on its positive side if `normal . u >= offset`.
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

const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// splitmix64 of `state`.
fn mix(state: u64) -> u64 {
    let mut z = state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn too_many_seeds(count: u64) -> String {
    format!("{count} seeds, and a partition takes at most {MAX_SEEDS}")
}

/// The doubled coordinates of the seeds `0..count` drawn from `seed` in the box `lo..=hi` (doubled): the axis `a` of seed `s` is
/// `lo + splitmix64(seed + (3 s + a + 1) * golden) % (hi - lo + 1)`. (The remainder is slightly biased toward the low end, by at most one part
/// in 2^22 over the size of the box: it is a cut, not a sample.) The box must be right way up and within [`MAX_SEED_COORD`] of the origin, and
/// `count` at most [`MAX_SEEDS`].
pub fn seeds_in(count: u32, seed: u64, lo: [i64; 3], hi: [i64; 3]) -> Result<Vec<[i64; 3]>, String> {
    if count > MAX_SEEDS {
        return Err(too_many_seeds(u64::from(count)));
    }
    if (0..3).any(|a| {
        lo[a] > hi[a] || lo[a].unsigned_abs() > MAX_SEED_COORD as u64 || hi[a].unsigned_abs() > MAX_SEED_COORD as u64
    }) {
        return Err(format!(
            "the box {lo:?} to {hi:?} of the seeds is upside down or farther than {MAX_SEED_COORD} from the origin"
        ));
    }
    Ok((0..u64::from(count))
        .map(|s| {
            std::array::from_fn(|a| {
                let range = (hi[a] - lo[a] + 1) as u64;
                let h = mix(seed.wrapping_add((3 * s + a as u64 + 1).wrapping_mul(GOLDEN)));
                lo[a] + (h % range) as i64
            })
        })
        .collect())
}

fn doubled(c: [i32; 3]) -> [i64; 3] {
    c.map(|k| 2 * i64::from(k) + 1)
}

fn scan_key(c: &[i32; 3]) -> (i32, i32, i32) {
    (c[2], c[1], c[0])
}

/// The index of the seed nearest to `u`, the lowest index on a tie. The seeds are within `MAX_SEED_COORD` and the cells within 2^31 of the
/// origin, so the differences are within 2^42 and their squares and sum are far inside an `i128`.
fn nearest(seeds: &[[i64; 3]], u: [i64; 3]) -> u32 {
    let mut best = (i128::MAX, 0u32);
    for (i, s) in seeds.iter().enumerate() {
        let d: i128 = (0..3).map(|a| i128::from(u[a]) - i128::from(s[a])).map(|d| d * d).sum();
        if d < best.0 {
            best = (d, i as u32);
        }
    }
    best.1
}

fn check_seeds(seeds: &[[i64; 3]]) -> Result<(), String> {
    if seeds.is_empty() {
        return Err("a Voronoi partition needs at least one seed".into());
    }
    if seeds.len() > MAX_SEEDS as usize {
        return Err(too_many_seeds(seeds.len() as u64));
    }
    if let Some(s) = seeds.iter().find(|s| s.iter().any(|v| v.unsigned_abs() > MAX_SEED_COORD as u64)) {
        return Err(format!("the seed {s:?} is farther than {MAX_SEED_COORD} from the origin on an axis"));
    }
    Ok(())
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
                        // up to 3 products of an i64 and a number within 2^32: far inside an i128, and compared, not subtracted from the offset
                        let side: i128 = (0..3).map(|a| i128::from(p.normal[a]) * i128::from(u[a])).sum();
                        mask | (u64::from(side >= p.offset) << i)
                    })
                })
                .collect()
        }
        Partition::Voronoi { seeds, seed } => {
            if seeds == 0 {
                return Err("a Voronoi partition needs at least one seed".into());
            }
            let seeds = seeds_in(seeds, seed, doubled(min), doubled(max))?;
            cells.iter().map(|c| u64::from(nearest(&seeds, doubled(*c)))).collect()
        }
        Partition::VoronoiAt(seeds) => {
            check_seeds(seeds)?;
            cells.iter().map(|c| u64::from(nearest(seeds, doubled(*c)))).collect()
        }
    };
    // the cells of each label, then the components of each, then every component by its first cell in the scan
    let mut parts: BTreeMap<u64, Vec<[i32; 3]>> = BTreeMap::new();
    for (c, l) in cells.iter().zip(&labels) {
        parts.entry(*l).or_default().push(*c);
    }
    let mut found: Vec<Vec<[i32; 3]>> = parts.values().flat_map(|part| components(part)).collect();
    if found.len() > max_pieces {
        return Err(format!("the partition makes {} pieces and the limit is {max_pieces} pieces", found.len()));
    }
    found.sort_by_key(|p| scan_key(&p[0]));
    let mut owner: Vec<([i32; 3], u32)> =
        found.iter().enumerate().flat_map(|(i, p)| p.iter().map(move |c| (*c, i as u32))).collect();
    owner.sort_by_key(|(c, _)| scan_key(c));
    let pieces: Vec<Piece> =
        found.into_iter().map(|cells| Piece { moments: Moments::of(cells.iter().copied()), cells }).collect();
    let piece_at = |cell: [i32; 3]| -> Option<u32> {
        owner.binary_search_by_key(&scan_key(&cell), |(c, _)| scan_key(c)).ok().map(|i| owner[i].1)
    };
    // the joints: every face between a cell and its neighbour one step up an axis, once, in the order of the scan
    let mut joints: BTreeMap<(u32, u32), Edge> = BTreeMap::new();
    for (c, mine) in &owner {
        for axis in 0..3 {
            let mut next = *c;
            next[axis] += 1;
            let Some(theirs) = piece_at(next) else { continue };
            if theirs == *mine {
                continue;
            }
            let (a, b) = ((*mine).min(theirs), (*mine).max(theirs));
            let e =
                joints.entry((a, b)).or_insert(Edge { a, b, faces: [0; 3], face_sum: [[0; 3]; 3], normal_sum: [0; 3] });
            e.faces[axis] = e.faces[axis].checked_add(1).ok_or("a joint has more faces than a u32 counts")?;
            let u = doubled(*c);
            for (k, sum) in e.face_sum[axis].iter_mut().enumerate() {
                let centre = u[k] + i64::from(k == axis);
                *sum = sum.checked_add(centre).ok_or("the sum of the centres of the faces of a joint overflows")?;
            }
            // the unit normal from a to b: up the axis if the lower cell is in a, down it if it is in b
            e.normal_sum[axis] = e.normal_sum[axis]
                .checked_add(if *mine == a { 1 } else { -1 })
                .ok_or("the sum of the normals of the faces of a joint overflows")?;
        }
    }
    Ok(PieceGraph { pieces, edges: joints.into_values().collect(), owner })
}
