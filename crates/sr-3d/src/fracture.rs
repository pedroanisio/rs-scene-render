//! Seeded closed-solid partitioning for cinematic fracture. Exterior faces keep
//! their source-triangle identity; new cut faces have explicit interior identity.
//! This is authored geometry, not a stress/failure prediction.
mod cut;
mod surface;
mod validate;
use glam::DVec3;
use std::collections::BTreeMap;
pub use surface::{surface, Surface, SurfaceMaterial};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid fracture input: {0}")]
    Invalid(&'static str),
    #[error("fracture resource limit: {0}")]
    Limit(&'static str),
    #[error("fracture geometry could not be resolved: {0}")]
    Geometry(&'static str),
}
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub pieces: usize,
    pub seed: u64,
    /// Total assigned source mass, distributed by fragment volume.
    pub mass: f64,
    /// Conservative working/output allocation allowance, excluding caller input.
    pub max_bytes: usize,
    pub max_work: usize,
}
impl Default for Spec {
    fn default() -> Self {
        Self { pieces: 8, seed: 0, mass: 1., max_bytes: 256 << 20, max_work: 100_000_000 }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub indices: [u32; 3],
    /// Input triangle for exterior attribute interpolation; None for cut faces.
    pub source: Option<usize>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// Coordinates relative to `center`, in the input object's axes and units.
    pub vertices: Vec<[f64; 3]>,
    pub faces: Vec<Face>,
    pub center: [f64; 3],
    pub volume: f64,
    pub mass: f64,
}
#[derive(Clone)]
struct Solid {
    vertices: Vec<DVec3>,
    faces: Vec<Face>,
}
struct Budget {
    max: usize,
    work: usize,
    retained: usize,
}
impl Budget {
    fn work(&mut self, n: usize) -> Result<(), Error> {
        self.work = self.work.checked_sub(n).ok_or(Error::Limit("work allowance"))?;
        Ok(())
    }
    fn admit(&self, v: usize, t: usize) -> Result<usize, Error> {
        let bytes = v
            .checked_mul(256)
            .and_then(|v| t.checked_mul(1024).and_then(|t| v.checked_add(t)))
            .and_then(|v| v.checked_add(8192))
            .ok_or(Error::Limit("memory overflow"))?;
        if bytes.checked_add(self.retained).is_none_or(|n| n > self.max) {
            return Err(Error::Limit("memory allowance"));
        }
        Ok(bytes)
    }
}
impl Solid {
    fn charge(&self) -> usize {
        self.vertices.len() * 256 + self.faces.len() * 1024 + 8192
    }
    fn bounds(&self) -> (DVec3, DVec3) {
        self.vertices
            .iter()
            .fold((DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)), |(lo, hi), &p| (lo.min(p), hi.max(p)))
    }
    fn properties(&self) -> Result<(f64, DVec3), Error> {
        let mut volume = 0.;
        let mut moment = DVec3::ZERO;
        for f in &self.faces {
            let [a, b, c] = f.indices.map(|i| self.vertices[i as usize]);
            let v = a.dot(b.cross(c)) / 6.;
            volume += v;
            moment += (a + b + c) * (v / 4.);
        }
        if !volume.is_finite() || volume.abs() < 1e-15 || !moment.is_finite() {
            return Err(Error::Geometry("zero or unresolved volume"));
        }
        Ok((volume, moment / volume))
    }
    fn check(&self, budget: &mut Budget) -> Result<(), Error> {
        budget.work(self.faces.len())?;
        let mut edges = BTreeMap::<[u32; 2], (u32, i32)>::new();
        for f in &self.faces {
            if f.indices.iter().any(|&i| i as usize >= self.vertices.len()) {
                return Err(Error::Invalid("triangle index"));
            }
            let [a, b, c] = f.indices.map(|i| self.vertices[i as usize]);
            if (b - a).cross(c - a).length_squared() <= 1e-30 {
                return Err(Error::Geometry("degenerate triangle"));
            }
            for i in 0..3 {
                let a = f.indices[i];
                let b = f.indices[(i + 1) % 3];
                let (key, s) = if a < b { ([a, b], 1) } else { ([b, a], -1) };
                let e = edges.entry(key).or_default();
                e.0 += 1;
                e.1 += s;
                if e.0 > 2 {
                    return Err(Error::Invalid("nonmanifold edge"));
                }
            }
        }
        if edges.values().any(|e| *e != (2, 0)) {
            return Err(Error::Invalid("surface must be closed and consistently oriented"));
        }
        self.properties()?;
        Ok(())
    }
}

/// Partition a closed, oriented, non-self-intersecting triangle solid. Exact
/// duplicate positions are welded; source triangle IDs retain material/UV seams.
/// Computation uses a normalized f64 frame. Resource/numerical failures return
/// errors atomically, without a partially fractured object.
pub fn fracture(vertices: &[[f64; 3]], triangles: &[[u32; 3]], spec: Spec) -> Result<Vec<Piece>, Error> {
    if !(1..=4096).contains(&spec.pieces) || !spec.mass.is_finite() || spec.mass <= 0. {
        return Err(Error::Invalid("piece count must be 1..4096 and mass positive finite"));
    }
    let mut budget = Budget { max: spec.max_bytes, work: spec.max_work, retained: 0 };
    budget.admit(vertices.len(), triangles.len())?;
    budget.work(vertices.len())?;
    if vertices.len() > 1_000_000 || triangles.len() > 1_000_000 {
        return Err(Error::Limit("one million input elements"));
    }
    if vertices.len() < 4 || triangles.len() < 4 || vertices.iter().flatten().any(|v| !v.is_finite()) {
        return Err(Error::Invalid("finite closed triangle surface required"));
    }
    let (lo, hi) = vertices
        .iter()
        .map(|p| DVec3::from(*p))
        .fold((DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)), |(lo, hi), p| (lo.min(p), hi.max(p)));
    let origin = lo * 0.5 + hi * 0.5;
    let scale = (hi - lo).max_element();
    if !scale.is_finite() || scale <= 0. || !scale.powi(3).is_finite() || scale.powi(3) == 0. {
        return Err(Error::Invalid("unrepresentable solid scale"));
    }
    let mut solid = Solid { vertices: Vec::new(), faces: Vec::with_capacity(triangles.len()) };
    let mut points = BTreeMap::new();
    let mut remap = Vec::with_capacity(vertices.len());
    for p in vertices {
        let key = p.map(|v| if v == 0. { 0 } else { v.to_bits() });
        let id = *points.entry(key).or_insert_with(|| {
            let id = solid.vertices.len() as u32;
            solid.vertices.push((DVec3::from(*p) - origin) / scale);
            id
        });
        remap.push(id);
    }
    for (source, indices) in triangles.iter().enumerate() {
        if indices.iter().any(|&i| i as usize >= remap.len()) {
            return Err(Error::Invalid("triangle index"));
        }
        solid.faces.push(Face { indices: indices.map(|i| remap[i as usize]), source: Some(source) });
    }
    solid.check(&mut budget)?;
    validate::intersections(&solid, &mut budget)?;
    if solid.properties()?.0 < 0. {
        for f in &mut solid.faces {
            f.indices.swap(1, 2);
        }
    }
    let total = solid.properties()?.0;
    drop(points);
    drop(remap);
    let mut solids = validate::separate(solid, &mut budget)?;
    let mut rng = spec.seed;
    if solids.len() > spec.pieces {
        return Err(Error::Invalid("piece count is below the number of disconnected solids"));
    }
    while solids.len() < spec.pieces {
        let mut largest = 0;
        let mut volume = 0.;
        for (i, s) in solids.iter().enumerate() {
            budget.work(s.faces.len())?;
            let v = s.properties()?.0;
            if v > volume {
                largest = i;
                volume = v;
            }
        }
        budget.retained = solids.iter().map(Solid::charge).sum();
        let source = &solids[largest];
        let (lo, hi) = source.bounds();
        let extent = hi - lo;
        let axis = if extent.x >= extent.y && extent.x >= extent.z {
            0
        } else if extent.y >= extent.z {
            1
        } else {
            2
        };
        let mut split = None;
        for _ in 0..32 {
            let mut normal = DVec3::new(random(&mut rng) - 0.5, random(&mut rng) - 0.5, random(&mut rng) - 0.5);
            normal[axis] = 1.;
            normal = normal.normalize();
            let (low, high) = source
                .vertices
                .iter()
                .map(|p| p.dot(normal))
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
            let bias = low + (high - low) * (0.4 + 0.2 * random(&mut rng));
            if let Some([left, right]) = cut::split(source, normal, bias, &mut budget)? {
                let mut pieces = validate::separate(left, &mut budget)?;
                pieces.extend(validate::separate(right, &mut budget)?);
                if solids.len() - 1 + pieces.len() <= spec.pieces {
                    split = Some(pieces);
                    break;
                }
            }
        }
        let pieces = split.ok_or(Error::Geometry("cannot produce requested fragment count"))?;
        let sum =
            pieces.iter().map(|s| s.properties().map(|v| v.0)).collect::<Result<Vec<_>, _>>()?.iter().sum::<f64>();
        if (sum - volume).abs() > total * 1e-9 {
            return Err(Error::Geometry("cut did not conserve volume"));
        }
        solids.remove(largest);
        solids.extend(pieces);
    }
    budget.retained = solids.iter().map(Solid::charge).sum();
    budget.admit(solids.iter().map(|s| s.vertices.len()).sum(), solids.iter().map(|s| s.faces.len()).sum())?;
    let mut out = Vec::with_capacity(solids.len());
    let sum = solids.iter().map(|s| s.properties().map(|v| v.0)).collect::<Result<Vec<_>, _>>()?.iter().sum::<f64>();
    if (sum - total).abs() > total * 1e-9 {
        return Err(Error::Geometry("partition did not conserve volume"));
    }
    for s in solids {
        let (volume, center) = s.properties()?;
        let mass = spec.mass * (volume / sum);
        let world_volume = volume * scale.powi(3);
        let world_center = origin + center * scale;
        if mass <= 0.
            || !mass.is_finite()
            || !world_center.is_finite()
            || world_volume <= 0.
            || !world_volume.is_finite()
        {
            return Err(Error::Geometry("unrepresentable fragment mass, volume or center"));
        }
        out.push(Piece {
            vertices: s.vertices.iter().map(|p| ((*p - center) * scale).to_array()).collect(),
            faces: s.faces,
            center: world_center.to_array(),
            volume: world_volume,
            mass,
        });
    }
    Ok(out)
}
fn random(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((z ^ (z >> 31)) >> 11) as f64 / ((1u64 << 53) as f64)
}
