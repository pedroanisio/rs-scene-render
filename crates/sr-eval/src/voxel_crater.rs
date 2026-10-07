//! A crater in ground made of cells: what the law's bowl takes out, what is thrown and what is heaped on the rim, counted in cells.
//!
//! Pure functions of an [`Occupancy`] and the kernel of the crater ([`sr_3d::crater::Crater`], which holds the law's bowl, its rim and
//! its volume budget), in metres in the frame of the cells: the cell `[i, j, k]` has its centre at `(i + 1/2, j + 1/2, k + 1/2)` cells.
//!
//! * **Removal.** A cell is taken out when its centre is inside the crest radius and between the floor of the bowl
//!   ([`Crater::bowl_depth_at`]) and one crest radius above the crater's plane: so the removed cells are the law's bowl, to the half cell
//!   that a centre decides, and what stands in its way above the plane.
//! * **The budget, in cells.** Of the `N` cells removed, the law says its share (0.8) is thrown out; the rest is uplift, which is not lost: it is
//!   heaped on the rim, so exactly `N - T` cells are made on the rim, with the palette of the cells that were uplifted. Cells are neither made nor
//!   lost, and no kilogram is: a cell is always a cell of the same mass.
//! * **Which cells are thrown.** The cells are ordered by their distance from the axis, then by height, then by the scan. The first `T` are
//!   thrown (the middle of the crater, which the law launches fastest), the others are the uplift (the wall).
//! * **Speeds.** Not from the cells: the energy of the impact is not in them. The engine's launch model ([`sr_sim::cratering::ejecta`])
//!   gives the distribution of mass over speed, and the thrown cells take it by quantiles: cell `i` of `T` has the speed above which
//!   the fraction `(i + 1/2) / T` of the law's mass lies. So the mass is the cells' to the last bit, the distribution is the law's to a
//!   cell, and nothing depends on a random draw but the launch angle, which is a hash of the seed and the cell.
//! * **The rim.** Cells are added one by one where the rim of the law stands (within its width of the crest radius, up to its height and a
//!   little more), each held up by ground or by a cell already added, nearest the crest first and lowest first.
//!
//! The result does not depend on the order the cells were put in the grid.

use sr_3d::crater::Crater;
use sr_3d::occupancy::Occupancy;
use sr_sim::cratering::ejecta::Ejecta;
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};

/// The law's distribution of mass over speed, read by quantile.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeedLaw {
    /// Speeds in descending order and the fraction of the mass at or above each.
    speeds: Vec<f64>,
    cumulative: Vec<f64>,
}

impl SpeedLaw {
    /// From the particles the engine's model launches: their speeds and their masses.
    pub fn from_ejecta(list: &[Ejecta]) -> Result<Self, String> {
        let speed = |p: &Ejecta| p.velocity.iter().map(|v| v * v).sum::<f64>().sqrt();
        if list.is_empty() || list.iter().any(|p| !(p.mass.is_finite() && p.mass > 0.0) || !speed(p).is_finite()) {
            return Err("a speed law needs particles with a mass and a speed".into());
        }
        let mut order: Vec<usize> = (0..list.len()).collect();
        order.sort_by(|a, b| speed(&list[*b]).total_cmp(&speed(&list[*a])).then(a.cmp(b)));
        let total: f64 = order.iter().map(|i| list[*i].mass).sum();
        let mut sum = 0.0;
        let (mut speeds, mut cumulative) = (Vec::new(), Vec::new());
        for i in order {
            sum += list[i].mass;
            speeds.push(speed(&list[i]));
            cumulative.push(sum / total);
        }
        Ok(Self { speeds, cumulative })
    }

    /// The speed above which the fraction `q` (0 to 1) of the mass lies.
    pub fn at(&self, q: f64) -> f64 {
        let i = self.cumulative.partition_point(|c| *c < q).min(self.speeds.len() - 1);
        self.speeds[i]
    }
}

/// How the thrown cells leave.
#[derive(Clone, Debug, PartialEq)]
pub struct Ejection {
    /// The share of what is taken out that is thrown, 0 to 1 (the law's is 0.8).
    pub share: f64,
    pub speeds: SpeedLaw,
    /// Degrees above the tangent plane, and the half width of the spread (uniform).
    pub angle: f64,
    pub spread: f64,
    pub seed: u64,
}

/// A cell that is thrown out: what it was and where, and how it leaves, in metres and metres a second in the frame of the cells.
#[derive(Clone, Debug, PartialEq)]
pub struct Thrown {
    pub cell: [i32; 3],
    pub palette: u8,
    pub position: [f64; 3],
    pub velocity: [f64; 3],
}

