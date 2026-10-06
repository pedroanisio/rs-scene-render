//! Validated GPU medium instances and sparse brick packing shared with the path tracer.
//! Records live in the existing geometry binding, preserving the downlevel eight-storage
//! binding limit even when the denoising pass uses its two additional buffers.

use std::sync::Arc;

use glam::{DMat3, DMat4, DVec3};
use sr_volume::medium::{March, Medium};
use sr_volume::thermal::{blackbody_table, TABLE_INTERVALS};

const RECORD_ROWS: usize = 64;
const BRICK_ROWS: usize = 129;
const MAX_MEDIA: usize = 64;
/// Voxels along a side of the cells of the shadow march's skip map.
const OCCUPANCY_CELL: i32 = 4;
/// Largest skip map, in cells (512 KiB of bits); a grid whose extent needs more is marched without
/// skipping.
const MAX_OCCUPANCY_CELLS: u64 = 1 << 22;
/// A cell is skipped by the shadow march only when the optical depth its samples could add along
/// any ray through the domain is below this: the largest |density| a sample in the cell can read,
/// times the density scale (which already includes the object's opacity) and the extinction, times
/// the longest path inside the domain. Transmittance is `exp(-optical depth)`, and 1e-10 is about
/// 300 times smaller than half a unit in the last place of an f32 near 1, so what is left out is far
/// below the resolution of the result; it is not zero, and is bounded the same way for any scene.
const SKIPPED_OPTICAL_DEPTH: f64 = 1e-10;
/// Largest brick directory, in entries (4 MiB): a dense table of one row index per possible brick
/// position in the density grid's bounding box, replacing the shader's binary search of the brick
/// list. A grid whose box needs more keeps the search.
const MAX_DIRECTORY_ENTRIES: u64 = 1 << 20;

/// Lowest brick key and per-axis extent of the grid's bricks, when a directory fits.
fn directory_extent(grid: &sr_volume::SparseGrid) -> Option<([i32; 3], [u32; 3])> {
    let mut lo = [i32::MAX; 3];
    let mut hi = [i32::MIN; 3];
    for (key, _) in grid.bricks() {
        for a in 0..3 {
            lo[a] = lo[a].min(key[a]);
            hi[a] = hi[a].max(key[a]);
        }
    }
    if lo[0] > hi[0] {
        return None;
    }
    let dims: [u32; 3] = std::array::from_fn(|a| (i64::from(hi[a]) - i64::from(lo[a]) + 1) as u32);
    (dims.iter().map(|d| u64::from(*d)).product::<u64>() <= MAX_DIRECTORY_ENTRIES).then_some((lo, dims))
}

/// The density at or below which a cell may be skipped, in the units of the grid.
fn negligible_density(medium: &Medium, diameter: f64) -> f32 {
    let optical = medium.optical();
    let factor = (optical.density_scale * optical.extinction).abs() * diameter;
    if factor > 0.0 {
        (SKIPPED_OPTICAL_DEPTH / factor) as f32
    } else {
        f32::INFINITY
    }
}

/// Cell range, in `OCCUPANCY_CELL`-voxel cells, that can hold density: the first cell and the extent
/// of the bounding box of the bricks, widened by the one cell an interpolated sample reaches into.
fn occupancy_extent(grid: &sr_volume::SparseGrid, negligible: f32) -> Option<([i32; 3], [u32; 3])> {
    // outside the bricks the density is the background, which must be negligible too
    let background = grid.background().abs();
    if background.is_nan() || background > negligible {
        return None;
    }
    let per_brick = 8 / OCCUPANCY_CELL;
    let mut lo = [i32::MAX; 3];
    let mut hi = [i32::MIN; 3];
    for (key, _) in grid.bricks() {
        for a in 0..3 {
            lo[a] = lo[a].min(key[a]);
            hi[a] = hi[a].max(key[a]);
        }
    }
    if lo[0] > hi[0] {
        return None;
    }
    let min: [i32; 3] = std::array::from_fn(|a| lo[a] * per_brick - 1);
    let dims: [u32; 3] = std::array::from_fn(|a| ((hi[a] - lo[a]) * per_brick + 3) as u32);
    let cells = dims.iter().map(|d| u64::from(*d)).product::<u64>();
    (cells <= MAX_OCCUPANCY_CELLS).then_some((min, dims))
}

/// Bit per cell of `occupancy_extent`: set when any voxel that a sample inside the cell can read
/// (the cell's voxels and the next one on each side) is above `negligible`. Cells outside the
/// extent and cells with the bit clear hold only negligible density.
fn occupancy(grid: &sr_volume::SparseGrid, negligible: f32) -> Option<([i32; 3], [u32; 3], Vec<u32>)> {
    let (min, dims) = occupancy_extent(grid, negligible)?;
    let cells = dims.iter().map(|d| *d as usize).product::<usize>();
    let mut words = vec![0u32; cells.div_ceil(32)];
    let cell = OCCUPANCY_CELL;
    // cells c with cell*c <= v <= cell*c + cell
    let range = |v: i32| ((v - cell).div_euclid(cell) + ((v - cell).rem_euclid(cell) != 0) as i32, v.div_euclid(cell));
    for (key, values) in grid.bricks() {
        for (i, &value) in values.iter().enumerate() {
            if value.abs() <= negligible {
                continue;
            }
            let voxel = [8 * key[0] + (i & 7) as i32, 8 * key[1] + ((i >> 3) & 7) as i32, 8 * key[2] + (i >> 6) as i32];
            let r = voxel.map(range);
            for z in r[2].0..=r[2].1 {
                for y in r[1].0..=r[1].1 {
                    for x in r[0].0..=r[0].1 {
                        let rel = [x - min[0], y - min[1], z - min[2]];
                        let index =
                            rel[0] as usize + dims[0] as usize * (rel[1] as usize + dims[1] as usize * rel[2] as usize);
                        words[index / 32] |= 1 << (index % 32);
                    }
                }
            }
        }
    }
    Some((min, dims, words))
}

