//! A crater in ground made of cells: what the law's bowl takes out, what is thrown and what is heaped on the rim, counted in cells.
#![allow(clippy::needless_range_loop)]

use sr_3d::crater::{Budget, Crater, Spec};
use sr_3d::occupancy::Occupancy;
use sr_eval::voxel_crater::{excavate, Ejection, SpeedLaw};
use sr_sim::cratering::ejecta::{ejecta, Spec as EjectaSpec};
use sr_sim::cratering::{crater, Impact, Material, Target};
use std::collections::{BTreeMap, BTreeSet};

/// Metres a side of a cell.
const H: f64 = 0.25;

/// The authored rock (90 478 kg of 2700 kg/m3, 100 m/s at 60 degrees) in soft rock: the law's crater and the ejecta the engine launches.
struct Law {
    kernel: Crater,
    volume: f64,
    list: Vec<sr_sim::cratering::ejecta::Ejecta>,
}

fn law() -> Law {
    let theta = 60f64.to_radians();
    let impact = Impact { mass: 90_478.0, density: 2700.0, normal_speed: 100.0 * theta.sin() };
    let c = crater(&impact, &Target { material: Material::SoftRock, density: None, strength: None, gravity: 9.80665 })
        .unwrap();
    // the ground is cells with y down: its surface is the plane y = 0 and the crater points along minus y
    let spec = Spec {
        center: [0.0; 3],
        outward: [0.0, -1.0, 0.0],
        radius: c.rim_radius,
        depth: c.depth,
        rim_height: c.rim_height,
        rim_width: c.rim_radius - c.radius,
        influence_depth: 2.0 * c.rim_radius,
    };
    let kernel = Crater::conserving(spec, Budget { volume: c.volume, ejecta: c.ejecta_volume, bulking: None }).unwrap();
    let list = ejecta(&EjectaSpec {
        material: Material::SoftRock,
        body_radius: (3.0 * impact.mass / (4.0 * std::f64::consts::PI * impact.density)).cbrt(),
        body_density: impact.density,
        target_density: 2100.0,
        impact_speed: 100.0,
        velocity_direction: [theta.cos(), -theta.sin(), 0.0],
        normal: [0.0, 1.0, 0.0],
        crater_volume: c.volume,
        crater_radius: c.radius,
        crater_duration: c.duration,
        particles: 4000,
        seed: 20_261_007,
        angle: 45.0,
        angle_spread: 15.0,
    })
    .unwrap();
    Law { kernel, volume: c.volume, list }
}

/// A slab of ground 30 m by 30 m by 10 m under the plane y = 0, with palette indices 1 to 3 in a pattern.
fn ground() -> Vec<([i32; 3], u8)> {
    let mut cells = Vec::new();
    for k in -60..60 {
        for j in 0..40 {
            for i in -60..60 {
                cells.push(([i, j, k], 1 + (i32::rem_euclid(i + j + k, 3)) as u8));
            }
        }
    }
    cells
}

fn occupancy(cells: &[([i32; 3], u8)]) -> Occupancy {
    Occupancy::from_cells(cells.iter().copied()).unwrap()
}

fn centre(c: [i32; 3]) -> [f64; 3] {
    c.map(|k| (f64::from(k) + 0.5) * H)
}

fn ejection(law: &Law) -> Ejection {
    Ejection {
        share: 0.8,
        speeds: SpeedLaw::from_ejecta(&law.list).unwrap(),
        angle: 45.0,
        spread: 15.0,
        seed: 20_261_007,
    }
}

fn histogram(cells: impl Iterator<Item = u8>) -> BTreeMap<u8, usize> {
    let mut h = BTreeMap::new();
    for p in cells {
        *h.entry(p).or_insert(0) += 1;
    }
    h
}

/// The radial distance of a cell centre from the axis of the crater (which is the y axis through the origin) and its height above the plane.
fn polar(c: [i32; 3]) -> (f64, f64) {
    let p = centre(c);
    (p[0].hypot(p[2]), -p[1])
}

