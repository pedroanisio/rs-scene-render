//! Authored crater growth in object-space scene units, shared by surfaces and colliders.
//!
//! The displacement is along a fixed outward axis. A compact bowl/rim profile
//! grows in radius, depth and rim dimensions with progress; an axial envelope
//! keeps the stamp from modifying a distant back surface. This is an authored
//! deformation, not a prediction of excavation from impact energy.
use crate::Vertex;
use glam::{DMat3, DMat4, DVec3};

#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub center: [f64; 3],
    /// Nonzero direction, normalized on construction.
    pub outward: [f64; 3],
    pub radius: f64,
    pub depth: f64,
    pub rim_height: f64,
    pub rim_width: f64,
    /// Axial half-width of the compact envelope. At least twice the larger of
    /// depth and rim height, ensuring an orientation-preserving deformation.
    pub influence_depth: f64,
}

/// What the volumes of a crater are to add up to, in cubic object units: the bowl excavates `volume`, the ejecta that
/// were thrown out come down outside the rim as a mantle of volume `ejecta`, and the rim holds the rest of what is
/// put back, up to `bulking` times the volume (a pile of broken rock takes more room than the rock did). Without a
/// `bulking` the rim keeps the height the spec gives it and the bulking is what that asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Budget {
    pub volume: f64,
    pub ejecta: f64,
    pub bulking: Option<f64>,
}

/// The volumes of a grown crater: what the bowl takes out and what the rim and the mantle put back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volumes {
    pub bowl: f64,
    pub rim: f64,
    pub mantle: f64,
}

/// The shape of the bowl and the mantle outside the rim that a crater of `Crater::conserving` has.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Profile {
    /// The bowl is `(1 - x^2)^p`; 2 for a crater of `Crater::new`.
    bowl_exponent: f64,
    mantle: Option<Mantle>,
}

/// A mantle of thickness `thickness (R / r)^3` outside the rim crest `R`, joined to the rim by a smooth ramp over
/// the rim's width and cut at `reach`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Mantle {
    thickness: f64,
    reach: f64,
    /// Cubic units that it holds.
    volume: f64,
}

/// How far, in crest radii, the mantle reaches before it is cut.
const MANTLE_REACH: f64 = 20.;

#[derive(Clone, Debug)]
pub struct Crater {
    spec: Spec,
    axis: DVec3,
    profile: Profile,
    deposit: Option<Deposit>,
}

/// Material that lies on the ground of a crater, as a height above the ground in the plane of the crater: the heights
/// of a regular grid of square cells, at the cells' centres, with none beyond the grid and bilinear weights between
/// the centres (so that the field is continuous, and is zero a half cell outside the grid).
#[derive(Clone, Debug, PartialEq)]
pub struct Deposit {
    origin: [f64; 2],
    cell: f64,
    cells: [usize; 2],
    heights: std::sync::Arc<[f64]>,
}

impl Deposit {
    /// `heights` is row by row (`cells[0]` to a row), the corner of the grid is at `origin` in the plane's axes
    /// ([`Crater::plane_basis`]) from the crater's centre. Heights are finite and not negative.
    pub fn new(origin: [f64; 2], cell: f64, cells: [usize; 2], heights: Vec<f64>) -> Result<Self, String> {
        if cells[0] == 0 || cells[1] == 0 || cells[0].checked_mul(cells[1]) != Some(heights.len()) {
            return Err("a deposit needs a height for each of its cells".into());
        }
        if !(cell.is_finite() && cell > 0.) || origin.iter().any(|v| !v.is_finite()) {
            return Err("a deposit needs a finite corner and cells of a positive size".into());
        }
        if heights.iter().any(|h| !h.is_finite() || *h < 0.) {
            return Err("the heights of a deposit are finite and not negative".into());
        }
        Ok(Self { origin, cell, cells, heights: heights.into() })
    }

    /// Cubic units that it holds: the integral of its height, which is the sum over its cells.
    pub fn volume(&self) -> f64 {
        self.heights.iter().sum::<f64>() * self.cell * self.cell
    }

