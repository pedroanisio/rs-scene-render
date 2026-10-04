//! Cinematic secondary water particles. The depth field is read-only: these
//! tracers do not exchange mass or momentum with the shallow-water solver.
use super::{Boundary, Error, Frame as Water, Spec};

#[derive(Clone, Debug)]
pub struct Settings {
    pub seed: u64,
    /// Expected particles per cell per second per unit of excess activity.
    pub rate: f64,
    /// Threshold of max(surface slope, horizontal Froude number).
    pub threshold: f64,
    pub start: f64,
    pub end: Option<f64>,
    pub lifetime: f64,
    pub spray_fraction: f64,
    /// Upward launch speed; scene y is downward.
    pub launch_speed: f64,
    pub drag: f64,
    pub radius: f64,
    pub max_particles: usize,
    pub max_bytes: usize,
    pub max_work: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            seed: 0,
            rate: 10.,
            threshold: 0.5,
            start: 0.,
            end: None,
            lifetime: 3.,
            spray_fraction: 0.4,
            launch_speed: 3.,
            drag: 0.1,
            radius: 0.05,
            max_particles: 10_000,
            max_bytes: 64 << 20,
            max_work: 100_000_000,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Foam,
    Spray,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Particle {
    pub id: u64,
    pub kind: Kind,
    pub birth: f64,
    pub lifetime: f64,
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    pub radius: f64,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub time: f64,
    pub particles: Vec<Particle>,
}
#[derive(Clone, Default)]
struct State {
    frame: Frame,
    next_id: u64,
}
/// Only the last canonical state is retained; backward seeks replay from zero.
/// Failed requests do not publish particles or alter the canonical state.
pub struct Whitewater {
    grid: Spec,
    bed: Vec<f64>,
    cfg: Settings,
    canonical: State,
    step: u64,
    frame: Frame,
}
impl Whitewater {
    pub fn new(grid: Spec, bed: Vec<f64>, cfg: Settings) -> Result<Self, Error> {
        let n = grid.cells[0].checked_mul(grid.cells[1]).ok_or(Error::Limit("whitewater grid"))?;
        if n == 0 || n > 4_000_000 || bed.len() != n || bed.iter().any(|v| !v.is_finite()) {
            return Err(Error::Invalid("whitewater bed/grid"));
        }
        if !grid.dt.is_finite()
            || grid.dt < 1e-6
            || !grid.cell_size.is_finite()
            || grid.cell_size <= 0.
            || !grid.gravity.is_finite()
            || grid.gravity <= 0.
            || !grid.dry_tolerance.is_finite()
            || grid.dry_tolerance <= 0.
            || grid.origin.iter().any(|v| !v.is_finite())
        {
            return Err(Error::Invalid("whitewater grid spacing, time or gravity"));
        }
        for a in 0..2 {
            let extent = grid.cells[a] as f64 * grid.cell_size;
            let end = grid.origin[a] + extent;
            if !extent.is_finite()
                || !(extent * 2.).is_finite()
                || !end.is_finite()
                || end <= grid.origin[a]
                || grid.origin[a] + grid.cell_size * 0.5 == grid.origin[a]
            {
                return Err(Error::Invalid("whitewater domain precision"));
            }
        }
        if [cfg.rate, cfg.threshold, cfg.start, cfg.launch_speed, cfg.drag].iter().any(|v| !v.is_finite() || *v < 0.)
            || !cfg.lifetime.is_finite()
            || cfg.lifetime <= 0.
            || !cfg.radius.is_finite()
            || cfg.radius <= 0.
            || !cfg.spray_fraction.is_finite()
            || !(0.0..=1.).contains(&cfg.spray_fraction)
            || cfg.end.is_some_and(|v| !v.is_finite() || v < cfg.start)
        {
            return Err(Error::Invalid("whitewater emission, lifetime, radius or drag"));
        }
        // Canonical + published + transactional + fractional particle copies,
        // water sample storage, owned bed and conservative vector overhead.
        let bytes = cfg
            .max_particles
            .checked_mul(512)
            .and_then(|v| v.checked_add(n.saturating_mul(48)))
            .and_then(|v| v.checked_add(bed.capacity().saturating_mul(8)))
            .and_then(|v| v.checked_add(4096))
            .ok_or(Error::Limit("whitewater memory"))?;
        if cfg.max_particles == 0 || cfg.max_particles > 1_000_000 || cfg.max_work == 0 || bytes > cfg.max_bytes {
            return Err(Error::Limit("whitewater memory, particle or work budget"));
        }
        Ok(Self { grid, bed, cfg, canonical: State::default(), step: 0, frame: Frame::default() })
    }
    pub fn frame(&self) -> &Frame {
        &self.frame
    }
    /// Source samples must return the requested time and exactly nx*nz cells.
    /// Births occur at canonical tick endpoints; fractional requests move
    /// existing tracers but never become a source of births or future state.
    pub fn at(&mut self, time: f64, mut source: impl FnMut(f64) -> Result<Water, Error>) -> Result<&Frame, Error> {
        if !time.is_finite() || time < 0. || time / self.grid.dt >= (1u64 << 53) as f64 {
            return Err(Error::Invalid("whitewater time"));
        }
        let mut target = (time / self.grid.dt).floor() as u64;
        while target > 0 && target as f64 * self.grid.dt > time {
            target -= 1;
        }
        if (target + 1) as f64 * self.grid.dt <= time {
            target += 1;
        }
        let (mut step, mut state) =
            if target >= self.step { (self.step, self.canonical.clone()) } else { (0, State::default()) };
        let mut work = self.cfg.max_work;
        while step < target {
            let end = (step + 1) as f64 * self.grid.dt;
            charge(&mut work, self.bed.len().saturating_mul(8) + state.frame.particles.len().saturating_mul(8))?;
            let water = source(end)?;
            self.validate_water(&water, end)?;
            self.move_particles(&mut state.frame, &water, end)?;
            self.emit(&mut state, &water, step + 1, &mut work)?;
            step += 1;
        }
        let mut frame = state.frame.clone();
        if time > frame.time {
            charge(&mut work, self.bed.len().saturating_mul(8) + frame.particles.len().saturating_mul(8))?;
            let water = source(time)?;
            self.validate_water(&water, time)?;
            self.move_particles(&mut frame, &water, time)?;
        }
        self.canonical = state;
        self.step = target;
        self.frame = frame;
        Ok(&self.frame)
    }
    fn validate_water(&self, water: &Water, time: f64) -> Result<(), Error> {
        if water.time != time
            || water.cells.len() != self.bed.len()
            || water.cells.iter().zip(&self.bed).any(|(c, b)| {
                !c.depth.is_finite()
                    || c.depth < 0.
                    || !(*b - c.depth).is_finite()
                    || !(self.grid.gravity * c.depth).is_finite()
                    || !c.velocity[0].hypot(c.velocity[1]).is_finite()
                    || c.velocity.iter().any(|v| !v.is_finite())
            })
        {
            return Err(Error::Invalid("whitewater source sample"));
        }
        Ok(())
    }
    // Bilinear field samples at cell centres, renormalized over wet corners.
    fn sample(&self, water: &Water, pos: [f64; 3]) -> Option<(f64, [f64; 2])> {
        let [nx, nz] = self.grid.cells;
        let q = [pos[0], pos[2]];
        let c: [f64; 2] = std::array::from_fn(|a| (q[a] - self.grid.origin[a]) / self.grid.cell_size - 0.5);
        let at = |i: isize, a: usize| -> usize {
            if self.grid.boundary == Boundary::Periodic {
                i.rem_euclid(self.grid.cells[a] as isize) as usize
            } else {
                i.clamp(0, self.grid.cells[a] as isize - 1) as usize
            }
        };
        let (x, z) = (c[0].floor() as isize, c[1].floor() as isize);
        let (tx, tz) = (c[0] - c[0].floor(), c[1] - c[1].floor());
        let mut total = 0.;
        let mut y = 0.;
        let mut vel = [0.; 2];
        for (ix, wx) in [(x, 1. - tx), (x + 1, tx)] {
            for (iz, wz) in [(z, 1. - tz), (z + 1, tz)] {
                let i = at(iz, 1).min(nz - 1) * nx + at(ix, 0);
                if water.cells[i].depth < self.grid.dry_tolerance {
                    continue;
                }
                let w = wx * wz;
                total += w;
                y += (self.bed[i] - water.cells[i].depth) * w;
                for (a, v) in vel.iter_mut().enumerate() {
                    *v += water.cells[i].velocity[a] * w;
                }
            }
        }
        (total > 0.).then(|| (y / total, vel.map(|v| v / total)))
    }
    fn boundary(&self, p: &mut Particle) -> Result<bool, Error> {
        if p.position.iter().chain(&p.velocity).any(|v| !v.is_finite()) {
            return Err(Error::Numerical("whitewater position/velocity overflow"));
        }
        for (a, xyz) in [0, 2].into_iter().enumerate() {
            let size = self.grid.cells[a] as f64 * self.grid.cell_size;
            let q = p.position[xyz] - self.grid.origin[a];
            if !q.is_finite() {
                return Err(Error::Numerical("whitewater extent overflow"));
            }
            if q >= 0. && q < size {
                continue;
            }
            match self.grid.boundary {
                Boundary::Open | Boundary::Absorbing => return Ok(false),
                Boundary::Periodic => p.position[xyz] = self.grid.origin[a] + q.rem_euclid(size),
                Boundary::Closed => {
                    let r = q.rem_euclid(2. * size);
                    p.position[xyz] = self.grid.origin[a] + if r < size { r } else { 2. * size - r };
                    if r >= size {
                        p.velocity[xyz] = -p.velocity[xyz];
                    }
                }
            }
        }
        Ok(true)
    }
    fn move_particles(&self, frame: &mut Frame, water: &Water, end: f64) -> Result<(), Error> {
        let dt = end - frame.time;
        let mut keep = 0;
        for i in 0..frame.particles.len() {
            let p = &mut frame.particles[i];
            if end - p.birth >= p.lifetime {
                continue;
            }
            match p.kind {
                Kind::Foam => {
                    let Some((_, v)) = self.sample(water, p.position) else { continue };
                    p.velocity = [v[0], 0., v[1]];
                    p.position[0] += v[0] * dt;
                    p.position[2] += v[1] * dt;
                }
                Kind::Spray => {
                    let x = self.cfg.drag * dt;
                    let decay = (-x).exp();
                    let (vtime, gtime) = if self.cfg.drag == 0. {
                        (dt, dt * dt * 0.5)
                    } else if x < 1e-4 {
                        (
                            dt * (1. - x / 2. + x * x / 6. - x * x * x / 24.),
                            dt * dt * (0.5 - x / 6. + x * x / 24. - x * x * x / 120.),
                        )
                    } else {
                        let v = -(-x).exp_m1() / self.cfg.drag;
                        (v, (dt - v) / self.cfg.drag)
                    };
                    for a in 0..3 {
                        let gravity = if a == 1 { self.grid.gravity } else { 0. };
                        p.position[a] += p.velocity[a] * vtime + gravity * gtime;
                        p.velocity[a] = p.velocity[a] * decay + gravity * vtime;
                    }
                }
            }
            if !self.boundary(p)? {
                continue;
            }
            let Some((y, v)) = self.sample(water, p.position) else { continue };
            let surface = y - p.radius * 0.2;
            if p.kind == Kind::Foam || p.position[1] >= surface {
                p.kind = Kind::Foam;
                p.position[1] = surface;
                p.velocity = [v[0], 0., v[1]];
            }
            if p.position.iter().chain(&p.velocity).any(|v| !v.is_finite()) {
                return Err(Error::Numerical("whitewater sample overflow"));
            }
            frame.particles.swap(keep, i);
            keep += 1;
        }
        frame.particles.truncate(keep);
        frame.time = end;
        Ok(())
    }
    fn emit(&self, state: &mut State, water: &Water, tick: u64, work: &mut u64) -> Result<(), Error> {
        let time = water.time;
        if time < self.cfg.start || self.cfg.end.is_some_and(|end| time >= end) || self.cfg.rate == 0. {
            return Ok(());
        }
        // One admitted allocation per transactional step, avoiding a realloc
        // and prefix copy for each emitting cell. Copies retain at most this cap.
        state.frame.particles.reserve_exact(self.cfg.max_particles - state.frame.particles.len());
        let [nx, nz] = self.grid.cells;
        for (i, cell) in water.cells.iter().enumerate() {
            if cell.depth < self.grid.dry_tolerance {
                continue;
            }
            let (x, z) = (i % nx, i / nx);
            let y = self.bed[i] - cell.depth;
            let at = |xx: isize, zz: isize| {
                let index = |p: isize, n: usize| {
                    if self.grid.boundary == Boundary::Periodic {
                        p.rem_euclid(n as isize) as usize
                    } else {
                        p.clamp(0, n as isize - 1) as usize
                    }
                };
                let j = index(zz, nz) * nx + index(xx, nx);
                if water.cells[j].depth < self.grid.dry_tolerance {
                    y
                } else {
                    self.bed[j] - water.cells[j].depth
                }
            };
            let sx = (at(x as isize + 1, z as isize) - at(x as isize - 1, z as isize)) / (2. * self.grid.cell_size);
            let sz = (at(x as isize, z as isize + 1) - at(x as isize, z as isize - 1)) / (2. * self.grid.cell_size);
            let froude = cell.velocity[0].hypot(cell.velocity[1]) / (self.grid.gravity * cell.depth).sqrt();
            let activity = sx.hypot(sz).max(froude) - self.cfg.threshold;
            let expected = activity.max(0.) * self.cfg.rate * self.grid.dt;
            if !expected.is_finite() || expected > self.cfg.max_particles as f64 {
                return Err(Error::Limit("whitewater particle emission"));
            }
            let seed = crate::rng::hash(self.cfg.seed, tick, i as u64);
            let count = expected.floor() as usize + usize::from(crate::rng::unit(seed, 0, 0) < expected.fract());
            charge(work, count.saturating_mul(8))?;
            if state.frame.particles.len().saturating_add(count) > self.cfg.max_particles {
                return Err(Error::Limit("whitewater particle count"));
            }
            for ordinal in 0..count {
                let random = |channel| crate::rng::unit(seed, ordinal as u64, channel);
                let pos = [
                    self.grid.origin[0] + (x as f64 + random(1)) * self.grid.cell_size,
                    y - self.cfg.radius * 0.2,
                    self.grid.origin[1] + (z as f64 + random(2)) * self.grid.cell_size,
                ];
                let Some((height, velocity)) = self.sample(water, pos) else { continue };
                let kind = if random(3) < self.cfg.spray_fraction { Kind::Spray } else { Kind::Foam };
                let p = Particle {
                    id: state.next_id,
                    kind,
                    birth: time,
                    lifetime: self.cfg.lifetime,
                    position: [pos[0], height - self.cfg.radius * 0.2, pos[2]],
                    velocity: [velocity[0], if kind == Kind::Spray { -self.cfg.launch_speed } else { 0. }, velocity[1]],
                    radius: self.cfg.radius,
                };
                if p.position.iter().chain(&p.velocity).any(|v| !v.is_finite()) {
                    return Err(Error::Numerical("whitewater birth overflow"));
                }
                state.next_id = state.next_id.checked_add(1).ok_or(Error::Limit("whitewater particle identity"))?;
                state.frame.particles.push(p);
            }
        }
        Ok(())
    }
}
fn charge(work: &mut u64, amount: usize) -> Result<(), Error> {
    *work = work.checked_sub(amount as u64).ok_or(Error::Limit("whitewater seek work"))?;
    Ok(())
}
