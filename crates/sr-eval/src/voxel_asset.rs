//! The cells of a `voxelAsset`: the file or the mesh it names, read within its limits and made into an [`Occupancy`] that has its
//! minimum corner at the origin, with the materials of the file, as numbers, and what a cache needs to know when any of it changed.
//!
//! * The asset key resolves like a mesh asset's (a local file; a remote scheme is an error that says so). The bytes are read bounded by
//!   `maxMemoryMiB`, hashed, and the declared `sha256` is checked on THE BYTES THAT WERE READ before anything is parsed: a file that is not
//!   what the document says is refused as that, and not as a malformed `.vox`.
//! * `maxCells` and `maxMemoryMiB` (the engine's 4,194,304 and 128) are the limits of the grid; over either is an error that names the
//!   number, and the importers add their own bounds (see `sr_3d::voxel::vox::Bounds`).
//! * `fromMesh` cuts the mesh asset in the frame it is drawn in ([`crate::voxel::from_model`] and the same flattening), the cells on a lattice
//!   of multiples of `cellSize` scene units; it has no palette and no materials, and its `sha256` is the mesh file's bytes' (the `sha256`
//!   of a `voxelAsset` that has `fromMesh` is not read: the mesh asset has its own).
//! * The cells are moved so that the corner of the box of the occupied cells is the origin ([`VoxelModel::origin_cells`] is how far they
//!   were from it: the minimum key in the lattice of the scene after the scene graph, so the cells of the file are `occupancy` plus that,
//!   and the pivot of a model of the file, `floor(size / 2)`, can be worked out again by whoever needs it).
//! * Every asset is read once for a program and key, and again when its bytes are not the ones that were read: an entry is reused only
//!   if the hash of the bytes is the same (with the length and the modification time that were seen kept on it, for whoever reports them).

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use sha2::{Digest, Sha256};
use sr_3d::occupancy::{Limits, Occupancy};
use sr_3d::voxel::material::{self, Material};
use sr_3d::voxel::{srvol, vox, Colours};
use sr_model::model::{AssetsChild, VoxelAsset, VoxelAssetFormat};

use crate::program::Program;

/// The engine's value of `maxCells` and of `maxMemoryMiB` when the asset names none.
pub const DEFAULT_MAX_CELLS: u64 = 4_194_304;
pub const DEFAULT_MAX_MEMORY_MIB: u64 = 128;

/// What the file that the cells came from was when they were read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    /// SHA-256 of the bytes that were read (of the mesh file, for `fromMesh`).
    pub sha256: [u8; 32],
    pub bytes: u64,
    pub modified: Option<SystemTime>,
}

/// A voxel asset as the scene uses it.
#[derive(Clone, Debug)]
pub struct VoxelModel {
    /// The cells, the box of the occupied ones having its minimum corner at the origin.
    pub occupancy: Occupancy,
    /// The minimum key of the occupied cells before they were moved, in the lattice of the scene after the scene graph.
    pub origin_cells: [i64; 3],
    /// Where the palette of `occupancy` comes from; none for a cache or a mesh, which have no palette.
    pub colours: Option<Colours>,
    /// The properties of the materials of the file by palette index, as numbers (see `sr_3d::voxel::material` for the units).
    pub materials: BTreeMap<u8, Material>,
    /// A hash of `materials`, for a cache of surfaces.
    pub materials_fingerprint: u64,
    /// A hash of the cells and of their palette indices ([`Occupancy::fingerprint`]).
    pub fingerprint: u64,
    /// The size of a cell that the asset says, in scene units: `cellSize` of a `fromMesh` asset, or the scale of the grid of a cache.
    pub cell_size: Option<f64>,
    pub source: Source,
}

#[derive(Debug)]
pub(crate) struct Entry {
    source: Source,
    model: Arc<VoxelModel>,
}

/// Models read for a program, by asset key; they live as long as the compiled scene.
pub(crate) type Cache = Mutex<HashMap<String, Arc<Entry>>>;

fn asset<'a>(p: &'a Program, key: &str) -> Result<(&'a VoxelAsset, std::path::PathBuf), String> {
    let (doc, id) = p.assets.get(key).ok_or_else(|| format!("asset {key} not found"))?;
    let scene = if *doc == 0 { &p.scene } else { &p.includes.get(*doc as usize - 1).ok_or("include missing")?.1 };
    let found = scene
        .assets
        .as_ref()
        .and_then(|a| {
            a.children.iter().find_map(|c| match c {
                AssetsChild::VoxelAsset(v) if v.id == *id => Some(v),
                _ => None,
            })
        })
        .ok_or_else(|| format!("asset {id} is not a voxelAsset"))?;
    Ok((found, p.base_dirs.get(*doc as usize).cloned().unwrap_or_default()))
}

