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
mod colliders;

#[derive(Debug, Clone)]
pub struct SimVolume {
    pub data: Arc<sr_volume::Volume>,
    /// Content hash used by compositor caches, including isolated groups.
    pub key: u64,
}

struct Runtime {
    timeline: Timeline,
    max_bytes: usize,
    meshes: HashMap<String, Arc<pyro::mesh::Mesh>>,
    colliders: colliders::Colliders,
    dt: f64,
    last: Option<Arc<SimVolume>>,
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
        solver: pyro::PressureSolver::default(),
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
    Ok(Runtime { timeline, max_bytes: bytes, meshes, colliders, dt: num(e, "dt", 1.0 / 60.0), last: None })
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

fn value(e: &dyn Element, props: Option<&Props>, key: &str, default: f64) -> f64 {
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

fn key(volume: &sr_volume::Volume) -> u64 {
    let mut h = 0;
    for (name, grid) in volume.grids() {
        h = crate::rng::hash(&[h, crate::rng::hash_str(name), u64::from(grid.background().to_bits())]);
        for v in grid.transform().columns() {
            h = crate::rng::hash(&[h, v.to_bits(), 0]);
        }
        for (coord, values) in grid.bricks() {
            for v in coord {
                h = crate::rng::hash(&[h, v as u64, 1]);
            }
            for v in values {
                h = crate::rng::hash(&[h, u64::from(v.to_bits()), 2]);
            }
        }
    }
    h
}

impl Sims {
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
                let meshes = &runtime.meshes;
                let colliders = &mut runtime.colliders;
                let dt = runtime.dt;
                let state = runtime
                    .timeline
                    .at_with_state(source_time, &mut |_, time, state| {
                        let (t, clocks) =
                            crate::sim::source_sample(p, node, time + p.nodes[node as usize].start, g.time);
                        let mut frame = if clocks.is_empty() {
                            graphs.at(t)
                        } else {
                            Arc::new(crate::eval::evaluate_with_clocks(p, t, &clocks))
                        };
                        if !fields.is_empty() || !colliders.is_empty() {
                            if let Some(physics) = physics.as_deref_mut() {
                                crate::sim::apply_physics(p, physics, Arc::make_mut(&mut frame), graphs, fields, t);
                            }
                        }
                        // Conditions can omit the owner during part of its
                        // history. Existing smoke still advances and cools,
                        // while an absent owner injects no new source material.
                        match frame.nodes.iter().position(|n| n.id == id) {
                            Some(i) => {
                                let mut input = inputs(&frame.nodes[i], meshes)?;
                                field_inputs(&mut input, state, &frame, i, fields)?;
                                if !colliders.is_empty() {
                                    let (next_t, next_clocks) = crate::sim::source_sample(
                                        p,
                                        node,
                                        time + dt + p.nodes[node as usize].start,
                                        g.time,
                                    );
                                    let mut next = crate::eval::evaluate_pose_with_clocks(p, next_t, &next_clocks);
                                    if let Some(physics) = physics.as_deref_mut() {
                                        crate::sim::apply_physics(p, physics, &mut next, graphs, fields, next_t);
                                    }
                                    colliders::apply(&mut input, colliders, &frame, &next, &id, dt)?;
                                }
                                Ok(input)
                            }
                            None => Ok(Inputs::default()),
                        }
                    })
                    .map_err(|e| e.to_string())?;
                let volume = state.volume(runtime.max_bytes).map_err(|e| e.to_string())?;
                let key = key(&volume);
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
