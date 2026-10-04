//! The crater an impact makes, from the pi-group scaling law of Holsapple (1993),
//! "The scaling of impact processes in planetary sciences", Annu. Rev. Earth Planet.
//! Sci. 21:333-373 (doi 10.1146/annurev.ea.21.050193.002001, Eq. 18), with the crater
//! shape factors and formation time of the calculator note that accompanies it
//! (Holsapple, "Theory and equations for Craters from Impacts and Explosions", at
//! lpi.usra.edu/lunar/tools/lunarcratercalc/theory.pdf).
//!
//! Everything here is SI and pure: no world, no state, so the rigid bodies, the smoke and
//! the water can all ask the same question. The law has two regimes joined by one smooth
//! interpolation, gravity (a lower crater the more gravity holds the ejecta) and strength
//! (the target resists), with
//!
//! ```text
//! pi_V = K1 { pi2 (rho/delta)^((6nu - 2 - mu)/(3 mu))
//!             + [K2 pi3 (rho/delta)^((6nu - 2)/(3 mu))]^((2 + mu)/2) }^(-3 mu/(2 + mu))
//! pi_V = rho V / m,  pi2 = g a / U^2,  pi3 = Y / (rho U^2)
//! ```
//!
//! where `m` and `a` are the mass and radius of the impactor, `delta` its density, `U`
//! the speed of its approach along the surface normal, `rho` and `Y` the density and
//! strength of the target, and `g` gravity. `pi2` has no factor of 3.22; the exponent of
//! the strength term is `(2 + mu)/2` (the 1993 table prints it as `(2 + mu)/mu`, which
//! Holsapple later corrected). The constants are those of the calculator note, which holds
//! both regimes for every material in one table; Holsapple says the dependence on strength
//! and porosity is "only poorly constrained", so strength and density can be overridden.
//! The 2022 table of the same author (arXiv:2203.07476) and the ejecta constants of Housen
//! and Holsapple (2011) are other parametrisations and are not mixed in here.

pub mod ejecta;

/// What the impactor brings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Impact {
    /// Kilograms.
    pub mass: f64,
    /// Kilograms per cubic metre.
    pub density: f64,
    /// Metres per second of the approach along the surface normal: the component of the
    /// relative velocity along the surface's normal at the impact point.
    pub normal_speed: f64,
}

/// What it hits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub material: Material,
    /// Kilograms per cubic metre; the table's value when `None`.
    pub density: Option<f64>,
    /// Pascals of cratering strength; the table's value when `None`.
    pub strength: Option<f64>,
    /// Metres per second squared; positive.
    pub gravity: f64,
}

/// A target material, with the constants of the calculator note (`nu = 0.33` for all).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    Water,
    DrySand,
    DrySoil,
    WetSoil,
    SoftRock,
    HardRock,
    Regolith,
    /// As printed in the note: its strength is far below that of the other solids, and the
    /// author's later table gives another, so treat it as the least certain entry.
    ColdIce,
}

impl Material {
    /// The names a document uses.
    pub fn parse(name: &str) -> Option<Material> {
        Some(match name {
            "water" => Material::Water,
            "drySand" => Material::DrySand,
            "drySoil" => Material::DrySoil,
            "wetSoil" => Material::WetSoil,
            "softRock" => Material::SoftRock,
            "hardRock" => Material::HardRock,
            "regolith" => Material::Regolith,
            "ice" => Material::ColdIce,
            _ => return None,
        })
    }

    /// `(K1, K2, mu, Y in Pa, rho in kg/m3)`; the note gives Y in dyne/cm2 and rho in g/cm3.
    fn constants(self) -> (f64, f64, f64, f64, f64) {
        match self {
            Material::Water => (0.98, 0.0, 0.55, 0.0, 1000.0),
            Material::DrySand => (0.132, 0.0, 0.41, 0.0, 1700.0),
            Material::DrySoil => (0.132, 0.26, 0.41, 2.0e5, 1700.0),
            Material::WetSoil => (0.095, 0.35, 0.55, 5.0e5, 2100.0),
            Material::SoftRock => (0.095, 0.215, 0.55, 1.0e6, 2100.0),
            Material::HardRock => (0.095, 0.257, 0.55, 1.0e7, 3200.0),
            Material::Regolith => (0.132, 0.26, 0.41, 1.0e4, 1500.0),
            Material::ColdIce => (0.095, 0.351, 0.55, 1.5e4, 930.0),
        }
    }

