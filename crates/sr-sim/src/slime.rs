//! Physarum transport networks (Jones 2010, "Characteristics of pattern formation and evolution
//! in approximations of Physarum transport networks"): agents sample a trail map at three
//! sensors (ahead, and `sensorAngle` to either side, `sensorDistance` away), rotate by
//! `turnAngle` toward the strongest reading (randomly when both sides beat the front), move and
//! deposit; the map then diffuses (a 3 × 3 mean mixed in by `diffuse`) and decays by `decay`.
//! One step every 1/60 s; distances are in box pixels.

use crate::rng;
use crate::timeline::State;

/// Where the agents start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spawn {
    Random,
    /// Uniform in a centred disc, facing outward.
    Disc,
    /// On a centred ring, facing inward.
    Ring,
    /// At the centre, in every direction.
    Center,
}

#[derive(Clone, Debug)]
pub struct SlimeSpec {
    pub seed: u64,
    pub size: [f64; 2],
    /// Trail cells along the longer side.
    pub resolution: usize,
    pub agents: usize,
    pub spawn: Spawn,
    /// Degrees.
    pub sensor_angle: f64,
    pub sensor_distance: f64,
    /// Degrees per step.
    pub turn_angle: f64,
    /// Pixels per second.
    pub speed: f64,
    pub deposit: f64,
    /// Fraction of the trail lost per step.
    pub decay: f64,
    pub diffuse: f64,
}

