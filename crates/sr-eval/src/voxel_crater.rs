//! A crater in ground made of cells: what the law's crater takes out, what is thrown and what is heaped on the rim, counted in cells.
//!
//! Pure functions of an [`Occupancy`] and the kernel of the crater ([`sr_3d::crater::Crater`], which holds the law's bowl, its rim and
//! its volume budget), in metres in the frame of the cells: the cell `[i, j, k]` has its centre at `(i + 1/2, j + 1/2, k + 1/2)` cells.
//! The kernel is the conserving one of a crater with a bulking of 1 (`Crater::conserving` with `Budget::bulking = Some(1.0)`), whose bowl
//! holds the law's volume and whose rim holds what the law does not throw out (0.2 of it): a cell is a cell of the same mass, so
//! nothing can be heaped that was not taken out. The kernel's lengths are in a unit that the caller declares (`kernel_unit`, metres a unit: 1 for a kernel in metres, 0.01
//! for one in the units of a scene of 100 to a metre), and the function reads them in metres: a kernel that is in the scene's units and declared as being in metres
//! is a crater of kilometres over cells of a quarter of a metre, whose box is refused (the box that is scanned has a cap) rather than scanned.
//!
//! * **Removal.** The surface the grown crater leaves over the original ground at distance `r` from the axis is the kernel's own:
//!   `S(r) = rim_height_at(r) - bowl_depth_at(r)` along the axis. A filled cell is taken out when its centre is inside the crest radius plus the width of
//!   the rim, over that surface, and under one crest radius above the crater's plane: so what is taken out is the law's bowl to the half cell that a
//!   centre decides, less the wall that the lip stands on, and what stands in the way above the plane.
//! * **The budget, in cells.** Of the `N` cells removed, the law says its share (0.8) is thrown out; the rest is uplift, which is not lost: it is
//!   heaped on the rim, so exactly `N - T` cells are made on the rim, with the palette of the cells that were uplifted. Cells are neither made nor
//!   lost, and no kilogram is.
//! * **Which cells are thrown.** The cells are ordered by their distance from the axis, then by height (the highest first), then by the scan. The
//!   first `T` are thrown: the middle of the crater, and at one distance the shallowest, which is what the law launches fastest (the Z-model of
//!   Maxwell: the speed of the ejecta falls with the depth they come from). The others are the uplift (the wall).
//! * **Speeds.** Not from the cells: the energy of the impact is not in them. The engine's launch model ([`sr_sim::cratering::ejecta`])
//!   gives the distribution of mass over speed, and the thrown cells take it by quantiles: cell `i` of `T` has the speed above which
//!   the fraction `(i + 1/2) / T` of the law's mass lies. So the mass is the cells' to the last bit, the distribution is the law's to a
//!   cell, and nothing depends on a random draw but the launch angle, which is a hash of the seed and the cell.
//! * **The rim.** The law's rim is the surface `S` raised by `rim_height_at(r)` over the bowl: the height of a cell over the floor of the bowl in units
//!   of the height of the rim is its *level*, and the rim is the level set of the first `N - T` cells that are held up, lowest level first: the surface of
//!   the law's rim, scaled by [`Excavation::rim_scale`] (near 1 when the budget is the law's), on both sides of the crest radius. A cell is held up when the
//!   face-neighbour under it (the one most nearly against the axis) is ground or another cell of the rim.
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
    /// The level of the highest cell of the rim: how much the law's rim was scaled to hold the cells that were not thrown (0 with none).
    pub rim_scale: f64,
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

/// The order cells are added to the rim in: the level, then the scan, on a total order of floats.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rank {
    level: f64,
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
        self.level.total_cmp(&other.level).then(self.scan.cmp(&other.scan))
    }
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

    /// The face direction of the lattice most nearly against the axis: what is under a cell.
    fn down(&self) -> [i32; 3] {
        let mut best = ([0, 0, 0], f64::NEG_INFINITY);
        for a in 0..3 {
            for d in [-1i32, 1] {
                let against = -f64::from(d) * self.axis[a];
                if against > best.1 {
                    let mut v = [0; 3];
                    v[a] = d;
                    best = (v, against);
                }
            }
        }
        best.0
    }
}

