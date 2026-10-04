//! World-space particle simulation bound to scene clocks, poses and fields.

use crate::{
    program::Program,
    sim::{num, text, FieldSrc, Graphs, PhysicsRt},
    FrameGraph, FrameNode,
};
use glam::DVec3;
use sr_model::element::children;
use sr_sim::particles3d::{self as sim, Birth, Driver, Emission, Error, Hit};
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Clone)]
pub struct SimParticles3D {
    pub frame: sim::Frame,
    pub key: u64,
    pub size_curve: crate::curve::Ease,
    pub color_curve: crate::curve::Ease,
}

mod colliders;

/// The ejecta of the crater that `crater` names (the effective id of its owner): `count`
/// particles, launched at `angle` degrees above the tangent plane, spread by `spread`.
struct CraterBurst {
    crater: Arc<str>,
    count: usize,
    angle: f64,
    spread: f64,
    /// The particles of the impact, once it is known, in order of birth in emitter time.
    ejecta: Option<Arc<Vec<Birth>>>,
}

thread_local! {
    static GAS_QUERIES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Gas queries made by 3D particle evaluation on the calling thread so far. A frame whose particle state was
/// already simulated makes none.
#[doc(hidden)]
pub fn gas_queries_on_this_thread() -> u64 {
    GAS_QUERIES.with(std::cell::Cell::get)
}

/// The smoke that drags an emitter's particles, and the gas of the steps it has asked for lately.
struct GasRt {
    /// Effective id of the smoke volume's object.
    id: Arc<str>,
    window: Vec<(u64, Arc<sr_sim::pyro::Gas>)>,
}

/// Everything the gas queries of one canonical particle step need, built once for all the particles that ask
/// in it: the volume's matrix at the step's start, the smoke's clock there and the smoke steps that cover the
/// step.
struct GasAt {
    /// Emitter time of the step's start, and the smoke's source time there and how fast it runs.
    t0: f64,
    source0: f64,
    rate: f64,
    dt: f64,
    world: glam::DMat4,
    inverse: glam::DMat4,
    snapshots: Vec<(u64, Arc<sr_sim::pyro::Gas>)>,
}

struct Runtime {
    gas: Option<GasRt>,
    seed: u64,
    /// How far above the surface the ejecta of a crater are born: the largest collision radius.
    lift: f64,
    crater_bursts: Vec<CraterBurst>,
    colliders: colliders::Colliders,
    emitter: sim::Emitter,
    names: Option<Vec<String>>,
    size_curve: crate::curve::Ease,
    color_curve: crate::curve::Ease,
}
#[derive(Default)]
pub(crate) struct Sims {
    runtimes: HashMap<Arc<str>, Result<Runtime, String>>,
}

fn config(n: &FrameNode) -> &sr_model::model::Particles3D {
    let sr_model::model::Node::Particles3D(e) = &*n.elem else { unreachable!("particles3D node") };
    e
}
fn build(p: &Program, node: u32, n: &FrameNode) -> Result<Runtime, String> {
    let e = config(n);
    let number = |k: &str, d: f64| num(e, k, d);
    let vec = |k: &str| ["X", "Y", "Z"].map(|a| number(&format!("{k}{a}"), 0.));
    let bytes =
        |k, d| (number(k, d) as usize).checked_mul(1 << 20).ok_or_else(|| "particle memory overflow".to_string());
    let shape = match text(e, "emitterShape").as_deref() {
        Some("box") => sim::Shape::Box {
            size: [number("emitterWidth", 1.), number("emitterHeight", 1.), number("emitterDepth", 1.)],
        },
        Some("sphere") => sim::Shape::Sphere { radius: number("emitterRadius", 1.) },
        Some("mesh") => {
            let ns = p.nodes[node as usize]
                .doc
                .checked_sub(1)
                .and_then(|d| p.includes.get(d as usize))
                .map(|d| &*d.0)
                .unwrap_or("");
            let id = text(e, "emitterMesh").ok_or("mesh emitter needs emitterMesh")?;
            let key = crate::sim::asset_key(&p.assets, ns, &id);
            let max = bytes("meshMemoryMiB", 128.)?;
            let (points, triangles) = crate::sim3d::mesh_asset_triangles(p, &key, max)?;
            sim::Shape::Mesh(Arc::new(sim::mesh::Mesh::new(&points, &triangles, max).map_err(|e| e.to_string())?))
        }
        _ => sim::Shape::Point,
    };
    let bursts = children(e)
        .into_iter()
        .filter(|c| c.element_name() == "burst" && text(*c, "crater").is_none())
        .map(|c| sim::Burst {
            time: num(c, "time", 0.),
            count: num(c, "count", 0.) as u64,
            repeat: num(c, "repeat", 0.) as u64,
            interval: num(c, "interval", 1.),
        })
        .collect();
    let spec = sim::Spec {
        seed: e.seed,
        start: number("emissionStart", 0.),
        end: e.emission_end.map(|_| number("emissionEnd", 0.)),
        step: number("dt", 1. / 60.),
        lifetime: number("lifetime", 2.),
        lifetime_variance: number("lifetimeVariance", 0.),
        velocity: vec("velocity"),
        rotation: vec("rotation0"),
        rotation_variance: vec("rotationVariance"),
        angular_velocity: vec("angularVelocity"),
        angular_velocity_variance: vec("angularVelocityVariance"),
        scale_variance: number("scaleVariance", 0.),
        direction: [number("directionX", 0.), number("directionY", -1.), number("directionZ", 0.)],
        speed: number("speed", 0.),
        speed_variance: number("speedVariance", 0.),
        spread: number("spread", 0.),
        gravity: vec("gravity"),
        drag: number("drag", 0.),
        radius: number("collisionRadius", 0.5),
        collision_tolerance: number("collisionTolerance", 0.001),
        restitution: number("bounce", 0.5),
        friction: number("friction", 0.),
        shape,
        bursts,
        max_particles: number("maxParticles", 10000.) as usize,
        max_events: number("maxEvents", 16384.) as usize,
        max_bytes: bytes("maxMemoryMiB", 256.)?,
        checkpoint_bytes: bytes("checkpointMemoryMiB", 64.)?,
        max_work: number("maxWork", 100000000.) as u64,
    };
    let crater_bursts = children(e)
        .into_iter()
        .filter(|c| c.element_name() == "burst")
        .filter_map(|c| {
            Some(CraterBurst {
                crater: text(c, "crater")?.into(),
                count: num(c, "count", 0.) as usize,
                angle: num(c, "angle", 45.),
                spread: num(c, "angleSpread", 15.),
                ejecta: None,
            })
        })
        .collect();
    let lift = (number("collisionRadius", 0.5) * (1. + number("scaleVariance", 0.))
        + number("collisionTolerance", 0.001))
    .max(0.);
    Ok(Runtime {
        gas: text(e, "gas").map(|id| GasRt { id: id.as_str().into(), window: Vec::new() }),
        seed: e.seed,
        lift,
        crater_bursts,
        colliders: colliders::build(p, n, bytes("meshMemoryMiB", 128.)?)?,
        emitter: sim::Emitter::new(spec).map_err(|e| e.to_string())?,
        names: crate::sim::field_names(e),
        size_curve: crate::sim::curve_of(e, "sizeCurve"),
        color_curve: crate::sim::curve_of(e, "colorCurve"),
    })
}

struct SceneDriver<'a, 'b> {
    p: &'a Program,
    node: u32,
    id: Arc<str>,
    at: f64,
    graphs: &'a mut Graphs<'b>,
    fields: &'a FieldSrc,
    names: Option<&'a [String]>,
    physics: Option<&'a mut PhysicsRt>,
    frames: Vec<(u64, Arc<FrameGraph>)>,
    colliders: &'a mut colliders::Colliders,
    sweeps: Vec<(u64, u64, Vec<sim::collider::Collider>)>,
    step_origin: f64,
    step: f64,
    seed: u64,
    lift: f64,
    bursts: &'a mut [CraterBurst],
    /// The smoke that drags the particles, with the emitter's drag as the coupling.
    gas: Option<(&'a mut GasRt, &'a mut crate::pyro::Sims, f64)>,
    /// The gas context of the last few canonical steps asked about, by step index; none where the volume is
    /// not in the frame.
    gas_at: Vec<(i64, Option<GasAt>)>,
}
impl SceneDriver<'_, '_> {
    fn frame(&mut self, time: f64) -> Arc<FrameGraph> {
        let key = time.to_bits();
        if let Some((_, f)) = self.frames.iter().find(|(k, _)| *k == key) {
            return f.clone();
        }
        let (t, clocks) =
            crate::sim::source_sample(self.p, self.node, time + self.p.nodes[self.node as usize].start, self.at);
        let mut f = if clocks.is_empty() {
            self.graphs.at(t)
        } else {
            Arc::new(crate::eval::evaluate_with_clocks(self.p, t, &clocks))
        };
        if let Some(ph) = self.physics.as_deref_mut() {
            crate::sim::apply_physics(self.p, ph, Arc::make_mut(&mut f), self.graphs, self.fields, t);
        }
        if self.frames.len() >= 8 {
            self.frames.remove(0);
        }
        self.frames.push((key, f.clone()));
        f
    }

    /// The gas of the smoke's step `step`: from the window of those asked for lately, or simulated now.
    fn gas_step(
        &mut self,
        frame: &Arc<FrameGraph>,
        i: usize,
        step: u64,
        keep: usize,
    ) -> Result<Arc<sr_sim::pyro::Gas>, Error> {
        let Some((gas, pyro, _)) = self.gas.as_mut() else { return Err(Error::Driver("no gas".into())) };
        if let Some((_, found)) = gas.window.iter().find(|(k, _)| *k == step) {
            return Ok(found.clone());
        }
        let step_gas = pyro
            .gas(self.p, frame, i, step, &mut *self.graphs, self.fields, self.physics.as_deref_mut())
            .map_err(|e| Error::Driver(format!("gas {}: {e}", gas.id)))?;
        let (fresh, held) = (step_gas.gas, step_gas.held);
        // the fields in use and the states kept for them are charged to the smoke's memory allowance, and
        // exceeding it is an error
        let clock = crate::pyro::gas_clock(&frame.nodes[i]).map_err(Error::Driver)?;
        let fields = (gas.window.len() + 1).saturating_mul(fresh.bytes());
        if fields.saturating_add(held) > clock.max_bytes {
            return Err(Error::Driver(format!(
                "gas {}: {} velocity fields of {} bytes and {held} bytes of kept states exceed its maxMemoryMiB",
                gas.id,
                gas.window.len() + 1,
                fresh.bytes()
            )));
        }
        gas.window.push((step, fresh.clone()));
        while gas.window.len() > keep {
            let oldest = (0..gas.window.len()).min_by_key(|&k| gas.window[k].0).expect("a window");
            gas.window.remove(oldest);
        }
        Ok(fresh)
    }

    /// The context of the canonical step that emitter time `time` belongs to, as an index into `gas_at`. It is a
    /// function of the time alone: the volume's matrix and the smoke's clock are read at the step's start, so
    /// the answer does not depend on which particle asks or in what order.
    fn gas_context(&mut self, time: f64) -> Result<usize, Error> {
        let index = ((time - self.step_origin) / self.step + 1e-9).floor() as i64;
        if let Some(found) = self.gas_at.iter().position(|(k, _)| *k == index) {
            return Ok(found);
        }
        let Some(id) = self.gas.as_ref().map(|(gas, _, _)| gas.id.clone()) else {
            return Err(Error::Driver("no gas".into()));
        };
        let t0 = self.step_origin + index as f64 * self.step;
        let frame = self.frame(t0);
        let built = match frame.nodes.iter().position(|n| n.id == id) {
            None => None,
            Some(i) => {
                let n = &frame.nodes[i];
                let clock = crate::pyro::gas_clock(n).map_err(Error::Driver)?;
                let rate = crate::pyro::value(&*n.elem, Some(&n.props), "animationSpeed", 1.0);
                let source0 =
                    n.local_time * rate + crate::pyro::value(&*n.elem, Some(&n.props), "animationOffset", 0.0);
                let world = crate::sim3d::world3(&frame, i, 0);
                let inverse = world.inverse();
                if !(source0.is_finite() && world.is_finite() && inverse.is_finite()) {
                    return Err(Error::Driver(format!("gas {id}: the volume's time or transform is not usable")));
                }
                // the smoke steps that cover the particle step, from the one at or before its start to the one
                // after its end
                let first = (source0.max(0.0) / clock.dt + 1e-9).floor() as u64;
                let last = (((source0 + rate * self.step).max(0.0) / clock.dt - 1e-9).ceil() as u64).max(first);
                let keep = (last - first + 1) as usize + 2;
                let mut snapshots = Vec::new();
                for k in first..=last {
                    snapshots.push((k, self.gas_step(&frame, i, k, keep)?));
                }
                Some(GasAt { t0, source0, rate, dt: clock.dt, world, inverse, snapshots })
            }
        };
        if self.gas_at.len() >= 4 {
            self.gas_at.remove(0);
        }
        self.gas_at.push((index, built));
        Ok(self.gas_at.len() - 1)
    }

    /// The velocity, in the world's axes, of the gas at a world position at emitter time `time`: the linear
    /// interpolation in time of the two smoke steps around the instant, each sampled in the volume's own axes.
    /// A volume that is not in the frame is no gas.
    fn gas_velocity(&mut self, time: f64, position: [f64; 3]) -> Result<[f64; 3], Error> {
        GAS_QUERIES.with(|count| count.set(count.get() + 1));
        let slot = self.gas_context(time)?;
        let Some(at) = self.gas_at[slot].1.as_ref() else { return Ok([0.0; 3]) };
        let ratio = (at.source0 + at.rate * (time - at.t0)).max(0.0) / at.dt;
        let nearest = ratio.round();
        let first =
            if (ratio - nearest).abs() <= f64::EPSILON * 4.0 * ratio.max(1.0) { nearest } else { ratio.floor() };
        let weight = (ratio - first).clamp(0.0, 1.0);
        let k = first as u64;
        let found = |k: u64| at.snapshots.iter().find(|(s, _)| *s == k).map(|(_, g)| g.clone());
        // a time a rounding past the step the snapshots were taken for asks for its own
        let (a, b) = match (found(k), found(k + 1)) {
            (Some(a), Some(b)) => (a, Some(b)),
            (Some(a), None) if weight <= 1e-12 => (a, None),
            _ => {
                let frame = self.frame(at.t0);
                let id = self.gas.as_ref().map(|(g, _, _)| g.id.clone()).expect("a gas");
                let i = frame
                    .nodes
                    .iter()
                    .position(|n| n.id == id)
                    .ok_or_else(|| Error::Driver("the smoke is gone".into()))?;
                let a = self.gas_step(&frame, i, k, 8)?;
                (a, if weight > 1e-12 { Some(self.gas_step(&frame, i, k + 1, 8)?) } else { None })
            }
        };
        let at = self.gas_at[slot].1.as_ref().expect("the context is still there");
        let point = at.inverse.transform_point3(DVec3::from(position)).to_array();
        let mut u = a.velocity_at(point);
        if let Some(next) = b {
            let later = next.velocity_at(point);
            u = std::array::from_fn(|c| (1.0 - weight) * u[c] + weight * later[c]);
        }
        Ok(at.world.transform_vector3(DVec3::from(u)).to_array())
    }

    /// The emitter's time at which the source clock reads `t`, for the clocks an emitter can
    /// have here: affine ones that run forward.
    fn emitter_time(&self, t: f64) -> Result<f64, Error> {
        let start = self.p.nodes[self.node as usize].start;
        let (f0, clocks) = crate::sim::source_sample(self.p, self.node, start, self.at);
        if clocks.is_empty() {
            return Ok(t - start);
        }
        let slope = crate::sim::source_sample(self.p, self.node, start + 1., self.at).0 - f0;
        if !(slope.is_finite() && slope > 0.) {
            return Err(Error::Driver("the ejecta of a crater need an emitter clock that runs forward".into()));
        }
        Ok((t - f0) / slope)
    }

    /// The particles that the impact `grown` throws out for burst `k`, in order of birth.
    fn ejecta(&mut self, k: usize, grown: &crate::crater::ImpactCrater) -> Result<Vec<Birth>, Error> {
        let cause = grown.cause;
        let (owner, count, angle, spread) = {
            let b = &self.bursts[k];
            (b.crater.clone(), b.count, b.angle, b.spread)
        };
        let frame = self.frame(self.emitter_time(cause.time)?);
        let i =
            frame.nodes.iter().position(|n| n.id == owner).ok_or_else(|| {
                Error::Driver(format!("the owner of crater ejecta, {owner}, is missing at the impact"))
            })?;
        // the crater is the owner's: where it hit and its axis follow the owner's pose at the impact
        let world = crate::sim3d::world3(&frame, i, 0);
        let normal = world.transform_vector3(DVec3::from(grown.spec.outward));
        // The contact point of the impact lies a little inside the surface, where a particle would be
        // buried under it; they are born clear of the surface by the radius they collide with.
        let point = world.transform_point3(DVec3::from(grown.spec.center)) + normal.normalize_or_zero() * self.lift;
        let metres = cause.pixels_per_meter;
        let list = sr_sim::cratering::ejecta::ejecta(&sr_sim::cratering::ejecta::Spec {
            material: cause.material,
            body_radius: (3. * cause.impactor.mass / (4. * std::f64::consts::PI * cause.impactor.density)).cbrt(),
            body_density: cause.impactor.density,
            target_density: cause.target_density,
            impact_speed: cause.speed,
            velocity_direction: cause.velocity,
            normal: normal.to_array(),
            crater_volume: cause.law.volume,
            crater_radius: cause.law.radius,
            crater_duration: cause.law.duration,
            particles: count,
            seed: crate::rng::hash(&[self.seed, k as u64, 0xe1ec7a]),
            angle,
            angle_spread: spread,
        })
        .map_err(Error::Driver)?;
        list.iter()
            .map(|e| {
                Ok(Birth {
                    time: self.emitter_time(cause.time + e.time)?,
                    position: std::array::from_fn(|c| point[c] + e.position[c] * metres),
                    velocity: e.velocity.map(|v| v * metres),
                    mass: e.mass,
                })
            })
            .collect()
    }
}
impl Driver for SceneDriver<'_, '_> {
    fn emission(&mut self, time: f64) -> Result<Emission, Error> {
        let f = self.frame(time);
        let Some(i) = f.nodes.iter().position(|n| n.id == self.id) else {
            return Ok(Emission { enabled: false, ..Default::default() });
        };
        let n = &f.nodes[i];
        let e = &*n.elem;
        let v = |k, d| n.props.get(k).and_then(crate::Value::as_num).unwrap_or_else(|| num(e, k, d));
        Ok(Emission {
            transform: sr_volume::Transform::new(crate::sim3d::world3(&f, i, 0).to_cols_array())
                .map_err(|e| Error::Driver(e.to_string()))?,
            rate: v("rate", 10.),
            inherited_velocity: [v("inheritedVelocityX", 0.), v("inheritedVelocityY", 0.), v("inheritedVelocityZ", 0.)],
            enabled: true,
        })
    }
    fn acceleration(&mut self, time: f64, position: [f64; 3], velocity: [f64; 3]) -> Result<[f64; 3], Error> {
        let mut a = [0.; 3];
        if !(self.fields.is_empty() || self.names.is_some_and(|n| n.is_empty())) {
            let frame = self.frame(time);
            let fields = match self.names {
                Some(n) => self.fields.named(n, frame.time, Some(&frame)),
                None => self.fields.at(frame.time, Some(&frame)),
            };
            a = sr_sim::fields::total3_for(&fields, position, velocity, frame.time, true);
        }
        // the gas drags the particle with the emitter's own coefficient: with the drag on the particle's
        // velocity the integrator already has, an acceleration of k u makes p'' = k (u - v)
        let drag = self.gas.as_ref().map_or(0., |(_, _, drag)| *drag);
        if drag > 0. {
            let u = self.gas_velocity(time, position)?;
            for k in 0..3 {
                a[k] += drag * u[k];
            }
        }
        Ok(a)
    }
    fn births(&mut self, lo: f64, hi: f64) -> Result<Vec<Birth>, Error> {
        let mut born = Vec::new();
        for k in 0..self.bursts.len() {
            if self.bursts[k].ejecta.is_none() {
                // The impact is known from the first frame after it; before that nothing is thrown out.
                let frame = self.frame(hi);
                let Some(grown) =
                    frame.nodes.iter().find(|n| n.id == self.bursts[k].crater).and_then(|n| n.crater_impact.clone())
                else {
                    continue;
                };
                self.bursts[k].ejecta = Some(Arc::new(self.ejecta(k, &grown)?));
            }
            let Some(list) = self.bursts[k].ejecta.clone() else { continue };
            let first = lo == self.step_origin;
            let from = list.partition_point(|b| if first { b.time < lo } else { b.time <= lo });
            let to = list.partition_point(|b| b.time <= hi);
            born.extend_from_slice(&list[from.min(to)..to]);
        }
        Ok(born)
    }
    fn sweep(&mut self, time: f64, dt: f64, from: [f64; 3], to: [f64; 3], radius: f64) -> Result<Option<Hit>, Error> {
        if self.colliders.is_empty() {
            return Ok(None);
        }
        // All contact refinements in one fixed step use the same linear
        // boundary motion. Resampling an accelerating crater after each impact
        // changes that motion and creates an endless sequence of tiny impacts.
        let offset = (time - self.step_origin) / self.step;
        let rounded = offset.round();
        let offset = if (offset - rounded).abs() <= (8. * f64::EPSILON * offset.abs().max(1.)).min(1e-6) {
            rounded
        } else {
            offset
        };
        let index = offset.floor();
        let start = self.step_origin + index * self.step;
        let key = (start.to_bits(), self.step.to_bits());
        if !self.sweeps.iter().any(|(t, d, _)| (*t, *d) == key) {
            // One interval owns geometry at a time. Keeping eight historical
            // deformations or scales would multiply the collider memory budget.
            self.sweeps.clear();
            let current = self.frame(start);
            let next = self.frame(start + self.step);
            let colliders = colliders::sample(self.colliders, &current, &next, start, self.step)?;
            self.sweeps.push((key.0, key.1, colliders));
        }
        let colliders = &self.sweeps.iter().find(|(t, d, _)| (*t, *d) == key).expect("sweep cache").2;
        let mut earliest: Option<Hit> = None;
        for collider in colliders {
            if let Some(hit) = collider.sweep(time, dt, from, to, radius)? {
                if earliest.as_ref().is_none_or(|h| hit.fraction < h.fraction) {
                    earliest = Some(hit);
                }
            }
        }
        Ok(earliest)
    }
}
impl Sims {
    pub(crate) fn apply(
        &mut self,
        p: &Program,
        g: &mut FrameGraph,
        graphs: &mut Graphs<'_>,
        fields: &FieldSrc,
        mut physics: Option<&mut PhysicsRt>,
        pyro: &mut crate::pyro::Sims,
    ) {
        for i in 0..g.nodes.len() {
            let n = &g.nodes[i];
            if n.kind != "particles3D" {
                continue;
            }
            let id = n.id.clone();
            let Some(node) = p.nodes.iter().position(|n| n.id == id).map(|n| n as u32) else {
                continue;
            };
            if !crate::sim::linear_emitter_clock(p, node) {
                self.runtimes.remove(&id);
            }
            let rt = self.runtimes.entry(id.clone()).or_insert_with(|| build(p, node, n));
            let result = (|| -> Result<Arc<SimParticles3D>, String> {
                let rt = rt.as_mut().map_err(|e| e.clone())?;
                let mut d = SceneDriver {
                    p,
                    node,
                    id: id.clone(),
                    at: g.time,
                    graphs,
                    fields,
                    names: rt.names.as_deref(),
                    physics: physics.as_deref_mut(),
                    frames: Vec::new(),
                    colliders: &mut rt.colliders,
                    sweeps: Vec::new(),
                    step_origin: num(&*n.elem, "emissionStart", 0.),
                    step: num(&*n.elem, "dt", 1. / 60.),
                    seed: rt.seed,
                    lift: rt.lift,
                    bursts: &mut rt.crater_bursts,
                    gas: rt.gas.as_mut().map(|gas| (gas, &mut *pyro, rt.emitter.spec().drag)),
                    gas_at: Vec::new(),
                };
                let frame = rt.emitter.at(n.local_time, &mut d).map_err(|e| e.to_string())?.clone();
                let mut key = crate::rng::hash(&[frame.time.to_bits(), frame.emitted, frame.dropped]);
                for p in &frame.particles {
                    key = crate::rng::hash(&[key, p.id, p.birth.to_bits(), p.lifetime.to_bits(), p.scale.to_bits()]);
                    if p.mass != 0. {
                        key = crate::rng::hash(&[key, p.mass.to_bits(), 1]);
                    }
                    for v in
                        p.position.iter().chain(&p.velocity).chain(p.basis.iter().flatten()).chain(&p.angular_velocity)
                    {
                        key = crate::rng::hash(&[key, v.to_bits(), 0]);
                    }
                }
                Ok(Arc::new(SimParticles3D { frame, key, size_curve: rt.size_curve, color_curve: rt.color_curve }))
            })();
            match result {
                Ok(f) => g.nodes[i].particles3d = Some(f),
                Err(e) => g.fail(format!("{id}: {e}")),
            }
        }
    }
}
