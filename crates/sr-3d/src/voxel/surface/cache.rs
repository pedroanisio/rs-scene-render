//! The surface of a grid kept between frames, remeshing only the planes that an edit can have changed.

use super::exposure::Index;
use super::greedy::merge;
use super::{Classes, Quad, BYTES_PER_QUAD};
use crate::occupancy::{Occupancy, BRICK};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// What an update did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Remesh {
    /// The planes (of any axis) that were meshed again.
    pub remeshed: usize,
    /// The whole surface was made again, whatever changed.
    pub full: bool,
}

/// The quads of one plane that look one way, in the order (v0, u0).
type Slice = Arc<[Quad]>;

/// The surface of a grid, by plane.
///
/// What a plane's quads are is a function of the cells of the layers on either side of it and of nothing else, so an edit of a brick
/// can change the nine planes of its slab along each axis and no others: they are the only ones meshed again, and the rest keep the quads
/// they had (the same allocations). The cache knows which grid it holds by its [`lineage`](Occupancy::lineage) and its revision, and
/// which classes it was made with, and makes the whole surface again when any of them is not the one it was given: another grid, a
/// revision of the same grid that is not past the one it holds (it cannot tell what changed), other classes. It reads the bricks
/// changed since its revision from the grid, so the grid must not be compacted past the revision of the cache before the cache has been
/// brought up to date (`compact(revision)` after the update is the order).
#[derive(Default)]
pub struct SurfaceCache {
    held: Option<(u64, u64, Classes)>,
    slices: BTreeMap<(u8, bool, i32), Slice>,
}

impl SurfaceCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Brings the surface to the state of `grid` under `classes`, within `max_bytes` of surface memory (the budget of
    /// [`mesh_quads_within`](super::mesh_quads_within)). A surface over the budget is an error and the cache forgets what it held: it
    /// would not be a surface of the grid.
    pub fn update(&mut self, grid: &Occupancy, classes: &Classes, max_bytes: usize) -> Result<Remesh, String> {
        let ident = (grid.lineage(), grid.revision());
        let revision = self.held.as_ref().map_or(0, |h| h.1);
        let dirty = matches!(&self.held, Some((lineage, held, _)) if *lineage == ident.0 && *held <= ident.1)
            .then(|| grid.changed_bricks_since(revision));
        self.apply(grid, classes, max_bytes, ident, dirty)
    }

    /// As [`SurfaceCache::update`] for a grid whose owner names it: `key` is `(body, revision)` as the owner counts them (a revision that
    /// only goes up for a body, with the same cells whenever it is asked for) and `changed` the keys of the bricks that changed between the
    /// revision the cache last read and this one, as the owner says (`None` when it no longer has that history). `grid` may be any
    /// copy of the cells of the revision (the lineage of a copy is its own, so it is not used). Another body, a revision behind the one held,
    /// other classes or no list is a full remesh from `grid`; the same key is nothing to do.
    pub fn update_known(
        &mut self,
        grid: &Occupancy,
        classes: &Classes,
        max_bytes: usize,
        key: (u64, u64),
        changed: Option<&[[i32; 3]]>,
    ) -> Result<Remesh, String> {
        let continues = matches!(&self.held, Some((body, held, _)) if *body == key.0 && *held <= key.1);
        let dirty = if continues { changed.map(<[_]>::to_vec) } else { None };
        self.apply(grid, classes, max_bytes, key, dirty)
    }

    /// The work of both: `ident` is what the cache will hold, `dirty` the bricks to take up (none: from the start).
    fn apply(
        &mut self,
        grid: &Occupancy,
        classes: &Classes,
        max_bytes: usize,
        ident: (u64, u64),
        dirty: Option<Vec<[i32; 3]>>,
    ) -> Result<Remesh, String> {
        let same_classes = matches!(&self.held, Some((_, _, held)) if held == classes);
        let incremental = dirty.is_some() && same_classes;
        if incremental && matches!(&self.held, Some((_, held, _)) if *held == ident.1) {
            return Ok(Remesh { remeshed: 0, full: false });
        }
        let index = Index::of(grid);
        let dirty: Option<BTreeSet<(usize, i32)>> = dirty.filter(|_| incremental).map(|bricks| {
            bricks
                .iter()
                .flat_map(|k| (0..3).flat_map(move |a| (0..=BRICK).map(move |i| (a, BRICK * k[a] + i))))
                .collect()
        });
        // more than half of the planes to do again is the whole of it
        let all: usize = (0..3).map(|a| index.planes(a).len()).sum();
        let dirty = dirty.filter(|d| 2 * d.len() <= all);
        let full = dirty.is_none();
        let remeshed = match &dirty {
            Some(dirty) => {
                for (axis, plane) in dirty {
                    self.remesh(&index, *axis, *plane, classes);
                }
                dirty.len()
            }
            None => {
                self.slices.clear();
                for axis in 0..3 {
                    for plane in index.planes(axis) {
                        self.remesh(&index, axis, plane, classes);
                    }
                }
                all
            }
        };
        let quads: usize = self.slices.values().map(|s| s.len()).sum();
        if quads > max_bytes / BYTES_PER_QUAD {
            *self = Self::default();
            return Err(format!(
                "voxel surface exceeds memory budget (surfaceMemoryMiB): its quads cost {BYTES_PER_QUAD} bytes each at the peak and the budget of {max_bytes} bytes admits {} quads, and the surface has more",
                max_bytes / BYTES_PER_QUAD
            ));
        }
        self.held = Some((ident.0, ident.1, classes.clone()));
        Ok(Remesh { remeshed, full })
    }

    fn remesh(&mut self, index: &Index, axis: usize, plane: i32, classes: &Classes) {
        let (minus, plus) = index.plane_faces(axis, plane, classes);
        for (faces, positive) in [(minus, false), (plus, true)] {
            let mut quads = Vec::new();
            merge(axis as u8, positive, plane, &faces, &mut quads);
            quads.sort_by_key(|q| (q.v0, q.u0));
            let key = (axis as u8, positive, plane);
            if quads.is_empty() {
                self.slices.remove(&key);
            } else {
                self.slices.insert(key, quads.into());
            }
        }
    }

    /// The quads of the surface in the canonical order: by axis, facing, plane, then `v0` and `u0`.
    pub fn quads(&self) -> Vec<Quad> {
        self.slices.values().flat_map(|s| s.iter().copied()).collect()
    }
}