/// Whether the shadow march may skip empty cells of this medium: the density must be a single
/// still frame (no second frame to blend, no advection) so a sample reads only the grid it is in.
fn skips_empty_space(medium: &Medium) -> bool {
    medium.next_density().is_none() && medium.advection().is_none()
}

#[derive(Clone, Debug)]
pub struct VolumeDraw {
    medium: Arc<Medium>,
    march: March,
    diameter: f64,
    cast_shadow: bool,
    receive_shadow: bool,
    thermal_color: DMat3,
    light_grid: Option<LightGridRequest>,
}

fn f32_ok(v: f64) -> bool {
    v.is_finite() && (v as f32).is_finite() && (v == 0.0 || v as f32 != 0.0)
}

impl VolumeDraw {
    /// Checks precision, coefficient and work limits before allocating GPU buffers.
    pub fn new(medium: Arc<Medium>, march: March) -> Result<Self, String> {
        if !f32_ok(march.step_size) || march.step_size <= 0.0 || march.max_steps == 0 || march.max_steps > 1_048_576 {
            return Err("volume stepSize must be positive f32 and maxSteps in 1..1048576".into());
        }
        let object = DMat4::from_cols_array(&medium.transform().columns());
        for matrix in [object, object.inverse()] {
            if matrix.to_cols_array().iter().any(|v| !f32_ok(*v)) || !matrix.as_mat4().inverse().is_finite() {
                return Err("volume transform cannot be represented invertibly in f32".into());
            }
        }
        for grid in medium
            .density_grids()
            .chain(medium.temperature().into_iter().flat_map(|t| std::iter::once(t.grid()).chain(t.next_grid())))
            .chain(medium.advection().into_iter().flatten().flat_map(|t| t.grids()))
        {
            let index = DMat4::from_cols_array(&grid.transform().columns());
            let matrix = (object * index).inverse();
            if matrix.to_cols_array().iter().any(|v| !f32_ok(*v)) || !matrix.as_mat4().inverse().is_finite() {
                return Err("volume grid transform cannot be represented invertibly in f32".into());
            }
            if grid.bricks().any(|(key, _)| key.iter().any(|k| k.unsigned_abs() >= (1 << 20) - 1)) {
                return Err("volume voxel coordinates exceed exact f32 interpolation range; recenter the cache".into());
            }
        }
        for trace in medium.advection().into_iter().flatten() {
            if !f32_ok(trace.elapsed()) || trace.displacement().into_iter().any(|v| !f32_ok(v)) {
                return Err("volume advection time or displacement exceeds GPU precision".into());
            }
            let index = DMat4::from_cols_array(&trace.grids()[0].transform().columns()).inverse();
            if index.to_cols_array().iter().any(|v| !f32_ok(*v)) || !index.as_mat4().inverse().is_finite() {
                return Err("velocity transform cannot be represented invertibly in f32".into());
            }
            if let Some(bounds) = medium.bounds() {
                for k in 0..8 {
                    let p = DVec3::from_array(std::array::from_fn(|i| {
                        if k & (1 << i) == 0 {
                            bounds.min()[i] - trace.displacement()[i]
                        } else {
                            bounds.max()[i] + trace.displacement()[i]
                        }
                    }));
                    for matrix in [index, object] {
                        if !p.as_vec3().is_finite() || !matrix.as_mat4().transform_point3(p.as_vec3()).is_finite() {
                            return Err("advected sampling domain exceeds GPU precision".into());
                        }
                    }
                }
            }
        }
        let optical = medium.optical();
        let mut scalars = vec![optical.density_scale, optical.extinction, optical.anisotropy];
        scalars.extend(optical.albedo);
        scalars.extend(optical.emission);
        if scalars.into_iter().any(|v| !f32_ok(v)) || (optical.anisotropy as f32).abs() >= 1.0 {
            return Err("volume optical coefficients exceed GPU precision".into());
        }
        let peak = density_peak(&medium);
        for coefficient in [optical.extinction, optical.emission[0], optical.emission[1], optical.emission[2]] {
            if !f32_ok(peak * optical.density_scale * coefficient) {
                return Err("volume density and coefficient product exceeds f32".into());
            }
        }
        let diameter = if let Some(bounds) = medium.bounds() {
            if bounds.min().into_iter().chain(bounds.max()).any(|v| !f32_ok(v)) {
                return Err("volume bounds exceed f32".into());
            }
            let mut lo = DVec3::splat(f64::INFINITY);
            let mut hi = DVec3::splat(f64::NEG_INFINITY);
            for k in 0..8 {
                let p = DVec3::from_array(std::array::from_fn(|i| {
                    if k & (1 << i) == 0 {
                        bounds.min()[i]
                    } else {
                        bounds.max()[i]
                    }
                }));
                let p = object.transform_point3(p);
                if p.to_array().iter().any(|v| !f32_ok(*v)) {
                    return Err("transformed volume bounds exceed f32".into());
                }
                lo = lo.min(p);
                hi = hi.max(p);
            }
            (hi - lo).length()
        } else {
            0.0
        };
        if (diameter / march.step_size).ceil() + 2.0 > f64::from(march.max_steps) {
            return Err("volume domain exceeds maxSteps at this stepSize; increase maxSteps or stepSize".into());
        }
        let draw = Self {
            medium,
            march,
            diameter,
            cast_shadow: true,
            receive_shadow: true,
            thermal_color: DMat3::IDENTITY,
            light_grid: None,
        };
        draw.validate_thermal()?;
        Ok(draw)
    }

