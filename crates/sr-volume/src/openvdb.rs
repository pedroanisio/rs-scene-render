//! Native bounded OpenVDB scalar/vector ingestion, following OpenVDB's
//! io/Archive, io/GridDescriptor, io/Compression and tree serialization.
//! Supported file versions are 222–225 with indexed 5/4/3 floating trees.
//! Values (including inactive non-background tiles) and affine maps are kept;
//! vector grids become `.x`, `.y`, `.z` channels. No OpenVDB runtime is needed.
mod codec;
mod read;

use crate::{Brick, CacheLimits, Error, SparseGrid, Transform, Volume, BRICK_VOXELS, MAX_GRIDS};
use codec::bit;
use read::Input;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Seek},
};

type Value = [f32; 3];
const WORKSPACE: u64 = 4 << 20;
const BRICK_CHARGE: u64 = BRICK_VOXELS as u64 * 4 + 128;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Kind {
    scalar_bytes: usize,
    vector: bool,
}
impl Kind {
    fn components(self) -> usize {
        if self.vector {
            3
        } else {
            1
        }
    }
    fn from_name(name: &str) -> Result<(Self, bool), Error> {
        let (name, half) = name.strip_suffix("_HalfFloat").map_or((name, false), |s| (s, true));
        let (scalar_bytes, vector) = match name {
            "Tree_half_5_4_3" => (2, false),
            "Tree_float_5_4_3" => (4, false),
            "Tree_double_5_4_3" => (8, false),
            "Tree_vec3s_5_4_3" => (4, true),
            "Tree_vec3d_5_4_3" => (8, true),
            _ => {
                return Err(Error::OpenVdb(format!(
                    "unsupported grid type {name}; expected floating scalar or vec3 5/4/3 tree"
                )))
            }
        };
        Ok((Self { scalar_bytes, vector }, half))
    }
}
struct Descriptor {
    name: String,
    kind: Kind,
    half: bool,
    parent: String,
    tree: u64,
    blocks: u64,
    end: u64,
    transform: Transform,
    compression: u32,
}
struct Budget {
    limit: CacheLimits,
    bricks: usize,
    work: usize,
    scratch: u64,
}
impl Budget {
    fn work(&mut self, n: usize) -> Result<(), Error> {
        self.work = self.work.checked_sub(n).ok_or(Error::Limit("OpenVDB decode work"))?;
        Ok(())
    }
    fn memory(&self, extra: u64) -> Result<(), Error> {
        let bytes = (self.bricks as u64)
            .checked_mul(BRICK_CHARGE)
            .and_then(|n| n.checked_add(WORKSPACE))
            .and_then(|n| n.checked_add(self.scratch))
            .and_then(|n| n.checked_add(extra))
            .ok_or(Error::Limit("OpenVDB decoded memory overflow"))?;
        if bytes > self.limit.max_bytes {
            return Err(Error::Limit("OpenVDB decoded memory"));
        }
        Ok(())
    }
    fn reserve_scratch(&mut self, bytes: u64) -> Result<(), Error> {
        self.memory(bytes)?;
        self.scratch = self.scratch.checked_add(bytes).ok_or(Error::Limit("OpenVDB scratch overflow"))?;
        Ok(())
    }
    fn admit(&mut self, count: usize) -> Result<(), Error> {
        let total = self.bricks.checked_add(count).ok_or(Error::Limit("OpenVDB brick overflow"))?;
        if total > self.limit.max_bricks {
            return Err(Error::Limit("OpenVDB total brick count"));
        }
        self.memory((count as u64).checked_mul(BRICK_CHARGE).ok_or(Error::Limit("OpenVDB decoded memory overflow"))?)?;
        self.bricks = total;
        Ok(())
    }
}

