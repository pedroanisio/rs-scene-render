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
    /// How many particle emitters fall into this ocean (`splash`): its canonical steps read what they bring.
    splash: usize,
}
/// Brings what the ocean named (first) needs of other solvers up to the ocean's canonical step (second): computes the
/// particles that fall into it as far as that step needs, which they cannot do before the ocean has got to the step
/// before.
pub(crate) type Pull<'a> =
    dyn FnMut(&FrameGraph, &mut Graphs<'_>, Option<&mut PhysicsRt>, &str, u64) -> Result<(), String> + 'a;

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
        body_push: false,
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
        let built = colliders::Colliders::build(
            p,
            &collider_ids,
            &spec,
            f("waterLevel", 0.),
            bytes("meshMemoryMiB", 128.)?,
            text(e, "bedResponse").as_deref() != Some("hydrostatic"),
            f("bodyDrag", 1.),
        )?;
        spec.moving_bed = true;
        spec.bodies = built.has_bodies();
        spec.body_owners = built.body_count();
        spec.body_push = built.pushes();
        Some(built)
    };
    let entries = cavity::read(e, &collider_ids)?;
    let splash = text(e, "splash").map_or(0, |l| l.split_whitespace().count());
    if splash > 0 {
        // the driver gives what the particles bring at the sample that closes each canonical step
        spec.moving_bed = true;
    }
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
        splash,
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
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply(
        &mut self,
        p: &Program,
        g: &mut FrameGraph,
        graphs: &mut Graphs<'_>,
        fields: &FieldSrc,
        mut physics: Option<&mut PhysicsRt>,
        splash: &crate::splash::Log,
        pull: &mut Pull<'_>,
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
                let Runtime {
                    solver,
                    spec,
                    bed,
                    waves,
                    whitewater,
                    surface_bytes,
                    last,
                    colliders,
                    entries,
                    splash: takes,
                } = rt;
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
                    if let Some((step, momentum)) = forcing.exchange {
                        tests::OFFERS.with(|o| o.borrow_mut().push((step, momentum, forcing.bodies.clone())));
                    }
                    // what the particles that fell into the ocean bring, at the sample that closes the step
                    if let (true, Some((step, _))) = (*takes > 0, forcing.exchange) {
                        let read = match splash.read(&id, step) {
                            Ok(read) if splash.knows(&id, *takes) => Ok(read),
                            // the particles are not there yet: they are made known and computed to it now, which
                            // asks the rigid world for what the water has already given
                            _ => pull(&*g, graphs, physics.as_deref_mut(), &id, step).and_then(|()| {
                                if splash.knows(&id, *takes) {
                                    splash.read(&id, step)
                                } else {
                                    // an empty read here would be nobody saying anything, not nothing falling
                                    Err(format!(
                                        "ocean {id} takes the splash of {} emitters and not all are known to it",
                                        takes
                                    ))
                                }
                            }),
                        }
                        .map_err(|message| {
                            *failure.borrow_mut() = Some(message);
                            sim::Error::Invalid("ocean splash unavailable")
                        })?;
                        forcing.splash = read;
                    }
                    let (Some(colliders), Some(node)) = (colliders.as_mut(), node) else {
                        if colliders.is_none() {
                            // nothing moves the bed: it is the bathymetry
                            forcing.bed.copy_from_slice(bed);
                            return Ok(());
                        }
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

    /// What a bed driver was offered with a completed step: its number, the momentum all the bodies gave
    /// the water in it, and the water around each body.
    pub(super) type Offer = (u64, [f64; 2], Vec<sim::BodySample>);

    thread_local! {
        /// What the bed driver was offered per body with each completed step.
        pub(super) static OFFERS: RefCell<Vec<Offer>> = const { RefCell::new(Vec::new()) };
    }

    #[test]
    fn the_columns_of_a_body_are_tagged_with_its_place_in_the_collider_list() {
        let xml = r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><composition>
          <object3D id="seabed" primitive="plane" width="400" height="400" segments="20" y="12" rotationX="-90">
            <crater radius="40" depth="2" rimHeight="1" rimWidth="8" start="2" end="2.5"/>
          </object3D>
          <object3D id="left" primitive="sphere" radius="6" x="-30" y="0"/>
          <object3D id="right" primitive="sphere" radius="6" x="30" y="2"/>
          <ocean id="sea" bedResponse="hydrostatic" width="128" depth="128" cellSize="2" bottomDepth="12" dt="0.0416666666666667" boundary="closed" colliders="seabed right left"/>
        </composition></scene>"#;
        let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = crate::Evaluator::new(&doc, &Default::default()).unwrap();
        OFFERS.with(|o| o.borrow_mut().clear());
        let frame = ev.evaluate(0.5);
        assert!(frame.problems.is_empty(), "{:?}", frame.problems);
        let offers = OFFERS.with(|o| o.borrow().clone());
        let (_, _, bodies) = offers.iter().find(|(step, ..)| *step == 3).expect("an offer for step 3");
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

    /// A sphere of `radius` at depth `y`, moving at (`vx`, `vz`), in water 20 deep that fills 128 x 128.
    fn moving(response: &str, radius: f64, y: f64, vx: f64, drag: f64) -> String {
        format!(
            r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
          <object3D id="rock" primitive="sphere" radius="{radius}" y="{y}" segments="32"><rigidBody shape="sphere" mass="1000" velocityX="{vx}" restitution="0" linearDamping="0" angularDamping="0"/></object3D>
          <ocean id="sea" bedResponse="{response}" bodyDrag="{drag}" width="128" depth="128" cellSize="1" bottomDepth="20" dt="0.0416666666666667" boundary="closed" colliders="rock" maxWork="100000000000"/>
        </composition><physics gravityY="0" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/></scene>"#
        )
    }

    fn offers_of(xml: &str, until: f64) -> Vec<Offer> {
        let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = crate::Evaluator::new(&doc, &Default::default()).unwrap();
        OFFERS.with(|o| o.borrow_mut().clear());
        let frame = ev.evaluate(until);
        assert!(frame.problems.is_empty(), "{:?}", frame.problems);
        OFFERS.with(|o| o.borrow().clone())
    }

    #[test]
    fn the_form_drag_on_the_water_is_that_of_a_sphere_through_still_water() {
        // 2 m radius, 20 m/s through water at rest: F / rho = (1/2) Cd A U^2 with A = pi a^2 and Cd = 1
        let offers = offers_of(&moving("depthFiltered", 2., 10., 20., 1.), 0.2);
        let (_, _, bodies) = offers.iter().find(|(step, ..)| *step == 1).expect("the first step");
        let wanted = 0.5 * 1. * std::f64::consts::PI * 4. * 400. * 0.0416666666666667;
        let given = bodies[0].impulse[0];
        println!("DRAG first step gave {given:.2}, (1/2) Cd A U^2 dt is {wanted:.2}");
        assert!((given - wanted).abs() < 0.04 * wanted, "{given} against {wanted}");
        assert!(bodies[0].impulse[1].abs() < 1e-9 * given, "along the motion only: {:?}", bodies[0].impulse);
        // a smaller Cd gives proportionally less, and none gives none
        let half = offers_of(&moving("depthFiltered", 2., 10., 20., 0.5), 0.2);
        let (_, _, half) = half.iter().find(|(step, ..)| *step == 1).unwrap();
        assert!((half[0].impulse[0] - 0.5 * given).abs() < 1e-6 * given);
        let none = offers_of(&moving("depthFiltered", 2., 10., 20., 0.), 0.2);
        let (_, _, none) = none.iter().find(|(step, ..)| *step == 1).unwrap();
        assert_eq!(none[0].impulse, [0.; 2]);
    }

    #[test]
    fn the_water_is_pushed_less_as_it_comes_to_move_with_the_body() {
        let offers = offers_of(&moving("depthFiltered", 2., 10., 20., 1.), 2.0);
        let each: Vec<f64> = offers.iter().filter(|(step, ..)| *step >= 1).map(|(_, _, b)| b[0].impulse[0]).collect();
        assert!(each.len() > 20 && each.iter().all(|v| *v > 0.), "{each:?}");
        // the drag is on the velocity of the body relative to the water under it, which the push raises
        assert!(each[10] < each[0], "{} against {}", each[10], each[0]);
    }

    #[test]
    fn what_all_the_bodies_gave_is_what_the_water_received_with_the_credit_for_each() {
        let xml = r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
          <object3D id="a" primitive="sphere" radius="2" x="-40" y="10" segments="24"><rigidBody shape="sphere" mass="1000" velocityX="15" restitution="0" linearDamping="0" angularDamping="0"/></object3D>
          <object3D id="b" primitive="sphere" radius="3" x="40" z="30" y="10" segments="24"><rigidBody shape="sphere" mass="1000" velocityX="-10" velocityZ="-5" restitution="0" linearDamping="0" angularDamping="0"/></object3D>
          <ocean id="sea" width="128" depth="128" cellSize="1" bottomDepth="20" dt="0.0416666666666667" boundary="closed" colliders="a b" maxWork="100000000000"/>
        </composition><physics gravityY="0" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/></scene>"#;
        let offers = offers_of(xml, 1.5);
        for (step, total, bodies) in offers.iter().filter(|(step, ..)| *step >= 1) {
            let sum = bodies.iter().fold([0.; 2], |s, b| [s[0] + b.impulse[0], s[1] + b.impulse[1]]);
            for axis in 0..2 {
                assert!(
                    (sum[axis] - total[axis]).abs() <= 1e-9 * total[axis].abs().max(1e-9),
                    "step {step}: {sum:?} against {total:?}"
                );
            }
            assert_eq!(bodies.len(), 2, "step {step}");
        }
        let (_, _, last) = offers.last().unwrap();
        assert!(last[0].impulse[0] > 0. && last[1].impulse[0] < 0. && last[1].impulse[1] < 0., "{last:?}");
    }

    #[test]
    fn the_frontal_area_of_a_closed_surface_is_what_the_flow_sees_of_it() {
        // a box 2 (x) by 3 (y) by 4 (z) across y in 0..3: flow along x sees 3 x 4, along z sees 2 x 3, and cut at y = 1 and 2 sees one third
        let (x, y, z) = (1., 3., 2.);
        let corners: Vec<[f64; 3]> = (0..8)
            .map(|i| {
                [if i & 1 == 0 { -x } else { x }, if i & 2 == 0 { 0. } else { y }, if i & 4 == 0 { -z } else { z }]
            })
            .collect();
        let faces: [[u32; 3]; 12] = [
            [0, 1, 3],
            [0, 3, 2],
            [4, 6, 7],
            [4, 7, 5],
            [0, 4, 5],
            [0, 5, 1],
            [2, 3, 7],
            [2, 7, 6],
            [0, 2, 6],
            [0, 6, 4],
            [1, 5, 7],
            [1, 7, 3],
        ];
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9 * b.max(1.);
        assert!(close(colliders::frontal_area(&corners, &faces, [1., 0.], -10., 10.), 12.));
        assert!(close(colliders::frontal_area(&corners, &faces, [0., 1.], -10., 10.), 6.));
        assert!(close(colliders::frontal_area(&corners, &faces, [1., 0.], 1., 2.), 4.));
        // at 45 degrees it sees both faces: (3 x 4 + 3 x 2) / sqrt(2)
        assert!(close(
            colliders::frontal_area(&corners, &faces, [0.5f64.sqrt(), 0.5f64.sqrt()], -10., 10.),
            18. * 0.5f64.sqrt()
        ));
        // a sphere of radius 2, tessellated, sees close to pi a^2
        let (rings, around) = (32usize, 48usize);
        let mut points = vec![[0., -2., 0.], [0., 2., 0.]];
        for r in 1..rings {
            let phi = std::f64::consts::PI * r as f64 / rings as f64;
            for a in 0..around {
                let theta = std::f64::consts::TAU * a as f64 / around as f64;
                points.push([2. * phi.sin() * theta.cos(), -2. * phi.cos(), 2. * phi.sin() * theta.sin()]);
            }
        }
        let at = |r: usize, a: usize| (2 + (r - 1) * around + a % around) as u32;
        let mut triangles = Vec::new();
        for a in 0..around {
            triangles.push([0, at(1, a), at(1, a + 1)]);
            triangles.push([1, at(rings - 1, a + 1), at(rings - 1, a)]);
            for r in 1..rings - 1 {
                triangles.push([at(r, a), at(r + 1, a), at(r + 1, a + 1)]);
                triangles.push([at(r, a), at(r + 1, a + 1), at(r, a + 1)]);
            }
        }
        let area = colliders::frontal_area(&points, &triangles, [1., 0.], -10., 10.);
        assert!((area - std::f64::consts::PI * 4.).abs() < 0.01 * std::f64::consts::PI * 4., "{area}");
    }

    #[test]
    fn a_body_in_shallow_water_is_credited_the_pressure_of_the_bed_it_raises() {
        // 6 m of water, a sphere of 2 m radius with its centre 3 m down, moving at 6 m/s: the water it lifts
        // is a mound the width of the body, and the slope of that mound pushes the water along
        let xml = r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
          <object3D id="rock" primitive="sphere" radius="2" x="-30" y="3" segments="24"><rigidBody shape="sphere" mass="1000" velocityX="6" restitution="0" linearDamping="0" angularDamping="0"/></object3D>
          <ocean id="sea" width="128" depth="128" cellSize="1" bottomDepth="6" dt="0.0416666666666667" boundary="closed" colliders="rock" maxWork="100000000000"/>
        </composition><physics gravityY="0" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/></scene>"#;
        let offers = offers_of(xml, 1.5);
        let steps: Vec<_> = offers.iter().filter(|(step, ..)| *step >= 2).collect();
        assert!(steps.len() > 20);
        for (step, _, bodies) in &steps {
            let b = &bodies[0];
            assert!(b.pressure[0].abs() > 0., "step {step}: {b:?}");
            assert!(b.pressure[1].abs() < 1e-6 * b.pressure[0].abs() + 1e-9, "along the motion only: {b:?}");
        }
        let (_, _, last) = steps.last().unwrap();
        println!("PRESSURE last step: pressure {:?} against push {:?}", last[0].pressure, last[0].impulse);
        // a lake without a body that moves has none to credit
        let still = xml.replace(r#"velocityX="6""#, r#"velocityX="0""#);
        let offers = offers_of(&still, 1.0);
        let (_, _, bodies) = offers.last().unwrap();
        assert!(bodies[0].pressure[0].abs() < 1e-3 * 1e3_f64.min(1.), "{:?}", bodies[0].pressure);
    }

    /// The ball of the coupled-ocean tests (radius 2, half the density of water, 3 m/s along x) in a closed basin of
    /// 160 m: the water's momentum, the pushes the bodies gave it and the pressure credited, by canonical step.
    fn balance(response: &str, order: &str, until: f64) -> Vec<(f64, f64, f64, f64)> {
        let xml = format!(
            r##"<scene version="1.3"><project width="64" height="64" fps="20" duration="4"/><composition>
              <object3D id="ball" primitive="sphere" radius="2" segments="24" x="-10" y="0" z="1">
                <rigidBody shape="sphere" mass="16755.16" velocityX="3" linearDamping="0" angularDamping="0"/>
              </object3D>
              <ocean id="sea" bedResponse="{response}" order="{order}" width="160" depth="160" cellSize="2" bottomDepth="10" dt="0.05" boundary="closed" colliders="ball" bodyCoupling="full"/>
            </composition>
            <physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/></scene>"##
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = crate::Evaluator::new(&doc, &Default::default()).unwrap();
        OFFERS.with(|o| o.borrow_mut().clear());
        let mut out = Vec::new();
        for t in [1.0, 2.0, 3.0].into_iter().filter(|t| *t <= until) {
            let frame = ev.evaluate(t);
            assert!(
                frame.problems.is_empty() && frame.failures.is_empty(),
                "{:?} {:?}",
                frame.problems,
                frame.failures
            );
            let sea = frame.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap();
            let water: f64 = sea.frame.cells.iter().map(|c| c.depth * c.velocity[0] * 4.0).sum();
            let steps = (t / 0.05_f64).round() as u64;
            let (mut push, mut pressure) = (0.0, 0.0);
            for (step, _, bodies) in OFFERS.with(|o| o.borrow().clone()) {
                if step >= 1 && step <= steps {
                    push += bodies.iter().map(|b| b.impulse[0]).sum::<f64>();
                    pressure += bodies.iter().map(|b| b.pressure[0]).sum::<f64>();
                }
            }
            out.push((t, water, push, pressure));
        }
        out
    }

    #[test]
    fn the_water_has_what_the_bodies_pushed_into_it_and_the_bed_source_credited_to_them() {
        // water = pushes + pressure + what the walls give, which is nothing until the waves reach them; with the
        // credit the balance closes to what the walls give, in both responses and both orders, where without it
        // the bed source term is 3 to 15% of the water
        for response in ["hydrostatic", "depthFiltered"] {
            for order in ["1", "2"] {
                for (t, water, push, pressure) in balance(response, order, 3.0) {
                    let (without, with) = ((water - push) / water, (water - push - pressure) / water);
                    println!("BALANCE {response} order {order} t {t}: water {water:.4}, push {push:.4}, pressure {pressure:.4}; residual without {:.3}%, with {:.4}%", 100. * without, 100. * with);
                    assert!(without.abs() > 0.03, "{response} {order} {t}: the term is there: {without}");
                    assert!(with.abs() < 0.002, "{response} {order} {t}: closed to the walls: {with}");
                }
            }
        }
    }

    /// The ball of the coupled-ocean tests in a closed basin, with a smoke that reads the rigid world in steps of 0.2 s.
    fn with_smoke(order: &str) -> String {
        format!(
            r##"<scene version="1.3"><project width="64" height="64" fps="20" duration="4"/><composition>
              <object3D id="ball" primitive="sphere" radius="2" segments="24" x="-10" y="0" z="1">
                <rigidBody shape="sphere" mass="16755.16" velocityX="3" linearDamping="0" angularDamping="0"/>
              </object3D>
              <object3D id="cloud" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="2" dt="0.2" colliders="ball"><pyroSource radius="2" densityRate="1"/></pyro></object3D>
              <ocean id="sea" bedResponse="hydrostatic" order="{order}" width="160" depth="160" cellSize="2" bottomDepth="10" dt="0.016666666666666666" boundary="closed" colliders="ball" bodyCoupling="full"/>
            </composition>
            <physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/></scene>"##
        )
    }

    /// The canonical steps the ocean computed for each of the frames 0 to 30 at 20 a second, in the scene whose smoke
    /// reads the rigid world a fifth of a second past each frame, and what the ocean of each frame looked like.
    fn steps_per_frame(order: &str) -> (Vec<usize>, Vec<Vec<sim::Cell>>, f64) {
        let doc = sr_model::load_str(&with_smoke(order), &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = crate::Evaluator::new(&doc, &Default::default()).unwrap();
        OFFERS.with(|o| o.borrow_mut().clear());
        let (mut seen, mut steps, mut seas) = (0, Vec::new(), Vec::new());
        let started = std::time::Instant::now();
        for k in 0..=30 {
            let frame = ev.evaluate(k as f64 * 0.05);
            assert!(
                frame.problems.is_empty() && frame.failures.is_empty(),
                "{:?} {:?}",
                frame.problems,
                frame.failures
            );
            let now = OFFERS.with(|o| o.borrow().len());
            steps.push(now - seen);
            seen = now;
            seas.push(
                frame.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap().frame.cells.clone(),
            );
        }
        (steps, seas, started.elapsed().as_secs_f64())
    }

    #[test]
    fn an_ocean_that_is_stepped_ahead_for_a_reader_does_not_start_again_for_the_next_frame() {
        for order in ["1", "2"] {
            let (steps, seas, seconds) = steps_per_frame(order);
            println!("STEPS order {order}: {steps:?}, {seconds:.3} s");
            // a frame is three canonical steps on, and the smoke asks for twelve more past it: after the first
            // frame no frame recomputes what it has already computed ahead
            assert!(steps.iter().skip(1).all(|&s| s <= 8), "order {order}: {steps:?}");
            // and what it shows is what an evaluator that was never stepped ahead computes from the start
            for k in [7usize, 19, 30] {
                let doc = sr_model::load_str(&with_smoke(order), &sr_model::LoadOptions::without_assets()).unwrap();
                let fresh = crate::Evaluator::new(&doc, &Default::default()).unwrap();
                let frame = fresh.evaluate(k as f64 * 0.05);
                let cells =
                    &frame.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap().frame.cells;
                let bits = |c: &[sim::Cell]| {
                    c.iter().flat_map(|c| [c.depth, c.velocity[0], c.velocity[1]]).map(f64::to_bits).collect::<Vec<_>>()
                };
                assert_eq!(bits(cells), bits(&seas[k]), "order {order}, frame {k}");
            }
        }
    }
}
