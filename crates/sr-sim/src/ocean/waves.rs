//! Authored travelling swell layered on the simulated disturbance at sample time.
//! It never feeds back into the shallow-water state. Depth taper, wet-cell mean
//! removal and a final nonnegative volume normalization bound shoreline motion.
use super::{Error, Frame, Spec};

#[derive(Clone, Copy, Debug)]
pub struct Wave {
    pub wavelength: f64,
    pub amplitude: f64,
    /// Degrees from +x toward +z.
    pub direction: f64,
    pub phase: f64,
    /// Authored phase speed in scene units/second; zero is stationary.
    pub speed: f64,
}

pub fn apply(spec: &Spec, frame: &Frame, waves: &[Wave]) -> Result<Frame, Error> {
    let n = spec.cells[0].checked_mul(spec.cells[1]).ok_or(Error::Limit("wave cell count"))?;
    if n == 0
        || n > 4_000_000
        || n != frame.cells.len()
        || waves.len() > 64
        || !frame.time.is_finite()
        || frame.time < 0.0
        || !spec.cell_size.is_finite()
        || spec.cell_size <= 0.0
        || !spec.dry_tolerance.is_finite()
        || spec.dry_tolerance <= 0.0
        || spec.origin.iter().any(|v| !v.is_finite())
    {
        return Err(Error::Invalid("wave grid or clock"));
    }
    if n.saturating_mul(128) > spec.max_bytes || (n as u64).saturating_mul(waves.len() as u64 + 5) > spec.max_work {
        return Err(Error::Limit("wave surface memory or work"));
    }
    for w in waves {
        if [w.wavelength, w.amplitude, w.direction, w.phase, w.speed].iter().any(|v| !v.is_finite())
            || w.wavelength < 2.0 * spec.cell_size
            || w.amplitude < 0.0
            || w.speed < 0.0
        {
            return Err(Error::Invalid("wave length, amplitude, angle or speed"));
        }
    }
    if frame.cells.iter().any(|c| !c.depth.is_finite() || c.depth < 0.0 || c.velocity.iter().any(|v| !v.is_finite())) {
        return Err(Error::Invalid("wave input cells"));
    }
    if waves.is_empty() {
        return Ok(frame.clone());
    }
    let mut offset = vec![[0.0; 3]; n];
    let (mut total, mut mean, mut wet) = (0.0, 0.0, 0usize);
    for (i, (cell, offset)) in frame.cells.iter().zip(&mut offset).enumerate() {
        if cell.depth < spec.dry_tolerance {
            continue;
        }
        total += cell.depth;
        wet += 1;
        let x = spec.origin[0] + (i % spec.cells[0]) as f64 * spec.cell_size + 0.5 * spec.cell_size;
        let z = spec.origin[1] + (i / spec.cells[0]) as f64 * spec.cell_size + 0.5 * spec.cell_size;
        for w in waves {
            let (sz, cx) = w.direction.to_radians().sin_cos();
            let phase = std::f64::consts::TAU * ((cx * x + sz * z - w.speed * frame.time) / w.wavelength)
                + w.phase.to_radians();
            let h = w.amplitude.min(0.5 * cell.depth) * phase.cos();
            let flow = w.speed * h / cell.depth;
            offset[0] += h;
            offset[1] += flow * cx;
            offset[2] += flow * sz;
        }
        if offset.iter().any(|v| !v.is_finite()) {
            return Err(Error::Numerical("wave phase or velocity overflow"));
        }
        mean += offset[0];
    }
    if wet == 0 {
        return Ok(frame.clone());
    }
    mean /= wet as f64;
    let mut out = frame.clone();
    let mut displaced = 0.0;
    for ((dst, src), offset) in out.cells.iter_mut().zip(&frame.cells).zip(&offset) {
        if src.depth < spec.dry_tolerance {
            continue;
        }
        dst.depth = (src.depth + offset[0] - mean).max(0.0);
        displaced += dst.depth;
        for a in 0..2 {
            dst.velocity[a] += offset[a + 1];
        }
    }
    if !total.is_finite() || !displaced.is_finite() || !mean.is_finite() || displaced <= 0.0 {
        return Err(Error::Numerical("wave volume overflow"));
    }
    let scale = total / displaced;
    for (dst, src) in out.cells.iter_mut().zip(&frame.cells) {
        if src.depth >= spec.dry_tolerance {
            dst.depth *= scale;
        }
        if dst.depth < spec.dry_tolerance {
            dst.velocity = [0.0; 2];
        }
        if !dst.depth.is_finite() || dst.velocity.iter().any(|v| !v.is_finite()) {
            return Err(Error::Numerical("wave output overflow"));
        }
    }
    Ok(out)
}
