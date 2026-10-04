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
}

/// A sphere, in closed form (a spherical cap, whatever the slope of the surface).
pub fn submerged_sphere(centre: [f64; 3], radius: f64, surface: &Surface) -> Submerged {
    let steep = (1.0 + surface.slope[0] * surface.slope[0] + surface.slope[1] * surface.slope[1]).sqrt();
    // the water side's unit normal, and the centre's perpendicular depth below the surface
    let down = [-surface.slope[0] / steep, 1.0 / steep, -surface.slope[1] / steep];
    let depth = surface.depth(centre) / steep;
    let h = (radius + depth).clamp(0.0, 2.0 * radius);
    if h <= 0.0 {
        return Submerged { volume: 0.0, centroid: centre };
    }
    let volume = std::f64::consts::PI * h * h * (3.0 * radius - h) / 3.0;
    // the cap's centroid, from the sphere's centre toward the water
    let toward = 3.0 * (2.0 * radius - h).powi(2) / (4.0 * (3.0 * radius - h));
    Submerged { volume, centroid: std::array::from_fn(|i| centre[i] + toward * down[i]) }
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
        }
        // the cut runs the other way round the loop from the clipped triangle's own edge
        if let (Some(enter), Some(exit)) = (enter, exit) {
            cut.push((enter, exit));
        }
    }
    if let Some(&(anchor, _)) = cut.first() {
        for &(from, to) in &cut {
            tetrahedron(anchor, from, to, &mut volume, &mut moment);
        }
    }
    if volume == 0.0 || !volume.is_finite() {
        return Ok(Submerged { volume: 0.0, centroid: origin });
    }
    let centroid = moment.map(|m| m / volume);
    Ok(Submerged { volume: volume.abs(), centroid })
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
