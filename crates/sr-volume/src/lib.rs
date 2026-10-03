//! Sparse volumetric fields shared by simulation, asset loading and rendering.
//!
//! A grid stores finite scalar samples at integer voxel centres in 8³ bricks.
//! Missing voxels have a constant background. An invertible affine transform maps
//! index coordinates to scene units. Density, temperature and each velocity component
//! are separate named grids, allowing their resolutions and transforms to differ.
//! [`Volume`] reads and writes the versioned, little-endian SRVOL cache format.
//! Cache readers enforce byte, channel and total brick budgets before allocating.

use std::collections::BTreeMap;
use std::io::{Read, Write};

use glam::{DMat4, DVec3};

pub mod advection;
pub mod bake;
pub mod medium;
pub mod openvdb;
pub mod sequence;
pub mod thermal;

/// Edge length of one sparse brick, in voxels.
pub const BRICK_SIDE: i32 = 8;
/// Scalar samples in a brick, with x varying fastest, then y, then z.
pub const BRICK_VOXELS: usize = 512;
const MAGIC: &[u8; 8] = b"SRVOL\0\r\n";
const VERSION: u32 = 1;
const MAX_NAME: usize = 64;
const MAX_GRIDS: usize = 64;

/// Invalid data, an exhausted resource budget, or an underlying I/O failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid volume: {0}")]
    Invalid(&'static str),
    #[error("invalid OpenVDB: {0}")]
    OpenVdb(String),
    #[error("volume resource limit: {0}")]
    Limit(&'static str),
    #[error("volume cache I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Validated index-to-world affine transform (column-major, f64).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    forward: DMat4,
    inverse: DMat4,
}

impl Transform {
    pub fn identity() -> Self {
        Self { forward: DMat4::IDENTITY, inverse: DMat4::IDENTITY }
    }

    /// Rejects nonfinite, singular and projective transforms.
    pub fn new(columns: [f64; 16]) -> Result<Self, Error> {
        if columns.iter().any(|x| !x.is_finite())
            || [columns[3], columns[7], columns[11], columns[15]] != [0.0, 0.0, 0.0, 1.0]
        {
            return Err(Error::Invalid("transform must be finite and affine"));
        }
        let forward = DMat4::from_cols_array(&columns);
        let det = forward.determinant();
        if !det.is_finite() || det == 0.0 {
            return Err(Error::Invalid("transform must be invertible"));
        }
        let inverse = forward.inverse();
        if !inverse.is_finite() {
            return Err(Error::Invalid("inverse transform is nonfinite"));
        }
        Ok(Self { forward, inverse })
    }

    pub fn columns(&self) -> [f64; 16] {
        self.forward.to_cols_array()
    }

    pub fn index_to_world(&self, index: [f64; 3]) -> [f64; 3] {
        self.forward.transform_point3(DVec3::from(index)).to_array()
    }

    pub fn world_to_index(&self, world: [f64; 3]) -> [f64; 3] {
        self.inverse.transform_point3(DVec3::from(world)).to_array()
    }
}

#[derive(Clone, Debug)]
struct Brick {
    values: Box<[f32; BRICK_VOXELS]>,
    active: u16,
}

/// Sparse scalar field. Its brick budget is enforced before every allocation.
///
/// ```
/// use sr_volume::{SparseGrid, Transform};
/// let mut density = SparseGrid::new(Transform::identity(), 0.0, 100)?;
/// density.set([-1, 0, 0], 2.0)?;
/// assert_eq!(density.sample_index([-0.5, 0.0, 0.0]), 1.0);
/// # Ok::<(), sr_volume::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct SparseGrid {
    transform: Transform,
    background: f32,
    max_bricks: usize,
    bricks: BTreeMap<[i32; 3], Brick>,
}

fn address(index: [i32; 3]) -> ([i32; 3], usize) {
    let brick = index.map(|v| v.div_euclid(BRICK_SIDE));
    let [x, y, z] = index.map(|v| v.rem_euclid(BRICK_SIDE) as usize);
    (brick, x + 8 * y + 64 * z)
}

impl SparseGrid {
    pub fn new(transform: Transform, background: f32, max_bricks: usize) -> Result<Self, Error> {
        if !background.is_finite() {
            return Err(Error::Invalid("background must be finite"));
        }
        Ok(Self { transform, background, max_bricks, bricks: BTreeMap::new() })
    }

    pub fn transform(&self) -> Transform {
        self.transform
    }

    pub fn background(&self) -> f32 {
        self.background
    }

    pub fn brick_count(&self) -> usize {
        self.bricks.len()
    }

    /// Approximate checkpoint accounting: scalar storage plus map/node bookkeeping.
    pub fn bytes(&self) -> usize {
        self.bricks.len().saturating_mul(BRICK_VOXELS * 4 + 128).saturating_add(std::mem::size_of::<Self>())
    }

