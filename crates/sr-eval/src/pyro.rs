//! Native smoke evaluation. Each volume object owns its solver and bounded
//! checkpoints; source animation is sampled on the object's source clock.

use crate::{
    program::Program,
    sim::{num, text, FieldSrc, Graphs},
    FrameGraph, FrameNode, Props,
};
use glam::{DMat4, DVec3};
use sr_model::element::{children, Element};
use sr_sim::pyro::{self, Boundary, Impulse, Inputs, Shape, Source, Spec, Timeline};
use std::{collections::HashMap, sync::Arc};

mod bake;
pub(crate) mod colliders;

#[derive(Debug, Clone)]
pub struct SimVolume {
    pub data: Arc<sr_volume::Volume>,
    /// Content hash used by compositor caches, including isolated groups.
    pub key: u64,
}

thread_local! {
    static EXPORTS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static STEPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Smoke steps simulated by native smoke evaluation on the calling thread so far, replays included.
#[doc(hidden)]
pub fn steps_on_this_thread() -> u64 {
    STEPS.with(std::cell::Cell::get)
}

/// Volume exports made by native smoke evaluation on the calling thread so far.
/// A frame whose simulation state did not change since the last export makes none.
#[doc(hidden)]
pub fn exports_on_this_thread() -> u64 {
    EXPORTS.with(std::cell::Cell::get)
}

struct Runtime {
    timeline: Timeline,
    max_bytes: usize,
    meshes: HashMap<String, Arc<pyro::mesh::Mesh>>,
    colliders: colliders::Colliders,
    dt: f64,
    last: Option<Arc<SimVolume>>,
    /// Timeline revision of the state `last` was exported from. Equal revisions
    /// are the same state, so nothing is exported or hashed again.
    last_revision: Option<u64>,
    /// Whether a source or impulse comes from a crater, whose impact the rigid world finds.
    craters: bool,
    /// The last states a reader of the gas has fetched (at most two, by step): a reader takes the timeline a step
    /// past the frame's own volume, and the frame must be able to export the step it is at without the timeline
    /// going back to a checkpoint and replaying.
    hold: Vec<Held>,
}

struct Held {
    step: u64,
    revision: u64,
    state: pyro::State,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum MeshRef {
    Asset(Arc<str>),
    Globe(Arc<str>),
}

#[derive(Default)]
pub(crate) struct Sims {
    runtimes: HashMap<Arc<str>, Result<Runtime, String>>,
}

fn config(node: &FrameNode) -> Option<&sr_model::model::Pyro> {
    let sr_model::model::Node::Object3D(object) = &*node.elem else { return None };
    object.children.iter().find_map(|child| match child {
        sr_model::model::Object3DChild::Pyro(pyro) => Some(pyro),
        _ => None,
    })
}

fn build(p: &Program, node: u32, e: &sr_model::model::Pyro) -> Result<Runtime, String> {
    let h = num(e, "voxelSize", 1.0);
    let size = [num(e, "width", 0.0), num(e, "height", 0.0), num(e, "depth", 0.0)];
    let bytes = (num(e, "maxMemoryMiB", 256.0) as usize).checked_mul(1 << 20).ok_or("pyro memory overflow")?;
    let checkpoint =
        (num(e, "checkpointMemoryMiB", 256.0) as usize).checked_mul(1 << 20).ok_or("pyro checkpoint overflow")?;
    let spec = Spec {
        cells: size.map(|v| (v / h).round() as usize),
        origin: size.map(|v| -0.5 * v),
        voxel_size: h,
        dt: num(e, "dt", 1.0 / 60.0),
        boundary: if text(e, "boundary").as_deref() == Some("open") { Boundary::Open } else { Boundary::Closed },
        ambient_temperature: num(e, "ambientTemperature", 300.0),
        dissipation: num(e, "dissipation", 0.0),
        cooling: num(e, "cooling", 0.0),
        buoyancy: num(e, "buoyancy", 0.0),
        vorticity: num(e, "vorticity", 0.0),
        turbulence: num(e, "turbulence", 0.0),
        seed: e.seed,
        pressure_iterations: num(e, "pressureIterations", 200.0) as usize,
        pressure_tolerance: num(e, "pressureTolerance", 1e-6),
        solver: match text(e, "solver").as_deref() {
            Some("multigrid") => pyro::PressureSolver::Multigrid,
            _ => pyro::PressureSolver::Jacobi,
        },
        advection: match text(e, "advection").as_deref() {
            Some("maccormack") => pyro::Advection::MacCormack,
            _ => pyro::Advection::SemiLagrangian,
        },
        max_bytes: bytes,
    };
    let timeline = Timeline::new(spec, checkpoint).map_err(|e| e.to_string())?;
    let mesh_budget = (num(e, "meshMemoryMiB", 128.0) as usize).checked_mul(1 << 20).ok_or("mesh memory overflow")?;
    let mut remaining = mesh_budget;
    let mut meshes = HashMap::new();
    let mut assets = HashMap::<MeshRef, Arc<pyro::mesh::Mesh>>::new();
    let namespace =
        p.nodes[node as usize].doc.checked_sub(1).and_then(|d| p.includes.get(d as usize)).map(|d| &*d.0).unwrap_or("");
    for source in children(e) {
        if text(source, "shape").as_deref() != Some("mesh") {
            continue;
        }
        let id = text(source, "mesh").ok_or("mesh source requires @mesh")?;
        if meshes.contains_key(&id) {
            continue;
        }
        let key = crate::sim::asset_key(&p.assets, namespace, &id);
        meshes.insert(id, load_mesh(p, &MeshRef::Asset(key), &mut remaining, &mut assets)?);
    }
    let colliders = colliders::build(p, e, &mut remaining, &mut assets)?;
    let craters = children(e).iter().any(|c| c.get_attr("crater").is_some());
    if craters {
        // the impact is on the composition clock, and so must the smoke be
        if !crate::sim::composition_clock(p, node, p.nodes[node as usize].start) {
            return Err("smoke from a crater needs the composition clock".into());
        }
    }
    Ok(Runtime {
        timeline,
        max_bytes: bytes,
        meshes,
        colliders,
        dt: num(e, "dt", 1.0 / 60.0),
        last: None,
        last_revision: None,
        craters,
        hold: Vec::new(),
    })
}

fn load_mesh(
    p: &Program,
    key: &MeshRef,
    remaining: &mut usize,
    assets: &mut HashMap<MeshRef, Arc<pyro::mesh::Mesh>>,
) -> Result<Arc<pyro::mesh::Mesh>, String> {
    if let Some(mesh) = assets.get(key) {
        return Ok(mesh.clone());
    }
    let (points, triangles) = match key {
        MeshRef::Globe(id) => {
            let node = p.nodes.iter().find(|n| n.id == *id).ok_or("globe collider is missing")?;
            crate::terrain::collider_triangles(p, node, *remaining)?
        }
        MeshRef::Asset(asset) => crate::sim3d::mesh_asset_triangles(p, asset, *remaining)?,
    };
    let mesh =
        Arc::new(pyro::mesh::Mesh::new(&points, &triangles, *remaining).map_err(|e| format!("mesh {key:?}: {e}"))?);
    *remaining = remaining.checked_sub(mesh.bytes()).ok_or("mesh region memory budget")?;
    assets.insert(key.clone(), mesh.clone());
    Ok(mesh)
}

pub(crate) fn value(e: &dyn Element, props: Option<&Props>, key: &str, default: f64) -> f64 {
    props.and_then(|p| p.get(key)).and_then(crate::Value::as_num).unwrap_or_else(|| num(e, key, default))
}

fn inputs(n: &FrameNode, meshes: &HashMap<String, Arc<pyro::mesh::Mesh>>) -> Result<Inputs, pyro::Error> {
    let e = config(n).ok_or(pyro::Error::Invalid("pyro configuration missing at source time"))?;
    let mut result = Inputs::default();
    let mut counts = HashMap::new();
    for child in children(e) {
        let name = child.element_name();
        let count = counts.entry(name).or_insert(0usize);
        let key = format!("{}/pyro[0]/{name}[{count}]", n.id);
        *count += 1;
        let props = n.parts.iter().find(|p| *p.key == key).map(|p| &p.props);
        let v = |name: &str, default: f64| value(child, props, name, default);
        if child.get_attr("crater").is_some() {
            // what a crater causes comes from its impact, not from the source's own attributes
            continue;
        }
        let shape = if text(child, "shape").as_deref() == Some("mesh") {
            let id = text(child, "mesh").ok_or(pyro::Error::Invalid("mesh source requires @mesh"))?;
            Shape::Mesh(meshes.get(&id).cloned().ok_or(pyro::Error::Invalid("mesh source asset was not loaded"))?)
        } else if text(child, "shape").as_deref() == Some("box") {
            let size = [v("width", 0.0), v("height", 0.0), v("depth", 0.0)];
            Shape::Box { min: size.map(|v| -0.5 * v), max: size.map(|v| 0.5 * v) }
        } else {
            Shape::Sphere { center: [0.0; 3], radius: v("radius", 1.0) }
        };
        let m = DMat4::from_translation(DVec3::new(v("x", 0.0), v("y", 0.0), v("z", 0.0)))
            * DMat4::from_rotation_z(v("rotation", 0.0).to_radians())
            * DMat4::from_rotation_y(v("rotationY", 0.0).to_radians())
            * DMat4::from_rotation_x(v("rotationX", 0.0).to_radians())
            * DMat4::from_scale(DVec3::new(v("scaleX", 1.0), v("scaleY", 1.0), v("scaleZ", 1.0)));
        let shape = Shape::Transformed {
            shape: Box::new(shape),
            transform: Box::new(sr_volume::Transform::new(m.to_cols_array())?),
        };
        match name {
            "pyroSource" => result.sources.push(Source {
                shape,
                start: v("start", 0.0),
                end: child.get_attr("end").map(|_| v("end", 0.0)),
                density_rate: v("densityRate", 0.0),
                temperature_rate: v("temperatureRate", 0.0),
                velocity_rate: [v("velocityRateX", 0.0), v("velocityRateY", 0.0), v("velocityRateZ", 0.0)],
                expansion: v("expansion", 0.0),
            }),
            "pyroImpulse" => result.impulses.push(Impulse {
                shape,
                time: v("time", 0.0),
                density: v("density", 0.0),
                temperature: v("temperature", 0.0),
                velocity: [v("velocityX", 0.0), v("velocityY", 0.0), v("velocityZ", 0.0)],
                expansion: v("expansion", 0.0),
            }),
            _ => return Err(pyro::Error::Invalid("unsupported pyro input element")),
        }
    }
    Ok(result)
}

/// The sources and impulses that craters cause, from the impacts the rigid world has found: for
/// each `pyroSource` or `pyroImpulse` with a crater whose impact has happened, a sphere of
/// the crater's radius at the impact point, with the dust and heat the engine makes of the
/// impact. `start` is the composition time at which the volume's clock begins. Nothing is
/// emitted for a crater that has not been hit.
fn crater_inputs(frame: &FrameGraph, node: usize, start: f64, dt: f64, input: &mut Inputs) -> Result<(), pyro::Error> {
    let Some(e) = config(&frame.nodes[node]) else { return Ok(()) };
    let domain = inverse_transform(crate::sim3d::world3(frame, node, 0))?;
    for child in children(e) {
        let Some(owner) = text(child, "crater") else { continue };
        let Some(j) = frame.nodes.iter().position(|n| *n.id == *owner) else { continue };
        let Some(grown) = frame.nodes[j].crater_impact.as_deref() else { continue };
        let cause = grown.cause;
        let number = |name: &str, default: f64| num(child, name, default);
        let params = sr_sim::cratering::SmokeParams {
            heat_fraction: number("heatFraction", 0.1),
            dust_fraction: number("dustFraction", 0.01),
            specific_heat: number("specificHeat", 1000.0),
            max_temperature: number("maxTemperature", 5000.0),
        };
        let made = sr_sim::cratering::smoke(&cause.impactor, cause.speed, &cause.law, cause.target_density, &params)
            .map_err(|_| pyro::Error::Invalid("the smoke of a crater has invalid parameters"))?;
        // the sphere of the crater's radius at the impact point, in the volume's own axes
        let owner_world = crate::sim3d::world3(frame, j, 0);
        let centre = domain.transform_point3(owner_world.transform_point3(DVec3::from(grown.spec.center)));
        let radius_world = cause.law.radius * cause.pixels_per_meter;
        let along = domain.transform_vector3(DVec3::new(radius_world, 0.0, 0.0)).length();
        let across = domain.transform_vector3(DVec3::new(0.0, radius_world, 0.0)).length();
        if !centre.is_finite()
            || (along - across).abs() > 1e-6 * along.max(across)
            || along.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
        {
            return Err(pyro::Error::Invalid("smoke from a crater needs a uniformly scaled volume"));
        }
        // a sphere smaller than a voxel would cover no cell centre: it is never smaller than one
        let h = num(e, "voxelSize", 1.0);
        let used = along.max(0.9 * h);
        let shape = Shape::Sphere { center: centre.to_array(), radius: used };
        // The dust is given as a total, in volume fractions summed over cells: the solver spreads it over the
        // cells whose centres the sphere covers and that are not solid, so that what is injected is the dust and
        // not what the grid, or a ground in the way, happens to leave.
        let voxel = (h * (cause.law.radius / along)).powi(3);
        let density = made.dust_volume / voxel;
        // the impact is known from the first step that starts after it, so that is where it begins
        let at = ((cause.time - start) / dt).ceil() * dt;
        let velocity = |names: [&str; 3]| names.map(|name| number(name, 0.0));
        match child.element_name() {
            "pyroSource" => input.heated.push(Source {
                shape,
                start: at,
                end: Some(at + made.duration),
                density_rate: density / made.duration,
                temperature_rate: made.temperature_rise / made.duration,
                velocity_rate: velocity(["velocityRateX", "velocityRateY", "velocityRateZ"]),
                expansion: 0.0,
            }),
            "pyroImpulse" => input.heated_impulses.push(Impulse {
                shape,
                time: at,
                density,
                temperature: made.temperature_rise,
                velocity: velocity(["velocityX", "velocityY", "velocityZ"]),
                expansion: 0.0,
            }),
            _ => {}
        }
    }
    Ok(())
}

/// The smoke of `id` at its source time `source_time`, simulated from the nearest checkpoint, and the revision
/// of that state. `at` is the composition time the request belongs to, for clocks that depend on it.
#[allow(clippy::too_many_arguments)]
fn state_at<'r>(
    timeline: &'r mut Timeline,
    meshes: &HashMap<String, Arc<pyro::mesh::Mesh>>,
    colliders: &mut colliders::Colliders,
    dt: f64,
    craters: bool,
    p: &Program,
    node: u32,
    id: &Arc<str>,
    at: f64,
    source_time: f64,
    graphs: &mut Graphs<'_>,
    fields: &FieldSrc,
    mut physics: Option<&mut crate::sim::PhysicsRt>,
) -> Result<(&'r pyro::State, u64), String> {
    let volume_start = p.nodes[node as usize].start;
    timeline
        .at_with_revision(source_time, &mut |_, time, state| {
            STEPS.with(|count| count.set(count.get() + 1));
            let (t, clocks) = crate::sim::source_sample(p, node, time + p.nodes[node as usize].start, at);
            let mut frame = if clocks.is_empty() {
                graphs.at(t)
            } else {
                Arc::new(crate::eval::evaluate_with_clocks(p, t, &clocks))
            };
            if !fields.is_empty() || !colliders.is_empty() || craters {
                if let Some(physics) = physics.as_deref_mut() {
                    // a rigid world that could not answer is an error, not a world with no impact yet
                    let before = frame.problems.len();
                    crate::sim::apply_physics(p, physics, Arc::make_mut(&mut frame), graphs, fields, t);
                    if frame.problems.len() > before {
                        return Err(pyro::Error::Driver(frame.problems[before..].join("; ")));
                    }
                }
            }
            // Conditions can omit the owner during part of its
            // history. Existing smoke still advances and cools,
            // while an absent owner injects no new source material.
            match frame.nodes.iter().position(|n| n.id == *id) {
                Some(i) => {
                    let mut input = inputs(&frame.nodes[i], meshes)?;
                    crater_inputs(&frame, i, volume_start, dt, &mut input)?;
                    field_inputs(&mut input, state, &frame, i, fields)?;
                    if !colliders.is_empty() {
                        let (next_t, next_clocks) =
                            crate::sim::source_sample(p, node, time + dt + p.nodes[node as usize].start, at);
                        let mut next = crate::eval::evaluate_pose_with_clocks(p, next_t, &next_clocks);
                        if let Some(physics) = physics.as_deref_mut() {
                            let before = next.problems.len();
                            crate::sim::apply_physics(p, physics, &mut next, graphs, fields, next_t);
                            if next.problems.len() > before {
                                return Err(pyro::Error::Driver(next.problems[before..].join("; ")));
                            }
                        }
                        colliders::apply(&mut input, colliders, &frame, &next, id, dt)?;
                    }
                    Ok(input)
                }
                None => Ok(Inputs::default()),
            }
        })
        .map_err(|e| e.to_string())
}

/// What a reader of a smoke's gas needs to know of it before asking: the step of its clock and what the
/// velocity fields it holds may use together.
pub(crate) struct GasClock {
    /// The smoke's fixed step, seconds.
    pub(crate) dt: f64,
    /// Bytes the smoke may use (`maxMemoryMiB`), which the fields held for a reader are charged to.
    pub(crate) max_bytes: usize,
}

/// The clock of the smoke volume `node` as a reader of its gas sees it.
pub(crate) fn gas_clock(node: &FrameNode) -> Result<GasClock, String> {
    let e = config(node).ok_or("a gas must be an object3D that holds a native pyro volume")?;
    let max_bytes = (num(e, "maxMemoryMiB", 256.0) as usize).checked_mul(1 << 20).ok_or("pyro memory overflow")?;
    let dt = num(e, "dt", 1.0 / 60.0);
    if !(dt.is_finite() && dt > 0.0) {
        return Err("the smoke has no positive step".into());
    }
    Ok(GasClock { dt, max_bytes })
}

/// The gas of one smoke step and the bytes the smoke keeps for readers of its gas.
pub(crate) struct GasStep {
    pub(crate) gas: Arc<pyro::Gas>,
    pub(crate) held: usize,
}

impl Sims {
    /// The gas of the smoke volume `frame.nodes[i]` at its fixed step `step`, simulating the smoke as far
    /// as that step needs. A step is a pure function of the document, so the copy can be kept as long as
    /// it is wanted.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn gas(
        &mut self,
        p: &Program,
        frame: &FrameGraph,
        i: usize,
        step: u64,
        graphs: &mut Graphs<'_>,
        fields: &FieldSrc,
        physics: Option<&mut crate::sim::PhysicsRt>,
    ) -> Result<GasStep, String> {
        let n = &frame.nodes[i];
        let e = config(n).ok_or("a gas must be an object3D that holds a native pyro volume")?;
        let id = n.id.clone();
        let node = p.nodes.iter().position(|n| n.id == id).map(|v| v as u32).ok_or("the smoke is not instantiated")?;
        let runtime = self.runtimes.entry(id.clone()).or_insert_with(|| build(p, node, e));
        let runtime = runtime.as_mut().map_err(|e| e.clone())?;
        let source_time = step as f64 * runtime.dt;
        let target = pyro::fixed_step_index(source_time.max(0.0) / runtime.dt);
        if let Some(held) = runtime.hold.iter().find(|h| h.step == target) {
            let bytes = runtime.hold.iter().map(|h| h.state.bytes()).sum();
            return Ok(GasStep { gas: Arc::new(held.state.gas()), held: bytes });
        }
        let (state, revision) = state_at(
            &mut runtime.timeline,
            &runtime.meshes,
            &mut runtime.colliders,
            runtime.dt,
            runtime.craters,
            p,
            node,
            &id,
            frame.time,
            source_time,
            graphs,
            fields,
            physics,
        )?;
        let gas = Arc::new(state.gas());
        // keep the state: the frame's own volume wants the step the reader started at, and the timeline is now
        // past it
        runtime.hold.push(Held { step: target, revision, state: state.clone() });
        while runtime.hold.len() > 2 {
            let oldest = (0..runtime.hold.len()).min_by_key(|&k| runtime.hold[k].step).expect("a held state");
            runtime.hold.remove(oldest);
        }
        let held = runtime.hold.iter().map(|h| h.state.bytes()).sum::<usize>();
        // what is kept for a reader is charged to the smoke's memory allowance, with the velocity fields the
        // reader holds, and exceeding it is an error
        if held > runtime.max_bytes {
            return Err(format!(
                "{} states kept for the particles that read its gas hold {held} bytes and exceed its maxMemoryMiB",
                runtime.hold.len()
            ));
        }
        Ok(GasStep { gas, held })
    }

    pub(crate) fn apply(
        &mut self,
        p: &Program,
        g: &mut FrameGraph,
        graphs: &mut Graphs<'_>,
        fields: &FieldSrc,
        mut physics: Option<&mut crate::sim::PhysicsRt>,
        problems: &mut Vec<String>,
    ) {
        for i in 0..g.nodes.len() {
            let n = &g.nodes[i];
            let Some(e) = config(n) else { continue };
            let id = n.id.clone();
            let Some(node) = p.nodes.iter().position(|n| n.id == id).map(|v| v as u32) else { continue };
            let source_time = n.local_time * value(&*n.elem, Some(&n.props), "animationSpeed", 1.0)
                + value(&*n.elem, Some(&n.props), "animationOffset", 0.0);
            if !crate::sim::linear_emitter_clock(p, node) {
                self.runtimes.remove(&id);
            }
            let runtime = self.runtimes.entry(id.clone()).or_insert_with(|| build(p, node, e));
            let result = (|| -> Result<Arc<SimVolume>, String> {
                let runtime = runtime.as_mut().map_err(|e| e.clone())?;
                let target = pyro::fixed_step_index(source_time.max(0.0) / runtime.dt);
                let (state, revision) = match runtime.hold.iter().find(|h| h.step == target) {
                    Some(held) => (&held.state, held.revision),
                    None => state_at(
                        &mut runtime.timeline,
                        &runtime.meshes,
                        &mut runtime.colliders,
                        runtime.dt,
                        runtime.craters,
                        p,
                        node,
                        &id,
                        g.time,
                        source_time,
                        graphs,
                        fields,
                        physics.as_deref_mut(),
                    )?,
                };
                if runtime.last_revision == Some(revision) {
                    if let Some(last) = &runtime.last {
                        return Ok(last.clone());
                    }
                }
                EXPORTS.with(|count| count.set(count.get() + 1));
                let volume = state.volume(runtime.max_bytes).map_err(|e| e.to_string())?;
                let key = pyro::volume_key(&volume);
                runtime.last_revision = Some(revision);
                if let Some(last) = runtime.last.as_ref().filter(|last| last.key == key) {
                    return Ok(last.clone());
                }
                let frame = Arc::new(SimVolume { data: Arc::new(volume), key });
                runtime.last = Some(frame.clone());
                Ok(frame)
            })();
            match result {
                Ok(volume) => g.nodes[i].sim_volume = Some(volume),
                Err(error) => problems.push(format!("{id}: {error}")),
            }
        }
    }
}

