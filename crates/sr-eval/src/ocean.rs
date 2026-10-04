//! Ocean fields, local clocks and bounded native surface geometry.
use crate::{
    program::Program,
    sim::{num, text, FieldSrc, Graphs, PhysicsRt},
    FrameGraph, FrameNode,
};
use sr_model::element::children;
use sr_sim::ocean::{self as sim, Cell, Impulse, ImpulseKind, Spec};
use std::{collections::HashMap, sync::Arc};
mod bathymetry;
mod cavity;
mod colliders;
mod surface;
mod whitewater;

#[derive(Clone, Debug)]
pub struct SimOcean {
    pub frame: sim::Frame,
    pub mesh: sr_3d::Primitive,
    pub whitewater: Option<sim::whitewater::Frame>,
    /// Foam and spray are each one bounded batch of native triangles.
    pub whitewater_mesh: [sr_3d::Primitive; 2],
    pub key: u64,
}
struct Runtime {
    solver: sim::Ocean,
    spec: Spec,
    bed: Vec<f64>,
    waves: Vec<sim::waves::Wave>,
    whitewater: Option<sim::whitewater::Whitewater>,
    surface_bytes: usize,
    last: Option<Arc<SimOcean>>,
    /// Craters that move the bed; `None` keeps the bed fixed at its bathymetry.
    colliders: Option<colliders::Colliders>,
    /// Cavities of bodies that enter the water.
    entries: Vec<cavity::Entry>,
}
#[derive(Default)]
pub(crate) struct Sims {
    runtimes: HashMap<Arc<str>, Result<Runtime, String>>,
}
fn build(p: &Program, n: &FrameNode) -> Result<Runtime, String> {
    let sr_model::model::Node::Ocean(e) = &*n.elem else { unreachable!("ocean node") };
    let f = |k, d| num(e, k, d);
    let bytes = |k, d| (f(k, d) as usize).checked_mul(1 << 20).ok_or("ocean memory overflow".to_string());
    let width = f("width", 64.);
    let depth = f("depth", 64.);
    let dx = f("cellSize", 1.);
    let collider_ids = colliders::ids(e);
    let mut spec = Spec {
        cells: [(width / dx).round() as usize, (depth / dx).round() as usize],
        origin: [-width / 2., -depth / 2.],
        cell_size: dx,
        dt: f("dt", 1. / 60.),
        gravity: f("gravity", 9.81),
        damping: f("damping", 0.),
        dry_tolerance: f("dryTolerance", 1e-10),
        boundary: match text(e, "boundary").as_deref() {
            Some("open") => sim::Boundary::Open,
            Some("periodic") => sim::Boundary::Periodic,
            _ => sim::Boundary::Closed,
        },
        order: if text(e, "order").as_deref() == Some("2") { sim::Order::Second } else { sim::Order::First },
        max_bytes: bytes("maxMemoryMiB", 256.)?,
        checkpoint_bytes: bytes("checkpointMemoryMiB", 64.)?,
        max_work: f("maxWork", 100_000_000.) as u64,
        moving_bed: false,
        bodies: false,
        body_owners: 0,
    };
    let count = spec.cells[0].checked_mul(spec.cells[1]).ok_or("ocean cell count overflow")?;
    if count == 0 || count > 4_000_000 || count.saturating_mul(256).saturating_add(4096) > spec.max_bytes {
        return Err("ocean grid exceeds memory or cell budget".into());
    }
    let bed =
        bathymetry::load(p, n, &spec, f("waterLevel", 0.) + f("bottomDepth", 10.), bytes("meshMemoryMiB", 128.)?)?;
    let cells = bed
        .iter()
        .map(|b| {
            let depth = (b - f("waterLevel", 0.)).max(0.);
            Cell {
                depth,
                velocity: if depth > 0. { [f("initialVelocityX", 0.), f("initialVelocityZ", 0.)] } else { [0.; 2] },
            }
        })
        .collect();
    let mut impulses = Vec::new();
    let mut waves = Vec::new();
    for c in children(e) {
        let v = |k, d| num(c, k, d);
        match c.element_name() {
            "waterImpulse" if text(c, "source").is_some() => {}
            "waterImpulse" => impulses.push(Impulse {
                time: v("time", 0.),
                center: [v("x", 0.), v("z", 0.)],
                radius: v("radius", 1.),
                amplitude: v("amplitude", 1.),
                velocity: [v("velocityX", 0.), v("velocityZ", 0.)],
                kind: if text(c, "type").as_deref() == Some("add-water") {
                    ImpulseKind::AddWater
                } else {
                    ImpulseKind::Displace
                },
            }),
            "wave" => {
                let random = crate::rng::hash(&[e.seed, waves.len() as u64, 0x6f6365616e]);
                let phase = ((random >> 11) as f64 / (1u64 << 53) as f64) * 360.;
                waves.push(sim::waves::Wave {
                    wavelength: v("wavelength", 16.),
                    amplitude: v("amplitude", 1.),
                    direction: v("direction", 0.),
                    phase: v("phase", phase),
                    speed: v("speed", (spec.gravity * f("bottomDepth", 10.)).sqrt()),
                });
            }
            _ => {}
        }
    }
    let colliders = if collider_ids.is_empty() {
        None
    } else {
        let built =
            colliders::Colliders::build(p, &collider_ids, &spec, f("waterLevel", 0.), bytes("meshMemoryMiB", 128.)?)?;
        spec.moving_bed = true;
        spec.bodies = built.has_bodies();
        spec.body_owners = built.body_count();
        Some(built)
    };
    let entries = cavity::read(e, &collider_ids)?;
    let solver = sim::Ocean::new(spec.clone(), bed.clone(), cells, impulses).map_err(|e| e.to_string())?;
    let whitewater = e
        .children
        .iter()
        .find_map(|c| match c {
            sr_model::model::OceanChild::Whitewater(w) => Some(w),
            _ => None,
        })
        .map(|c| {
            let f = |k, d| num(c, k, d);
            let cfg = sim::whitewater::Settings {
                seed: c.seed,
                rate: f("emissionRate", 10.),
                threshold: f("threshold", 0.5),
                start: f("start", 0.),
                end: text(c, "end").map(|_| f("end", 0.)),
                lifetime: f("lifetime", 3.),
                spray_fraction: f("sprayFraction", 0.4),
                launch_speed: f("launchSpeed", 3.),
                drag: f("drag", 0.1),
                radius: f("radius", 0.05),
                max_particles: f("maxParticles", 10_000.) as usize,
                max_bytes: (f("maxMemoryMiB", 64.) as usize).saturating_mul(1 << 20),
                max_work: f("maxWork", 100_000_000.) as u64,
                checkpoint_bytes: (f("checkpointMemoryMiB", 64.) as usize).saturating_mul(1 << 20),
            };
            sim::whitewater::Whitewater::new(spec.clone(), bed.clone(), cfg).map_err(|e| e.to_string())
        })
        .transpose()?;
    Ok(Runtime {
        solver,
        spec,
        bed,
        waves,
        whitewater,
        surface_bytes: bytes("surfaceMemoryMiB", 128.)?,
        last: None,
        colliders,
        entries,
    })
}
/// The solver's frame at `time`, over the driven bed when there is a driver.
fn frame_at<'a>(
    solver: &'a mut sim::Ocean,
    time: f64,
    driver: Option<&mut dyn sim::Driver>,
) -> Result<&'a sim::Frame, sim::Error> {
    match driver {
        Some(driver) => solver.at_driven(time, driver),
        None => solver.at(time),
    }
}