/// The kernel of the crater read in metres: its lengths are in `unit` metres (1 for a kernel in metres, 0.01 for one in the units of a scene of
/// 100 to a metre), and the cells are in metres.
struct Profile<'a> {
    crater: &'a Crater,
    unit: f64,
}

impl Profile<'_> {
    fn crest(&self) -> f64 {
        self.crater.spec().radius * self.unit
    }
    fn width(&self) -> f64 {
        self.crater.spec().rim_width * self.unit
    }
    fn depth(&self) -> f64 {
        self.crater.spec().depth * self.unit
    }
    fn rim_height(&self) -> f64 {
        self.crater.spec().rim_height * self.unit
    }
    fn centre(&self) -> [f64; 3] {
        self.crater.spec().center.map(|c| c * self.unit)
    }
    /// How far under the original surface the bowl's floor is, at distance `r` metres from the axis.
    fn bowl(&self, r: f64) -> f64 {
        self.unit * self.crater.bowl_depth_at(r / self.unit)
    }
    /// How far over the original surface the rim stands, at distance `r` metres from the axis.
    fn rim(&self, r: f64) -> f64 {
        self.unit * self.crater.rim_height_at(r / self.unit)
    }
}

/// The most cells that the box around a crater may hold: the rim is looked for by a pass over it. A crater in other units than the cells' makes a box
/// of billions, which is refused here and not scanned.
const MAX_SCAN_CELLS: u128 = 1 << 27;
/// The rim is looked for up to this level: a rim that would need more is an error (the law's rim has not the room).
const MAX_LEVEL: f64 = 4.0;

/// The range of cell indices, inclusive, that a span of `centre` plus or minus `reach` metres covers.
fn index_range(centre: f64, reach: f64, h: f64) -> Result<(i32, i32), String> {
    let (lo, hi) = (((centre - reach) / h).floor(), ((centre + reach) / h).ceil());
    let limit = f64::from(sr_3d::occupancy::KEY_LIMIT) - 4.0;
    if !(lo.is_finite() && hi.is_finite()) || lo < -limit || hi > limit {
        return Err(
            "the crater is out of the range of the cells' keys, or in other units than the cells' (metres)".into()
        );
    }
    Ok((lo as i32, hi as i32))
}

