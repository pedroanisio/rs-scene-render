//! The part of a body below the water's surface: its volume and the centroid of that volume,
//! which is what buoyancy needs. The surface is a plane, the fit of the water's free surface
//! under the body; in scene axes y points down, so the water is where y is greater than the
//! surface's.

/// The free surface, `y = offset + slope[0] * x + slope[1] * z` in scene axes and units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surface {
    pub offset: f64,
    pub slope: [f64; 2],
}

impl Surface {
    /// How far below the surface the point is, along the vertical: positive in the water.
    pub fn depth(&self, p: [f64; 3]) -> f64 {
        p[1] - (self.offset + self.slope[0] * p[0] + self.slope[1] * p[2])
    }

    fn is_finite(&self) -> bool {
        self.offset.is_finite() && self.slope.iter().all(|s| s.is_finite())
    }
}

/// The volume of a body below the surface and where its centre is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Submerged {
    /// Cubic scene units; zero when the body is clear of the water.
    pub volume: f64,
    /// Centroid of that volume, scene axes; the body's own centre when there is none.
    pub centroid: [f64; 3],
    /// Square scene units of the body at the surface, seen from above: the waterline's area.
    pub waterline: f64,
    /// Square scene units of the submerged part seen from above: the area it presents to
    /// something moving through the water vertically.
    pub projected: f64,
}

/// A sphere, in closed form (a spherical cap, whatever the slope of the surface).
pub fn submerged_sphere(centre: [f64; 3], radius: f64, surface: &Surface) -> Submerged {
    let steep = (1.0 + surface.slope[0] * surface.slope[0] + surface.slope[1] * surface.slope[1]).sqrt();
    // the water side's unit normal, and the centre's perpendicular depth below the surface
    let down = [-surface.slope[0] / steep, 1.0 / steep, -surface.slope[1] / steep];
    let depth = surface.depth(centre) / steep;
    let h = (radius + depth).clamp(0.0, 2.0 * radius);
    if h <= 0.0 {
        return Submerged { volume: 0.0, centroid: centre, waterline: 0.0, projected: 0.0 };
    }
    let volume = std::f64::consts::PI * h * h * (3.0 * radius - h) / 3.0;
    // seen from above the waterline is the circle the surface cuts, foreshortened by the tilt of the surface (the
    // cut is across the surface, and from above it is the cut times the cosine of the tilt, `1 / steep`); the
    // submerged cap fills the whole disc of the ball once it is more than half under
    let cut = if h < 2.0 * radius { std::f64::consts::PI * (2.0 * radius * h - h * h) / steep } else { 0.0 };
    let (waterline, projected) = (cut, if h <= radius { cut } else { std::f64::consts::PI * radius * radius });
    // the cap's centroid, from the sphere's centre toward the water
    let toward = 3.0 * (2.0 * radius - h).powi(2) / (4.0 * (3.0 * radius - h));
    Submerged { volume, centroid: std::array::from_fn(|i| centre[i] + toward * down[i]), waterline, projected }
}

