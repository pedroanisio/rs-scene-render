//! Flocks, fluids, slime networks and erosion: builds `sr-sim` simulations from their
//! elements, steps them on the node's own timeline (1/60 s steps, checkpointed) and attaches
//! the result to the frame: flocks as particles in frame space, the grid simulations as a
//! picture filling the node's box. Force fields named in `forceFields` (all of them when
//! absent) act on flocks and fluids, evaluated at each step's time.

use std::collections::HashMap;
use std::sync::Arc;

use sr_model::element::Element;
use sr_sim::timeline::Timeline;
use sr_sim::{erosion, flock, fluid, slime};

use crate::eval::{Affine, FrameGraph, FrameNode};
use crate::program::Program;
use crate::sim::{color_attr, index_of, num, text, FieldSrc, Graphs, ParticleFrame, ParticleShape};
use crate::value::Value;

/// Simulation step of flocks and the grid simulations.
pub const STEP: f64 = 1.0 / 60.0;

/// A grid simulation's picture: premultiplied linear-sRGB RGBA, row-major, filling the node's box.
#[derive(Debug, Clone)]
pub struct SimImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<[f32; 4]>,
    /// Changes whenever the picture does (for render caches).
    pub key: u64,
}

enum Kind {
    Flock {
        spec: flock::FlockSpec,
        tl: Timeline<flock::Agents>,
        look: FlockLook,
    },
    Fluid {
        spec: fluid::FluidSpec,
        tl: Timeline<fluid::Fluid>,
    },
    Slime {
        spec: slime::SlimeSpec,
        tl: Timeline<slime::Slime>,
        saturation: f64,
        low: [f32; 4],
        high: [f32; 4],
    },
    Erosion {
        spec: erosion::ErosionSpec,
        tl: Timeline<erosion::Terrain>,
        shade: [f64; 3],
        low: [f32; 4],
        high: [f32; 4],
    },
}

struct FlockLook {
    size: f64,
    color: Option<Value>,
    shape: ParticleShape,
    sprite: Option<Arc<str>>,
    trail: f64,
}

pub(crate) struct Sim {
    kind: Kind,
    fields: Option<Vec<String>>,
    /// Whether any force field can act (skips evaluating the node's transform per step).
    forced: bool,
    last: Option<(u64, Arc<SimImage>)>,
}

/// The grid and agent simulations of one evaluator.
#[derive(Default)]
pub(crate) struct Sims {
    sims: HashMap<Arc<str>, Sim>,
}

/// Whether node kind `k` is simulated here.
pub fn is_sim(k: &str) -> bool {
    matches!(k, "flock" | "fluid" | "slime" | "erosion")
}

fn srgb_to_linear(c: f64) -> f32 {
    (if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }) as f32
}

/// A literal colour attribute as premultiplied linear sRGB.
fn linear_color(e: &dyn Element, n: &FrameNode, attr: &str, d: [f32; 4], problems: &mut Vec<String>) -> [f32; 4] {
    match color_attr(e, &n.props, attr) {
        Some(Value::Color([r, g, b, a])) => {
            let a = a as f32;
            [srgb_to_linear(r) * a, srgb_to_linear(g) * a, srgb_to_linear(b) * a, a]
        }
        Some(_) => {
            problems.push(format!("{}: @{attr}: simulations take literal colours; the default applies", n.id));
            d
        }
        None => d,
    }
}

fn seed(e: &dyn Element, id: &str) -> u64 {
    match e.get_attr("seed") {
        Some(sr_model::element::AttrValue::Num(v)) => v as u64,
        _ => crate::rng::hash_str(id),
    }
}