    fn validate_thermal(&self) -> Result<(), String> {
        if !self.thermal_color.is_finite() {
            return Err("thermal color transform must be finite".into());
        }
        if let Some(t) = self.medium.temperature() {
            if !f32_ok(t.scale()) || !f32_ok(t.emission_scale()) {
                return Err("temperature and emission scales exceed GPU precision".into());
            }
            let peak = density_peak(&self.medium) * self.medium.optical().density_scale;
            // Include constant emission and the complete interpolation table in the bound.
            for rgb in blackbody_table() {
                let rgb = (self.thermal_color * DVec3::from_array(*rgb)).max(DVec3::ZERO) * t.emission_scale();
                for (i, v) in rgb.to_array().into_iter().enumerate() {
                    let total = v + self.medium.optical().emission[i];
                    if !(v as f32).is_finite() || !((peak * total) as f32).is_finite() {
                        return Err("thermal emission exceeds finite GPU coefficients".into());
                    }
                }
            }
        }
        Ok(())
    }

    /// Linear sRGB to the renderer's linear working space (matrix columns).
    /// Thermal colors outside the destination gamut are clipped to nonnegative RGB.
    pub fn with_thermal_color(mut self, matrix: DMat3) -> Result<Self, String> {
        self.thermal_color = matrix;
        self.validate_thermal()?;
        Ok(self)
    }

    pub fn medium(&self) -> &Arc<Medium> {
        &self.medium
    }

    pub fn march(&self) -> March {
        self.march
    }

    /// Lights this medium's in-scattering from per-light grids built once a frame (`lighting="grid"`):
    /// `cell` voxels between grid nodes, `dome_directions` fixed dome directions, and the most memory
    /// in MiB the pass's grids may take.
    pub fn with_light_grid(mut self, cell: u32, dome_directions: u32, memory_mib: u64) -> Result<Self, String> {
        if !(1..=64).contains(&cell) || !(8..=512).contains(&dome_directions) || !(1..=4096).contains(&memory_mib) {
            return Err(
                "lightGridCell must be 1-64, lightGridDomeDirections 8-512 and lightGridMemoryMiB 1-4096".into()
            );
        }
        self.light_grid = Some(LightGridRequest { cell, dome_directions, memory_bytes: memory_mib << 20 });
        Ok(self)
    }

    /// A note for `stats.unsupported` when grid lighting is least accurate: the grid interpolates
    /// transmittance between nodes, which is a poor model where a cell's optical depth is well
    /// above 1. None for exact lighting or a medium thin enough for its cells.
    pub fn light_grid_note(&self) -> Option<String> {
        let request = self.light_grid?;
        let m = &self.medium;
        let object = DMat4::from_cols_array(&m.transform().columns());
        let index = DMat4::from_cols_array(&m.density().transform().columns());
        let world = object * index;
        let voxel = (0..3).map(|axis| world.col(axis).truncate().length()).fold(f64::INFINITY, f64::min);
        let peak = m
            .density()
            .bricks()
            .flat_map(|(_, values)| values.iter().copied())
            .chain(std::iter::once(m.density().background()))
            .fold(0.0f32, |peak, v| peak.max(v.abs()));
        let spacing = voxel * f64::from(request.cell);
        let optical = m.optical();
        let depth = optical.extinction * optical.density_scale * f64::from(peak) * spacing;
        (depth.is_finite() && depth > LIGHT_GRID_CELL_OPTICAL_DEPTH).then(|| {
            format!(
                "lighting=\"grid\" lights this medium from nodes {spacing:.2} units apart and the optical depth across one cell reaches {depth:.1}; the grid is less accurate where cells are that thick, use lighting=\"exact\" or a smaller lightGridCell"
            )
        })
    }

    pub fn with_shadows(mut self, cast: bool, receive: bool) -> Self {
        self.cast_shadow = cast;
        self.receive_shadow = receive;
        self
    }
}

pub(crate) fn bytes(volumes: &[VolumeDraw]) -> u64 {
    volumes
        .iter()
        .map(|v| {
            let thermal = v.medium.temperature().map_or(0, |t| {
                (t.grid().brick_count() + t.next_grid().map_or(0, |g| g.brick_count())) * BRICK_ROWS
                    + TABLE_INTERVALS
                    + 1
            });
            let density = v.medium.density_grids().map(|g| g.brick_count()).sum::<usize>() * BRICK_ROWS;
            let velocity = v
                .medium
                .advection()
                .into_iter()
                .flatten()
                .flat_map(|t| t.grids())
                .map(|g| g.brick_count())
                .sum::<usize>()
                * BRICK_ROWS;
            let directory = directory_extent(v.medium.density())
                .map_or(0, |(_, d)| d.iter().map(|x| *x as usize).product::<usize>().div_ceil(4));
            let occupancy = if skips_empty_space(&v.medium) {
                occupancy_extent(v.medium.density(), negligible_density(&v.medium, v.diameter)).map_or(
                    0,
                    |(_, dims)| {
                        let cells = dims.iter().map(|d| *d as usize).product::<usize>();
                        cells.div_ceil(32).div_ceil(4)
                    },
                )
            } else {
                0
            };
            (RECORD_ROWS as u64
                + density as u64
                + thermal as u64
                + velocity as u64
                + occupancy as u64
                + directory as u64)
                * 16
        })
        .sum()
}

