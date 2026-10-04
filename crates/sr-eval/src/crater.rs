//! Timed crater configuration shared by rendering and simulation.
//!
//! A crater is either authored (radius, depth, rim and the interval it grows over) or grows
//! from an impact: `crater@source` names a rigid body, and the size and timing come from
//! the first impact of that body on the crater's owner, by the scaling law of
//! [`sr_sim::cratering`].
use crate::{
    sim::{curve_of, num, text},
    FrameNode,
};
use sr_model::element::{children, Element};
use sr_sim::cratering::{Impact, Material, Target};
use sr_sim::physics3d::Impact3;

pub struct Deformation {
    pub kernel: sr_3d::crater::Crater,
    pub progress: f64,
    pub max_bytes: usize,
}

/// A crater that grows from an impact, as a frame carries it: the impact has happened,
/// `age` seconds ago in composition time, and the crater has the shape `spec` once grown,
/// over `duration` seconds, in the owner's object space.
#[derive(Debug, Clone)]
pub struct ImpactCrater {
    pub age: f64,
    pub duration: f64,
    pub spec: sr_3d::crater::Spec,
    /// What the impact itself was, for what it causes besides the crater (smoke).
    pub(crate) cause: ImpactCause,
}

/// The physical impact behind a crater, in SI units.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ImpactCause {
    /// Composition time of the impact.
    pub(crate) time: f64,
    /// The crater the law gives, metres and seconds.
    pub(crate) law: sr_sim::cratering::Crater,
    pub(crate) impactor: Impact,
    pub(crate) material: Material,
    /// Metres per second of the whole relative speed, and the direction of that velocity in the
    /// world's axes (into the surface).
    pub(crate) speed: f64,
    pub(crate) velocity: [f64; 3],
    /// Kilograms per cubic metre of the target.
    pub(crate) target_density: f64,
    /// Scene units per metre.
    pub(crate) pixels_per_meter: f64,
}

/// What a crater needs, besides the impact, to grow from it. Lengths are in scene units; the
/// law reads and gives metres, which `pixels_per_meter` converts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CraterSource {
    pub(crate) material: Material,
    pub(crate) density: Option<f64>,
    pub(crate) strength: Option<f64>,
    /// Metres per second squared.
    pub(crate) gravity: f64,
    /// Kilograms.
    pub(crate) mass: f64,
    /// Kilograms per cubic metre.
    pub(crate) source_density: f64,
    pub(crate) pixels_per_meter: f64,
    /// The owner's scale, which is uniform.
    pub(crate) scale: f64,
    pub(crate) influence_depth: Option<f64>,
    pub(crate) curve: Option<String>,
    pub(crate) max_bytes: usize,
}

impl CraterSource {
    /// The crater element's side of it: material, overrides and budget. The body's side
    /// (mass, density, scale) and the world's (gravity, scale of units) come from `body`.
    pub(crate) fn read(
        element: &dyn Element,
        owner: &str,
        gravity: f64,
        pixels_per_meter: f64,
        mass: f64,
        source_volume: f64,
        scale: f64,
    ) -> Result<CraterSource, String> {
        let material = text(element, "targetMaterial")
            .and_then(|name| Material::parse(&name))
            .ok_or_else(|| format!("{owner}: crater targetMaterial is missing or unknown"))?;
        let gravity = element_number(element, "gravity").unwrap_or(gravity);
        if !(gravity.is_finite() && gravity > 0.0) {
            return Err(format!(
                "{owner}: a crater from an impact needs gravity, and the physics gravity is zero; give the crater a gravity"
            ));
        }
        let metres = source_volume / pixels_per_meter.powi(3);
        Ok(CraterSource {
            material,
            density: element_number(element, "targetDensity"),
            strength: element_number(element, "strength"),
            gravity,
            mass,
            source_density: mass / metres,
            pixels_per_meter,
            scale,
            influence_depth: element_number(element, "influenceDepth"),
            curve: text(element, "curve"),
            max_bytes: (num(element, "maxMemoryMiB", 128.) as usize)
                .checked_mul(1 << 20)
                .ok_or("crater memory budget overflow")?,
        })
    }
}

fn element_number(e: &dyn Element, name: &str) -> Option<f64> {
    text(e, name).and_then(|s| s.trim().parse().ok())
}

