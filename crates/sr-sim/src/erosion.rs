//! Hydraulic erosion by water droplets (Beyer 2015, "Implementation of a method for hydraulic
//! erosion", TU München). Each droplet starts at a random cell with unit water and speed,
//! follows the terrain's gradient with some inertia, and at every move compares the sediment
//! it carries with its capacity (proportional to its height loss, speed and water): above it,
//! or when climbing, it deposits (bilinearly at its cell); below it, it erodes within a radius
//! (a normalised cone brush). Its speed follows the height change under gravity and its water
//! evaporates. Heights run from 0 to 1.

use crate::rng;
use crate::timeline::State;

#[derive(Clone, Debug)]
pub struct ErosionSpec {
    pub seed: u64,
    pub size: [f64; 2],
    /// Cells along the longer side.
    pub resolution: usize,
    pub octaves: u32,
    /// Noise features across the longer side.
    pub frequency: f64,
    /// Droplets per second (simulated at 60 steps per second).
    pub droplets: f64,
    pub inertia: f64,
    pub capacity: f64,
    pub erode_rate: f64,
    pub deposit_rate: f64,
    pub evaporation: f64,
    pub gravity: f64,
    pub radius: usize,
    /// Droplet lifetime in moves.
    pub lifetime: usize,
}

impl ErosionSpec {
    pub fn grid(&self) -> (usize, usize) {
        let h = self.size[0].max(self.size[1]) / self.resolution.max(2) as f64;
        (((self.size[0] / h).round() as usize).max(4), ((self.size[1] / h).round() as usize).max(4))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Terrain {
    pub nx: usize,
    pub ny: usize,
    pub height: Vec<f32>,
    /// Droplets run so far, and the fractional carry of the rate.
    pub dropped: u64,
    pub carry: f64,
}

impl State for Terrain {
    fn bytes(&self) -> usize {
        self.height.len() * 4
    }
}

fn value_noise(seed: u64, x: f64, y: f64) -> f64 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let s = |t: f64| t * t * (3.0 - 2.0 * t);
    let at = |i: f64, j: f64| rng::unit(seed, ((i as i64 as u64) << 32) ^ (j as i64 as u64 & 0xFFFF_FFFF), 9);
    let (a, b, c, d) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
    let (u, v) = (s(fx), s(fy));
    a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v
}

/// Fractal value noise normalised to [0, 1], or `from` (resampled heights in [0, 1]).
pub fn init(s: &ErosionSpec, from: Option<(usize, usize, Vec<f32>)>) -> Terrain {
    let (nx, ny) = s.grid();
    let mut height = vec![0.0f32; nx * ny];
    if let Some((w, h, px)) = from.filter(|(w, h, px)| *w > 0 && *h > 0 && px.len() == w * h) {
        for j in 0..ny {
            for i in 0..nx {
                let (x, y) = ((i * w / nx).min(w - 1), (j * h / ny).min(h - 1));
                height[j * nx + i] = px[y * w + x];
            }
        }
    } else {
        let long = nx.max(ny) as f64;
        for j in 0..ny {
            for i in 0..nx {
                let (mut amp, mut f, mut v) = (1.0, s.frequency / long, 0.0);
                for o in 0..s.octaves.max(1) {
                    v += amp * value_noise(s.seed.wrapping_add(o as u64 * 7919), i as f64 * f, j as f64 * f);
                    amp *= 0.5;
                    f *= 2.0;
                }
                height[j * nx + i] = v as f32;
            }
        }
    }
    let (lo, hi) = height.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let span = (hi - lo).max(1e-9);
    height.iter_mut().for_each(|v| *v = (*v - lo) / span);
    Terrain { nx, ny, height, dropped: 0, carry: 0.0 }
}

/// Height and gradient at a position (bilinear over the cell).
fn sample(t: &Terrain, x: f64, y: f64) -> (f64, [f64; 2]) {
    let (i, j) = (x.floor() as usize, y.floor() as usize);
    let (u, v) = (x - i as f64, y - j as f64);
    let at = |a: usize, b: usize| t.height[b * t.nx + a] as f64;
    let (nw, ne, sw, se) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
    let gx = (ne - nw) * (1.0 - v) + (se - sw) * v;
    let gy = (sw - nw) * (1.0 - u) + (se - ne) * u;
    let h = nw * (1.0 - u) * (1.0 - v) + ne * u * (1.0 - v) + sw * (1.0 - u) * v + se * u * v;
    (h, [gx, gy])
}

/// Runs one droplet. Returns the net height it moved (eroded minus deposited) for tests.
fn droplet(s: &ErosionSpec, t: &mut Terrain, k: u64, brush: &[(i64, i64, f64)]) -> f64 {
    let (nx, ny) = (t.nx, t.ny);
    let mut x = rng::unit(s.seed ^ 0xD20F, k, 1) * (nx - 1) as f64;
    let mut y = rng::unit(s.seed ^ 0xD20F, k, 2) * (ny - 1) as f64;
    let (mut dx, mut dy) = (0.0f64, 0.0f64);
    let (mut speed, mut water, mut sediment) = (1.0f64, 1.0f64, 0.0f64);
    let mut moved = 0.0;
    for _ in 0..s.lifetime {
        let (ci, cj) = (x.floor() as usize, y.floor() as usize);
        let (u, v) = (x - ci as f64, y - cj as f64);
        let (h, g) = sample(t, x, y);
        dx = dx * s.inertia - g[0] * (1.0 - s.inertia);
        dy = dy * s.inertia - g[1] * (1.0 - s.inertia);
        let l = (dx * dx + dy * dy).sqrt();
        if l < 1e-12 {
            break;
        }
        dx /= l;
        dy /= l;
        x += dx;
        y += dy;
        if x < 0.0 || y < 0.0 || x >= (nx - 1) as f64 || y >= (ny - 1) as f64 {
            break;
        }
        let dh = sample(t, x, y).0 - h;
        let cap = (-dh * speed * water * s.capacity).max(0.01);
        if sediment > cap || dh > 0.0 {
            let amount = if dh > 0.0 { dh.min(sediment) } else { (sediment - cap) * s.deposit_rate };
            sediment -= amount;
            let idx = |a: usize, b: usize| b * nx + a;
            t.height[idx(ci, cj)] += (amount * (1.0 - u) * (1.0 - v)) as f32;
            t.height[idx(ci + 1, cj)] += (amount * u * (1.0 - v)) as f32;
            t.height[idx(ci, cj + 1)] += (amount * (1.0 - u) * v) as f32;
            t.height[idx(ci + 1, cj + 1)] += (amount * u * v) as f32;
            moved -= amount;
        } else {
            let amount = ((cap - sediment) * s.erode_rate).min(-dh);
            for &(bi, bj, w) in brush {
                let (a, b) = (ci as i64 + bi, cj as i64 + bj);
                if a < 0 || b < 0 || a >= nx as i64 || b >= ny as i64 {
                    continue;
                }
                let k = b as usize * nx + a as usize;
                let take = (amount * w).min(t.height[k] as f64);
                t.height[k] -= take as f32;
                sediment += take;
                moved += take;
            }
        }
        speed = (speed * speed + dh * s.gravity).max(0.0).sqrt();
        water *= 1.0 - s.evaporation;
    }
    // a droplet that stops inside the map drops what it still carries (Beyer's droplets simply
    // vanish; depositing keeps the material, so only droplets leaving the map take any away)
    if sediment > 0.0 && x >= 0.0 && y >= 0.0 && x < (nx - 1) as f64 && y < (ny - 1) as f64 {
        let (ci, cj) = (x.floor() as usize, y.floor() as usize);
        let (u, v) = (x - ci as f64, y - cj as f64);
        t.height[cj * nx + ci] += (sediment * (1.0 - u) * (1.0 - v)) as f32;
        t.height[cj * nx + ci + 1] += (sediment * u * (1.0 - v)) as f32;
        t.height[(cj + 1) * nx + ci] += (sediment * (1.0 - u) * v) as f32;
        t.height[(cj + 1) * nx + ci + 1] += (sediment * u * v) as f32;
        moved -= sediment;
    }
    moved
}

fn brush(radius: usize) -> Vec<(i64, i64, f64)> {
    let r = radius.max(1) as i64;
    let mut b: Vec<(i64, i64, f64)> = Vec::new();
    for j in -r..=r {
        for i in -r..=r {
            let w = r as f64 - ((i * i + j * j) as f64).sqrt();
            if w > 0.0 {
                b.push((i, j, w));
            }
        }
    }
    let sum: f64 = b.iter().map(|x| x.2).sum();
    b.iter_mut().for_each(|x| x.2 /= sum);
    b
}

/// One step of `dt`: the droplets due in it.
pub fn step(s: &ErosionSpec, t: &mut Terrain, dt: f64) {
    let exact = t.carry + s.droplets * dt;
    let n = exact.floor() as u64;
    t.carry = exact - n as f64;
    let b = brush(s.radius);
    for _ in 0..n {
        let k = t.dropped;
        droplet(s, t, k, &b);
        t.dropped += 1;
    }
}

/// Opaque premultiplied linear RGBA: hill-shaded terrain, coloured by height from `low` to
/// `high`. `relief` scales slopes for the shading; the sun comes from `azimuth` degrees
/// (0 = up the image, clockwise) at `elevation` degrees.
pub fn image(t: &Terrain, relief: f64, azimuth: f64, elevation: f64, low: [f32; 4], high: [f32; 4]) -> Vec<[f32; 4]> {
    let (az, el) = (azimuth.to_radians(), elevation.to_radians());
    let sun = [az.sin() * el.cos(), -az.cos() * el.cos(), el.sin()];
    let scale = relief * t.nx.max(t.ny) as f64 * 0.25;
    let h = |i: i64, j: i64| {
        t.height[(j.clamp(0, t.ny as i64 - 1) as usize) * t.nx + i.clamp(0, t.nx as i64 - 1) as usize] as f64
    };
    let mut out = Vec::with_capacity(t.height.len());
    for j in 0..t.ny as i64 {
        for i in 0..t.nx as i64 {
            let gx = (h(i + 1, j) - h(i - 1, j)) * 0.5 * scale;
            let gy = (h(i, j + 1) - h(i, j - 1)) * 0.5 * scale;
            let n = [-gx, -gy, 1.0];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let lambert = ((n[0] * sun[0] + n[1] * sun[1] + n[2] * sun[2]) / l).max(0.0);
            let shade = (0.25 + 0.75 * lambert) as f32;
            let v = h(i, j) as f32;
            let c: [f32; 4] = std::array::from_fn(|k| low[k] + (high[k] - low[k]) * v);
            out.push([c[0] * shade, c[1] * shade, c[2] * shade, c[3]]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ErosionSpec {
        ErosionSpec {
            seed: 11,
            size: [128.0, 128.0],
            resolution: 128,
            octaves: 5,
            frequency: 3.0,
            droplets: 60000.0,
            inertia: 0.05,
            capacity: 4.0,
            erode_rate: 0.3,
            deposit_rate: 0.3,
            evaporation: 0.01,
            gravity: 4.0,
            radius: 3,
            lifetime: 30,
        }
    }

    /// Mean absolute Laplacian: large for rough terrain, small for smooth.
    fn roughness(t: &Terrain) -> f64 {
        let mut s = 0.0;
        for j in 1..t.ny - 1 {
            for i in 1..t.nx - 1 {
                let k = j * t.nx + i;
                let lap =
                    t.height[k - 1] + t.height[k + 1] + t.height[k - t.nx] + t.height[k + t.nx] - 4.0 * t.height[k];
                s += lap.abs() as f64;
            }
        }
        s / ((t.nx - 2) * (t.ny - 2)) as f64
    }

    #[test]
    fn droplets_carve_valleys_and_conserve_material() {
        let s = spec();
        let mut t = init(&s, None);
        let before: f64 = t.height.iter().map(|&v| v as f64).sum();
        let peaks_before = t.height.iter().cloned().fold(0.0f32, f32::max);
        for _ in 0..60 {
            step(&s, &mut t, 1.0 / 60.0);
        }
        assert_eq!(t.dropped, 60000);
        let after: f64 = t.height.iter().map(|&v| v as f64).sum();
        // material only moves; what droplets still carry when they leave or die is lost,
        // so the total may fall a little but never rises
        assert!(after <= before + 1e-3 && after > before * 0.9, "{before} -> {after}");
        assert!(t.height.iter().cloned().fold(0.0f32, f32::max) <= peaks_before + 1e-3);
        // erosion wears the high ground down and fills the low ground
        let start = init(&s, None);
        let mut order: Vec<usize> = (0..start.height.len()).collect();
        order.sort_by(|a, b| start.height[*a].total_cmp(&start.height[*b]));
        let tenth = order.len() / 10;
        let change =
            |ks: &[usize]| ks.iter().map(|&k| (t.height[k] - start.height[k]) as f64).sum::<f64>() / ks.len() as f64;
        let (low, high) = (change(&order[..tenth]), change(&order[order.len() - tenth..]));
        assert!(low > 0.0 && high < 0.0, "lowest tenth {low:+}, highest tenth {high:+}");
        assert!(roughness(&t).is_finite());
        let img = image(&t, 1.0, 315.0, 45.0, [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]);
        assert!(img.iter().all(|p| p.iter().all(|c| c.is_finite() && *c >= 0.0)));
    }
}