/// Global work is bounded even when different domains introduce extra step boundaries.
pub(crate) fn validate(volumes: &[VolumeDraw]) -> Result<(), String> {
    if volumes.len() > MAX_MEDIA {
        return Err(format!("a 3D pass supports at most {MAX_MEDIA} volume domains"));
    }
    if volumes.is_empty() {
        return Ok(());
    }
    let step = volumes.iter().map(|v| v.march.step_size).fold(f64::INFINITY, f64::min);
    let cap = volumes.iter().map(|v| v.march.max_steps).min().unwrap_or(1);
    let steps = volumes.iter().map(|v| (v.diameter / step).ceil() + 2.0).sum::<f64>();
    if steps > f64::from(cap) {
        return Err("combined volume domains exceed maxSteps at the smallest stepSize".into());
    }
    Ok(())
}

/// Returns the row offset and count, sample budget and reserved word for shader parameters.
pub(crate) fn pack(out: &mut Vec<[f32; 4]>, volumes: &[VolumeDraw]) -> [u32; 4] {
    let offset = out.len();
    out.resize(offset + volumes.len() * RECORD_ROWS, [0.0; 4]);
    for (i, volume) in volumes.iter().enumerate() {
        let m = volume.medium();
        let base = offset + i * RECORD_ROWS;
        let world = DMat4::from_cols_array(&m.transform().columns());
        let index = DMat4::from_cols_array(&m.density().transform().columns());
        out[base..base + 4].copy_from_slice(&world.inverse().as_mat4().to_cols_array_2d());
        let o = m.optical();
        let bounds = m.bounds();
        let lo = bounds.map_or([0.0; 3], |b| b.min());
        let hi = bounds.map_or([0.0; 3], |b| b.max());
        out[base + 4] = [lo[0] as f32, lo[1] as f32, lo[2] as f32, o.density_scale as f32];
        out[base + 5] = [hi[0] as f32, hi[1] as f32, hi[2] as f32, o.extinction as f32];
        out[base + 6] = [o.albedo[0] as f32, o.albedo[1] as f32, o.albedo[2] as f32, o.anisotropy as f32];
        out[base + 7] = [o.emission[0] as f32, o.emission[1] as f32, o.emission[2] as f32, 0.0];
        out[base + 8..base + 12].copy_from_slice(&(world * index).inverse().as_mat4().to_cols_array_2d());
        out[base + 12] = pack_grid(out, m.density());
        out[base + 12][3] = volume.march.step_size as f32;
        out[base + 13] = [
            volume.cast_shadow as u8 as f32,
            volume.receive_shadow as u8 as f32,
            volume.light_grid.is_some() as u8 as f32,
            0.0,
        ];
        if let Some(next) = m.next_density() {
            let index = DMat4::from_cols_array(&next.transform().columns());
            out[base + 24..base + 28].copy_from_slice(&(world * index).inverse().as_mat4().to_cols_array_2d());
            out[base + 28] = pack_grid(out, next);
            out[base + 14] = [m.frame_blend() as f32, 1.0, 0.0, 0.0];
        }
        if let Some(traces) = m.advection() {
            out[base + 14][2] = 1.0;
            out[base + 56..base + 60].copy_from_slice(&world.as_mat4().to_cols_array_2d());
            for (i, trace) in traces.iter().enumerate() {
                let row = base + 40 + i * 8;
                let inverse = DMat4::from_cols_array(&trace.grids()[0].transform().columns()).inverse();
                out[row..row + 4].copy_from_slice(&inverse.as_mat4().to_cols_array_2d());
                for (component, grid) in trace.grids().iter().enumerate() {
                    out[row + 4 + component] = pack_grid(out, grid);
                }
                out[row + 7] = [trace.elapsed() as f32, 3.0, 2.0, 0.0];
            }
        }
        if let Some(t) = m.temperature() {
            let index = DMat4::from_cols_array(&t.grid().transform().columns());
            out[base + 16..base + 20].copy_from_slice(&(world * index).inverse().as_mat4().to_cols_array_2d());
            out[base + 20] = pack_grid(out, t.grid());
            if let Some(next) = t.next_grid() {
                let index = DMat4::from_cols_array(&next.transform().columns());
                out[base + 32..base + 36].copy_from_slice(&(world * index).inverse().as_mat4().to_cols_array_2d());
                out[base + 36] = pack_grid(out, next);
            }
            out[base + 21] = [t.scale() as f32, 1.0, f32::from_bits(out.len() as u32), 0.0];
            out.extend(blackbody_table().iter().map(|rgb| {
                let rgb = (volume.thermal_color * DVec3::from_array(*rgb)).max(DVec3::ZERO) * t.emission_scale();
                [rgb.x as f32, rgb.y as f32, rgb.z as f32, 0.0]
            }));
        }
        if let Some((lo, dims)) = directory_extent(m.density()) {
            let first = f32::to_bits(out[base + 12][2]) as usize;
            let entries: usize = dims.iter().map(|d| *d as usize).product();
            let mut table = vec![u32::MAX; entries.next_multiple_of(4)];
            for (i, (key, _)) in m.density().bricks().enumerate() {
                let rel = [(key[0] - lo[0]) as usize, (key[1] - lo[1]) as usize, (key[2] - lo[2]) as usize];
                table[rel[0] + dims[0] as usize * (rel[1] + dims[1] as usize * rel[2])] =
                    (first + i * BRICK_ROWS) as u32;
            }
            let offset = out.len();
            out.extend(table.chunks(4).map(|c| std::array::from_fn(|i| f32::from_bits(c[i]))));
            out[base + 23] = [
                f32::from_bits(lo[0] as u32),
                f32::from_bits(lo[1] as u32),
                f32::from_bits(lo[2] as u32),
                f32::from_bits(offset as u32),
            ];
            out[base + 29] = [f32::from_bits(dims[0]), f32::from_bits(dims[1]), f32::from_bits(dims[2]), 1.0];
        }
        if skips_empty_space(m) {
            if let Some((min, dims, words)) = occupancy(m.density(), negligible_density(m, volume.diameter)) {
                let offset = out.len();
                out.extend(
                    words.chunks(4).map(|c| std::array::from_fn(|i| f32::from_bits(c.get(i).copied().unwrap_or(0)))),
                );
                out[base + 15] = [
                    f32::from_bits(min[0] as u32),
                    f32::from_bits(min[1] as u32),
                    f32::from_bits(min[2] as u32),
                    f32::from_bits(offset as u32),
                ];
                out[base + 22] = [f32::from_bits(dims[0]), f32::from_bits(dims[1]), f32::from_bits(dims[2]), 1.0];
            }
        }
    }
    [offset as u32, volumes.len() as u32, volumes.iter().map(|v| v.march.max_steps).min().unwrap_or(0), 0]
}