/// The crater `impact` makes, `age` seconds after it.
pub(crate) fn impact_crater(source: &CraterSource, impact: &Impact3, age: f64) -> Result<ImpactCrater, String> {
    let impactor = Impact {
        mass: source.mass,
        density: source.source_density,
        normal_speed: impact.closing_speed / source.pixels_per_meter,
    };
    let law = sr_sim::cratering::crater(
        &impactor,
        &Target {
            material: source.material,
            density: source.density,
            strength: source.strength,
            gravity: source.gravity,
        },
    )?;
    // metres to the owner's object space
    let units = source.pixels_per_meter / source.scale;
    let (radius, depth, rim_height) = (law.rim_radius * units, law.depth * units, law.rim_height * units);
    let spec = sr_3d::crater::Spec {
        center: impact.point.map(|c| c / source.scale),
        outward: impact.normal,
        radius,
        depth,
        rim_height,
        // the rim reaches from the crater's edge out to its crest
        rim_width: (law.rim_radius - law.radius) * units,
        influence_depth: source.influence_depth.unwrap_or(2. * radius.max(depth).max(rim_height)),
    };
    let speed = impact.relative_velocity.iter().map(|c| c * c).sum::<f64>().sqrt() / source.pixels_per_meter;
    let cause = ImpactCause {
        time: impact.time,
        law,
        impactor,
        material: source.material,
        speed,
        velocity: impact.relative_velocity,
        target_density: source.density.unwrap_or(source.material.table_density()),
        pixels_per_meter: source.pixels_per_meter,
    };
    Ok(ImpactCrater { age, duration: law.duration, spec, cause })
}

pub fn at(node: &FrameNode) -> Result<Option<Deformation>, String> {
    let from_impact = node.crater_impact.as_deref();
    from_element_with(&*node.elem, node.local_time, from_impact)
}

pub(crate) fn from_element(owner: &dyn Element, time: f64) -> Result<Option<Deformation>, String> {
    from_element_with(owner, time, None)
}

/// The deformation a crater with `impact` makes now.
pub(crate) fn from_impact(element: &dyn Element, impact: &ImpactCrater) -> Result<Deformation, String> {
    let progress = if impact.age <= 0. {
        0.
    } else if impact.age >= impact.duration {
        1.
    } else {
        curve_of(element, "curve").apply(impact.age / impact.duration)
    };
    let max_bytes =
        (num(element, "maxMemoryMiB", 128.) as usize).checked_mul(1 << 20).ok_or("crater memory budget overflow")?;
    Ok(Deformation { kernel: sr_3d::crater::Crater::new(impact.spec)?, progress, max_bytes })
}

fn from_element_with(
    owner: &dyn Element,
    time: f64,
    impact: Option<&ImpactCrater>,
) -> Result<Option<Deformation>, String> {
    let Some(c) = children(owner).into_iter().find(|c| c.element_name() == "crater") else { return Ok(None) };
    if text(c, "source").is_some() {
        // the crater of an impact that has happened, or none yet: a shape that does nothing
        return match impact {
            Some(impact) => from_impact(c, impact).map(Some),
            None => {
                let kernel = sr_3d::crater::Crater::new(sr_3d::crater::Spec {
                    center: [0.; 3],
                    outward: [0., 0., -1.],
                    radius: 1.,
                    depth: 0.,
                    rim_height: 0.,
                    rim_width: 1.,
                    influence_depth: 1.,
                })?;
                let max_bytes = (num(c, "maxMemoryMiB", 128.) as usize)
                    .checked_mul(1 << 20)
                    .ok_or("crater memory budget overflow")?;
                Ok(Some(Deformation { kernel, progress: 0., max_bytes }))
            }
        };
    }
    let radius = num(c, "radius", 50.);
    let depth = num(c, "depth", 10.);
    let rim_height = num(c, "rimHeight", 2.);
    let kernel = sr_3d::crater::Crater::new(sr_3d::crater::Spec {
        center: [num(c, "centerX", 0.), num(c, "centerY", 0.), num(c, "centerZ", 0.)],
        outward: [num(c, "normalX", 0.), num(c, "normalY", 0.), num(c, "normalZ", -1.)],
        radius,
        depth,
        rim_height,
        rim_width: num(c, "rimWidth", 10.),
        influence_depth: num(c, "influenceDepth", 2. * radius.max(depth).max(rim_height)),
    })?;
    let start = num(c, "start", 0.);
    let end = num(c, "end", 1.);
    if !time.is_finite() || !start.is_finite() || !end.is_finite() || end <= start {
        return Err("crater requires finite local time and an ordered growth interval".into());
    }
    let progress = if time <= start {
        0.
    } else if time >= end {
        1.
    } else {
        curve_of(c, "curve").apply((time - start) / (end - start))
    };
    let max_bytes =
        (num(c, "maxMemoryMiB", 128.) as usize).checked_mul(1 << 20).ok_or("crater memory budget overflow")?;
    Ok(Some(Deformation { kernel, progress, max_bytes }))
}
