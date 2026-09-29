//! Boids (Reynolds 1987, "Flocks, herds and schools: a distributed behavioral model"): each
//! agent steers by separation, alignment and cohesion with the neighbours it perceives, with
//! Reynolds' steering rule `steer = desired velocity − velocity`, clamped to a maximum force.
//! Neighbours come from a uniform grid of `perception`-sized cells visited in a fixed order,
//! and every agent updates from the previous step's state, so a run is deterministic.

use crate::rng;
use crate::timeline::State;

/// How agents stay in their box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bounds {
    /// Turn back within `margin` of an edge.
    Steer,
    Bounce,
    Wrap,
}

/// Flock parameters, in box pixels, seconds and pixels per second.
#[derive(Clone, Debug)]
pub struct FlockSpec {
    pub seed: u64,
    pub count: usize,
    pub size: [f64; 2],
    /// Cruise speed: agents never slow below it.
    pub speed: f64,
    pub max_speed: f64,
    pub max_force: f64,
    pub perception: f64,
    pub separation_distance: f64,
    pub separation: f64,
    pub alignment: f64,
    pub cohesion: f64,
    pub bounds: Bounds,
    pub margin: f64,
}

/// Agent positions and velocities, structure-of-arrays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Agents {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub vx: Vec<f64>,
    pub vy: Vec<f64>,
}

impl State for Agents {
    fn bytes(&self) -> usize {
        self.x.len() * 32
    }
}

/// The flock at its start: uniformly spread, heading in random directions at cruise speed.
pub fn init(s: &FlockSpec) -> Agents {
    let mut a = Agents::default();
    for k in 0..s.count as u64 {
        a.x.push(rng::unit(s.seed, k, 1) * s.size[0]);
        a.y.push(rng::unit(s.seed, k, 2) * s.size[1]);
        let dir = rng::unit(s.seed, k, 3) * std::f64::consts::TAU;
        let v = s.speed.max(s.max_speed * 0.5);
        a.vx.push(dir.cos() * v);
        a.vy.push(dir.sin() * v);
    }
    a
}

fn limit(v: [f64; 2], max: f64) -> [f64; 2] {
    let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if l > max && l > 0.0 {
        [v[0] * max / l, v[1] * max / l]
    } else {
        v
    }
}

/// Reynolds steering toward direction `d` at maximum speed.
fn steer(d: [f64; 2], v: [f64; 2], s: &FlockSpec) -> [f64; 2] {
    let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
    if l <= 1e-12 {
        return [0.0, 0.0];
    }
    limit([d[0] / l * s.max_speed - v[0], d[1] / l * s.max_speed - v[1]], s.max_force)
}