/// Takes the crater out of `before` (cells of `h` metres a side), throws its share and heaps the rest on the rim.
pub fn excavate(
    before: &Occupancy,
    crater: &Crater,
    kernel_unit: f64,
    h: f64,
    ejection: &Ejection,
) -> Result<Excavation, String> {
    if !(h.is_finite() && h > 0.0) {
        return Err("a cell must have a positive size".into());
    }
    if !(ejection.angle.is_finite() && ejection.spread.is_finite()) {
        return Err("the launch angle and its spread must be numbers".into());
    }
    if !(kernel_unit.is_finite() && kernel_unit > 0.0) {
        return Err("the unit of the crater's lengths, in metres, must be positive".into());
    }
    if !(ejection.share.is_finite() && (0.0..=1.0).contains(&ejection.share)) {
        return Err("the share that is thrown is between 0 and 1".into());
    }
    let prof = Profile { crater, unit: kernel_unit };
    let frame = Frame { centre: prof.centre(), axis: crater.axis(), h };
    let (crest, width) = (prof.crest(), prof.width());
    let ceiling = crest;
    let surface = |r: f64| prof.rim(r) - prof.bowl(r);
    let taken = |p: [f64; 3]| {
        let (a, r) = frame.polar(p);
        r < crest + width && a >= surface(r) && a <= ceiling
    };
    // the filled cells that the crater takes out: only the bricks that the crater's box touches are looked at
    // the region that is taken out is inside the crest radius plus the width of the rim round the axis and one crest radius up it: along a lattice axis that is
    // as far as the hypotenuse of the two when the axis is slanted
    let reach = crest.hypot(crest + width) + prof.depth().max(prof.rim_height()) + 2.0 * h;
    let (x0, x1) = index_range(frame.centre[0], reach, h)?;
    let (y0, y1) = index_range(frame.centre[1], reach, h)?;
    let (z0, z1) = index_range(frame.centre[2], reach, h)?;
    let (lo, hi) = ([i64::from(x0), i64::from(y0), i64::from(z0)], [i64::from(x1), i64::from(y1), i64::from(z1)]);
    let box_cells: u128 = (0..3).map(|i| (hi[i] - lo[i] + 1) as u128).product();
    if box_cells > MAX_SCAN_CELLS {
        return Err(format!(
            "the box that the crater reaches holds {box_cells} cells, over the {MAX_SCAN_CELLS} that are scanned: the crater is in other units than \
             the unit {kernel_unit} m that it was declared in, or it is too large for cells of {h} m"
        ));
    }
    let brick = i64::from(sr_3d::occupancy::BRICK);
    let mut removed: Vec<[i32; 3]> = Vec::new();
    for (key, cells) in before.bricks() {
        let corner = key.map(|k| i64::from(k) * brick);
        if (0..3).any(|i| corner[i] + brick <= lo[i] || corner[i] > hi[i]) {
            continue;
        }
        for (i, palette) in cells.iter().enumerate() {
            if *palette == 0 {
                continue;
            }
            let cell = [corner[0] + (i % 8) as i64, corner[1] + ((i / 8) % 8) as i64, corner[2] + (i / 64) as i64]
                .map(|c| c as i32);
            if taken(frame.point(cell)) {
                removed.push(cell);
            }
        }
    }
    removed.sort_by_key(scan);
    let n = removed.len();
    let thrown_count = ((ejection.share * n as f64).round() as usize).min(n);
    // the middle first: by distance from the axis, then the highest, then the scan
    let key = |c: &[i32; 3]| {
        let (a, r) = frame.polar(frame.point(*c));
        (r, -a, scan(c))
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
    let (heaped, rim_scale) = heap_rim(before, &prof, &frame, &removed, uplift.len(), (lo, hi))?;
    let mut rim: Vec<([i32; 3], u8)> =
        heaped.into_iter().zip(uplift.iter()).map(|(cell, source)| (cell, before.get(*source))).collect();
    rim.sort_by_key(|(c, _)| scan(c));
    Ok(Excavation { removed, thrown, uplift, rim, rim_scale })
}

/// The `count` cells to add on the rim, in the order they are added, and the level of the last: the lowest level first, each held up.
fn heap_rim(
    before: &Occupancy,
    prof: &Profile,
    frame: &Frame,
    removed: &[[i32; 3]],
    count: usize,
    (lo, hi): ([i64; 3], [i64; 3]),
) -> Result<(Vec<[i32; 3]>, f64), String> {
    if count == 0 {
        return Ok((Vec::new(), 0.0));
    }
    let (crest, width) = (prof.crest(), prof.width());
    let gone: BTreeSet<(i32, i32, i32)> = removed.iter().map(scan).collect();
    let down = frame.down();
    let below = |c: [i32; 3]| [c[0] + down[0], c[1] + down[1], c[2] + down[2]];
    let above = |c: [i32; 3]| [c[0] - down[0], c[1] - down[1], c[2] - down[2]];
    let ground = |c: [i32; 3]| before.get(c) != 0 && !gone.contains(&scan(&c));
    // the level of a cell that the rim could hold: empty, not taken out, over the floor of the bowl, under the highest level looked for
    let level = |c: [i32; 3]| -> Option<f64> {
        if before.get(c) != 0 || gone.contains(&scan(&c)) {
            return None;
        }
        let (a, r) = frame.polar(frame.point(c));
        let rim = prof.rim(r);
        if r >= crest + width || rim <= 0.0 {
            return None;
        }
        let g = (a + prof.bowl(r)) / rim;
        (g > 0.0 && g < MAX_LEVEL).then_some(g)
    };
    let mut heap: BinaryHeap<Reverse<(Rank, [i32; 3])>> = BinaryHeap::new();
    let mut queued: BTreeSet<(i32, i32, i32)> = BTreeSet::new();
    let rank = |g: f64, c: [i32; 3]| Rank { level: g, scan: scan(&c) };
    for k in lo[2]..=hi[2] {
        for j in lo[1]..=hi[1] {
            for i in lo[0]..=hi[0] {
                let cell = [i as i32, j as i32, k as i32];
                if let Some(g) = level(cell) {
                    if ground(below(cell)) && queued.insert(scan(&cell)) {
                        heap.push(Reverse((rank(g, cell), cell)));
                    }
                }
            }
        }
    }
    let mut order = Vec::with_capacity(count);
    let mut last = 0.0;
    while order.len() < count {
        let Some(Reverse((r, cell))) = heap.pop() else {
            return Err(format!(
                "the rim has room for {} of the {count} cells that are to be heaped on it",
                order.len()
            ));
        };
        last = r.level;
        order.push(cell);
        // the cell over this one is held up now
        let up = above(cell);
        if let Some(g) = level(up) {
            if queued.insert(scan(&up)) {
                heap.push(Reverse((rank(g, up), up)));
            }
        }
    }
    Ok((order, last))
}

/// What the ground is, for the body that a crater cuts: the cells' size in scene units, its density, the scene's units to a metre and what becomes of
/// the parts that the crater leaves loose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rock {
    pub size: [f64; 3],
    pub density: f64,
    pub pixels_per_meter: f64,
    pub policy: crate::voxels::Policy,
}