    /// Kilograms per cubic metre of the target in the table.
    pub fn table_density(self) -> f64 {
        self.constants().4
    }

    /// `(Kr, Kd)`: the radius and the depth of the bowl in units of the cube root of its
    /// volume. Water is almost a hemisphere, dry sand is shallow, the rest in between;
    /// regolith and ice take the soil values.
    fn shape(self) -> (f64, f64) {
        match self {
            Material::Water => (0.8, 0.75),
            Material::DrySand => (1.4, 0.35),
            _ => (1.1, 0.6),
        }
    }
}

const NU: f64 = 0.33;

/// Rim crest radius over the radius of the crater at the original surface (Housen et al.
/// 1983 profiles, as used in the note).
const RIM_RADIUS: f64 = 1.3;
/// Rim height over the rim diameter (Pike 1977 lunar simple craters).
const RIM_HEIGHT: f64 = 0.036;
/// Share of the crater's volume that is thrown out (the note's value).
const EJECTA_SHARE: f64 = 0.8;
/// Formation time over `sqrt(V^(1/3) / g)` (Schmidt and Housen 1987, via the note). Other
/// sources give 0.5 to 1, so the time is good to a factor of two.
const FORMATION_TIME: f64 = 0.8;

/// A bowl-shaped simple crater, in SI units, with what is thrown out of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crater {
    /// Cubic metres excavated.
    pub volume: f64,
    /// Metres, at the original surface.
    pub radius: f64,
    /// Metres below the original surface.
    pub depth: f64,
    /// Metres to the crest of the rim.
    pub rim_radius: f64,
    /// Metres of the crest above the original surface.
    pub rim_height: f64,
    /// Seconds to form.
    pub duration: f64,
    /// Cubic metres thrown out.
    pub ejecta_volume: f64,
    /// Metres of ejecta at the crest, for a blanket `t(r) = blanket_thickness (rim_radius / r)^3`
    /// outside it that holds `ejecta_volume` (the inverse-cube fall-off of McGetchin et al.
    /// 1973 and Collins et al. 2005).
    pub blanket_thickness: f64,
}

fn positive(name: &str, v: f64) -> Result<f64, String> {
    if v.is_finite() && v > 0.0 {
        Ok(v)
    } else {
        Err(format!("{name} must be a positive finite number, not {v}"))
    }
}

/// The crater of `impact` into `target`.
pub fn crater(impact: &Impact, target: &Target) -> Result<Crater, String> {
    let (m, delta, u) = (
        positive("impactor mass", impact.mass)?,
        positive("impactor density", impact.density)?,
        positive("impact speed along the normal", impact.normal_speed)?,
    );
    let g = positive("gravity", target.gravity)?;
    let (k1, k2, mu, table_strength, table_density) = target.material.constants();
    let rho = positive("target density", target.density.unwrap_or(table_density))?;
    let y = target.strength.unwrap_or(table_strength);
    if !(y.is_finite() && y >= 0.0) {
        return Err(format!("target strength must be a non-negative finite number, not {y}"));
    }
    let a = (3.0 * m / (4.0 * std::f64::consts::PI * delta)).cbrt();
    let ratio = rho / delta;
    let pi2 = g * a / (u * u);
    let pi3 = y / (rho * u * u);
    let gravity_term = pi2 * ratio.powf((6.0 * NU - 2.0 - mu) / (3.0 * mu));
    let strength_term = (k2 * pi3 * ratio.powf((6.0 * NU - 2.0) / (3.0 * mu))).powf((2.0 + mu) / 2.0);
    let pi_v = k1 * (gravity_term + strength_term).powf(-3.0 * mu / (2.0 + mu));
    let volume = pi_v * m / rho;
    if !(volume.is_finite() && volume > 0.0) {
        return Err("the impact makes no finite crater".into());
    }
    let (kr, kd) = target.material.shape();
    let scale = volume.cbrt();
    let radius = kr * scale;
    let rim_radius = RIM_RADIUS * radius;
    let ejecta_volume = EJECTA_SHARE * volume;
    Ok(Crater {
        volume,
        radius,
        depth: kd * scale,
        rim_radius,
        rim_height: RIM_HEIGHT * 2.0 * rim_radius,
        duration: FORMATION_TIME * (scale / g).sqrt(),
        ejecta_volume,
        blanket_thickness: ejecta_volume / (2.0 * std::f64::consts::PI * rim_radius * rim_radius),
    })
}