impl SlimeSpec {
    pub fn grid(&self) -> (usize, usize, f64) {
        let h = self.size[0].max(self.size[1]) / self.resolution.max(2) as f64;
        (((self.size[0] / h).round() as usize).max(2), ((self.size[1] / h).round() as usize).max(2), h)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Slime {
    pub nx: usize,
    pub ny: usize,
    /// Cell size in pixels.
    pub h: f64,
    pub trail: Vec<f32>,
    /// Agent positions (cells) and headings (radians).
    pub x: Vec<f32>,
    pub y: Vec<f32>,
    pub heading: Vec<f32>,
}

impl State for Slime {
    fn bytes(&self) -> usize {
        self.trail.len() * 4 + self.x.len() * 12
    }
}

pub fn init(s: &SlimeSpec) -> Slime {
    let (nx, ny, h) = s.grid();
    let (cx, cy) = (nx as f64 / 2.0, ny as f64 / 2.0);
    let r = nx.min(ny) as f64 * 0.35;
    let mut sl = Slime { nx, ny, h, trail: vec![0.0; nx * ny], x: Vec::new(), y: Vec::new(), heading: Vec::new() };
    for k in 0..s.agents as u64 {
        let (u, w, a) = (rng::unit(s.seed, k, 1), rng::unit(s.seed, k, 2), rng::unit(s.seed, k, 3));
        let ang = a * std::f64::consts::TAU;
        let (x, y, hd) = match s.spawn {
            Spawn::Random => (u * nx as f64, w * ny as f64, ang),
            Spawn::Disc => {
                let (rr, t) = (u.sqrt() * r, w * std::f64::consts::TAU);
                (cx + rr * t.cos(), cy + rr * t.sin(), t)
            }
            Spawn::Ring => {
                let t = w * std::f64::consts::TAU;
                (cx + r * t.cos(), cy + r * t.sin(), t + std::f64::consts::PI)
            }
            Spawn::Center => (cx, cy, ang),
        };
        sl.x.push(x as f32);
        sl.y.push(y as f32);
        sl.heading.push(hd as f32);
    }
    sl
}

pub fn step(s: &SlimeSpec, sl: &mut Slime, step_index: u64, dt: f64) {
    let (nx, ny) = (sl.nx, sl.ny);
    let (w, hgt) = (nx as f32, ny as f32);
    let sa = s.sensor_angle.to_radians() as f32;
    let sd = (s.sensor_distance / sl.h) as f32;
    let ta = s.turn_angle.to_radians() as f32;
    let dist = (s.speed * dt / sl.h) as f32;
    let dep = s.deposit as f32;
    let sense = |t: &[f32], x: f32, y: f32, a: f32| {
        let (px, py) = ((x + a.cos() * sd).rem_euclid(w), (y + a.sin() * sd).rem_euclid(hgt));
        t[(py as usize).min(ny - 1) * nx + (px as usize).min(nx - 1)]
    };
    // agents read the map as it was at the start of the step
    let snapshot = sl.trail.clone();
    for i in 0..sl.x.len() {
        let (x, y, hd) = (sl.x[i], sl.y[i], sl.heading[i]);
        let (f, l, r) = (sense(&snapshot, x, y, hd), sense(&snapshot, x, y, hd - sa), sense(&snapshot, x, y, hd + sa));
        let mut nh = hd;
        if f >= l && f >= r {
        } else if f < l && f < r {
            let coin = rng::unit(s.seed ^ 0x51A1, i as u64, step_index) < 0.5;
            nh += if coin { ta } else { -ta };
        } else if l > r {
            nh -= ta;
        } else {
            nh += ta;
        }
        let (mut nx_, mut ny_) = (x + nh.cos() * dist, y + nh.sin() * dist);
        // bounce off the box edges with a random new heading
        if nx_ < 0.0 || nx_ >= w || ny_ < 0.0 || ny_ >= hgt {
            nx_ = nx_.clamp(0.0, w - 1e-3);
            ny_ = ny_.clamp(0.0, hgt - 1e-3);
            nh = (rng::unit(s.seed ^ 0xB0A1, i as u64, step_index) * std::f64::consts::TAU) as f32;
        }
        sl.x[i] = nx_;
        sl.y[i] = ny_;
        sl.heading[i] = nh;
        let k = (ny_ as usize).min(ny - 1) * nx + (nx_ as usize).min(nx - 1);
        sl.trail[k] += dep;
    }
    // diffuse (3 × 3 mean, clamped edges) and decay
    let t0 = sl.trail.clone();
    let (df, keep) = (s.diffuse as f32, (1.0 - s.decay) as f32);
    for j in 0..ny {
        for i in 0..nx {
            let mut sum = 0.0;
            for dj in -1i64..=1 {
                for di in -1i64..=1 {
                    let (ii, jj) = ((i as i64 + di).clamp(0, nx as i64 - 1), (j as i64 + dj).clamp(0, ny as i64 - 1));
                    sum += t0[jj as usize * nx + ii as usize];
                }
            }
            let k = j * nx + i;
            sl.trail[k] = (t0[k] + (sum / 9.0 - t0[k]) * df) * keep;
        }
    }
}

/// Premultiplied linear RGBA: the trail through a ramp from `low` (0) to `high` (`saturation`).
pub fn image(sl: &Slime, saturation: f64, low: [f32; 4], high: [f32; 4]) -> Vec<[f32; 4]> {
    let inv = (1.0 / saturation.max(1e-9)) as f32;
    sl.trail
        .iter()
        .map(|&v| {
            let t = (v * inv).clamp(0.0, 1.0);
            std::array::from_fn(|c| low[c] + (high[c] - low[c]) * t)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> SlimeSpec {
        SlimeSpec {
            seed: 5,
            size: [256.0, 256.0],
            resolution: 128,
            agents: 6000,
            spawn: Spawn::Random,
            sensor_angle: 30.0,
            sensor_distance: 18.0,
            turn_angle: 45.0,
            speed: 60.0,
            deposit: 1.0,
            decay: 0.05,
            diffuse: 0.5,
        }
    }

    /// Coefficient of variation of the trail: 0 for a uniform film, large for sharp veins.
    fn contrast(sl: &Slime) -> f64 {
        let n = sl.trail.len() as f64;
        let m = sl.trail.iter().map(|&v| v as f64).sum::<f64>() / n;
        let var = sl.trail.iter().map(|&v| (v as f64 - m).powi(2)).sum::<f64>() / n;
        var.sqrt() / m.max(1e-12)
    }

    #[test]
    fn agents_self_organise_into_a_network() {
        // the same agents with no sensing (turning toward nothing) spread a near-uniform film;
        // with sensing they concentrate on a network of veins
        let s = spec();
        let mut a = init(&s);
        let blind = SlimeSpec { turn_angle: 0.0, ..spec() };
        let mut b = init(&blind);
        for k in 0..300 {
            step(&s, &mut a, k, 1.0 / 60.0);
            step(&blind, &mut b, k, 1.0 / 60.0);
        }
        let (ca, cb) = (contrast(&a), contrast(&b));
        assert!(ca > 2.0 * cb, "network contrast {ca} vs blind {cb}");
        // deposits and decay balance: total trail is bounded by deposit / decay per agent
        let total: f64 = a.trail.iter().map(|&v| v as f64).sum();
        assert!(total <= s.agents as f64 * s.deposit / s.decay * 1.001, "{total}");
    }
}