    /// The height at plane coordinates `(a, b)` and its derivatives along them.
    pub fn height(&self, a: f64, b: f64) -> (f64, [f64; 2]) {
        let (fx, fz) = ((a - self.origin[0]) / self.cell - 0.5, (b - self.origin[1]) / self.cell - 0.5);
        let (ix, iz) = (fx.floor(), fz.floor());
        if ix < -1. || iz < -1. || ix >= self.cells[0] as f64 || iz >= self.cells[1] as f64 {
            return (0., [0.; 2]);
        }
        let (tx, tz) = (fx - ix, fz - iz);
        let at = |x: f64, z: f64| {
            if x < 0. || z < 0. || x >= self.cells[0] as f64 || z >= self.cells[1] as f64 {
                0.
            } else {
                self.heights[z as usize * self.cells[0] + x as usize]
            }
        };
        let (h00, h10, h01, h11) = (at(ix, iz), at(ix + 1., iz), at(ix, iz + 1.), at(ix + 1., iz + 1.));
        let height = (h00 * (1. - tx) + h10 * tx) * (1. - tz) + (h01 * (1. - tx) + h11 * tx) * tz;
        let da = ((h10 - h00) * (1. - tz) + (h11 - h01) * tz) / self.cell;
        let db = ((h01 - h00) * (1. - tx) + (h11 - h10) * tx) / self.cell;
        (height, [da, db])
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Mapping {
    pub position: [f64; 3],
    /// Differential of the position map. Normals use its inverse transpose;
    /// tangents and material displacements use it directly.
    pub jacobian: DMat3,
}

impl Crater {
    pub fn new(spec: Spec) -> Result<Self, String> {
        if spec
            .center
            .into_iter()
            .chain(spec.outward)
            .chain([spec.radius, spec.depth, spec.rim_height, spec.rim_width, spec.influence_depth])
            .any(|v| !v.is_finite())
            || spec.radius <= 0.
            || spec.depth < 0.
            || spec.rim_height < 0.
            || spec.rim_width <= 0.
            || spec.rim_width > spec.radius
            || spec.influence_depth <= 0.
            || spec.influence_depth * 0.5 < spec.depth.max(spec.rim_height)
        {
            return Err("invalid crater dimensions, envelope or center".into());
        }
        let axis = DVec3::from(spec.outward);
        let max = axis.abs().max_element();
        if max == 0. {
            return Err("crater outward direction must be nonzero".into());
        }
        // Normalize without squaring potentially enormous authored components.
        let axis = (axis / max).normalize();
        Ok(Self { spec, axis, profile: Profile { bowl_exponent: 2., mantle: None }, deposit: None })
    }

    /// A crater whose volumes add up (see [`Budget`]): the bowl is given the exponent that makes it excavate exactly
    /// `budget.volume` with the depth and the radius of the spec (a deeper bowl, with the exponent 2 that keeps the
    /// bump compact and smooth, when the volume is more than the bowl of that exponent holds), the rim has the volume that the bulking leaves
    /// after the mantle (or the height of the spec), and the mantle is scaled to hold exactly `budget.ejecta`.
    pub fn conserving(spec: Spec, budget: Budget) -> Result<Self, String> {
        let Budget { volume, ejecta, bulking } = budget;
        if !(volume.is_finite() && volume > 0. && ejecta.is_finite() && ejecta >= 0.)
            || bulking.is_some_and(|b| !b.is_finite())
        {
            return Err(
                "a crater's volume and ejecta must be finite, the volume positive and the ejecta not negative".into()
            );
        }
        let pi = std::f64::consts::PI;
        let mut spec = spec;
        // the volume of the bowl, pi R^2 d / (p + 1), is the one asked for
        let exponent = pi * spec.radius * spec.radius * spec.depth / volume - 1.;
        let exponent = if exponent.is_finite() && exponent >= 2. {
            exponent
        } else {
            spec.depth = 3. * volume / (pi * spec.radius * spec.radius);
            2.
        };
        let ring = 32. * pi / 15. * spec.rim_width * spec.radius;
        if let Some(b) = bulking {
            let rim = b * volume - ejecta;
            if rim < 0. {
                return Err("the bulking leaves no room for the rim after the mantle".into());
            }
            spec.rim_height = rim / ring;
        }
        let mut crater = Crater::new(spec)?;
        crater.profile.bowl_exponent = exponent;
        let reach = MANTLE_REACH * spec.radius;
        let unit = Mantle { thickness: 1., reach, volume: 0. }.volume_for(&crater);
        let mantle = Mantle { thickness: if ejecta > 0. { ejecta / unit } else { 0. }, reach, volume: ejecta };
        // the mantle sits on top of the rim: the envelope must still leave the map orientation-preserving
        if spec.influence_depth * 0.5 < spec.depth.max(spec.rim_height + mantle.thickness) {
            return Err("the influence depth of a crater with a mantle must be at least twice its depth and its rim and mantle together".into());
        }
        crater.profile.mantle = Some(mantle);
        Ok(crater)
    }

    /// The same crater with `deposit` lying on its ground: added to the height the crater gives, whatever its progress
    /// (it is what came down, not what grew).
    pub fn with_deposit(mut self, deposit: Deposit) -> Self {
        self.deposit = Some(deposit);
        self
    }

    pub fn deposit(&self) -> Option<&Deposit> {
        self.deposit.as_ref()
    }

    /// The two unit axes of the plane of the crater, perpendicular to its axis and to each other, from which the
    /// coordinates of a [`Deposit`] are taken: the first is the axis crossed with the coordinate axis it is least aligned
    /// with, and the second is the axis crossed with the first.
    pub fn plane_basis(&self) -> ([f64; 3], [f64; 3]) {
        let (u, v) = self.plane_axes();
        (u.to_array(), v.to_array())
    }

    fn plane_axes(&self) -> (DVec3, DVec3) {
        let a = self.axis.abs();
        let helper = if a.x <= a.y && a.x <= a.z {
            DVec3::X
        } else if a.y <= a.z {
            DVec3::Y
        } else {
            DVec3::Z
        };
        let u = self.axis.cross(helper).normalize();
        (u, self.axis.cross(u))
    }

    /// The volumes of the grown crater.
    pub fn volumes(&self) -> Volumes {
        let pi = std::f64::consts::PI;
        Volumes {
            bowl: pi * self.spec.radius * self.spec.radius * self.spec.depth / (self.profile.bowl_exponent + 1.),
            rim: 32. * pi / 15. * self.spec.rim_height * self.spec.rim_width * self.spec.radius,
            mantle: self.profile.mantle.map_or(0., |m| m.volume),
        }
    }

    /// The thickness of the mantle at the rim crest, once it is as big as the crater is (zero without one).
    pub fn mantle_thickness(&self) -> f64 {
        self.profile.mantle.map_or(0., |m| m.thickness)
    }

    /// The distance from the axis beyond which the grown crater moves nothing.
    pub fn reach(&self) -> f64 {
        self.profile.mantle.map_or(self.spec.radius + self.spec.rim_width, |m| m.reach)
    }

    /// The exponent `p` of the bowl, `(1 - x^2)^p`: 2 for a crater of [`Crater::new`], the one that makes the bowl hold the volume of the
    /// budget for [`Crater::conserving`].
    pub fn bowl_exponent(&self) -> f64 {
        self.profile.bowl_exponent
    }

    /// The unit vector the crater points along, out of the ground.
    pub fn axis(&self) -> [f64; 3] {
        self.axis.to_array()
    }

    /// How far under the original surface the floor of the grown bowl is at distance `r` from the axis: the depth at the centre, falling to
    /// nothing at the crest radius and beyond.
    pub fn bowl_depth_at(&self, r: f64) -> f64 {
        let x = r / self.spec.radius;
        if x.abs() >= 1. {
            0.
        } else {
            self.spec.depth * bowl(x, self.profile.bowl_exponent).0
        }
    }

    /// How far the grown rim stands over the original surface at distance `r` from the axis: highest at the crest radius, and nothing
    /// beyond the rim's width either side of it.
    pub fn rim_height_at(&self, r: f64) -> f64 {
        let x = (r - self.spec.radius) / self.spec.rim_width;
        if x.abs() >= 1. {
            0.
        } else {
            self.spec.rim_height * bump(x).0
        }
    }

    /// How far the grown mantle stands over the original surface at distance `r` from the axis: nothing for a crater without one, nothing
    /// under the crest radius's ramp or beyond its reach, and half its thickness at the crest radius.
    pub fn mantle_height_at(&self, r: f64) -> f64 {
        match self.profile.mantle {
            Some(m) if r < m.reach => m.thickness * mantle_shape(r, self.spec.radius, self.spec.rim_width).0,
            _ => 0.0,
        }
    }

    /// The dimensions this crater was made from; the axis is the one given, not normalised.
    pub fn spec(&self) -> Spec {
        self.spec
    }

    /// Progress is clamped to [0,1]. The map is deterministic and stateless.
    pub fn map(&self, point: [f64; 3], progress: f64) -> Result<Mapping, String> {
        if !progress.is_finite() || point.iter().any(|v| !v.is_finite()) {
            return Err("nonfinite crater point or progress".into());
        }
        let identity = Mapping { position: point, jacobian: DMat3::IDENTITY };
        let progress = progress.clamp(0., 1.);
        if progress == 0. {
            return Ok(identity);
        }
        let p = DVec3::from(point);
        let delta = p - DVec3::from(self.spec.center);
        let h = delta.dot(self.axis);
        if !delta.is_finite() || !h.is_finite() {
            return Err("crater relative position exceeds numeric range".into());
        }
        let q = h / self.spec.influence_depth;
        if q.abs() >= 1. {
            return Ok(identity);
        }
        let radial = delta - self.axis * h;
        let r = radial.x.hypot(radial.y).hypot(radial.z);
        let radius = self.spec.radius * progress;
        let width = self.spec.rim_width * progress;
        if radius == 0. || width == 0. {
            return Err("crater growth dimensions underflow".into());
        }
        let mantle = self.profile.mantle;
        // what lies on the ground, where it does: it does not grow with the crater
        let (lift, lift_slope) = self.deposit.as_ref().map_or((0., [0.; 2]), |d| {
            let (u, v) = self.plane_axes();
            d.height(delta.dot(u), delta.dot(v))
        });
        let beyond = r / radius >= 1. + width / radius && mantle.is_none_or(|m| r >= m.reach * progress);
        if beyond && lift == 0. {
            return Ok(identity);
        }
        let (bowl, bowl_derivative) = if beyond { (0., 0.) } else { bowl(r / radius, self.profile.bowl_exponent) };
        let (rim, rim_derivative) = if beyond { (0., 0.) } else { bump((r - radius) / width) };
        // the mantle grows with the crater as the rest of it does: its crest and its width are the grown ones
        let (heap, heap_derivative) = mantle.filter(|_| !beyond).map_or((0., 0.), |m| {
            let (shape, slope) = mantle_shape(r, radius, width);
            (m.thickness * shape, m.thickness * slope)
        });
        let amount = (-self.spec.depth * bowl + self.spec.rim_height * rim + heap) * progress;
        let amount = if self.deposit.is_some() { amount + lift } else { amount };
        // Cancel progress analytically instead of dividing two tiny dimensions.
        let derivative = |height: f64, scale: f64, slope: f64| {
            if slope == 0. || height == 0. {
                0.
            } else {
                height / scale * slope
            }
        };
        let dr = -derivative(self.spec.depth, self.spec.radius, bowl_derivative)
            + derivative(self.spec.rim_height, self.spec.rim_width, rim_derivative)
            + heap_derivative * progress;
        let (envelope, de) = bump(q);
        let mut gradient = self.axis * (amount / self.spec.influence_depth * de)
            + if r > 0. { radial / r * (dr * envelope) } else { DVec3::ZERO };
        if self.deposit.is_some() {
            let (u, v) = self.plane_axes();
            gradient += (u * lift_slope[0] + v * lift_slope[1]) * envelope;
        }
        let position = p + self.axis * (amount * envelope);
        let jacobian =
            DMat3::IDENTITY + DMat3::from_cols(self.axis * gradient.x, self.axis * gradient.y, self.axis * gradient.z);
        // det(I + axis⊗gradient) = 1 + axis·gradient. Since max|bump'|
        // is 8/(3√3), influence_depth >= 2 max(depth,rim_height) bounds
        // this determinant below by 1 - 4/(3√3), which is positive.
        let determinant = jacobian.determinant();
        if !position.is_finite() || !jacobian.is_finite() || !determinant.is_finite() || determinant <= 0. {
            return Err("crater deformation exceeds numeric range".into());
        }
        Ok(Mapping { position: position.to_array(), jacobian })
    }

    /// Allocate one output vertex array after admission. Input geometry and any
    /// caller-owned indices/caches are outside this destination-buffer budget.
    /// Missing (zero) normal/tangent attributes stay missing. Other attributes
    /// are preserved, and a failure never mutates the input.
    pub fn deform(&self, vertices: &[Vertex], progress: f64, max_bytes: usize) -> Result<Vec<Vertex>, String> {
        self.deform_in(vertices, DMat4::IDENTITY, progress, max_bytes)
    }

    /// Deform vertices in an imported primitive's coordinates, applying the
    /// crater after its basis/node transform. Output stays in primitive space,
    /// preserving the original transform and triangle winding in both renderers.
    pub fn deform_in(
        &self,
        vertices: &[Vertex],
        object_from_mesh: DMat4,
        progress: f64,
        max_bytes: usize,
    ) -> Result<Vec<Vertex>, String> {
        let bytes = vertices.len().checked_mul(std::mem::size_of::<Vertex>()).ok_or("crater buffer overflow")?;
        if bytes > max_bytes {
            return Err("crater vertex buffer exceeds memory budget".into());
        }
        if !progress.is_finite() {
            return Err("nonfinite crater progress".into());
        }
        let inverse = object_from_mesh.inverse();
        if !object_from_mesh.is_finite() || !inverse.is_finite() || object_from_mesh.row(3) != glam::DVec4::W {
            return Err("crater mesh transform must be finite, affine and invertible".into());
        }
        let linear = DMat3::from_mat4(object_from_mesh);
        let linear_inverse = DMat3::from_mat4(inverse);
        let mut out = Vec::new();
        out.try_reserve_exact(vertices.len()).map_err(|_| "cannot allocate crater vertex buffer")?;
        for v in vertices {
            if bytemuck::cast_slice::<Vertex, f32>(std::slice::from_ref(v)).iter().any(|v| !v.is_finite()) {
                return Err("nonfinite crater source vertex".into());
            }
            let point = object_from_mesh.transform_point3(DVec3::from(v.pos.map(f64::from)));
            let mapped = self.map(point.to_array(), progress)?;
            if mapped.jacobian == DMat3::IDENTITY && mapped.position == point.to_array() {
                out.push(*v);
                continue;
            }
            let jacobian = linear_inverse * mapped.jacobian * linear;
            let normal = jacobian.inverse().transpose() * DVec3::from(v.normal.map(f64::from));
            let normal = normalize_or_missing(normal)?;
            let tangent = jacobian * DVec3::new(v.tangent[0] as f64, v.tangent[1] as f64, v.tangent[2] as f64);
            let tangent = normalize_or_missing(tangent - normal * tangent.dot(normal))?;
            let vertex = Vertex {
                pos: inverse.transform_point3(DVec3::from(mapped.position)).as_vec3().to_array(),
                normal: normal.as_vec3().to_array(),
                tangent: [tangent.x as f32, tangent.y as f32, tangent.z as f32, v.tangent[3]],
                ..*v
            };
            if vertex.pos.iter().any(|v| !v.is_finite()) {
                return Err("crater position exceeds render precision".into());
            }
            out.push(vertex);
        }
        Ok(out)
    }
}

impl Mantle {
    /// The cubic units that a mantle of unit thickness holds on `crater`: `2 pi` times the integral of `r (R / r)^3` times
    /// the ramp, from where the ramp begins to the reach (Simpson's rule on 20 000 intervals).
    fn volume_for(&self, crater: &Crater) -> f64 {
        let (radius, width) = (crater.spec.radius, crater.spec.rim_width);
        let (from, intervals) = (radius - width, 20_000);
        let h = (self.reach - from) / intervals as f64;
        let f = |r: f64| r * mantle_shape(r, radius, width).0;
        let mut sum = f(from) + f(self.reach);
        for i in 1..intervals {
            sum += f(from + i as f64 * h) * if i % 2 == 1 { 4. } else { 2. };
        }
        2. * std::f64::consts::PI * sum * h / 3.
    }
}

/// The mantle of unit thickness at distance `r` from the axis, for a crest at `radius` and a rim `width` wide, and its derivative:
/// `(R / r)^3` outside the rim, brought to zero over the rim's width by a smoothstep that ends at the crest's outer edge.
fn mantle_shape(r: f64, radius: f64, width: f64) -> (f64, f64) {
    let u = (r - (radius - width)) / (2. * width);
    if u <= 0. {
        return (0., 0.);
    }
    let (ramp, slope) = if u >= 1. { (1., 0.) } else { (u * u * (3. - 2. * u), 6. * u * (1. - u) / (2. * width)) };
    let cube = (radius / r).powi(3);
    (cube * ramp, cube * (slope - 3. * ramp / r))
}

/// Compact C¹ bump and its derivative with respect to its argument.
fn bump(x: f64) -> (f64, f64) {
    if x.abs() >= 1. {
        return (0., 0.);
    }
    let a = 1. - x * x;
    (a * a, -4. * x * a)
}
/// The bowl's bump with the exponent `p`: [`bump`] for 2, where it keeps the bits it always had.
fn bowl(x: f64, p: f64) -> (f64, f64) {
    if p == 2. {
        return bump(x);
    }
    if x.abs() >= 1. {
        return (0., 0.);
    }
    let a = 1. - x * x;
    (a.powf(p), -2. * p * x * a.powf(p - 1.))
}
fn normalize_or_missing(v: DVec3) -> Result<DVec3, String> {
    if v == DVec3::ZERO {
        return Ok(v);
    }
    v.try_normalize().ok_or_else(|| "crater direction exceeds numeric range".into())
}