/// What a crater does to ground made of cells.
#[derive(Clone, Debug, PartialEq)]
pub struct Excavation {
    /// The cells taken out, in the order of the scan.
    pub removed: Vec<[i32; 3]>,
    /// Those of them that are thrown, the fastest first.
    pub thrown: Vec<Thrown>,
    /// Those that are not thrown: their material is heaped on the rim. In the order of the scan.
    pub uplift: Vec<[i32; 3]>,
    /// The cells made on the rim with their palette index, as many as the uplift, in the order of the scan.
    pub rim: Vec<([i32; 3], u8)>,
}

fn scan(c: &[i32; 3]) -> (i32, i32, i32) {
    (c[2], c[1], c[0])
}

fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

/// A number in [0, 1) that is a function of the seed and the cell.
fn unit(seed: u64, cell: [i32; 3]) -> f64 {
    let mut h = mix(seed);
    for c in cell {
        h = mix(h ^ u64::from(c as u32));
    }
    (h >> 11) as f64 / (1u64 << 53) as f64
}

struct Frame {
    centre: [f64; 3],
    axis: [f64; 3],
    h: f64,
}

impl Frame {
    /// The centre of a cell, in metres.
    fn point(&self, c: [i32; 3]) -> [f64; 3] {
        c.map(|k| (f64::from(k) + 0.5) * self.h)
    }

    /// Height of a point above the plane of the crater along its axis, and its distance from the axis.
    fn polar(&self, p: [f64; 3]) -> (f64, f64) {
        let d: [f64; 3] = std::array::from_fn(|i| p[i] - self.centre[i]);
        let a: f64 = (0..3).map(|i| d[i] * self.axis[i]).sum();
        let r2: f64 = (0..3).map(|i| (d[i] - a * self.axis[i]).powi(2)).sum();
        (a, r2.sqrt())
    }
}

/// Ordered by total order on floats, for the ranks of the rim.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rank {
    distance: f64,
    height: f64,
    scan: (i32, i32, i32),
}

impl Eq for Rank {}

impl PartialOrd for Rank {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Rank {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.distance
            .total_cmp(&other.distance)
            .then(self.height.total_cmp(&other.height))
            .then(self.scan.cmp(&other.scan))
    }
}

/// Takes the crater's bowl out of `before` (cells of `h` metres a side), throws its share and heaps the rest on the rim.
pub fn excavate(before: &Occupancy, crater: &Crater, h: f64, ejection: &Ejection) -> Result<Excavation, String> {
    if !(h.is_finite() && h > 0.0) {
        return Err("a cell must have a positive size".into());
    }
    if !(ejection.share.is_finite() && (0.0..=1.0).contains(&ejection.share)) {
        return Err("the share that is thrown is between 0 and 1".into());
    }
    let spec = crater.spec();
    let frame = Frame { centre: spec.center, axis: crater.axis(), h };
    let (crest, width) = (spec.radius, spec.rim_width);
    let ceiling = crest;
    let in_bowl = |p: [f64; 3]| {
        let (a, r) = frame.polar(p);
        r < crest && a >= -crater.bowl_depth_at(r) && a <= ceiling
    };
    // the filled cells that are in the bowl: only the bricks that the crater's box touches are looked at
    let reach = crest + spec.depth + width + 2.0 * h;
    let (lo, hi): ([i32; 3], [i32; 3]) = (
        std::array::from_fn(|i| ((frame.centre[i] - reach) / h).floor() as i32 - 1),
        std::array::from_fn(|i| ((frame.centre[i] + reach) / h).ceil() as i32 + 1),
    );
    let mut removed: Vec<[i32; 3]> = Vec::new();
    for (key, cells) in before.bricks() {
        let corner = key.map(|k| k * sr_3d::occupancy::BRICK);
        if (0..3).any(|i| corner[i] + sr_3d::occupancy::BRICK <= lo[i] || corner[i] > hi[i]) {
            continue;
        }
        for (i, palette) in cells.iter().enumerate() {
            if *palette == 0 {
                continue;
            }
            let cell = [corner[0] + (i % 8) as i32, corner[1] + ((i / 8) % 8) as i32, corner[2] + (i / 64) as i32];
            if in_bowl(frame.point(cell)) {
                removed.push(cell);
            }
        }
    }
    removed.sort_by_key(scan);
    let n = removed.len();
    let thrown_count = ((ejection.share * n as f64).round() as usize).min(n);
    // the middle first: by distance from the axis, then by height, then by the scan
    let key = |c: &[i32; 3]| {
        let (a, r) = frame.polar(frame.point(*c));
        (r, a, scan(c))
    };
    let mut ranked: Vec<[i32; 3]> = removed.clone();
    ranked.sort_by(|x, y| {
        let (kx, ky) = (key(x), key(y));
        kx.0.total_cmp(&ky.0).then(kx.1.total_cmp(&ky.1)).then(kx.2.cmp(&ky.2))
    });
    let (plane_u, _) = crater.plane_basis();
    let mut thrown = Vec::with_capacity(thrown_count);
    for (i, cell) in ranked.iter().take(thrown_count).enumerate() {
        let p = frame.point(*cell);
        let d: [f64; 3] = std::array::from_fn(|k| p[k] - frame.centre[k]);
        let a: f64 = (0..3).map(|k| d[k] * frame.axis[k]).sum();
        let radial: [f64; 3] = std::array::from_fn(|k| d[k] - a * frame.axis[k]);
        let length = radial.iter().map(|v| v * v).sum::<f64>().sqrt();
        let along = if length > 1e-12 { radial.map(|v| v / length) } else { plane_u };
        let speed = ejection.speeds.at((i as f64 + 0.5) / thrown_count as f64);
        let elevation = (ejection.angle + ejection.spread * (2.0 * unit(ejection.seed, *cell) - 1.0)).to_radians();
        let velocity = std::array::from_fn(|k| speed * (elevation.cos() * along[k] + elevation.sin() * frame.axis[k]));
        thrown.push(Thrown { cell: *cell, palette: before.get(*cell), position: p, velocity });
    }
    let mut uplift: Vec<[i32; 3]> = ranked[thrown_count..].to_vec();
    uplift.sort_by_key(scan);
    let rim = heap_rim(before, crater, &frame, &removed, uplift.len())?
        .into_iter()
        .zip(uplift.iter())
        .map(|(cell, source)| (cell, before.get(*source)))
        .collect::<Vec<_>>();
    let mut rim = rim;
    rim.sort_by_key(|(c, _)| scan(c));
    Ok(Excavation { removed, thrown, uplift, rim })
}