/// The bytes of a file, no more than `max` of them: a longer file is an error before it is read whole.
fn read_bounded(path: &std::path::Path, max: u64) -> Result<(Vec<u8>, Source), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let meta = file.metadata().map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > max {
        return Err(format!(
            "{}: the file has {} bytes and the limit is {max} bytes (maxMemoryMiB)",
            path.display(),
            meta.len()
        ));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(max + 1).read_to_end(&mut bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() as u64 > max {
        return Err(format!("{}: the file is longer than the limit of {max} bytes (maxMemoryMiB)", path.display()));
    }
    let sha256: [u8; 32] = Sha256::digest(&bytes).into();
    let source = Source { sha256, bytes: bytes.len() as u64, modified: meta.modified().ok() };
    Ok((bytes, source))
}

fn local(src: &str, base: &std::path::Path, what: &str) -> Result<std::path::PathBuf, String> {
    match sr_model::assets::resolve(src, base) {
        sr_model::assets::Resolved::Local(path) => Ok(path),
        sr_model::assets::Resolved::Remote(scheme) => {
            Err(format!("{what} {src} is remote ({scheme}), and a voxel asset is read from a local file"))
        }
    }
}

/// Moves the cells so that the minimum corner of their box is the origin; the minimum key is what was taken away.
fn at_origin(occupancy: Occupancy, limits: Limits) -> Result<(Occupancy, [i64; 3]), String> {
    let Some((min, _)) = occupancy.bounds() else {
        return Err("the voxel asset has no cell".into());
    };
    let mut moved = Occupancy::with_limits(limits);
    for cell in occupancy.cells() {
        // the keys are within 2^30 of the origin, so a cell minus the minimum is not negative and is inside an i32
        moved.set([cell[0] - min[0], cell[1] - min[1], cell[2] - min[2]], occupancy.get(cell))?;
    }
    moved.set_palette(*occupancy.palette().colors());
    Ok((moved, min.map(i64::from)))
}

/// The cells of the voxel asset `key` of `program`, read once and read again only if the bytes of the file are not the ones that were.
pub fn load(program: &Program, key: &str) -> Result<Arc<VoxelModel>, String> {
    let (asset, base) = asset(program, key)?;
    let name = &asset.id;
    let max_cells = asset.max_cells.unwrap_or(DEFAULT_MAX_CELLS);
    let max_bytes = asset.max_memory_mi_b.unwrap_or(DEFAULT_MAX_MEMORY_MIB) << 20;
    let limits =
        Limits { max_bricks: usize::MAX, max_cells, max_bytes: usize::try_from(max_bytes).unwrap_or(usize::MAX) };
    let with = |e: String| format!("voxelAsset {name}: {e}");
    let (bytes, source) = match (&asset.src, &asset.from_mesh) {
        (Some(src), None) => read_bounded(&local(src, &base, "the voxel asset")?, max_bytes).map_err(with)?,
        (None, Some(mesh)) => {
            let (path, _) = crate::sim3d::mesh_path(program, mesh).map_err(with)?;
            read_bounded(&path, max_bytes).map_err(with)?
        }
        _ => return Err(with("has exactly one of src and fromMesh (VOX2)".into())),
    };
    if let (Some(declared), None) = (&asset.sha256, &asset.from_mesh) {
        if declared.0 != source.sha256 {
            let got: String = source.sha256.iter().map(|b| format!("{b:02x}")).collect();
            return Err(with(format!(
                "the file is not the one the document names: its sha256 is {got} and the document says {declared}"
            )));
        }
    }
    if let Some(entry) = program.voxel_models.lock().unwrap().get(key) {
        if entry.source.sha256 == source.sha256 {
            return Ok(entry.model.clone());
        }
    }
    let (occupancy, colours, materials, cell_size) = match (&asset.src, &asset.from_mesh) {
        (Some(src), _) => {
            let by_extension = match src.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
                Some("vox") => Some(VoxelAssetFormat::Vox),
                Some("srvol") => Some(VoxelAssetFormat::Srvol),
                _ => None,
            };
            match asset.format.or(by_extension) {
                Some(VoxelAssetFormat::Vox) => {
                    let bounds = vox::Bounds { max_file_bytes: bytes.len(), ..vox::Bounds::default() };
                    let model = asset.model.map(|m| m as usize);
                    let imported = vox::import(&bytes, model, limits, &bounds).map_err(with)?;
                    let materials = material::parse_all(&imported.materials).map_err(with)?;
                    (imported.occupancy, Some(imported.colours), materials, None)
                }
                Some(VoxelAssetFormat::Srvol) => {
                    let grid = asset.voxel_grid.as_deref().unwrap_or(srvol::GRID);
                    let cache = sr_volume::CacheLimits { max_bytes, ..Default::default() };
                    let imported = srvol::import(&bytes, grid, limits, cache).map_err(with)?;
                    (imported.occupancy, None, BTreeMap::new(), imported.cell_size)
                }
                None => return Err(with(format!("the format of {src} is not vox or srvol: say it with format"))),
            }
        }
        (None, Some(mesh)) => {
            let cell = asset.cell_size.ok_or_else(|| with("fromMesh needs cellSize (VOX2)".into()))?.get();
            let (points, triangles) =
                crate::sim3d::mesh_asset_triangles(program, mesh, usize::try_from(max_bytes).unwrap_or(usize::MAX))
                    .map_err(with)?;
            let bounds = crate::voxel::Bounds::default();
            let occupancy = crate::voxel::from_triangles(&points, &triangles, cell, limits, &bounds).map_err(with)?;
            (occupancy, None, BTreeMap::new(), Some(cell))
        }
        _ => unreachable!("one source was checked above"),
    };
    let (occupancy, origin_cells) = at_origin(occupancy, limits).map_err(with)?;
    let model = Arc::new(VoxelModel {
        fingerprint: occupancy.fingerprint(),
        materials_fingerprint: material::fingerprint(&materials),
        occupancy,
        origin_cells,
        colours,
        materials,
        cell_size,
        source: source.clone(),
    });
    program.voxel_models.lock().unwrap().insert(key.to_string(), Arc::new(Entry { source, model: model.clone() }));
    Ok(model)
}
