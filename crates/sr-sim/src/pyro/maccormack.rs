//! MacCormack advection with an extrema limiter (`Advection::MacCormack`).
//!
//! For every channel (density, temperature, each velocity component) and every
//! element at position `p`, with `SL` the midpoint semi-Lagrangian trace used by
//! the default scheme:
//!
//! 1. `hat = SL(old)` at `p` (the plain semi-Lagrangian value);
//! 2. `til` = `hat` sampled where the forward trace of `p` lands;
//! 3. `value = hat + 0.5 * (old[p] - til)`, clamped to the minimum and maximum of
//!    the eight `old` values around the backward-trace origin, so the scheme can
//!    create no new extremum (density stays non-negative, temperature stays in
//!    its old range);
//! 4. where a collider cuts either trace short, `value = hat`.
//!
//! Dissipation and cooling are applied to the limited value, as the default scheme
//! applies them to its sample. Every element is computed from immutable inputs
//! by the same expression, so results do not depend on the thread count.
//!
//! Density and temperature share their traces; each velocity component has its
//! own. Measured against the default scheme (impact scene, 1 thread) the
//! advection stage costs 2.5x at 64^3 and 2.4x at 128^3.

use super::*;

/// Pre-advection scalar fields and the per-step decay factors.
pub(super) struct Scalars<'a> {
    pub density: &'a [f64],
    pub temperature: &'a [f64],
    pub ambient: f64,
    pub decay: f64,
    pub cool: f64,
}

/// One advected array: where its elements sit and what lies outside the domain.
struct Channel<'a> {
    old: &'a [f64],
    dims: [usize; 3],
    /// Grid position of element `c` is `c + offset`; samples are taken at
    /// `trace point - offset`.
    offset: [f64; 3],
    background: f64,
}

impl Channel<'_> {
    fn position(&self, k: usize) -> [f64; 3] {
        let c = coords(k, self.dims);
        std::array::from_fn(|i| c[i] as f64 + self.offset[i])
    }

    fn at(&self, q: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|i| q[i] - self.offset[i])
    }
}

/// Where the backward trace of an element starts and where its forward trace
/// lands, or that a collider cut one of them short.
enum Traces {
    Cut,
    Free { source: [f64; 3], landing: [f64; 3] },
}

fn traces(field: &Field<'_>, p: [f64; 3], dt: f64) -> Result<Traces, Error> {
    let v = field.velocity_grid(p);
    let (source, cut) = field.trace_from(p, v, dt)?;
    if cut {
        return Ok(Traces::Cut);
    }
    let (landing, cut) = field.trace_from(p, v, -dt)?;
    Ok(if cut { Traces::Cut } else { Traces::Free { source, landing } })
}

/// The corrected, limited value of element `k`, from its semi-Lagrangian `hat`.
fn corrected(ch: &Channel<'_>, boundary: Boundary, hat: &[f64], k: usize, traces: &Traces) -> f64 {
    match traces {
        Traces::Cut => hat[k],
        Traces::Free { source, landing } => {
            let til = sample(hat, ch.dims, ch.at(*landing), boundary, ch.background);
            let (lo, hi) = sample_range(ch.old, ch.dims, ch.at(*source), boundary, ch.background);
            (hat[k] + 0.5 * (ch.old[k] - til)).clamp(lo, hi)
        }
    }
}

pub(super) fn advect(
    field: &Field<'_>,
    scalars: Scalars<'_>,
    (density, temperature, velocity): (&mut [f64], &mut [f64], &mut [Vec<f64>; 3]),
    dt: f64,
) -> Result<(), Error> {
    advect_scalars(field, &scalars, dt, density, temperature)?;
    for (a, out) in velocity.iter_mut().enumerate() {
        let channel = Channel {
            old: field.velocity[a],
            dims: face_dims(field.cells, a),
            offset: std::array::from_fn(|i| if i == a { 0.0 } else { 0.5 }),
            background: 0.0,
        };
        advect_velocity(field, &channel, dt, out)?;
    }
    Ok(())
}

/// Density and temperature live in the same cells, so their traces are shared.
/// Solid cells keep the cleared values (zero density, ambient temperature) that
/// `density` and `temperature` hold on entry.
fn advect_scalars(
    field: &Field<'_>,
    scalars: &Scalars<'_>,
    dt: f64,
    density: &mut [f64],
    temperature: &mut [f64],
) -> Result<(), Error> {
    let (boundary, solid) = (field.boundary, field.solid);
    let Scalars { ambient, decay, cool, .. } = *scalars;
    let channels = [
        Channel { old: scalars.density, dims: field.cells, offset: [0.5; 3], background: 0.0 },
        Channel { old: scalars.temperature, dims: field.cells, offset: [0.5; 3], background: ambient },
    ];
    let count = density.len();
    let (mut hat_density, mut hat_temperature) = (vec![0.0; count], vec![ambient; count]);
    let results = hat_density
        .par_chunks_mut(HEAVY)
        .zip(hat_temperature.par_chunks_mut(HEAVY))
        .enumerate()
        .map(|(chunk, (hd, ht))| -> Result<(), Error> {
            for (i, (d, t)) in hd.iter_mut().zip(ht.iter_mut()).enumerate() {
                let k = chunk * HEAVY + i;
                if !solid[k] {
                    let q = field.trace(channels[0].position(k), dt)?;
                    *d = sample(channels[0].old, channels[0].dims, channels[0].at(q), boundary, 0.0);
                    *t = sample(channels[1].old, channels[1].dims, channels[1].at(q), boundary, ambient);
                }
            }
            Ok(())
        })
        .collect();
    first_error(results)?;

    let (hat_density, hat_temperature) = (&hat_density, &hat_temperature);
    let results = density
        .par_chunks_mut(HEAVY)
        .zip(temperature.par_chunks_mut(HEAVY))
        .enumerate()
        .map(|(chunk, (od, ot))| -> Result<(), Error> {
            for (i, (d, t)) in od.iter_mut().zip(ot.iter_mut()).enumerate() {
                let k = chunk * HEAVY + i;
                if !solid[k] {
                    let shared = traces(field, channels[0].position(k), dt)?;
                    *d = corrected(&channels[0], boundary, hat_density, k, &shared) * decay;
                    let value = corrected(&channels[1], boundary, hat_temperature, k, &shared);
                    *t = ambient + (value - ambient) * cool;
                }
            }
            Ok(())
        })
        .collect();
    first_error(results)
}