/// A closed triangle mesh in scene axes, wound consistently (either way), cut by the surface.
/// The part below it is measured by the divergence theorem: the clipped triangles plus the cut
/// across the surface, as tetrahedra from one point.
pub fn submerged_mesh(points: &[[f64; 3]], triangles: &[[u32; 3]], surface: &Surface) -> Result<Submerged, String> {
    if !surface.is_finite() {
        return Err("the water surface is not finite".into());
    }
    if points.iter().flatten().any(|c| !c.is_finite()) {
        return Err("the body has a point that is not finite".into());
    }
    let at = |i: u32| points.get(i as usize).copied().ok_or("a triangle names a missing vertex");
    let origin = points.first().copied().ok_or("the body has no points")?;
    let mut volume = 0.0;
    let mut moment = [0.0; 3];
    let mut cut: Vec<([f64; 3], [f64; 3])> = Vec::new();
    let mut seen_from_above = 0.0;
    let tetrahedron = |a: [f64; 3], b: [f64; 3], c: [f64; 3], volume: &mut f64, moment: &mut [f64; 3]| {
        let (u, v, w) = (sub(a, origin), sub(b, origin), sub(c, origin));
        let signed = dot(u, cross(v, w)) / 6.0;
        *volume += signed;
        for k in 0..3 {
            moment[k] += signed * (origin[k] + a[k] + b[k] + c[k]) / 4.0;
        }
    };
    for t in triangles {
        let corners = [at(t[0])?, at(t[1])?, at(t[2])?];
        let depth = corners.map(|p| surface.depth(p));
        // Sutherland-Hodgman against the half space of points strictly in the water
        let mut polygon: Vec<[f64; 3]> = Vec::with_capacity(4);
        let (mut enter, mut exit) = (None, None);
        for k in 0..3 {
            let (a, b) = (k, (k + 1) % 3);
            let (inside_a, inside_b) = (depth[a] > 0.0, depth[b] > 0.0);
            let crossing = || {
                let s = depth[a] / (depth[a] - depth[b]);
                std::array::from_fn(|i| corners[a][i] + (corners[b][i] - corners[a][i]) * s)
            };
            match (inside_a, inside_b) {
                (true, true) => polygon.push(corners[b]),
                (true, false) => {
                    let p = crossing();
                    exit = Some(p);
                    polygon.push(p);
                }
                (false, true) => {
                    let p = crossing();
                    enter = Some(p);
                    polygon.push(p);
                    polygon.push(corners[b]);
                }
                (false, false) => {}
            }
        }
        if polygon.len() < 3 {
            continue;
        }
        for i in 1..polygon.len() - 1 {
            tetrahedron(polygon[0], polygon[i], polygon[i + 1], &mut volume, &mut moment);
            seen_from_above += 0.5 * cross(sub(polygon[i], polygon[0]), sub(polygon[i + 1], polygon[0]))[1].abs();
        }
        // the cut runs the other way round the loop from the clipped triangle's own edge
        if let (Some(enter), Some(exit)) = (enter, exit) {
            cut.push((enter, exit));
        }
    }
    let mut waterline = 0.0;
    if let Some(&(anchor, _)) = cut.first() {
        for &(from, to) in &cut {
            tetrahedron(anchor, from, to, &mut volume, &mut moment);
            waterline += 0.5 * cross(sub(from, anchor), sub(to, anchor))[1];
        }
    }
    let waterline = waterline.abs();
    if volume == 0.0 || !volume.is_finite() {
        return Ok(Submerged { volume: 0.0, centroid: origin, waterline: 0.0, projected: 0.0 });
    }
    let centroid = moment.map(|m| m / volume);
    // a closed surface is seen twice from above, once from each side
    Ok(Submerged { volume: volume.abs(), centroid, waterline, projected: 0.5 * (seen_from_above + waterline) })
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// The water a body floats in, and how its buoyancy is applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Water {
    /// Kilograms per cubic metre.
    pub density: f64,
    /// Metres per second squared.
    pub gravity: f64,
    /// Coefficient of form drag for vertical motion through the water (an engine parameter,
    /// not a value from the impact literature).
    pub drag: f64,
    /// Scene units per metre.
    pub pixels_per_meter: f64,
}

/// The load water puts on a body, and the stiffness the explicit coupling has to be stable for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Buoyancy {
    pub load: crate::physics3d::Load3,
    /// Angular frequency, radians per second, of the vertical spring the waterline makes.
    pub omega: f64,
}

/// A step times the frequency of the waterline spring above which the coupling is not stable:
/// semi-implicit Euler is stable up to 2, and this leaves a margin.
pub const STABILITY_LIMIT: f64 = 1.8;

