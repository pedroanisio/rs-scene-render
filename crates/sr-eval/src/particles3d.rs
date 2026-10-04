//! World-space particle simulation bound to scene clocks, poses and fields.

use crate::{
    program::Program,
    sim::{num, text, FieldSrc, Graphs, PhysicsRt},
    FrameGraph, FrameNode,
};
use sr_model::element::children;
use sr_sim::particles3d::{self as sim, Driver, Emission, Error, Hit};
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Clone)]
pub struct SimParticles3D {
    pub frame: sim::Frame,
    pub key: u64,
    pub size_curve: crate::curve::Ease,
    pub color_curve: crate::curve::Ease,
}

mod colliders;

struct Runtime {
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
        .filter(|c| c.element_name() == "burst")
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
    Ok(Runtime {
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
        if self.fields.is_empty() || self.names.is_some_and(|n| n.is_empty()) {
            return Ok([0.; 3]);
        }
        let frame = self.frame(time);
        let fields = match self.names {
            Some(n) => self.fields.named(n, frame.time, Some(&frame)),
            None => self.fields.at(frame.time, Some(&frame)),
        };
        Ok(sr_sim::fields::total3_for(&fields, position, velocity, frame.time, true))
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
                };
                let frame = rt.emitter.at(n.local_time, &mut d).map_err(|e| e.to_string())?.clone();
                let mut key = crate::rng::hash(&[frame.time.to_bits(), frame.emitted, frame.dropped]);
                for p in &frame.particles {
                    key = crate::rng::hash(&[key, p.id, p.birth.to_bits(), p.lifetime.to_bits(), p.scale.to_bits()]);
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
