//! The cells of a `voxelAsset`: the file or the mesh it names, read within its limits and made into an [`Occupancy`] that has its
//! minimum corner at the origin, with the materials of the file, as numbers, and what a cache needs to know when any of it changed.
//!
//! * The asset key resolves like a mesh asset's (a local file; a remote scheme is an error that says so). The bytes are read bounded by
//!   `maxMemoryMiB`, hashed, and the declared `sha256` is checked on THE BYTES THAT WERE READ before anything is parsed: a file that is not
//!   what the document says is refused as that, and not as a malformed `.vox`.
//! * `maxCells` and `maxMemoryMiB` (the engine's 4,194,304 and 128) are the limits of the grid; over either is an error that names the
//!   number, and the importers add their own bounds (see `sr_3d::voxel::vox::Bounds`).
//! * `fromMesh` cuts the mesh asset in the frame it is drawn in ([`crate::voxel::from_model`] and the same flattening), the cells on a lattice
//!   of multiples of `cellSize` scene units; it has no palette and no materials. The mesh asset is the one of the same document as the voxel
//!   asset (an asset of an included document is named by its namespace and its id, and so is the mesh it cuts); the `sha256` of a
//!   `voxelAsset` that has `fromMesh` is not read, the mesh asset's is.
//! * The cells are moved so that the corner of the box of the occupied cells is the origin ([`VoxelModel::origin_cells`] is how far they
//!   were from it: the minimum key in the lattice of the scene after the scene graph, so the cells of the file are `occupancy` plus that,
//!   and the pivot of a model of the file, `floor(size / 2)`, can be worked out again by whoever needs it).
//! * Every asset is read once for a program and key ("parse once"): a later call finds the file as it was (the same length and modification
//!   time, for the file and for every file it reads) and returns the model without reading it. If the length or the time changed the bytes
//!   are read and hashed, and the entry is reused only if the hash is the same (a rewrite of the same bytes is not a change); a file that
//!   changes with the same length AND the same time is taken for the same, which is the price of not hashing up to `maxMemoryMiB` on every
//!   call. A `fromMesh` source is the mesh file and every file the importer reads for it (a `.bin` beside a `.gltf`, the materials and textures
//!   of an `.obj`), and its digest covers all of them; the mesh is cut after its digest is taken and the digest is taken again after, and a
//!   file that changed between the two is an error and not a model that its digest does not describe. The `sha256` that a mesh asset declares
//!   is checked against its own file's bytes.
//!   How easily that hole is fallen into: the modification time of a file is as coarse as the clock of the kernel on Linux (about four
//!   milliseconds), two seconds on FAT, and `cp -p`, `rsync -t` and `tar` keep the time of the file they copy, so a file replaced by another of
//!   the same length with its time kept is taken for the old one; without a modification time (a file system that has none) only the length decides.
//! * The order of a load is: the size of the main file against the limit, then the cache (by the sizes and times of the files that the model was
//!   made from, the ones that were not there included, as absent), then the files that the importer reads for a mesh (this scan reads and parses
//!   the mesh, so it is behind the limit and the cache), then the bytes. The size and the time of a file are taken from the handle that is read,
//!   before it is read, so that a write after the read is a change and not a model that is held with the time of the newer file.
//! * The peak memory of a load: for a file asset about three times `maxMemoryMiB` (the bytes of the file, the grid that the importer builds and,
//!   when the corner of the cells is not already at the origin, the copy that is moved); for a `fromMesh` up to about four times it, plus what the
//!   mesh importer holds for the model (not bounded by `maxMemoryMiB` beyond the size of the file): the bytes of the mesh are hashed as they are
//!   read and not kept, and what stays is the importer's model, the triangles that are made of it (within `maxMemoryMiB`), the grid and the
//!   copy.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use sha2::{Digest, Sha256};
use sr_3d::occupancy::{Limits, Occupancy};
use sr_3d::voxel::material::{self, Material};
use sr_3d::voxel::{srvol, vox, Colours};
use sr_model::model::{AssetsChild, MeshAsset, VoxelAsset, VoxelAssetFormat};

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

/// The size of a cell of an object of primitive voxels, in the object's units: `cellSize` of the object, else the asset's own, else 1.
pub fn cell_size(object: Option<f64>, model: &VoxelModel) -> f64 {
    object.or(model.cell_size).unwrap_or(1.0)
}