fn build(p: &Program, n: &FrameNode, problems: &mut Vec<String>) -> Sim {
    let e: &dyn Element = &*n.elem;
    let size = [num(e, "width", 1.0), num(e, "height", 1.0)];
    let sd = seed(e, &n.id);
    let fields = text(e, "forceFields").map(|s| s.split_whitespace().map(str::to_string).collect());
    let kind = match n.kind {
        "flock" => {
            let spec = flock::FlockSpec {
                seed: sd,
                count: num(e, "count", 200.0).clamp(1.0, 100_000.0) as usize,
                size,
                speed: num(e, "speed", 120.0),
                max_speed: num(e, "maxSpeed", 240.0),
                max_force: num(e, "maxForce", 400.0),
                perception: num(e, "perception", 60.0),
                separation_distance: num(e, "separationDistance", 24.0),
                separation: num(e, "separation", 1.5),
                alignment: num(e, "alignment", 1.0),
                cohesion: num(e, "cohesion", 1.0),
                bounds: match text(e, "bounds").as_deref() {
                    Some("bounce") => flock::Bounds::Bounce,
                    Some("wrap") => flock::Bounds::Wrap,
                    _ => flock::Bounds::Steer,
                },
                margin: num(e, "edgeMargin", 40.0),
            };
            let sprite = text(e, "sprite").map(|id| {
                p.assets
                    .keys()
                    .find(|k| k.rsplit('/').next() == Some(id.as_str()))
                    .cloned()
                    .unwrap_or_else(|| id.into())
            });
            let shape = match text(e, "shape").as_deref() {
                Some("disc") => ParticleShape::Disc,
                Some("sprite") if sprite.is_some() => ParticleShape::Sprite,
                _ => ParticleShape::Streak,
            };
            let size_px = num(e, "size", 6.0);
            // a streak is drawn along the velocity: at least three body sizes long at cruise speed
            let trail = num(e, "trail", 0.0);
            let trail = if shape == ParticleShape::Streak {
                trail.max(3.0 * size_px / spec.speed.max(spec.max_speed * 0.5).max(1.0))
            } else {
                trail
            };
            let look = FlockLook { size: size_px, color: color_attr(e, &n.props, "color"), shape, sprite, trail };
            let init = flock::init(&spec);
            Kind::Flock { spec, tl: Timeline::new(0.0, STEP, init), look }
        }
        "fluid" => {
            let sources = sr_model::element::children(e)
                .iter()
                .filter(|c| c.element_name() == "fluidSource")
                .map(|c| fluid::Source {
                    pos: [num(*c, "x", 0.0), num(*c, "y", 0.0)],
                    radius: num(*c, "radius", 20.0),
                    color: linear_color(*c, n, "color", [1.0; 4], problems),
                    density: num(*c, "density", 1.0),
                    velocity: [num(*c, "velocityX", 0.0), num(*c, "velocityY", 0.0)],
                    start: num(*c, "start", 0.0),
                    end: crate::sim::opt(*c, "end"),
                })
                .collect();
            let spec = fluid::FluidSpec {
                size,
                resolution: num(e, "resolution", 128.0).clamp(8.0, 1024.0) as usize,
                viscosity: num(e, "viscosity", 0.0),
                diffusion: num(e, "diffusion", 0.0),
                dissipation: num(e, "dissipation", 0.0),
                velocity_dissipation: num(e, "velocityDissipation", 0.0),
                vorticity: num(e, "vorticity", 0.0),
                buoyancy: num(e, "buoyancy", 0.0),
                iterations: num(e, "iterations", 40.0).clamp(1.0, 500.0) as usize,
                edge: match text(e, "bounds").as_deref() {
                    Some("open") => fluid::Edge::Open,
                    Some("wrap") => fluid::Edge::Wrap,
                    _ => fluid::Edge::Closed,
                },
                sources,
            };
            let init = fluid::init(&spec);
            Kind::Fluid { spec, tl: Timeline::new(0.0, STEP, init) }
        }
        "slime" => {
            let spec = slime::SlimeSpec {
                seed: sd,
                size,
                resolution: num(e, "resolution", 256.0).clamp(8.0, 2048.0) as usize,
                agents: num(e, "agents", 20000.0).clamp(1.0, 2_000_000.0) as usize,
                spawn: match text(e, "spawn").as_deref() {
                    Some("random") => slime::Spawn::Random,
                    Some("ring") => slime::Spawn::Ring,
                    Some("center") => slime::Spawn::Center,
                    _ => slime::Spawn::Disc,
                },
                sensor_angle: num(e, "sensorAngle", 30.0),
                sensor_distance: num(e, "sensorDistance", 9.0),
                turn_angle: num(e, "turnAngle", 45.0),
                speed: num(e, "speed", 60.0),
                deposit: num(e, "deposit", 1.0),
                decay: num(e, "decay", 0.1),
                diffuse: num(e, "diffuse", 0.5),
            };
            let init = slime::init(&spec);
            Kind::Slime {
                tl: Timeline::new(0.0, STEP, init),
                saturation: num(e, "saturation", 4.0),
                low: linear_color(e, n, "colorLow", [0.0; 4], problems),
                high: linear_color(e, n, "color", [1.0, 0.64, 0.2, 1.0], problems),
                spec,
            }
        }
        _ => {
            let spec = erosion::ErosionSpec {
                seed: sd,
                size,
                resolution: num(e, "resolution", 256.0).clamp(16.0, 2048.0) as usize,
                octaves: num(e, "octaves", 6.0).clamp(1.0, 12.0) as u32,
                frequency: num(e, "frequency", 3.0),
                droplets: num(e, "droplets", 20000.0),
                inertia: num(e, "inertia", 0.05),
                capacity: num(e, "capacity", 4.0),
                erode_rate: num(e, "erodeRate", 0.3),
                deposit_rate: num(e, "depositRate", 0.3),
                evaporation: num(e, "evaporation", 0.01),
                gravity: num(e, "gravity", 4.0),
                radius: num(e, "erodeRadius", 3.0).clamp(1.0, 16.0) as usize,
                lifetime: 30,
            };
            let from = text(e, "heightmap").and_then(|id| {
                let key = p
                    .assets
                    .keys()
                    .find(|k| k.rsplit('/').next() == Some(id.as_str()))
                    .map(|k| k.to_string())
                    .unwrap_or(id);
                match crate::sim::image_path(p, &key)
                    .and_then(|path| image::open(&path).map_err(|e| format!("{}: {e}", path.display())))
                {
                    Ok(img) => {
                        let img = img.to_luma32f();
                        Some((img.width() as usize, img.height() as usize, img.into_raw()))
                    }
                    Err(err) => {
                        problems.push(format!("{}: heightmap: {err}; noise applies", n.id));
                        None
                    }
                }
            });
            let init = erosion::init(&spec, from);
            Kind::Erosion {
                tl: Timeline::new(0.0, STEP, init),
                shade: [num(e, "relief", 1.0), num(e, "sunAzimuth", 315.0), num(e, "sunElevation", 45.0)],
                low: linear_color(e, n, "colorLow", [0.043, 0.066, 0.028, 1.0], problems),
                high: linear_color(e, n, "colorHigh", [0.8, 0.76, 0.63, 1.0], problems),
                spec,
            }
        }
    };
    Sim { kind, forced: false, fields, last: None }
}

