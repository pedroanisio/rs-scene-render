//! Particle emitters: deterministic emission (rate and bursts), motion under
//! gravity, drag, turbulence and force fields, and collisions, stored as
//! structure-of-arrays and checkpointed every simulated second.

use std::collections::BTreeMap;

use crate::fields::{self, Field};
use crate::rng;

/// Where particles are born, in the emitter's local pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum EmitShape {
    Point,
    /// Centred on the origin.
    Rect {
        w: f64,
        h: f64,
    },
    Ellipse {
        w: f64,
        h: f64,
    },
    /// Along x, centred on the origin.
    Line {
        w: f64,
    },
    /// Samples of a path or an image's alpha.
    Points(Vec<[f64; 2]>),
}

/// A burst of `count` particles at `time`, repeated `repeat` more times every `interval` seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Burst {
    pub time: f64,
    pub count: u64,
    pub repeat: u64,
    pub interval: f64,
}

/// Boundaries particles bounce off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Walls {
    None,
    Floor { h: f64 },
    Frame { w: f64, h: f64 },
}

/// Emitter parameters (document pixels, seconds, degrees).
#[derive(Clone, Debug)]
pub struct EmitterSpec {
    pub seed: u64,
    pub start: f64,
    pub end: Option<f64>,
    pub preroll: f64,
    pub step: f64,
    pub lifetime: f64,
    pub lifetime_variance: f64,
    pub speed: f64,
    pub speed_variance: f64,
    /// Degrees, 0 = +x, −90 = up.
    pub direction: f64,
    pub spread: f64,
    pub gravity: [f64; 2],
    pub drag: f64,
    pub turbulence: f64,
    pub turbulence_scale: f64,
    pub rotation0: f64,
    pub rotation_variance: f64,
    pub angular_velocity: f64,
    pub angular_velocity_variance: f64,
    pub size_variance: f64,
    pub max_particles: usize,
    pub shape: EmitShape,
    pub bursts: Vec<Burst>,
    pub collide: bool,
    pub bounce: f64,
    pub walls: Walls,
}

/// Animated inputs, asked for at simulation-step times.
pub trait EmitterDriver {
    /// Emitter local → world affine `[a, b, c, d, e, f]` (x' = a x + c y + e).
    fn origin(&mut self, t: f64) -> [f64; 6];
    /// Particles per second.
    fn rate(&mut self, t: f64) -> f64;
    fn fields(&mut self, t: f64) -> Vec<Field>;
    /// Surface point and outward normal when `p` lies inside a physics body.
    fn hit(&mut self, p: [f64; 2]) -> Option<([f64; 2], [f64; 2])>;
}

/// Live particles, structure-of-arrays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Store {
    pub id: Vec<u64>,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub vx: Vec<f64>,
    pub vy: Vec<f64>,
    pub age: Vec<f64>,
    pub life: Vec<f64>,
    pub rot: Vec<f64>,
    pub spin: Vec<f64>,
    /// Size multiplier (1 ± sizeVariance).
    pub scale: Vec<f64>,
}

impl Store {
    pub fn len(&self) -> usize {
        self.id.len()
    }
    pub fn is_empty(&self) -> bool {
        self.id.is_empty()
    }
}

#[derive(Clone)]
struct State {
    step: u64,
    emitted: u64,
    carry: f64,
    p: Store,
}

/// A deterministic emitter.
pub struct Emitter {
    spec: EmitterSpec,
    state: State,
    checkpoints: BTreeMap<u64, State>,
    per_checkpoint: u64,
}

fn shape_point(shape: &EmitShape, seed: u64, k: u64) -> [f64; 2] {
    let (u, w) = (rng::unit(seed, k, 11), rng::unit(seed, k, 12));
    match shape {
        EmitShape::Point => [0.0, 0.0],
        EmitShape::Rect { w: sw, h: sh } => [(u - 0.5) * sw, (w - 0.5) * sh],
        EmitShape::Ellipse { w: sw, h: sh } => {
            let (r, a) = (u.sqrt(), w * std::f64::consts::TAU);
            [r * a.cos() * sw * 0.5, r * a.sin() * sh * 0.5]
        }
        EmitShape::Line { w: sw } => [(u - 0.5) * sw, 0.0],
        EmitShape::Points(ps) if !ps.is_empty() => ps[((u * ps.len() as f64) as usize).min(ps.len() - 1)],
        EmitShape::Points(_) => [0.0, 0.0],
    }
}