/// A file as it was when it was read: where, whether it was there, how long, and when it was last written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stat {
    path: PathBuf,
    present: bool,
    len: u64,
    modified: Option<SystemTime>,
}

impl Stat {
    /// The file as it is, or as absent (a file that is not there is something that can be made, and then the source is not the one that was read).
    fn of(path: &Path) -> Stat {
        match std::fs::metadata(path) {
            Ok(meta) => {
                Stat { path: path.to_path_buf(), present: true, len: meta.len(), modified: meta.modified().ok() }
            }
            Err(_) => Stat { path: path.to_path_buf(), present: false, len: 0, modified: None },
        }
    }

    fn from_handle(path: &Path, meta: &std::fs::Metadata) -> Stat {
        Stat { path: path.to_path_buf(), present: true, len: meta.len(), modified: meta.modified().ok() }
    }
}

#[derive(Debug)]
pub(crate) struct Entry {
    source: Source,
    /// The files that the model was made from, as they were, a file that was not there as absent: the model is returned only while every one of
    /// them is as it was.
    stats: Vec<Stat>,
    model: Arc<VoxelModel>,
}

/// Models read for a program, by asset key; they live as long as the compiled scene.
pub(crate) type Cache = Mutex<HashMap<String, Arc<Entry>>>;

fn document<'a>(p: &'a Program, key: &str) -> Result<(&'a sr_model::model::Scene, PathBuf, String), String> {
    let (doc, id) = p.assets.get(key).ok_or_else(|| format!("asset {key} not found"))?;
    let scene = if *doc == 0 { &p.scene } else { &p.includes.get(*doc as usize - 1).ok_or("include missing")?.1 };
    Ok((scene, p.base_dirs.get(*doc as usize).cloned().unwrap_or_default(), id.clone()))
}

fn asset<'a>(p: &'a Program, key: &str) -> Result<(&'a VoxelAsset, PathBuf), String> {
    let (scene, base, id) = document(p, key)?;
    let found = scene
        .assets
        .as_ref()
        .and_then(|a| {
            a.children.iter().find_map(|c| match c {
                AssetsChild::VoxelAsset(v) if v.id == id => Some(v),
                _ => None,
            })
        })
        .ok_or_else(|| format!("asset {id} is not a voxelAsset"))?;
    Ok((found, base))
}

fn mesh_asset<'a>(p: &'a Program, key: &str) -> Result<(&'a MeshAsset, PathBuf), String> {
    let (scene, base, id) = document(p, key)?;
    let found = scene
        .assets
        .as_ref()
        .and_then(|a| {
            a.children.iter().find_map(|c| match c {
                AssetsChild::Mesh(m) if m.id == id => Some(m),
                _ => None,
            })
        })
        .ok_or_else(|| format!("asset {id} is not a mesh"))?;
    Ok((found, base))
}

fn hex(digest: &[u8; 32]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// A file read within `max` bytes: its SHA-256, its bytes if they are wanted, and the file as it was (from the handle, before it was read).
struct Reading {
    sha: [u8; 32],
    bytes: Vec<u8>,
    stat: Stat,
}

/// Reads a file, no more than `max` of it: a longer file is an error before it is read whole. The bytes are hashed as they come, and kept only
/// if `keep`.
fn read_bounded(path: &Path, max: u64, keep: bool) -> Result<Reading, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let meta = file.metadata().map_err(|e| format!("{}: {e}", path.display()))?;
    let stat = Stat::from_handle(path, &meta);
    if meta.len() > max {
        return Err(format!(
            "{}: the file has {} bytes and the limit is {max} bytes (maxMemoryMiB)",
            path.display(),
            meta.len()
        ));
    }
    let mut hash = Sha256::new();
    let mut bytes = Vec::new();
    let mut total = 0u64;
    let mut chunk = vec![0u8; 1 << 16];
    let mut reader = file.take(max + 1);
    loop {
        let n = reader.read(&mut chunk).map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        hash.update(&chunk[..n]);
        if keep {
            bytes.extend_from_slice(&chunk[..n]);
        }
    }
    if total > max {
        return Err(format!("{}: the file is longer than the limit of {max} bytes (maxMemoryMiB)", path.display()));
    }
    Ok(Reading { sha: hash.finalize().into(), bytes, stat })
}