/// What the engine makes of an impact's energy for smoke. The literature gives ranges, not
/// values, for how much of it goes into a rising plume (the target's internal energy is
/// 0.70 to 0.91 of an impact's at 5 to 45 km/s in strong rock, O'Keefe and Ahrens 1977; the
/// ejecta's kinetic energy 0.07 to 0.5; no value is published for the part in a plume), so
/// these are the engine's own, with defaults inside those ranges and no published value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmokeParams {
    /// Share of the impact's energy, scaled by the angle, that heats the dust. Engine default 0.1.
    pub heat_fraction: f64,
    /// Share of the ejected volume that is dust held in the smoke. Engine default 0.01.
    pub dust_fraction: f64,
    /// Joules per kilogram and kelvin of the dust. Default 1000.
    pub specific_heat: f64,
    /// The most the dust is heated by, kelvin: a declared physical cap (vaporisation), not a
    /// fallback. Default 5000, below the solver's own limit of 50000.
    pub max_temperature: f64,
}

impl Default for SmokeParams {
    fn default() -> Self {
        SmokeParams { heat_fraction: 0.1, dust_fraction: 0.01, specific_heat: 1000.0, max_temperature: 5000.0 }
    }
}

/// The smoke an impact makes, in SI units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Smoke {
    /// Cubic metres of solid dust put into the smoke.
    pub dust_volume: f64,
    /// Kilograms of it.
    pub dust_mass: f64,
    /// Joules that heat it.
    pub heat: f64,
    /// Kelvin the dust is heated by, at most `max_temperature`.
    pub temperature_rise: f64,
    /// Seconds over which a continuous source delivers it: the crater's formation time.
    pub duration: f64,
}

/// The smoke of `impact` into `crater` of a target of density `target_density`. `speed` is the
/// whole relative speed, of which the impact's `normal_speed` is the part along the normal, so
/// that the angle from the surface has `sin(theta) = normal_speed / speed`.
///
/// The dust is `dust_fraction` of the ejected volume and, as a volume fraction of solids, does
/// not depend on a unit of mass. The heat is `heat_fraction x (1/2) m U^2 x sin(theta)^1.5`:
/// shock energy falls with the sine of the angle to the 3/2 in the 3D hydrocode runs of Pierazzo
/// and Melosh (2000, doi 10.1146/annurev.earth.28.1.141). It all goes into the dust, which
/// warms by `heat / (mass x specific_heat)` up to the cap. The physics is simple on purpose: a
/// body falling a few metres makes almost no heat, which is the true behaviour in physical units.
pub fn smoke(
    impact: &Impact,
    speed: f64,
    crater: &Crater,
    target_density: f64,
    params: &SmokeParams,
) -> Result<Smoke, String> {
    positive("impact speed", speed)?;
    positive("target density", target_density)?;
    if speed.partial_cmp(&impact.normal_speed) == Some(std::cmp::Ordering::Less) {
        return Err("the speed along the normal cannot exceed the speed".into());
    }
    if !(params.heat_fraction.is_finite() && (0.0..=1.0).contains(&params.heat_fraction)) {
        return Err("heatFraction must be between 0 and 1".into());
    }
    if !(params.dust_fraction.is_finite() && params.dust_fraction > 0.0 && params.dust_fraction <= 1.0) {
        return Err("dustFraction must be above 0 and at most 1".into());
    }
    positive("specificHeat", params.specific_heat)?;
    positive("maxTemperature", params.max_temperature)?;
    let dust_volume = params.dust_fraction * crater.ejecta_volume;
    let dust_mass = dust_volume * target_density;
    let sine = impact.normal_speed / speed;
    let heat = params.heat_fraction * 0.5 * impact.mass * speed * speed * sine.powf(1.5);
    Ok(Smoke {
        dust_volume,
        dust_mass,
        heat,
        temperature_rise: (heat / (dust_mass * params.specific_heat)).min(params.max_temperature),
        duration: crater.duration,
    })
}
