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

mod key;
mod maccormack;
pub mod mesh;
mod multigrid;

pub use key::volume_key;

/// Minimum elements per rayon task. Task boundaries never change a value: every
/// parallel loop writes each element from the same expression as the serial loop
/// did, and all reductions stay serial and in index order.
const HEAVY: usize = 256;
const LIGHT: usize = 8192;
/// A temperature that differs from ambient by less than this share of it is the rounding of the interpolation, not
/// heat: the window that follows a plume does not count it as smoke.
const HEAT_NOISE: f64 = 1e-9;

#[cfg(test)]
mod atomicity;
#[cfg(test)]
mod detail;
#[cfg(test)]
mod determinism;
#[cfg(test)]
mod export;
#[cfg(test)]
mod pockets;
#[cfg(test)]
mod sampling;
#[cfg(test)]
mod voxel_memory;
#[cfg(test)]
mod window;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid pyro input: {0}")]
    Invalid(&'static str),
    #[error("pyro resource limit: {0}")]
    Limit(&'static str),
    /// What the scene's rigid world, a driver of the inputs, said when it could not answer.
    #[error("pyro inputs: {0}")]
    Driver(String),
    #[error(
        "pyro pressure solve did not converge (residual {residual}{}); increase iterations or check sealed-domain expansion and collider motion",
        worst.map_or(String::new(), |w| format!(", largest at cell {:?} with {}", w.cell, w.value))
    )]
    Pressure { residual: f64, worst: Option<Worst> },
    #[error(transparent)]
    Volume(#[from] sr_volume::Error),
}

/// Where a pressure solve was furthest from its target: the cell and the residual there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Worst {
    pub cell: [usize; 3],
    pub value: f64,
}

/// The cell with the largest absolute value among `values` (a value that is not finite counts as the largest, the
/// first of them), if any.
fn worst_of(values: impl Iterator<Item = (usize, f64)>, cells: [usize; 3]) -> Option<Worst> {
    let mut found: Option<(usize, f64)> = None;
    for (k, v) in values {
        let bigger = match found {
            None => true,
            Some((_, w)) if w.is_finite() => !v.is_finite() || v.abs() > w.abs(),
            Some(_) => false,
        };
        if bigger {
            found = Some((k, v));
        }
    }
    found.map(|(k, value)| Worst { cell: coords(k, cells), value })
}

/// The fluid cell where the divergence of `state` is furthest from `target`: where the flow cannot be made to
/// satisfy it, which is where the sealed or expanding region is.
fn worst_divergence(state: &State, target: &[f64]) -> Option<Worst> {
    let left = (0..target.len())
        .filter(|k| !state.solid[*k])
        .map(|k| (k, state.divergence_at(coords(k, state.cells)) - target[k]));
    worst_of(left, state.cells)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boundary {
    Closed,
    Open,
}

/// Transport scheme for density, temperature and velocity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Advection {
    /// Midpoint semi-Lagrangian: unconditionally stable, numerically diffusive.
    /// The reference results of every release before `MacCormack`.
    #[default]
    SemiLagrangian,
    /// Semi-Lagrangian with a backward-forward error correction (Selle et al.,
    /// 2008), limited to the local extrema of the interpolation stencil, and
    /// plain semi-Lagrangian where a collider cuts a trace short. It keeps
    /// finer detail at the same resolution and cannot create new extrema.
    MacCormack,
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
    pub advection: Advection,
    /// Conservative resident state plus step workspace budget, before allocation.
    pub max_bytes: usize,
    /// A window that follows its plume: absent, the domain is where it began for good.
    pub follow: Option<Follow>,
}

/// A domain whose window moves by whole cells to keep the smoke away from its faces, with the same number of cells
/// whatever it does, so that its memory and the cost of a step do not change. It needs an open domain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Follow {
    /// Cells the smoke is kept from every face, when the window can move without leaving too much smoke behind.
    pub margin: usize,
    /// The share of all the smoke that the window may leave behind, each side of it: zero lets go of no smoke at
    /// all (only slabs that hold none), and more lets go of the thin tail a plume drags behind it, the loss counted
    /// in [`State::lost`].
    pub loss: f64,
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
            advection: Advection::SemiLagrangian,
            max_bytes: 256 << 20,
            follow: None,
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

/// A blast: the front of a Sedov-Taylor blast (see [`crate::sedov`]) as an incompressible spherical piston in the smoke.
///
/// `energy` is released at `center` (scene units) at `time` (seconds from the start of the simulation) into air of density `ambient_density`
/// (kg/m^3) and pressure `ambient_pressure` (Pa), a gas of `gamma`; a metre is `pixels_per_meter` scene units. The front is at
/// `R(t) = xi0 (E t^2 / rho0)^(1/5)` metres until it reaches `0.3 (E / p0)^(1/3)` (the end of the strong phase: the radius at which the pressure
/// behind the front has fallen to a few times the ambient one, 5.8 for air), and stays there. Each step the air that the front swept in the step
/// is displaced: the volume `4 pi (R1^3 - R0^3) / 3` is given to the cells whose centres are in the sphere of the radius that the front has at
/// the end of the step (at least a cell: a front smaller than that is the sphere of one cell, and the volume is still the volume swept), spread
/// evenly over the cells that the sphere has (all of them: a window that the sphere cuts holds some, and the divergence is the one that the whole sphere
/// makes in free space, which agrees with the flow outside), so that the air given is exactly the volume swept whatever the cells make of the sphere.
/// The flow is that of the piston, made apart from the smoke's own: the analytic flow (inside, `u = d (x - c) / 3` for the divergence `d`; outside,
/// `u = Q / (4 pi r^2)` away from the centre `c` of the blast, `Q` the volume over the step) is projected alone, which is linear, and carries the
/// smoke (the density and the temperature) once over the step. It is not kept in the velocity of the smoke: a potential flow with open faces is not
/// removed by a projection with no divergence, so it would stay for ever, and the velocity is what it would have been with no blast, to the bit
/// ([`Simulation::blast_flow`] gives the flow of the last step). The shock is not carried (the solver is incompressible), nothing is heated and no smoke
/// is made. It needs an open domain.
#[derive(Clone, Debug)]
pub struct Blast {
    center: [f64; 3],
    time: f64,
    energy: f64,
    ambient_density: f64,
    ambient_pressure: f64,
    gamma: f64,
    pixels_per_meter: f64,
    law: std::sync::Arc<crate::sedov::Sedov>,
}

impl Blast {
    pub fn new(
        center: [f64; 3],
        time: f64,
        energy: f64,
        ambient_density: f64,
        ambient_pressure: f64,
        gamma: f64,
        pixels_per_meter: f64,
    ) -> Result<Blast, Error> {
        if !finite3(center) || !nonnegative(time) {
            return Err(Error::Invalid("a blast has a finite place and a time from zero"));
        }
        if !nonnegative(energy) || !(ambient_density.is_finite() && ambient_density > 0.0) {
            return Err(Error::Invalid("a blast has an energy that is not negative and an air of positive density"));
        }
        if !(ambient_pressure.is_finite() && ambient_pressure > 0.0)
            || !(pixels_per_meter.is_finite() && pixels_per_meter > 0.0)
        {
            return Err(Error::Invalid("a blast has a positive ambient pressure and pixels to the metre"));
        }
        let law = crate::sedov::solve_cached(gamma)
            .map_err(|_| Error::Invalid("a blast needs a ratio of specific heats from 1.1 to 3"))?;
        Ok(Blast { center, time, energy, ambient_density, ambient_pressure, gamma, pixels_per_meter, law })
    }

