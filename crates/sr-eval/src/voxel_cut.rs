//! The cut that a crater makes in an object of cells, and what a frame says of the object and the pieces that have come away from it.
use crate::voxel_crater::{crater_cut, CraterCut, Ejection, Rock, SpeedLaw};
use crate::voxels::Stay;
use sr_3d::occupancy::Occupancy;
use std::sync::Arc;

/// Which part of what a crater's cut leaves stays the body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Largest,
    Base,
}

/// How a crater cuts an object of cells: the ground's cells, size and density, and which part stays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub rock: Rock,
    pub anchor: Anchor,
}

/// The cut that the crater `grown` makes in `before`: the conserving kernel of the law's crater with a bulking of 1 (the bowl holds the law's volume, the rim
/// the share that is not thrown), the speeds of the engine's launch model by quantiles of mass, and the settings' policy for the parts that are left loose.
pub fn crater_cut_of(
    grown: &crate::crater::ImpactCrater,
    before: &Occupancy,
    settings: &Settings,
    seed: u64,
) -> Result<CraterCut, String> {
    let cause = &grown.cause;
    let law = &cause.law;
    let cubic = grown.units.powi(3);
    // the contact point lies inside the ground by as far as the body went on before the contact was found (up to a rigid step of its motion), and the
    // crater is made at the surface: the plane of the kernel is where the axis through the point meets the cells' surface
    let cell = settings.rock.size[0] / settings.rock.pixels_per_meter * grown.units;
    let mut spec = grown.spec;
    spec.center = surface_along(before, spec.center, spec.outward, cell);
    let kernel = sr_3d::crater::Crater::conserving(
        spec,
        sr_3d::crater::Budget { volume: law.volume * cubic, ejecta: law.ejecta_volume * cubic, bulking: Some(1.0) },
    )?;
    let (angle, spread) = (EJECTA_ANGLE, EJECTA_SPREAD);
    let list = sr_sim::cratering::ejecta::ejecta(&sr_sim::cratering::ejecta::Spec {
        material: cause.material,
        body_radius: (3.0 * cause.impactor.mass / (4.0 * std::f64::consts::PI * cause.impactor.density)).cbrt(),
        body_density: cause.impactor.density,
        target_density: cause.target_density,
        impact_speed: cause.speed,
        velocity_direction: cause.velocity,
        normal: spec.outward,
        crater_volume: law.volume,
        crater_radius: law.radius,
        crater_duration: law.duration,
        particles: LAW_SAMPLES,
        seed,
        angle,
        angle_spread: spread,
    })?;
    let ejection =
        Ejection { share: law.ejecta_volume / law.volume, speeds: SpeedLaw::from_ejecta(&list)?, angle, spread, seed };
    let mut rock = settings.rock;
    rock.policy.stay = match settings.anchor {
        Anchor::Largest => Stay::Largest,
        Anchor::Base => Stay::Anchored,
    };
    // the base: the layer of cells with the greatest key along y (the scene's y points down), which the object stands on
    let base = before.bounds().map_or(i32::MAX, |(_, max)| max[1]);
    let anchored = move |c: &[i32; 3]| c[1] == base;
    crater_cut(before, &kernel, 1.0 / grown.units, &ejection, 1, &rock, &anchored)
}