/// The load of the water on a body of `mass` kilograms whose centre of mass is at
/// `centre_of_mass` and which moves vertically at `vertical_velocity` scene units per second
/// (scene y points down), of which `submerged` is below the surface, for a step of `step`
/// seconds. Two parts, both in scene axes and units:
///
/// - buoyancy: the weight of the displaced water, `rho g V`, upward through the centroid of
///   the submerged volume, so a body that tilts is turned by it;
/// - a quadratic drag on the vertical motion, `-(1/2) rho C_d A |v| v`, with `A` the area the
///   submerged part presents from above, limited to what stops the body within the step so that
///   an explicit step cannot reverse it. The surface is taken to be still vertically.
///
/// It is an error if `omega * step` is not below [`STABILITY_LIMIT`]: a body that light for the
/// step would be driven unstable by an explicit coupling, and that is reported, not damped away.
pub fn buoyant_load(
    submerged: &Submerged,
    centre_of_mass: [f64; 3],
    mass: f64,
    vertical_velocity: f64,
    water: &Water,
    step: f64,
) -> Result<Buoyancy, String> {
    let ppm = water.pixels_per_meter;
    if !(mass.is_finite() && mass > 0.0 && ppm.is_finite() && ppm > 0.0 && step.is_finite() && step > 0.0) {
        return Err("buoyancy needs a positive mass, scale and step".into());
    }
    if !(water.density.is_finite() && water.density > 0.0 && water.gravity.is_finite() && water.gravity >= 0.0) {
        return Err("buoyancy needs a positive water density and a gravity".into());
    }
    if !(water.drag.is_finite() && water.drag >= 0.0) {
        return Err("the drag coefficient must not be negative".into());
    }
    if submerged.volume <= 0.0 {
        return Ok(Buoyancy { load: Default::default(), omega: 0.0 });
    }
    // the waterline is a spring of stiffness rho g A (newtons per metre)
    let omega = (water.density * water.gravity * submerged.waterline / (ppm * ppm) / mass).sqrt();
    if omega * step >= STABILITY_LIMIT {
        return Err(format!(
            "buoyancy is unstable at this step: omega * dt = {:.2} (limit {STABILITY_LIMIT}); lower fixedStep or raise the mass",
            omega * step
        ));
    }
    let up = water.density * water.gravity * submerged.volume / (ppm * ppm * ppm) * ppm;
    let lift = [0.0, -up, 0.0];
    let arm = sub(submerged.centroid, centre_of_mass);
    let torque = cross(arm, lift);
    let speed = vertical_velocity.abs();
    let area = submerged.projected / (ppm * ppm);
    let drag = 0.5 * water.density * water.drag * area * (speed / ppm) * (speed / ppm) * ppm;
    let stops = mass * speed / step;
    let resist = -vertical_velocity.signum() * drag.min(stops);
    Ok(Buoyancy { load: crate::physics3d::Load3 { force: [0.0, -up + resist, 0.0], torque }, omega })
}

/// Where a body at `position`, moving at `velocity`, will be halfway through a step of `step`.
/// A load is held for the whole step, so evaluating it at the start would add energy (it is an
/// explicit force on a spring-like response, which grows the motion); evaluated halfway it adds
/// none, and the step only takes a little out.
pub fn halfway(position: [f64; 3], velocity: [f64; 3], step: f64) -> [f64; 3] {
    std::array::from_fn(|i| position[i] + 0.5 * step * velocity[i])
}

/// Mesh points placed in the world: rotated by the quaternion `[x, y, z, w]`, then moved.
pub fn place(points: &[[f64; 3]], position: [f64; 3], rotation: [f64; 4]) -> Vec<[f64; 3]> {
    let (u, w) = ([rotation[0], rotation[1], rotation[2]], rotation[3]);
    points
        .iter()
        .map(|&v| {
            let t = cross(u, v).map(|c| 2.0 * c);
            let ut = cross(u, t);
            std::array::from_fn(|i| position[i] + v[i] + w * t[i] + ut[i])
        })
        .collect()
}

