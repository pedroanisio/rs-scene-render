//! What an impact throws out of its crater: where each particle is launched from, when,
//! how fast, at what angle and how heavy it is. Pure and in SI units, so the rigid world,
//! the water and the particles can all ask it.
//!
//! The speed of ejecta launched from distance `x` of the impact point follows Housen and
//! Holsapple (2011), "Ejecta from impact craters", Icarus 211:856-875
//! (doi 10.1016/j.icarus.2010.09.017):
//!
//! ```text
//! v / U = C1 [ (x / a) (rho / delta)^nu ]^(-1 / mu) (1 - x / (n2 R))^p,   n1 a <= x <= n2 R
//! ```
//!
//! with `a` the radius of the body, `U` its speed, `rho` and `delta` the densities of the
//! target and of the body, `R` the radius of the crater and `nu = 0.4`, `n1 = 1.2`. The
//! constants `mu, C1, k, n2, p` of each material are those of the paper's table. The mass
//! launched from within `x` is proportional to `x^3 - (n1 a)^3`; only that shape is used,
//! scaled so that the launched mass is 80% of the crater's. The paper itself could not be
//! consulted when this was written: the equations and the constants are those of the
//! specification this module was written from, so they are to be checked against it.
//!
//! Where the paper is silent the engine decides, and says so: the launch angle (45 degrees
//! above the tangent plane, spread uniformly by +-15), the time of launch
//! (`T (x / R)^3`, at most the formation time `T`, so the fastest material leaves first)
//! and the lopsidedness of an oblique impact. For the last, the density of mass over the
//! azimuth `az` from the downrange direction is `1 + b cos(az)` with
//! `b = clamp((45 - theta) / 20, 0, 1)` and `theta` the angle of the impact velocity with
//! the surface, in degrees. It rests on the published thresholds of 45 and 25 degrees
//! (Herrick and Forsberg-Taylor 2003, doi 10.1111/j.1945-5100.2003.tb00001.x) for the
//! onset of asymmetry and of a forbidden zone uprange, without a published formula.
//! `nu = 0.4` here against `0.33` in the cratering law, and `n1`, are parametrisations
//! without a checked mapping to each other.

use super::Material;
use rayon::prelude::*;

const NU: f64 = 0.4;
const N1: f64 = 1.2;
/// Share of the crater's mass that is launched.
const LAUNCHED_SHARE: f64 = 0.8;
const MAX_PARTICLES: usize = 1_000_000;
/// Above this many particles the work is spread over the rayon pool; every particle is a
/// pure function of its index, so the result does not depend on it.
const PARALLEL: usize = 16_384;

/// `(mu, C1, k, n2, p)` of a material: water; dry sand (also dry soil); hard rock; weakly
/// cemented basalt (also wet soil and soft rock). Regolith and ice have no row.
fn row(material: Material) -> Result<(f64, f64, f64, f64, f64), String> {
    Ok(match material {
        Material::Water => (0.55, 1.5, 0.2, 1.5, 0.5),
        Material::DrySand | Material::DrySoil => (0.41, 0.55, 0.3, 1.3, 0.3),
        Material::HardRock => (0.55, 1.5, 0.3, 1.0, 0.5),
        Material::WetSoil | Material::SoftRock => (0.46, 0.18, 0.3, 1.0, 0.3),
        Material::Regolith => return Err("no ejecta table row for regolith".into()),
        Material::ColdIce => return Err("no ejecta table row for ice".into()),
    })
}

/// The impact and the crater it made.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub material: Material,
    /// Metres: the radius of the sphere with the body's volume.
    pub body_radius: f64,
    /// Kilograms per cubic metre.
    pub body_density: f64,
    pub target_density: f64,
    /// Metres per second.
    pub impact_speed: f64,
    /// Direction of the impact velocity, which points into the surface; any length.
    pub velocity_direction: [f64; 3],
    /// Contact normal, pointing out of the target; any length.
    pub normal: [f64; 3],
    /// Cubic metres excavated, metres of radius at the surface and seconds to form.
    pub crater_volume: f64,
    pub crater_radius: f64,
    pub crater_duration: f64,
    pub particles: usize,
    pub seed: u64,
    /// Degrees above the tangent plane, and the half-width of the uniform spread.
    pub angle: f64,
    pub angle_spread: f64,
}

/// One launched particle, relative to the impact point and to the instant of impact.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ejecta {
    /// Seconds after the impact.
    pub time: f64,
    /// Metres from the impact point, in the tangent plane of the contact.
    pub position: [f64; 3],
    /// Metres per second.
    pub velocity: [f64; 3],
    /// Kilograms.
    pub mass: f64,
}

