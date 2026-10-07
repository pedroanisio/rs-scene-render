//! Bodies of cells for the rigid world, and what becomes of one when cells are taken out of it.
//!
//! Pure functions of an [`Occupancy`]: the evaluator wires them to the scene when the schema has the attributes for it. A cut says, from the
//! cells that are destroyed, which parts of what is left stay in the body, which separate into the slots reserved for them
//! ([`sr_sim::physics3d::VoxelSplit3`]) and which are too small to be bodies (dust, whose cells leave the body too). Every part is a union
//! of the connected components of [`sr_3d::occupancy::components`], so the result is the same for the same cells whatever the order they were put in,
//! and every cell and every kilogram of the body is in exactly one part.

use sr_3d::occupancy::{components, Moments, Occupancy};
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
    if !(size.iter().all(|s| s.is_finite() && *s > 0.0)
        && density.is_finite()
        && density > 0.0
        && pixels_per_meter.is_finite()
        && pixels_per_meter > 0.0)
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
        cut: VoxelCut3 { added: vec![], revision, destroyed: leaving, parent_mass: stays.len() as f64 * one, pieces },
        stays,
        dust,
    })
}

/// What a fracture does with the pieces that are small or too many: the cut's own rules (see [`Policy`]), without the part that stays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FracturePolicy {
    /// Pieces with fewer cells than this are dust, not bodies.
    pub min_cells: usize,
    /// Slots there are for pieces.
    pub max_fragments: usize,
    pub overflow: Overflow,
}

/// A body of cells divided into pieces that are bodies of cells: the source and the pieces that take its place, with the joints between them.
#[derive(Clone, Debug, PartialEq)]
pub struct Fractured {
    /// The body before it breaks: all its cells and all their mass, the dust's included, which is what it weighs until it breaks.
    pub source: Body,
    /// The pieces that are bodies, in the order of [`sr_3d::pieces::partition`], each a body of its own cells (keys of the source's lattice) and their mass.
    pub pieces: Vec<Body>,
    /// For each of `pieces`, its index in `graph`.
    pub piece_ids: Vec<usize>,
    /// The cells of the pieces that are not bodies (too small, or the smallest of too many), in the order of the scan: they leave with the fracture.
    pub dust: Vec<[i32; 3]>,
    /// The dust as the world is told of it (`Fracture3::dust`): its mass, its centre of mass in the source's axes and units, its tensor. None without dust.
    pub dust_body: Option<sr_sim::physics3d::Dust3>,
    /// All the pieces of the partition, dust included, and the faces they share.
    pub graph: sr_3d::pieces::PieceGraph,
}

/// Divides the body of `occupancy` by `rule` (see [`sr_3d::pieces::partition`]) into pieces that are bodies of cells of the same lattice, each with the
/// mass of its cells, and the source with all of them. Every cell is in exactly one piece or in the dust, the pieces are in the order of the partition
/// (by their first cell in the scan), and the result is the same for the same cells in any order.
///
/// The pieces are sorted out as the cut sorts the loose parts: those with fewer than `policy.min_cells` cells are dust (their cells are no body and leave
/// with the fracture); of the rest, up to `policy.max_fragments` are bodies, and if there are more the policy says whether that is an error or the
/// smallest are dust too (the largest keep their slots, of equals the first). The source weighs all its cells until it breaks; the fragments and the
/// dust (`dust_body`, which goes to `Fracture3::dust`) sum to it, and what the dust takes with it is recorded by the world as lost.
pub fn fracture(
    occupancy: &Occupancy,
    rule: sr_3d::pieces::Partition,
    policy: &FracturePolicy,
    size: [f64; 3],
    density: f64,
    pixels_per_meter: f64,
) -> Result<Fractured, String> {
    let one = cell_mass(size, density, pixels_per_meter)?;
    if occupancy.count() == 0 {
        return Err("a body of cells needs at least one cell".into());
    }
    let source = Body {
        shape: Shape3::Voxels { size, cells: occupancy.cells().collect() },
        mass: occupancy.count() as f64 * one,
    };
    // the partition's own cap is the most the world takes as fragments of one event
    let graph = sr_3d::pieces::partition(occupancy, rule, MAX_PIECES)?;
    let all = graph.pieces();
    let (mut kept, mut dust_parts): (Vec<usize>, Vec<usize>) =
        (0..all.len()).partition(|i| all[*i].cells().len() >= policy.min_cells);
    if kept.len() > policy.max_fragments {
        if policy.overflow == Overflow::Error {
            return Err(format!(
                "a fracture makes {} pieces and there are {} slots for them (maxFragments)",
                kept.len(),
                policy.max_fragments
            ));
        }
        let mut by_size = kept.clone();
        by_size.sort_by_key(|i| (std::cmp::Reverse(all[*i].cells().len()), *i));
        let slots: BTreeSet<usize> = by_size.into_iter().take(policy.max_fragments).collect();
        dust_parts.extend(kept.iter().filter(|i| !slots.contains(i)));
        kept.retain(|i| slots.contains(i));
    }
    if kept.is_empty() {
        return Err(
            "no piece of the fracture is a body: they are all too small for minCells or there are no slots".into()
        );
    }
    let mut dust: Vec<[i32; 3]> = dust_parts.iter().flat_map(|i| all[*i].cells().iter().copied()).collect();
    dust.sort_by_key(scan);
    let pieces: Vec<Body> = kept
        .iter()
        .map(|i| {
            let cells = all[*i].cells();
            Body { shape: Shape3::Voxels { size, cells: cells.to_vec() }, mass: cells.len() as f64 * one }
        })
        .collect();
    // the dust's own properties, exactly, in metres, and its centre in the scene's units as the offsets of the fragments are
    let dust_body = if dust.is_empty() {
        None
    } else {
        let metres = size.map(|c| c / pixels_per_meter);
        let q = Moments::of(dust.iter().copied()).properties(metres, density)?;
        Some(sr_sim::physics3d::Dust3 {
            mass: q.mass,
            centre: q.centre.map(|c| c * pixels_per_meter),
            inertia: q.inertia,
        })
    };
    Ok(Fractured { source, pieces, piece_ids: kept, dust, dust_body, graph })
}

