//! The cavity a body makes of the water it enters, from the scaling law of a crater in water.
use super::*;
use colliders::Colliders;
use sr_sim::cratering::{self, Impact, Material, Target};

/// A water impulse that comes from a body: found once, at the body's first entry.
pub(super) struct Entry {
    source: Arc<str>,
    /// `None` while the body has not entered; `Some(None)` when its entry made nothing.
    found: Option<Option<Cavity>>,
}

/// The cavity of an entry as a whole: the impulse it would be if it formed at once (at the instant of
/// entry), and the time over which the law has it form.
struct Cavity {
    whole: Impulse,
    duration: f64,
}

impl Cavity {
    /// The part of the cavity that forms in the step `(from, time]`: one impulse in the middle of the stretch
    /// of the step in which it is forming, with the fraction of the whole it is; the parts of every step
    /// together are the whole.
    fn part(&self, from: f64, time: f64) -> Option<Impulse> {
        let start = self.whole.time;
        let (lo, hi) = (start.max(from), time.min(start + self.duration));
        if hi <= lo {
            return None;
        }
        let before = grown((lo - start) / self.duration);
        let part = grown((hi - start) / self.duration) - before;
        let at = (0.5 * (lo + hi)).clamp(from.next_up(), time);
        (part > 0.).then(|| Impulse {
            time: at,
            amplitude: self.whole.amplitude * part,
            kind: ImpulseKind::CavityPart { share: part.min(1.), before },
            ..self.whole.clone()
        })
    }
}

/// How much of a cavity has formed a fraction `s` of the way through its formation: smooth at both
/// ends, from 0 to 1. The shape of the growth is the engine's choice; the law gives only the duration.
fn grown(s: f64) -> f64 {
    let s = s.clamp(0., 1.);
    s * s * (3. - 2. * s)
}

/// Cells across, at least, of the ring that receives the water of a cavity.
const RING_CELLS: f64 = 4.;

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
        // The part that forms in this step, as one impulse in the middle of the stretch of the step in
        // which the cavity is forming; the parts of all the steps are the whole.
        let parts = self.found.iter().flatten().filter_map(|c| c.part(from, time));
        Ok(parts.collect())
    }
}

/// The cavity of an entry, in ocean-local units; none when the body enters outside the ocean.
fn cavity(c: &colliders::Crossing, spec: &Spec, pixels_per_meter: f64) -> Result<Option<Cavity>, String> {
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
    // The disc that is emptied is the kernel's central disc, half the impulse's radius. Its profile
    // (1 - (r/R)^2)^2 holds pi R^2 / 3 per unit of depth at the centre, so a volume V with the depth d that
    // the law gives for the bowl takes R = sqrt(3 V / (pi d)): the peak of the removal is then the law's
    // depth, where the law's own radius would ask for about twice that of a profile so peaked, more than the
    // water can give. Neither the disc nor the ring that receives the water is narrower than RING_CELLS
    // cells, so the water goes to a smooth ring that the grid resolves.
    let (volume_u, depth_u) = (law.volume / metres.powi(3), law.depth / metres);
    let disc =
        if depth_u > 0. { (3. * volume_u / (std::f64::consts::PI * depth_u)).sqrt() } else { law.radius / metres };
    let radius = (2. * disc).max(2. * RING_CELLS * spec.cell_size);
    // The central kernel (1 - 4 r^2)^2 holds pi radius^2 / 12 per unit of amplitude.
    let amplitude = 12. * volume_u / (std::f64::consts::PI * radius * radius);
    Ok(Some(Cavity {
        whole: Impulse {
            time: c.time,
            center: c.centre,
            radius,
            amplitude,
            velocity: [0.; 2],
            kind: ImpulseKind::Cavity,
        },
        duration: law.duration,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whole(time: f64) -> Cavity {
        Cavity {
            whole: Impulse {
                time,
                center: [0.; 2],
                radius: 10.,
                amplitude: 3.,
                velocity: [0.; 2],
                kind: ImpulseKind::Cavity,
            },
            duration: 0.43,
        }
    }

    #[test]
    fn the_parts_of_every_step_are_the_whole_cavity_and_each_lies_inside_its_step() {
        let dt = 1.0 / 24.0;
        for start in [0.3, 0.3 + 0.5 * dt, 7.0 * dt, 0.0123] {
            let c = whole(start);
            let (mut total, mut volume) = (0., 0.);
            for k in 1..40 {
                let (from, time) = ((k - 1) as f64 * dt, k as f64 * dt);
                if let Some(part) = c.part(from, time) {
                    assert!(
                        part.time > from && part.time <= time,
                        "start {start}, step {k}: {} not in ({from}, {time}]",
                        part.time
                    );
                    let ImpulseKind::CavityPart { share, .. } = part.kind else { panic!("a part") };
                    assert!((part.amplitude - 3. * share).abs() < 1e-15);
                    total += share;
                    volume += part.amplitude;
                }
            }
            assert!((total - 1.).abs() < 1e-12, "start {start}: {total}");
            assert!((volume - 3.).abs() < 1e-12, "start {start}: {volume}");
        }
    }

    #[test]
    fn it_forms_smoothly_slowest_at_the_ends_and_a_step_longer_than_it_takes_holds_all_of_it() {
        let c = whole(1.0);
        let dt = 0.43 / 8.;
        let shares: Vec<f64> = (1..=8)
            .map(|k| match c.part(1.0 + (k - 1) as f64 * dt, 1.0 + k as f64 * dt).unwrap().kind {
                ImpulseKind::CavityPart { share, .. } => share,
                _ => unreachable!(),
            })
            .collect();
        assert!(shares[0] < shares[3] && shares[7] < shares[4], "{shares:?}");
        let one = c.part(0.9, 2.0).unwrap();
        assert_eq!(one.kind, ImpulseKind::CavityPart { share: 1.0, before: 0.0 });
        assert!(c.part(0.0, 0.9).is_none() && c.part(1.5, 2.0).is_none());
    }
}