#[test]
fn the_cells_the_bowl_takes_out_are_the_ones_under_its_floor_and_they_hold_the_volume_of_the_law_to_the_surface_of_the_bowl(
) {
    let law = law();
    let cells = ground();
    let before = occupancy(&cells);
    let x = excavate(&before, &law.kernel, H, &ejection(&law)).unwrap();
    let n = x.removed.len();
    // exactly the filled cells whose centres are over the floor of the bowl and inside its crest radius (brute force over the whole ground)
    let expected: BTreeSet<[i32; 3]> = cells
        .iter()
        .map(|c| c.0)
        .filter(|c| {
            let (r, a) = polar(*c);
            r < law.kernel.spec().radius && a >= -law.kernel.bowl_depth_at(r)
        })
        .collect();
    assert_eq!(x.removed.iter().copied().collect::<BTreeSet<_>>(), expected);
    // in the order of the scan
    let mut scan = x.removed.clone();
    scan.sort_by_key(|c| (c[2], c[1], c[0]));
    assert_eq!(x.removed, scan);
    // the volume: a cell is in or out by its centre, so the error is at most half a cell on each cell the surface of the bowl crosses
    let (spec, simpson_n) = (law.kernel.spec(), 20_000);
    let step = spec.radius / simpson_n as f64;
    let mut area = 0.0;
    for i in 0..simpson_n {
        let r = (i as f64 + 0.5) * step;
        let slope = (law.kernel.bowl_depth_at(r + 1e-6) - law.kernel.bowl_depth_at(r - 1e-6)) / 2e-6;
        area += 2.0 * std::f64::consts::PI * r * (1.0 + slope * slope).sqrt() * step;
    }
    let bound = area / (H * H) * H * H * H / 2.0;
    let volume = n as f64 * H * H * H;
    println!("VOXEL CRATER {n} cells, volume {volume:.3} m3 against the law's {:.3}; the bound is {bound:.3} (the bowl's surface is {area:.1} m2)", law.volume);
    assert!((volume - law.volume).abs() <= bound, "{volume} against {} with {bound}", law.volume);
    assert!((volume - law.volume).abs() < 0.03 * law.volume, "and in practice within 3 percent: {volume}");
}

#[test]
fn what_is_taken_out_is_thrown_or_heaped_in_counts_of_cells_and_the_materials_are_all_accounted_for() {
    let law = law();
    let cells = ground();
    let before = occupancy(&cells);
    let x = excavate(&before, &law.kernel, H, &ejection(&law)).unwrap();
    let n = x.removed.len();
    let thrown = (0.8f64 * n as f64).round() as usize;
    assert_eq!(x.thrown.len(), thrown, "the law's share, in cells");
    assert_eq!(x.uplift.len(), n - thrown);
    assert_eq!(x.rim.len(), n - thrown, "what is heaped on the rim is what was not thrown");
    let mut parts: Vec<[i32; 3]> = x.thrown.iter().map(|t| t.cell).chain(x.uplift.iter().copied()).collect();
    parts.sort_by_key(|c| (c[2], c[1], c[0]));
    assert_eq!(parts, x.removed, "every removed cell is thrown or uplifted, once");
    // the cells after: the ground less the removed plus the rim; the materials that were there are there or thrown (the palette of the rim
    // cells is that of the uplifted cells)
    let removed: BTreeSet<[i32; 3]> = x.removed.iter().copied().collect();
    let mut after: Vec<u8> = cells.iter().filter(|c| !removed.contains(&c.0)).map(|c| c.1).collect();
    after.extend(x.rim.iter().map(|r| r.1));
    let thrown_palette = x.thrown.iter().map(|t| t.palette);
    let mut all = histogram(after.iter().copied());
    for (k, v) in histogram(thrown_palette) {
        *all.entry(k).or_insert(0) += v;
    }
    assert_eq!(all, histogram(cells.iter().map(|c| c.1)), "no cell and no material is made or lost");
    // the rim is made of empty cells that are not part of the bowl, and each is held up by ground or by another cell of the rim
    let rim: BTreeSet<[i32; 3]> = x.rim.iter().map(|r| r.0).collect();
    assert_eq!(rim.len(), x.rim.len(), "no cell twice");
    let solid = |c: &[i32; 3]| (before.get(*c) != 0 && !removed.contains(c)) || rim.contains(c);
    for c in &rim {
        assert_eq!(before.get(*c), 0, "{c:?} was not empty");
        assert!(!removed.contains(c));
    }
    let mut reached: BTreeSet<[i32; 3]> = BTreeSet::new();
    let mut frontier: Vec<[i32; 3]> = rim
        .iter()
        .copied()
        .filter(|c| {
            (0..3).any(|a| {
                [-1i32, 1].iter().any(|d| {
                    let mut n = *c;
                    n[a] += d;
                    before.get(n) != 0 && !removed.contains(&n)
                })
            })
        })
        .collect();
    while let Some(c) = frontier.pop() {
        if !reached.insert(c) {
            continue;
        }
        for a in 0..3 {
            for d in [-1i32, 1] {
                let mut next = c;
                next[a] += d;
                if rim.contains(&next) && solid(&next) && !reached.contains(&next) {
                    frontier.push(next);
                }
            }
        }
    }
    assert_eq!(reached.len(), rim.len(), "a cell of the rim floats");
    // the rim is where the law puts it: round the crest, over the original surface, not above the height of the rim by more than a cell
    let spec = law.kernel.spec();
    for c in &rim {
        let (r, a) = polar(*c);
        assert!((r - spec.radius).abs() < spec.rim_width + 2.0 * H, "{c:?} at {r} m from the axis");
        assert!(a > 0.0 && a < law.kernel.rim_height_at(r).max(spec.rim_height) + 4.0 * H, "{c:?} at {a} m up");
    }
}

