//! Ocean fields, local clocks and bounded native surface geometry.
use crate::{
    program::Program,
    sim::{num, text},
    FrameGraph, FrameNode,
};
use sr_model::element::children;
use sr_sim::ocean::{self as sim, Cell, Impulse, ImpulseKind, Spec};
use std::{collections::HashMap, sync::Arc};
mod bathymetry;
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
    let spec = Spec {
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
            };
            sim::whitewater::Whitewater::new(spec.clone(), bed.clone(), cfg).map_err(|e| e.to_string())
        })
        .transpose()?;
    Ok(Runtime { solver, spec, bed, waves, whitewater, surface_bytes: bytes("surfaceMemoryMiB", 128.)?, last: None })
}
impl Sims {
    pub(crate) fn apply(&mut self, p: &Program, g: &mut FrameGraph) {
        for n in &mut g.nodes {
            if n.kind != "ocean" {
                continue;
            }
            let rt = self.runtimes.entry(n.id.clone()).or_insert_with(|| build(p, n));
            let result = (|| -> Result<Arc<SimOcean>, String> {
                let rt = rt.as_mut().map_err(|e| e.clone())?;
                if let Some(last) = rt.last.as_ref().filter(|s| s.frame.time == n.local_time) {
                    return Ok(last.clone());
                }
                let foam = rt
                    .whitewater
                    .as_mut()
                    .map(|w| {
                        w.at(n.local_time, |time| sim::waves::apply(&rt.spec, rt.solver.at(time)?, &rt.waves))
                            .cloned()
                            .map_err(|e| e.to_string())
                    })
                    .transpose()?;
                let base = rt.solver.at(n.local_time).map_err(|e| e.to_string())?;
                let frame = sim::waves::apply(&rt.spec, base, &rt.waves).map_err(|e| e.to_string())?;
                let mesh = surface::mesh(&rt.spec, &rt.bed, &frame, rt.surface_bytes)?;
                let mut key = crate::rng::hash(&[frame.time.to_bits(), mesh.indices.len() as u64, 0x6f6365616e]);
                for cell in &frame.cells {
                    key = crate::rng::hash(&[
                        key,
                        cell.depth.to_bits(),
                        cell.velocity[0].to_bits(),
                        cell.velocity[1].to_bits(),
                    ]);
                }
                let whitewater_mesh = if let Some(foam) = &foam {
                    let available = rt.surface_bytes.saturating_sub(surface::memory_cost(&rt.spec)?);
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
                rt.last = Some(out.clone());
                Ok(out)
            })();
            match result {
                Ok(out) => n.sim_ocean = Some(out),
                Err(e) => {
                    // a failed solver is a problem and a failure (a disjoint borrow of `g.nodes` is held)
                    let message = format!("{}: {e}", n.id);
                    g.problems.push(message.clone());
                    g.failures.push(message);
                }
            }
        }
    }
}