impl Emitter {
    pub fn new(spec: EmitterSpec) -> Emitter {
        let per = ((1.0 / spec.step.max(1e-6)).round() as u64).max(1);
        let state = State { step: 0, emitted: 0, carry: 0.0, p: Store::default() };
        let mut checkpoints = BTreeMap::new();
        checkpoints.insert(0, state.clone());
        Emitter { spec, state, checkpoints, per_checkpoint: per }
    }

    /// Simulation clock origin: the start minus the preroll.
    fn t0(&self) -> f64 {
        self.spec.start - self.spec.preroll
    }

    fn emitting(&self, t: f64) -> bool {
        t >= self.t0() && self.spec.end.map(|e| t < e).unwrap_or(true)
    }

    fn spawn(&mut self, k: u64, birth: f64, advance: f64, drv: &mut dyn EmitterDriver) {
        let s = &self.spec;
        let seed = s.seed;
        let o = drv.origin(birth);
        let lp = shape_point(&s.shape, seed, k);
        let (x, y) = (o[0] * lp[0] + o[2] * lp[1] + o[4], o[1] * lp[0] + o[3] * lp[1] + o[5]);
        let turn = o[1].atan2(o[0]);
        let dir = (s.direction + s.spread * (rng::unit(seed, k, 1) - 0.5)).to_radians() + turn;
        let speed = s.speed + s.speed_variance * rng::signed(seed, k, 2);
        let (vx, vy) = (dir.cos() * speed, dir.sin() * speed);
        let life = (s.lifetime + s.lifetime_variance * rng::signed(seed, k, 3)).max(1e-3);
        let p = &mut self.state.p;
        p.id.push(k);
        p.x.push(x + vx * advance);
        p.y.push(y + vy * advance);
        p.vx.push(vx);
        p.vy.push(vy);
        p.age.push(advance);
        p.life.push(life);
        p.rot.push(s.rotation0 + s.rotation_variance * rng::signed(seed, k, 4));
        p.spin.push(s.angular_velocity + s.angular_velocity_variance * rng::signed(seed, k, 5));
        p.scale.push((1.0 + s.size_variance * rng::signed(seed, k, 6)).max(0.0));
    }