/// Read an entire seekable archive with file, output, channel and work limits.
/// `max_bytes` bounds input size and conservative decoded storage independently.
/// Decoding reserves 4 MiB for bounded node/codec scratch; tile expansion is
/// admitted before allocation. Double values must fit finite f32 storage.
pub fn read(input: impl Read + Seek, limits: CacheLimits) -> Result<Volume, Error> {
    let mut r = Input::new(input, limits.max_bytes)?;
    if r.u64()? != 0x56444220 {
        return Err(Error::Invalid("OpenVDB magic"));
    }
    let version = r.u32()?;
    if !(222..=225).contains(&version) {
        return Err(Error::OpenVdb(format!("unsupported file version {version}; expected 222..225")));
    }
    r.u32()?;
    r.u32()?;
    if r.u8()? != 1 {
        return Err(Error::Invalid("OpenVDB archive must contain seekable grid offsets"));
    }
    r.bytes::<36>()?;
    r.metadata()?;
    let count = r.u32()? as usize;
    if count > limits.max_grids.min(MAX_GRIDS) {
        return Err(Error::Limit("OpenVDB grid count"));
    }
    let mut descriptors = Vec::with_capacity(count);
    let mut names = BTreeSet::new();
    let mut channels = 0;
    for _ in 0..count {
        let name = r.string(64)?;
        if !crate::valid_name(&name) || !names.insert(name.clone()) {
            return Err(Error::Invalid("OpenVDB grid names must be valid and unique"));
        }
        let (kind, half) = Kind::from_name(&r.string(128)?)?;
        channels += kind.components();
        if channels > limits.max_grids.min(MAX_GRIDS) {
            return Err(Error::Limit("OpenVDB scalar channel count"));
        }
        let parent = r.string(64)?;
        let grid = r.u64()?;
        let blocks = r.u64()?;
        let end = r.u64()?;
        if grid < r.position || end < grid || end > r.length || (parent.is_empty() && (blocks < grid || blocks > end)) {
            return Err(Error::Invalid("OpenVDB grid offsets"));
        }
        r.end = end;
        r.seek(grid)?;
        let compression = r.u32()?;
        if compression & !7 != 0 {
            return Err(Error::Invalid("OpenVDB compression flags"));
        }
        r.metadata()?;
        let transform = transform(&mut r)?;
        let tree = r.position;
        if parent.is_empty() && tree > blocks {
            return Err(Error::Invalid("OpenVDB topology offset"));
        }
        if !parent.is_empty() && (blocks != 0 || tree != end) {
            return Err(Error::Invalid("OpenVDB instance payload or block offset"));
        }
        descriptors.push(Descriptor { name, kind, half, parent, tree, blocks, end, transform, compression });
        r.seek(end)?;
        r.end = r.length;
    }
    if r.position != r.length {
        return Err(Error::Invalid("OpenVDB trailing archive data"));
    }
    let mut volume = Volume::new();
    let mut budget = Budget { limit: limits, bricks: 0, work: 100_000_000, scratch: 0 };
    if count > 0 {
        budget.memory(0)?;
    }
    let mut done = BTreeMap::<String, (Kind, Vec<SparseGrid>)>::new();
    // Parent trees normally precede instances. Multiple passes also support
    // forward references while rejecting cycles and unknown instance parents.
    let mut pending: Vec<_> = (0..count).collect();
    while !pending.is_empty() {
        let before = pending.len();
        let mut next = Vec::new();
        for index in pending {
            let d = &descriptors[index];
            let grids = if d.parent.is_empty() {
                decode(&mut r, d, &mut budget)?
            } else {
                let Some((kind, parent)) = done.get(&d.parent) else {
                    next.push(index);
                    continue;
                };
                if *kind != d.kind {
                    return Err(Error::Invalid("OpenVDB instance tree type mismatch"));
                }
                budget.admit(parent.iter().map(SparseGrid::brick_count).sum())?;
                parent
                    .iter()
                    .map(|grid| {
                        let mut grid = grid.clone();
                        grid.transform = d.transform;
                        grid
                    })
                    .collect()
            };
            done.insert(d.name.clone(), (d.kind, grids));
        }
        if next.len() == before {
            return Err(Error::Invalid("OpenVDB cyclic or missing instance parent"));
        }
        pending = next;
    }
    for (name, (kind, grids)) in done {
        for (i, grid) in grids.into_iter().enumerate() {
            let channel = if kind.vector { format!("{name}.{}", ["x", "y", "z"][i]) } else { name.clone() };
            volume.insert(&channel, grid)?;
        }
    }
    Ok(volume)
}

