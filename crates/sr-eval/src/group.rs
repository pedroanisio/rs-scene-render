//! Solvers that depend on one another: a coupled group.
//!
//! A rigid body that moves through an ocean is carried by the water the ocean moves around it,
//! and the ocean needs the body's pose at each of its steps. Neither can be replayed alone by
//! asking the other again, since that would restore the other's checkpoints in turn. A group
//! therefore passes what each needs through an immutable log: the ocean writes, once, the
//! outcome of each canonical step (what the water offered about each body), and the rigid world reads from it the load for each of its
//! own steps. Replaying either member reads the same records.
//!
//! The group is implicit: an ocean whose `colliders` list a rigid body forms one with the rigid
//! world, and no attribute says so. A step of the rigid world that starts while the ocean's
//! canonical step `m` is the last to have completed reads the outcome of step `m - 1` (a delay of
//! one whole ocean step). That delay is what makes the exchange causal: to advance step `k` the
//! ocean samples the bodies at `k * dt` and at `(k + 1) * dt`, the second to measure their
//! velocity, so it reads the rigid world a full step ahead of the outcomes it has. The ocean runs
//! first in a frame, its steps pull the rigid world ahead in time order, and the rigid world
//! never needs an outcome that does not exist. A load that is not there is an error, never a
//! stand-in: the last known value would make the result depend on the order of calls.
//!
//! Something else may read the rigid world past the ocean's reach (smoke and 3D particles look
//! one of their own steps ahead), so after answering a frame the group steps the ocean on, up
//! to the outcomes those readers can ask for. The group therefore simulates a little beyond the
//! instant asked for.
//!
//! Every member of a group must run on the composition clock: the ocean's local time is the
//! composition time less its start, with no remapping, because the exchange is indexed by it.

use std::sync::{Arc, Mutex};

use sr_model::element::children;
use sr_sim::exchange::{ExchangeLog, Put, Record};
use sr_sim::hydrostatics::{buoyant_load, halfway, place, submerged_mesh, submerged_sphere, Surface, Water};
use sr_sim::physics3d::{BodyState, Load3};

use crate::program::Program;
use crate::sim::{num, text};

/// What an ocean gave the bodies in it during one canonical step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Exchange {
    /// Horizontal momentum per unit water density that the bodies gave the water.
    pub(crate) momentum: [f64; 2],
}

/// What the water offered about one body in a canonical step: where the body stands in it, the
/// plane of the free surface under it, and the horizontal momentum per unit density that the
/// body gave the water in the step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Around {
    /// The body's tag in the ocean: its place in the `colliders` list.
    pub(crate) owner: u32,
    /// Columns of its footprint that hold water.
    pub(crate) wet: u32,
    /// Centre of those columns (x, z) in the ocean's own axes, and the surface there and its slopes
    /// (`y` down).
    pub(crate) centroid: [f64; 2],
    pub(crate) surface: [f64; 3],
    pub(crate) impulse: [f64; 2],
    /// What the slope of the bed the body raises gave the water in the step: credited to the body and recorded, not
    /// applied to it.
    pub(crate) pressure: [f64; 2],
}

impl From<&sr_sim::ocean::BodySample> for Around {
    fn from(s: &sr_sim::ocean::BodySample) -> Around {
        Around {
            owner: s.owner,
            wet: s.wet as u32,
            centroid: s.centroid,
            surface: s.surface,
            impulse: s.impulse,
            pressure: s.pressure,
        }
    }
}

impl Record for Around {
    fn same(&self, other: &Self) -> bool {
        let bits = |a: &[f64], b: &[f64]| a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits());
        self.owner == other.owner
            && self.wet == other.wet
            && bits(&self.centroid, &other.centroid)
            && bits(&self.surface, &other.surface)
            && bits(&self.impulse, &other.impulse)
            && bits(&self.pressure, &other.pressure)
    }
}

/// The plane of the water's free surface under a body, in the ocean's own axes: `level` at
/// `centroid`, falling away with `slope`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Plane {
    pub(crate) centroid: [f64; 2],
    pub(crate) level: f64,
    pub(crate) slope: [f64; 2],
}

/// An ocean as the world sees it at an instant.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WaterFrame {
    /// The water's surface in the world's axes: the `Plane` asked for, or the rest level.
    pub(crate) surface: Surface,
    /// Where the ocean's own x and z axes point in the world.
    pub(crate) axes: [[f64; 3]; 2],
}

/// What gives an ocean's place in the world at an instant, with the plane of the water under a body if
/// there is one.
pub(crate) type WaterAt<'a> = dyn FnMut(&GroupOcean, Option<&Plane>) -> Result<WaterFrame, String> + 'a;

