//! Bounded participating media and a deterministic reference transport integrator.
//!
//! Extinction follows Beer–Lambert transmittance. Emission is radiance per scene
//! unit at unit density; scattering uses the caller's phase-weighted incident light.
//! The midpoint quadrature integrates each constant-coefficient step analytically,
//! clips to the nearest surface, and combines overlapping media before integration.
//! It is single scattering, not a predictive radiation or fluid dynamics solver.

use std::sync::Arc;

use glam::DVec3;

use crate::{Error, SparseGrid, Transform};

/// A finite, nonempty domain in the volume asset's world coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    lo: [f64; 3],
    hi: [f64; 3],
}

impl Bounds {
    pub fn new(lo: [f64; 3], hi: [f64; 3]) -> Result<Self, Error> {
        if (0..3).any(|i| !lo[i].is_finite() || !hi[i].is_finite() || lo[i] >= hi[i]) {
            return Err(Error::Invalid("domain bounds must be finite with minimum < maximum"));
        }
        Ok(Self { lo, hi })
    }

    pub fn min(&self) -> [f64; 3] {
        self.lo
    }

    pub fn max(&self) -> [f64; 3] {
        self.hi
    }

    pub fn contains(&self, p: [f64; 3]) -> bool {
        (0..3).all(|i| p[i] >= self.lo[i] && p[i] <= self.hi[i])
    }

    fn intersect(&self, o: DVec3, d: DVec3, max: f64) -> Option<[f64; 2]> {
        let (mut near, mut far) = (0.0_f64, max);
        for i in 0..3 {
            if d[i] == 0.0 {
                if o[i] < self.lo[i] || o[i] > self.hi[i] {
                    return None;
                }
            } else {
                let a = (self.lo[i] - o[i]) / d[i];
                let b = (self.hi[i] - o[i]) / d[i];
                near = near.max(a.min(b));
                far = far.min(a.max(b));
            }
        }
        (far > near).then_some([near, far])
    }
}

/// Scalar extinction preserves exact RGBA compositing; albedo and emission are linear RGB.
#[derive(Clone, Copy, Debug)]
pub struct Optical {
    pub density_scale: f64,
    pub extinction: f64,
    pub albedo: [f64; 3],
    pub emission: [f64; 3],
    /// Cosine between incoming and outgoing propagation directions: positive is forward.
    pub anisotropy: f64,
}

impl Default for Optical {
    fn default() -> Self {
        Self { density_scale: 1.0, extinction: 1.0, albedo: [0.0; 3], emission: [0.0; 3], anisotropy: 0.0 }
    }
}

impl Optical {
    fn validate(&self) -> Result<(), Error> {
        if !self.density_scale.is_finite()
            || self.density_scale < 0.0
            || !self.extinction.is_finite()
            || self.extinction < 0.0
            || self.albedo.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || self.emission.iter().any(|v| !v.is_finite() || *v < 0.0)
            || !self.anisotropy.is_finite()
            || self.anisotropy.abs() >= 1.0
        {
            return Err(Error::Invalid(
                "medium coefficients must be finite, nonnegative; albedo in [0,1], anisotropy in (-1,1)",
            ));
        }
        Ok(())
    }
}

/// Immutable validated density and material, with an object-to-scene transform.
#[derive(Clone, Debug)]
pub struct Medium {
    density: Arc<SparseGrid>,
    bounds: Option<Bounds>,
    transform: Transform,
    optical: Optical,
    temperature: Option<Temperature>,
    authored_bounds: Option<Bounds>,
    next_density: Option<Arc<SparseGrid>>,
    frame_blend: f64,
    advection: Option<[crate::advection::Advection; 2]>,
}

/// Validated kelvin field, independent of the density grid's transform and resolution.
#[derive(Clone, Debug)]
pub struct Temperature {
    grid: Arc<SparseGrid>,
    scale: f64,
    emission_scale: f64,
    next_grid: Option<Arc<SparseGrid>>,
}

impl Temperature {
    pub fn grid(&self) -> &Arc<SparseGrid> {
        &self.grid
    }
    pub fn scale(&self) -> f64 {
        self.scale
    }
    pub fn emission_scale(&self) -> f64 {
        self.emission_scale
    }
    pub fn next_grid(&self) -> Option<&Arc<SparseGrid>> {
        self.next_grid.as_ref()
    }
}