/// Points and triangles of a closed mesh.
pub type Mesh = (Vec<[f64; 3]>, Vec<[u32; 3]>);

/// A body's shape as a closed triangle mesh in its own axes, for the shapes that have no closed
/// form here: a box exactly, a cylinder, cone or capsule tessellated, a mesh or decomposition as
/// it is. A sphere is measured in closed form instead, and a convex hull is not supported.
pub fn hull_mesh(shape: &crate::physics3d::Shape3) -> Result<Mesh, String> {
    use crate::physics3d::Shape3;
    const AROUND: usize = 48;
    match shape {
        Shape3::Box(h) => {
            let points: Vec<[f64; 3]> =
                (0..8).map(|k| [0, 1, 2].map(|a| if (k >> a) & 1 == 1 { h[a] } else { -h[a] })).collect();
            let faces: [[u32; 3]; 12] = [
                [0, 2, 1],
                [1, 2, 3],
                [4, 5, 6],
                [5, 7, 6],
                [0, 1, 4],
                [1, 5, 4],
                [2, 6, 3],
                [3, 6, 7],
                [0, 4, 2],
                [2, 4, 6],
                [1, 3, 5],
                [3, 7, 5],
            ];
            Ok((points, faces.to_vec()))
        }
        Shape3::TriMesh(points, triangles) | Shape3::Decomposition(points, triangles) => {
            Ok((points.clone(), triangles.clone()))
        }
        Shape3::Cylinder(half, r) => Ok(revolved(&[[-half, 0.0], [-half, *r], [*half, *r], [*half, 0.0]], AROUND)),
        Shape3::Cone(half, r) => Ok(revolved(&[[-half, 0.0], [*half, *r], [*half, 0.0]], AROUND)),
        Shape3::Capsule(half, r) => {
            const RINGS: usize = 12;
            let mut profile = vec![[-half - r, 0.0]];
            for i in 1..RINGS {
                let a = std::f64::consts::FRAC_PI_2 * i as f64 / RINGS as f64;
                profile.push([-half - r * a.cos(), r * a.sin()]);
            }
            for i in 0..RINGS {
                let a = std::f64::consts::FRAC_PI_2 * i as f64 / RINGS as f64;
                profile.push([half + r * a.sin(), r * a.cos()]);
            }
            profile.push([half + r, 0.0]);
            Ok(revolved(&profile, AROUND))
        }
        Shape3::Sphere(_) => Err("a sphere is measured in closed form".into()),
        Shape3::Convex(_) => {
            Err("buoyancy of a convex hull is not supported: give the body a primitive or a closed mesh".into())
        }
    }
}

/// The closed surface made by revolving a profile of `[y, radius]` points about the y axis; the
/// first and last points are on the axis.
fn revolved(profile: &[[f64; 2]], around: usize) -> Mesh {
    let mut points = vec![[0.0, profile[0][0], 0.0]];
    for p in &profile[1..profile.len() - 1] {
        for j in 0..around {
            let a = 2.0 * std::f64::consts::PI * j as f64 / around as f64;
            points.push([p[1] * a.cos(), p[0], p[1] * a.sin()]);
        }
    }
    let last = profile[profile.len() - 1];
    points.push([0.0, last[0], 0.0]);
    let rings = profile.len() - 2;
    let ring = |i: usize, j: usize| (1 + i * around + j % around) as u32;
    let mut triangles = Vec::new();
    for j in 0..around {
        triangles.push([0, ring(0, j + 1), ring(0, j)]);
    }
    for i in 0..rings - 1 {
        for j in 0..around {
            let (a, b, c, d) = (ring(i, j), ring(i, j + 1), ring(i + 1, j), ring(i + 1, j + 1));
            triangles.push([a, b, c]);
            triangles.push([b, d, c]);
        }
    }
    let end = (points.len() - 1) as u32;
    for j in 0..around {
        triangles.push([end, ring(rings - 1, j), ring(rings - 1, j + 1)]);
    }
    (points, triangles)
}