#[test]
fn the_thrown_cells_leave_with_the_speeds_of_the_law_the_fastest_from_the_middle_and_the_same_every_time() {
    let law = law();
    let cells = ground();
    let before = occupancy(&cells);
    let e = ejection(&law);
    let x = excavate(&before, &law.kernel, H, &e).unwrap();
    let t = x.thrown.len();
    let speed = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    // the middle first: the radius of each cell from the axis does not fall, and the speed does not rise
    let radii: Vec<f64> = x.thrown.iter().map(|c| polar(c.cell).0).collect();
    assert!(radii.windows(2).all(|w| w[1] >= w[0] - 1e-12), "ordered by distance from the axis");
    let speeds: Vec<f64> = x.thrown.iter().map(|c| speed(c.velocity)).collect();
    assert!(speeds.windows(2).all(|w| w[1] <= w[0] + 1e-9), "the fastest are nearest the axis");
    // the distribution of the thrown mass over speed is the law's, to a cell: for several speeds, the share of the mass of the law's particles
    // that is faster against the share of the thrown cells that are
    let total: f64 = law.list.iter().map(|p| p.mass).sum();
    let speed_of = |p: &sr_sim::cratering::ejecta::Ejecta| speed(p.velocity);
    for q in [0.05, 0.2, 0.5, 0.8, 0.95] {
        let mut sorted: Vec<(f64, f64)> = law.list.iter().map(|p| (speed_of(p), p.mass)).collect();
        sorted.sort_by(|a, b| b.0.total_cmp(&a.0));
        let (mut sum, mut v) = (0.0, 0.0);
        for (s, m) in &sorted {
            sum += m;
            v = *s;
            if sum >= q * total {
                break;
            }
        }
        let faster = speeds.iter().filter(|s| **s > v).count() as f64 / t as f64;
        assert!(
            (faster - q).abs() < 0.01 + 2.0 / t as f64,
            "the share faster than {v:.2} m/s is {faster:.4} against {q}"
        );
    }
    // out of the ground, away from the axis, in the angles of the law
    for c in &x.thrown {
        let p = centre(c.cell);
        let (h, up) = ([c.velocity[0], c.velocity[2]], -c.velocity[1]);
        let horizontal = h[0].hypot(h[1]);
        let elevation = up.atan2(horizontal).to_degrees();
        assert!((30.0 - 1e-9..=60.0 + 1e-9).contains(&elevation), "{c:?}: {elevation} degrees");
        assert!((c.position[0] - p[0]).abs() < 1e-12 && (c.position[2] - p[2]).abs() < 1e-12);
        if p[0].hypot(p[2]) > 1e-9 {
            // along the radius that goes out through the cell
            let cross = h[0] * p[2] - h[1] * p[0];
            let dot = h[0] * p[0] + h[1] * p[2];
            assert!(cross.abs() < 1e-9 * horizontal * p[0].hypot(p[2]) + 1e-12 && dot > 0.0, "{c:?}");
        }
    }
    // the same whatever the order the cells were put in
    let mut shuffled = cells.clone();
    shuffled.reverse();
    let again = excavate(&occupancy(&shuffled), &law.kernel, H, &e).unwrap();
    assert_eq!(again, x);
}

#[test]
fn nothing_taken_out_or_all_of_it_thrown_and_what_does_not_make_sense_is_an_error() {
    let law = law();
    let before = occupancy(&ground());
    // a crater far from the ground takes nothing out
    let far = Occupancy::from_cells([([500, 500, 500], 1u8)]).unwrap();
    let x = excavate(&far, &law.kernel, H, &ejection(&law)).unwrap();
    assert!(x.removed.is_empty() && x.thrown.is_empty() && x.rim.is_empty());
    // the whole of it thrown leaves no rim
    let all = Ejection { share: 1.0, ..ejection(&law) };
    let x = excavate(&before, &law.kernel, H, &all).unwrap();
    assert_eq!((x.thrown.len(), x.rim.len(), x.uplift.len()), (x.removed.len(), 0, 0));
    // less thrown is more heaped, as far as the rim has room for it: 40 percent thrown leaves 60 percent to heap, which fits
    let less = Ejection { share: 0.4, ..ejection(&law) };
    let x = excavate(&before, &law.kernel, H, &less).unwrap();
    assert_eq!((x.thrown.len() + x.rim.len(), x.rim.len()), (x.removed.len(), x.uplift.len()));
    assert!(x.rim.len() > x.thrown.len());
    // none thrown asks the rim to hold the whole of the bowl, which the rim of the law has not the room for: an error that says so, not a
    // rim that is cut short and a bowl whose material is lost
    let none = Ejection { share: 0.0, ..ejection(&law) };
    let err = excavate(&before, &law.kernel, H, &none).unwrap_err();
    assert!(err.contains("rim"), "{err}");
    for bad in [-0.1, 1.1, f64::NAN] {
        assert!(excavate(&before, &law.kernel, H, &Ejection { share: bad, ..ejection(&law) }).is_err());
    }
    assert!(excavate(&before, &law.kernel, 0.0, &ejection(&law)).is_err());
    assert!(excavate(&before, &law.kernel, f64::NAN, &ejection(&law)).is_err());
    assert!(SpeedLaw::from_ejecta(&[]).is_err());
}
