//! The exposed faces of a grid merged into quads, plane by plane.

use super::exposure::for_each_plane;
use super::Classes;
use crate::occupancy::Occupancy;
use std::collections::HashMap;
use std::ops::ControlFlow;

/// The most cells a quad spans along either side: what a `u16` can say (the compact list of quads keeps them in 16 bits).
const MOST: i32 = u16::MAX as i32;

/// A rectangle of exposed faces of one class on one plane: the faces `u0..u0 + w` by `v0..v0 + h` of the plane `plane` along `axis`,
/// facing `+axis` when `positive` (the coordinates are those of [`super::Face`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quad {
    pub axis: u8,
    pub positive: bool,
    pub plane: i32,
    pub u0: i32,
    pub v0: i32,
    pub w: u32,
    pub h: u32,
    pub class: u8,
}

/// The surface of a grid as quads, in the order (axis, facing, plane, `v0`, `u0`).
///
/// The faces of a plane that look the same way are merged greedily: scanning by `v` and then by `u`, the first face not yet taken
/// starts a quad that takes the widest run of faces of its class along `u`, and then grows along `v` for as long as the whole row under
/// it is of that class and not taken. The scan order is the whole of the rule (another order merges a different set of quads out of
/// the same faces), so it is fixed here and by the hashes the tests state. A quad is cut where a side would pass 65,535 cells.
pub fn mesh_quads(grid: &Occupancy, classes: &Classes) -> Vec<Quad> {
    mesh_quads_within(grid, classes, usize::MAX).expect("a surface with no budget to exceed")
}

/// What a quad costs at the most, in bytes: the 16 of the compact list, the 408 of its four vertices and six indices in the builder, the
/// 408 of the copy a mesh upload keeps on the host and the 408 of the buffers on the device. A surface budget in bytes admits that
/// many quads divided by this.
pub const BYTES_PER_QUAD: usize = 16 + 3 * 408;

/// [`mesh_quads`] under a budget of `max_bytes` (the object's `surfaceMemoryMiB`): the quads are counted as the planes are merged, in
/// a fixed order, and the first plane that takes the surface over the budget stops the work with an error that says how many quads the
/// budget admits, so a surface that cannot fit costs the time of the quads it was allowed and the error does not depend on how many
/// there would have been. Nothing is returned for a surface that does not fit, not even a part.
pub fn mesh_quads_within(grid: &Occupancy, classes: &Classes, max_bytes: usize) -> Result<Vec<Quad>, String> {
    let admitted = max_bytes / BYTES_PER_QUAD;
    let mut quads = Vec::new();
    let mut over = false;
    for_each_plane(grid, classes, |axis, plane, minus, plus| {
        merge(axis, false, plane, minus, &mut quads);
        merge(axis, true, plane, plus, &mut quads);
        if quads.len() > admitted {
            over = true;
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });
    if over {
        return Err(format!(
            "voxel surface exceeds memory budget (surfaceMemoryMiB): its quads cost {BYTES_PER_QUAD} bytes each at the peak and the budget of {max_bytes} bytes admits {admitted} quads, and the surface has more"
        ));
    }
    quads.sort_by_key(|q| (q.axis, q.positive, q.plane, q.v0, q.u0));
    Ok(quads)
}

pub(super) fn merge(axis: u8, positive: bool, plane: i32, faces: &[(i32, i32, u8)], out: &mut Vec<Quad>) {
    // the faces not yet taken, by place; a taken face is removed
    let mut open: HashMap<(i32, i32), u8> = faces.iter().map(|&(u, v, class)| ((u, v), class)).collect();
    let mut scan: Vec<(i32, i32)> = open.keys().copied().collect();
    scan.sort_unstable_by_key(|&(u, v)| (v, u));
    for (u, v) in scan {
        let Some(&class) = open.get(&(u, v)) else { continue };
        let mut w = 1;
        while w < MOST && open.get(&(u + w, v)) == Some(&class) {
            w += 1;
        }
        let mut h = 1;
        while h < MOST && (0..w).all(|d| open.get(&(u + d, v + h)) == Some(&class)) {
            h += 1;
        }
        for e in 0..h {
            for d in 0..w {
                open.remove(&(u + d, v + e));
            }
        }
        out.push(Quad { axis, positive, plane, u0: u, v0: v, w: w as u32, h: h as u32, class });
    }
}

/// The FNV-1a hash (64 bits) of a list of quads, each as its axis (u8), facing (u8, 0 for `-axis`, 1 for `+axis`), plane (i32), `u0`
/// (i32), `v0` (i32), `w` (u16), `h` (u16) and class (u8), little endian, in the order of the list: the number that
/// `tools/voxel_reference_mesher.py` prints for the same cells, and that the tests fix.
pub fn quads_hash(quads: &[Quad]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut byte = |b: u8| {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for q in quads {
        byte(q.axis);
        byte(u8::from(q.positive));
        for b in q.plane.to_le_bytes().into_iter().chain(q.u0.to_le_bytes()).chain(q.v0.to_le_bytes()) {
            byte(b);
        }
        for b in (q.w as u16).to_le_bytes().into_iter().chain((q.h as u16).to_le_bytes()) {
            byte(b);
        }
        byte(q.class);
    }
    hash
}
