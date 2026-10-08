//! The exposed faces of a grid, read brick by brick.

use super::Classes;
use crate::occupancy::{Occupancy, BRICK};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::ControlFlow;

/// A face of a cell that is exposed: on the plane `plane` along `axis` (facing `+axis` when `positive`), at `u`, `v` of the two other
/// coordinates in cyclic order, of the class of the cell that owns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Face {
    pub axis: u8,
    pub positive: bool,
    pub plane: i32,
    pub u: i32,
    pub v: i32,
    pub class: u8,
}

/// Every exposed face, in the order (axis, facing, plane, v, u).
///
/// The grid is read a plane of the lattice at a time: the two layers of cells on either side of it are the bricks of one or two slabs,
/// and a pair of bricks (the one on each side, an absent one being empty) gives the 64 faces of that plane in their span, so nothing
/// is looked up cell by cell and the cost is that of the bricks present.
pub fn exposed_faces(grid: &Occupancy, classes: &Classes) -> Vec<Face> {
    let mut out = Vec::new();
    for_each_plane(grid, classes, |axis, plane, minus, plus| {
        for (list, positive) in [(minus, false), (plus, true)] {
            for &(u, v, class) in list {
                out.push(Face { axis, positive, plane, u, v, class });
            }
        }
        ControlFlow::Continue(())
    });
    out.sort_by_key(|f| (f.axis, f.positive, f.plane, f.v, f.u));
    out
}

/// The bricks of a grid by their key.
struct Bricks<'a> {
    by_key: HashMap<[i32; 3], &'a [u8; 512]>,
}

impl<'a> Bricks<'a> {
    fn of(grid: &'a Occupancy) -> Self {
        Self { by_key: grid.bricks().collect() }
    }

    /// The brick with `along` of its key along `axis` and `u`, `v` of the other two (in cyclic order), if it holds anything.
    fn at(&self, axis: usize, along: i32, bu: i32, bv: i32) -> Option<&'a [u8; 512]> {
        let mut key = [0; 3];
        key[axis] = along;
        key[(axis + 1) % 3] = bu;
        key[(axis + 2) % 3] = bv;
        self.by_key.get(&key).copied()
    }
}

/// A grid read for its surface: its bricks by key and, along each axis, the columns of bricks of each slab.
pub(super) struct Index<'a> {
    bricks: Bricks<'a>,
    /// For each axis, the bricks of each slab (their key along the axis) by the key of the other two in cyclic order.
    slabs: [BTreeMap<i32, BTreeSet<(i32, i32)>>; 3],
}

impl<'a> Index<'a> {
    pub(super) fn of(grid: &'a Occupancy) -> Self {
        let bricks = Bricks::of(grid);
        let mut slabs: [BTreeMap<i32, BTreeSet<(i32, i32)>>; 3] = Default::default();
        for key in bricks.by_key.keys() {
            for (axis, slab) in slabs.iter_mut().enumerate() {
                slab.entry(key[axis]).or_default().insert((key[(axis + 1) % 3], key[(axis + 2) % 3]));
            }
        }
        Self { bricks, slabs }
    }

    /// The planes of the lattice along `axis` that have a layer of a brick on either side, ascending: the nine from the low plane of a slab
    /// to its high one.
    pub(super) fn planes(&self, axis: usize) -> Vec<i32> {
        let planes: BTreeSet<i32> = self.slabs[axis].keys().flat_map(|s| (s * BRICK)..=(s * BRICK + BRICK)).collect();
        planes.into_iter().collect()
    }

    /// The faces of the plane `plane` along `axis` as `(minus, plus)`: those that look toward `-axis` (owned by the layer above the
    /// plane) and toward `+axis` (owned by the layer below), as `(u, v, class)`, in no particular order.
    pub(super) fn plane_faces(&self, axis: usize, plane: i32, classes: &Classes) -> (Vec<Faces>, Vec<Faces>) {
        let (below, above) = (plane - 1, plane);
        let (slab_below, slab_above) = (below.div_euclid(BRICK), above.div_euclid(BRICK));
        let (layer_below, layer_above) = (below.rem_euclid(BRICK) as usize, above.rem_euclid(BRICK) as usize);
        let columns: BTreeSet<(i32, i32)> = [slab_below, slab_above]
            .iter()
            .filter_map(|s| self.slabs[axis].get(s))
            .flat_map(|c| c.iter().copied())
            .collect();
        let (mut minus, mut plus) = (Vec::new(), Vec::new());
        for (bu, bv) in columns {
            let lo = self.bricks.at(axis, slab_below, bu, bv);
            let hi = self.bricks.at(axis, slab_above, bu, bv);
            for lv in 0..BRICK as usize {
                for lu in 0..BRICK as usize {
                    let cell = |brick: Option<&[u8; 512]>, layer: usize| {
                        let mut local = [0usize; 3];
                        local[axis] = layer;
                        local[(axis + 1) % 3] = lu;
                        local[(axis + 2) % 3] = lv;
                        brick.map_or(0, |b| classes.class(b[local[0] + 8 * (local[1] + 8 * local[2])]))
                    };
                    let (a, b) = (cell(lo, layer_below), cell(hi, layer_above));
                    let (u, v) = (bu * BRICK + lu as i32, bv * BRICK + lv as i32);
                    if classes.exposes(a, b) {
                        plus.push((u, v, a));
                    }
                    if classes.exposes(b, a) {
                        minus.push((u, v, b));
                    }
                }
            }
        }
        (minus, plus)
    }
}

/// One face of a plane: `(u, v, class)`.
pub(super) type Faces = (i32, i32, u8);

/// Calls `f(axis, plane, minus, plus)` for every plane of the lattice that has a face (by axis and then by plane, ascending) with the
/// faces of [`Index::plane_faces`], until `f` says to stop.
pub(super) fn for_each_plane(
    grid: &Occupancy,
    classes: &Classes,
    mut f: impl FnMut(u8, i32, &[Faces], &[Faces]) -> ControlFlow<()>,
) {
    let index = Index::of(grid);
    for axis in 0..3usize {
        for plane in index.planes(axis) {
            let (minus, plus) = index.plane_faces(axis, plane, classes);
            if (!minus.is_empty() || !plus.is_empty()) && f(axis as u8, plane, &minus, &plus).is_break() {
                return;
            }
        }
    }
}