    pub fn center(&self) -> [f64; 3] {
        self.center
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn energy(&self) -> f64 {
        self.energy
    }

    pub fn gamma(&self) -> f64 {
        self.gamma
    }

    /// The front `t` seconds after the release, in scene units.
    fn front(&self, t: f64) -> f64 {
        if t <= 0.0 || self.energy == 0.0 {
            return 0.0;
        }
        let strong = self
            .law
            .radius(self.energy, self.ambient_density, t)
            .min(self.law.max_radius(self.energy, self.ambient_pressure));
        strong * self.pixels_per_meter
    }

    /// The radius of the front at the start and at the end of the fixed step `step`, in scene units: nothing before the step in which the blast is
    /// released, and the front at the end of that step (the time since the release) after it.
    pub fn radii(&self, dt: f64, step: u64) -> (f64, f64) {
        let first = fixed_step_index(self.time / dt);
        if step < first {
            return (0.0, 0.0);
        }
        let end = self.front((step + 1) as f64 * dt - self.time);
        let start = if step == first { 0.0 } else { self.front(step as f64 * dt - self.time) };
        (start, end)
    }

    /// What the front sweeps in the step, if it moved: the radius that it ends the step at and the volume between the two spheres, in cubic scene
    /// units.
    pub fn swept(&self, dt: f64, step: u64) -> Option<(f64, f64)> {
        let (r0, r1) = self.radii(dt, step);
        (r1 > r0).then(|| (r1, 4.0 / 3.0 * std::f64::consts::PI * (r1.powi(3) - r0.powi(3))))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Inputs {
    pub sources: Vec<Source>,
    pub impulses: Vec<Impulse>,
    /// Blasts, which need an open domain.
    pub blasts: Vec<Blast>,
    /// Sources whose expansion follows from their heat rather than being given: smoke heated at
    /// constant pressure, as an ideal gas, expands at `(dT/dt) / T` in every cell it heats, so
    /// `expansion` must be zero. Their `density_rate` is a total: the sum of the volume fractions
    /// they add, per second, spread evenly over the cells their shape covers that are not solid.
    /// Otherwise they are `sources`.
    pub heated: Vec<Source>,
    /// One-shot totals with the same derived expansion; `expansion` must be zero. Their `density`
    /// is the sum of the volume fractions they add, spread evenly over the free cells they cover.
    pub heated_impulses: Vec<Impulse>,
    pub obstacles: Vec<Obstacle>,
    pub acceleration: [f64; 3],
    /// Optional per-cell acceleration, x-fastest. Empty means no spatial field;
    /// otherwise its length must match the domain cell count exactly.
    pub spatial_acceleration: Vec<[f64; 3]>,
}

impl Inputs {
    /// The shapes of the sources and impulses that act in the fixed step `step` from `start` to `end`.
    fn acting(&self, start: f64, end: f64, step: u64, dt: f64) -> Vec<&Shape> {
        let acts = |s: &Source| (end.min(s.end.unwrap_or(end)) - start.max(s.start)).max(0.0) > 0.0;
        let fires = |i: &Impulse| {
            let ratio = i.time / dt;
            ratio.is_finite() && fixed_step_index(ratio) == step
        };
        self.sources
            .iter()
            .chain(&self.heated)
            .filter(|s| acts(s))
            .map(|s| &s.shape)
            .chain(self.impulses.iter().chain(&self.heated_impulses).filter(|i| fires(i)).map(|i| &i.shape))
            .collect()
    }

    fn validate(&self) -> Result<(), Error> {
        let heated = self.heated.len().saturating_add(self.heated_impulses.len());
        if self.sources.len().saturating_add(self.impulses.len()).saturating_add(heated) > 4096
            || self.obstacles.len() > 4096
        {
            return Err(Error::Limit("at most 4096 sources/colliders per step"));
        }
        if !finite3(self.acceleration) {
            return Err(Error::Invalid("nonfinite acceleration"));
        }
        if self.heated.iter().any(|s| s.expansion != 0.0) || self.heated_impulses.iter().any(|i| i.expansion != 0.0) {
            return Err(Error::Invalid("the expansion of a heated source is derived from its heat"));
        }
        for s in self.sources.iter().chain(&self.heated) {
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
        for impulse in self.impulses.iter().chain(&self.heated_impulses) {
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
    /// The origin the domain began with, and the whole cells it has moved by since (a window that follows its
    /// plume): `origin` is always `base + window * h`, computed afresh, never accumulated.
    base: [f64; 3],
    window: [i64; 3],
    /// The density the window has let go of by following the plume, summed over cells: none unless it was
    /// allowed to.
    lost: f64,
    h: f64,
    ambient: f64,
    boundary: Boundary,
    density: Vec<f64>,
    temperature: Vec<f64>,
    velocity: [Vec<f64>; 3],
    solid: Vec<bool>,
}

/// The velocity of one step's gas, copied out of a [`State`] so that something that reads it (the
/// particles an emitter drags along) does not hold the simulation.
#[derive(Clone, Debug, PartialEq)]
pub struct Gas {
    cells: [usize; 3],
    origin: [f64; 3],
    h: f64,
    boundary: Boundary,
    velocity: [Vec<f64>; 3],
    solid: Vec<bool>,
}

impl State {
    /// A copy of this step's velocity field.
    pub fn gas(&self) -> Gas {
        Gas {
            cells: self.cells,
            origin: self.origin,
            h: self.h,
            boundary: self.boundary,
            velocity: self.velocity.clone(),
            solid: self.solid.clone(),
        }
    }
}

impl Gas {
    /// Bytes the copy holds.
    pub fn bytes(&self) -> usize {
        self.velocity.iter().map(|v| v.len() * std::mem::size_of::<f64>()).sum::<usize>() + self.solid.len()
    }

    /// The gas's velocity at a point in the volume's own axes: the solver's trilinear interpolation
    /// inside the domain, nothing more than a cell outside it, and in between the value at the nearest
    /// point of the domain fading to nothing across that cell, so that what a particle feels has no jump
    /// at the boundary (a closed domain would otherwise hold its wall value forever outside). A point that
    /// is not finite is at rest.
    pub fn velocity_at(&self, p: [f64; 3]) -> [f64; 3] {
        if !finite3(p) {
            return [0.0; 3];
        }
        let q: [f64; 3] = std::array::from_fn(|a| (p[a] - self.origin[a]) / self.h);
        let outside = (0..3).map(|a| (-q[a]).max(q[a] - self.cells[a] as f64).max(0.0)).fold(0.0, f64::max);
        if outside >= 1.0 {
            return [0.0; 3];
        }
        let inside: [f64; 3] = std::array::from_fn(|a| q[a].clamp(0.0, self.cells[a] as f64));
        let field = Field {
            cells: self.cells,
            h: self.h,
            boundary: self.boundary,
            velocity: [&self.velocity[0], &self.velocity[1], &self.velocity[2]],
            solid: &self.solid,
        };
        let v = field.velocity_grid(inside);
        if outside == 0.0 {
            v
        } else {
            v.map(|c| c * (1.0 - outside))
        }
    }
}

/// Prescribed velocities at the low and high MAC faces of one solid cell,
/// including nonlinear deforming boundaries. They are recomputed from the
/// colliders at the start of every step and used only during it, so they are
/// kept sparsely for the step and not stored in the state.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SolidFaces {
    cell: usize,
    low: [f64; 3],
    high: [f64; 3],
}

/// The solid-cell mask of `obstacles` (the first listed collider that contains a
/// cell centre wins) and each solid cell's face velocities, in cell order.
fn voxelize(
    cells: [usize; 3],
    origin: [f64; 3],
    h: f64,
    obstacles: &[Obstacle],
) -> Result<(Vec<bool>, Vec<SolidFaces>), Error> {
    let count = cells.iter().product();
    let mut solid = vec![false; count];
    let centre = |k: usize| world_point(origin, h, coords(k, cells).map(|v| v as f64 + 0.5));
    // the mask first, so that the list of faces is allocated once, at the size it is charged for
    let mut solid_cells = 0usize;
    for (k, is_solid) in solid.iter_mut().enumerate() {
        let p = centre(k);
        if obstacles.iter().any(|o| o.shape.contains(p)) {
            *is_solid = true;
            solid_cells += 1;
        }
    }
    let mut faces = Vec::with_capacity(solid_cells);
    for (k, _) in solid.iter().enumerate().filter(|(_, s)| **s) {
        let p = centre(k);
        let collider = obstacles.iter().find(|o| o.shape.contains(p)).expect("a cell of the mask is in a collider");
        let (mut low_velocity, mut high_velocity) = ([0.0; 3], [0.0; 3]);
        for (axis, (low_v, high_v)) in low_velocity.iter_mut().zip(&mut high_velocity).enumerate() {
            let mut low = p;
            let mut high = p;
            low[axis] -= 0.5 * h;
            high[axis] += 0.5 * h;
            *low_v = collider.velocity_at(low)?[axis];
            *high_v = collider.velocity_at(high)?[axis];
        }
        faces.push(SolidFaces { cell: k, low: low_velocity, high: high_velocity });
    }
    Ok((solid, faces))
}

/// Brick coordinates and the 8x8x8 samples of a brick that holds something other than the background.
type ExportedBrick = ([i32; 3], Box<[f32; 512]>);

impl State {
    /// Resident state storage and checkpoint-map bookkeeping. Step scratch is
    /// accounted separately by `Spec::max_bytes`.
    pub fn bytes(&self) -> usize {
        self.density.capacity() * 8
            + self.temperature.capacity() * 8
            + self.velocity.iter().map(|v| v.capacity() * 8).sum::<usize>()
            + self.solid.capacity()
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
    /// The domain minimum in scene units now: where the window is.
    pub fn origin(&self) -> [f64; 3] {
        self.origin
    }
    /// Whole cells the window has moved by since the domain began.
    pub fn window(&self) -> [i64; 3] {
        self.window
    }
    /// The density summed over the cells the window has let go of while following its plume.
    pub fn lost(&self) -> f64 {
        self.lost
    }
    /// The velocity on the faces normal to `axis`, x-fastest, one more face than cells along that axis.
    pub fn velocity_faces(&self, axis: usize) -> &[f64] {
        &self.velocity[axis]
    }

    /// Whether the smoke is somewhere in cell `k`: it holds density, or it is warmer or cooler than ambient by
    /// more than [`HEAT_NOISE`] of the ambient temperature. The interpolation of the advection leaves rounding
    /// (299.99999999999994 K for 300 K) in air that nothing has touched, and that is not heat.
    fn active(&self, k: usize) -> bool {
        self.density[k] != 0.0 || (self.temperature[k] - self.ambient).abs() > HEAT_NOISE * self.ambient
    }

    /// Where the window should move so that the smoke is `margin` cells from every face, and on which axes it
    /// cannot: the whole cells to move by (the sign says toward which side the cells are numbered up to, as in
    /// [`Simulation::shift_window`]) and, for each axis, whether the smoke is nearer a face than `margin` cells
    /// (plus the cells the fastest air along the axis goes in a step of `dt`)
    /// and the window cannot move away from it (or the smoke is near both). The smoke along an axis is the span
    /// of slabs between the ends that hold at most `loss / 2` of all the density each (none at all, and no heat,
    /// for a `loss` of zero): integer indices from sums in a fixed order, the same on any number of threads. A
    /// move is only as long as the slabs it leaves behind allow, and never leaves a slab with a cell in `keep` (the
    /// shapes of the sources that are acting: a plume begins at its source, and a window that left the source
    /// would part the plume from it).
    pub fn follow_decision(&self, margin: usize, loss: f64, dt: f64, keep: &[&Shape]) -> ([i64; 3], [bool; 3]) {
        let n = self.cells;
        // the density and the heat of each slab of cells normal to each axis, summed in the order of the cells
        let mut mass: [Vec<f64>; 3] = std::array::from_fn(|a| vec![0.0; n[a]]);
        let mut heat: [Vec<u64>; 3] = std::array::from_fn(|a| vec![0; n[a]]);
        // and the density times the velocity of the air along the axis, to tell which way the smoke at each end is going
        let mut flux: [Vec<f64>; 3] = std::array::from_fn(|a| vec![0.0; n[a]]);
        for z in 0..n[2] {
            for y in 0..n[1] {
                let row = index([0, y, z], n);
                for x in 0..n[0] {
                    let k = row + x;
                    let d = self.density[k];
                    let hot = u64::from((self.temperature[k] - self.ambient).abs() > HEAT_NOISE * self.ambient);
                    for (a, i) in [x, y, z].into_iter().enumerate() {
                        mass[a][i] += d;
                        heat[a][i] += hot;
                        if d != 0.0 {
                            let dims = face_dims(n, a);
                            let (low, mut high) = ([x, y, z], [x, y, z]);
                            high[a] += 1;
                            let v = 0.5 * (self.velocity[a][index(low, dims)] + self.velocity[a][index(high, dims)]);
                            flux[a][i] += d * v;
                        }
                    }
                }
            }
        }
        // a step moves the smoke by as many cells as the fastest air along the axis goes in it, so the margin
        // is that many cells more than asked for, as far as a cell is left between the faces
        let reach: [usize; 3] = std::array::from_fn(|a| {
            let fastest = self.velocity[a].iter().fold(0.0f64, |m, v| m.max(v.abs()));
            let cells = (fastest * dt / self.h).ceil();
            let room = (n[a] - 1) / 2;
            if cells.is_finite() {
                (cells as usize).min(room.saturating_sub(margin))
            } else {
                0
            }
        });
        let total: f64 = mass[0].iter().sum();
        let allowed = loss / 2.0 * total;
        let mut by = [0i64; 3];
        let mut blocked = [false; 3];
        for a in 0..3 {
            // a slab may be trimmed off the end of the smoke while the trimmed density stays within the share
            let gone = |i: usize, trimmed: f64| {
                if loss == 0.0 {
                    mass[a][i] == 0.0 && heat[a][i] == 0
                } else {
                    trimmed + mass[a][i] <= allowed
                }
            };
            let (mut lo, mut trimmed) = (0usize, 0.0);
            while lo < n[a] && gone(lo, trimmed) {
                trimmed += mass[a][lo];
                lo += 1;
            }
            if lo == n[a] {
                continue;
            }
            let (mut hi, mut trimmed) = (n[a] - 1, 0.0);
            while hi > lo && gone(hi, trimmed) {
                trimmed += mass[a][hi];
                hi -= 1;
            }
            let width = margin + reach[a];
            // A face is asked for room only if the air in the smoke nearest it is going toward it (the density-weighted
            // velocity along the axis in the slabs of the smoke within the margin of that end): a face behind a plume
            // that is rising away from it is no reason to move the window, which would take the room that the plume
            // rises into, and smoke at rest is going toward none.
            let going = |from: usize, to: usize| flux[a][from..=to].iter().sum::<f64>();
            let toward_low = going(lo, (lo + width).min(hi)) < 0.0;
            let toward_high = going(hi.saturating_sub(width).max(lo), hi) > 0.0;
            let (n, lo, hi, margin) = (n[a] as i64, lo as i64, hi as i64, width as i64);
            // smoke nearer the low face than the margin: the window moves toward lower cells by what is short, at
            // most as far as the slabs at the high end that may be left behind
            let near_low = if toward_low { (margin - lo).max(0) } else { 0 };
            let near_high = if toward_high { (hi + margin + 1 - n).max(0) } else { 0 };
            if near_low > 0 && near_high > 0 {
                blocked[a] = true;
            } else if near_low > 0 {
                // the slabs left behind are the last ones: n - 1, n - 2, ...
                let allowed = self.keeping(a, near_low.min(n - 1 - hi), false, keep);
                by[a] = -allowed;
                blocked[a] = allowed < near_low;
            } else if near_high > 0 {
                let allowed = self.keeping(a, near_high.min(lo), true, keep);
                by[a] = allowed;
                blocked[a] = allowed < near_high;
            }
        }
        (by, blocked)
    }

    /// How many of the first `wanted` slabs normal to `axis`, counting from the low end if `from_low` and else from
    /// the high end, hold no cell whose centre is in a shape of `keep`: the window can leave that many behind.
    fn keeping(&self, axis: usize, wanted: i64, from_low: bool, keep: &[&Shape]) -> i64 {
        if keep.is_empty() {
            return wanted;
        }
        let n = self.cells;
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        for t in 0..wanted {
            let slab = if from_low { t as usize } else { n[axis] - 1 - t as usize };
            let held = (0..n[v]).any(|j| {
                (0..n[u]).any(|i| {
                    let mut cell = [0; 3];
                    (cell[axis], cell[u], cell[v]) = (slab, i, j);
                    let p = self.cell_world(index(cell, n));
                    keep.iter().any(|shape| shape.contains(p))
                })
            });
            if held {
                return t;
            }
        }
        wanted
    }

    /// This state with its window moved by `by` whole cells: the cell that was at `c + by` is at `c`, what
    /// was kept is bit for bit what it was, and what comes in is the background (no density, ambient
    /// temperature, air at rest). It is refused, and nothing changes, if a cell that would be left behind holds
    /// smoke: the window never lets go of any.
    fn shifted(&self, by: [i64; 3], exact: bool) -> Result<State, Error> {
        let n = self.cells.map(|n| n as i64);
        if by.iter().zip(&n).any(|(b, n)| b.abs() >= *n) {
            return Err(Error::Invalid("a window cannot move by its whole size or more"));
        }
        let count = self.density.len();
        let mut lost = 0.0;
        for k in 0..count {
            let c = coords(k, self.cells).map(|v| v as i64);
            // the cell that is left behind is the one whose new place `c - by` is outside the window
            let left = (0..3).any(|a| !(0..n[a]).contains(&(c[a] - by[a])));
            if left {
                if exact && self.active(k) {
                    return Err(Error::Invalid("the window cannot move: the cells it would leave behind hold smoke"));
                }
                lost += self.density[k];
            }
        }
        let shift = |src: &[f64], dims: [usize; 3], fill: f64| -> Vec<f64> {
            let size = dims.map(|d| d as i64);
            let mut out = vec![fill; src.len()];
            out.par_chunks_mut(dims[0]).enumerate().for_each(|(row, line)| {
                let (y, z) = ((row % dims[1]) as i64, (row / dims[1]) as i64);
                let (y, z) = (y + by[1], z + by[2]);
                if !(0..size[1]).contains(&y) || !(0..size[2]).contains(&z) {
                    return;
                }
                for (x, v) in line.iter_mut().enumerate() {
                    let x = x as i64 + by[0];
                    if (0..size[0]).contains(&x) {
                        *v = src[index([x as usize, y as usize, z as usize], dims)];
                    }
                }
            });
            out
        };
        let window: [i64; 3] = std::array::from_fn(|a| self.window[a] + by[a]);
        let origin: [f64; 3] = std::array::from_fn(|a| self.base[a] + window[a] as f64 * self.h);
        if (0..3).any(|a| !origin[a].is_finite() || origin[a] + 0.5 * self.h == origin[a]) {
            return Err(Error::Invalid("the window has gone so far that the origin cannot represent a voxel centre"));
        }
        Ok(State {
            cells: self.cells,
            origin,
            base: self.base,
            window,
            lost: self.lost + lost,
            h: self.h,
            ambient: self.ambient,
            boundary: self.boundary,
            density: shift(&self.density, self.cells, 0.0),
            temperature: shift(&self.temperature, self.cells, self.ambient),
            velocity: std::array::from_fn(|a| shift(&self.velocity[a], face_dims(self.cells, a), 0.0)),
            solid: vec![false; count],
        })
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

    /// Working state of a step: the evolving fields copied from `self` and a fresh
    /// (empty) solid mask.
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
            base: self.base,
            window: self.window,
            lost: self.lost,
            h: self.h,
            ambient: self.ambient,
            boundary: self.boundary,
            density: copy(&self.density),
            temperature: copy(&self.temperature),
            velocity: std::array::from_fn(|a| copy(&self.velocity[a])),
            solid: vec![false; count],
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
        // The five channels are independent; results are inserted in channel order,
        // so the first error is the one a serial export would report.
        let grids: Vec<Result<SparseGrid, Error>> =
            (0..5).into_par_iter().map(|channel| self.export_channel(channel, transform, bricks)).collect();
        let mut volume = Volume::new();
        for (name, grid) in ["density", "temperature", "velocity.x", "velocity.y", "velocity.z"].iter().zip(grids) {
            volume.insert(name, grid?)?;
        }
        Ok(volume)
    }

    /// One exported channel: 0 density, 1 temperature, 2..=4 velocity components.
    /// Each 8x8x8 brick is filled from the state in parallel and stored whole; a
    /// brick whose samples all equal the background is not stored, and voxels
    /// outside the domain keep the background.
    fn export_channel(&self, channel: usize, transform: Transform, bricks: usize) -> Result<SparseGrid, Error> {
        let bg = if channel == 1 { self.ambient as f32 } else { 0.0 };
        let mut grid = SparseGrid::new(transform, bg, bricks)?;
        let per_axis = self.cells.map(|n| n.div_ceil(8));
        let field = self.field();
        let sample = |k: usize| -> f64 {
            match channel {
                0 => self.density[k],
                1 => self.temperature[k],
                _ => {
                    // `velocity_at(cell_world(k))[axis]` without the two unused components.
                    let p = self.cell_world(k);
                    if finite3(p) {
                        field.velocity_axis(self.grid(p), channel - 2)
                    } else {
                        0.0
                    }
                }
            }
        };
        let filled: Vec<Option<ExportedBrick>> = (0..bricks)
            .into_par_iter()
            .with_min_len(4)
            .map(|b| -> Result<_, Error> {
                let brick = coords(b, per_axis);
                let mut values = Box::new([bg; 512]);
                let mut stored = false;
                for z in 0..8usize.min(self.cells[2] - brick[2] * 8) {
                    for y in 0..8usize.min(self.cells[1] - brick[1] * 8) {
                        for x in 0..8usize.min(self.cells[0] - brick[0] * 8) {
                            let k = index([brick[0] * 8 + x, brick[1] * 8 + y, brick[2] * 8 + z], self.cells);
                            let value = sample(k) as f32;
                            if !value.is_finite() {
                                return Err(Error::Invalid("exported field exceeds f32 representation"));
                            }
                            values[x + 8 * y + 64 * z] = value;
                            stored |= value != bg;
                        }
                    }
                }
                Ok(stored.then(|| (brick.map(|v| v as i32), values)))
            })
            .collect::<Result<_, Error>>()?;
        for (key, values) in filled.into_iter().flatten() {
            grid.set_brick(key, &values)?;
        }
        Ok(grid)
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
        // p is within [0, cells), so truncation is the floor.
        self.solid[index(p.map(|v| v as usize), self.cells)]
    }

    fn trace(&self, p: [f64; 3], dt: f64) -> Result<[f64; 3], Error> {
        self.trace_from(p, self.velocity_grid(p), dt).map(|(end, _)| end)
    }

    /// Midpoint trace of `p` back by `dt` (forward when negative), given the
    /// velocity `v` at `p`. The flag reports a trace cut short by a collider.
    fn trace_from(&self, p: [f64; 3], v: [f64; 3], dt: f64) -> Result<([f64; 3], bool), Error> {
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
                return Ok((previous, true));
            }
            previous = q;
        }
        Ok((end, false))
    }
}

/// `v.floor() as i64` for finite `v` of moderate size, without a libm call (the
/// baseline x86-64 target has no rounding instruction). Truncation, then one step
/// down when it rounded up, is exactly the floor.
fn floor_i64(v: f64) -> i64 {
    let truncated = v as i64;
    truncated - i64::from((truncated as f64) > v)
}

/// Trilinear sample of a cell- or face-centred array. The result is the left to
/// right sum, starting from +0.0, of `weight * value` over the eight corners in
/// x-fastest order, with `weight = (wx * wy) * wz`; corners outside the array
/// contribute `background`. The in-range path below evaluates exactly that
/// expression without per-corner range checks.
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
    let lo = p.map(floor_i64);
    let f: [f64; 3] = std::array::from_fn(|a| p[a] - lo[a] as f64);
    let w: [[f64; 2]; 3] = std::array::from_fn(|a| [1.0 - f[a], f[a]]);
    if (0..3).all(|a| lo[a] >= 0 && lo[a] + 1 < dims[a] as i64) {
        let (sy, sz) = (dims[0], dims[0] * dims[1]);
        let base = (lo[2] as usize * dims[1] + lo[1] as usize) * dims[0] + lo[0] as usize;
        let corner = |bit: usize| values[base + (bit & 1) + ((bit >> 1) & 1) * sy + ((bit >> 2) & 1) * sz];
        let weight = |bit: usize| (w[0][bit & 1] * w[1][(bit >> 1) & 1]) * w[2][(bit >> 2) & 1];
        let mut result = 0.0;
        result += weight(0) * corner(0);
        result += weight(1) * corner(1);
        result += weight(2) * corner(2);
        result += weight(3) * corner(3);
        result += weight(4) * corner(4);
        result += weight(5) * corner(5);
        result += weight(6) * corner(6);
        result += weight(7) * corner(7);
        return result;
    }
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
    /// The flow that the blasts of the last step made (the velocity of the faces, by axis), if there were any: not part of the state.
    blast_flow: Option<[Vec<f64>; 3]>,
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
        if let Some(Follow { margin, loss }) = s.follow {
            if !(0.0..=1.0).contains(&loss) {
                return Err(Error::Invalid("the loss of a following window is a share between 0 and 1"));
            }
            if s.boundary != Boundary::Open {
                return Err(Error::Invalid("a window that follows its plume needs an open domain"));
            }
            if margin == 0 || s.cells.iter().any(|&n| 2 * margin >= n) {
                return Err(Error::Invalid(
                    "the margin of a following window must be at least one cell and leave a cell between its faces",
                ));
            }
        }
        for a in 0..3 {
            let hi = s.origin[a] + s.voxel_size * s.cells[a] as f64;
            if !hi.is_finite() || hi <= s.origin[a] || s.origin[a] + 0.5 * s.voxel_size == s.origin[a] {
                return Err(Error::Invalid("domain coordinates cannot represent voxel centres"));
            }
        }
        let count =
            s.cells.iter().try_fold(1usize, |v, n| v.checked_mul(*n)).ok_or(Error::Limit("cell count overflow"))?;
        // Worst-case live bytes per cell during one step, with every page touched:
        // committed state 41 (density 8, temperature 8, velocity ~24, solid 1) +
        // working state 41 + solid-face list 56 (only if every cell were solid; it
        // is built before the pressure workspace exists) + pressure workspace 65
        // (six CG vectors, open-face mask, divergence target) + ~26 for the
        // multigrid hierarchy, level vectors and component labels = ~229, plus ~25%
        // for allocator overhead and transient volume metadata.
        let bytes =
            count.checked_mul(288).and_then(|v| v.checked_add(8192)).ok_or(Error::Limit("grid memory overflow"))?;
        if bytes > s.max_bytes {
            return Err(Error::Limit("grid and step workspace memory budget"));
        }
        let state = State {
            cells: s.cells,
            origin: s.origin,
            base: s.origin,
            window: [0; 3],
            lost: 0.0,
            h: s.voxel_size,
            ambient: s.ambient_temperature,
            boundary: s.boundary,
            density: vec![0.0; count],
            temperature: vec![s.ambient_temperature; count],
            velocity: std::array::from_fn(|a| vec![0.0; face_dims(s.cells, a).iter().product()]),
            solid: vec![false; count],
        };
        Ok(Self { spec, state, step: 0, blast_flow: None })
    }

    /// The velocity of the faces, by axis, of the flow that the blasts of the last step made (none if there was none). It is the flow that carried the smoke
    /// in that step, and is not kept in the state: the velocity of the state is what it would have been with no blast.
    pub fn blast_flow(&self, axis: usize) -> Option<&[f64]> {
        self.blast_flow.as_ref().map(|f| f[axis].as_slice())
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// Move the window of the domain by `by` whole cells, toward the side the cells are numbered up to: the cell
    /// that was at `c + by` is at `c`. What the window keeps is what it had to the bit and what it takes in is
    /// the background of an open face. A move that would leave smoke behind is an error and changes nothing.
    pub fn shift_window(&mut self, by: [i64; 3]) -> Result<(), Error> {
        self.state = self.state.shifted(by, true)?;
        Ok(())
    }
    /// Move the window as far as the smoke of the state asks for, if the spec follows its plume (see
    /// [`State::follow_decision`]); the whole cells it moved by, none if there is nothing to do. It is a function
    /// of the state and of the sources that act in the step (`input`, sampled for the window as it is), so a replay
    /// from a checkpoint moves where the first run did. A caller that samples inputs for the step calls it first
    /// and samples them again if it moved, so that they are taken for the window the step will have.
    pub fn follow(&mut self, input: &Inputs) -> Result<[i64; 3], Error> {
        let Some(Follow { margin, loss }) = self.spec.follow else { return Ok([0; 3]) };
        let keep = input.acting(self.time(), (self.step + 1) as f64 * self.spec.dt, self.step, self.spec.dt);
        let (by, _) = self.state.follow_decision(margin, loss, self.spec.dt, &keep);
        if by != [0; 3] {
            self.state = self.state.shifted(by, false)?;
        }
        Ok(by)
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
        let (solid, solids) = voxelize(state.cells, state.origin, state.h, &input.obstacles)?;
        state.solid = solid;
        for cell in &solids {
            state.density[cell.cell] = 0.0;
            state.temperature[cell.cell] = state.ambient;
        }
        profile.obstacles = lap(&mut clock);
        boundaries(&mut state, &solids);
        validate_state(&state)?;
        profile.boundaries_validate += lap(&mut clock);
        advect(&mut state, dt, &self.spec, &solids, &mut profile)?;
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
                Injection {
                    density: source.density_rate * overlap,
                    temperature: source.temperature_rate * overlap,
                    velocity: source.velocity_rate.map(|v| v * overlap),
                    expansion: source.expansion * (overlap / dt),
                    heated: None,
                },
            );
        }
        for source in &input.heated {
            let overlap = (end.min(source.end.unwrap_or(end)) - start.max(source.start)).max(0.0);
            if overlap == 0.0 {
                continue;
            }
            inject(
                &mut state,
                &mut target,
                &source.shape,
                Injection {
                    density: source.density_rate * overlap,
                    temperature: source.temperature_rate * overlap,
                    velocity: source.velocity_rate.map(|v| v * overlap),
                    expansion: 0.0,
                    heated: Some(dt),
                },
            );
        }
        for (impulse, heated) in
            input.impulses.iter().map(|i| (i, None)).chain(input.heated_impulses.iter().map(|i| (i, Some(dt))))
        {
            let ratio = impulse.time / dt;
            if ratio.is_finite() && fixed_step_index(ratio) == self.step {
                inject(
                    &mut state,
                    &mut target,
                    &impulse.shape,
                    Injection {
                        density: impulse.density,
                        temperature: impulse.temperature,
                        velocity: impulse.velocity,
                        expansion: impulse.expansion / dt,
                        heated,
                    },
                );
            }
        }
        // the blasts that sweep air in this step: their flow is made apart from the smoke's, after it (see `blast_flow`)
        let mut pulses = Vec::new();
        for blast in &input.blasts {
            if self.spec.boundary != Boundary::Open {
                return Err(Error::Invalid("a blast needs an open domain"));
            }
            if let Some((radius, volume)) = blast.swept(dt, self.step) {
                pulses.push((blast.center, radius.max(self.spec.voxel_size), volume));
            }
        }
        profile.inject = lap(&mut clock);
        forces(&mut state, &self.spec, input, self.step);
        profile.forces = lap(&mut clock);
        boundaries(&mut state, &solids);
        validate_state(&state)?;
        if target.iter().any(|v| !v.is_finite()) {
            return Err(Error::Invalid("nonfinite expansion"));
        }
        profile.boundaries_validate += lap(&mut clock);
        let report = project(&mut state, &target, &self.spec, &mut profile)?;
        clock = Instant::now();
        validate_state(&state)?;
        profile.boundaries_validate += lap(&mut clock);
        // The flow of the blasts is made apart: the analytic flow of each piston projected alone (the projection is linear, so this is the share of
        // the step's flow that the blasts make), and it carries the smoke (the density and the temperature) once, over this step. It is not kept in the
        // velocity: a potential flow with open faces is not removed by a projection with no divergence, so it would stay for ever, and the smoke's own
        // velocity is therefore what it would have been without the blast, to the bit.
        let mut blast_flow = None;
        if !pulses.is_empty() {
            let mut flow = state.clone();
            for axis in &mut flow.velocity {
                axis.iter_mut().for_each(|v| *v = 0.0);
            }
            let mut flow_target = vec![0.0; flow.density.len()];
            for (center, reach, volume) in &pulses {
                inject_piston(&mut flow, &mut flow_target, *center, *reach, *volume, dt);
            }
            boundaries(&mut flow, &solids);
            project(&mut flow, &flow_target, &self.spec, &mut profile)?;
            validate_state(&flow)?;
            let mut carried = flow.clone();
            carried.density = state.density.clone();
            carried.temperature = state.temperature.clone();
            advect(&mut carried, dt, &self.spec, &solids, &mut profile)?;
            state.density = carried.density;
            state.temperature = carried.temperature;
            validate_state(&state)?;
            blast_flow = Some(flow.velocity);
        }
        self.blast_flow = blast_flow;
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

fn boundaries(s: &mut State, solids: &[SolidFaces]) {
    let (cells, boundary) = (s.cells, s.boundary);
    let solid = &s.solid;
    let faces_of =
        |c: usize| &solids[solids.binary_search_by_key(&c, |f| f.cell).expect("a solid cell has face velocities")];
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
                        let faces = faces_of(c);
                        value += if side == 0 { faces.high[a] } else { faces.low[a] };
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
fn advect(s: &mut State, dt: f64, spec: &Spec, solids: &[SolidFaces], profile: &mut StepProfile) -> Result<(), Error> {
    let (dissipation, cooling) = (spec.dissipation, spec.cooling);
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
    if spec.advection == Advection::MacCormack {
        let scalars =
            maccormack::Scalars { density: &old_density, temperature: &old_temperature, ambient, decay, cool };
        maccormack::advect(&old, scalars, (&mut s.density, &mut s.temperature, &mut s.velocity), dt)?;
        boundaries(s, solids);
        return Ok(());
    }
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
    boundaries(s, solids);
    Ok(())
}

/// A number for the cell of space that cell `c` of a window moved by `window` cells is. A cell inside the box that the domain began
/// as has the number it has without a follow (its index in that box), so that a window that has not moved, or has moved and
/// looks at cells of the first box, has the noise that the domain had; a cell outside it has a number of its own, the high bit set
/// and 21 bits of each of its three coordinates, and no two cells share one.
fn global_cell(c: [usize; 3], window: [i64; 3], cells: [usize; 3]) -> u64 {
    let g: [i64; 3] = std::array::from_fn(|a| c[a] as i64 + window[a]);
    if (0..3).all(|a| (0..cells[a] as i64).contains(&g[a])) {
        return index(g.map(|v| v as usize), cells) as u64;
    }
    let part = |a: usize| ((g[a] + (1 << 20)) as u64) & 0x1f_ffff;
    1 << 63 | part(0) | part(1) << 21 | part(2) << 42
}

fn forces(s: &mut State, spec: &Spec, input: &Inputs, step: u64) {
    let count = s.density.len();
    let mut force = vec![[0.0; 3]; count];
    {
        let (temperature, ambient) = (&s.temperature, s.ambient);
        let (cells, window, follows) = (s.cells, s.window, spec.follow.is_some());
        force.par_iter_mut().enumerate().with_min_len(HEAVY).for_each(|(k, f)| {
            // the noise of a window that moves belongs to the cell of space, not to the cell of the window
            let key = if follows { global_cell(coords(k, cells), window, cells) } else { k as u64 };
            for a in 0..3 {
                f[a] = input.acceleration[a]
                    + input.spatial_acceleration.get(k).map_or(0.0, |v| v[a])
                    + spec.turbulence * crate::rng::signed(spec.seed, key, step * 3 + a as u64);
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

/// The vectors of a conjugate-gradient solve: the pressure, the residual, the preconditioned residual, the search
/// direction and the operator applied to it.
struct Vectors<'a> {
    pressure: &'a mut [f64],
    residual: &'a mut [f64],
    z: &'a mut [f64],
    direction: &'a mut [f64],
    applied: &'a mut [f64],
}

/// The state a conjugate-gradient loop starts from and what it stops on.
struct Cg<'a> {
    tolerance: f64,
    iterations: usize,
    /// The inner product of the residual with the preconditioned residual, and the norm of the residual.
    rz: f64,
    current: f64,
    from_squares: &'a dyn Fn(f64) -> f64,
}

/// The preconditioned conjugate-gradient loop of the pressure solve, the same for both preconditioners: what differs
/// is how the inner product of two vectors is summed (`dot`) and how a step updates the pressure and the residual,
/// preconditions it and sums the two reductions it needs (`step`, which returns `r.z` and `r.r`), each of which keeps
/// the order of its own sums so that the bits of the result are those the solver has always had. The error is the residual the loop stopped on without converging (or at a non-positive curvature).
fn conjugate_gradient<S>(
    cg: Cg<'_>,
    vectors: Vectors<'_>,
    apply: &dyn Fn(&[f64], &mut [f64]),
    dot: &dyn Fn(&[f64], &[f64]) -> f64,
    mut step: S,
    (profile, clock): (&mut StepProfile, &mut Instant),
) -> Result<(f64, usize), f64>
where
    S: FnMut(f64, &mut [f64], &mut [f64], &mut [f64], &[f64], &mut [f64], &mut StepProfile, &mut Instant) -> (f64, f64),
{
    let Vectors { pressure, residual, z, direction, applied } = vectors;
    let Cg { tolerance, iterations, mut rz, mut current, from_squares } = cg;
    let mut used = 0;
    loop {
        profile.project_reduce += lap(clock);
        if !(current > tolerance && used < iterations) {
            break;
        }
        apply(direction, applied);
        profile.project_apply += lap(clock);
        let denom = dot(direction, applied);
        profile.project_reduce += lap(clock);
        if !denom.is_finite() || denom <= 0.0 || !rz.is_finite() {
            return Err(current);
        }
        let alpha = rz / denom;
        let (next, squares) = step(alpha, pressure, residual, z, direction, applied, profile, clock);
        current = from_squares(squares);
        let beta = next / rz;
        direction.par_iter_mut().zip(z.par_iter()).with_min_len(LIGHT).for_each(|(d, z)| *d = z + beta * *d);
        profile.project_update += lap(clock);
        rz = next;
        used += 1;
    }
    if current > tolerance {
        return Err(current);
    }
    Ok((current, used))
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
            let rz = inner(&residual, &z);
            // Both folds in `step` start from `Iterator::sum`'s identity and add in index
            // order, exactly like `inner`/`rms`; fusing them only shares the pass over
            // memory, since the two accumulation chains are independent.
            let current = rms(&residual);
            profile.project_setup = lap(&mut clock);
            let identity = std::iter::empty::<f64>().sum::<f64>();
            let step = |alpha: f64,
                        pressure: &mut [f64],
                        residual: &mut [f64],
                        z: &mut [f64],
                        direction: &[f64],
                        applied: &mut [f64],
                        profile: &mut StepProfile,
                        clock: &mut Instant| {
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
                profile.project_update += lap(clock);
                let (mut next, mut squares) = (identity, identity);
                for (r, z) in residual.iter().zip(z.iter()) {
                    next += r * z;
                    squares += r * r;
                }
                profile.project_reduce += lap(clock);
                (next, squares)
            };
            let (_, used) = conjugate_gradient(
                Cg { tolerance, iterations, rz, current, from_squares: &from_squares },
                Vectors {
                    pressure: &mut pressure,
                    residual: &mut residual,
                    z: &mut z,
                    direction: &mut direction,
                    applied: &mut applied,
                },
                &apply,
                &inner,
                step,
                (profile, &mut clock),
            )
            .map_err(|current| Error::Pressure { residual: current, worst: worst_divergence(s, target) })?;
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
            let current = from_squares(squares);
            profile.project_reduce += lap(&mut clock);
            // A residual already within tolerance needs no preconditioner pass.
            let (mut direction, rz) = if current > tolerance {
                hierarchy.precondition(&residual, &mut z, &mut applied, &mut scratch);
                profile.project_precondition += lap(&mut clock);
                let [rz] = multigrid::blocked_sums(count, |k| [residual[k] * z[k]]);
                profile.project_reduce += lap(&mut clock);
                (z.clone(), rz)
            } else {
                (Vec::new(), 0.0)
            };
            let step = |alpha: f64,
                        pressure: &mut [f64],
                        residual: &mut [f64],
                        z: &mut [f64],
                        direction: &[f64],
                        applied: &mut [f64],
                        profile: &mut StepProfile,
                        clock: &mut Instant| {
                pressure.par_iter_mut().zip(residual.par_iter_mut()).enumerate().with_min_len(LIGHT).for_each(
                    |(k, (p, r))| {
                        *p += alpha * direction[k];
                        *r -= alpha * applied[k];
                    },
                );
                profile.project_update += lap(clock);
                // the preconditioner uses `applied` as scratch: the operator rewrites it before it is read
                hierarchy.precondition(residual, z, applied, &mut scratch);
                profile.project_precondition += lap(clock);
                let [next, squares] =
                    multigrid::blocked_sums(count, |k| [residual[k] * z[k], residual[k] * residual[k]]);
                profile.project_reduce += lap(clock);
                (next, squares)
            };
            let dot = |a: &[f64], b: &[f64]| multigrid::blocked_sums(count, |k| [a[k] * b[k]])[0];
            let (_, used) = conjugate_gradient(
                Cg { tolerance, iterations, rz, current, from_squares: &from_squares },
                Vectors {
                    pressure: &mut pressure,
                    residual: &mut residual,
                    z: &mut z,
                    direction: &mut direction,
                    applied: &mut applied,
                },
                &apply,
                &dot,
                step,
                (profile, &mut clock),
            )
            .map_err(|current| Error::Pressure { residual: current, worst: worst_divergence(s, target) })?;
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
        return Err(Error::Pressure { residual: after, worst: worst_divergence(s, target) });
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
    revision: u64,
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
        Ok(Self { simulation, revision: 0, checkpoints, every, checkpoint_budget: checkpoint_bytes, state_bytes })
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
        self.seek(time, input)?;
        Ok(self.simulation.state())
    }

    /// Like `at_with_state`, also returning the state's revision. The revision
    /// changes whenever the timeline replaces its state, by a step or by a restore
    /// from a checkpoint, even when the replacement lands on the same step index as
    /// before. Two calls that return equal revisions therefore returned the very
    /// same state, and anything derived from it can be reused.
    pub fn at_with_revision(
        &mut self,
        time: f64,
        input: &mut impl FnMut(u64, f64, &State) -> Result<Inputs, Error>,
    ) -> Result<(&State, u64), Error> {
        self.seek(time, input)?;
        Ok((self.simulation.state(), self.revision))
    }

    fn seek(
        &mut self,
        time: f64,
        input: &mut impl FnMut(u64, f64, &State) -> Result<Inputs, Error>,
    ) -> Result<(), Error> {
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
            self.revision += 1;
        } else if let Some((&step, state)) = self.checkpoints.range(self.simulation.step..=target).next_back() {
            if step > self.simulation.step {
                self.simulation.state = state.clone();
                self.simulation.step = step;
                self.revision += 1;
            }
        }
        while self.simulation.step < target {
            let mut value = input(self.simulation.step, self.simulation.time(), self.simulation.state())?;
            if self.simulation.follow(&value)? != [0; 3] {
                // the inputs of a step are those of the window it has
                value = input(self.simulation.step, self.simulation.time(), self.simulation.state())?;
            }
            self.simulation.step(&value)?;
            self.revision += 1;
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
        Ok(())
    }
}

/// The fixed step a time at `ratio` steps from the start falls at, correcting only the roundoff at exact
/// boundaries, as [`Timeline`] does when it seeks.
pub fn fixed_step_index(ratio: f64) -> u64 {
    let nearest = ratio.round();
    (if (ratio - nearest).abs() <= f64::EPSILON * 4.0 * ratio.max(1.0) { nearest } else { ratio.floor() }) as u64
}

/// What a source or impulse adds to the cells it covers in one step.
struct Injection {
    density: f64,
    temperature: f64,
    velocity: [f64; 3],
    /// Divergence to project to, 1/second.
    expansion: f64,
    /// For a source whose expansion follows from its heat: the step, seconds.
    heated: Option<f64>,
}

fn inject(state: &mut State, target: &mut [f64], shape: &Shape, add: Injection) {
    let Injection { density, temperature, velocity, expansion, heated } = add;
    let (cells, origin, h) = (state.cells, state.origin, state.h);
    let State { density: cell_density, temperature: cell_temperature, velocity: faces, solid, .. } = state;
    let solid = &*solid;
    // what a heated source gives is a total, so that a solid in its way does not take any of it
    let density = if heated.is_some() {
        let free = (0..solid.len())
            .into_par_iter()
            .with_min_len(HEAVY)
            .filter(|&k| !solid[k] && shape.contains(world_point(origin, h, coords(k, cells).map(|v| v as f64 + 0.5))))
            .count();
        if free == 0 {
            0.0
        } else {
            density / free as f64
        }
    } else {
        density
    };
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
                // an ideal gas heated at constant pressure: dT over a step of dt is a divergence of dT / (T dt)
                if let Some(dt) = heated {
                    if temperature > 0.0 && *t > 0.0 {
                        *target_value += temperature / (*t * dt);
                    }
                }
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

/// Puts the flow of a spherical piston that has swept `volume` (cubic scene units) in a step of `dt` seconds, as the sphere of radius `reach` about
/// `center`: the cells whose centres are in the sphere and that are not solid are given the divergence that makes the volume that they let out the
/// volume swept (`volume / (cells * h^3 * dt)`, 1/second), and the faces of the whole domain are given the analytic velocity of the piston, inside
/// `d (x - c) / 3` and outside `Q / (4 pi r^2)` along `x - c` (`Q = volume / dt`), so that the projection that follows has only the faces and the
/// discretization to correct and pushes from the centre of the blast wherever the window is.
fn inject_piston(state: &mut State, target: &mut [f64], center: [f64; 3], reach: f64, volume: f64, dt: f64) {
    let (cells, origin, h) = (state.cells, state.origin, state.h);
    let State { velocity: faces, solid, .. } = state;
    let solid = &*solid;
    let inside = |p: [f64; 3]| (0..3).map(|a| (p[a] - center[a]).powi(2)).sum::<f64>() <= reach * reach;
    let in_sphere = |k: usize| inside(world_point(origin, h, coords(k, cells).map(|v| v as f64 + 0.5)));
    let in_window = (0..solid.len()).into_par_iter().with_min_len(HEAVY).filter(|&k| in_sphere(k)).count();
    let covered = (0..solid.len()).into_par_iter().with_min_len(HEAVY).filter(|&k| !solid[k] && in_sphere(k)).count();
    if covered == 0 {
        return;
    }
    // the cells that the volume is shared among are the sphere's whole: a window that the sphere cuts holds only some of them, and the cells of the
    // sphere that it does not hold take their share (so that the divergence in the window is what the sphere makes in free space, and agrees with the flow
    // outside). A sphere that is inside the window has its cells counted here (the same number); the solid ones take none.
    let whole = (0..3).all(|a| center[a] - reach >= origin[a] && center[a] + reach <= origin[a] + cells[a] as f64 * h);
    let shared = if whole {
        covered
    } else {
        lattice_count(center, origin, h, reach).saturating_sub(in_window - covered).max(covered)
    };
    let rate = volume / dt;
    let divergence = rate / (shared as f64 * h * h * h);
    target.par_iter_mut().enumerate().with_min_len(HEAVY).for_each(|(k, t)| {
        if !solid[k] && in_sphere(k) {
            *t += divergence;
        }
    });
    let outside = rate / (4.0 * std::f64::consts::PI);
    for (a, axis) in faces.iter_mut().enumerate() {
        let dims = face_dims(cells, a);
        axis.par_iter_mut().enumerate().with_min_len(HEAVY).for_each(|(k, v)| {
            let cell = coords(k, dims);
            let p = world_point(origin, h, std::array::from_fn(|i| cell[i] as f64 + if i == a { 0.0 } else { 0.5 }));
            let d: [f64; 3] = std::array::from_fn(|i| p[i] - center[i]);
            let r2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            if r2 <= reach * reach {
                *v += divergence / 3.0 * d[a];
            } else {
                *v += outside / (r2 * r2.sqrt()) * d[a];
            }
        });
    }
}

/// The number of cell centres of the infinite lattice (`origin` plus half a cell, every `h`) in the sphere, by columns (a cost of the square of the
/// radius in cells; a sphere of more than 30000 cells across is its volume over `h^3`).
fn lattice_count(center: [f64; 3], origin: [f64; 3], h: f64, reach: f64) -> usize {
    if 2.0 * reach / h > 30_000.0 {
        return (4.0 / 3.0 * std::f64::consts::PI * (reach / h).powi(3)) as usize;
    }
    let index = |v: f64, a: usize| (v - origin[a]) / h - 0.5;
    let (lo_x, hi_x) = (index(center[0] - reach, 0).ceil() as i64, index(center[0] + reach, 0).floor() as i64);
    let (lo_y, hi_y) = (index(center[1] - reach, 1).ceil() as i64, index(center[1] + reach, 1).floor() as i64);
    let mut total = 0i64;
    for i in lo_x..=hi_x {
        let dx = origin[0] + (i as f64 + 0.5) * h - center[0];
        for j in lo_y..=hi_y {
            let dy = origin[1] + (j as f64 + 0.5) * h - center[1];
            let rest = reach * reach - dx * dx - dy * dy;
            if rest < 0.0 {
                continue;
            }
            let half = rest.sqrt();
            let lo = index(center[2] - half, 2).ceil() as i64;
            let hi = index(center[2] + half, 2).floor() as i64;
            total += (hi - lo + 1).max(0);
        }
    }
    total as usize
}