/// Everything about a frame the compositor can tell apart, bed included.
fn frame_key(frame: &sim::Frame, triangles: usize) -> u64 {
    let mut key = crate::rng::hash(&[frame.time.to_bits(), triangles as u64, 0x6f6365616e]);
    for cell in &frame.cells {
        key = crate::rng::hash(&[key, cell.depth.to_bits(), cell.velocity[0].to_bits(), cell.velocity[1].to_bits()]);
    }
    // The same depths over another bed are another surface.
    for bed in &frame.bed {
        key = crate::rng::hash(&[key, bed.to_bits(), 0x626564]);
    }
    key
}

impl Sims {
    pub(crate) fn apply(
        &mut self,
        p: &Program,
        g: &mut FrameGraph,
        graphs: &mut Graphs<'_>,
        fields: &FieldSrc,
        mut physics: Option<&mut PhysicsRt>,
    ) {
        let at = g.time;
        for i in 0..g.nodes.len() {
            let n = &g.nodes[i];
            if n.kind != "ocean" {
                continue;
            }
            let id = n.id.clone();
            let local_time = n.local_time;
            let node = p.nodes.iter().position(|pn| pn.id == id).map(|v| v as u32);
            if node.is_some_and(|node| !crate::sim::linear_emitter_clock(p, node)) {
                // A looping or remapped parent clock revisits source times on other
                // branches, so the bed history is replayed for this frame's mapping.
                let moves = self.runtimes.get(&id).is_some_and(|rt| rt.as_ref().is_ok_and(|rt| rt.colliders.is_some()));
                if moves {
                    self.runtimes.remove(&id);
                }
            }
            let rt = self.runtimes.entry(id.clone()).or_insert_with(|| build(p, &g.nodes[i]));
            let result = (|| -> Result<Arc<SimOcean>, String> {
                let rt = rt.as_mut().map_err(|e| e.clone())?;
                if let Some(last) = rt.last.as_ref().filter(|s| s.frame.time == local_time) {
                    return Ok(last.clone());
                }
                let Runtime { solver, spec, bed, waves, whitewater, surface_bytes, last, colliders, entries } = rt;
                let ppm = p.scene.physics.as_ref().map_or(100.0, |ph| ph.pixels_per_meter.get());
                // The bed is a function of ocean-local time, read from the scene at every
                // solver step; a failure in it is reported as the solver's.
                let failure = std::cell::RefCell::new(None::<String>);
                let start = node.map_or(0., |node| p.nodes[node as usize].start);
                // what the bodies in this ocean get back from it, if it exchanges with them
                let group = physics.as_deref().and_then(|ph| ph.group.clone());
                let channel = group.as_ref().and_then(|group| group.channel(&id));
                let mut bed_driver = |time: f64, forcing: &mut sim::Forcing| -> Result<(), sim::Error> {
                    // a step's outcome is written before anything reads the bodies it loads
                    if let (Some(group), Some(channel), Some((step, momentum))) = (&group, channel, forcing.exchange) {
                        group.record(channel, step, crate::group::Exchange { momentum }).map_err(|message| {
                            *failure.borrow_mut() = Some(message);
                            sim::Error::Invalid("ocean exchange diverged")
                        })?;
                    }
                    if let (Some(group), Some(channel), Some((step, _))) = (&group, channel, forcing.exchange) {
                        let around: Vec<crate::group::Around> = forcing.bodies.iter().map(Into::into).collect();
                        group.record_around(channel, step, &around).map_err(|message| {
                            *failure.borrow_mut() = Some(message);
                            sim::Error::Invalid("ocean exchange diverged")
                        })?;
                    }
                    #[cfg(test)]
                    if let Some((step, _)) = forcing.exchange {
                        tests::OFFERS.with(|o| o.borrow_mut().push((step, forcing.bodies.clone())));
                    }
                    let (Some(colliders), Some(node)) = (colliders.as_mut(), node) else {
                        return Err(sim::Error::Invalid("ocean collider without a scene node"));
                    };
                    let mut scene_at = |sim_time: f64| -> Result<Arc<FrameGraph>, String> {
                        let (t, clocks) = crate::sim::source_sample(p, node, sim_time + start, at);
                        let mut frame = if clocks.is_empty() {
                            graphs.at(t)
                        } else {
                            Arc::new(crate::eval::evaluate_with_clocks(p, t, &clocks))
                        };
                        if let Some(physics) = physics.as_deref_mut() {
                            let before = frame.problems.len();
                            crate::sim::apply_physics(p, physics, Arc::make_mut(&mut frame), graphs, fields, t);
                            // a rigid world that could not answer must not be read as an empty one
                            if frame.problems.len() > before {
                                return Err(frame.problems[before..].join("; "));
                            }
                        }
                        Ok(frame)
                    };
                    colliders.sample(spec, &id, &mut scene_at, time, bed, forcing).map_err(|message| {
                        *failure.borrow_mut() = Some(message);
                        sim::Error::Invalid("ocean bed sampling failed")
                    })?;
                    for entry in entries.iter_mut() {
                        let events =
                            entry.events(colliders, spec, &id, &mut scene_at, time, ppm).map_err(|message| {
                                *failure.borrow_mut() = Some(message);
                                sim::Error::Invalid("ocean water entry failed")
                            })?;
                        forcing.events.extend(events);
                    }
                    Ok(())
                };
                let moving = spec.moving_bed;
                let reported = |error: sim::Error| failure.borrow_mut().take().unwrap_or_else(|| error.to_string());
                let foam = whitewater
                    .as_mut()
                    .map(|w| {
                        w.at(local_time, |time| {
                            let driver: Option<&mut dyn sim::Driver> =
                                if moving { Some(&mut bed_driver) } else { None };
                            sim::waves::apply(spec, frame_at(solver, time, driver)?, waves)
                        })
                        .cloned()
                        .map_err(|e| e.to_string())
                    })
                    .transpose();
                let foam = match foam {
                    Ok(foam) => foam,
                    Err(message) => return Err(failure.borrow_mut().take().unwrap_or(message)),
                };
                let driver: Option<&mut dyn sim::Driver> = if moving { Some(&mut bed_driver) } else { None };
                let base = frame_at(solver, local_time, driver).map_err(reported)?;
                let frame = sim::waves::apply(spec, base, waves).map_err(|e| e.to_string())?;
                let bed_now: &[f64] = if frame.bed.is_empty() { bed } else { &frame.bed };
                let mesh = surface::mesh(spec, bed_now, &frame, *surface_bytes)?;
                let mut key = frame_key(&frame, mesh.indices.len());
                let whitewater_mesh = if let Some(foam) = &foam {
                    let available = surface_bytes.saturating_sub(surface::memory_cost(spec)?);
                    for p in &foam.particles {
                        key = crate::rng::hash(&[
                            key,
                            p.id,
                            p.position[0].to_bits(),
                            p.position[1].to_bits(),
                            p.position[2].to_bits(),
                            p.kind as u64,
                        ]);
                    }
                    whitewater::meshes(foam, available)?
                } else {
                    Default::default()
                };
                let out = Arc::new(SimOcean { frame, mesh, whitewater: foam, whitewater_mesh, key });
                *last = Some(out.clone());
                // Other solvers read the rigid bodies a little past this frame: step on, to the
                // last canonical step that reach covers, so the loads they need exist.
                if let Some(group) = &group {
                    // a reader at `local_time + reach` needs the outcome of the step before the one that
                    // ends there, which is complete once the solver has stepped to its end
                    let target = (((local_time + group.reach()) / spec.dt).floor() - 1.0) * spec.dt;
                    if target > local_time {
                        let driver: Option<&mut dyn sim::Driver> = if moving { Some(&mut bed_driver) } else { None };
                        frame_at(solver, target, driver).map_err(reported)?;
                    }
                }
                Ok(out)
            })();
            match result {
                Ok(out) => g.nodes[i].sim_ocean = Some(out),
                Err(e) => {
                    // a failed solver is a problem and a failure
                    let message = format!("{id}: {e}");
                    g.problems.push(message.clone());
                    g.failures.push(message);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        /// What the bed driver was offered per body with each completed step.
        pub(super) static OFFERS: RefCell<Vec<(u64, Vec<sim::BodySample>)>> = const { RefCell::new(Vec::new()) };
    }

    #[test]
    fn the_columns_of_a_body_are_tagged_with_its_place_in_the_collider_list() {
        let xml = r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><composition>
          <object3D id="seabed" primitive="plane" width="400" height="400" segments="20" y="12" rotationX="-90">
            <crater radius="40" depth="2" rimHeight="1" rimWidth="8" start="2" end="2.5"/>
          </object3D>
          <object3D id="left" primitive="sphere" radius="6" x="-30" y="0"/>
          <object3D id="right" primitive="sphere" radius="6" x="30" y="2"/>
          <ocean id="sea" width="128" depth="128" cellSize="2" bottomDepth="12" dt="0.0416666666666667" boundary="closed" colliders="seabed right left"/>
        </composition></scene>"#;
        let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = crate::Evaluator::new(&doc, &Default::default()).unwrap();
        OFFERS.with(|o| o.borrow_mut().clear());
        let frame = ev.evaluate(0.5);
        assert!(frame.problems.is_empty(), "{:?}", frame.problems);
        let offers = OFFERS.with(|o| o.borrow().clone());
        let (_, bodies) = offers.iter().find(|(step, _)| *step == 3).expect("an offer for step 3");
        // the surface is entry 0 of the list; `right` is 1 and `left` is 2
        assert_eq!(bodies.iter().map(|b| b.owner).collect::<Vec<_>>(), [1, 2]);
        for b in bodies {
            assert!(b.columns > 4 && b.wet == b.columns, "{b:?}");
            // the displaced water stands above the rest level (y downward) and is almost at rest
            assert!(b.surface[0] < -1.0 && b.surface[1].abs() < 1e-5 && b.surface[2].abs() < 1e-5, "{b:?}");
            assert!((b.bed - 12.0).abs() < 1e-6, "{b:?}");
            assert!(b.velocity.iter().all(|v| v.abs() < 1e-5), "{b:?}");
        }
        // `right` sits 2 lower, so it displaces more water than `left`... and holds more columns
        assert!(
            bodies[0].centroid[0] > 0.0 && bodies[1].centroid[0] < 0.0,
            "{:?}",
            [bodies[0].centroid, bodies[1].centroid]
        );
    }

    fn frame(bed: Vec<f64>) -> sim::Frame {
        sim::Frame { time: 1.0, cells: vec![Cell { depth: 12., velocity: [0.; 2] }; 4], bed }
    }

    #[test]
    fn the_frame_key_tells_the_same_depths_over_different_beds_apart() {
        let flat = frame(vec![12.; 4]);
        let raised = frame(vec![12., 12., 11., 12.]);
        assert_ne!(frame_key(&flat, 24), frame_key(&raised, 24));
        assert_eq!(frame_key(&flat, 24), frame_key(&frame(vec![12.; 4]), 24));
        // Without a driver the key is the one a frame without a bed always had.
        let fixed = frame(Vec::new());
        let mut expected = crate::rng::hash(&[1.0_f64.to_bits(), 24, 0x6f6365616e]);
        for cell in &fixed.cells {
            expected = crate::rng::hash(&[
                expected,
                cell.depth.to_bits(),
                cell.velocity[0].to_bits(),
                cell.velocity[1].to_bits(),
            ]);
        }
        assert_eq!(frame_key(&fixed, 24), expected);
    }
}