    /// Brick coordinates (not voxel origins) in lexicographic order, with x-fastest samples.
    pub fn bricks(&self) -> impl Iterator<Item = ([i32; 3], &[f32; BRICK_VOXELS])> {
        self.bricks.iter().map(|(key, brick)| (*key, brick.values.as_ref()))
    }

    pub fn value(&self, index: [i32; 3]) -> f32 {
        let (key, offset) = address(index);
        self.bricks.get(&key).map_or(self.background, |b| b.values[offset])
    }

    /// Changes one sample atomically. Background-only bricks are reclaimed immediately.
    pub fn set(&mut self, index: [i32; 3], value: f32) -> Result<(), Error> {
        if !value.is_finite() {
            return Err(Error::Invalid("voxel values must be finite"));
        }
        let (key, offset) = address(index);
        if !self.bricks.contains_key(&key) {
            if value == self.background {
                return Ok(());
            }
            if self.bricks.len() >= self.max_bricks {
                return Err(Error::Limit("brick count"));
            }
            self.bricks.insert(key, Brick { values: Box::new([self.background; BRICK_VOXELS]), active: 0 });
        }
        let brick = self.bricks.get_mut(&key).expect("inserted above");
        brick.active -= u16::from(brick.values[offset] != self.background);
        brick.active += u16::from(value != self.background);
        brick.values[offset] = value;
        if brick.active == 0 {
            self.bricks.remove(&key);
        }
        Ok(())
    }

    /// Trilinear interpolation at continuous index coordinates. Nonfinite/out-of-range
    /// positions sample the background. Interpolation across absent bricks uses background.
    pub fn sample_index(&self, p: [f64; 3]) -> f32 {
        if p.iter().any(|v| !v.is_finite() || *v < f64::from(i32::MIN) - 1.0 || *v > f64::from(i32::MAX) + 1.0) {
            return self.background;
        }
        let lo = p.map(|v| v.floor() as i64);
        let f: [f64; 3] = std::array::from_fn(|i| p[i] - lo[i] as f64);
        let mut sum = 0.0;
        for z in 0..2 {
            for y in 0..2 {
                for x in 0..2 {
                    let delta = [x, y, z];
                    let q: [i64; 3] = std::array::from_fn(|i| lo[i] + delta[i]);
                    let v = if q.iter().all(|v| i32::try_from(*v).is_ok()) {
                        self.value(q.map(|v| v as i32))
                    } else {
                        self.background
                    };
                    let weight = (0..3).map(|i| if delta[i] == 0 { 1.0 - f[i] } else { f[i] }).product::<f64>();
                    sum += f64::from(v) * weight;
                }
            }
        }
        sum.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32
    }

    pub fn sample_world(&self, world: [f64; 3]) -> f32 {
        self.sample_index(self.transform.world_to_index(world))
    }
}

/// Decoder budgets, applied to the entire file rather than separately to each channel.
#[derive(Clone, Copy, Debug)]
pub struct CacheLimits {
    pub max_bytes: u64,
    pub max_bricks: usize,
    pub max_grids: usize,
}

impl Default for CacheLimits {
    fn default() -> Self {
        Self { max_bytes: 256 << 20, max_bricks: 100_000, max_grids: MAX_GRIDS }
    }
}

/// Named fields in a single volume frame. Channel identities are unique ASCII names.
#[derive(Clone, Debug, Default)]
pub struct Volume {
    grids: BTreeMap<String, std::sync::Arc<SparseGrid>>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_NAME && name.bytes().all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
}