/// What a coupling is asked for the load on one body.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // read by the couplings that load bodies
pub(crate) struct Reaction {
    /// Which ocean of the group.
    pub(crate) ocean: usize,
    /// The canonical step whose outcome this is.
    pub(crate) step: u64,
    /// Index of the body in the 3D world.
    pub(crate) body: usize,
    pub(crate) exchange: Exchange,
    /// The ocean's canonical step, seconds.
    pub(crate) dt: f64,
}

/// A rigid body's mass and shape, for the loads that depend on them.
#[derive(Clone, Debug)]
pub(crate) struct BodyHull {
    pub(crate) id: Arc<str>,
    /// Kilograms.
    pub(crate) mass: f64,
    shape: sr_sim::physics3d::Shape3,
    /// Whether forces move it: the water loads only a body that they do.
    dynamic: bool,
}

impl BodyHull {
    pub(crate) fn new(id: Arc<str>, mass: f64, shape: &sr_sim::physics3d::Shape3) -> BodyHull {
        BodyHull { id, mass, shape: shape.clone(), dynamic: true }
    }

    /// A body that the water does not load: static, or moved by the scene and not by forces.
    pub(crate) fn fixed(mut self) -> BodyHull {
        self.dynamic = false;
        self
    }
}

/// A coupling that reads an ocean's outcome for a canonical step.
pub(crate) type ReactionFn = Arc<dyn Fn(&Reaction) -> Option<Load3> + Send + Sync>;

/// How the water loads a body.
#[derive(Clone)]
pub(crate) enum Coupling {
    /// By the momentum the bodies gave the water in a canonical step, the sum of what the ocean offered about
    /// each body in it. Nothing in a document builds one: a document couples by buoyancy.
    #[allow(dead_code)]
    Reaction(ReactionFn),
    /// By the weight of the water the body displaces below the ocean's rest surface.
    Buoyancy(Arc<Buoyant>),
}

impl std::fmt::Debug for Coupling {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Coupling::Reaction(_) => f.write_str("Reaction"),
            Coupling::Buoyancy(_) => f.write_str("Buoyancy"),
        }
    }
}

/// The buoyancy of the bodies one ocean carries.
pub(crate) struct Buoyant {
    /// By index of the body in the 3D world.
    hulls: Vec<(usize, Hull, f64, Arc<str>)>,
    water: Water,
    /// The rigid world's step, seconds.
    step: f64,
    /// Whether the body also gets back the horizontal momentum it gave the water.
    horizontal: bool,
    /// Whether the waterline is the plane of the surface under the body, as the ocean offers it, and
    /// not the rest level.
    read_surface: bool,
}

enum Hull {
    Sphere(f64),
    Mesh(Vec<[f64; 3]>, Vec<[u32; 3]>),
}

impl Buoyant {
    fn new(bodies: &[BodyHull], listed: &[usize], water: Water, step: f64) -> Result<Buoyant, String> {
        let mut hulls = Vec::new();
        for &k in listed {
            let body = &bodies[k];
            // the water loads what forces move: a static seabed has no hull to float, whatever its shape
            if !body.dynamic {
                continue;
            }
            let hull = match &body.shape {
                sr_sim::physics3d::Shape3::Sphere(r) => Hull::Sphere(*r),
                other => {
                    let (points, triangles) = sr_sim::hydrostatics::hull_mesh(other)
                        .map_err(|e| format!("{}: buoyancy of the body {e}", body.id))?;
                    Hull::Mesh(points, triangles)
                }
            };
            hulls.push((k, hull, body.mass, body.id.clone()));
        }
        Ok(Buoyant { hulls, water, step, horizontal: false, read_surface: true })
    }

    /// These bodies also get the momentum they gave the water back, as a horizontal force.
    pub(crate) fn with_horizontal(mut self, horizontal: bool) -> Buoyant {
        self.horizontal = horizontal;
        self
    }

    /// These bodies float at the water's rest level, whatever the waves do under them.
    #[cfg(test)]
    pub(crate) fn at_rest_level(mut self) -> Buoyant {
        self.read_surface = false;
        self
    }