/// The `count` cells to add on the rim, in the order they are added.
fn heap_rim(
    before: &Occupancy,
    crater: &Crater,
    frame: &Frame,
    removed: &[[i32; 3]],
    count: usize,
) -> Result<Vec<[i32; 3]>, String> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let spec = crater.spec();
    let (crest, width, h) = (spec.radius, spec.rim_width, frame.h);
    let gone: BTreeSet<(i32, i32, i32)> = removed.iter().map(scan).collect();
    let solid = |c: [i32; 3], chosen: &BTreeSet<(i32, i32, i32)>| {
        (before.get(c) != 0 && !gone.contains(&scan(&c))) || chosen.contains(&scan(&c))
    };
    let eligible = |c: [i32; 3]| -> Option<Rank> {
        if before.get(c) != 0 || gone.contains(&scan(&c)) {
            return None;
        }
        let (a, r) = frame.polar(frame.point(c));
        let in_bowl = r < crest && a >= -crater.bowl_depth_at(r) && a <= crest;
        if in_bowl || (r - crest).abs() > width + h || a > spec.rim_height + 2.0 * h {
            return None;
        }
        Some(Rank { distance: (r - crest).abs(), height: a, scan: scan(&c) })
    };
    let neighbours = |c: [i32; 3]| {
        (0..3).flat_map(move |a| {
            [-1i32, 1].into_iter().map(move |d| {
                let mut n = c;
                n[a] += d;
                n
            })
        })
    };
    // the cells of the region, and which of them stand on the ground
    let reach = crest + width + spec.rim_height + 3.0 * h;
    let (lo, hi): ([i32; 3], [i32; 3]) = (
        std::array::from_fn(|i| ((frame.centre[i] - reach) / h).floor() as i32),
        std::array::from_fn(|i| ((frame.centre[i] + reach) / h).ceil() as i32),
    );
    let mut heap: BinaryHeap<Reverse<(Rank, [i32; 3])>> = BinaryHeap::new();
    let mut queued: BTreeSet<(i32, i32, i32)> = BTreeSet::new();
    let none = BTreeSet::new();
    for k in lo[2]..=hi[2] {
        for j in lo[1]..=hi[1] {
            for i in lo[0]..=hi[0] {
                let cell = [i, j, k];
                if let Some(rank) = eligible(cell) {
                    if neighbours(cell).any(|n| solid(n, &none)) && queued.insert(scan(&cell)) {
                        heap.push(Reverse((rank, cell)));
                    }
                }
            }
        }
    }
    let mut chosen: BTreeSet<(i32, i32, i32)> = BTreeSet::new();
    let mut order = Vec::with_capacity(count);
    while order.len() < count {
        let Some(Reverse((_, cell))) = heap.pop() else {
            return Err(format!(
                "the rim has room for {} of the {count} cells that are to be heaped on it",
                order.len()
            ));
        };
        chosen.insert(scan(&cell));
        order.push(cell);
        for next in neighbours(cell) {
            if let Some(rank) = eligible(next) {
                if queued.insert(scan(&next)) {
                    heap.push(Reverse((rank, next)));
                }
            }
        }
    }
    Ok(order)
}