    fn step_once(&mut self, drv: &mut dyn EmitterDriver) {
        let dt = self.spec.step;
        let t = self.t0() + self.state.step as f64 * dt;
        // emission over [t, t + dt)
        if self.emitting(t) {
            let rate = drv.rate(t).max(0.0);
            let exact = self.state.carry + rate * dt;
            let n = exact.floor() as u64;
            self.state.carry = exact - n as f64;
            for i in 0..n {
                let frac = (i as f64 + 0.5) / n as f64;
                if self.state.p.len() < self.spec.max_particles {
                    let k = self.state.emitted;
                    self.spawn(k, t + frac * dt, (1.0 - frac) * dt, drv);
                }
                self.state.emitted += 1;
            }
        }
        let bursts = self.spec.bursts.clone();
        for b in &bursts {
            for r in 0..=b.repeat {
                let bt = b.time + r as f64 * b.interval.max(1e-6);
                if bt >= t && bt < t + dt {
                    for _ in 0..b.count {
                        if self.state.p.len() < self.spec.max_particles {
                            let k = self.state.emitted;
                            self.spawn(k, bt, t + dt - bt, drv);
                        }
                        self.state.emitted += 1;
                    }
                }
            }
        }
        // motion
        let s = &self.spec;
        let flds = drv.fields(t);
        let damp = (1.0 - s.drag * dt).max(0.0);
        let p = &mut self.state.p;
        for i in 0..p.len() {
            let mut ax = s.gravity[0];
            let mut ay = s.gravity[1];
            if s.turbulence > 0.0 {
                let c = rng::curl2(
                    s.seed ^ 0x5A5A,
                    p.x[i] / s.turbulence_scale.max(1e-6),
                    p.y[i] / s.turbulence_scale.max(1e-6) + t * 0.2,
                );
                ax += c[0] * s.turbulence * 2.0;
                ay += c[1] * s.turbulence * 2.0;
            }
            if !flds.is_empty() {
                let a = fields::total(&flds, [p.x[i], p.y[i]], [p.vx[i], p.vy[i]], t, true);
                ax += a[0];
                ay += a[1];
            }
            p.vx[i] = (p.vx[i] + ax * dt) * damp;
            p.vy[i] = (p.vy[i] + ay * dt) * damp;
            p.x[i] += p.vx[i] * dt;
            p.y[i] += p.vy[i] * dt;
            p.rot[i] += p.spin[i] * dt;
            p.age[i] += dt;
        }
        // collisions
        if s.collide || s.walls != Walls::None {
            let bounce = s.bounce;
            for i in 0..p.len() {
                let mut hit: Option<([f64; 2], [f64; 2])> = None;
                match s.walls {
                    Walls::Floor { h } | Walls::Frame { h, .. } if p.y[i] > h => hit = Some(([p.x[i], h], [0.0, -1.0])),
                    _ => {}
                }
                if let Walls::Frame { w, .. } = s.walls {
                    if p.x[i] < 0.0 {
                        hit = Some(([0.0, p.y[i]], [1.0, 0.0]));
                    } else if p.x[i] > w {
                        hit = Some(([w, p.y[i]], [-1.0, 0.0]));
                    } else if p.y[i] < 0.0 {
                        hit = Some(([p.x[i], 0.0], [0.0, 1.0]));
                    }
                }
                if hit.is_none() && s.collide {
                    hit = drv.hit([p.x[i], p.y[i]]);
                }
                if let Some((q, n)) = hit {
                    p.x[i] = q[0];
                    p.y[i] = q[1];
                    let vn = p.vx[i] * n[0] + p.vy[i] * n[1];
                    if vn < 0.0 {
                        p.vx[i] -= (1.0 + bounce) * vn * n[0];
                        p.vy[i] -= (1.0 + bounce) * vn * n[1];
                        p.vx[i] *= 0.9;
                        p.vy[i] *= 0.9;
                    }
                }
            }
        }
        // retire expired particles, keeping birth order
        if p.age.iter().zip(&p.life).any(|(a, l)| a >= l) {
            let keep: Vec<bool> = p.age.iter().zip(&p.life).map(|(a, l)| a < l).collect();
            macro_rules! compact {
                ($($f:ident),*) => {$(
                    let mut k = 0;
                    p.$f.retain(|_| { let r = keep[k]; k += 1; r });
                )*};
            }
            compact!(id, x, y, vx, vy, age, life, rot, spin, scale);
        }
        self.state.step += 1;
        if self.state.step % self.per_checkpoint == 0 && !self.checkpoints.contains_key(&self.state.step) {
            self.checkpoints.insert(self.state.step, self.state.clone());
        }
    }

    /// Particles alive at `t`.
    pub fn at(&mut self, t: f64, drv: &mut dyn EmitterDriver) -> &Store {
        let target = if t <= self.t0() { 0 } else { ((t - self.t0()) / self.spec.step + 1e-9).floor() as u64 };
        if self.state.step > target || target - self.state.step > self.per_checkpoint {
            if let Some((_, cp)) = self.checkpoints.range(..=target).next_back() {
                if cp.step > self.state.step || self.state.step > target {
                    self.state = cp.clone();
                }
            }
        }
        while self.state.step < target {
            self.step_once(drv);
        }
        &self.state.p
    }

    /// Particles of the last requested time.
    pub fn store(&self) -> &Store {
        &self.state.p
    }

    /// Particles emitted so far (including dropped ones over `maxParticles`).
    pub fn emitted(&self) -> u64 {
        self.state.emitted
    }
}