/// A crater in a body of cells as the rigid world is told of it, with what it threw.
#[derive(Clone, Debug, PartialEq)]
pub struct CraterCut {
    pub excavation: Excavation,
    pub cut: crate::voxels::Cut,
}

/// The crater of `crater` (its lengths in `kernel_unit` metres) in the ground `before`, as the cut the rigid world is given: the cells it takes out are
/// destroyed, the rim is heaped on (`added`, before anything leaves, so that a part of the rim that the crater leaves loose goes with its piece), and
/// what is left is divided as [`crate::voxels::cut`] does. The cells are cubes of `rock.size` scene units, a metre being `rock.pixels_per_meter` of them.
/// `revision` is the world's name for the result: it has to be a function of the time of the cut, as the contract of the driver says.
pub fn crater_cut(
    before: &Occupancy,
    crater: &Crater,
    kernel_unit: f64,
    ejection: &Ejection,
    revision: u64,
    rock: &Rock,
    anchored: &dyn Fn(&[i32; 3]) -> bool,
) -> Result<CraterCut, String> {
    let [sx, sy, sz] = rock.size;
    if !(sx.is_finite() && sx > 0.0) || sx != sy || sx != sz {
        return Err("the cells of the ground must be cubes of a positive size".into());
    }
    if !(rock.pixels_per_meter.is_finite() && rock.pixels_per_meter > 0.0) {
        return Err("the scale of the ground must be positive".into());
    }
    if before.count() == 0 {
        return Err("a crater needs ground made of cells".into());
    }
    let excavation = excavate(before, crater, kernel_unit, sx / rock.pixels_per_meter, ejection)?;
    // the ground with the rim on it: the body the cut divides (the rim is on empty cells, so no cell is made twice)
    let after =
        Occupancy::from_cells(before.cells().map(|c| (c, before.get(c))).chain(excavation.rim.iter().copied()))?;
    let mut cut = crate::voxels::cut(
        &after,
        &excavation.removed,
        revision,
        rock.size,
        rock.density,
        rock.pixels_per_meter,
        &rock.policy,
        anchored,
    )?;
    cut.cut.added = excavation.rim.iter().map(|r| r.0).collect();
    cut.cut.added.sort_unstable();
    Ok(CraterCut { excavation, cut })
}