fn inverse_transform(world: DMat4) -> Result<DMat4, pyro::Error> {
    let determinant = world.determinant();
    if !world.is_finite() || !determinant.is_finite() || determinant == 0.0 {
        return Err(pyro::Error::Invalid("pyro domain transform must be finite and invertible"));
    }
    let inverse = world.inverse();
    if !inverse.is_finite() {
        return Err(pyro::Error::Invalid("pyro domain inverse transform must be finite"));
    }
    Ok(inverse)
}

fn field_inputs(
    input: &mut Inputs,
    state: &pyro::State,
    frame: &FrameGraph,
    node: usize,
    fields: &FieldSrc,
) -> Result<(), pyro::Error> {
    let e = config(&frame.nodes[node]).ok_or(pyro::Error::Invalid("pyro configuration missing at source time"))?;
    let fields = match crate::sim::field_names(e) {
        Some(ids) => fields.named(&ids, frame.time, Some(frame)),
        None => fields.at(frame.time, Some(frame)),
    };
    if !fields.iter().any(|field| field.particles) {
        return Ok(());
    }
    let world = crate::sim3d::world3(frame, node, 0);
    let inverse = inverse_transform(world)?;
    input.spatial_acceleration = state
        .cell_centres()
        .map(|p| {
            let velocity = world.transform_vector3(DVec3::from(state.velocity_at(p))).to_array();
            let position = world.transform_point3(DVec3::from(p)).to_array();
            let acceleration = sr_sim::fields::total3_for(&fields, position, velocity, frame.time, true);
            inverse.transform_vector3(DVec3::from(acceleration)).to_array()
        })
        .collect();
    Ok(())
}
