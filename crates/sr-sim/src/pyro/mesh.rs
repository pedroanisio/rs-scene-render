//! Closed mesh regions for smoke sources and obstacles. Validation happens
//! before BVH construction; point tests use Parry's oriented-surface query.

use super::Error;
use rapier3d_f64::parry::{
    math::Vector,
    query::{PointQuery, PointQueryWithLocation},
    shape::{TriMesh, TriMeshFlags},
};
use std::collections::BTreeMap;

/// Immutable, bounded closed triangle mesh. Edges must have two oppositely
/// directed incident faces. A globally reversed winding is normalized, while
/// oppositely oriented inner components preserve cavities. Surfaces must not
/// self-intersect; the inside convention is the nearest oriented surface.
#[derive(Debug)]
pub struct Mesh {
    mesh: TriMesh,
    center: [f64; 3],
    scale: f64,
    min: [f64; 3],
    max: [f64; 3],
    bytes: usize,
    velocity: Option<Vec<[f64; 3]>>,
}

impl Mesh {
    /// Build a region from indexed triangles. Input coordinates are normalized
    /// internally before geometric predicates, preserving scene-space queries.
    /// The conservative memory bound includes validation/BVH scratch and is
    /// checked before copying the caller's buffers.
    pub fn new(vertices: &[[f64; 3]], triangles: &[[u32; 3]], max_bytes: usize) -> Result<Self, Error> {
        Self::build(vertices, triangles, None, max_bytes)
    }

    /// Conservative charge including endpoint/midpoint buffers and BVH scratch.
    pub fn moving_bytes(vertices: usize, triangles: usize) -> Result<usize, Error> {
        Self::required_bytes(vertices, triangles, true)
    }

    fn required_bytes(vertices: usize, triangles: usize, moving: bool) -> Result<usize, Error> {
        vertices
            .checked_mul(if moving { 352 } else { 256 })
            .and_then(|v| triangles.checked_mul(768).and_then(|t| v.checked_add(t)))
            .and_then(|v| v.checked_add(4096))
            .ok_or(Error::Limit("mesh region size overflow"))
    }

    /// Linear vertex motion over one interval. Occupancy is sampled at its
    /// midpoint; nearest-surface barycentric velocities prescribe the boundary.
    /// A collapsed midpoint or inconsistent seam motion is an error.
    pub fn moving(
        start: &[[f64; 3]],
        end: &[[f64; 3]],
        triangles: &[[u32; 3]],
        dt: f64,
        max_bytes: usize,
    ) -> Result<Self, Error> {
        if start.len() != end.len() || !dt.is_finite() || dt <= 0.0 {
            return Err(Error::Invalid("deforming mesh requires matching endpoints and positive finite dt"));
        }
        if Self::moving_bytes(start.len(), triangles.len())? > max_bytes
            || start.len() > 1_000_000
            || triangles.len() > 1_000_000
        {
            return Err(Error::Limit("deforming mesh memory or one-million-element budget"));
        }
        let mut points = Vec::with_capacity(start.len());
        let mut velocity = Vec::with_capacity(start.len());
        for (a, b) in start.iter().zip(end) {
            if a.iter().chain(b).any(|v| !v.is_finite()) {
                return Err(Error::Invalid("nonfinite deforming mesh endpoint"));
            }
            points.push(std::array::from_fn(|i| 0.5 * a[i] + 0.5 * b[i]));
            let v = std::array::from_fn(|i| (b[i] - a[i]) / dt);
            if v.iter().any(|x: &f64| !x.is_finite()) {
                return Err(Error::Invalid("nonfinite deforming mesh velocity"));
            }
            velocity.push(v);
        }
        Self::build(&points, triangles, Some(&velocity), max_bytes)
    }