    /// The load on body `index` of the 3D world at the start of a step, with the water at rest
    /// at `surface`; none for a body this ocean does not carry.
    fn load(&self, index: usize, state: &BodyState, surface: &Surface) -> Result<Option<Load3>, String> {
        let Some((_, hull, mass, id)) = self.hulls.iter().find(|(k, ..)| *k == index) else { return Ok(None) };
        // the load is held for the step, so it is the load where the body will be halfway through it
        let velocity = state.velocity.linear;
        let centre = halfway(state.centre, velocity, self.step);
        let origin = halfway(state.pose.pos, velocity, self.step);
        let submerged = match hull {
            Hull::Sphere(r) => submerged_sphere(origin, *r, surface),
            Hull::Mesh(points, triangles) => {
                let world = place(points, origin, state.pose.rot);
                submerged_mesh(&world, triangles, surface).map_err(|e| format!("{id}: {e}"))?
            }
        };
        let made = buoyant_load(&submerged, centre, *mass, velocity[1], &self.water, self.step)
            .map_err(|e| format!("{id}: {e}"))?;
        Ok(Some(made.load))
    }
}

/// Bytes the exchange log of a group may use: records are 16 bytes and an ocean writes one a
/// canonical step, so this is hours of simulated time; exceeding it is an error.
const LOG_BYTES: usize = 16 << 20;

/// One ocean of a group.
#[derive(Debug)]
pub(crate) struct GroupOcean {
    pub(crate) id: Arc<str>,
    /// Canonical step, seconds.
    pub(crate) dt: f64,
    /// Composition time of the ocean's local time zero.
    pub(crate) start: f64,
    /// Indices into the 3D world's bodies of the rigid bodies in its `colliders`.
    pub(crate) bodies: Vec<usize>,
    /// The tag each of those bodies has in the ocean (its place in the `colliders` list), by index of
    /// the body in the 3D world.
    slots: Vec<(usize, u32)>,
    /// The rest level of the water in the ocean's own axes.
    pub(crate) water_level: f64,
    coupling: Coupling,
}

/// The rigid world and the oceans it exchanges with.
#[derive(Clone)]
pub(crate) struct Group {
    /// What the water offered about each body, per ocean and canonical step: the one record of a step.
    around: Arc<Mutex<ExchangeLog<Around>>>,
    oceans: Arc<[GroupOcean]>,
    /// Seconds past a frame's time that something reads the rigid world.
    reach: f64,
    /// The rigid world's step, seconds.
    step: f64,
}

impl std::fmt::Debug for Group {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Group").field("oceans", &self.oceans).field("reach", &self.reach).finish()
    }
}

impl Group {
    /// The group of the document, or `None` when no ocean lists a rigid body or there is no
    /// coupling to run. `bodies` are the effective ids of the 3D world's bodies in order.
    pub(crate) fn detect(
        p: &Program,
        bodies: &[BodyHull],
        rigid_step: f64,
        injected: Option<Coupling>,
    ) -> Result<Option<Group>, String> {
        let mut oceans = Vec::new();
        for (index, n) in p.nodes.iter().enumerate() {
            let sr_model::model::Node::Ocean(e) = &*n.elem else { continue };
            let slots: Vec<(usize, u32)> = text(e, "colliders")
                .unwrap_or_default()
                .split_whitespace()
                .enumerate()
                .filter_map(|(slot, id)| bodies.iter().position(|b| &*b.id == id).map(|body| (body, slot as u32)))
                .collect();
            let listed: Vec<usize> = slots.iter().map(|(body, _)| *body).collect();
            if listed.is_empty() {
                continue;
            }
            let water = Water {
                density: 1000.0,
                gravity: num(e, "gravity", 9.81),
                drag: num(e, "bodyDrag", 1.0),
                pixels_per_meter: p.scene.physics.as_ref().map_or(100.0, |ph| ph.pixels_per_meter.get()),
            };
            let coupling = match (&injected, text(e, "bodyCoupling").as_deref()) {
                (Some(c), _) => c.clone(),
                (None, Some(mode @ ("buoyancy" | "full"))) => Coupling::Buoyancy(Arc::new(
                    Buoyant::new(bodies, &listed, water, rigid_step)?.with_horizontal(mode == "full"),
                )),
                _ => continue,
            };
            // the exchange is indexed by the ocean's own time: it must be the composition's
            let composition = |x: f64| crate::sim::source_sample(p, index as u32, x, x).0;
            if [0.0, 1.0, 7.5].iter().any(|&x| composition(x + n.start) != x + n.start) {
                return Err(format!("{}: an ocean that exchanges with rigid bodies needs the composition clock", n.id));
            }
            let dt = num(e, "dt", 1.0 / 60.0);
            if !(dt.is_finite() && dt > 0.0) {
                return Err(format!("{}: an ocean that exchanges with rigid bodies needs a positive dt", n.id));
            }
            oceans.push(GroupOcean {
                id: n.id.clone(),
                dt,
                start: n.start,
                bodies: listed,
                slots,
                water_level: num(e, "waterLevel", 0.0),
                coupling,
            });
        }
        if oceans.is_empty() {
            return Ok(None);
        }
        let listed = |colliders: Option<String>| {
            colliders.unwrap_or_default().split_whitespace().any(|id| bodies.iter().any(|b| &*b.id == id))
        };
        // everything else that reads the rigid bodies reads them at most one of its own steps ahead
        let mut reach = 0.0f64;
        for n in &p.nodes {
            if matches!(&*n.elem, sr_model::model::Node::Particles3D(_)) && listed(text(&*n.elem, "colliders")) {
                reach = reach.max(num(&*n.elem, "dt", 1.0 / 60.0));
            }
            for c in children(&*n.elem).into_iter().filter(|c| c.element_name() == "pyro") {
                if listed(text(c, "colliders")) {
                    reach = reach.max(num(c, "dt", 1.0 / 60.0));
                }
            }
        }
        Ok(Some(Group {
            around: Arc::new(Mutex::new(ExchangeLog::new(LOG_BYTES))),
            oceans: oceans.into(),
            reach,
            step: rigid_step,
        }))
    }