/// The most pieces a partition may make: the world's own limit on the fragments of one event.
const MAX_PIECES: usize = 4096;

/// What a body of cells that breaks by stress needs besides its cells and the rule that cuts it into pieces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StressConfig {
    /// Pascals: the principal tension that breaks a joint.
    pub strength: f64,
    /// Loose parts of fewer cells than this are dust.
    pub min_cells: usize,
    /// More loose parts than slots: the smallest are dust (true), or it is an error (false).
    pub overflow_to_dust: bool,
}

/// The body of `occupancy` as pieces joined at the faces that they share, for the world to break where a load makes more stress than `config.strength` in a
/// joint ([`sr_sim::physics3d::World3::with_stress`]): `rule` cuts it into pieces ([`sr_3d::pieces::partition`]), each piece has the mass of its cells
/// (`density` kilograms a cubic metre, cells of `size` scene units, a metre being `pixels_per_meter` of them), its centre and second moment, and each joint has
/// the exact section of the faces that its two pieces share ([`sr_3d::pieces::sections`], in metres, turned from the axes of the lattice into the physics'). `parent` is
/// the body of the world that the cells are the shape of, which has to be the parent of a split with a slot for each piece that is to come loose. At most
/// [`sr_sim::physics3d::MAX_STRESS_PIECES`] pieces: the cuts of the body are worked out for every joint.
pub fn stress(
    occupancy: &Occupancy,
    rule: sr_3d::pieces::Partition,
    config: &StressConfig,
    parent: usize,
    size: [f64; 3],
    density: f64,
    pixels_per_meter: f64,
) -> Result<sr_sim::physics3d::Stress3, String> {
    use sr_sim::physics3d::{Stress3, StressJoint3, StressPiece3, MAX_STRESS_PIECES};
    if !(config.strength.is_finite() && config.strength > 0.0) {
        return Err("a body breaks by stress at a strength that is a positive number of pascals".into());
    }
    let one = cell_mass(size, density, pixels_per_meter)?;
    if occupancy.count() == 0 {
        return Err("a body of cells needs at least one cell".into());
    }
    let metres = size.map(|c| c / pixels_per_meter);
    let graph = sr_3d::pieces::partition(occupancy, rule, MAX_STRESS_PIECES)?;
    let sections = sr_3d::pieces::sections(&graph, occupancy, metres)?;
    let mut pieces = Vec::with_capacity(graph.pieces().len());
    for (i, piece) in graph.pieces().iter().enumerate() {
        // the world looks a cell up in the order of the keys, and the pieces list theirs in the order of the scan
        let mut cells = piece.cells().to_vec();
        cells.sort_unstable();
        pieces.push(
            StressPiece3::from_cells(&cells, metres, cells.len() as f64 * one)
                .ok_or_else(|| format!("the piece {i} has no mass or no cells"))?,
        );
    }
    let joints = graph
        .edges()
        .iter()
        .zip(&sections)
        .map(|(edge, s)| StressJoint3 {
            a: edge.a(),
            b: edge.b(),
            // a joint whose faces' normals cancel (a core inside a shell) has no direction: zero, which the world reads as none
            section: sr_sim::stress::JointSection::from_lattice(
                s.area,
                s.centroid,
                s.second,
                s.normal.unwrap_or([0.0; 3]),
                s.lo,
                s.hi,
            ),
        })
        .collect();
    Ok(Stress3 {
        parent,
        strength: config.strength,
        pieces,
        joints,
        min_cells: config.min_cells,
        overflow_to_dust: config.overflow_to_dust,
    })
}