    fn build(
        vertices: &[[f64; 3]],
        triangles: &[[u32; 3]],
        velocity: Option<&[[f64; 3]]>,
        max_bytes: usize,
    ) -> Result<Self, Error> {
        let bytes = Self::required_bytes(vertices.len(), triangles.len(), velocity.is_some())?;
        if bytes > max_bytes || vertices.len() > 1_000_000 || triangles.len() > 1_000_000 {
            return Err(Error::Limit("mesh region memory or one-million-element budget"));
        }
        if vertices.len() < 4 || triangles.len() < 4 || vertices.iter().flatten().any(|v| !v.is_finite()) {
            return Err(Error::Invalid("mesh region requires finite vertices and a closed triangle surface"));
        }
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        let mut used = vec![false; vertices.len()];
        for &i in triangles.iter().flatten() {
            *used.get_mut(i as usize).ok_or(Error::Invalid("mesh region triangle index is out of range"))? = true;
        }
        for (p, &used) in vertices.iter().zip(&used) {
            if !used {
                continue;
            }
            for a in 0..3 {
                min[a] = min[a].min(p[a]);
                max[a] = max[a].max(p[a]);
            }
        }
        let center = std::array::from_fn(|a| min[a] * 0.5 + max[a] * 0.5);
        let scale = (0..3).map(|a| max[a] - min[a]).fold(0.0_f64, f64::max);
        if !scale.is_finite() || scale <= 0.0 {
            return Err(Error::Invalid("mesh region extent cannot be represented"));
        }
        // Render meshes duplicate positions along normal/UV seams. Weld exact
        // positions only: a tolerance-based weld could erase a small cavity.
        let mut welded = BTreeMap::new();
        let mut points = Vec::new();
        let mut remap = Vec::with_capacity(vertices.len());
        let mut speeds = velocity.map(|_| Vec::new());
        for (input, (p, &used)) in vertices.iter().zip(&used).enumerate() {
            if !used {
                remap.push(u32::MAX);
                continue;
            }
            let key = p.map(|v| if v == 0.0 { 0 } else { v.to_bits() });
            let i = *welded.entry(key).or_insert_with(|| {
                let i = points.len() as u32;
                points.push(Vector::from_array(std::array::from_fn(|a| (p[a] - center[a]) / scale)));
                if let (Some(speeds), Some(velocity)) = (&mut speeds, velocity) {
                    speeds.push(velocity[input]);
                }
                i
            });
            if let (Some(speeds), Some(velocity)) = (&speeds, velocity) {
                if speeds[i as usize] != velocity[input] {
                    return Err(Error::Invalid("deforming mesh seam has inconsistent material velocities"));
                }
            }
            remap.push(i);
        }
        let triangles: Vec<[u32; 3]> = triangles
            .iter()
            .map(|triangle| {
                let mut result = [0; 3];
                for (out, &input) in result.iter_mut().zip(triangle) {
                    *out = *remap
                        .get(input as usize)
                        .ok_or(Error::Invalid("mesh region triangle index is out of range"))?;
                }
                Ok(result)
            })
            .collect::<Result<_, Error>>()?;
        drop(welded);
        drop(remap);
        let vertices = points;
        let mut edges = BTreeMap::<(u32, u32), (u32, i32)>::new();
        let mut volume = 0.0;
        let mut compensation = 0.0;
        for &[a, b, c] in &triangles {
            if [a, b, c].iter().any(|&v| v as usize >= vertices.len()) {
                return Err(Error::Invalid("mesh region triangle index is out of range"));
            }
            let (v0, v1, v2) = (vertices[a as usize], vertices[b as usize], vertices[c as usize]);
            if (v1 - v0).cross(v2 - v0).length_squared() <= 1e-24 {
                return Err(Error::Invalid("mesh region has degenerate or numerically unresolved triangles"));
            }
            for (from, to) in [(a, b), (b, c), (c, a)] {
                let key = (from.min(to), from.max(to));
                let edge = edges.entry(key).or_default();
                edge.0 += 1;
                edge.1 += if from < to { 1 } else { -1 };
                if edge.0 > 2 {
                    return Err(Error::Invalid("mesh region has a nonmanifold edge"));
                }
            }
            let term = v0.dot(v1.cross(v2)) - compensation;
            let next = volume + term;
            compensation = (next - volume) - term;
            volume = next;
        }
        if edges.values().any(|&(count, orientation)| count != 2 || orientation != 0) {
            return Err(Error::Invalid("mesh region must be closed with consistent face orientation"));
        }
        if !volume.is_finite() || volume.abs() < 1e-15 {
            return Err(Error::Invalid("mesh region has zero or numerically unresolved signed volume"));
        }
        drop(edges);
        let triangles = triangles.iter().map(|&[a, b, c]| if volume < 0.0 { [a, c, b] } else { [a, b, c] }).collect();
        let mesh = TriMesh::with_flags(vertices, triangles, TriMeshFlags::ORIENTED)
            .map_err(|_| Error::Invalid("mesh region BVH construction failed"))?;
        Ok(Self { mesh, center, scale, min, max, bytes, velocity: speeds })
    }

    /// Conservative construction/resident memory charge, shared by cloned Arcs.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Nearest material boundary velocity, extended off the surface for voxel
    /// faces. Stationary regions return zero. Query failures are explicit.
    pub fn boundary_velocity(&self, p: [f64; 3]) -> Result<[f64; 3], Error> {
        let Some(velocity) = &self.velocity else { return Ok([0.0; 3]) };
        let point = Vector::from_array(std::array::from_fn(|a| (p[a] - self.center[a]) / self.scale));
        if !point.is_finite() {
            return Err(Error::Invalid("nonfinite deforming mesh boundary query"));
        }
        let (_, (triangle, location)) = self
            .mesh
            .project_local_point_and_get_location_with_max_dist(point, false, f64::MAX)
            .ok_or(Error::Invalid("unresolved deforming mesh boundary query"))?;
        let weights = location.barycentric_coordinates().ok_or(Error::Invalid("unresolved deforming mesh boundary"))?;
        let indices = self.mesh.indices()[triangle as usize];
        let result =
            std::array::from_fn(|a| (0..3).map(|i| weights[i] * velocity[indices[i] as usize][a]).sum::<f64>());
        if result.iter().any(|x| !x.is_finite()) {
            return Err(Error::Invalid("nonfinite interpolated boundary velocity"));
        }
        Ok(result)
    }

    /// Includes the surface. Nonfinite points and points beyond the mesh bounds
    /// are outside, without entering the BVH query.
    pub fn contains(&self, p: [f64; 3]) -> bool {
        if p.iter().any(|v| !v.is_finite()) || (0..3).any(|a| p[a] < self.min[a] || p[a] > self.max[a]) {
            return false;
        }
        let point = Vector::from_array(std::array::from_fn(|a| (p[a] - self.center[a]) / self.scale));
        self.mesh.contains_local_point(point)
    }
}