/// Advances the flock by `dt`; `extra` gives an acceleration at a box position and velocity
/// (force fields).
pub fn step(s: &FlockSpec, a: &mut Agents, dt: f64, extra: &mut dyn FnMut([f64; 2], [f64; 2]) -> [f64; 2]) {
    let n = a.x.len();
    let cell = s.perception.max(1.0);
    let (cols, rows) = (((s.size[0] / cell).ceil() as usize).max(1), ((s.size[1] / cell).ceil() as usize).max(1));
    let cell_of = |x: f64, y: f64| {
        let c = ((x / cell).floor().max(0.0) as usize).min(cols - 1);
        let r = ((y / cell).floor().max(0.0) as usize).min(rows - 1);
        r * cols + c
    };
    // counting sort into cells (stable, so neighbour order is fixed)
    let mut start = vec![0usize; cols * rows + 1];
    let cells: Vec<usize> = (0..n).map(|i| cell_of(a.x[i], a.y[i])).collect();
    for &c in &cells {
        start[c + 1] += 1;
    }
    for c in 0..cols * rows {
        start[c + 1] += start[c];
    }
    let mut order = vec![0usize; n];
    let mut fill = start.clone();
    for (i, &c) in cells.iter().enumerate() {
        order[fill[c]] = i;
        fill[c] += 1;
    }
    let wrap = s.bounds == Bounds::Wrap;
    let (w, h) = (s.size[0], s.size[1]);
    let mut acc = vec![[0.0f64; 2]; n];
    let p2 = s.perception * s.perception;
    let sd2 = s.separation_distance * s.separation_distance;
    for i in 0..n {
        let (xi, yi) = (a.x[i], a.y[i]);
        let (ci, ri) = (cells[i] % cols, cells[i] / cols);
        let (mut sep, mut ali, mut coh, mut count) = ([0.0f64; 2], [0.0f64; 2], [0.0f64; 2], 0usize);
        for dr in -1i64..=1 {
            for dc in -1i64..=1 {
                let (mut r, mut c) = (ri as i64 + dr, ci as i64 + dc);
                if wrap {
                    r = r.rem_euclid(rows as i64);
                    c = c.rem_euclid(cols as i64);
                } else if r < 0 || c < 0 || r >= rows as i64 || c >= cols as i64 {
                    continue;
                }
                let k = r as usize * cols + c as usize;
                for &j in &order[start[k]..start[k + 1]] {
                    if j == i {
                        continue;
                    }
                    let (mut dx, mut dy) = (a.x[j] - xi, a.y[j] - yi);
                    if wrap {
                        dx -= w * (dx / w).round();
                        dy -= h * (dy / h).round();
                    }
                    let d2 = dx * dx + dy * dy;
                    if d2 > p2 {
                        continue;
                    }
                    count += 1;
                    ali[0] += a.vx[j];
                    ali[1] += a.vy[j];
                    coh[0] += dx;
                    coh[1] += dy;
                    if d2 < sd2 && d2 > 1e-12 {
                        // away from the neighbour, weighted by inverse distance
                        sep[0] -= dx / d2;
                        sep[1] -= dy / d2;
                    }
                }
            }
        }
        let v = [a.vx[i], a.vy[i]];
        let mut f = [0.0f64; 2];
        if count > 0 {
            for (k, (dir, wgt)) in [(sep, s.separation), (ali, s.alignment), (coh, s.cohesion)].into_iter().enumerate()
            {
                if wgt == 0.0 || (k == 0 && dir == [0.0, 0.0]) {
                    continue;
                }
                let st = steer(dir, v, s);
                f[0] += st[0] * wgt;
                f[1] += st[1] * wgt;
            }
        }
        if s.bounds == Bounds::Steer && s.margin > 0.0 {
            let push = |d: f64| if d < s.margin { s.max_force * (1.0 - d.max(0.0) / s.margin) } else { 0.0 };
            f[0] += push(xi) - push(w - xi);
            f[1] += push(yi) - push(h - yi);
        }
        let e = extra([xi, yi], v);
        acc[i] = [f[0] + e[0], f[1] + e[1]];
    }
    for (i, ac) in acc.iter().enumerate() {
        let mut v = limit([a.vx[i] + ac[0] * dt, a.vy[i] + ac[1] * dt], s.max_speed);
        let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
        if s.speed > 0.0 && l < s.speed {
            v = if l > 1e-9 { [v[0] / l * s.speed, v[1] / l * s.speed] } else { [s.speed, 0.0] };
        }
        let (mut x, mut y) = (a.x[i] + v[0] * dt, a.y[i] + v[1] * dt);
        match s.bounds {
            Bounds::Wrap => {
                x = x.rem_euclid(w);
                y = y.rem_euclid(h);
            }
            Bounds::Bounce | Bounds::Steer => {
                if x < 0.0 || x > w {
                    v[0] = -v[0];
                    x = x.clamp(0.0, w);
                }
                if y < 0.0 || y > h {
                    v[1] = -v[1];
                    y = y.clamp(0.0, h);
                }
            }
        }
        a.x[i] = x;
        a.y[i] = y;
        a.vx[i] = v[0];
        a.vy[i] = v[1];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> FlockSpec {
        FlockSpec {
            seed: 7,
            count: 150,
            size: [600.0, 400.0],
            speed: 60.0,
            max_speed: 120.0,
            max_force: 300.0,
            perception: 60.0,
            separation_distance: 20.0,
            separation: 1.5,
            alignment: 1.0,
            cohesion: 1.0,
            bounds: Bounds::Wrap,
            margin: 40.0,
        }
    }

    /// Polarisation: |mean heading|, 1 when everyone flies the same way.
    fn order(a: &Agents) -> f64 {
        let (mut x, mut y) = (0.0, 0.0);
        for i in 0..a.x.len() {
            let l = (a.vx[i].hypot(a.vy[i])).max(1e-9);
            x += a.vx[i] / l;
            y += a.vy[i] / l;
        }
        x.hypot(y) / a.x.len() as f64
    }

    #[test]
    fn alignment_orders_the_flock_and_separation_keeps_distance() {
        let s = spec();
        let mut a = init(&s);
        let before = order(&a);
        for _ in 0..600 {
            step(&s, &mut a, 1.0 / 60.0, &mut |_, _| [0.0, 0.0]);
        }
        assert!(before < 0.3 && order(&a) > 0.8, "polarisation {before} -> {}", order(&a));
        // without alignment and cohesion the headings stay disordered
        let lone = FlockSpec { alignment: 0.0, cohesion: 0.0, ..spec() };
        let mut b = init(&lone);
        for _ in 0..600 {
            step(&lone, &mut b, 1.0 / 60.0, &mut |_, _| [0.0, 0.0]);
        }
        assert!(order(&b) < 0.4, "{}", order(&b));
        // separation: close pairs are rarer with it than without
        let close = |a: &Agents| {
            let mut c = 0;
            for i in 0..a.x.len() {
                for j in i + 1..a.x.len() {
                    if (a.x[i] - a.x[j]).hypot(a.y[i] - a.y[j]) < 5.0 {
                        c += 1;
                    }
                }
            }
            c
        };
        let tight = FlockSpec { separation: 0.0, cohesion: 3.0, ..spec() };
        let mut c = init(&tight);
        for _ in 0..600 {
            step(&tight, &mut c, 1.0 / 60.0, &mut |_, _| [0.0, 0.0]);
        }
        assert!(close(&a) * 3 < close(&c).max(1), "separated {} vs unseparated {}", close(&a), close(&c));
        // speeds stay within [speed, maxSpeed] and everyone stays in the box
        for i in 0..a.x.len() {
            let v = a.vx[i].hypot(a.vy[i]);
            assert!(v >= s.speed - 1e-6 && v <= s.max_speed + 1e-6, "{v}");
            assert!((0.0..=600.0).contains(&a.x[i]) && (0.0..=400.0).contains(&a.y[i]));
        }
    }

    #[test]
    fn steering_bounds_keep_agents_inside() {
        let s = FlockSpec { bounds: Bounds::Steer, ..spec() };
        let mut a = init(&s);
        for _ in 0..600 {
            step(&s, &mut a, 1.0 / 60.0, &mut |_, _| [0.0, 0.0]);
        }
        assert!(a.x.iter().all(|x| (0.0..=600.0).contains(x)) && a.y.iter().all(|y| (0.0..=400.0).contains(y)));
    }
}