fn transform<R: Read + Seek>(r: &mut Input<R>) -> Result<Transform, Error> {
    let name = r.string(128)?;
    let mut matrix = glam::DMat4::IDENTITY.to_cols_array();
    match name.as_str() {
        "AffineMap" | "UnitaryMap" => {
            for v in &mut matrix {
                *v = r.f64()?;
            }
        }
        "TranslationMap" => {
            for v in &mut matrix[12..15] {
                *v = r.f64()?;
            }
        }
        "ScaleMap" | "UniformScaleMap" | "ScaleTranslateMap" | "UniformScaleTranslateMap" => {
            if name.contains("Translate") {
                for v in &mut matrix[12..15] {
                    *v = r.f64()?;
                }
            }
            for i in [0, 5, 10] {
                matrix[i] = r.f64()?;
            }
            // Derived acceleration values are serialized too; reconstruct
            // from the actual scale after checking the supplied numbers.
            for _ in 0..12 {
                if !r.f64()?.is_finite() {
                    return Err(Error::Invalid("OpenVDB derived scale values"));
                }
            }
        }
        _ => return Err(Error::OpenVdb(format!("unsupported transform {name}; resample to an affine grid"))),
    }
    Transform::new(matrix)
}

fn decode<R: Read + Seek>(r: &mut Input<R>, d: &Descriptor, budget: &mut Budget) -> Result<Vec<SparseGrid>, Error> {
    r.end = d.blocks;
    r.seek(d.tree)?;
    if r.u32()? != 1 {
        return Err(Error::Invalid("OpenVDB tree buffer count"));
    }
    let bg = d.kind.raw(r)?;
    let tiles = r.u32()? as usize;
    let children = r.u32()? as usize;
    budget.work(tiles.checked_add(children).ok_or(Error::Limit("OpenVDB root count"))?)?;
    let mut grids: Vec<_> = (0..d.kind.components())
        .map(|i| SparseGrid::new(d.transform, bg[i], budget.limit.max_bricks))
        .collect::<Result<_, _>>()?;
    let mut roots = BTreeSet::new();
    if tiles.saturating_add(children) > 1_000_000 {
        return Err(Error::Limit("OpenVDB root entries"));
    }
    // Retained topology bookkeeping overlaps every later output allocation.
    // Include conservative BTreeSet node overhead, even for background tiles.
    budget.reserve_scratch((tiles as u64 + children as u64) * 128)?;
    let mut leaves = Vec::new();
    for _ in 0..tiles {
        let origin = root(r, &mut roots)?;
        let value = d.kind.raw(r)?;
        if r.u8()? > 1 {
            return Err(Error::Invalid("OpenVDB tile active flag"));
        }
        fill(&mut grids, origin, 4096, value, budget)?;
    }
    for _ in 0..children {
        let origin = root(r, &mut roots)?;
        node(r, d, origin, 5, bg, &mut grids, &mut leaves, budget)?;
    }
    if r.position != d.blocks {
        return Err(Error::Invalid("OpenVDB topology size"));
    }
    r.end = d.end;
    for (origin, topology_mask) in leaves {
        let mask = r.mask(512)?;
        if mask != topology_mask {
            return Err(Error::Invalid("OpenVDB leaf topology/value mask mismatch"));
        }
        let values = codec::values(r, 512, &mask, d.kind, d.half, d.compression, bg)?;
        for (component, grid) in grids.iter_mut().enumerate() {
            let mut values_out = [grid.background(); 512];
            for (i, v) in values.iter().enumerate() {
                // OpenVDB is z-fastest; the renderer's bricks are x-fastest.
                let x = i / 64;
                let y = i / 8 % 8;
                let z = i % 8;
                values_out[x + 8 * y + 64 * z] = v[component];
            }
            insert(grid, origin.map(|v| v / 8), values_out, budget)?;
        }
    }
    if r.position != d.end {
        return Err(Error::Invalid("OpenVDB grid payload size"));
    }
    budget.scratch = 0;
    Ok(grids)
}
fn root<R: Read + Seek>(r: &mut Input<R>, roots: &mut BTreeSet<[i32; 3]>) -> Result<[i32; 3], Error> {
    let origin = r.coord()?;
    if origin.iter().any(|v| v.rem_euclid(4096) != 0) || !roots.insert(origin) {
        return Err(Error::Invalid("OpenVDB root origin alignment or duplicate"));
    }
    Ok(origin)
}
type Leaf = ([i32; 3], Vec<u64>);
#[allow(clippy::too_many_arguments)]
fn node<R: Read + Seek>(
    r: &mut Input<R>,
    d: &Descriptor,
    origin: [i32; 3],
    log: usize,
    bg: Value,
    grids: &mut [SparseGrid],
    leaves: &mut Vec<Leaf>,
    budget: &mut Budget,
) -> Result<(), Error> {
    let count = 1 << (3 * log);
    budget.work(count)?;
    let child = r.mask(count)?;
    let mask = r.mask(count)?;
    if child.iter().zip(&mask).any(|(a, b)| a & b != 0) {
        return Err(Error::Invalid("OpenVDB child/value mask overlap"));
    }
    let values = codec::values(r, count, &mask, d.kind, d.half, d.compression, bg)?;
    let size = if log == 5 { 128 } else { 8 };
    for (i, value) in values.into_iter().enumerate() {
        let row = (1 << log) - 1;
        let offsets = [i >> (2 * log), (i >> log) & row, i & row];
        let point = std::array::from_fn(|a| origin[a] + (offsets[a] * size) as i32);
        if bit(&child, i) {
            if log == 5 {
                node(r, d, point, 4, bg, grids, leaves, budget)?;
            } else {
                if leaves.len() >= budget.limit.max_bricks {
                    return Err(Error::Limit("OpenVDB leaf count"));
                }
                // Mask allocation plus descriptor capacity, allowing geometric
                // Vec growth. Retain the charge until this grid is decoded.
                budget.reserve_scratch(192)?;
                leaves.push((point, r.mask(512)?));
            }
        } else {
            fill(grids, point, size as i32, value, budget)?;
        }
    }
    Ok(())
}