/// What the source of a model is: the bytes of its main file (kept, for a format that is read from them), and the digest of the main
/// file and of every other file that it reads, in the order of their names (a file that is missing counts as missing, so that creating it
/// is a change). With no other file the digest is the main file's own SHA-256.
struct Sourced {
    bytes: Vec<u8>,
    /// SHA-256 of the main file, which is what a document declares.
    main: [u8; 32],
    source: Source,
    stats: Vec<Stat>,
}

fn read_source(main: &Path, others: &[PathBuf], max: u64, keep_main: bool) -> Result<Sourced, String> {
    let first = read_bounded(main, max, keep_main)?;
    let mut stats = vec![first.stat.clone()];
    let mut total = first.stat.len;
    let mut modified = first.stat.modified;
    let mut digest = first.sha;
    if !others.is_empty() {
        let mut h = Sha256::new();
        h.update(first.sha);
        for path in others {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            h.update((name.len() as u64).to_le_bytes());
            h.update(name.as_bytes());
            if !path.exists() {
                h.update([0u8]);
                stats.push(Stat::of(path));
                continue;
            }
            let dependency = read_bounded(path, max.saturating_sub(total), false)?;
            total += dependency.stat.len;
            modified = modified.max(dependency.stat.modified);
            h.update([1u8]);
            h.update(dependency.sha);
            stats.push(dependency.stat);
        }
        digest = h.finalize().into();
    }
    Ok(Sourced {
        bytes: first.bytes,
        main: first.sha,
        source: Source { sha256: digest, bytes: total, modified },
        stats,
    })
}

fn local(src: &str, base: &Path, what: &str) -> Result<PathBuf, String> {
    match sr_model::assets::resolve(src, base) {
        sr_model::assets::Resolved::Local(path) => Ok(path),
        sr_model::assets::Resolved::Remote(scheme) => {
            Err(format!("{what} {src} is remote ({scheme}), and a voxel asset is read from a local file"))
        }
    }
}

/// Moves the cells so that the minimum corner of their box is the origin; the minimum key is what was taken away. The box can be at most
/// as wide as the keys of an occupancy are on an axis (2^30 cells), and a wider one is an error that says how wide it is.
fn at_origin(occupancy: Occupancy, limits: Limits) -> Result<(Occupancy, [i64; 3]), String> {
    let Some((min, max)) = occupancy.bounds() else {
        return Err("the voxel asset has no cell".into());
    };
    let limit = i64::from(sr_3d::occupancy::KEY_LIMIT);
    for axis in 0..3 {
        let span = i64::from(max[axis]) - i64::from(min[axis]) + 1;
        if span > limit {
            return Err(format!(
                "the cells span {span} cells along axis {axis} and an occupancy has at most {limit} keys on an axis once its corner is at the origin"
            ));
        }
    }
    let origin = min.map(i64::from);
    if min == [0, 0, 0] {
        return Ok((occupancy, origin));
    }
    let mut moved = Occupancy::with_limits(limits);
    for cell in occupancy.cells() {
        // the box is at most 2^30 wide, so a cell minus the minimum is not negative and is a key of an occupancy
        moved.set([cell[0] - min[0], cell[1] - min[1], cell[2] - min[2]], occupancy.get(cell))?;
    }
    moved.set_palette(*occupancy.palette().colors());
    Ok((moved, origin))
}

