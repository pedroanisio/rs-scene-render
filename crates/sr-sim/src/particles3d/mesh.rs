//! Bounded surface emission, uniformly distributed by triangle area. Unlike
//! solid collision/source regions, an emitting surface need not be closed.

use super::{cross, finite, length, Error};
use crate::rng;

#[derive(Debug)]
pub struct Mesh {
    points: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    area: Vec<f64>,
    bytes: usize,
}
impl Mesh {
    pub fn new(points: &[[f64; 3]], triangles: &[[u32; 3]], max_bytes: usize) -> Result<Self, Error> {
        if !(3..=1_000_000).contains(&points.len()) || !(1..=1_000_000).contains(&triangles.len()) {
            return Err(Error::Limit("mesh emission vertex/triangle count"));
        }
        let bytes = points
            .len()
            .checked_mul(24)
            .and_then(|n| n.checked_add(triangles.len() * 20 + 256))
            .ok_or(Error::Limit("emission mesh size overflow"))?;
        if bytes > max_bytes {
            return Err(Error::Limit("emission mesh bytes"));
        }
        if points.iter().any(|p| !finite(*p)) {
            return Err(Error::Invalid("emission mesh nonfinite vertex"));
        }
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        // Mesh imports may retain vertices belonging to other primitives.
        // Only referenced geometry defines the scale of our area predicates.
        for index in triangles.iter().flatten() {
            let p = points.get(*index as usize).ok_or(Error::Invalid("emission mesh index"))?;
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        let extent = (0..3).map(|i| hi[i] - lo[i]).fold(0., f64::max);
        if extent <= 0. || !extent.is_finite() {
            return Err(Error::Invalid("emission mesh extent"));
        }
        let mut area = Vec::with_capacity(triangles.len());
        let mut total = 0.;
        let mut compensation = 0.;
        for triangle in triangles {
            let point = |index: u32| -> Result<[f64; 3], Error> {
                let p = points.get(index as usize).ok_or(Error::Invalid("emission mesh index"))?;
                Ok(std::array::from_fn(|i| (p[i] - lo[i]) / extent))
            };
            let a = point(triangle[0])?;
            let b = point(triangle[1])?;
            let c = point(triangle[2])?;
            let ab = std::array::from_fn(|i| b[i] - a[i]);
            let ac = std::array::from_fn(|i| c[i] - a[i]);
            let weight = length(cross(ab, ac));
            if weight <= 0. || !weight.is_finite() {
                return Err(Error::Invalid("degenerate emission triangle"));
            }
            let y = weight - compensation;
            let t = total + y;
            compensation = (t - total) - y;
            total = t;
            area.push(total);
        }
        Ok(Self { points: points.to_vec(), triangles: triangles.to_vec(), area, bytes })
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn sample(&self, seed: u64, id: u64) -> [f64; 3] {
        let target = rng::unit(seed, id, 21) * self.area.last().expect("nonempty mesh");
        let i = self.area.partition_point(|&a| a <= target).min(self.area.len() - 1);
        let [a, b, c] = self.triangles[i].map(|i| self.points[i as usize]);
        let u = rng::unit(seed, id, 22).sqrt();
        let v = rng::unit(seed, id, 23);
        std::array::from_fn(|i| a[i] * (1. - u) + b[i] * (u * (1. - v)) + c[i] * (u * v))
    }
}
