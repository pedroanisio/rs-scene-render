//! Bounded, deterministic three-dimensional smoke on a staggered MAC grid.
//!
//! Cell-centred density/kelvin and face-centred scene-unit velocities are advanced
//! with midpoint semi-Lagrangian advection, exponential decay, timed sources,
//! buoyancy and vorticity confinement. A matrix-free preconditioned conjugate
//! gradient projection enforces the authored divergence, with solid no-through
//! boundaries and either sealed or zero-pressure open domain edges.
//!
//! This is an incompressible cinematic gas model, not a compressible impact solver.
//! The discretization follows Bridson/Müller-Fischer's SIGGRAPH 2007 fluid notes;
//! confinement follows Fedkiw, Stam and Jensen, SIGGRAPH 2001. Obstacles are
//! voxelized at cell centres and must be resolved by the authored grid.

use rayon::prelude::*;
use sr_volume::{SparseGrid, Transform, Volume};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub mod mesh;
mod multigrid;

/// Minimum elements per rayon task. Task boundaries never change a value: every
/// parallel loop writes each element from the same expression as the serial loop
/// did, and all reductions stay serial and in index order.
const HEAVY: usize = 256;
const LIGHT: usize = 8192;

#[cfg(test)]
mod atomicity;
#[cfg(test)]
mod determinism;
#[cfg(test)]
mod pockets;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid pyro input: {0}")]
    Invalid(&'static str),
    #[error("pyro resource limit: {0}")]
    Limit(&'static str),
    #[error("pyro pressure solve did not converge (residual {0}); increase iterations or check sealed-domain expansion and collider motion")]
    Pressure(f64),
    #[error(transparent)]
    Volume(#[from] sr_volume::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boundary {
    Closed,
    Open,
}

/// Preconditioner of the pressure conjugate-gradient solve.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PressureSolver {
    /// Diagonal (Jacobi) preconditioner with serial, in-order reductions. The
    /// reference results of every release before `Multigrid`.
    #[default]
    Jacobi,
    /// Algebraic-multigrid V(1,1) preconditioner with fixed-block reductions;
    /// iteration counts stay roughly independent of resolution. Results are
    /// deterministic for any thread count but differ from `Jacobi` in the last bits.
    Multigrid,
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub cells: [usize; 3],
    /// Domain minimum in scene units; scalar cell zero is half a voxel inward.
    pub origin: [f64; 3],
    pub voxel_size: f64,
    pub dt: f64,
    pub boundary: Boundary,
    pub ambient_temperature: f64,
    /// Exponential density decay, per second.
    pub dissipation: f64,
    /// Exponential decay of excess temperature, per second.
    pub cooling: f64,
    /// Acceleration toward negative y per kelvin above ambient.
    pub buoyancy: f64,
    pub vorticity: f64,
    /// Seeded acceleration amplitude, scene units/second².
    pub turbulence: f64,
    pub seed: u64,
    /// Maximum conjugate-gradient iterations.
    pub pressure_iterations: usize,
    /// RMS divergence error allowed, in 1/second.
    pub pressure_tolerance: f64,
    pub solver: PressureSolver,
    /// Conservative resident state plus step workspace budget, before allocation.
    pub max_bytes: usize,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            cells: [32; 3],
            origin: [0.0; 3],
            voxel_size: 1.0,
            dt: 1.0 / 60.0,
            boundary: Boundary::Closed,
            ambient_temperature: 300.0,
            dissipation: 0.0,
            cooling: 0.0,
            buoyancy: 0.0,
            vorticity: 0.0,
            turbulence: 0.0,
            seed: 0,
            pressure_iterations: 200,
            pressure_tolerance: 1e-6,
            solver: PressureSolver::Jacobi,
            max_bytes: 256 << 20,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Shape {
    /// Validated closed, oriented triangle surface; shared immutable BVH.
    Mesh(std::sync::Arc<mesh::Mesh>),
    Sphere {
        center: [f64; 3],
        radius: f64,
    },
    Box {
        min: [f64; 3],
        max: [f64; 3],
    },
    /// Centred at the origin, with its axis along y.
    Cylinder {
        radius: f64,
        half_height: f64,
    },
    /// Apex at -half_height on y; base at +half_height.
    Cone {
        radius: f64,
        half_height: f64,
    },
    /// Swept sphere along the y segment [-half_segment, half_segment].
    Capsule {
        radius: f64,
        half_segment: f64,
    },
    /// Ring in the xz plane; tube radius may equal the ring radius.
    Torus {
        major_radius: f64,
        minor_radius: f64,
    },
    /// Shape-local coordinates mapped to domain scene coordinates. Supports
    /// translation, rotation, shear and nonuniform scale without changing units.
    Transformed {
        shape: Box<Shape>,
        transform: Box<Transform>,
    },
}

impl Shape {
    fn boundary_velocity(&self, p: [f64; 3]) -> Result<[f64; 3], Error> {
        match self {
            Self::Mesh(mesh) => mesh.boundary_velocity(p),
            Self::Transformed { shape, transform } => {
                let velocity = shape.boundary_velocity(transform.world_to_index(p))?;
                let columns = transform.columns();
                Ok(std::array::from_fn(|a| (0..3).map(|i| columns[i * 4 + a] * velocity[i]).sum()))
            }
            _ => Ok([0.0; 3]),
        }
    }

    fn validate(&self) -> Result<(), Error> {
        let mut current = self;
        for _ in 0..32 {
            match current {
                Self::Transformed { shape, .. } => current = shape,
                _ => return current.validate_leaf(),
            }
        }
        Err(Error::Limit("at most 32 nested source/collider transforms"))
    }

    fn validate_leaf(&self) -> Result<(), Error> {
        match self {
            Self::Mesh(_) => Ok(()),
            Self::Sphere { center, radius } if finite3(*center) && radius.is_finite() && *radius > 0.0 => Ok(()),
            Self::Box { min, max } if finite3(*min) && finite3(*max) && (0..3).all(|a| min[a] < max[a]) => Ok(()),
            Self::Cylinder { radius, half_height } | Self::Cone { radius, half_height }
                if nonnegative(*radius) && *radius > 0.0 && nonnegative(*half_height) && *half_height > 0.0 =>
            {
                Ok(())
            }
            Self::Capsule { radius, half_segment }
                if nonnegative(*radius) && *radius > 0.0 && nonnegative(*half_segment) =>
            {
                Ok(())
            }
            Self::Torus { major_radius, minor_radius }
                if nonnegative(*major_radius)
                    && nonnegative(*minor_radius)
                    && *minor_radius > 0.0
                    && minor_radius <= major_radius =>
            {
                Ok(())
            }
            _ => Err(Error::Invalid("source/collider shape must be finite with positive extent")),
        }
    }

    fn contains(&self, p: [f64; 3]) -> bool {
        match self {
            Self::Mesh(mesh) => mesh.contains(p),
            Self::Sphere { center, radius } => {
                let q = std::array::from_fn::<_, 3, _>(|a| (p[a] - center[a]) / radius);
                dot(q, q) <= 1.0
            }
            Self::Box { min, max } => (0..3).all(|a| p[a] >= min[a] && p[a] < max[a]),
            Self::Cylinder { radius, half_height } => p[1].abs() <= *half_height && p[0].hypot(p[2]) <= *radius,
            Self::Cone { radius, half_height } => {
                p[1].abs() <= *half_height && p[0].hypot(p[2]) / radius <= (p[1] / half_height + 1.0) * 0.5
            }
            Self::Capsule { radius, half_segment } => {
                p[0].hypot(p[2]).hypot((p[1].abs() - half_segment).max(0.0)) <= *radius
            }
            Self::Torus { major_radius, minor_radius } => {
                (p[0].hypot(p[2]) - major_radius).hypot(p[1]) <= *minor_radius
            }
            Self::Transformed { shape, transform } => shape.contains(transform.world_to_index(p)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Source {
    pub shape: Shape,
    pub start: f64,
    pub end: Option<f64>,
    pub density_rate: f64,
    /// Added kelvin per second, independent of density.
    pub temperature_rate: f64,
    /// Added velocity per second (scene units/second²).
    pub velocity_rate: [f64; 3],
    /// Target local divergence while active, in 1/second.
    pub expansion: f64,
}

impl Default for Source {
    fn default() -> Self {
        Self {
            shape: Shape::Sphere { center: [0.0; 3], radius: 1.0 },
            start: 0.0,
            end: None,
            density_rate: 0.0,
            temperature_rate: 0.0,
            velocity_rate: [0.0; 3],
            expansion: 0.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Obstacle {
    pub shape: Shape,
    /// Velocity at `velocity_origin`, in scene units/second.
    pub velocity: [f64; 3],
    pub velocity_origin: [f64; 3],
    /// Row-major spatial derivative: v(p) = velocity + gradient * (p - origin).
    /// Supports translation, rotation, scale and shear of a moving boundary.
    pub velocity_gradient: [[f64; 3]; 3],
}

impl Obstacle {
    pub fn stationary(shape: Shape) -> Self {
        Self { shape, velocity: [0.0; 3], velocity_origin: [0.0; 3], velocity_gradient: [[0.0; 3]; 3] }
    }

    fn velocity_at(&self, p: [f64; 3]) -> Result<[f64; 3], Error> {
        let offset = std::array::from_fn(|a| p[a] - self.velocity_origin[a]);
        let deformation = self.shape.boundary_velocity(p)?;
        let velocity =
            std::array::from_fn(|a| self.velocity[a] + dot(self.velocity_gradient[a], offset) + deformation[a]);
        if !finite3(velocity) {
            return Err(Error::Invalid("nonfinite solid boundary velocity"));
        }
        Ok(velocity)
    }
}

/// One-shot totals, applied once in the fixed interval containing `time`.
/// `expansion` is integrated divergence (dimensionless), divided by that step's
/// duration for the pressure target. This does not model a physical shock front.
#[derive(Clone, Debug)]
pub struct Impulse {
    pub shape: Shape,
    pub time: f64,
    pub density: f64,
    pub temperature: f64,
    pub velocity: [f64; 3],
    pub expansion: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Inputs {
    pub sources: Vec<Source>,
    pub impulses: Vec<Impulse>,
    pub obstacles: Vec<Obstacle>,
    pub acceleration: [f64; 3],
    /// Optional per-cell acceleration, x-fastest. Empty means no spatial field;
    /// otherwise its length must match the domain cell count exactly.
    pub spatial_acceleration: Vec<[f64; 3]>,
}

impl Inputs {
    fn validate(&self) -> Result<(), Error> {
        if self.sources.len().saturating_add(self.impulses.len()) > 4096 || self.obstacles.len() > 4096 {
            return Err(Error::Limit("at most 4096 sources/colliders per step"));
        }
        if !finite3(self.acceleration) {
            return Err(Error::Invalid("nonfinite acceleration"));
        }
        for s in &self.sources {
            s.shape.validate()?;
            if !s.start.is_finite()
                || s.end.is_some_and(|e| !e.is_finite() || e <= s.start)
                || !nonnegative(s.density_rate)
                || !nonnegative(s.temperature_rate)
                || !finite3(s.velocity_rate)
                || !s.expansion.is_finite()
            {
                return Err(Error::Invalid("invalid source rates or activation window"));
            }
        }
        for o in &self.obstacles {
            o.shape.validate()?;
            if !finite3(o.velocity) || !finite3(o.velocity_origin) || o.velocity_gradient.iter().any(|v| !finite3(*v)) {
                return Err(Error::Invalid("nonfinite collider velocity"));
            }
        }
        for impulse in &self.impulses {
            impulse.shape.validate()?;
            if !nonnegative(impulse.time)
                || !nonnegative(impulse.density)
                || !nonnegative(impulse.temperature)
                || !finite3(impulse.velocity)
                || !impulse.expansion.is_finite()
            {
                return Err(Error::Invalid("invalid impulse time or totals"));
            }
        }
        if self.spatial_acceleration.iter().any(|v| !finite3(*v)) {
            return Err(Error::Invalid("nonfinite spatial acceleration"));
        }
        Ok(())
    }
}

fn finite3(v: [f64; 3]) -> bool {
    v.iter().all(|v| v.is_finite())
}
fn nonnegative(v: f64) -> bool {
    v.is_finite() && v >= 0.0
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn index(p: [usize; 3], dims: [usize; 3]) -> usize {
    (p[2] * dims[1] + p[1]) * dims[0] + p[0]
}
fn coords(k: usize, dims: [usize; 3]) -> [usize; 3] {
    [k % dims[0], (k / dims[0]) % dims[1], k / (dims[0] * dims[1])]
}
fn world_point(origin: [f64; 3], h: f64, p: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|a| origin[a] + h * p[a])
}
fn face_dims(mut dims: [usize; 3], axis: usize) -> [usize; 3] {
    dims[axis] += 1;
    dims
}

/// Immutable observation of one fixed simulation step. Storage cannot be mutated
/// by callers; invalid/nonfinite fields therefore cannot bypass step validation.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    cells: [usize; 3],
    origin: [f64; 3],
    h: f64,
    ambient: f64,
    boundary: Boundary,
    density: Vec<f64>,
    temperature: Vec<f64>,
    velocity: [Vec<f64>; 3],
    solid: Vec<bool>,
    solid_velocity_low: Vec<[f64; 3]>,
    // Prescribed velocities at the low/high MAC faces, including nonlinear
    // deforming boundaries. Uses the same storage as the former affine samples.
    solid_velocity_high: Vec<[f64; 3]>,
}

impl State {
    /// Resident state storage and checkpoint-map bookkeeping. Step scratch is
    /// accounted separately by `Spec::max_bytes`.
    pub fn bytes(&self) -> usize {
        self.density.capacity() * 8
            + self.temperature.capacity() * 8
            + self.velocity.iter().map(|v| v.capacity() * 8).sum::<usize>()
            + self.solid.capacity()
            + self.solid_velocity_low.capacity() * 24
            + self.solid_velocity_high.capacity() * 24
            + std::mem::size_of::<Self>()
            + 256
    }
    pub fn density(&self) -> &[f64] {
        &self.density
    }
    pub fn temperature(&self) -> &[f64] {
        &self.temperature
    }
    pub fn cells(&self) -> [usize; 3] {
        self.cells
    }

    /// Domain-space cell centres in the same x-fastest order as scalar fields.
    pub fn cell_centres(&self) -> impl ExactSizeIterator<Item = [f64; 3]> + '_ {
        (0..self.density.len()).map(|k| self.cell_world(k))
    }

    fn world(&self, p: [f64; 3]) -> [f64; 3] {
        world_point(self.origin, self.h, p)
    }
    fn cell_world(&self, k: usize) -> [f64; 3] {
        self.world(coords(k, self.cells).map(|v| v as f64 + 0.5))
    }
    fn grid(&self, p: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|a| (p[a] - self.origin[a]) / self.h)
    }

    /// Trilinear MAC interpolation at a scene-space point. Nonfinite coordinates
    /// return zero velocity; outside an open domain the background is at rest.
    pub fn velocity_at(&self, p: [f64; 3]) -> [f64; 3] {
        if !finite3(p) {
            return [0.0; 3];
        }
        self.velocity_grid(self.grid(p))
    }

    fn field(&self) -> Field<'_> {
        Field {
            cells: self.cells,
            h: self.h,
            boundary: self.boundary,
            velocity: [&self.velocity[0], &self.velocity[1], &self.velocity[2]],
            solid: &self.solid,
        }
    }

    fn velocity_grid(&self, p: [f64; 3]) -> [f64; 3] {
        self.field().velocity_grid(p)
    }

    /// Working state of a step: the evolving fields copied from `self`, a fresh
    /// (empty) solid mask and zero solid velocities. The zero arrays are
    /// allocated lazily, so only pages near colliders are ever touched.
    fn working_copy(&self) -> State {
        let copy = |src: &[f64]| {
            let mut dst = vec![0.0; src.len()];
            dst.par_chunks_mut(LIGHT).zip(src.par_chunks(LIGHT)).for_each(|(d, s)| d.copy_from_slice(s));
            dst
        };
        let count = self.density.len();
        State {
            cells: self.cells,
            origin: self.origin,
            h: self.h,
            ambient: self.ambient,
            boundary: self.boundary,
            density: copy(&self.density),
            temperature: copy(&self.temperature),
            velocity: std::array::from_fn(|a| copy(&self.velocity[a])),
            solid: vec![false; count],
            solid_velocity_low: vec![[0.0; 3]; count],
            solid_velocity_high: vec![[0.0; 3]; count],
        }
    }

    /// Divergence in 1/second; an out-of-domain cell returns zero.
    pub fn divergence_at(&self, p: [usize; 3]) -> f64 {
        if (0..3).any(|a| p[a] >= self.cells[a]) {
            return 0.0;
        }
        (0..3)
            .map(|a| {
                let mut next = p;
                next[a] += 1;
                let dims = face_dims(self.cells, a);
                (self.velocity[a][index(next, dims)] - self.velocity[a][index(p, dims)]) / self.h
            })
            .sum()
    }

    /// Export all five scalar channels in scene units. The bound includes the
    /// worst-case brick storage before allocating any grid; no field is dropped.
    pub fn volume(&self, max_bytes: usize) -> Result<Volume, Error> {
        let bricks = self.cells.iter().map(|n| n.div_ceil(8)).product::<usize>();
        let needed = bricks
            .checked_mul(5 * (512 * 4 + 128))
            .and_then(|v| v.checked_add(4096))
            .ok_or(Error::Limit("volume export size overflow"))?;
        if needed > max_bytes {
            return Err(Error::Limit("volume export memory budget"));
        }
        let h = self.h;
        let o = self.origin.map(|v| v + h * 0.5);
        let transform = Transform::new([h, 0.0, 0.0, 0.0, 0.0, h, 0.0, 0.0, 0.0, 0.0, h, 0.0, o[0], o[1], o[2], 1.0])?;
        let mut volume = Volume::new();
        for (channel, name) in ["density", "temperature", "velocity.x", "velocity.y", "velocity.z"].iter().enumerate() {
            let bg = if channel == 1 { self.ambient as f32 } else { 0.0 };
            let mut grid = SparseGrid::new(transform, bg, bricks)?;
            for k in 0..self.density.len() {
                let value = match channel {
                    0 => self.density[k],
                    1 => self.temperature[k],
                    _ => self.velocity_at(self.cell_world(k))[channel - 2],
                } as f32;
                if !value.is_finite() {
                    return Err(Error::Invalid("exported field exceeds f32 representation"));
                }
                grid.set(coords(k, self.cells).map(|v| v as i32), value)?;
            }
            volume.insert(name, grid)?;
        }
        Ok(volume)
    }
}

/// Read-only view of the fields semi-Lagrangian tracing needs, so advection can
/// sample the pre-advection velocity while the new fields are written elsewhere.
#[derive(Clone, Copy)]
struct Field<'a> {
    cells: [usize; 3],
    h: f64,
    boundary: Boundary,
    velocity: [&'a [f64]; 3],
    solid: &'a [bool],
}

impl Field<'_> {
    fn velocity_grid(&self, p: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|axis| self.velocity_axis(p, axis))
    }

    /// One component of `velocity_grid`, bit-identical to indexing its result.
    fn velocity_axis(&self, p: [f64; 3], axis: usize) -> f64 {
        let q = std::array::from_fn(|a| p[a] - if a == axis { 0.0 } else { 0.5 });
        sample(self.velocity[axis], face_dims(self.cells, axis), q, self.boundary, 0.0)
    }

    fn solid_at(&self, p: [f64; 3]) -> bool {
        if (0..3).any(|a| p[a] < 0.0 || p[a] >= self.cells[a] as f64) {
            return false;
        }
        self.solid[index(p.map(|v| v.floor() as usize), self.cells)]
    }

    fn trace(&self, p: [f64; 3], dt: f64) -> Result<[f64; 3], Error> {
        let v = self.velocity_grid(p);
        let mid = std::array::from_fn(|a| p[a] - 0.5 * dt * v[a] / self.h);
        let v = self.velocity_grid(mid);
        let mut end = std::array::from_fn(|a| p[a] - dt * v[a] / self.h);
        if !finite3(end) {
            return Err(Error::Invalid("nonfinite advection trajectory"));
        }
        if self.boundary == Boundary::Closed {
            for (a, value) in end.iter_mut().enumerate() {
                *value = value.clamp(0.0, self.cells[a] as f64);
            }
        }
        // Do not let a long semi-Lagrangian trace tunnel through a voxel collider.
        let distance = (0..3).map(|a| (end[a] - p[a]).abs()).fold(0.0_f64, f64::max);
        let steps = (distance * 2.0).ceil().max(1.0);
        if steps > 4096.0 {
            return Err(Error::Limit("advection exceeds 2048 voxels per step; reduce fixed step"));
        }
        let mut previous = p;
        for i in 1..=steps as usize {
            let q = std::array::from_fn(|a| p[a] + (end[a] - p[a]) * i as f64 / steps);
            if self.solid_at(q) {
                return Ok(previous);
            }
            previous = q;
        }
        Ok(end)
    }
}

fn sample(values: &[f64], dims: [usize; 3], mut p: [f64; 3], boundary: Boundary, background: f64) -> f64 {
    if !finite3(p) {
        return background;
    }
    for a in 0..3 {
        if boundary == Boundary::Closed {
            p[a] = p[a].clamp(0.0, (dims[a] - 1) as f64);
        } else if p[a] < -1.0 || p[a] > dims[a] as f64 {
            return background;
        }
    }
    let lo = p.map(|v| v.floor() as i64);
    let f: [f64; 3] = std::array::from_fn(|a| p[a] - lo[a] as f64);
    let mut result = 0.0;
    for bit in 0..8 {
        let q: [i64; 3] = std::array::from_fn(|a| lo[a] + ((bit >> a) & 1));
        let weight: f64 = (0..3).map(|a| if (bit >> a) & 1 == 0 { 1.0 - f[a] } else { f[a] }).product();
        let v = if (0..3).all(|a| q[a] >= 0 && q[a] < dims[a] as i64) {
            values[index(q.map(|v| v as usize), dims)]
        } else {
            background
        };
        result += weight * v;
    }
    result
}

#[derive(Clone, Copy, Debug)]
pub struct StepReport {
    pub divergence_before: f64,
    pub divergence_after: f64,
    pub pressure_iterations: usize,
}

/// Wall time per stage of one fixed step, from `Simulation::step_profiled`.
/// `project_*` split the pressure solve; the conjugate-gradient parts sum over
/// all iterations. Timing never feeds back into the solver.
#[derive(Clone, Copy, Debug, Default)]
pub struct StepProfile {
    /// Copying the committed fields into the working state at the start of the step.
    pub clone: Duration,
    /// Collider voxelization, solid velocities and the cell clearing they imply.
    pub obstacles: Duration,
    /// Face boundary conditions and state validation, summed over the step.
    pub boundaries_validate: Duration,
    /// Semi-Lagrangian advection, including allocation of the advected fields.
    pub advect: Duration,
    pub advect_clone: Duration,
    /// Sources and impulses.
    pub inject: Duration,
    /// Acceleration, buoyancy, turbulence and vorticity confinement.
    pub forces: Duration,
    /// Diagonal, right-hand side and initial residual of the pressure solve.
    pub project_setup: Duration,
    /// Operator applications inside the conjugate-gradient loop.
    pub project_apply: Duration,
    /// V-cycle preconditioner applications (multigrid solver only).
    pub project_precondition: Duration,
    /// Inner products and residual norms inside that loop.
    pub project_reduce: Duration,
    /// Elementwise vector updates inside that loop.
    pub project_update: Duration,
    /// Pressure-gradient subtraction and the final divergence check.
    pub project_finish: Duration,
    pub total: Duration,
    pub pressure_iterations: usize,
}

fn lap(since: &mut Instant) -> Duration {
    let now = Instant::now();
    let elapsed = now - *since;
    *since = now;
    elapsed
}

pub struct Simulation {
    spec: Spec,
    state: State,
    step: u64,
}

impl Simulation {
    pub fn new(spec: Spec) -> Result<Self, Error> {
        let s = &spec;
        if s.cells.iter().any(|&n| !(2..=1024).contains(&n))
            || !finite3(s.origin)
            || !s.voxel_size.is_finite()
            || s.voxel_size <= 0.0
            || !(s.voxel_size * s.voxel_size).is_normal()
            || !s.dt.is_finite()
            || s.dt <= 0.0
            || !s.ambient_temperature.is_finite()
            || !(0.0..=50_000.0).contains(&s.ambient_temperature)
            || s.ambient_temperature == 0.0
            || !nonnegative(s.dissipation)
            || !nonnegative(s.cooling)
            || !nonnegative(s.buoyancy)
            || !nonnegative(s.vorticity)
            || !nonnegative(s.turbulence)
            || !(1..=10_000).contains(&s.pressure_iterations)
            || !s.pressure_tolerance.is_finite()
            || s.pressure_tolerance <= 0.0
        {
            return Err(Error::Invalid("invalid grid dimensions, rates, temperatures or solver settings"));
        }
        for a in 0..3 {
            let hi = s.origin[a] + s.voxel_size * s.cells[a] as f64;
            if !hi.is_finite() || hi <= s.origin[a] || s.origin[a] + 0.5 * s.voxel_size == s.origin[a] {
                return Err(Error::Invalid("domain coordinates cannot represent voxel centres"));
            }
        }
        let count =
            s.cells.iter().try_fold(1usize, |v, n| v.checked_mul(*n)).ok_or(Error::Limit("cell count overflow"))?;
        // Includes two states, advection copies, face arrays, PCG vectors, curl,
        // collider velocities, allocator overhead and transient volume metadata.
        let bytes =
            count.checked_mul(512).and_then(|v| v.checked_add(8192)).ok_or(Error::Limit("grid memory overflow"))?;
        if bytes > s.max_bytes {
            return Err(Error::Limit("grid and step workspace memory budget"));
        }
        let state = State {
            cells: s.cells,
            origin: s.origin,
            h: s.voxel_size,
            ambient: s.ambient_temperature,
            boundary: s.boundary,
            density: vec![0.0; count],
            temperature: vec![s.ambient_temperature; count],
            velocity: std::array::from_fn(|a| vec![0.0; face_dims(s.cells, a).iter().product()]),
            solid: vec![false; count],
            solid_velocity_low: vec![[0.0; 3]; count],
            solid_velocity_high: vec![[0.0; 3]; count],
        };
        Ok(Self { spec, state, step: 0 })
    }

    pub fn state(&self) -> &State {
        &self.state
    }
    pub fn time(&self) -> f64 {
        self.step as f64 * self.spec.dt
    }

    /// Advance exactly one fixed step. Inputs must describe this substep's
    /// collider pose. Failure leaves both the previous state and clock intact.
    pub fn step(&mut self, input: &Inputs) -> Result<StepReport, Error> {
        self.step_profiled(input).map(|(report, _)| report)
    }

    /// `step` plus the wall time of each stage. Results are identical.
    pub fn step_profiled(&mut self, input: &Inputs) -> Result<(StepReport, StepProfile), Error> {
        let mut profile = StepProfile::default();
        let started = Instant::now();
        let mut clock = started;
        input.validate()?;
        if !input.spatial_acceleration.is_empty() && input.spatial_acceleration.len() != self.state.density.len() {
            return Err(Error::Invalid("spatial acceleration must match the domain cell count"));
        }
        if self.step >= 10_000_000 {
            return Err(Error::Limit("simulation exceeds ten million fixed steps"));
        }
        let start = self.time();
        let end = (self.step + 1) as f64 * self.spec.dt;
        if !end.is_finite() || end <= start {
            return Err(Error::Invalid("fixed-step clock cannot advance"));
        }
        let dt = self.spec.dt;
        let mut state = self.state.working_copy();
        profile.clone = lap(&mut clock);
        for k in 0..state.density.len() {
            let p = state.cell_world(k);
            let collider = input.obstacles.iter().find(|o| o.shape.contains(p));
            if let Some(collider) = collider {
                state.solid[k] = true;
                for axis in 0..3 {
                    let mut low = p;
                    let mut high = p;
                    low[axis] -= 0.5 * state.h;
                    high[axis] += 0.5 * state.h;
                    state.solid_velocity_low[k][axis] = collider.velocity_at(low)?[axis];
                    state.solid_velocity_high[k][axis] = collider.velocity_at(high)?[axis];
                }
            }
            if state.solid[k] {
                state.density[k] = 0.0;
                state.temperature[k] = state.ambient;
            }
        }
        profile.obstacles = lap(&mut clock);
        boundaries(&mut state);
        validate_state(&state)?;
        profile.boundaries_validate += lap(&mut clock);
        advect(&mut state, dt, self.spec.dissipation, self.spec.cooling, &mut profile)?;
        profile.advect = lap(&mut clock);
        let mut target = vec![0.0; state.density.len()];
        for source in &input.sources {
            let overlap = (end.min(source.end.unwrap_or(end)) - start.max(source.start)).max(0.0);
            if overlap == 0.0 {
                continue;
            }
            inject(
                &mut state,
                &mut target,
                &source.shape,
                source.density_rate * overlap,
                source.temperature_rate * overlap,
                source.velocity_rate.map(|v| v * overlap),
                source.expansion * (overlap / dt),
            );
        }
        for impulse in &input.impulses {
            let ratio = impulse.time / dt;
            if ratio.is_finite() && fixed_step_index(ratio) == self.step {
                inject(
                    &mut state,
                    &mut target,
                    &impulse.shape,
                    impulse.density,
                    impulse.temperature,
                    impulse.velocity,
                    impulse.expansion / dt,
                );
            }
        }
        profile.inject = lap(&mut clock);
        forces(&mut state, &self.spec, input, self.step);
        profile.forces = lap(&mut clock);
        boundaries(&mut state);
        validate_state(&state)?;
        if target.iter().any(|v| !v.is_finite()) {
            return Err(Error::Invalid("nonfinite expansion"));
        }
        profile.boundaries_validate += lap(&mut clock);
        let report = project(&mut state, &target, &self.spec, &mut profile)?;
        clock = Instant::now();
        validate_state(&state)?;
        profile.boundaries_validate += lap(&mut clock);
        self.state = state;
        self.step += 1;
        profile.pressure_iterations = report.pressure_iterations;
        profile.total = started.elapsed();
        Ok((report, profile))
    }
}

fn validate_state(s: &State) -> Result<(), Error> {
    if s.density.par_iter().with_min_len(LIGHT).any(|v| !nonnegative(*v) || *v > f32::MAX as f64)
        || s.temperature.par_iter().with_min_len(LIGHT).any(|v| !v.is_finite() || !(0.0..=50_000.0).contains(v))
        || s.velocity
            .iter()
            .any(|axis| axis.par_iter().with_min_len(LIGHT).any(|v| !v.is_finite() || v.abs() > f32::MAX as f64))
    {
        return Err(Error::Invalid("simulated fields exceed finite density, velocity or 0–50000 K limits"));
    }
    Ok(())
}

fn face_cells(p: [usize; 3], axis: usize, dims: [usize; 3]) -> [Option<usize>; 2] {
    let left = if p[axis] > 0 {
        let mut q = p;
        q[axis] -= 1;
        Some(index(q, dims))
    } else {
        None
    };
    let right = if p[axis] < dims[axis] { Some(index(p, dims)) } else { None };
    [left, right]
}

fn boundaries(s: &mut State) {
    let (cells, boundary) = (s.cells, s.boundary);
    let (solid, high, low) = (&s.solid, &s.solid_velocity_high, &s.solid_velocity_low);
    for a in 0..3 {
        let dims = face_dims(cells, a);
        s.velocity[a].par_iter_mut().enumerate().with_min_len(LIGHT).for_each(|(k, velocity)| {
            let p = coords(k, dims);
            let faces = face_cells(p, a, cells);
            if boundary == Boundary::Closed && faces.iter().any(Option::is_none) {
                *velocity = 0.0;
            } else {
                let mut value = 0.0;
                let mut count = 0;
                for (side, cell) in faces.into_iter().enumerate() {
                    if let Some(c) = cell.filter(|&c| solid[c]) {
                        value += if side == 0 { high[c][a] } else { low[c][a] };
                        count += 1;
                    }
                }
                if count > 0 {
                    *velocity = value / count as f64;
                }
            }
        });
    }
}

/// Serial first error: chunks are collected in index order, so the error
/// returned is the one the serial loop would have hit first.
fn first_error(results: Vec<Result<(), Error>>) -> Result<(), Error> {
    results.into_iter().collect()
}

/// Advects density, temperature and velocity. The pre-advection fields are moved
/// out of `s` and sampled in place; fresh arrays are written, so no copy of the
/// state is made. Solid cells keep the cleared values the collider stage set.
/// On error `s` is left without fields; callers discard it.
fn advect(s: &mut State, dt: f64, dissipation: f64, cooling: f64, profile: &mut StepProfile) -> Result<(), Error> {
    let started = Instant::now();
    let (cells, boundary, ambient, count) = (s.cells, s.boundary, s.ambient, s.density.len());
    let old_density = std::mem::take(&mut s.density);
    let old_temperature = std::mem::take(&mut s.temperature);
    let old_velocity = std::mem::take(&mut s.velocity);
    s.density = vec![0.0; count];
    s.temperature = vec![ambient; count];
    s.velocity = std::array::from_fn(|a| vec![0.0; old_velocity[a].len()]);
    profile.advect_clone = started.elapsed();
    let (decay, cool) = ((-dissipation * dt).exp(), (-cooling * dt).exp());
    let solid = &s.solid;
    let old =
        Field { cells, h: s.h, boundary, velocity: [&old_velocity[0], &old_velocity[1], &old_velocity[2]], solid };
    let results = s
        .density
        .par_chunks_mut(HEAVY)
        .zip(s.temperature.par_chunks_mut(HEAVY))
        .enumerate()
        .map(|(chunk, (density, temperature))| -> Result<(), Error> {
            for (i, (d, t)) in density.iter_mut().zip(temperature.iter_mut()).enumerate() {
                let k = chunk * HEAVY + i;
                if solid[k] {
                    continue;
                }
                let p = coords(k, cells).map(|v| v as f64 + 0.5);
                let q = old.trace(p, dt)?.map(|v| v - 0.5);
                *d = sample(&old_density, cells, q, boundary, 0.0) * decay;
                let sampled = sample(&old_temperature, cells, q, boundary, ambient);
                *t = ambient + (sampled - ambient) * cool;
            }
            Ok(())
        })
        .collect();
    first_error(results)?;
    for a in 0..3 {
        let dims = face_dims(cells, a);
        let results = s.velocity[a]
            .par_chunks_mut(HEAVY)
            .enumerate()
            .map(|(chunk, values)| -> Result<(), Error> {
                for (i, value) in values.iter_mut().enumerate() {
                    let cell = coords(chunk * HEAVY + i, dims);
                    let p = std::array::from_fn(|i| cell[i] as f64 + if i == a { 0.0 } else { 0.5 });
                    let q = old.trace(p, dt)?;
                    *value = old.velocity_axis(q, a);
                }
                Ok(())
            })
            .collect();
        first_error(results)?;
    }
    boundaries(s);
    Ok(())
}

fn forces(s: &mut State, spec: &Spec, input: &Inputs, step: u64) {
    let count = s.density.len();
    let mut force = vec![[0.0; 3]; count];
    {
        let (temperature, ambient) = (&s.temperature, s.ambient);
        force.par_iter_mut().enumerate().with_min_len(HEAVY).for_each(|(k, f)| {
            for a in 0..3 {
                f[a] = input.acceleration[a]
                    + input.spatial_acceleration.get(k).map_or(0.0, |v| v[a])
                    + spec.turbulence * crate::rng::signed(spec.seed, k as u64, step * 3 + a as u64);
            }
            f[1] -= spec.buoyancy * (temperature[k] - ambient);
        });
    }
    if spec.vorticity > 0.0 {
        let state: &State = s;
        let mut curl = vec![[0.0; 3]; count];
        curl.par_iter_mut().enumerate().with_min_len(HEAVY).for_each(|(k, c)| {
            let p = coords(k, state.cells).map(|v| v as f64 + 0.5);
            let derivatives: [[f64; 3]; 3] = std::array::from_fn(|a| {
                let mut lo = p;
                lo[a] -= 1.0;
                let mut hi = p;
                hi[a] += 1.0;
                let vl = state.velocity_grid(lo);
                let vh = state.velocity_grid(hi);
                std::array::from_fn(|b| (vh[b] - vl[b]) / (2.0 * state.h))
            });
            *c = [
                derivatives[1][2] - derivatives[2][1],
                derivatives[2][0] - derivatives[0][2],
                derivatives[0][1] - derivatives[1][0],
            ];
        });
        let curl = &curl;
        force.par_iter_mut().enumerate().with_min_len(HEAVY).for_each(|(k, f)| {
            let p = coords(k, state.cells);
            let grad = std::array::from_fn(|a| {
                let mut lo = p;
                lo[a] = lo[a].saturating_sub(1);
                let mut hi = p;
                hi[a] = (hi[a] + 1).min(state.cells[a] - 1);
                let l = curl[index(lo, state.cells)];
                let h = curl[index(hi, state.cells)];
                (dot(h, h).sqrt() - dot(l, l).sqrt()) / (2.0 * state.h)
            });
            let length = dot(grad, grad).sqrt();
            if length > 1e-12 {
                let c = cross(grad.map(|v| v / length), curl[k]);
                for a in 0..3 {
                    f[a] += spec.vorticity * state.h * c[a];
                }
            }
        });
    }
    let (cells, solid) = (s.cells, &s.solid);
    let force = &force;
    for (a, values) in s.velocity.iter_mut().enumerate() {
        let dims = face_dims(cells, a);
        values.par_iter_mut().enumerate().with_min_len(LIGHT).for_each(|(k, value)| {
            let mut sum = 0.0;
            let mut n = 0;
            for c in face_cells(coords(k, dims), a, cells).into_iter().flatten() {
                if !solid[c] {
                    sum += force[c][a];
                    n += 1;
                }
            }
            if n > 0 {
                *value += spec.dt * sum / n as f64;
            }
        });
    }
}

fn neighbours(p: [usize; 3], dims: [usize; 3]) -> [Option<usize>; 6] {
    std::array::from_fn(|side| {
        let a = side / 2;
        let mut q = p;
        if side % 2 == 0 {
            if q[a] == 0 {
                return None;
            }
            q[a] -= 1;
        } else {
            q[a] += 1;
            if q[a] >= dims[a] {
                return None;
            }
        }
        Some(index(q, dims))
    })
}

fn project(s: &mut State, target: &[f64], spec: &Spec, profile: &mut StepProfile) -> Result<StepReport, Error> {
    let (iterations, tolerance, solver) = (spec.pressure_iterations, spec.pressure_tolerance, spec.solver);
    let mut clock = Instant::now();
    let count = s.density.len();
    let fluid = s.solid.iter().filter(|&&solid| !solid).count();
    if fluid == 0 {
        return Ok(StepReport { divergence_before: 0.0, divergence_after: 0.0, pressure_iterations: 0 });
    }
    let mut diagonal = vec![0.0; count];
    let mut rhs = vec![0.0; count];
    // Bit `side` (neighbours() order: -x, +x, -y, +y, -z, +z) is set when that
    // neighbour exists and is fluid, i.e. when the operator subtracts its value.
    let mut open = vec![0u8; count];
    {
        let state: &State = s;
        diagonal
            .par_iter_mut()
            .zip(rhs.par_iter_mut())
            .zip(open.par_iter_mut())
            .enumerate()
            .with_min_len(HEAVY)
            .for_each(|(k, ((d, r), mask))| {
                if state.solid[k] {
                    return;
                }
                let p = coords(k, state.cells);
                for (side, n) in neighbours(p, state.cells).into_iter().enumerate() {
                    if n.map_or(state.boundary == Boundary::Open, |n| !state.solid[n]) {
                        *d += 1.0;
                    }
                    if n.is_some_and(|n| !state.solid[n]) {
                        *mask |= 1 << side;
                    }
                }
                *r = (target[k] - state.divergence_at(p)) * state.h * state.h;
            });
    }
    let strides = [1, s.cells[0], s.cells[0] * s.cells[1]];
    let from_squares = |squares: f64| (squares / fluid as f64).sqrt() / (s.h * s.h);
    // Same per-cell arithmetic as the neighbours()-based operator: diagonal term
    // first, then the open neighbours subtracted in -x, +x, -y, +y, -z, +z order.
    // A solid cell has an empty mask, so it keeps only its diagonal term.
    let apply = |x: &[f64], out: &mut [f64]| {
        out.par_iter_mut().enumerate().with_min_len(LIGHT).for_each(|(k, o)| {
            *o = diagonal[k] * x[k];
            let mask = open[k];
            if mask == 0x3F {
                *o -= x[k - strides[0]];
                *o -= x[k + strides[0]];
                *o -= x[k - strides[1]];
                *o -= x[k + strides[1]];
                *o -= x[k - strides[2]];
                *o -= x[k + strides[2]];
            } else if mask != 0 {
                for side in 0..6 {
                    if mask >> side & 1 == 1 {
                        let stride = strides[side / 2];
                        *o -= x[if side % 2 == 0 { k - stride } else { k + stride }];
                    }
                }
            }
        });
    };
    let (pressure, before, used) = match solver {
        PressureSolver::Jacobi => {
            let rms = |values: &[f64]| from_squares(values.iter().map(|v| v * v).sum::<f64>());
            let before = rms(&rhs);
            if !before.is_finite() {
                return Err(Error::Invalid("nonfinite pressure right hand side"));
            }
            let inner = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
            let mut pressure = vec![0.0; count];
            let mut residual = rhs;
            let mut z: Vec<_> = residual
                .par_iter()
                .zip(diagonal.par_iter())
                .with_min_len(LIGHT)
                .map(|(r, d)| if *d > 0.0 { r / d } else { *r })
                .collect();
            let mut direction = z.clone();
            let mut applied = vec![0.0; count];
            let mut rz = inner(&residual, &z);
            // Both folds below start from `Iterator::sum`'s identity and add in index
            // order, exactly like `inner`/`rms`; fusing them only shares the pass over
            // memory, since the two accumulation chains are independent.
            let identity = std::iter::empty::<f64>().sum::<f64>();
            let mut current = rms(&residual);
            let mut used = 0;
            profile.project_setup = lap(&mut clock);
            loop {
                profile.project_reduce += lap(&mut clock);
                if !(current > tolerance && used < iterations) {
                    break;
                }
                apply(&direction, &mut applied);
                profile.project_apply += lap(&mut clock);
                let denom = inner(&direction, &applied);
                profile.project_reduce += lap(&mut clock);
                if !denom.is_finite() || denom <= 0.0 || !rz.is_finite() {
                    return Err(Error::Pressure(current));
                }
                let alpha = rz / denom;
                pressure
                    .par_iter_mut()
                    .zip(residual.par_iter_mut())
                    .zip(z.par_iter_mut())
                    .enumerate()
                    .with_min_len(LIGHT)
                    .for_each(|(k, ((p, r), z))| {
                        *p += alpha * direction[k];
                        *r -= alpha * applied[k];
                        *z = if diagonal[k] > 0.0 { *r / diagonal[k] } else { *r };
                    });
                profile.project_update += lap(&mut clock);
                let (mut next, mut squares) = (identity, identity);
                for (r, z) in residual.iter().zip(&z) {
                    next += r * z;
                    squares += r * r;
                }
                current = from_squares(squares);
                profile.project_reduce += lap(&mut clock);
                let beta = next / rz;
                direction.par_iter_mut().zip(z.par_iter()).with_min_len(LIGHT).for_each(|(d, z)| *d = z + beta * *d);
                profile.project_update += lap(&mut clock);
                rz = next;
                used += 1;
            }
            if current > tolerance {
                return Err(Error::Pressure(current));
            }
            (pressure, before, used)
        }
        PressureSolver::Multigrid => {
            let from_values =
                |values: &[f64]| from_squares(multigrid::blocked_sums(count, |k| [values[k] * values[k]])[0]);
            let before = from_values(&rhs);
            if !before.is_finite() {
                return Err(Error::Invalid("nonfinite pressure right hand side"));
            }
            let mut rhs = rhs;
            let (_, hierarchy) = rayon::join(
                || multigrid::remove_floating_means(s.cells, &s.solid, &open, &diagonal, &mut rhs),
                || multigrid::Hierarchy::new(multigrid::Fine { dims: s.cells, open: &open, diagonal: &diagonal }),
            );
            let mut scratch = hierarchy.scratch();
            let mut pressure = vec![0.0; count];
            let mut residual = rhs;
            let mut z = vec![0.0; count];
            let mut applied = vec![0.0; count];
            profile.project_setup = lap(&mut clock);
            let [squares] = multigrid::blocked_sums(count, |k| [residual[k] * residual[k]]);
            let mut current = from_squares(squares);
            let mut used = 0;
            profile.project_reduce += lap(&mut clock);
            // A residual already within tolerance needs no preconditioner pass.
            let (mut direction, mut rz) = if current > tolerance {
                hierarchy.precondition(&residual, &mut z, &mut applied, &mut scratch);
                profile.project_precondition += lap(&mut clock);
                let [rz] = multigrid::blocked_sums(count, |k| [residual[k] * z[k]]);
                profile.project_reduce += lap(&mut clock);
                (z.clone(), rz)
            } else {
                (Vec::new(), 0.0)
            };
            loop {
                if !(current > tolerance && used < iterations) {
                    break;
                }
                apply(&direction, &mut applied);
                profile.project_apply += lap(&mut clock);
                let [denom] = multigrid::blocked_sums(count, |k| [direction[k] * applied[k]]);
                profile.project_reduce += lap(&mut clock);
                if !denom.is_finite() || denom <= 0.0 || !rz.is_finite() {
                    return Err(Error::Pressure(current));
                }
                let alpha = rz / denom;
                pressure.par_iter_mut().zip(residual.par_iter_mut()).enumerate().with_min_len(LIGHT).for_each(
                    |(k, (p, r))| {
                        *p += alpha * direction[k];
                        *r -= alpha * applied[k];
                    },
                );
                profile.project_update += lap(&mut clock);
                hierarchy.precondition(&residual, &mut z, &mut applied, &mut scratch);
                profile.project_precondition += lap(&mut clock);
                let [next, squares] =
                    multigrid::blocked_sums(count, |k| [residual[k] * z[k], residual[k] * residual[k]]);
                current = from_squares(squares);
                profile.project_reduce += lap(&mut clock);
                let beta = next / rz;
                direction.par_iter_mut().zip(z.par_iter()).with_min_len(LIGHT).for_each(|(d, z)| *d = z + beta * *d);
                profile.project_update += lap(&mut clock);
                rz = next;
                used += 1;
            }
            if current > tolerance {
                return Err(Error::Pressure(current));
            }
            (pressure, before, used)
        }
    };
    let (cells, boundary, h) = (s.cells, s.boundary, s.h);
    for a in 0..3 {
        let dims = face_dims(cells, a);
        let (solid, pressure) = (&s.solid, &pressure);
        s.velocity[a].par_iter_mut().enumerate().with_min_len(LIGHT).for_each(|(k, velocity)| {
            let faces = face_cells(coords(k, dims), a, cells);
            if faces.iter().any(|c| c.is_some_and(|c| solid[c]))
                || (boundary == Boundary::Closed && faces.iter().any(Option::is_none))
            {
                return;
            }
            let [left, right] = faces.map(|c| c.map_or(0.0, |c| pressure[c]));
            *velocity -= (right - left) / h;
        });
    }
    let after = match solver {
        PressureSolver::Jacobi => {
            let squares: Vec<f64> = {
                let state: &State = s;
                (0..count)
                    .into_par_iter()
                    .with_min_len(HEAVY)
                    .map(|k| (state.divergence_at(coords(k, state.cells)) - target[k]).powi(2))
                    .collect()
            };
            (squares.iter().enumerate().filter(|(k, _)| !s.solid[*k]).map(|(_, t)| *t).sum::<f64>() / fluid as f64)
                .sqrt()
        }
        PressureSolver::Multigrid => {
            let state: &State = s;
            let [sum] = multigrid::blocked_sums(count, |k| {
                if state.solid[k] {
                    [0.0]
                } else {
                    [(state.divergence_at(coords(k, state.cells)) - target[k]).powi(2)]
                }
            });
            (sum / fluid as f64).sqrt()
        }
    };
    if !after.is_finite() || after > tolerance * 1.01 {
        return Err(Error::Pressure(after));
    }
    profile.project_finish = lap(&mut clock);
    Ok(StepReport { divergence_before: before, divergence_after: after, pressure_iterations: used })
}

/// Fixed-step, fallible playback with an independent hard checkpoint budget.
/// The input callback is sampled at each replayed step's local start time. It
/// must depend only on that time/index and immutable scene data, never playback
/// history. Recreate this timeline when the authored scene changes.
pub struct Timeline {
    simulation: Simulation,
    checkpoints: BTreeMap<u64, State>,
    every: u64,
    checkpoint_budget: usize,
    state_bytes: usize,
}

impl Timeline {
    pub fn new(spec: Spec, checkpoint_bytes: usize) -> Result<Self, Error> {
        let simulation = Simulation::new(spec)?;
        let state_bytes = simulation.state.bytes();
        if checkpoint_bytes < state_bytes {
            return Err(Error::Limit("checkpoint budget cannot hold the initial state"));
        }
        let every = (1.0 / simulation.spec.dt).ceil().clamp(1.0, 10_000_000.0) as u64;
        let checkpoints = BTreeMap::from([(0, simulation.state.clone())]);
        Ok(Self { simulation, checkpoints, every, checkpoint_budget: checkpoint_bytes, state_bytes })
    }

    pub fn checkpoint_bytes(&self) -> usize {
        self.checkpoints.len() * self.state_bytes
    }

    /// State at the last completed fixed step at or before `time`; negative time
    /// selects the initial state. A failed callback/step returns the error and
    /// retains the last successfully completed step, allowing a corrected retry.
    pub fn at(
        &mut self,
        time: f64,
        input: &mut impl FnMut(u64, f64) -> Result<Inputs, Error>,
    ) -> Result<&State, Error> {
        self.at_with_state(time, &mut |step, time, _| input(step, time))
    }

    /// Like `at`, with read-only access to the state before each fixed step.
    /// Velocity-dependent fields therefore sample the replayed state, including
    /// when a backward seek resumes from a retained checkpoint.
    pub fn at_with_state(
        &mut self,
        time: f64,
        input: &mut impl FnMut(u64, f64, &State) -> Result<Inputs, Error>,
    ) -> Result<&State, Error> {
        if !time.is_finite() {
            return Err(Error::Invalid("seek time must be finite"));
        }
        let ratio = time.max(0.0) / self.simulation.spec.dt;
        if !ratio.is_finite() || ratio > 10_000_000.0 {
            return Err(Error::Limit("seek exceeds ten million steps"));
        }
        // Correct only floating-point roundoff at exact fixed-step boundaries.
        let target = fixed_step_index(ratio);
        if self.simulation.step > target {
            let (&step, state) = self.checkpoints.range(..=target).next_back().expect("initial checkpoint is retained");
            self.simulation.state = state.clone();
            self.simulation.step = step;
        } else if let Some((&step, state)) = self.checkpoints.range(self.simulation.step..=target).next_back() {
            if step > self.simulation.step {
                self.simulation.state = state.clone();
                self.simulation.step = step;
            }
        }
        while self.simulation.step < target {
            let value = input(self.simulation.step, self.simulation.time(), self.simulation.state())?;
            self.simulation.step(&value)?;
            let step = self.simulation.step;
            if step % self.every == 0 && !self.checkpoints.contains_key(&step) {
                // Thin before cloning: transient insertion never exceeds budget.
                while self.checkpoints.len() >= self.checkpoint_budget / self.state_bytes && self.checkpoints.len() > 1
                {
                    self.every = self.every.saturating_mul(2);
                    let every = self.every;
                    self.checkpoints.retain(|k, _| k % every == 0);
                }
                if step % self.every == 0 && self.checkpoints.len() < self.checkpoint_budget / self.state_bytes {
                    self.checkpoints.insert(step, self.simulation.state.clone());
                }
            }
        }
        Ok(self.simulation.state())
    }
}

fn fixed_step_index(ratio: f64) -> u64 {
    let nearest = ratio.round();
    (if (ratio - nearest).abs() <= f64::EPSILON * 4.0 * ratio.max(1.0) { nearest } else { ratio.floor() }) as u64
}

fn inject(
    state: &mut State,
    target: &mut [f64],
    shape: &Shape,
    density: f64,
    temperature: f64,
    velocity: [f64; 3],
    expansion: f64,
) {
    let (cells, origin, h) = (state.cells, state.origin, state.h);
    let State { density: cell_density, temperature: cell_temperature, velocity: faces, solid, .. } = state;
    let solid = &*solid;
    cell_density
        .par_iter_mut()
        .zip(cell_temperature.par_iter_mut())
        .zip(target.par_iter_mut())
        .enumerate()
        .with_min_len(HEAVY)
        .for_each(|(k, ((d, t), target_value))| {
            if !solid[k] && shape.contains(world_point(origin, h, coords(k, cells).map(|v| v as f64 + 0.5))) {
                *d += density;
                *t += temperature;
                *target_value += expansion;
            }
        });
    for (a, amount) in velocity.into_iter().enumerate() {
        let dims = face_dims(cells, a);
        faces[a].par_iter_mut().enumerate().with_min_len(HEAVY).for_each(|(k, v)| {
            let cell = coords(k, dims);
            let p = world_point(origin, h, std::array::from_fn(|i| cell[i] as f64 + if i == a { 0.0 } else { 0.5 }));
            if shape.contains(p) {
                *v += amount;
            }
        });
    }
}
