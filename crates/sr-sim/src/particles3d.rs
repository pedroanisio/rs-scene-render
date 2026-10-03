//! Deterministic world-space 3D particles. Births carry exact timestamps;
//! fractional samples replay from fixed checkpoints without changing the fixed
//! trajectory. Geometry drivers provide continuous sphere sweeps in scene axes.

pub use crate::particles::Burst;
use crate::rng;
use std::{
    cmp::{Ordering, Reverse},
    collections::{BTreeMap, BinaryHeap},
};
pub mod collider;
pub mod mesh;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid 3D particle input: {0}")]
    Invalid(&'static str),
    #[error("3D particle resource limit: {0}")]
    Limit(&'static str),
    #[error("3D particle driver: {0}")]
    Driver(String),
}
#[derive(Clone, Debug)]
pub enum Shape {
    Point,
    Box { size: [f64; 3] },
    Sphere { radius: f64 },
    Mesh(std::sync::Arc<mesh::Mesh>),
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub seed: u64,
    pub start: f64,
    pub end: Option<f64>,
    pub step: f64,
    pub lifetime: f64,
    pub lifetime_variance: f64,
    pub velocity: [f64; 3],
    /// Birth Euler angles in degrees, applied locally as Rz * Ry * Rx.
    pub rotation: [f64; 3],
    pub rotation_variance: [f64; 3],
    /// Constant spin vector about world axes, in degrees per second.
    pub angular_velocity: [f64; 3],
    pub angular_velocity_variance: [f64; 3],
    /// Uniform birth scale is sampled from [1-v,1+v); requires 0 <= v < 1.
    /// It scales both the render basis and the world-space collision radius.
    pub scale_variance: f64,
    pub direction: [f64; 3],
    pub speed: f64,
    pub speed_variance: f64,
    /// Full cone angle, in degrees, uniformly sampled by solid angle.
    pub spread: f64,
    pub gravity: [f64; 3],
    pub drag: f64,
    pub radius: f64,
    /// Maximum centre-path deviation from a swept chord, in scene units,
    /// for each locally constant acceleration/drag integration segment.
    pub collision_tolerance: f64,
    pub restitution: f64,
    pub friction: f64,
    pub shape: Shape,
    pub bursts: Vec<Burst>,
    pub max_particles: usize,
    pub max_events: usize,
    pub max_bytes: usize,
    pub checkpoint_bytes: usize,
    /// Maximum emission events, accepted births, motion segments and curvature
    /// refinements in one seek.
    pub max_work: u64,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            seed: 0,
            start: 0.,
            end: None,
            step: 1. / 60.,
            lifetime: 2.,
            lifetime_variance: 0.,
            velocity: [0.; 3],
            rotation: [0.; 3],
            rotation_variance: [0.; 3],
            angular_velocity: [0.; 3],
            angular_velocity_variance: [0.; 3],
            scale_variance: 0.,
            direction: [0., -1., 0.],
            speed: 0.,
            speed_variance: 0.,
            spread: 0.,
            gravity: [0.; 3],
            drag: 0.,
            radius: 0.5,
            collision_tolerance: 0.001,
            restitution: 0.5,
            friction: 0.,
            shape: Shape::Point,
            bursts: vec![],
            max_particles: 10_000,
            max_events: 16_384,
            max_bytes: 256 << 20,
            checkpoint_bytes: 64 << 20,
            max_work: 100_000_000,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Emission {
    /// Birth points and local velocities use this full affine. Existing
    /// particles keep their birth basis and are not attached to this transform.
    pub transform: sr_volume::Transform,
    /// Rate is held from the start of each fixed integration interval.
    pub rate: f64,
    pub enabled: bool,
    pub inherited_velocity: [f64; 3],
}
impl Default for Emission {
    fn default() -> Self {
        Self { transform: sr_volume::Transform::identity(), rate: 0., enabled: true, inherited_velocity: [0.; 3] }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    /// Fraction of the straight centre trajectory, in [0,1].
    pub fraction: f64,
    /// Corrected sphere centre at contact, including initial penetration recovery.
    pub position: [f64; 3],
    pub normal: [f64; 3],
    pub velocity: [f64; 3],
}
/// Input queries must be deterministic functions of their arguments. Mutable
/// state may cache queries, but results must not depend on prior query order.
/// Birth pose/enabled are queried at each exact emission event; acceleration
/// and colliders are queried at integration segment times.
pub trait Driver {
    fn emission(&mut self, time: f64) -> Result<Emission, Error>;
    fn acceleration(&mut self, time: f64, position: [f64; 3], velocity: [f64; 3]) -> Result<[f64; 3], Error>;
    fn sweep(&mut self, time: f64, dt: f64, from: [f64; 3], to: [f64; 3], radius: f64) -> Result<Option<Hit>, Error>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Particle {
    pub id: u64,
    pub birth: f64,
    pub lifetime: f64,
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    /// Columns of the birth affine's linear part, including local rotation and scale.
    pub basis: [[f64; 3]; 3],
    pub angular_velocity: [f64; 3],
    pub scale: f64,
}
impl Particle {
    pub fn age(&self, time: f64) -> f64 {
        time - self.birth
    }
    /// Instance transform at this frame's position. Spin is evaluated directly
    /// from age so seeks and fractional shutter samples do not accumulate drift.
    /// Supply the time of the frame containing this particle.
    pub fn transform(&self, time: f64) -> Result<sr_volume::Transform, Error> {
        use rapier3d_f64::parry::math::{Rotation, Vector};
        let spin = Vector::from_array(self.angular_velocity.map(f64::to_radians));
        let speed = length(spin.to_array());
        let age = self.age(time);
        let angle = speed * age;
        if !speed.is_finite() || !age.is_finite() || age < 0. || !angle.is_finite() {
            return Err(Error::Invalid("particle spin time or angular velocity"));
        }
        let rotation = if speed > 0. {
            Rotation::from_axis_angle(spin / speed, angle.rem_euclid(std::f64::consts::TAU))
        } else {
            Rotation::IDENTITY
        };
        let mut m = [0.; 16];
        for (i, b) in self.basis.iter().enumerate() {
            m[4 * i..4 * i + 3].copy_from_slice(&(rotation * Vector::from_array(*b)).to_array());
        }
        m[12..15].copy_from_slice(&self.position);
        m[15] = 1.;
        sr_volume::Transform::new(m).map_err(|_| Error::Invalid("particle instance transform"))
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub time: f64,
    pub particles: Vec<Particle>,
    pub emitted: u64,
    pub dropped: u64,
}
#[derive(Clone, Debug)]
struct State {
    frame: Frame,
    carry: f64,
    burst_cursor: Vec<u64>,
}
impl State {
    fn bytes(&self) -> usize {
        256 + self.frame.particles.len() * std::mem::size_of::<Particle>() + self.burst_cursor.len() * 8
    }
}
#[derive(Clone, Copy, Debug)]
struct Event {
    time: f64,
    count: u64,
}
#[derive(Clone, Copy, Debug)]
struct Time(f64);
impl PartialEq for Time {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for Time {}
impl PartialOrd for Time {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Time {
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.total_cmp(&o.0)
    }
}

pub struct Emitter {
    spec: Spec,
    state: State,
    step: u64,
    frame: Frame,
    checkpoints: BTreeMap<u64, State>,
    every: u64,
}
impl Emitter {
    pub fn new(spec: Spec) -> Result<Self, Error> {
        validate(&spec)?;
        let frame = Frame { time: spec.start, ..Default::default() };
        let state = State { frame: frame.clone(), carry: 0., burst_cursor: vec![0; spec.bursts.len()] };
        if state.bytes() > spec.checkpoint_bytes {
            return Err(Error::Limit("initial checkpoint bytes"));
        }
        let checkpoints = BTreeMap::from([(0, state.clone())]);
        let every = (1. / spec.step).round().max(1.) as u64;
        Ok(Self { spec, state, step: 0, frame, checkpoints, every })
    }
    pub fn spec(&self) -> &Spec {
        &self.spec
    }
    pub fn frame(&self) -> &Frame {
        &self.frame
    }
    pub fn checkpoint_bytes(&self) -> usize {
        self.checkpoints.values().map(State::bytes).sum()
    }
    /// Each successful fixed step is committed atomically. A failed request keeps
    /// the previously published frame; already completed checkpoints remain usable.
    pub fn at(&mut self, time: f64, driver: &mut dyn Driver) -> Result<&Frame, Error> {
        if !time.is_finite() {
            return Err(Error::Invalid("time must be finite"));
        }
        if time < self.spec.start {
            self.frame = Frame { time, ..Default::default() };
            return Ok(&self.frame);
        }
        let offset = (time - self.spec.start) / self.spec.step;
        if !offset.is_finite() || offset >= u64::MAX as f64 {
            return Err(Error::Limit("step index"));
        }
        let rounded = offset.round();
        let offset = if (offset - rounded).abs() <= (8. * f64::EPSILON * offset.abs().max(1.)).min(1e-6) {
            rounded
        } else {
            offset
        };
        let target = offset.floor() as u64;
        if self.step > target {
            let (&step, state) = self.checkpoints.range(..=target).next_back().expect("initial checkpoint");
            self.state = state.clone();
            self.step = step;
        }
        if target - self.step > 1_000_000 {
            return Err(Error::Limit("seek exceeds one million fixed steps"));
        }
        let mut work = self.spec.max_work;
        while self.step < target {
            let hi = self.spec.start + (self.step + 1) as f64 * self.spec.step;
            let next = advance(&self.spec, &self.state, hi, driver, &mut work)?;
            self.state = next;
            self.step += 1;
            if self.step % self.every == 0 && !self.checkpoints.contains_key(&self.step) {
                let bytes = self.state.bytes();
                while self.checkpoint_bytes().saturating_add(bytes) > self.spec.checkpoint_bytes
                    && self.checkpoints.len() > 1
                {
                    let old = *self.checkpoints.keys().find(|&&k| k != 0).expect("noninitial checkpoint");
                    self.checkpoints.remove(&old);
                }
                if self.checkpoint_bytes().saturating_add(bytes) <= self.spec.checkpoint_bytes {
                    self.checkpoints.insert(self.step, self.state.clone());
                }
            }
        }
        self.frame = advance(&self.spec, &self.state, time.max(self.state.frame.time), driver, &mut work)?.frame;
        self.frame.time = time;
        Ok(&self.frame)
    }
}

fn validate(s: &Spec) -> Result<(), Error> {
    if !s.start.is_finite()
        || s.start < 0.
        || !s.step.is_finite()
        || s.step < 1e-6
        || s.start + s.step <= s.start
        || s.end.is_some_and(|e| !e.is_finite() || e < s.start)
    {
        return Err(Error::Invalid("time range and positive fixed step"));
    }
    if !s.lifetime.is_finite()
        || s.lifetime <= 0.
        || !s.lifetime_variance.is_finite()
        || !(0. ..s.lifetime).contains(&s.lifetime_variance)
    {
        return Err(Error::Invalid("lifetime and variance"));
    }
    if !finite(s.velocity)
        || !finite(s.gravity)
        || !finite(s.direction)
        || length(s.direction) == 0.
        || !length(s.direction).is_finite()
    {
        return Err(Error::Invalid("finite vectors and nonzero direction"));
    }
    if !finite(s.rotation)
        || !finite(s.angular_velocity)
        || [s.rotation_variance, s.angular_velocity_variance].iter().any(|v| !finite(*v) || v.iter().any(|&x| x < 0.))
        || !s.scale_variance.is_finite()
        || !(0. ..1.).contains(&s.scale_variance)
    {
        return Err(Error::Invalid("particle rotation, spin or scale variance"));
    }
    for v in [s.speed, s.speed_variance, s.drag, s.radius, s.friction] {
        if !v.is_finite() || v < 0. {
            return Err(Error::Invalid("nonnegative speed, drag, radius and friction"));
        }
    }
    if s.speed_variance > s.speed
        || !s.collision_tolerance.is_finite()
        || s.collision_tolerance <= 0.
        || !s.spread.is_finite()
        || !(0. ..=360.).contains(&s.spread)
        || !s.restitution.is_finite()
        || !(0. ..=1.).contains(&s.restitution)
    {
        return Err(Error::Invalid("speed variance, collision tolerance, spread or restitution"));
    }
    match &s.shape {
        Shape::Box { size } if !finite(*size) || size.iter().any(|&v| v <= 0.) => {
            return Err(Error::Invalid("box dimensions"))
        }
        Shape::Sphere { radius } if !radius.is_finite() || *radius <= 0. => {
            return Err(Error::Invalid("sphere radius"))
        }
        _ => {}
    }
    if !(1..=1_000_000).contains(&s.max_particles)
        || !(1..=1_000_000).contains(&s.max_events)
        || s.bursts.len() > 4096
        || s.max_work == 0
    {
        return Err(Error::Limit("particle, event, burst or work count"));
    }
    let bytes = s
        .max_particles
        .checked_mul(4 * std::mem::size_of::<Particle>() + 32)
        .and_then(|n| n.checked_add(s.max_events * 48 + 4096 + s.bursts.len() * 32))
        .ok_or(Error::Limit("memory overflow"))?;
    let mesh_bytes = match &s.shape {
        Shape::Mesh(mesh) => mesh.bytes(),
        _ => 0,
    };
    if bytes.checked_add(mesh_bytes).is_none_or(|b| b > s.max_bytes) {
        return Err(Error::Limit("live and workspace memory budget"));
    }
    for b in &s.bursts {
        if !b.time.is_finite()
            || b.time < s.start
            || b.repeat == u64::MAX
            || !b.interval.is_finite()
            || b.interval < 0.
            || (b.repeat > 0 && b.interval <= 0.)
            || !(b.time + b.repeat as f64 * b.interval).is_finite()
        {
            return Err(Error::Invalid("burst time or repeat interval"));
        }
    }
    Ok(())
}

fn advance(s: &Spec, state: &State, hi: f64, d: &mut dyn Driver, work: &mut u64) -> Result<State, Error> {
    let lo = state.frame.time;
    let mut next = State {
        frame: Frame {
            time: hi,
            emitted: state.frame.emitted,
            dropped: state.frame.dropped,
            particles: Vec::with_capacity(s.max_particles),
        },
        carry: state.carry,
        burst_cursor: state.burst_cursor.clone(),
    };
    let mut events = Vec::new();
    let end = hi.min(s.end.unwrap_or(hi));
    if end > lo {
        let input = emission(d, lo)?;
        let rate = if input.enabled { input.rate } else { 0. };
        let total = state.carry + rate * (end - lo);
        // Fixed-step endpoints such as 59/60 and 1 subtract with finite
        // precision. Preserve an intended integral quota within that arithmetic
        // error, instead of delaying its last particle by an entire step.
        let error = (8. * f64::EPSILON * (1. + rate * (end.abs() + lo.abs()))).min(1e-8);
        let total = if rate > 0. && (total - total.round()).abs() <= error { total.round() } else { total };
        if !total.is_finite() || total.floor() > s.max_events as f64 {
            return Err(Error::Limit("rate events per step"));
        }
        let count = total.floor() as usize;
        next.carry = total - count as f64;
        for i in 0..count {
            events.push(Event { time: (lo + (i as f64 + 1. - state.carry) / rate).min(end), count: 1 });
        }
    }
    for (i, b) in s.bursts.iter().enumerate() {
        let cursor = &mut next.burst_cursor[i];
        while *cursor <= b.repeat {
            let time = b.time + *cursor as f64 * b.interval;
            if time > hi || s.end.is_some_and(|e| time >= e) {
                break;
            }
            if events.len() >= s.max_events {
                return Err(Error::Limit("burst events per step"));
            }
            *cursor += 1;
            events.push(Event { time, count: b.count });
        }
    }
    events.sort_by(|a, b| a.time.total_cmp(&b.time));
    let mut deaths = BinaryHeap::new();
    for particle in &state.frame.particles {
        let death = particle.birth + particle.lifetime;
        deaths.push(Reverse(Time(death)));
        if death > hi {
            let mut p = particle.clone();
            motion(s, &mut p, lo, hi - lo, d, work)?;
            next.frame.particles.push(p);
        }
    }
    for event in events {
        charge(work)?;
        let input = emission(d, event.time)?;
        if !input.enabled {
            continue;
        }
        while deaths.peek().is_some_and(|t| t.0 .0 <= event.time) {
            deaths.pop();
        }
        let room = (s.max_particles - deaths.len()).min(usize::try_from(event.count).unwrap_or(usize::MAX));
        let first = next.frame.emitted;
        next.frame.emitted = first.checked_add(event.count).ok_or(Error::Limit("particle ID overflow"))?;
        next.frame.dropped =
            next.frame.dropped.checked_add(event.count - room as u64).ok_or(Error::Limit("dropped count overflow"))?;
        for i in 0..room {
            charge(work)?;
            let mut p = spawn(s, &input, first + i as u64, event.time)?;
            let death = p.birth + p.lifetime;
            if !death.is_finite() || death <= p.birth {
                return Err(Error::Invalid("unrepresentable particle lifetime"));
            }
            deaths.push(Reverse(Time(death)));
            if death > hi {
                motion(s, &mut p, event.time, hi - event.time, d, work)?;
                next.frame.particles.push(p);
            }
        }
    }
    Ok(next)
}

fn emission(d: &mut dyn Driver, t: f64) -> Result<Emission, Error> {
    let v = d.emission(t)?;
    if !v.rate.is_finite() || v.rate < 0. || !finite(v.inherited_velocity) {
        return Err(Error::Invalid("emission rate or inherited velocity"));
    }
    Ok(v)
}
fn spawn(s: &Spec, e: &Emission, id: u64, birth: f64) -> Result<Particle, Error> {
    use rapier3d_f64::parry::math::{Rotation, Vector};
    let u = |k| rng::unit(s.seed, id, k);
    let point = match &s.shape {
        Shape::Point => [0.; 3],
        Shape::Box { size } => std::array::from_fn(|i| (u(11 + i as u64) - 0.5) * size[i]),
        Shape::Sphere { radius } => scale(unit_sphere(u(11), u(12)), radius * u(13).cbrt()),
        Shape::Mesh(mesh) => mesh.sample(s.seed, id),
    };
    let dir = normalize(s.direction);
    let axis = if dir[2].abs() < 0.9 { [0., 0., 1.] } else { [0., 1., 0.] };
    let x = normalize(cross(axis, dir));
    let y = cross(dir, x);
    let cos = 1. - u(1) * (1. - (s.spread.to_radians() * 0.5).cos());
    let sin = (1. - cos * cos).max(0.).sqrt();
    let phi = u(2) * std::f64::consts::TAU;
    let velocity = add(
        s.velocity,
        scale(
            add(scale(dir, cos), add(scale(x, sin * phi.cos()), scale(y, sin * phi.sin()))),
            s.speed + s.speed_variance * rng::signed(s.seed, id, 3),
        ),
    );
    let m = e.transform.columns();
    let velocity = add(
        std::array::from_fn(|i| m[i] * velocity[0] + m[4 + i] * velocity[1] + m[8 + i] * velocity[2]),
        e.inherited_velocity,
    );
    let rotation: [f64; 3] =
        std::array::from_fn(|i| s.rotation[i] + s.rotation_variance[i] * rng::signed(s.seed, id, 31 + i as u64));
    let angular_velocity = std::array::from_fn(|i| {
        s.angular_velocity[i] + s.angular_velocity_variance[i] * rng::signed(s.seed, id, 34 + i as u64)
    });
    if !finite(rotation) || !finite(angular_velocity) {
        return Err(Error::Invalid("nonfinite birth rotation or spin"));
    }
    let rotation = rotation.map(|r| r.rem_euclid(360.).to_radians());
    let rotation = Rotation::from_axis_angle(Vector::Z, rotation[2])
        * Rotation::from_axis_angle(Vector::Y, rotation[1])
        * Rotation::from_axis_angle(Vector::X, rotation[0]);
    let size = 1. + s.scale_variance * rng::signed(s.seed, id, 37);
    let basis = [Vector::X, Vector::Y, Vector::Z].map(|axis| {
        let v = (rotation * axis).to_array();
        std::array::from_fn(|i| (m[i] * v[0] + m[4 + i] * v[1] + m[8 + i] * v[2]) * size)
    });
    let p = Particle {
        id,
        birth,
        lifetime: s.lifetime + s.lifetime_variance * rng::signed(s.seed, id, 4),
        position: e.transform.index_to_world(point),
        velocity,
        basis,
        angular_velocity,
        scale: size,
    };
    if !finite(p.position)
        || !finite(p.velocity)
        || p.basis.iter().any(|b| !finite(*b))
        || !(s.radius * size).is_finite()
    {
        return Err(Error::Invalid("nonfinite particle birth"));
    }
    Ok(p)
}
fn motion(
    s: &Spec,
    p: &mut Particle,
    mut time: f64,
    mut dt: f64,
    d: &mut dyn Driver,
    work: &mut u64,
) -> Result<(), Error> {
    let mut contact: Option<([f64; 3], [f64; 3])> = None;
    let mut collisions = 0;
    while dt > 0. {
        charge(work)?;
        let a0 = add(s.gravity, d.acceleration(time, p.position, p.velocity)?);
        if !finite(a0) {
            return Err(Error::Invalid("nonfinite particle acceleration"));
        }
        let mut span = dt;
        let a = loop {
            let (mp, mv) = flight(p.position, p.velocity, a0, s.drag, span * 0.5);
            let mut a = add(s.gravity, d.acceleration(time + span * 0.5, mp, mv)?);
            if let Some((n, v)) = contact {
                let inward = dot(a, n) - s.drag * dot(v, n);
                if inward < 0. {
                    a = add(a, scale(n, -inward));
                }
            }
            // For fixed a and nonnegative drag k, p''(t) = (a-k*v0)*exp(-k*t).
            // Linear interpolation error is bounded by max|p''|*span²/8.
            // Subdivide before a chord query can miss the interior of an arc.
            let acceleration = length(add(a, scale(p.velocity, -s.drag)));
            if !acceleration.is_finite() || !finite(a) {
                return Err(Error::Invalid("nonfinite particle acceleration"));
            }
            let limit = s.collision_tolerance.sqrt() * 8f64.sqrt() / acceleration.sqrt();
            if span <= limit {
                break a;
            }
            charge(work)?;
            span = (span * 0.5).min(limit);
            if span <= 0. || time + span <= time {
                return Err(Error::Limit("unrepresentable collision subdivision"));
            }
        };
        let radius = s.radius * p.scale;
        let (to, velocity, hit) = loop {
            let (to, velocity) = flight(p.position, p.velocity, a, s.drag, span);
            if !finite(to) || !finite(velocity) {
                return Err(Error::Invalid("nonfinite particle motion"));
            }
            let hit = d.sweep(time, span, p.position, to, radius)?;
            if let Some(hit) = &hit {
                if !hit.fraction.is_finite()
                    || !(0. ..=1.).contains(&hit.fraction)
                    || !finite(hit.position)
                    || !finite(hit.normal)
                    || !finite(hit.velocity)
                    || !length(hit.normal).is_finite()
                    || length(hit.normal) == 0.
                {
                    return Err(Error::Invalid("collision result"));
                }
                let (_, actual_velocity) = flight(p.position, p.velocity, a, s.drag, span * hit.fraction);
                let relative = add(actual_velocity, scale(hit.velocity, -1.));
                // A chord may cut the surface while the true curved trajectory
                // is still leaving it. Applying that early hit repeatedly loses
                // time and exhausts the collision count. Refine until the hit
                // is approaching, or the shorter flight clears the surface.
                if hit.fraction > 0. && dot(relative, normalize(hit.normal)) > 1e-9 * length(relative).max(1.) {
                    charge(work)?;
                    span *= 0.5;
                    if time + span <= time {
                        return Err(Error::Limit("unrepresentable curved contact subdivision"));
                    }
                    continue;
                }
            }
            break (to, velocity, hit);
        };
        let Some(hit) = hit else {
            p.position = to;
            p.velocity = velocity;
            time += span;
            dt -= span;
            continue;
        };
        collisions += 1;
        if collisions > 16 {
            return Err(Error::Limit("more than 16 collisions in one particle step"));
        }
        let n = normalize(hit.normal);
        let elapsed = span * hit.fraction;
        let (_, v) = flight(p.position, p.velocity, a, s.drag, elapsed);
        let relative = add(v, scale(hit.velocity, -1.));
        let vn = dot(relative, n);
        let tangent = add(relative, scale(n, -vn));
        let speed = length(tangent);
        let friction =
            if speed > 0. { (1. - s.friction * (1. + s.restitution) * (-vn).max(0.) / speed).max(0.) } else { 0. };
        let mut rebound = if vn < 0. { -s.restitution * vn } else { vn };
        // Inelastic bounces under inward acceleration have a finite-time
        // accumulation point. Resolve sub-tolerance rebound height as contact
        // instead of spending the impact budget on an endless shrinking series.
        // Keep elastic and force-free impacts, including very slow ones, intact.
        let normal_acceleration = dot(add(a, scale(hit.velocity, -s.drag)), n);
        let settled = s.restitution > 0.
            && s.restitution < 1.
            && vn < 0.
            && normal_acceleration < 0.
            && rebound * rebound <= -2. * normal_acceleration * s.collision_tolerance;
        if settled {
            rebound = 0.;
        }
        p.velocity = add(hit.velocity, add(scale(n, rebound), scale(tangent, friction)));
        // Persistent contact on a changing normal needs finite contact slop.
        // A numerical epsilon alone causes infinitely many grazing impacts on
        // an accelerating/rotating boundary. Bound separation by the authored
        // spatial collision tolerance; isolated impacts retain the small bias.
        let separation = if settled || contact.is_some_and(|(previous, _)| dot(previous, n) > 0.95) {
            s.collision_tolerance
        } else {
            radius.max(1.) * 1e-8
        };
        p.position = add(hit.position, scale(n, separation));
        if !finite(p.position) || !finite(p.velocity) {
            return Err(Error::Invalid("nonfinite collision response"));
        }
        contact = (dot(add(p.velocity, scale(hit.velocity, -1.)), n).abs() < 1e-9).then_some((n, hit.velocity));
        time += elapsed;
        dt -= elapsed;
    }
    Ok(())
}
fn flight(p: [f64; 3], v: [f64; 3], a: [f64; 3], drag: f64, dt: f64) -> ([f64; 3], [f64; 3]) {
    let z = drag * dt;
    let (e, phi, psi) = if drag == 0. {
        (1., dt, 0.5 * dt * dt)
    } else {
        let phi = -(-z).exp_m1() / drag;
        let psi =
            if z.abs() < 1e-3 { dt * dt * (0.5 - z / 6. + z * z / 24. - z * z * z / 120.) } else { (dt - phi) / drag };
        ((-z).exp(), phi, psi)
    };
    (add(p, add(scale(v, phi), scale(a, psi))), add(scale(v, e), scale(a, phi)))
}
fn charge(work: &mut u64) -> Result<(), Error> {
    *work = work.checked_sub(1).ok_or(Error::Limit("seek work budget"))?;
    Ok(())
}
fn finite(v: [f64; 3]) -> bool {
    v.iter().all(|v| v.is_finite())
}
fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] + b[i])
}
fn scale(a: [f64; 3], b: f64) -> [f64; 3] {
    a.map(|x| x * b)
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
fn length(a: [f64; 3]) -> f64 {
    a[0].hypot(a[1]).hypot(a[2])
}
fn normalize(a: [f64; 3]) -> [f64; 3] {
    let n = length(a);
    a.map(|v| v / n)
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn unit_sphere(u: f64, v: f64) -> [f64; 3] {
    let z = 2. * u - 1.;
    let r = (1. - z * z).max(0.).sqrt();
    let p = v * std::f64::consts::TAU;
    [r * p.cos(), r * p.sin(), z]
}
