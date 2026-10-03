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

#[derive(Clone, Debug)]
pub struct VolumeDraw {
    medium: Arc<Medium>,
    march: March,
    diameter: f64,
    cast_shadow: bool,
    receive_shadow: bool,
    thermal_color: DMat3,
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
        let draw =
            Self { medium, march, diameter, cast_shadow: true, receive_shadow: true, thermal_color: DMat3::IDENTITY };
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
            (RECORD_ROWS as u64 + density as u64 + thermal as u64 + velocity as u64) * 16
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
        out[base + 13] = [volume.cast_shadow as u8 as f32, volume.receive_shadow as u8 as f32, 0.0, 0.0];
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
