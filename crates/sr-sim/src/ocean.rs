//! Bounded cinematic shallow water in the scene's x/z plane, with y downward.
//!
//! First-order finite volumes use local Lax–Friedrichs fluxes and hydrostatic
//! reconstruction (Audusse et al., SIAM J. Sci. Comput. 25, 2004,
//! <https://doi.org/10.1137/S1064827503431090>). A conservative two-dimensional
//! CFL bound controls internal substeps. This depth-averaged model cannot
//! represent overturning sheets, compressible impact physics or vertical jets.

mod flux;
mod impulse;
pub mod waves;
pub mod whitewater;
pub use impulse::{Impulse, ImpulseKind};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid ocean input: {0}")]
    Invalid(&'static str),
    #[error("ocean resource limit: {0}")]
    Limit(&'static str),
    #[error("ocean numerical failure: {0}")]
    Numerical(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boundary {
    /// Reflect normal momentum at each domain edge.
    Closed,
    /// Zero-gradient extrapolation; not an absorbing-wave boundary.
    Open,
    /// Wrap both horizontal axes, including bathymetry.
    Periodic,
    /// Zero-gradient edge faces plus a relaxation layer of `Spec::sponge_cells`
    /// cells that drives the state toward its initial rest state. It does not
    /// conserve water.
    Absorbing,
}

/// Spatial/temporal accuracy of the finite-volume update.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    /// Piecewise-constant states, forward Euler.
    #[default]
    First,
    /// MUSCL reconstruction with the minmod limiter and SSP-RK2.
    Second,
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub cells: [usize; 2],
    /// Lower x/z corner. Samples are at cell centres; x varies fastest.
    pub origin: [f64; 2],
    pub cell_size: f64,
    /// Canonical replay interval; CFL and events may subdivide it.
    pub dt: f64,
    /// Positive acceleration magnitude in scene units/second².
    pub gravity: f64,
    /// Exponential horizontal velocity decay per second.
    pub damping: f64,
    pub boundary: Boundary,
    pub order: Order,
    /// Sponge layer width in cells; read only for `Boundary::Absorbing`.
    pub sponge_cells: usize,
    /// Momentum is zeroed below this depth; water is never discarded.
    pub dry_tolerance: f64,
    /// Conservative resident state/workspace ceiling, excluding checkpoints.
    pub max_bytes: usize,
    pub checkpoint_bytes: usize,
    /// Work units per seek: eight per cell for each impulse and each substep.
    pub max_work: u64,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            cells: [64; 2],
            origin: [0.0; 2],
            cell_size: 1.0,
            dt: 1.0 / 60.0,
            gravity: 9.81,
            damping: 0.0,
            boundary: Boundary::Closed,
            order: Order::First,
            sponge_cells: 16,
            dry_tolerance: 1e-10,
            max_bytes: 256 << 20,
            checkpoint_bytes: 64 << 20,
            max_work: 100_000_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cell {
    pub depth: f64,
    pub velocity: [f64; 2],
}
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub time: f64,
    pub cells: Vec<Cell>,
}

/// Conserved depth and two horizontal momenta.
type Q = [f64; 3];

/// Relaxation layer of `Boundary::Absorbing`. The target is the initial rest
/// surface elevation and velocity, so the target depth follows the current bed.
struct Sponge {
    /// Per cell: surface ordinate (depth minus downward bed) and velocity.
    rest: Vec<Q>,
    /// Longest rest-state wave speed; scales the relaxation rate.
    c_ref: f64,
}
/// Relaxation per layer crossing: one pass attenuates by `exp(-STRENGTH)`.
const SPONGE_STRENGTH: f64 = 2.0;
impl Sponge {
    /// Relaxes a quadratic ramp of `spec.sponge_cells` cells along every edge
    /// toward the rest state with the exact exponential, which is stable for
    /// any step and keeps depth non-negative. Momentum of dry cells is zeroed.
    fn relax(&self, spec: &Spec, bed: &[f64], q: &mut [Q], dt: f64) {
        let [nx, nz] = spec.cells;
        let w = spec.sponge_cells;
        for iz in 0..nz {
            let edge_z = iz.min(nz - 1 - iz);
            let mut visit = |ix: usize| {
                let d = edge_z.min(ix.min(nx - 1 - ix));
                if d >= w {
                    return;
                }
                let ramp = (w as f64 - (d as f64 + 0.5)) / w as f64;
                let rate = 3.0 * SPONGE_STRENGTH * self.c_ref * ramp * ramp / (w as f64 * spec.cell_size);
                let keep = (-rate * dt).exp();
                let i = iz * nx + ix;
                let rest = self.rest[i];
                let depth = (rest[0] + bed[i]).max(0.0);
                let cell = &mut q[i];
                for (a, v) in cell.iter_mut().enumerate() {
                    let target = if a == 0 { depth } else { depth * rest[a] };
                    *v = target + (*v - target) * keep;
                }
                if cell[0] < spec.dry_tolerance {
                    cell[1] = 0.0;
                    cell[2] = 0.0;
                }
            };
            if edge_z < w {
                (0..nx).for_each(&mut visit);
            } else {
                (0..w).chain(nx - w..nx).for_each(&mut visit);
            }
        }
    }
}
#[derive(Clone)]
struct State {
    time: f64,
    next_impulse: usize,
    q: Vec<Q>,
}
struct Work {
    remaining: u64,
}
impl Work {
    fn take(&mut self, amount: usize) -> Result<(), Error> {
        self.remaining = self.remaining.checked_sub(amount as u64).ok_or(Error::Limit("seek work"))?;
        Ok(())
    }
}

pub struct Ocean {
    spec: Spec,
    bed_y: Vec<f64>,
    impulses: Vec<Impulse>,
    initial: State,
    canonical: State,
    step: u64,
    checkpoints: Vec<(u64, State)>,
    checkpoint_capacity: usize,
    frame: Frame,
    sponge: Option<Sponge>,
}
impl Ocean {
    /// Bed ordinates and initial cells have exactly nx*nz samples. Inputs are
    /// validated before cloning or allocating solver workspaces. Impulses at
    /// time zero are part of the initial state; equal-time events retain order.
    pub fn new(spec: Spec, bed_y: Vec<f64>, cells: Vec<Cell>, mut impulses: Vec<Impulse>) -> Result<Self, Error> {
        let n = spec.cells[0].checked_mul(spec.cells[1]).ok_or(Error::Limit("cell count"))?;
        if spec.cells.contains(&0) || n > 4_000_000 {
            return Err(Error::Limit("cell dimensions (1–4,000,000 cells)"));
        }
        if !spec.cell_size.is_finite()
            || spec.cell_size <= 0.0
            || !spec.dt.is_finite()
            || spec.dt < 1e-6
            || !spec.gravity.is_finite()
            || spec.gravity <= 0.0
            || !spec.damping.is_finite()
            || spec.damping < 0.0
            || !spec.dry_tolerance.is_finite()
            || spec.dry_tolerance <= 0.0
            || spec.origin.iter().any(|v| !v.is_finite())
            || spec.max_work == 0
        {
            return Err(Error::Invalid("spacing, clock, gravity, damping, dry tolerance or work budget"));
        }
        for axis in 0..2 {
            let end = spec.origin[axis] + spec.cells[axis] as f64 * spec.cell_size;
            if !end.is_finite()
                || end <= spec.origin[axis]
                || spec.origin[axis] + 0.5 * spec.cell_size == spec.origin[axis]
            {
                return Err(Error::Invalid("domain extent or spatial precision"));
            }
        }
        let absorbing = spec.boundary == Boundary::Absorbing;
        if absorbing
            && (spec.sponge_cells < 4 || spec.sponge_cells.saturating_mul(2) >= spec.cells[0].min(spec.cells[1]))
        {
            return Err(Error::Invalid("sponge width (at least 4 cells, under half of each dimension)"));
        }
        if !(spec.cell_size * spec.cell_size).is_finite() || spec.cell_size * spec.cell_size == 0.0 {
            return Err(Error::Invalid("cell area"));
        }
        if bed_y.len() != n || cells.len() != n || impulses.len() > 16_384 {
            return Err(Error::Invalid("grid lengths or impulse count"));
        }
        // Input, initial/canonical/published/candidate/fractional/flux copies,
        // impulse scratch and Vec headers; checkpoint storage is separate.
        let bytes = n
            .checked_mul(256)
            .and_then(|v| bed_y.capacity().saturating_sub(n).checked_mul(8).and_then(|b| v.checked_add(b)))
            .and_then(|v| {
                // Stable sorting preserves authored equal-time order and may
                // allocate a temporary event array in addition to the input.
                impulses.capacity().checked_mul(2 * std::mem::size_of::<Impulse>()).and_then(|b| v.checked_add(b))
            })
            .and_then(|v| {
                if absorbing {
                    n.checked_mul(std::mem::size_of::<Q>()).and_then(|b| v.checked_add(b))
                } else {
                    Some(v)
                }
            })
            .and_then(|v| v.checked_add(4096))
            .ok_or(Error::Limit("resident bytes"))?;
        if bytes > spec.max_bytes {
            return Err(Error::Limit("resident bytes"));
        }
        for (&bed, cell) in bed_y.iter().zip(&cells) {
            if !bed.is_finite()
                || !cell.depth.is_finite()
                || cell.depth < 0.0
                || !(bed - cell.depth).is_finite()
                || !(spec.gravity * cell.depth * cell.depth).is_finite()
                || cell.velocity.iter().any(|v| !v.is_finite() || !(v * cell.depth).is_finite())
                || (cell.depth == 0.0 && cell.velocity != [0.0; 2])
            {
                return Err(Error::Invalid("bathymetry, depth or velocity"));
            }
        }
        for impulse in &impulses {
            impulse.validate()?;
        }
        impulses.sort_by(|a, b| a.time.total_cmp(&b.time));
        let sponge = absorbing.then(|| Sponge {
            rest: bed_y.iter().zip(&cells).map(|(&bed, c)| [c.depth - bed, c.velocity[0], c.velocity[1]]).collect(),
            c_ref: (spec.gravity * cells.iter().fold(0.0_f64, |m, c| m.max(c.depth))).sqrt(),
        });
        let mut initial = State {
            time: 0.0,
            next_impulse: 0,
            q: cells.iter().map(|c| [c.depth, c.depth * c.velocity[0], c.depth * c.velocity[1]]).collect(),
        };
        let mut work = Work { remaining: spec.max_work };
        advance(&spec, &bed_y, sponge.as_ref(), &impulses, &mut initial, 0.0, &mut work)?;
        let frame = publish(&initial, spec.dry_tolerance)?;
        let checkpoint_capacity = (spec.checkpoint_bytes / (n * std::mem::size_of::<Q>() + 128)).min(4096);
        Ok(Self {
            spec,
            bed_y,
            impulses,
            canonical: initial.clone(),
            initial,
            step: 0,
            checkpoints: Vec::new(),
            checkpoint_capacity,
            frame,
            sponge,
        })
    }

    /// Samples exact time. Fractional steps are disposable; they never alter
    /// canonical replay. An error leaves the published frame and caches intact.
    pub fn at(&mut self, time: f64) -> Result<&Frame, Error> {
        if !time.is_finite() || time < 0.0 {
            return Err(Error::Invalid("sample time"));
        }
        if time == self.frame.time {
            return Ok(&self.frame);
        }
        let quotient = time / self.spec.dt;
        if !quotient.is_finite() || quotient > (1_u64 << 48) as f64 {
            return Err(Error::Limit("timeline precision"));
        }
        let mut target = quotient.floor() as u64;
        // Division can round a time immediately before a tick up to the tick
        // index (e.g. 0.85/0.05 == 17, but 17*0.05 > 0.85). Compare the actual
        // canonical times so an impulse never fires before its timestamp.
        while target > 0 && target as f64 * self.spec.dt > time {
            target -= 1;
        }
        while (target + 1) as f64 * self.spec.dt <= time {
            target += 1;
        }
        let (mut k, mut state) =
            if self.step <= target { (self.step, self.canonical.clone()) } else { (0, self.initial.clone()) };
        if let Some((step, checkpoint)) =
            self.checkpoints.iter().filter(|(step, _)| *step <= target && *step > k).max_by_key(|(step, _)| step)
        {
            k = *step;
            state = checkpoint.clone();
        }
        let mut work = Work { remaining: self.spec.max_work };
        // Reject impossible replays before entering the loop.
        if target - k > self.spec.max_work / self.initial.q.len() as u64 {
            return Err(Error::Limit("seek work"));
        }
        while k < target {
            k += 1;
            advance(
                &self.spec,
                &self.bed_y,
                self.sponge.as_ref(),
                &self.impulses,
                &mut state,
                k as f64 * self.spec.dt,
                &mut work,
            )?;
        }
        let mut sampled = state.clone();
        advance(&self.spec, &self.bed_y, self.sponge.as_ref(), &self.impulses, &mut sampled, time, &mut work)?;
        let frame = publish(&sampled, self.spec.dry_tolerance)?;
        if self.checkpoint_capacity > 0 && target > 0 && !self.checkpoints.iter().any(|(k, _)| *k == target) {
            if self.checkpoints.len() == self.checkpoint_capacity {
                self.checkpoints.remove(0);
            }
            self.checkpoints.push((target, state.clone()));
        }
        self.canonical = state;
        self.step = target;
        self.frame = frame;
        Ok(&self.frame)
    }
    pub fn frame(&self) -> &Frame {
        &self.frame
    }
    pub fn checkpoint_bytes(&self) -> usize {
        self.checkpoints.len() * (self.initial.q.len() * std::mem::size_of::<Q>() + 128)
    }
}

fn publish(state: &State, dry: f64) -> Result<Frame, Error> {
    let cells: Result<Vec<_>, _> = state
        .q
        .iter()
        .map(|q| {
            if q.iter().any(|v| !v.is_finite()) || q[0] < 0.0 {
                return Err(Error::Numerical("nonfinite or negative state"));
            }
            let velocity = if q[0] < dry { [0.0; 2] } else { [q[1] / q[0], q[2] / q[0]] };
            if velocity.iter().any(|v| !v.is_finite()) {
                return Err(Error::Numerical("velocity overflow"));
            }
            Ok(Cell { depth: q[0], velocity })
        })
        .collect();
    Ok(Frame { time: state.time, cells: cells? })
}

/// Work units per cell and substep. A second-order substep runs two stages,
/// each a reconstruction plus a flux sweep; it measured 2.8-3.0x the wall time
/// of a first-order substep on a 256x256 grid (the CFL halving that doubles the
/// substep count is charged separately, by counting substeps).
const ORDER2_WORK_FACTOR: usize = 3;
fn step_work(spec: &Spec) -> usize {
    match spec.order {
        Order::First => 8,
        Order::Second => 8 * ORDER2_WORK_FACTOR,
    }
}

fn advance(
    spec: &Spec,
    bed: &[f64],
    sponge: Option<&Sponge>,
    impulses: &[Impulse],
    state: &mut State,
    target: f64,
    work: &mut Work,
) -> Result<(), Error> {
    loop {
        while let Some(event) = impulses.get(state.next_impulse).filter(|i| i.time <= state.time) {
            work.take(state.q.len().saturating_mul(8))?;
            event.apply(spec, &mut state.q)?;
            state.next_impulse += 1;
        }
        if state.time >= target {
            break;
        }
        let end = impulses.get(state.next_impulse).map_or(target, |i| i.time.min(target));
        work.take(state.q.len().saturating_mul(step_work(spec)))?;
        let mut speed = [0.0_f64; 2];
        for q in &state.q {
            let c = (spec.gravity * q[0]).sqrt();
            for a in 0..2 {
                let u = if q[0] >= spec.dry_tolerance { (q[a + 1] / q[0]).abs() } else { 0.0 };
                speed[a] = speed[a].max(u + c);
            }
        }
        let rate = (speed[0] + speed[1]) / spec.cell_size;
        if !rate.is_finite() {
            return Err(Error::Numerical("wave speed overflow"));
        }
        let dt = if rate == 0.0 { end - state.time } else { (end - state.time).min(flux::cfl(spec) / rate) };
        if dt <= 0.0 || state.time + dt == state.time {
            return Err(Error::Numerical("timestep precision"));
        }
        flux::step(spec, bed, sponge, &mut state.q, dt)?;
        state.time = if dt == end - state.time { end } else { state.time + dt };
    }
    Ok(())
}