/// The cells of the voxel asset `key` of `program`, read once and read again only if the file is not the one that was read.
pub fn load(program: &Program, key: &str) -> Result<Arc<VoxelModel>, String> {
    let (asset, base) = asset(program, key)?;
    let name = &asset.id;
    let max_cells = asset.max_cells.unwrap_or(DEFAULT_MAX_CELLS);
    let max_bytes = asset.max_memory_mi_b.unwrap_or(DEFAULT_MAX_MEMORY_MIB) << 20;
    let limits =
        Limits { max_bricks: usize::MAX, max_cells, max_bytes: usize::try_from(max_bytes).unwrap_or(usize::MAX) };
    let with = |e: String| format!("voxelAsset {name}: {e}");
    // the files: the one a file asset names, or the mesh asset's (of the same document, so under the same namespace) and what it reads
    let namespace = key.strip_suffix(name.as_str()).unwrap_or("");
    let mesh_key = asset.from_mesh.as_ref().map(|m| format!("{namespace}{m}"));
    let (main, mesh_format, declared) = match (&asset.src, &mesh_key) {
        (Some(src), None) => (local(src, &base, "the voxel asset")?, None, asset.sha256),
        (None, Some(mesh_key)) => {
            let (mesh, mesh_base) = mesh_asset(program, mesh_key).map_err(with)?;
            let path = local(&mesh.src, &mesh_base, "the mesh").map_err(with)?;
            (path, Some(mesh.format.map(|f| f.to_string())), mesh.sha256)
        }
        _ => return Err(with("has exactly one of src and fromMesh (VOX2)".into())),
    };
    // the size of the main file against the limit, before any file is read or parsed (the scan for what a mesh reads parses the whole mesh)
    let head = Stat::of(&main);
    if head.present && head.len > max_bytes {
        return Err(with(format!(
            "{}: the file has {} bytes and the limit is {max_bytes} bytes (maxMemoryMiB)",
            main.display(),
            head.len
        )));
    }
    // the files are as they were when the model was made: the model, with no reading of any of them
    if let Some(entry) = program.voxel_models.lock().unwrap().get(key) {
        if !entry.stats.is_empty() && entry.stats.iter().all(|s| Stat::of(&s.path) == *s) {
            return Ok(entry.model.clone());
        }
    }
    // the files that the importer reads for a mesh, now that the file is within the limit and is not a cached one
    let others = match &mesh_format {
        Some(format) => {
            let mut others = sr_3d::import::dependencies_as(&main, format.as_deref()).map_err(with)?;
            others.retain(|p| *p != main);
            others.sort();
            others.dedup();
            others
        }
        None => Vec::new(),
    };
    let read = read_source(&main, &others, max_bytes, asset.src.is_some()).map_err(with)?;
    if let Some(declared) = declared {
        if declared.0 != read.main {
            return Err(with(format!(
                "the file is not the one the document names: its sha256 is {} and the document says {declared}",
                hex(&read.main)
            )));
        }
    }
    // the file was written again without a change of its bytes: the model is the same, and the entry is of the file as it is now
    let known =
        program.voxel_models.lock().unwrap().get(key).filter(|e| e.source.sha256 == read.source.sha256).cloned();
    if let Some(entry) = known {
        let model = entry.model.clone();
        let renewed = Entry { source: read.source, stats: read.stats, model: model.clone() };
        program.voxel_models.lock().unwrap().insert(key.to_string(), Arc::new(renewed));
        return Ok(model);
    }
    let bytes = &read.bytes;
    let (occupancy, colours, materials, cell_size) = match (&asset.src, &mesh_key) {
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
                    let imported = vox::import(bytes, model, limits, &bounds).map_err(with)?;
                    let materials = material::parse_all(&imported.materials).map_err(with)?;
                    (imported.occupancy, Some(imported.colours), materials, None)
                }
                Some(VoxelAssetFormat::Srvol) => {
                    let grid = asset.voxel_grid.as_deref().unwrap_or(srvol::GRID);
                    let cache = sr_volume::CacheLimits { max_bytes, ..Default::default() };
                    let imported = srvol::import(bytes, grid, limits, cache).map_err(with)?;
                    (imported.occupancy, None, BTreeMap::new(), imported.cell_size)
                }
                None => return Err(with(format!("the format of {src} is not vox or srvol: say it with format"))),
            }
        }
        (None, Some(mesh_key)) => {
            let cell = asset.cell_size.ok_or_else(|| with("fromMesh needs cellSize (VOX2)".into()))?.get();
            let budget = usize::try_from(max_bytes).unwrap_or(usize::MAX);
            let (points, triangles) = crate::sim3d::mesh_asset_triangles(program, mesh_key, budget).map_err(with)?;
            let bounds = crate::voxel::Bounds::default();
            let occupancy = crate::voxel::from_triangles(&points, &triangles, cell, limits, &bounds).map_err(with)?;
            // the importer read the files itself: they are the ones that the digest was taken of, or the digest does not describe the cells
            let again = read_source(&main, &others, max_bytes, false).map_err(with)?;
            if again.source.sha256 != read.source.sha256 {
                return Err(with(
                    "the mesh changed while it was read, and the cells are not those of the digest that was taken"
                        .into(),
                ));
            }
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
        source: read.source.clone(),
    });
    program
        .voxel_models
        .lock()
        .unwrap()
        .insert(key.to_string(), Arc::new(Entry { source: read.source, stats: read.stats, model: model.clone() }));
    Ok(model)
}
