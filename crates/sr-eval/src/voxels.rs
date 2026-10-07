//! Bodies of cells for the rigid world, and what becomes of one when cells are taken out of it.
//!
//! Pure functions of an [`Occupancy`]: the evaluator wires them to the scene when the schema has the attributes for it. A cut says, from the
//! cells that are destroyed, which parts of what is left stay in the body, which separate into the slots reserved for them
//! ([`sr_sim::physics3d::VoxelSplit3`]) and which are too small to be bodies (dust, whose cells leave the body too). Every part is a union
//! of the connected components of [`sr_3d::occupancy::components`], so the result is the same for the same cells whatever the order they were put in,
//! and every cell and every kilogram of the body is in exactly one part.

use sr_3d::occupancy::{components, Occupancy};
use sr_sim::physics3d::{Shape3, VoxelCut3, VoxelPiece3};
use std::collections::BTreeSet;

/// A body of cells ready for the world: its shape (the cells in the order of the scan) and its mass in kilograms.
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    pub shape: Shape3,
    pub mass: f64,
}

/// Kilograms of one cell of `size` scene units a side, a metre being `pixels_per_meter` of them, of `density` kilograms a cubic metre.
fn cell_mass(size: [f64; 3], density: f64, pixels_per_meter: f64) -> Result<f64, String> {
    if !size.iter().all(|s| s.is_finite() && *s > 0.0)
        || !(density.is_finite() && density > 0.0)
        || !(pixels_per_meter.is_finite() && pixels_per_meter > 0.0)
    {
        return Err("a body of cells needs a positive cell size, density and scale".into());
    }
    Ok(density * size[0] * size[1] * size[2] / pixels_per_meter.powi(3))
}

/// The body of the cells of `occupancy`: mass is the number of cells times the mass of one.
pub fn body(occupancy: &Occupancy, size: [f64; 3], density: f64, pixels_per_meter: f64) -> Result<Body, String> {
    let one = cell_mass(size, density, pixels_per_meter)?;
    if occupancy.count() == 0 {
        return Err("a body of cells needs at least one cell".into());
    }
    Ok(Body {
        shape: Shape3::Voxels { size, cells: occupancy.cells().collect() },
        mass: occupancy.count() as f64 * one,
    })
}

/// Which part of what is left stays in the body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stay {
    /// The part with most cells (the first in the scan if equal): the body of a free object.
    Largest,
    /// Every part that has a cell for which the anchor is true (terrain held by what is under it): the others come loose.
    Anchored,
}

/// What to do with more loose parts than slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overflow {
    Error,
    /// The smallest become dust (the first of equals stays a body).
    Dust,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub stay: Stay,
    /// Loose parts with fewer cells than this are dust, not bodies.
    pub min_cells: usize,
    /// Slots there are for pieces.
    pub max_fragments: usize,
    pub overflow: Overflow,
}

/// The result of a cut, in the scan order of the cells.
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    /// What the world is told. Its `destroyed` holds the cells that were destroyed and those of the dust: all that leaves the body and
    /// is no body.
    pub cut: VoxelCut3,
    /// The cells that stay in the body.
    pub stays: Vec<[i32; 3]>,
    /// The cells of the loose parts that are not bodies (also in `cut.destroyed`).
    pub dust: Vec<[i32; 3]>,
}

fn scan(c: &[i32; 3]) -> (i32, i32, i32) {
    (c[2], c[1], c[0])
}

/// Takes `destroyed` out of the cells of `before` (those of them that are in it; the revision is the world's name for the result)
/// and divides what is left.
#[allow(clippy::too_many_arguments)]
pub fn cut(
    before: &Occupancy,
    destroyed: &[[i32; 3]],
    revision: u64,
    size: [f64; 3],
    density: f64,
    pixels_per_meter: f64,
    policy: &Policy,
    anchored: &dyn Fn(&[i32; 3]) -> bool,
) -> Result<Cut, String> {
    let one = cell_mass(size, density, pixels_per_meter)?;
    let gone: BTreeSet<(i32, i32, i32)> = destroyed.iter().filter(|c| before.get(**c) != 0).map(scan).collect();
    let remaining: Vec<[i32; 3]> = before.cells().filter(|c| !gone.contains(&scan(c))).collect();
    let parts = components(&remaining);
    // which stay
    let staying: BTreeSet<usize> = match policy.stay {
        Stay::Largest => parts
            .iter()
            .enumerate()
            .max_by_key(|(i, p)| (p.len(), std::cmp::Reverse(*i)))
            .map(|(i, _)| i)
            .into_iter()
            .collect(),
        Stay::Anchored => parts.iter().enumerate().filter(|(_, p)| p.iter().any(anchored)).map(|(i, _)| i).collect(),
    };
    // the loose ones: bodies, or dust
    let loose: Vec<usize> = (0..parts.len()).filter(|i| !staying.contains(i)).collect();
    let (bodies, small): (Vec<usize>, Vec<usize>) =
        loose.into_iter().partition(|i| parts[*i].len() >= policy.min_cells);
    let mut dust_parts = small;
    let mut pieces = bodies;
    if pieces.len() > policy.max_fragments {
        if policy.overflow == Overflow::Error {
            return Err(format!(
                "a cut makes {} loose parts and there are {} slots for them (maxFragments)",
                pieces.len(),
                policy.max_fragments
            ));
        }
        // the largest keep their slots, the first of equals first; the rest are dust
        let mut by_size = pieces.clone();
        by_size.sort_by_key(|i| (std::cmp::Reverse(parts[*i].len()), *i));
        let kept: BTreeSet<usize> = by_size.into_iter().take(policy.max_fragments).collect();
        dust_parts.extend(pieces.iter().filter(|i| !kept.contains(i)));
        pieces.retain(|i| kept.contains(i));
    }
    let mut dust: Vec<[i32; 3]> = dust_parts.iter().flat_map(|i| parts[*i].iter().copied()).collect();
    dust.sort_by_key(scan);
    let mut stays: Vec<[i32; 3]> = staying.iter().flat_map(|i| parts[*i].iter().copied()).collect();
    stays.sort_by_key(scan);
    let mut leaving: Vec<[i32; 3]> = gone.iter().map(|&(z, y, x)| [x, y, z]).chain(dust.iter().copied()).collect();
    leaving.sort_by_key(scan);
    let pieces: Vec<VoxelPiece3> =
        pieces.iter().map(|i| VoxelPiece3 { cells: parts[*i].clone(), mass: parts[*i].len() as f64 * one }).collect();
    Ok(Cut {
        cut: VoxelCut3 { revision, destroyed: leaving, parent_mass: stays.len() as f64 * one, pieces },
        stays,
        dust,
    })
}