impl Medium {
    /// A nonzero grid background requires an explicit finite domain. Otherwise bounds
    /// are inferred from nonzero samples plus the full trilinear interpolation support.
    pub fn new(
        density: Arc<SparseGrid>,
        bounds: Option<Bounds>,
        transform: Transform,
        optical: Optical,
    ) -> Result<Self, Error> {
        optical.validate()?;
        if density.background() < 0.0 {
            return Err(Error::Invalid("negative density background"));
        }
        let mut lo = DVec3::splat(f64::INFINITY);
        let mut hi = DVec3::splat(f64::NEG_INFINITY);
        for (key, values) in density.bricks() {
            for (i, v) in values.iter().enumerate() {
                if *v < 0.0 {
                    return Err(Error::Invalid("negative density sample"));
                }
                if *v == 0.0 {
                    continue;
                }
                let p = DVec3::new(
                    f64::from(key[0]) * 8.0 + (i % 8) as f64,
                    f64::from(key[1]) * 8.0 + (i / 8 % 8) as f64,
                    f64::from(key[2]) * 8.0 + (i / 64) as f64,
                );
                lo = lo.min(p - DVec3::ONE);
                hi = hi.max(p + DVec3::ONE);
            }
        }
        let authored_bounds = bounds;
        let bounds = if bounds.is_some() {
            bounds
        } else if density.background() != 0.0 {
            return Err(Error::Invalid("nonzero density background requires explicit domain bounds"));
        } else if lo.is_finite() {
            let mut world_lo = DVec3::splat(f64::INFINITY);
            let mut world_hi = DVec3::splat(f64::NEG_INFINITY);
            for k in 0..8 {
                let p = std::array::from_fn(|i| if k & (1 << i) == 0 { lo[i] } else { hi[i] });
                let p = DVec3::from(density.transform().index_to_world(p));
                world_lo = world_lo.min(p);
                world_hi = world_hi.max(p);
            }
            Some(Bounds::new(world_lo.to_array(), world_hi.to_array())?)
        } else {
            None
        };
        Ok(Self {
            density,
            bounds,
            transform,
            optical,
            temperature: None,
            authored_bounds,
            next_density: None,
            frame_blend: 0.0,
            advection: None,
        })
    }

    /// Adds thermal emission to the constant optical emission. The emission scale
    /// converts the normalized blackbody RGB to radiance per unit density and length.
    /// Temperature is interpolated in field space before converting to emission.
    pub fn with_temperature(mut self, grid: Arc<SparseGrid>, scale: f64, emission_scale: f64) -> Result<Self, Error> {
        if self.next_density.is_some() {
            return Err(Error::Invalid("set temperature before adding a second volume frame"));
        }
        if !scale.is_finite() || scale <= 0.0 || !emission_scale.is_finite() || emission_scale < 0.0 {
            return Err(Error::Invalid("temperature scale must be positive; emission scale nonnegative and finite"));
        }
        if std::iter::once(grid.background())
            .chain(grid.bricks().flat_map(|(_, v)| v.iter().copied()))
            .any(|v| !(0.0..=crate::thermal::MAX_KELVIN).contains(&(f64::from(v) * scale)))
        {
            return Err(Error::Invalid("scaled temperature samples must be in 0..50000 kelvin"));
        }
        self.temperature = Some(Temperature { grid, scale, emission_scale, next_grid: None });
        Ok(self)
    }

    /// Adds a second cache frame. Both scalar fields are sampled in their own
    /// asset-world space and blended before computing extinction or thermal emission.
    /// Material and object transform belong to this evaluated instant, not to frames.
    pub fn with_next_frame(
        mut self,
        density: Arc<SparseGrid>,
        temperature: Option<Arc<SparseGrid>>,
        blend: f64,
    ) -> Result<Self, Error> {
        if !blend.is_finite() || !(0.0..=1.0).contains(&blend) {
            return Err(Error::Invalid("volume frame blend must be finite and in [0,1]"));
        }
        if self.next_density.is_some() {
            return Err(Error::Invalid("a medium interpolates at most two volume frames"));
        }
        let mut next = Self::new(density.clone(), self.authored_bounds, self.transform, self.optical)?;
        match (&mut self.temperature, temperature) {
            (Some(t), Some(grid)) => {
                next = next.with_temperature(grid.clone(), t.scale, t.emission_scale)?;
                t.next_grid = Some(grid);
            }
            (None, None) => {}
            _ => return Err(Error::Invalid("both volume frames require the selected temperature channel")),
        }
        self.bounds = match (self.bounds, next.bounds) {
            (Some(a), Some(b)) => Some(Bounds::new(
                std::array::from_fn(|i| a.lo[i].min(b.lo[i])),
                std::array::from_fn(|i| a.hi[i].max(b.hi[i])),
            )?),
            (a, b) => a.or(b),
        };
        self.next_density = Some(density);
        self.frame_blend = blend;
        Ok(self)
    }