    /// Seconds past a frame that other solvers may read the rigid bodies.
    pub(crate) fn reach(&self) -> f64 {
        self.reach
    }

    /// The channel of the ocean named `id`.
    pub(crate) fn channel(&self, id: &str) -> Option<usize> {
        self.oceans.iter().position(|o| &*o.id == id)
    }

    /// What the water offered about each body in canonical step `step` of ocean `ocean`, once it has been computed:
    /// the momentum each gave the water and the pressure of the bed it raised, by body.
    pub(crate) fn around_at(&self, ocean: usize, step: u64) -> Option<Vec<Around>> {
        let log = self.around.lock().unwrap_or_else(|e| e.into_inner());
        log.get(ocean as u32, step).map(<[Around]>::to_vec)
    }

    /// Record what the water offered about each body in canonical step `step` of ocean `ocean`.
    pub(crate) fn record_around(&self, ocean: usize, step: u64, around: &[Around]) -> Result<Put, String> {
        let mut log = self.around.lock().unwrap_or_else(|e| e.into_inner());
        log.put(ocean as u32, step, around)
    }

    /// Canonical steps the group holds a record of, over all its oceans.
    #[cfg(test)]
    pub(crate) fn recorded_steps(&self) -> usize {
        self.around.lock().unwrap().len()
    }

    /// The canonical step of `ocean` whose outcome a rigid step that starts at composition time `t`
    /// reads: the one before the last it has completed by then, or none while there is no such step.
    fn step_read(&self, ocean: &GroupOcean, t: f64) -> Option<u64> {
        let local = t - ocean.start;
        if local < 0.0 {
            return None;
        }
        // When the ocean's step is a whole number of rigid steps and a rigid step starts at `local`, count
        // in whole rigid steps, so that every canonical step is read by exactly that many of them: a load
        // that is the momentum of a step over its length must be applied for exactly that length, and
        // rounding the boundary either way gives a window of one step more or less.
        let (per, nth) = (ocean.dt / self.step, local / self.step);
        let whole = |x: f64, tolerance: f64| x.round() >= 1.0 && (x - x.round()).abs() < tolerance;
        let completed = if whole(per, 1e-9) && (nth - nth.round()).abs() < 1e-6 {
            nth.round() as u64 / per.round() as u64
        } else {
            // canonical steps completed at `local`, the same rounding the ocean uses
            let mut completed = (local / ocean.dt).floor() as u64;
            while completed > 0 && completed as f64 * ocean.dt > local {
                completed -= 1;
            }
            while (completed + 1) as f64 * ocean.dt <= local {
                completed += 1;
            }
            completed
        };
        completed.checked_sub(1)
    }

    fn not_computed(body: usize, t: f64, step: u64, ocean: &GroupOcean, reached: Option<u64>) -> String {
        format!(
            "the water load on body {body} at t = {t} needs step {step} of ocean {}, which has not been \
             computed (it has reached {})",
            ocean.id,
            reached.map_or("no step".to_string(), |k| format!("step {k}"))
        )
    }