/// Acceleration (frame px/s²) of the force fields acting on the node at composition time `t`
/// at a box position, turned into the box's axes.
struct Forces<'a, 'b> {
    graphs: &'a mut Graphs<'b>,
    src: &'a FieldSrc,
    names: Option<&'a [String]>,
    id: &'a str,
    /// Composition time minus the node's local time.
    offset: f64,
}

impl Forces<'_, '_> {
    fn at(&mut self, local_t: f64) -> Option<(Affine, Vec<sr_sim::Field>)> {
        let t = local_t + self.offset;
        let g = self.graphs.at(t);
        let world = index_of(&g, self.id).map(|i| g.nodes[i].world)?;
        let gg = if self.src.animated { Some(&*g) } else { None };
        let f = match self.names {
            Some(ids) => self.src.named(ids, t, gg),
            None => self.src.at(t, gg),
        };
        Some((world, f))
    }
}

/// Acceleration from `fields` at box position `p` with box velocity `v`, in box axes.
fn accel(world: &Affine, fields: &[sr_sim::Field], p: [f64; 2], v: [f64; 2], t: f64) -> [f64; 2] {
    if fields.is_empty() {
        return [0.0, 0.0];
    }
    let [a, b, c, d, ..] = world.0;
    let fp = world.apply(p);
    let fv = [a * v[0] + c * v[1], b * v[0] + d * v[1]];
    let acc = sr_sim::fields::total(fields, fp, fv, t, true);
    // the inverse of the linear part turns frame axes into box axes
    let det = a * d - b * c;
    if det.abs() < 1e-12 {
        return [0.0, 0.0];
    }
    [(d * acc[0] - c * acc[1]) / det, (-b * acc[0] + a * acc[1]) / det]
}