    pub fn next_density(&self) -> Option<&Arc<SparseGrid>> {
        self.next_density.as_ref()
    }

    /// Applies independent endpoint traces before interpolating density and kelvin.
    /// Add the second frame first. Authored bounds remain a clipping domain;
    /// inferred bounds expand conservatively to include advected field support.
    pub fn with_advection(
        mut self,
        first: crate::advection::Advection,
        second: crate::advection::Advection,
    ) -> Result<Self, Error> {
        if self.next_density.is_none() || self.advection.is_some() {
            return Err(Error::Invalid("advection requires a frame pair and may only be set once"));
        }
        if let Some(mut bounds) = self.bounds {
            if self.authored_bounds.is_none() {
                let travel: [f64; 3] = std::array::from_fn(|i| first.displacement()[i].max(second.displacement()[i]));
                bounds = Bounds::new(
                    std::array::from_fn(|i| bounds.lo[i] - travel[i]),
                    std::array::from_fn(|i| bounds.hi[i] + travel[i]),
                )?;
            }
            first.validate_domain(bounds.lo, bounds.hi)?;
            second.validate_domain(bounds.lo, bounds.hi)?;
            self.bounds = Some(bounds);
        }
        self.advection = Some([first, second]);
        Ok(self)
    }

    pub fn advection(&self) -> Option<&[crate::advection::Advection; 2]> {
        self.advection.as_ref()
    }
    pub fn frame_blend(&self) -> f64 {
        self.frame_blend
    }

    /// Density fields referenced by this instance, for upload and resource accounting.
    pub fn density_grids(&self) -> impl Iterator<Item = &Arc<SparseGrid>> {
        std::iter::once(&self.density).chain(self.next_density.as_ref())
    }

    fn sample_field(&self, first: &SparseGrid, next: Option<&Arc<SparseGrid>>, scene: [f64; 3]) -> f64 {
        let local = self.transform.world_to_index(scene);
        let points = self.advection.as_ref().map_or([local; 2], |traces| traces.each_ref().map(|t| t.backtrace(local)));
        let a = f64::from(first.sample_world(points[0]));
        next.map_or(a, |b| a * (1.0 - self.frame_blend) + f64::from(b.sample_world(points[1])) * self.frame_blend)
    }

    pub fn temperature(&self) -> Option<&Temperature> {
        self.temperature.as_ref()
    }

    /// Per-density emission; callers multiply by density and integrate in scene units.
    pub fn sample_emission(&self, scene: [f64; 3]) -> [f64; 3] {
        let mut emission = self.optical.emission;
        if let Some(t) = &self.temperature {
            let kelvin = self.sample_field(&t.grid, t.next_grid.as_ref(), scene) * t.scale;
            let rgb = crate::thermal::blackbody_rgb(kelvin.clamp(0.0, crate::thermal::MAX_KELVIN))
                .expect("validated temperature field");
            for c in 0..3 {
                emission[c] += rgb[c] * t.emission_scale;
            }
        }
        emission
    }

    pub fn density(&self) -> &Arc<SparseGrid> {
        &self.density
    }

    pub fn bounds(&self) -> Option<Bounds> {
        self.bounds
    }

    pub fn transform(&self) -> Transform {
        self.transform
    }

    pub fn optical(&self) -> Optical {
        self.optical
    }

    pub fn sample_density(&self, scene: [f64; 3]) -> f64 {
        let local = self.transform.world_to_index(scene);
        if !self.bounds.is_some_and(|b| b.contains(local)) {
            return 0.0;
        }
        self.sample_field(&self.density, self.next_density.as_ref(), scene) * self.optical.density_scale
    }

    /// Ray interval in scene-distance units, including nonuniform object scales.
    pub fn interval(&self, ray: Ray) -> Option<[f64; 2]> {
        let o = self.transform.inverse.transform_point3(ray.origin);
        let d = self.transform.inverse.transform_vector3(ray.direction);
        self.bounds?.intersect(o, d, ray.distance)
    }
}

/// A scene-space ray with a unit direction and a finite nearest-surface distance.
#[derive(Clone, Copy, Debug)]
pub struct Ray {
    origin: DVec3,
    direction: DVec3,
    distance: f64,
}

impl Ray {
    pub fn new(origin: [f64; 3], direction: [f64; 3], distance: f64) -> Result<Self, Error> {
        let origin = DVec3::from(origin);
        let direction = DVec3::from(direction);
        if !origin.is_finite() || !direction.is_finite() || !distance.is_finite() || distance < 0.0 {
            return Err(Error::Invalid("ray must be finite with nonnegative distance"));
        }
        let direction = direction.try_normalize().ok_or(Error::Invalid("ray direction must be nonzero"))?;
        Ok(Self { origin, direction, distance })
    }
}