fn density_peak(medium: &Medium) -> f64 {
    medium
        .density_grids()
        .map(|g| g.bricks().flat_map(|(_, v)| v.iter().copied()).fold(g.background(), f32::max) as f64)
        .fold(0.0, f64::max)
}

fn pack_grid(out: &mut Vec<[f32; 4]>, grid: &sr_volume::SparseGrid) -> [f32; 4] {
    let info = [grid.background(), f32::from_bits(grid.brick_count() as u32), f32::from_bits(out.len() as u32), 0.0];
    for (key, values) in grid.bricks() {
        out.push([f32::from_bits(key[0] as u32), f32::from_bits(key[1] as u32), f32::from_bits(key[2] as u32), 0.0]);
        out.extend(values.as_chunks::<4>().0.iter().copied());
    }
    info
}

/// What a medium asks for with `lighting="grid"`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightGridRequest {
    pub cell: u32,
    pub dome_directions: u32,
    pub memory_bytes: u64,
}

/// The world-space lattice and buffer layout of a pass's light grids.
#[derive(Clone, Debug, PartialEq)]
pub struct LightGridPlan {
    pub origin: [f32; 3],
    /// World distance between nodes.
    pub spacing: f32,
    pub nodes: [u32; 3],
    /// Analytic lights, one scalar slot each; the dome's fixed directions follow when `directional`.
    pub lights: u32,
    pub dome_directions: u32,
    /// Dome radiance pre-integrated per node (a medium with isotropic scattering asks for it).
    pub radiance: bool,
    /// One scalar slot per dome direction (a medium with anisotropic scattering asks for them).
    pub directional: bool,
    pub rows_per_slot: u32,
    pub scalar_slots: u32,
    pub bytes: u64,
}

impl LightGridPlan {
    pub fn node_count(&self) -> u32 {
        self.nodes.iter().product()
    }

    fn radiance_row(&self) -> u32 {
        4 + self.scalar_slots * self.rows_per_slot
    }
}

/// Optical depth across one cell above which grid lighting is reported as banding.
const LIGHT_GRID_CELL_OPTICAL_DEPTH: f64 = 4.0;

/// Most nodes a grid may have: the dome kernel runs one thread a node in at most 65535 groups.
const MAX_GRID_NODES: u64 = 64 * 65535;

/// The grids a pass needs, or none when no medium asks for them. The lattice covers the bounds of
/// the media that ask; its spacing is the finest of their `lightGridCell` voxels. Memory over the
/// strictest request is an error.
pub fn plan_light_grid(volumes: &[VolumeDraw], lights: u32, dome: bool) -> Result<Option<LightGridPlan>, String> {
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = DVec3::splat(f64::NEG_INFINITY);
    let mut spacing = f64::INFINITY;
    let (mut directions, mut memory) = (0u32, u64::MAX);
    let (mut radiance, mut directional) = (false, false);
    let mut any = false;
    for v in volumes {
        let Some(request) = v.light_grid else { continue };
        let m = v.medium();
        // A medium without bounds holds no density (a smoke that has not been born yet): nothing
        // in it scatters, so it needs no lattice.
        let Some(bounds) = m.bounds() else { continue };
        any = true;
        let object = DMat4::from_cols_array(&m.transform().columns());
        for k in 0..8 {
            let p = DVec3::from_array(std::array::from_fn(|i| {
                if k & (1 << i) == 0 {
                    bounds.min()[i]
                } else {
                    bounds.max()[i]
                }
            }));
            let w = object.transform_point3(p);
            lo = lo.min(w);
            hi = hi.max(w);
        }
        let index = DMat4::from_cols_array(&m.density().transform().columns());
        let world = object * index;
        let voxel = (0..3).map(|axis| world.col(axis).truncate().length()).fold(f64::INFINITY, f64::min);
        if !voxel.is_finite() || voxel <= 0.0 {
            return Err("lighting=\"grid\" needs a density grid with a valid transform".into());
        }
        spacing = spacing.min(voxel * f64::from(request.cell));
        directions = directions.max(request.dome_directions);
        memory = memory.min(request.memory_bytes);
        if m.optical().anisotropy == 0.0 {
            radiance = true;
        } else {
            directional = true;
        }
    }
    if !any {
        return Ok(None);
    }
    let extent = hi - lo;
    // counted in f64 and refused before any conversion: an absurd domain saturates a u32 axis and wraps a u64 product
    let axes: [f64; 3] = std::array::from_fn(|a| (extent[a] / spacing).ceil().max(1.0) + 1.0);
    let wanted = axes.iter().product::<f64>();
    if wanted.is_nan() || wanted > MAX_GRID_NODES as f64 {
        return Err(format!(
            "the volume light grid would have {wanted:.3e} nodes (at most {MAX_GRID_NODES}); raise lightGridCell"
        ));
    }
    let nodes: [u32; 3] = axes.map(|n| n as u32);
    let count = nodes.iter().map(|n| u64::from(*n)).product::<u64>();
    let (radiance, directional) = (radiance && dome, directional && dome);
    let scalar_slots = lights + if directional { directions } else { 0 };
    let rows_per_slot = count.div_ceil(4);
    let rows = 4 + rows_per_slot * u64::from(scalar_slots) + if radiance { count } else { 0 };
    let bytes = rows * 16;
    if bytes > memory {
        return Err(format!(
            "the volume light grid needs {:.1} MiB for {count} nodes and lightGridMemoryMiB allows {:.1} MiB; raise it, raise lightGridCell or use lighting=\"exact\"",
            bytes as f64 / 1048576.0,
            memory as f64 / 1048576.0
        ));
    }
    Ok(Some(LightGridPlan {
        origin: lo.as_vec3().to_array(),
        spacing: spacing as f32,
        nodes,
        lights,
        dome_directions: directions,
        radiance,
        directional,
        rows_per_slot: rows_per_slot as u32,
        scalar_slots,
        bytes,
    }))
}

