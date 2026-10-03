//! Radial globe relief shared by render and collision consumers.
use crate::{Primitive, Vertex};
use glam::Vec3;

#[derive(Clone, Debug)]
pub struct Globe {
    /// Undisplaced radius in scene units.
    pub radius: f64,
    /// Physical radius in metres, used only to convert elevation to scene units.
    pub planet_radius: f64,
    pub exaggeration: f64,
    pub segments: u32,
    /// CPU construction + upload mirror; external retained copies are separate.
    pub max_bytes: usize,
}
impl Default for Globe {
    fn default() -> Self {
        Self { radius: 200., planet_radius: 6_378_137., exaggeration: 1., segments: 96, max_bytes: 128 << 20 }
    }
}
impl Globe {
    pub fn memory_cost(&self) -> Result<usize, String> {
        if !(8..=512).contains(&self.segments)
            || !self.radius.is_finite()
            || self.radius <= 0.
            || !self.planet_radius.is_finite()
            || self.planet_radius <= 0.
            || !self.exaggeration.is_finite()
            || self.exaggeration < 0.
        {
            return Err("globe requires finite radii, nonnegative exaggeration and 8..512 segments".into());
        }
        let n = self.segments as usize;
        let bytes = (n + 1) * (n / 2 + 1) * 256 + n * (n / 2) * 6 * 8 + 4096;
        if bytes > self.max_bytes {
            return Err("globe surface exceeds memory budget".into());
        }
        Ok(bytes)
    }
}

/// Longitude is east-positive in degrees, latitude north-positive. Longitude
/// zero faces -z; north points -y. UVs cover [-180,180] x [90,-90]. Exact seam
/// duplicates and single positions at the poles form a closed triangle surface
/// after exact welding. `height` returns elevation in metres above the datum.
pub fn globe(spec: &Globe, mut height: impl FnMut(f64, f64) -> Result<f64, String>) -> Result<Primitive, String> {
    spec.memory_cost()?;
    let (nx, ny) = (spec.segments as usize, spec.segments as usize / 2);
    let count = (nx + 1) * (ny + 1);
    let mut vertices: Vec<Vertex> = Vec::with_capacity(count);
    let mut indices = Vec::with_capacity(nx * (ny - 1) * 6);
    for j in 0..=ny {
        let lat = 90. - 180. * j as f64 / ny as f64;
        for i in 0..=nx {
            let uv = [i as f32 / nx as f32, j as f32 / ny as f32];
            let canonical = if j == 0 || j == ny || i == nx { j * (nx + 1) } else { j * (nx + 1) + i };
            if canonical < vertices.len() {
                vertices.push(Vertex { uv, map_uv: [uv; 4], ..vertices[canonical] });
                continue;
            }
            let lon = if j == 0 || j == ny { 0. } else { 360. * i as f64 / nx as f64 - 180. };
            let h = height(lon, lat)?;
            let r = spec.radius * (1. + (h / spec.planet_radius) * spec.exaggeration);
            if !h.is_finite() || !r.is_finite() || r <= 0. {
                return Err("globe elevation produces a nonfinite or nonpositive radius".into());
            }
            let (s, c) = lat.to_radians().sin_cos();
            let (sl, cl) = lon.to_radians().sin_cos();
            let pos = if j == 0 {
                [0., -r, 0.]
            } else if j == ny {
                [0., r, 0.]
            } else {
                [r * c * sl, -r * s, -r * c * cl]
            };
            let pos = pos.map(|v| v as f32);
            if pos.iter().any(|v| !v.is_finite()) || pos.iter().all(|v| *v == 0.) {
                return Err("globe exceeds device coordinate precision".into());
            }
            vertices.push(Vertex { pos, uv, map_uv: [uv; 4], ..Default::default() });
        }
    }
    for j in 0..ny {
        for i in 0..nx {
            let a = (j * (nx + 1) + i) as u32;
            let (b, c, d) = (a + 1, a + nx as u32 + 1, a + nx as u32 + 2);
            if j > 0 {
                indices.extend_from_slice(&[a, c, b]);
            }
            if j + 1 < ny {
                indices.extend_from_slice(&[b, c, d]);
            }
        }
    }
    let canonical = |i: usize| {
        let (j, x) = (i / (nx + 1), i % (nx + 1));
        if j == 0 || j == ny || x == nx {
            j * (nx + 1)
        } else {
            i
        }
    };
    // Accumulate in f64 to avoid normal underflow/overflow for small/large
    // scene scales. Seams and poles share the same area-weighted normal.
    let mut normals = vec![glam::DVec3::ZERO; count];
    for t in indices.chunks_exact(3) {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| glam::DVec3::from_array(vertices[i as usize].pos.map(f64::from)));
        let n = (b - a).cross(c - a);
        if !n.is_finite() || n.length_squared() == 0. || n.dot(a + b + c) <= 0. {
            return Err("globe has unresolved or inverted surface triangles".into());
        }
        for &i in t {
            normals[canonical(i as usize)] += n;
        }
    }
    for (i, v) in vertices.iter_mut().enumerate() {
        let n = normals[canonical(i)].normalize_or_zero().as_vec3();
        if n.length_squared() < 0.99 {
            return Err("globe normal cannot be resolved".into());
        }
        let x = if i % (nx + 1) == nx { 0 } else { i % (nx + 1) };
        let lon = (360. * x as f64 / nx as f64 - 180.).to_radians();
        let east = Vec3::new(lon.cos() as f32, 0., lon.sin() as f32);
        let t = (east - n * n.dot(east)).normalize_or_zero();
        if t.length_squared() < 0.99 {
            return Err("globe tangent cannot be resolved".into());
        }
        v.normal = n.to_array();
        v.tangent = [t.x, t.y, t.z, -1.];
    }
    Ok(Primitive { vertices, indices, ..Default::default() })
}
