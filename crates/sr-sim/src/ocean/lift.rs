//! How deep water attenuates what a displacement of its fluid or its bed does to the surface.
//!
//! In linear potential flow an instantaneous displacement `zeta` of the bed of water of depth `h`
//! raises the free surface by `zeta(k) / cosh(k h)` in Fourier space (the filter of Kajiura, 1963,
//! "The leading wave of a tsunami", Bull. Earthq. Res. Inst. 41:535-571, as it is used to start
//! tsunami simulations; known here from the literature that quotes it, not from the paper). The
//! engine also needs the case where the displaced volume is not on the bed but at height `z0`
//! above it, and derives it: for a unit volume source at height `z0` in water of depth `h` with a
//! rigid bed and, for an impulse, zero pressure at the surface, the potential per wavenumber is
//! `A cosh(k z)` below the source and `B sinh(k (h - z))` above it; continuity at the source and the
//! jump `-q` in the vertical derivative give `B = q cosh(k z0) / (k cosh(k h))`, so the vertical
//! velocity of the surface, and with it the response, is `cosh(k z0) / cosh(k h)`. At `z0 = 0` that
//! is Kajiura's filter, and at `z0 = h` (a source at the surface) it is 1.
//!
//! It is the response to an impulse: what comes after, in a shallow-water solver, is not dispersive.
//! The filter has no mass of its own: `k = 0` gives 1, and the result is renormalised so that what
//! is displaced is what is lifted, after the window is cut off, the domain ends or a column is dry.
use super::Error;

/// The widest window, in cells, one transform is made over.
pub const MAX_WINDOW: usize = 1024;
/// Cells of margin beyond a footprint, in depths: the filter's kernel falls as
/// `exp(-pi r / (2 h))`, 0.2% at four depths.
const REACH: f64 = 4.0;

type Complex = [f64; 2];

fn fft(data: &mut [Complex], inverse: bool) {
    let n = data.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            data.swap(i, j);
        }
    }
    let mut length = 2;
    while length <= n {
        let angle = std::f64::consts::TAU / length as f64 * if inverse { 1.0 } else { -1.0 };
        let step = [angle.cos(), angle.sin()];
        for start in (0..n).step_by(length) {
            let mut w = [1.0, 0.0];
            for k in 0..length / 2 {
                let (a, b) = (data[start + k], data[start + k + length / 2]);
                let t = [b[0] * w[0] - b[1] * w[1], b[0] * w[1] + b[1] * w[0]];
                data[start + k] = [a[0] + t[0], a[1] + t[1]];
                data[start + k + length / 2] = [a[0] - t[0], a[1] - t[1]];
                w = [w[0] * step[0] - w[1] * step[1], w[0] * step[1] + w[1] * step[0]];
            }
        }
        length <<= 1;
    }
}

fn fft2(data: &mut [Complex], [nx, nz]: [usize; 2], inverse: bool) {
    for row in data.chunks_mut(nx) {
        fft(row, inverse);
    }
    let mut column = vec![[0.0; 2]; nz];
    for x in 0..nx {
        for z in 0..nz {
            column[z] = data[z * nx + x];
        }
        fft(&mut column, inverse);
        for z in 0..nz {
            data[z * nx + x] = column[z];
        }
    }
}

/// `cosh(k z0) / cosh(k h)` without overflow, for `0 <= z0 <= h`.
fn response(k: f64, depth: f64, height: f64) -> f64 {
    let a = (-k * (depth - height)).exp();
    let b = (-k * (depth + height)).exp();
    let c = (-2.0 * k * depth).exp();
    (a + b) / (1.0 + c)
}