fn advect_velocity(field: &Field<'_>, ch: &Channel<'_>, dt: f64, out: &mut [f64]) -> Result<(), Error> {
    let boundary = field.boundary;
    let mut hat = vec![0.0; out.len()];
    let results = hat
        .par_chunks_mut(HEAVY)
        .enumerate()
        .map(|(chunk, values)| -> Result<(), Error> {
            for (i, value) in values.iter_mut().enumerate() {
                let q = field.trace(ch.position(chunk * HEAVY + i), dt)?;
                *value = sample(ch.old, ch.dims, ch.at(q), boundary, ch.background);
            }
            Ok(())
        })
        .collect();
    first_error(results)?;

    let hat = &hat;
    let results = out
        .par_chunks_mut(HEAVY)
        .enumerate()
        .map(|(chunk, values)| -> Result<(), Error> {
            for (i, value) in values.iter_mut().enumerate() {
                let k = chunk * HEAVY + i;
                *value = corrected(ch, boundary, hat, k, &traces(field, ch.position(k), dt)?);
            }
            Ok(())
        })
        .collect();
    first_error(results)
}

/// Minimum and maximum of the values `sample` would blend at `p`: the in-range
/// corners, plus `background` for an out-of-range corner with nonzero weight.
fn sample_range(values: &[f64], dims: [usize; 3], mut p: [f64; 3], boundary: Boundary, background: f64) -> (f64, f64) {
    if !finite3(p) {
        return (background, background);
    }
    for a in 0..3 {
        if boundary == Boundary::Closed {
            p[a] = p[a].clamp(0.0, (dims[a] - 1) as f64);
        } else if p[a] < -1.0 || p[a] > dims[a] as f64 {
            return (background, background);
        }
    }
    let lo = p.map(floor_i64);
    if (0..3).all(|a| lo[a] >= 0 && lo[a] + 1 < dims[a] as i64) {
        let (sy, sz) = (dims[0], dims[0] * dims[1]);
        let base = (lo[2] as usize * dims[1] + lo[1] as usize) * dims[0] + lo[0] as usize;
        let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
        for offset in [0, 1, sy, sy + 1, sz, sz + 1, sz + sy, sz + sy + 1] {
            let value = values[base + offset];
            min = min.min(value);
            max = max.max(value);
        }
        return (min, max);
    }
    sample_range_general(values, dims, p, background)
}

/// Corner by corner, for stencils that touch the edge of the array.
fn sample_range_general(values: &[f64], dims: [usize; 3], p: [f64; 3], background: f64) -> (f64, f64) {
    let lo = p.map(floor_i64);
    let f: [f64; 3] = std::array::from_fn(|a| p[a] - lo[a] as f64);
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for bit in 0..8usize {
        let q: [i64; 3] = std::array::from_fn(|a| lo[a] + ((bit >> a) & 1) as i64);
        let weight: f64 = (0..3).map(|a| if (bit >> a) & 1 == 0 { 1.0 - f[a] } else { f[a] }).product();
        let value = if (0..3).all(|a| q[a] >= 0 && q[a] < dims[a] as i64) {
            values[index(q.map(|v| v as usize), dims)]
        } else if weight > 0.0 {
            background
        } else {
            continue;
        };
        min = min.min(value);
        max = max.max(value);
    }
    (min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_in_range_fast_path_matches_the_corner_by_corner_range() {
        for dims in [[3, 3, 3], [5, 4, 3], [9, 2, 6]] {
            let n: usize = dims.iter().product();
            let values: Vec<f64> = (0..n).map(|k| crate::rng::signed(2, k as u64, 0) * 50.0).collect();
            for boundary in [Boundary::Open, Boundary::Closed] {
                for i in 0..4000u64 {
                    let p: [f64; 3] =
                        std::array::from_fn(|a| crate::rng::unit(9, i, a as u64) * (dims[a] as f64 + 2.0) - 1.0);
                    let (got, want) = (sample_range(&values, dims, p, boundary, 7.0), {
                        // The same clamping as `sample_range`, then the general path.
                        let mut q = p;
                        let mut outside = false;
                        for a in 0..3 {
                            if boundary == Boundary::Closed {
                                q[a] = q[a].clamp(0.0, (dims[a] - 1) as f64);
                            } else if q[a] < -1.0 || q[a] > dims[a] as f64 {
                                outside = true;
                            }
                        }
                        if outside {
                            (7.0, 7.0)
                        } else {
                            sample_range_general(&values, dims, q, 7.0)
                        }
                    });
                    assert_eq!(
                        (got.0.to_bits(), got.1.to_bits()),
                        (want.0.to_bits(), want.1.to_bits()),
                        "{dims:?} {p:?}"
                    );
                }
            }
        }
    }
}