impl Sims {
    /// Steps every simulation node of `g` to its local time and attaches the result.
    pub(crate) fn apply(
        &mut self,
        p: &Program,
        g: &mut FrameGraph,
        graphs: &mut Graphs<'_>,
        fields: &FieldSrc,
        problems: &mut Vec<String>,
    ) {
        let ids: Vec<usize> = g.nodes.iter().enumerate().filter(|(_, n)| is_sim(n.kind)).map(|(i, _)| i).collect();
        for i in ids {
            let id = g.nodes[i].id.clone();
            let sim = self.sims.entry(id.clone()).or_insert_with(|| {
                let mut s = build(p, &g.nodes[i], problems);
                s.forced = !fields.is_empty();
                s
            });
            let local = g.nodes[i].local_time.max(0.0);
            let offset = g.time - local;
            let world = g.nodes[i].world;
            let names = sim.fields.clone();
            let mut forces = Forces { graphs, src: fields, names: names.as_deref(), id: &id, offset };
            let forced = sim.forced;
            match &mut sim.kind {
                Kind::Flock { spec, tl, look } => {
                    let spec = &*spec;
                    let a = tl.at(local, &mut |a, _, t0| {
                        let got = if forced { forces.at(t0) } else { None };
                        let (w, f) = got.unwrap_or((Affine::IDENTITY, Vec::new()));
                        flock::step(spec, a, STEP, &mut |pos, vel| accel(&w, &f, pos, vel, t0 + offset));
                    });
                    g.nodes[i].particles = Some(Arc::new(flock_frame(a, &world, look)));
                }
                Kind::Fluid { spec, tl } => {
                    let spec = &*spec;
                    let step_index = ((local / STEP) + 1e-9).floor() as u64;
                    let f = tl.at(local, &mut |f, _, t0| {
                        let got = if forced { forces.at(t0) } else { None };
                        let (w, fl) = got.unwrap_or((Affine::IDENTITY, Vec::new()));
                        fluid::step(spec, f, t0, STEP, &mut |pos| accel(&w, &fl, pos, [0.0, 0.0], t0 + offset));
                    });
                    let (w, h) = (f.nx as u32, f.ny as u32);
                    g.nodes[i].sim_image = Some(cached(&mut sim.last, &id, step_index, || fluid::image(f), w, h));
                }
                Kind::Slime { spec, tl, saturation, low, high } => {
                    let spec = &*spec;
                    let step_index = ((local / STEP) + 1e-9).floor() as u64;
                    let s = tl.at(local, &mut |s, k, _| slime::step(spec, s, k, STEP));
                    let (w, h) = (s.nx as u32, s.ny as u32);
                    let (sat, lo, hi) = (*saturation, *low, *high);
                    g.nodes[i].sim_image =
                        Some(cached(&mut sim.last, &id, step_index, || slime::image(s, sat, lo, hi), w, h));
                }
                Kind::Erosion { spec, tl, shade, low, high } => {
                    let spec = &*spec;
                    let step_index = ((local / STEP) + 1e-9).floor() as u64;
                    let t = tl.at(local, &mut |t, _, _| erosion::step(spec, t, STEP));
                    let (w, h) = (t.nx as u32, t.ny as u32);
                    let (sh, lo, hi) = (*shade, *low, *high);
                    g.nodes[i].sim_image = Some(cached(
                        &mut sim.last,
                        &id,
                        step_index,
                        || erosion::image(t, sh[0], sh[1], sh[2], lo, hi),
                        w,
                        h,
                    ));
                }
            }
        }
    }
}

/// The picture of step `k`, reused while the step does not change.
fn cached(
    last: &mut Option<(u64, Arc<SimImage>)>,
    id: &str,
    k: u64,
    make: impl FnOnce() -> Vec<[f32; 4]>,
    w: u32,
    h: u32,
) -> Arc<SimImage> {
    if let Some((lk, img)) = last {
        if *lk == k {
            return img.clone();
        }
    }
    let img = Arc::new(SimImage {
        width: w,
        height: h,
        rgba: make(),
        key: crate::rng::hash_str(id) ^ k.wrapping_mul(0x9E37_79B9_7F4A_7C15),
    });
    *last = Some((k, img.clone()));
    img
}

fn flock_frame(a: &flock::Agents, world: &Affine, look: &FlockLook) -> ParticleFrame {
    let [m0, m1, m2, m3, ..] = world.0;
    let mut f = ParticleFrame {
        shape: look.shape,
        color0: look.color.clone(),
        sprite: look.sprite.clone(),
        cols: 1,
        rows: 1,
        trail: look.trail,
        orient: true,
        ..Default::default()
    };
    for k in 0..a.x.len() {
        let p = world.apply([a.x[k], a.y[k]]);
        let v = [m0 * a.vx[k] + m2 * a.vy[k], m1 * a.vx[k] + m3 * a.vy[k]];
        f.pos.push([p[0] as f32, p[1] as f32]);
        f.vel.push([v[0] as f32, v[1] as f32]);
        f.size.push(look.size as f32);
        f.rot.push(v[1].atan2(v[0]).to_degrees() as f32);
        f.color_t.push(0.0);
        f.alpha.push(1.0);
        f.frame.push(0);
    }
    f
}