fn fill(grids: &mut [SparseGrid], origin: [i32; 3], size: i32, value: Value, budget: &mut Budget) -> Result<(), Error> {
    let side = (size / 8) as usize;
    let count = side.checked_pow(3).ok_or(Error::Limit("OpenVDB tile expansion"))?;
    let components = grids.iter().enumerate().filter(|(i, g)| value[*i] != g.background()).count();
    let total = count.checked_mul(components).ok_or(Error::Limit("OpenVDB tile expansion"))?;
    if total == 0 {
        return Ok(());
    }
    budget.work(total)?;
    budget.admit(total)?;
    for (i, grid) in grids.iter_mut().enumerate() {
        if value[i] == grid.background() {
            continue;
        }
        for x in 0..side {
            for y in 0..side {
                for z in 0..side {
                    let delta = [x, y, z];
                    let key = std::array::from_fn(|a| origin[a] / 8 + delta[a] as i32);
                    if grid.bricks.contains_key(&key) {
                        return Err(Error::Invalid("overlapping OpenVDB tiles"));
                    }
                    grid.bricks.insert(key, Brick { values: Box::new([value[i]; 512]), active: 512 });
                }
            }
        }
    }
    Ok(())
}
fn insert(grid: &mut SparseGrid, key: [i32; 3], values: [f32; 512], budget: &mut Budget) -> Result<(), Error> {
    let active = values.iter().filter(|&&v| v != grid.background()).count() as u16;
    if active == 0 {
        return Ok(());
    }
    if grid.bricks.contains_key(&key) {
        return Err(Error::Invalid("overlapping OpenVDB leaves"));
    }
    budget.admit(1)?;
    grid.bricks.insert(key, Brick { values: Box::new(values), active });
    Ok(())
}