fn finite_positive(name: &str, v: f64) -> Result<f64, String> {
    if v.is_finite() && v > 0.0 {
        Ok(v)
    } else {
        Err(format!("{name} must be a positive finite number, not {v}"))
    }
}
fn unit_vector(name: &str, v: [f64; 3]) -> Result<[f64; 3], String> {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if !length.is_finite() || length == 0.0 {
        return Err(format!("{name} must be a finite nonzero vector"));
    }
    Ok(v.map(|c| c / length))
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// The azimuth, in `[-pi, pi]` from the downrange direction, whose cumulative share of
/// the density `1 + b cos(az)` is `u`; found by bisection, which is deterministic.
fn azimuth(u: f64, b: f64) -> f64 {
    let pi = std::f64::consts::PI;
    let (mut low, mut high) = (-pi, pi);
    for _ in 0..64 {
        let mid = 0.5 * (low + high);
        if (mid + pi + b * mid.sin()) / (2.0 * pi) < u {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
}

/// The particles of an impact, in order of increasing launch distance.
pub fn ejecta(spec: &Spec) -> Result<Vec<Ejecta>, String> {
    let (mu, c1, _k, n2, p) = row(spec.material)?;
    let a = finite_positive("body radius", spec.body_radius)?;
    let delta = finite_positive("body density", spec.body_density)?;
    let rho = finite_positive("target density", spec.target_density)?;
    let u = finite_positive("impact speed", spec.impact_speed)?;
    let volume = finite_positive("crater volume", spec.crater_volume)?;
    let radius = finite_positive("crater radius", spec.crater_radius)?;
    let duration = finite_positive("crater duration", spec.crater_duration)?;
    if !(1..=MAX_PARTICLES).contains(&spec.particles) {
        return Err(format!("the number of particles must be 1 to {MAX_PARTICLES}, not {}", spec.particles));
    }
    if !(spec.angle.is_finite()
        && spec.angle_spread.is_finite()
        && spec.angle_spread >= 0.0
        && spec.angle - spec.angle_spread >= 0.0
        && spec.angle + spec.angle_spread <= 90.0)
    {
        return Err("the launch angle and its spread must stay between 0 and 90 degrees".into());
    }
    let (x_min, x_max) = (N1 * a, n2 * radius);
    if x_max.partial_cmp(&x_min) != Some(std::cmp::Ordering::Greater) {
        return Err(format!("the crater (launch range up to {x_max} m) is no larger than the body's ({x_min} m)"));
    }
    let normal = unit_vector("contact normal", spec.normal)?;
    let approach = unit_vector("impact velocity direction", spec.velocity_direction)?;
    // Frame of the tangent plane: e1 downrange, e2 completing a right-handed set with n.
    let along = dot(approach, normal);
    let mut downrange =
        [approach[0] - along * normal[0], approach[1] - along * normal[1], approach[2] - along * normal[2]];
    if dot(downrange, downrange) < 1e-24 {
        // A normal impact has no downrange: any tangent direction, chosen without a seed.
        let axis = (0..3).min_by(|&i, &j| normal[i].abs().total_cmp(&normal[j].abs())).unwrap_or(0);
        let mut e = [0.0; 3];
        e[axis] = 1.0;
        let along = dot(e, normal);
        downrange = [e[0] - along * normal[0], e[1] - along * normal[1], e[2] - along * normal[2]];
    }
    let e1 = unit_vector("downrange direction", downrange)?;
    let e2 = cross(normal, e1);
    let theta = along.abs().clamp(0.0, 1.0).asin().to_degrees();
    let b = ((45.0 - theta) / 20.0).clamp(0.0, 1.0);
    let ratio = rho / delta;
    let total = LAUNCHED_SHARE * rho * volume;
    let (lo3, hi3) = (x_min.powi(3), x_max.powi(3));
    let n = spec.particles;
    let one = |i: usize| -> Ejecta {
        let q = (i as f64 + 0.5) / n as f64;
        let x = (lo3 + q * (hi3 - lo3)).cbrt();
        let speed = (u * c1 * ((x / a) * ratio.powf(NU)).powf(-1.0 / mu) * (1.0 - x / x_max).powf(p)).min(u);
        let time = (duration * (x / radius).powi(3)).min(duration);
        let elevation =
            (spec.angle + spec.angle_spread * (2.0 * crate::rng::unit(spec.seed, i as u64, 1) - 1.0)).to_radians();
        let az = azimuth(crate::rng::unit(spec.seed, i as u64, 2), b);
        let direction: [f64; 3] = std::array::from_fn(|c| az.cos() * e1[c] + az.sin() * e2[c]);
        Ejecta {
            time,
            position: direction.map(|c| x * c),
            velocity: std::array::from_fn(|c| speed * (elevation.cos() * direction[c] + elevation.sin() * normal[c])),
            mass: total / n as f64,
        }
    };
    Ok(if n >= PARALLEL { (0..n).into_par_iter().map(one).collect() } else { (0..n).map(one).collect() })
}