/// Checks a pass's grids against the device's binding size, with the memory check of the sizing.
pub(crate) fn validate_light_grid(
    volumes: &[VolumeDraw],
    lights: u32,
    dome: bool,
    limits: &wgpu::Limits,
) -> Result<(), String> {
    if let Some(plan) = plan_light_grid(volumes, lights, dome)? {
        let cap = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
        if plan.bytes > cap {
            return Err(format!(
                "the volume light grid needs {} MiB and this device binds at most {} MiB; raise lightGridCell",
                plan.bytes.div_ceil(1 << 20),
                cap >> 20
            ));
        }
    }
    Ok(())
}

/// The grids' buffer contents: the four header rows, then zeroed data the compute passes fill.
pub fn light_grid_buffer(plan: &LightGridPlan) -> Vec<[f32; 4]> {
    let mut rows = vec![[0.0; 4]; (plan.bytes / 16) as usize];
    rows[0] = [plan.origin[0], plan.origin[1], plan.origin[2], plan.spacing];
    let flags = u32::from(plan.radiance) | (u32::from(plan.directional) << 1);
    rows[1] = [
        f32::from_bits(plan.nodes[0]),
        f32::from_bits(plan.nodes[1]),
        f32::from_bits(plan.nodes[2]),
        f32::from_bits(flags),
    ];
    rows[2] = [
        f32::from_bits(4),
        f32::from_bits(plan.rows_per_slot),
        f32::from_bits(plan.dome_directions),
        f32::from_bits(plan.lights),
    ];
    rows[3] = [f32::from_bits(plan.radiance_row()), 0.0, 0.0, 0.0];
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ball(voxel: f64) -> SparseGrid {
        let scale = Transform::new(DMat4::from_scale(DVec3::splat(voxel)).to_cols_array()).unwrap();
        let mut grid = SparseGrid::new(scale, 0.0, 64).unwrap();
        for z in 0..8 {
            for y in 0..8 {
                for x in 0..8 {
                    grid.set([x, y, z], 1.0).unwrap();
                }
            }
        }
        grid
    }

    fn draw(voxel: f64, anisotropy: f64, grid: Option<(u32, u32, u64)>) -> VolumeDraw {
        let medium = Medium::new(
            Arc::new(ball(voxel)),
            Some(sr_volume::medium::Bounds::new([0.0; 3], [8.0; 3]).unwrap()),
            Transform::identity(),
            sr_volume::medium::Optical { extinction: 1.0, anisotropy, ..Default::default() },
        )
        .unwrap();
        let draw = VolumeDraw::new(Arc::new(medium), March { step_size: 0.5, max_steps: 65536 }).unwrap();
        match grid {
            Some((cell, directions, mib)) => draw.with_light_grid(cell, directions, mib).unwrap(),
            None => draw,
        }
    }

    #[test]
    fn light_grid_plan_covers_the_asking_media_with_one_slot_per_light_and_direction() {
        // no medium asks: no grid
        assert_eq!(plan_light_grid(&[draw(1.0, 0.0, None)], 2, true).unwrap(), None);
        // an 8-unit domain at voxel 1 and cell 1: 9 nodes a side
        let plan = plan_light_grid(&[draw(1.0, 0.0, Some((1, 64, 128)))], 2, true).unwrap().unwrap();
        assert_eq!((plan.nodes, plan.spacing, plan.origin), ([9, 9, 9], 1.0, [0.0; 3]));
        assert!(plan.radiance && !plan.directional);
        assert_eq!(plan.scalar_slots, 2, "one scalar slot per light, none per direction for isotropic media");
        assert_eq!(plan.rows_per_slot, 729_u32.div_ceil(4));
        assert_eq!(plan.bytes, (4 + u64::from(plan.rows_per_slot) * 2 + 729) * 16);
        assert_eq!(light_grid_buffer(&plan).len() as u64 * 16, plan.bytes);
        // a coarser cell has fewer nodes; the finest request among the media sets the spacing
        let coarse = plan_light_grid(&[draw(1.0, 0.0, Some((4, 64, 128)))], 2, true).unwrap().unwrap();
        assert_eq!((coarse.nodes, coarse.spacing), ([3, 3, 3], 4.0));
        let fine = plan_light_grid(&[draw(1.0, 0.0, Some((4, 64, 128))), draw(1.0, 0.0, Some((2, 8, 64)))], 2, true)
            .unwrap()
            .unwrap();
        assert_eq!((fine.spacing, fine.dome_directions), (2.0, 64));
        // anisotropic scattering takes one scalar grid per dome direction instead of the radiance grid
        let aniso = plan_light_grid(&[draw(1.0, 0.5, Some((1, 64, 128)))], 2, true).unwrap().unwrap();
        assert!(!aniso.radiance && aniso.directional);
        assert_eq!(aniso.scalar_slots, 2 + 64);
        // no dome, no dome grids
        let dry = plan_light_grid(&[draw(1.0, 0.5, Some((1, 64, 128)))], 2, false).unwrap().unwrap();
        assert!(!dry.radiance && !dry.directional);
        assert_eq!(dry.scalar_slots, 2);
    }

    #[test]
    fn grid_lighting_of_a_medium_thick_across_a_cell_is_noted() {
        // the ball has density 1 and voxels of 1 unit: the optical depth across a cell is the extinction
        let thick = |extinction: f64, cell: u32, grid: bool| {
            let medium = Medium::new(
                Arc::new(ball(1.0)),
                Some(sr_volume::medium::Bounds::new([0.0; 3], [8.0; 3]).unwrap()),
                Transform::identity(),
                sr_volume::medium::Optical { extinction, ..Default::default() },
            )
            .unwrap();
            let draw = VolumeDraw::new(Arc::new(medium), March { step_size: 0.5, max_steps: 65536 }).unwrap();
            if grid {
                draw.with_light_grid(cell, 64, 128).unwrap()
            } else {
                draw
            }
        };
        assert_eq!(thick(30.0, 1, false).light_grid_note(), None, "exact lighting does not band");
        assert_eq!(thick(2.0, 1, true).light_grid_note(), None);
        let note = thick(30.0, 1, true).light_grid_note().unwrap();
        assert!(note.contains("reaches 30.0") && note.contains("1.00 units apart") && note.contains("exact"), "{note}");
        // a larger cell is thicker, not thinner
        assert!(thick(2.0, 4, true).light_grid_note().unwrap().contains("reaches 8.0"));
    }

    #[test]
    fn an_empty_medium_asking_for_the_grid_needs_no_lattice() {
        // a smoke that has not been born yet has no bricks and so no bounds
        let scale = Transform::new(DMat4::from_scale(DVec3::splat(1.0)).to_cols_array()).unwrap();
        let empty = Medium::new(
            Arc::new(SparseGrid::new(scale, 0.0, 64).unwrap()),
            None,
            Transform::identity(),
            sr_volume::medium::Optical { extinction: 1.0, ..Default::default() },
        )
        .unwrap();
        assert!(empty.bounds().is_none());
        let drawn = VolumeDraw::new(Arc::new(empty), March { step_size: 0.5, max_steps: 64 })
            .unwrap()
            .with_light_grid(1, 64, 128)
            .unwrap();
        assert_eq!(plan_light_grid(std::slice::from_ref(&drawn), 2, true).unwrap(), None);
        // beside a medium with density, only that one is covered
        let plan = plan_light_grid(&[drawn, draw(1.0, 0.0, Some((1, 64, 128)))], 2, true).unwrap().unwrap();
        assert_eq!(plan.nodes, [9, 9, 9]);
    }

    #[test]
    fn light_grid_over_its_memory_is_an_error_naming_the_attribute() {
        // an 8-unit domain at voxel 0.01 would have 801^3 nodes, more than a grid may
        let error = plan_light_grid(&[draw(0.01, 0.0, Some((1, 64, 4096)))], 2, true).unwrap_err();
        assert!(error.contains("raise lightGridCell"), "{error}");
        // 134^3 nodes fit the limit but not 1 MiB; the strictest request among the media decides
        let strict = [draw(0.06, 0.0, Some((1, 64, 4096))), draw(0.06, 0.0, Some((1, 64, 1)))];
        let error = plan_light_grid(&strict, 4, true).unwrap_err();
        assert!(error.contains("lightGridMemoryMiB allows 1.0 MiB"), "{error}");
        assert!(plan_light_grid(&strict[..1], 4, true).is_ok());
        assert!(draw(1.0, 0.0, None).with_light_grid(0, 64, 128).is_err());
        assert!(draw(1.0, 0.0, None).with_light_grid(1, 4, 128).is_err());
        assert!(draw(1.0, 0.0, None).with_light_grid(1, 64, 0).is_err());
    }

    #[test]
    fn light_grid_of_an_absurd_domain_is_refused_and_does_not_wrap_its_node_count() {
        // 8 units at voxel 2e-6 is 4e6 nodes an axis: the product, 6.4e19, wraps a u64 to a small number
        let error = plan_light_grid(&[draw(2e-6, 0.0, Some((1, 64, 4096)))], 2, true).unwrap_err();
        assert!(error.contains("raise lightGridCell"), "{error}");
        // 8e9 nodes an axis do not fit a u32 either
        let error = plan_light_grid(&[draw(1e-9, 0.0, Some((1, 64, 4096)))], 2, true).unwrap_err();
        assert!(error.contains("raise lightGridCell"), "{error}");
    }

    #[test]
    fn brick_directory_covers_the_bricks_and_gives_way_to_the_search_when_too_large() {
        let mut grid = SparseGrid::new(Transform::identity(), 0.0, 64).unwrap();
        grid.set([-9, 2, 40], 1.0).unwrap();
        grid.set([30, 2, 41], 1.0).unwrap();
        let (lo, dims) = directory_extent(&grid).unwrap();
        // bricks at keys (-2, 0, 5) and (3, 0, 5)
        assert_eq!((lo, dims), ([-2, 0, 5], [6, 1, 1]));
        let mut far = SparseGrid::new(Transform::identity(), 0.0, 64).unwrap();
        far.set([0, 0, 0], 1.0).unwrap();
        far.set([8 * 5000, 8 * 5000, 8 * 5000], 1.0).unwrap();
        assert!(directory_extent(&far).is_none(), "an extent past the table limit keeps the binary search");
        assert!(directory_extent(&SparseGrid::new(Transform::identity(), 0.0, 4).unwrap()).is_none());
    }
    use sr_volume::medium::{Bounds, Optical};
    use sr_volume::{SparseGrid, Transform};

    fn grid() -> SparseGrid {
        let mut grid = SparseGrid::new(Transform::identity(), 0.0, 64).unwrap();
        // a dense core, a faint skirt around it and a far brick holding only a negligible tail
        for z in 0..4 {
            for y in 0..4 {
                for x in 0..4 {
                    grid.set([x, y, z], 1.0).unwrap();
                }
            }
        }
        grid.set([9, 1, 1], 1e-3).unwrap();
        for z in 24..32 {
            for y in 24..32 {
                for x in 24..32 {
                    grid.set([x, y, z], 1e-15).unwrap();
                }
            }
        }
        grid
    }

    fn medium(grid: SparseGrid, extinction: f64, density_scale: f64) -> Medium {
        Medium::new(
            Arc::new(grid),
            Some(Bounds::new([0.0; 3], [32.0; 3]).unwrap()),
            Transform::identity(),
            Optical { extinction, density_scale, ..Default::default() },
        )
        .unwrap()
    }

    #[test]
    fn skip_map_clears_only_cells_whose_readable_voxels_are_negligible() {
        let grid = grid();
        let negligible = 1e-10;
        let (min, dims, words) = occupancy(&grid, negligible).unwrap();
        let bit = |c: [i32; 3]| {
            let rel = [c[0] - min[0], c[1] - min[1], c[2] - min[2]];
            if rel.iter().any(|r| *r < 0) || rel.iter().zip(dims).any(|(r, d)| *r as u32 >= d) {
                return false;
            }
            let index = rel[0] as usize + dims[0] as usize * (rel[1] as usize + dims[1] as usize * rel[2] as usize);
            words[index / 32] >> (index % 32) & 1 == 1
        };
        let (mut kept, mut cleared) = (0, 0);
        for z in min[2] - 2..min[2] + dims[2] as i32 + 2 {
            for y in min[1] - 2..min[1] + dims[1] as i32 + 2 {
                for x in min[0] - 2..min[0] + dims[0] as i32 + 2 {
                    // the voxels a sample inside cell c reads lie in [4c, 4c + 4] on each axis
                    let readable_max = (0..=4)
                        .flat_map(|k| (0..=4).flat_map(move |j| (0..=4).map(move |i| [i, j, k])))
                        .map(|[i, j, k]| grid.value([4 * x + i, 4 * y + j, 4 * z + k]).abs())
                        .fold(0.0f32, f32::max);
                    if readable_max > negligible {
                        assert!(bit([x, y, z]), "cell {x},{y},{z} can read {readable_max} but is skipped");
                        kept += 1;
                    } else {
                        // clearing is an optimisation, not a duty, but the map should find the empty space
                        if !bit([x, y, z]) {
                            cleared += 1;
                        }
                    }
                }
            }
        }
        assert!(kept > 0 && cleared > kept, "kept {kept}, cleared {cleared}");
        // the negligible tail's own cells are clear
        assert!(!bit([6, 6, 6]) && !bit([7, 7, 7]));
    }

    #[test]
    fn the_skipped_density_shrinks_with_extinction_density_scale_and_domain() {
        let diameter = 100.0;
        let base = negligible_density(&medium(grid(), 1.0, 1.0), diameter);
        assert!((f64::from(base) * diameter - SKIPPED_OPTICAL_DEPTH).abs() < 1e-3 * SKIPPED_OPTICAL_DEPTH);
        // a thin, very dense medium: a density of 1e-6 is not negligible
        assert!(negligible_density(&medium(grid(), 1e6, 1.0), diameter) < 1e-6);
        assert!(negligible_density(&medium(grid(), 1.0, 1e6), diameter) < 1e-6);
        assert!(negligible_density(&medium(grid(), 1.0, 1.0), 10.0 * diameter) < base);
        // no extinction: every cell is negligible
        assert_eq!(negligible_density(&medium(grid(), 0.0, 1.0), diameter), f32::INFINITY);
    }

    #[test]
    fn media_with_a_second_frame_or_a_nonzero_background_are_not_skipped() {
        assert!(skips_empty_space(&medium(grid(), 1.0, 1.0)));
        let second = medium(grid(), 1.0, 1.0).with_next_frame(Arc::new(grid()), None, 0.5).unwrap();
        assert!(!skips_empty_space(&second));
        let mut foggy = SparseGrid::new(Transform::identity(), 0.02, 16).unwrap();
        foggy.set([1, 1, 1], 1.0).unwrap();
        assert!(occupancy_extent(&foggy, 1e-10).is_none(), "a nonzero background fills the space between bricks");
    }
}
