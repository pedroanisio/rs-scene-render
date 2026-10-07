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
    let kernel = sr_3d::crater::Crater::conserving(
        grown.spec,
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
        normal: grown.spec.outward,
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
    pub revision: u64,
    pub grid: Arc<Occupancy>,
    pub changed_bricks: Vec<[i32; 3]>,
    /// The bricks that each edit changed, by the revision it made: `(r, bricks)` for r from 1 to `revision`, so that a surface cache that read revision `a`
    /// remeshes the union of those with r over `a`. Empty for a revision that the world no longer has the history of.
    pub steps: Vec<(u64, Vec<[i32; 3]>)>,
    pub pieces: Vec<SimVoxelPiece>,
}

/// A piece that has come away from an object of cells: its cells (in the object's own keys), and its pose, a world matrix (column-major, scene space) of the
/// frame of the object's lattice.
#[derive(Debug)]
pub struct SimVoxelPiece {
    pub body: usize,
    pub revision: u64,
    pub grid: Arc<Occupancy>,
    pub pose3: [f64; 16],
}

/// An object of cells that a crater can cut, as the evaluator holds it: the asset's cells, how it is cut, the bodies of the world that take its pieces, the
/// cut of the impact (a function of the impact alone, so the world and a frame both ask for it and it is made once) and the grids made of its revisions.
#[derive(Debug)]
pub(crate) struct VoxelOwner {
    pub(crate) model: Arc<crate::voxel_asset::VoxelModel>,
    pub(crate) settings: Settings,
    /// The bodies of the world that take the pieces of its cuts, in order.
    pub(crate) slots: Vec<usize>,
    cut: std::sync::Mutex<Option<(sr_sim::physics3d::Impact3, Arc<CraterCut>)>>,
    grids: std::sync::Mutex<std::collections::BTreeMap<(usize, u64), Arc<Occupancy>>>,
}

impl VoxelOwner {
    pub(crate) fn new(model: Arc<crate::voxel_asset::VoxelModel>, settings: Settings) -> Self {
        VoxelOwner { model, settings, slots: Vec::new(), cut: Default::default(), grids: Default::default() }
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
        let grid = Arc::new(Occupancy::from_cells(cells.iter().map(|c| {
            let index = asset.get(*c);
            (*c, if index != 0 { index } else { rim.get(c).copied().unwrap_or(1) })
        }))?);
        grids.insert((body, revision), grid.clone());
        Ok(grid)
    }
}