/// Explicit accuracy and work budgets. Insufficient work budgets fail before sampling.
#[derive(Clone, Copy, Debug)]
pub struct March {
    pub step_size: f64,
    pub max_steps: u32,
}

impl Default for March {
    fn default() -> Self {
        Self { step_size: 0.5, max_steps: 4096 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transport {
    pub radiance: [f64; 3],
    pub transmittance: f64,
    pub steps: u32,
}

/// Normalized Henyey–Greenstein phase. Directions follow propagation, so g>0
/// peaks at cos(theta)=1. Callers supply finite cosine and |g|<1.
pub fn phase_hg(cos_theta: f64, g: f64) -> f64 {
    let denominator = 1.0 + g * g - 2.0 * g * cos_theta.clamp(-1.0, 1.0);
    (1.0 - g * g) / (4.0 * std::f64::consts::PI * denominator.powf(1.5))
}

/// Integrates emission, absorption and single scattering across all overlapping media.
/// `incident` returns phase-weighted, visibility-attenuated incident RGB at a point,
/// given the outgoing propagation direction (toward the viewer) and anisotropy.
pub fn integrate(
    media: &[Medium],
    ray: Ray,
    march: March,
    mut incident: impl FnMut([f64; 3], [f64; 3], f64) -> [f64; 3],
) -> Result<Transport, Error> {
    if !march.step_size.is_finite() || march.step_size <= 0.0 || march.max_steps == 0 {
        return Err(Error::Invalid("march requires positive finite step size and positive step budget"));
    }
    // Each domain boundary splits quadrature intervals, avoiding skipped thin layers
    // and expensive sampling of arbitrarily large empty gaps between media.
    let ranges: Vec<_> = media.iter().filter_map(|m| m.interval(ray)).collect();
    let mut edges: Vec<_> = ranges.iter().flatten().copied().collect();
    edges.sort_by(f64::total_cmp);
    edges.dedup();
    let mut segments = Vec::new();
    let mut required = 0u32;
    for edge in edges.windows(2) {
        let [a, b] = [edge[0], edge[1]];
        let middle = a + (b - a) * 0.5;
        if !ranges.iter().any(|r| middle >= r[0] && middle < r[1]) {
            continue;
        }
        let n = ((b - a) / march.step_size).ceil();
        if n > f64::from(march.max_steps - required) {
            return Err(Error::Limit("medium ray exceeds sample budget; increase maxSteps or stepSize"));
        }
        let n = (n as u32).max(1);
        required = required.checked_add(n).filter(|n| *n <= march.max_steps).ok_or(Error::Limit("medium samples"))?;
        segments.push((a, b, n));
    }
    let mut out = Transport { radiance: [0.0; 3], transmittance: 1.0, steps: required };
    for (a, b, n) in segments {
        let ds = (b - a) / f64::from(n);
        for k in 0..n {
            let point = (ray.origin + ray.direction * (a + (f64::from(k) + 0.5) * ds)).to_array();
            let mut extinction = 0.0;
            let mut source = [0.0; 3];
            for medium in media {
                let density = medium.sample_density(point);
                if density == 0.0 {
                    continue;
                }
                let optical = medium.optical;
                let sigma = density * optical.extinction;
                extinction += sigma;
                let light = if optical.albedo == [0.0; 3] {
                    [0.0; 3]
                } else {
                    incident(point, (-ray.direction).to_array(), optical.anisotropy)
                };
                if light.iter().any(|v| !v.is_finite() || *v < 0.0) {
                    return Err(Error::Invalid("incident radiance must be finite and nonnegative"));
                }
                let emission = medium.sample_emission(point);
                for c in 0..3 {
                    source[c] += density * emission[c] + sigma * optical.albedo[c] * light[c];
                }
            }
            if !extinction.is_finite() || source.iter().any(|v| !v.is_finite()) {
                return Err(Error::Invalid("medium coefficient overflow"));
            }
            let lost = -(-extinction * ds).exp_m1();
            let weight = out.transmittance * if extinction == 0.0 { ds } else { lost / extinction };
            for (dst, src) in out.radiance.iter_mut().zip(source) {
                *dst += weight * src;
            }
            out.transmittance *= (-extinction * ds).exp();
        }
    }
    if out.radiance.iter().any(|v| !v.is_finite()) {
        return Err(Error::Invalid("integrated radiance overflow"));
    }
    Ok(out)
}