/// Where the line through `point` along `outward` (object units, unit) meets the surface of the cells of `before`, `cell` object units a side: from a point in a
/// filled cell out along the axis to the first empty one, from an empty one in against it to the first filled; the boundary found by halving. The point
/// itself if the line meets no boundary within six cells.
pub(crate) fn surface_along(before: &Occupancy, point: [f64; 3], outward: [f64; 3], cell: f64) -> [f64; 3] {
    let filled = |p: [f64; 3]| sr_3d::voxel::object_to_cell(p, cell).is_some_and(|k| before.get(k) != 0);
    let at = |t: f64| -> [f64; 3] { std::array::from_fn(|i| point[i] + outward[i] * t) };
    let inside = filled(point);
    // along the axis outward from filled ground, inward from empty
    let sign = if inside { 1.0 } else { -1.0 };
    let step = 0.05 * cell;
    let mut t = 0.0;
    while t < 6.0 * cell {
        if filled(at(sign * (t + step))) != inside {
            let (mut lo, mut hi) = (t, t + step);
            for _ in 0..40 {
                let mid = 0.5 * (lo + hi);
                if filled(at(sign * mid)) == inside {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            return at(sign * 0.5 * (lo + hi));
        }
        t += step;
    }
    point
}

/// The launch angle above the ground and its spread, degrees, of the cells a crater throws (those of a burst's defaults).
const EJECTA_ANGLE: f64 = 45.0;
const EJECTA_SPREAD: f64 = 15.0;
/// How many particles the law's launch model is sampled with to give the speeds of the cells.
const LAW_SAMPLES: usize = 4000;

/// A seed for what a crater throws, from the id of the object it is in.
pub fn seed_of(id: &str) -> u64 {
    id.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// An object of cells at a frame: how many cuts it has had, its cells, the bricks that differ from the cells it started with, and the pieces that
/// have come away from it.
#[derive(Debug)]
pub struct SimVoxels {
    /// Whether the body takes part at this frame: an object of cells that has broken into its pieces does not.
    pub enabled: bool,
    pub revision: u64,
    pub grid: Arc<Occupancy>,
    pub changed_bricks: Vec<[i32; 3]>,
    /// The bricks that each edit changed, by the revision it made: `(r, bricks)` for r from 1 to `revision`, so that a surface cache that read revision `a`
    /// remeshes the union of those with r over `a`. Empty for a revision that the world no longer has the history of.
    pub steps: Vec<(u64, Vec<[i32; 3]>)>,
    /// The cells that the cut threw (once there has been one): what the burst of the crater's ejecta is made of.
    pub thrown: Option<Arc<ThrownCells>>,
    pub pieces: Vec<SimVoxelPiece>,
}

/// The cells that a crater's cut threw, and then those that it left as dust, each with where it leaves from and how fast (metres and metres a second in the frame of
/// the cells; the dust is at rest in it), and the mass of one.
#[derive(Debug)]
pub struct ThrownCells {
    pub cells: Vec<crate::voxel_crater::Thrown>,
    /// Kilograms of a cell.
    pub mass: f64,
}

/// A piece that has come away from an object of cells: its cells (in the object's own keys), and its pose, a world matrix (column-major, scene space) of the
/// frame of the object's lattice.
#[derive(Debug)]
pub struct SimVoxelPiece {
    pub body: usize,
    pub enabled: bool,
    pub revision: u64,
    pub grid: Arc<Occupancy>,
    pub pose3: [f64; 16],
}

/// An object of cells that a crater can cut, as the evaluator holds it: the asset's cells, how it is cut, the bodies of the world that take its pieces, the
/// cut of the impact (a function of the impact alone, so the world and a frame both ask for it and it is made once) and the grids made of its revisions.
#[derive(Debug)]
pub(crate) struct VoxelOwner {
    pub(crate) model: Arc<crate::voxel_asset::VoxelModel>,
    settings: Settings,
    /// The bodies of the world that take the pieces of its cuts, in order.
    pub(crate) slots: Vec<usize>,
    cut: std::sync::Mutex<Option<(sr_sim::physics3d::Impact3, Arc<CraterCut>)>>,
    grids: std::sync::Mutex<std::collections::BTreeMap<(usize, u64), Arc<Occupancy>>>,
}

impl VoxelOwner {
    pub(crate) fn new(info: &CellsInfo) -> Self {
        VoxelOwner {
            model: info.model.clone(),
            settings: info.settings(),
            slots: Vec::new(),
            cut: Default::default(),
            grids: Default::default(),
        }
    }

    pub(crate) fn settings(&self) -> Settings {
        self.settings
    }

    /// The cut that `impact` makes in the cells of the asset.
    pub(crate) fn cut_for(
        &self,
        source: &crate::crater::CraterSource,
        impact: &sr_sim::physics3d::Impact3,
        id: &str,
    ) -> Result<Arc<CraterCut>, String> {
        let mut memo = self.cut.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((known, cut)) = memo.as_ref() {
            if known == impact {
                return Ok(cut.clone());
            }
        }
        let grown = crate::crater::impact_crater(source, impact, 0.0)?;
        let cut = Arc::new(crater_cut_of(&grown, &self.model.occupancy, &self.settings, seed_of(id))?);
        *memo = Some((*impact, cut.clone()));
        Ok(cut)
    }

    /// The grid of body `body` at `revision`, made of its `cells`: the palette index of a cell is the asset's, or the rim's for a cell that the cut heaped.
    pub(crate) fn grid_of(
        &self,
        body: usize,
        revision: u64,
        cells: &[[i32; 3]],
        cut: Option<&CraterCut>,
    ) -> Result<Arc<Occupancy>, String> {
        let mut grids = self.grids.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(grid) = grids.get(&(body, revision)) {
            return Ok(grid.clone());
        }
        let rim: std::collections::BTreeMap<[i32; 3], u8> =
            cut.map_or_else(Default::default, |c| c.excavation.rim.iter().copied().collect());
        let asset = &self.model.occupancy;
        let mut painted = Vec::with_capacity(cells.len());
        for c in cells {
            let index = match asset.get(*c) {
                0 => rim.get(c).copied().ok_or_else(|| {
                    format!("the cell {c:?} is neither the asset's nor the rim's: it has no palette index")
                })?,
                index => index,
            };
            painted.push((*c, index));
        }
        let grid = Arc::new(Occupancy::from_cells(painted)?);
        grids.insert((body, revision), grid.clone());
        Ok(grid)
    }
}

/// What the rigidBody of an object of cells says of it: its cells, how big they are, its density and what is done with the parts that come away.
#[derive(Debug)]
pub(crate) struct CellsInfo {
    pub(crate) model: Arc<crate::voxel_asset::VoxelModel>,
    pub(crate) size: [f64; 3],
    pub(crate) density: f64,
    pub(crate) pixels_per_meter: f64,
    pub(crate) min_cells: usize,
    pub(crate) max_fragments: usize,
    pub(crate) overflow: crate::voxels::Overflow,
    pub(crate) anchor: Anchor,
}

impl CellsInfo {
    pub(crate) fn settings(&self) -> Settings {
        Settings {
            rock: Rock {
                size: self.size,
                density: self.density,
                pixels_per_meter: self.pixels_per_meter,
                policy: crate::voxels::Policy {
                    stay: Stay::Largest,
                    min_cells: self.min_cells,
                    max_fragments: self.max_fragments,
                    overflow: self.overflow,
                },
            },
            anchor: self.anchor,
        }
    }
}

/// An object of cells that breaks into pieces at its time (`fracture`): the event of the world it is, and the pieces' bodies with their cells.
#[derive(Debug)]
pub(crate) struct CellFracture {
    pub(crate) event: usize,
    pub(crate) source: Arc<Occupancy>,
    pub(crate) pieces: Vec<(usize, Arc<Occupancy>)>,
}

/// The planes of a `fracture@planes` (groups of nx ny nz offset in object units: a cell is on the positive side if its centre has n . p >= offset),
/// as the integer planes of a partition over doubled cell coordinates. The normal is scaled by 2^20 and rounded, so a plane whose numbers are multiples
/// of 2^-20 is exact and any other is rounded to the cell side that its rounded numbers say; the offset is 2 offset / cellSize scaled the same.
pub(crate) fn planes_of(text: &str, cell_size: f64) -> Result<Vec<sr_3d::pieces::Plane>, String> {
    const K: f64 = 1_048_576.0;
    let numbers: Vec<f64> = text
        .split_whitespace()
        .map(|t| t.parse::<f64>().map_err(|_| format!("the plane number {t:?} is not a number")))
        .collect::<Result<_, _>>()?;
    if numbers.is_empty() || !numbers.len().is_multiple_of(4) || numbers.len() > 4 * sr_3d::pieces::MAX_PLANES {
        return Err(format!("planes takes from one to {} planes of four numbers", sr_3d::pieces::MAX_PLANES));
    }
    numbers
        .chunks(4)
        .map(|q| {
            let normal = [q[0], q[1], q[2]].map(|c| (c * K).round());
            let offset = (q[3] * 2.0 / cell_size * K).round();
            if !normal.iter().chain(&[offset]).all(|v| v.is_finite() && v.abs() < 1e15) {
                return Err("a plane has a number out of range".to_string());
            }
            Ok(sr_3d::pieces::Plane { normal: normal.map(|c| c as i64), offset: offset as i128 })
        })
        .collect()
}