impl Volume {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: &str, grid: SparseGrid) -> Result<(), Error> {
        if !valid_name(name) {
            return Err(Error::Invalid("channel name must contain 1..64 ASCII letters, digits, _, . or -"));
        }
        if self.grids.contains_key(name) {
            return Err(Error::Invalid("duplicate channel name"));
        }
        if self.grids.len() >= MAX_GRIDS {
            return Err(Error::Limit("channel count"));
        }
        self.grids.insert(name.to_owned(), std::sync::Arc::new(grid));
        Ok(())
    }

    pub fn grid(&self, name: &str) -> Option<&SparseGrid> {
        self.grids.get(name).map(AsRef::as_ref)
    }

    /// Shares an immutable field without copying sparse bricks for each scene instance.
    pub fn shared_grid(&self, name: &str) -> Option<std::sync::Arc<SparseGrid>> {
        self.grids.get(name).cloned()
    }

    pub fn grids(&self) -> impl Iterator<Item = (&str, &SparseGrid)> {
        self.grids.iter().map(|(name, grid)| (name.as_str(), grid.as_ref()))
    }

    /// Conservative estimate of owned grid storage, including channel/tree overhead.
    /// Arc clones share this storage; callers account shared frames only once.
    pub fn bytes(&self) -> usize {
        self.grids.iter().fold(std::mem::size_of::<Self>(), |bytes, (name, grid)| {
            bytes.saturating_add(name.len() + 128).saturating_add(grid.bytes())
        })
    }

    /// Writes canonical SRVOL version 1 bytes. Grids and bricks are sorted; no timestamps,
    /// uninitialized padding or platform-endian data enter the file.
    pub fn write(&self, mut out: impl Write) -> Result<(), Error> {
        out.write_all(MAGIC)?;
        out.write_all(&VERSION.to_le_bytes())?;
        out.write_all(&(self.grids.len() as u32).to_le_bytes())?;
        for (name, grid) in &self.grids {
            let count = u32::try_from(grid.bricks.len()).map_err(|_| Error::Limit("brick count exceeds u32"))?;
            out.write_all(&(name.len() as u16).to_le_bytes())?;
            out.write_all(name.as_bytes())?;
            out.write_all(&grid.background.to_le_bytes())?;
            for v in grid.transform.columns() {
                out.write_all(&v.to_le_bytes())?;
            }
            out.write_all(&count.to_le_bytes())?;
            for (key, brick) in &grid.bricks {
                for v in key {
                    out.write_all(&v.to_le_bytes())?;
                }
                for v in brick.values.iter() {
                    out.write_all(&v.to_le_bytes())?;
                }
            }
        }
        Ok(())
    }

    /// Streams a complete cache frame. Rejects unknown versions, duplicate/unsorted keys,
    /// nonfinite data, invalid transforms, truncated payloads and trailing bytes.
    pub fn read(input: impl Read, limits: CacheLimits) -> Result<Self, Error> {
        let mut r = Decoder { input, remaining: limits.max_bytes };
        if &r.bytes::<8>()? != MAGIC {
            return Err(Error::Invalid("cache magic"));
        }
        if r.u32()? != VERSION {
            return Err(Error::Invalid("unsupported cache version"));
        }
        let grids = r.u32()? as usize;
        if grids > limits.max_grids.min(MAX_GRIDS) {
            return Err(Error::Limit("channel count"));
        }
        let mut volume = Self::new();
        let mut total_bricks = 0usize;
        let mut previous_name = String::new();
        for _ in 0..grids {
            let n = u16::from_le_bytes(r.bytes()?) as usize;
            if n == 0 || n > MAX_NAME {
                return Err(Error::Invalid("channel name length"));
            }
            let mut name = [0u8; MAX_NAME];
            r.read_exact(&mut name[..n])?;
            let name = std::str::from_utf8(&name[..n]).map_err(|_| Error::Invalid("channel name encoding"))?;
            if !valid_name(name) || name <= previous_name.as_str() {
                return Err(Error::Invalid("channel names must be valid, unique and sorted"));
            }
            previous_name = name.to_owned();
            let background = f32::from_le_bytes(r.bytes()?);
            let mut matrix = [0.0; 16];
            for value in &mut matrix {
                *value = f64::from_le_bytes(r.bytes()?);
            }
            let mut grid = SparseGrid::new(Transform::new(matrix)?, background, limits.max_bricks)?;
            let count = r.u32()? as usize;
            total_bricks = total_bricks.checked_add(count).ok_or(Error::Limit("brick count overflow"))?;
            if total_bricks > limits.max_bricks {
                return Err(Error::Limit("total brick count"));
            }
            if count as u64 * (12 + BRICK_VOXELS as u64 * 4) > r.remaining {
                return Err(Error::Limit("cache bytes"));
            }
            let mut previous_key = None;
            for _ in 0..count {
                let mut key = [0; 3];
                for v in &mut key {
                    *v = i32::from_le_bytes(r.bytes()?);
                }
                if key.iter().any(|v| !(i32::MIN / 8..=i32::MAX / 8).contains(v)) {
                    return Err(Error::Invalid("brick coordinate exceeds voxel index range"));
                }
                if previous_key.is_some_and(|p| key <= p) {
                    return Err(Error::Invalid("brick keys must be unique and sorted"));
                }
                previous_key = Some(key);
                let mut brick = Brick { values: Box::new([background; BRICK_VOXELS]), active: 0 };
                for value in brick.values.iter_mut() {
                    *value = f32::from_le_bytes(r.bytes()?);
                    if !value.is_finite() {
                        return Err(Error::Invalid("voxel values must be finite"));
                    }
                    brick.active += u16::from(*value != background);
                }
                if brick.active == 0 {
                    return Err(Error::Invalid("background-only brick"));
                }
                grid.bricks.insert(key, brick);
            }
            volume.insert(name, grid)?;
        }
        // Reading one byte here distinguishes an exact byte budget from trailing payload.
        // read_exact retries interrupted reads, unlike a single Read::read call.
        match r.input.read_exact(&mut [0]) {
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(volume),
            Err(e) => Err(e.into()),
            Ok(()) => Err(Error::Invalid("trailing cache bytes")),
        }
    }
}

struct Decoder<R> {
    input: R,
    remaining: u64,
}

impl<R: Read> Decoder<R> {
    fn read_exact(&mut self, out: &mut [u8]) -> Result<(), Error> {
        self.remaining = self.remaining.checked_sub(out.len() as u64).ok_or(Error::Limit("cache bytes"))?;
        self.input.read_exact(out)?;
        Ok(())
    }

    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let mut bytes = [0; N];
        self.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }
}