/// Filters the non-negative `part` over the grid `cells` of square cells of side `cell` and returns
/// the columns that receive water, with the amounts, in order of column. `wet` says which cells may
/// receive water. The flag is false, and the amounts are the part itself, when its extent is too
/// wide for one transform.
fn filter_sparse(
    cells: [usize; 2],
    cell: f64,
    depth: f64,
    height: f64,
    part: &[f64],
    wet: &dyn Fn(usize) -> bool,
) -> (Vec<(usize, f64)>, bool) {
    let [nx, nz] = cells;
    let mut low = [usize::MAX; 2];
    let mut high = [0usize; 2];
    let mut total = 0.0;
    for (c, &v) in part.iter().enumerate() {
        if v != 0.0 {
            let (x, z) = (c % nx, c / nx);
            low = [low[0].min(x), low[1].min(z)];
            high = [high[0].max(x), high[1].max(z)];
            total += v;
        }
    }
    if total == 0.0 || low[0] == usize::MAX {
        return (Vec::new(), true);
    }
    let extent = [high[0] - low[0] + 1, high[1] - low[1] + 1];
    if extent[0] > MAX_WINDOW || extent[1] > MAX_WINDOW {
        return (part.iter().enumerate().filter(|(_, v)| **v != 0.0).map(|(c, v)| (c, *v)).collect(), false);
    }
    let reach = (REACH * depth / cell).ceil().max(1.0) as usize;
    // room for the margin when it fits in a window, else as much as does
    let size: [usize; 2] = std::array::from_fn(|a| {
        let wanted = (extent[a] + 2 * reach).next_power_of_two();
        wanted.min(MAX_WINDOW).max(extent[a].next_power_of_two())
    });
    // the window sits in the middle of the transform, which wraps around it
    let margin: [usize; 2] = std::array::from_fn(|a| (size[a] - extent[a]) / 2);
    let mut data = vec![[0.0; 2]; size[0] * size[1]];
    for z in low[1]..=high[1] {
        for x in low[0]..=high[0] {
            data[(z - low[1] + margin[1]) * size[0] + (x - low[0] + margin[0])][0] = part[z * nx + x];
        }
    }
    fft2(&mut data, size, false);
    for iz in 0..size[1] {
        let kz = std::f64::consts::TAU * (if iz <= size[1] / 2 { iz as f64 } else { iz as f64 - size[1] as f64 })
            / (size[1] as f64 * cell);
        for ix in 0..size[0] {
            let kx = std::f64::consts::TAU * (if ix <= size[0] / 2 { ix as f64 } else { ix as f64 - size[0] as f64 })
                / (size[0] as f64 * cell);
            let r = response(kx.hypot(kz), depth, height);
            let v = &mut data[iz * size[0] + ix];
            *v = [v[0] * r, v[1] * r];
        }
    }
    fft2(&mut data, size, true);
    let scale = 1.0 / (size[0] * size[1]) as f64;
    // what falls inside the domain, on wet cells, is renormalised to the whole
    let mut kept = 0.0;
    let mut placed: Vec<(usize, f64)> = Vec::new();
    for iz in 0..size[1] {
        for ix in 0..size[0] {
            let (x, z) = ((low[0] + ix) as isize - margin[0] as isize, (low[1] + iz) as isize - margin[1] as isize);
            if x < 0 || z < 0 || x >= nx as isize || z >= nz as isize {
                continue;
            }
            let c = z as usize * nx + x as usize;
            let v = data[iz * size[0] + ix][0] * scale;
            if wet(c) && v > 0.0 {
                kept += v;
                placed.push((c, v));
            }
        }
    }
    if kept <= 0.0 {
        return (part.iter().enumerate().filter(|(_, v)| **v != 0.0).map(|(c, v)| (c, *v)).collect(), true);
    }
    let renormalise = total / kept;
    for (_, v) in placed.iter_mut() {
        *v *= renormalise;
    }
    (placed, true)
}

/// As [`depth_response`] for a non-negative `field`, giving the columns and amounts instead of
/// adding them to a vector the size of the grid, in order of column.
pub fn depth_response_sparse(
    cells: [usize; 2],
    cell: f64,
    depth: f64,
    height: f64,
    field: &[f64],
    wet: &dyn Fn(usize) -> bool,
) -> Result<(Vec<(usize, f64)>, bool), Error> {
    if !(depth.is_finite() && depth > 0.0 && cell.is_finite() && cell > 0.0 && height.is_finite())
        || field.len() != cells[0] * cells[1]
        || field.iter().any(|v| !v.is_finite() || *v < 0.0)
    {
        return Err(Error::Invalid("depth response depth, cell size, grid or displacement"));
    }
    Ok(filter_sparse(cells, cell, depth, height.clamp(0.0, depth), field, wet))
}

fn filter_part(
    cells: [usize; 2],
    cell: f64,
    depth: f64,
    height: f64,
    part: &[f64],
    out: &mut [f64],
    wet: &dyn Fn(usize) -> bool,
) -> bool {
    let (placed, whole) = filter_sparse(cells, cell, depth, height, part, wet);
    for (c, v) in placed {
        out[c] += v;
    }
    whole
}

/// Adds to `out` the surface response, at depth `depth` and for a displacement at height `height`
/// above the bed (`0` for the bed itself), to `field`, a displacement of fluid per column on the
/// grid of `cells` square cells of side `cell`; `wet` says which columns may receive any. The
/// positive and the negative parts are filtered apart and each keeps its total, so the net
/// volume is what it was. `height` is limited to `[0, depth]`. A part too wide for a window
/// (`MAX_WINDOW` cells) is left unfiltered; the result is `false` then.
pub fn depth_response(
    cells: [usize; 2],
    cell: f64,
    depth: f64,
    height: f64,
    field: &[f64],
    out: &mut [f64],
    wet: &dyn Fn(usize) -> bool,
) -> Result<bool, Error> {
    if !(depth.is_finite() && depth > 0.0 && cell.is_finite() && cell > 0.0 && height.is_finite())
        || field.len() != cells[0] * cells[1]
        || out.len() != field.len()
    {
        return Err(Error::Invalid("depth response depth, cell size or grid"));
    }
    let height = height.clamp(0.0, depth);
    let positive: Vec<f64> = field.iter().map(|v| v.max(0.0)).collect();
    let mut all = filter_part(cells, cell, depth, height, &positive, out, wet);
    if field.iter().any(|v| *v < 0.0) {
        let negative: Vec<f64> = field.iter().map(|v| (-v).max(0.0)).collect();
        let mut minus = vec![0.0; out.len()];
        all &= filter_part(cells, cell, depth, height, &negative, &mut minus, wet);
        for (o, m) in out.iter_mut().zip(minus) {
            *o -= m;
        }
    }
    Ok(all)
}