    /// The load on `body` for a rigid step that starts at composition time `t`, in the state
    /// `state`, from each ocean that carries it. An ocean reads the outcome of the canonical step
    /// before the last one it has completed by then (none while there is no such step, when the
    /// water is at rest). One that couples by buoyancy floats the body on the plane of the free
    /// surface under it in that outcome, or on the rest level where it offered none, and one that
    /// couples fully also gives the body back, as a force over the step, the horizontal momentum
    /// the body gave the water in it. `water` gives the ocean's place in the world and the plane.
    pub(crate) fn load(
        &self,
        t: f64,
        body: usize,
        state: &BodyState,
        water: &mut WaterAt<'_>,
    ) -> Result<Option<Load3>, String> {
        let mut total: Option<Load3> = None;
        for (channel, ocean) in self.oceans.iter().enumerate() {
            if !ocean.bodies.contains(&body) {
                continue;
            }
            let step = self.step_read(ocean, t);
            let load = match &ocean.coupling {
                Coupling::Buoyancy(buoyant) => {
                    // what the water offered about this body in that step; a body it does not list
                    // is in no water
                    let around = match step {
                        None => None,
                        Some(step) => {
                            let log = self.around.lock().unwrap_or_else(|e| e.into_inner());
                            let Some(offered) = log.get(channel as u32, step) else {
                                return Err(Self::not_computed(body, t, step, ocean, log.last_step(channel as u32)));
                            };
                            let slot = ocean.slots.iter().find(|(b, _)| *b == body).map(|(_, slot)| *slot);
                            offered.iter().find(|a| Some(a.owner) == slot).copied()
                        }
                    };
                    let plane = around.filter(|a| buoyant.read_surface && a.wet > 0).map(|a| Plane {
                        centroid: a.centroid,
                        level: a.surface[0],
                        slope: [a.surface[1], a.surface[2]],
                    });
                    let frame = water(ocean, plane.as_ref())?;
                    let mut load = buoyant.load(body, state, &frame.surface)?;
                    if let (true, Some(around)) = (buoyant.horizontal, around) {
                        // momentum per unit density in the ocean's units to newtons-in-scene-units: times the
                        // density, over the scale to the fourth for the momentum and back up by the scale for
                        // the force, over the step the momentum was given in
                        let ppm = buoyant.water.pixels_per_meter;
                        let scale = -buoyant.water.density / (ppm * ppm * ppm * ocean.dt);
                        let sum = load.get_or_insert_with(Load3::default);
                        for i in 0..3 {
                            sum.force[i] +=
                                scale * (around.impulse[0] * frame.axes[0][i] + around.impulse[1] * frame.axes[1][i]);
                        }
                    }
                    load
                }
                Coupling::Reaction(reaction) => {
                    let Some(step) = step else { continue };
                    let log = self.around.lock().unwrap_or_else(|e| e.into_inner());
                    let Some(offered) = log.get(channel as u32, step) else {
                        return Err(Self::not_computed(body, t, step, ocean, log.last_step(channel as u32)));
                    };
                    // the momentum the bodies gave the water in the step is the sum of what each gave
                    let momentum = offered.iter().fold([0.0; 2], |m, a| [m[0] + a.impulse[0], m[1] + a.impulse[1]]);
                    drop(log);
                    (reaction)(&Reaction { ocean: channel, step, body, exchange: Exchange { momentum }, dt: ocean.dt })
                }
            };
            if let Some(load) = load {
                let sum = total.get_or_insert_with(Load3::default);
                for i in 0..3 {
                    sum.force[i] += load.force[i];
                    sum.torque[i] += load.torque[i];
                }
            }
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Evaluator;

    /// A 3-unit-radius rigid sphere half in the water, sliding at 8 units a second, in an
    /// ocean that lists it as a collider. `extra` adds consumers of the rigid world.
    fn scene(extra: &str) -> String {
        format!(
            r#"<scene version="1.3"><project width="64" height="64" fps="20" duration="3"/><composition>
              <object3D id="barge" primitive="sphere" radius="3" segments="16" x="-30" y="1">
                <rigidBody shape="sphere" mass="500" velocityX="8" linearDamping="0" angularDamping="0"/>
              </object3D>
              <ocean id="sea" bedResponse="hydrostatic" width="128" depth="128" cellSize="2" bottomDepth="10" dt="0.05" boundary="closed" colliders="barge"/>
              {extra}
            </composition>
            <physics gravityY="0" pixelsPerMeter="1" fixedStep="0.01" bounds="none"/></scene>"#
        )
    }

    fn evaluator(xml: &str, coupling: Option<Coupling>, frame_memory: Option<usize>) -> Evaluator {
        let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
        let ev = Evaluator::new(&doc, &Default::default()).unwrap();
        {
            let mut rt = ev.sim.as_ref().unwrap().lock().unwrap();
            rt.coupling = coupling;
            rt.frame_memory = frame_memory;
        }
        ev
    }

    /// The water slows the barge in proportion to the momentum it gave the water.
    fn drag(sign: f64) -> Coupling {
        Coupling::Reaction(Arc::new(move |r: &Reaction| {
            Some(Load3 {
                force: [-4.0 * sign * r.exchange.momentum[0] / r.dt, 0.0, -4.0 * sign * r.exchange.momentum[1] / r.dt],
                torque: [0.0; 3],
            })
        }))
    }

    /// The barge as the rigid world holds it.
    fn barge() -> BodyHull {
        BodyHull::new("barge".into(), 500.0, &sr_sim::physics3d::Shape3::Sphere(3.0))
    }

    fn nothing() -> Coupling {
        Coupling::Reaction(Arc::new(|_: &Reaction| None))
    }

    /// What a frame holds of the two members, bit for bit.
    fn observe(ev: &Evaluator, t: f64) -> (Vec<u64>, u64) {
        let frame = ev.evaluate(t);
        assert!(
            frame.problems.is_empty() && frame.failures.is_empty(),
            "t = {t}: {:?} {:?}",
            frame.problems,
            frame.failures
        );
        let pose =
            frame.nodes.iter().find(|n| &*n.id == "barge").and_then(|n| n.pose3).expect("the barge is simulated");
        let key = frame.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap().key;
        (pose.iter().map(|v| v.to_bits()).collect(), key)
    }

    fn barge_x(ev: &Evaluator, t: f64) -> f64 {
        f64::from_bits(observe(ev, t).0[12])
    }

    #[test]
    fn a_group_with_no_load_changes_nothing() {
        let plain = evaluator(&scene(""), None, None);
        let grouped = evaluator(&scene(""), Some(nothing()), None);
        for t in [0.0, 0.25, 0.8, 1.5, 2.2, 3.0] {
            assert_eq!(observe(&plain, t), observe(&grouped, t), "t = {t}");
        }
        assert!(
            grouped.sim.as_ref().unwrap().lock().unwrap().physics.as_ref().is_some_and(|ph| ph.group.is_some()),
            "the ocean that lists a rigid body forms a group"
        );
    }

    #[test]
    fn the_water_loads_the_body_it_carries_and_only_when_coupled() {
        let free = evaluator(&scene(""), None, None);
        let carried = evaluator(&scene(""), Some(drag(1.0)), None);
        let reversed = evaluator(&scene(""), Some(drag(-1.0)), None);
        let (x_free, x_carried, x_reversed) = (barge_x(&free, 2.0), barge_x(&carried, 2.0), barge_x(&reversed, 2.0));
        assert!((x_free - (-30.0 + 16.0)).abs() < 1e-6, "no coupling: the barge slides on: {x_free}");
        // the load follows the momentum the body gave the water, whichever way that sign points
        assert!((x_carried - x_free).abs() > 0.5, "the water moves it: {x_carried} against {x_free}");
        assert!((x_carried - x_free) * (x_reversed - x_free) < 0.0, "{x_carried} {x_reversed} {x_free}");
        // and the ocean felt the changed body: its wave is another surface
        assert_ne!(observe(&free, 2.0).1, observe(&carried, 2.0).1);
    }

    #[test]
    fn any_order_of_frames_and_a_fresh_group_give_the_same_frames_bit_for_bit() {
        let times = [2.0, 0.5, 1.5, 2.5, 0.0, 2.0, 1.0, 0.75];
        let ev = evaluator(&scene(""), Some(drag(1.0)), None);
        let first: Vec<_> = times.iter().map(|&t| observe(&ev, t)).collect();
        let fresh = evaluator(&scene(""), Some(drag(1.0)), None);
        for (k, &t) in times.iter().enumerate() {
            assert_eq!(first[k], observe(&fresh, t), "t = {t}");
            assert_eq!(first[k], observe(&ev, t), "again, t = {t}");
        }
    }

    #[test]
    fn a_member_replayed_from_its_checkpoints_reads_the_log_and_agrees_with_the_whole_group() {
        // the rigid world keeps no frames, so every request restores a checkpoint and replays
        // its steps, asking the log for each load again
        let times = [2.5, 0.4, 1.7, 0.0, 2.2, 1.1, 2.5];
        let whole = evaluator(&scene(""), Some(drag(1.0)), None);
        let reference: Vec<_> = times.iter().map(|&t| observe(&whole, t)).collect();
        let alone = evaluator(&scene(""), Some(drag(1.0)), Some(0));
        for (k, &t) in times.iter().enumerate() {
            assert_eq!(reference[k], observe(&alone, t), "t = {t}");
        }
    }

    /// What reading the rigid world at `t`, as some other solver would, reports.
    fn read_rigid(ev: &Evaluator, t: f64) -> Vec<String> {
        let mut rt = ev.sim.as_ref().unwrap().lock().unwrap();
        let fields = rt.fields.take().expect("a frame has been evaluated");
        let base = |ts: f64| crate::eval::evaluate(&ev.program, ts);
        let mut graphs = crate::sim::Graphs::new(&base);
        let mut g = crate::eval::evaluate(&ev.program, t);
        let before = g.problems.len();
        crate::sim::apply_physics(&ev.program, rt.physics.as_mut().unwrap(), &mut g, &mut graphs, &fields, t);
        rt.fields = Some(fields);
        g.problems[before..].to_vec()
    }

    #[test]
    fn only_a_body_that_moves_gets_a_hull_so_a_static_collider_cannot_fail_the_ocean() {
        // a static seabed of a shape that has no hull to float (a convex hull) lists beside a ball
        let seabed = BodyHull::new(
            "seabed".into(),
            0.0,
            &sr_sim::physics3d::Shape3::Convex(vec![
                [-1.0, 0.0, -1.0],
                [1.0, 0.0, -1.0],
                [0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0],
            ]),
        )
        .fixed();
        let ball = BodyHull::new("ball".into(), 500.0, &sr_sim::physics3d::Shape3::Sphere(1.0));
        let water = Water { density: 1000.0, gravity: 9.81, drag: 1.0, pixels_per_meter: 1.0 };
        let buoyant = Buoyant::new(&[seabed, ball], &[0, 1], water, 0.01).expect("the seabed is not floated");
        assert_eq!(buoyant.hulls.len(), 1, "a hull for the ball only");
    }

    #[test]
    fn the_group_keeps_one_record_of_each_canonical_step_and_a_reaction_reads_the_bodies_in_it() {
        let seen = Arc::new(Mutex::new(Vec::<(u64, [f64; 2])>::new()));
        let log = seen.clone();
        let coupling = Coupling::Reaction(Arc::new(move |r: &Reaction| {
            log.lock().unwrap().push((r.step, r.exchange.momentum));
            None
        }));
        let ev = evaluator(&scene(""), Some(coupling), None);
        let _ = observe(&ev, 1.0);
        let guard = ev.sim.as_ref().unwrap().lock().unwrap();
        let group = guard.physics.as_ref().unwrap().group.as_ref().unwrap();
        let steps = (0..200u64).filter(|&s| group.around_at(0, s).is_some()).count();
        assert!(steps >= 20, "{steps} steps computed");
        assert_eq!(group.recorded_steps(), steps, "one record of each canonical step");
        // what a reaction is told is the sum of what the bodies gave the water in the step, as it was recorded
        let seen = seen.lock().unwrap();
        assert!(seen.iter().any(|(_, m)| m[0] != 0.0), "the barge gives the water momentum");
        for (step, momentum) in seen.iter() {
            let around = group.around_at(0, *step).expect("the step was recorded");
            let total = around.iter().fold([0.0; 2], |m, a| [m[0] + a.impulse[0], m[1] + a.impulse[1]]);
            assert_eq!(momentum.map(f64::to_bits), total.map(f64::to_bits), "step {step}");
        }
    }

    #[test]
    fn a_reader_past_the_ocean_gets_an_error_naming_the_step_not_a_stand_in() {
        let ev = evaluator(&scene(""), Some(drag(1.0)), None);
        let _ = observe(&ev, 0.5);
        let problems = read_rigid(&ev, 1.5);
        assert!(
            problems.iter().any(|p| p.contains("has not been computed")
                && p.contains("ocean sea")
                && p.contains("needs step 11")
                && p.contains("reached step 10")),
            "{problems:?}"
        );
        // nothing was made up: the same request after the ocean has caught up is fine
        let _ = observe(&ev, 1.5);
        assert!(read_rigid(&ev, 1.5).is_empty());
    }

    #[test]
    fn the_group_steps_the_ocean_on_as_far_as_the_readers_reach() {
        let ev = evaluator(&scene(""), Some(drag(1.0)), None);
        // something that reads the rigid world up to a second past a frame
        let _ = observe(&ev, 0.0);
        ev.sim.as_ref().unwrap().lock().unwrap().physics.as_mut().unwrap().group.as_mut().unwrap().reach = 1.0;
        let _ = observe(&ev, 0.5);
        assert!(read_rigid(&ev, 1.5).is_empty(), "the loads up to 1.5 were already there");
        // but not further than that
        assert!(!read_rigid(&ev, 2.5).is_empty());
    }

    #[test]
    fn the_reach_is_the_longest_step_of_anything_that_reads_the_rigid_bodies() {
        let extra = r#"<object3D id="cloud" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="2" dt="0.2" colliders="barge"><pyroSource radius="2" densityRate="1"/></pyro></object3D>
            <particles3D id="spray" rate="0" speed="0" lifetime="2" dt="0.1" colliders="barge" collisionRadius="0.2"><burst time="0.5" count="4"/></particles3D>
            <particles3D id="elsewhere" rate="0" speed="0" lifetime="2" dt="0.9"><burst time="0.5" count="4"/></particles3D>"#;
        let doc = sr_model::load_str(&scene(extra), &sr_model::LoadOptions::without_assets()).unwrap();
        let p = crate::program::build(&doc, &Default::default()).unwrap();
        let group = Group::detect(&p, &[barge()], 0.01, Some(nothing())).unwrap().unwrap();
        assert!((group.reach() - 0.2).abs() < 1e-12, "{}", group.reach());
        let plain = sr_model::load_str(&scene(""), &sr_model::LoadOptions::without_assets()).unwrap();
        let p = crate::program::build(&plain, &Default::default()).unwrap();
        assert_eq!(Group::detect(&p, &[barge()], 0.01, Some(nothing())).unwrap().unwrap().reach(), 0.0);
    }

    #[test]
    fn an_ocean_that_lists_no_rigid_body_forms_no_group() {
        let xml = scene("").replace(
            r#"<rigidBody shape="sphere" mass="500" velocityX="8" linearDamping="0" angularDamping="0"/>"#,
            "",
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let p = crate::program::build(&doc, &Default::default()).unwrap();
        assert!(Group::detect(&p, &[], 0.01, Some(nothing())).unwrap().is_none());
        assert!(Group::detect(&p, &[barge()], 0.01, None).unwrap().is_none(), "no coupling, no group");
    }

    /// A ball of radius 1 and 2094 kg dropped from 1.5 m above the rest level into a closed 16 m ocean 20 m
    /// deep, carried by buoyancy as a coupling that reads the water the way `read` says.
    fn dropped(read_surface: bool) -> Evaluator {
        let xml = r#"<scene version="1.3"><project width="64" height="64" fps="20" duration="4"/><composition>
              <object3D id="float" primitive="sphere" radius="1" segments="24" y="-1.5">
                <rigidBody shape="sphere" mass="2094" linearDamping="0" angularDamping="0"/>
              </object3D>
              <ocean id="sea" bedResponse="hydrostatic" width="16" depth="16" cellSize="1" bottomDepth="20" dt="0.05" boundary="closed" colliders="float"/>
            </composition>
            <physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/></scene>"#;
        let hull = BodyHull::new("float".into(), 2094.0, &sr_sim::physics3d::Shape3::Sphere(1.0));
        let water = Water { density: 1000.0, gravity: 9.81, drag: 1.0, pixels_per_meter: 1.0 };
        let mut buoyant = Buoyant::new(&[hull], &[0], water, 1.0 / 120.0).unwrap();
        if !read_surface {
            buoyant = buoyant.at_rest_level();
        }
        evaluator(xml, Some(Coupling::Buoyancy(Arc::new(buoyant))), None)
    }

    /// The last of the seconds from the start, in quarters, at which the ball's centre is more than 3 cm
    /// from where its weight holds it, and how far it is from there at 60 s.
    fn settling(ev: &Evaluator) -> (f64, f64) {
        let (mut low, mut high) = (-1.0, 1.0);
        for _ in 0..200 {
            let mid = 0.5 * (low + high);
            let volume = submerged_sphere([0.0, mid, 0.0], 1.0, &Surface { offset: 0.0, slope: [0.0, 0.0] }).volume;
            if volume > 2.094 {
                high = mid;
            } else {
                low = mid;
            }
        }
        let want = 0.5 * (low + high);
        let mut last = 0.0;
        for k in 1..=240 {
            let t = 0.25 * k as f64;
            let frame = ev.evaluate(t);
            assert!(frame.problems.is_empty() && frame.failures.is_empty(), "t = {t}: {:?}", frame.problems);
            let y = frame.nodes.iter().find(|n| &*n.id == "float").and_then(|n| n.pose3).unwrap()[13];
            if (y - want).abs() > 0.03 {
                last = t;
            }
            if k == 240 {
                return (last, y - want);
            }
        }
        unreachable!()
    }

    #[test]
    fn reading_the_surface_under_the_ball_settles_it_sooner_than_the_rest_level() {
        let (with_plane, off_plane) = (settling(&dropped(true)), settling(&dropped(false)));
        println!(
            "WATER within 3 cm for good from {} s reading the surface under the ball (off by {:.4} m at 60 s), {} s at the rest level ({:.4} m)",
            with_plane.0, with_plane.1, off_plane.0, off_plane.1
        );
        assert!(with_plane.1.abs() < 0.03 && off_plane.1.abs() < 0.03, "both settle at the draft the weight needs");
        assert!(with_plane.0 < off_plane.0, "{} s against {} s", with_plane.0, off_plane.0);
    }
}
