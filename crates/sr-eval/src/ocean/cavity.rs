//! The cavity a body makes of the water it enters, from the scaling law of a crater in water.
use super::*;
use colliders::Colliders;
use sr_sim::cratering::{self, Impact, Material, Target};

/// A water impulse that comes from a body: found once, at the body's first entry.
pub(super) struct Entry {
    source: Arc<str>,
    /// `None` while the body has not entered; `Some(None)` when its entry made nothing.
    found: Option<Option<Impulse>>,
}

/// The entries the ocean's `waterImpulse` children with a `source` ask for.
pub(super) fn read(e: &sr_model::model::Ocean, colliders: &[String]) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for c in children(e).into_iter().filter(|c| c.element_name() == "waterImpulse") {
        let Some(source) = text(c, "source") else { continue };
        if !colliders.contains(&source) {
            return Err(format!("water impulse source {source} is not in the ocean's colliders"));
        }
        entries.push(Entry { source: source.as_str().into(), found: None });
    }
    Ok(entries)
}

impl Entry {
    /// The impulses of this entry for the canonical step that ends at ocean-local `time`: the
    /// one cavity, when the body's first entry is inside that step.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn events(
        &mut self,
        colliders: &Colliders,
        spec: &Spec,
        ocean: &str,
        frame_at: &mut dyn FnMut(f64) -> Result<Arc<FrameGraph>, String>,
        time: f64,
        pixels_per_meter: f64,
    ) -> Result<Vec<Impulse>, String> {
        if self.found.is_none() {
            // Entries are asked for in order, a step at a time, so the first crossing seen is the first.
            let Some(crossing) = colliders.crossing(&self.source, spec, ocean, frame_at, time)? else {
                return Ok(Vec::new());
            };
            self.found = Some(cavity(&crossing, spec, pixels_per_meter)?);
        }
        let from = time - spec.dt;
        Ok(self.found.iter().flatten().filter(|i| i.time > from && i.time <= time).cloned().collect())
    }
}

/// The cavity of an entry, in ocean-local units; none when the body enters outside the ocean.
fn cavity(c: &colliders::Crossing, spec: &Spec, pixels_per_meter: f64) -> Result<Option<Impulse>, String> {
    let inside =
        (0..2).all(|a| (spec.origin[a]..spec.origin[a] + spec.cells[a] as f64 * spec.cell_size).contains(&c.centre[a]));
    if !inside {
        return Ok(None);
    }
    // metres per ocean-local unit
    let metres = c.scale / pixels_per_meter;
    let volume = c.volume * metres.powi(3);
    if !(volume.is_finite() && volume > 0. && c.mass.is_finite() && c.mass > 0.) {
        return Err("a body that enters the water needs a volume and a mass".into());
    }
    let law = cratering::crater(
        &Impact { mass: c.mass, density: c.mass / volume, normal_speed: c.speed * metres },
        &Target { material: Material::Water, density: None, strength: None, gravity: spec.gravity * metres },
    )?;
    // the disc that is emptied is the kernel's central disc, half the impulse's radius, and it is
    // never smaller than two cells across each way, so that it covers cell centres
    let radius = (2. * law.radius / metres).max(4. * spec.cell_size);
    // The central kernel (1 - 4 r^2)^2 holds pi radius^2 / 12 per unit of amplitude.
    let amplitude = 12. * (law.volume / metres.powi(3)) / (std::f64::consts::PI * radius * radius);
    Ok(Some(Impulse {
        time: c.time,
        center: c.centre,
        radius,
        amplitude,
        velocity: [0.; 2],
        kind: ImpulseKind::Cavity,
    }))
}
