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
    let kernel =
        Crater::conserving(spec, Budget { volume: c.volume, ejecta: c.ejecta_volume, bulking: Some(1.0) }).unwrap();
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

/// A slab of ground 30 m by 30 m by 10 m under the plane y = 0, with palette indices 1 to 3 in a pattern, and a pillar of 15 m on it near the
/// axis (so that something stands over the plane inside the crater, and something stands higher than the crater reaches).
fn ground() -> Vec<([i32; 3], u8)> {
    let mut cells = Vec::new();
    for k in -60..60 {
        for j in 0..40 {
            for i in -60..60 {
                cells.push(([i, j, k], 1 + (i32::rem_euclid(i + j + k, 3)) as u8));
            }
        }
    }
    for k in 4..6 {
        for j in -60..0 {
            for i in 4..6 {
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

/// The surface the grown crater leaves over the original ground at distance `r` from the axis, along the axis (positive out of the ground).
fn surface(law: &Law, r: f64) -> f64 {
    -law.kernel.bowl_depth_at(r) + law.kernel.rim_height_at(r)
}

#[test]
fn the_cells_above_the_surface_of_the_crater_are_taken_out_to_the_reach_of_the_crater_and_they_hold_the_volume_of_the_law(
) {
    let law = law();
    let cells = ground();
    let before = occupancy(&cells);
    let x = excavate(&before, &law.kernel, H, &ejection(&law)).unwrap();
    let n = x.removed.len();
    let spec = law.kernel.spec();
    // exactly the filled cells inside the crest radius whose centres are over the grown surface of the crater and under one crest radius
    // above its plane (brute force over the whole ground)
    let expected: BTreeSet<[i32; 3]> = cells
        .iter()
        .map(|c| c.0)
        .filter(|c| {
            let (r, a) = polar(*c);
            r < spec.radius + spec.rim_width && a >= surface(&law, r) && a <= spec.radius
        })
        .collect();
    assert_eq!(x.removed.iter().copied().collect::<BTreeSet<_>>(), expected);
    // in the order of the scan
    let mut scan = x.removed.clone();
    scan.sort_by_key(|c| (c[2], c[1], c[0]));
    assert_eq!(x.removed, scan);
    // the pillar: what stands over the plane in the crater goes, up to the reach, and what is higher stays
    let pillar_top = cells.iter().filter(|c| c.0[0] >= 4 && c.0[0] < 6 && c.0[1] < 0).map(|c| c.0).collect::<Vec<_>>();
    let (inside, above): (Vec<_>, Vec<_>) = pillar_top.iter().partition(|c| polar(**c).1 <= spec.radius);
    assert!(inside.iter().all(|c| x.removed.contains(c)), "the pillar inside the reach of the crater goes");
    assert!(above.iter().all(|c| !x.removed.contains(c)), "and the part above it stays");
    assert!(!inside.is_empty() && !above.is_empty());
    // the volume: pinned for the authored rock and 0.25 m cells, and within a hundredth of a percent of the law's
    let volume = n as f64 * H * H * H;
    println!("VOXEL CRATER {n} cells, volume {volume:.3} m3 against the law's {:.3}", law.volume);
    assert_eq!(n, 6460, "the number of cells the crater takes out (pinned from the run: the law's volume is {:.3} m3, which is 6459.5 cells)", law.volume);
    assert!((volume - 100.9375).abs() < 1e-9, "{volume}");
    assert!(
        (volume - law.volume).abs() < 0.03 * law.volume,
        "within 3 percent of the law's volume: {volume} against {}",
        law.volume
    );
}

#[test]
fn what_is_taken_out_is_thrown_or_heaped_in_counts_of_cells_and_the_materials_are_all_accounted_for() {
    let law = law();
    let cells = ground();
    let before = occupancy(&cells);
    let x = excavate(&before, &law.kernel, H, &ejection(&law)).unwrap();
    let n = x.removed.len();
    assert_eq!(
        (x.thrown.len(), x.uplift.len(), x.rim.len()),
        (5168, 1292, 1292),
        "0.8 of {n}, the rest, and as many on the rim"
    );
    let mut parts: Vec<[i32; 3]> = x.thrown.iter().map(|t| t.cell).chain(x.uplift.iter().copied()).collect();
    parts.sort_by_key(|c| (c[2], c[1], c[0]));
    assert_eq!(parts, x.removed, "every removed cell is thrown or uplifted, once");
    // the cells after: the ground less the removed plus the rim; the materials that were there are there or thrown (the palette of the rim
    // cells is that of the uplifted cells)
    let removed: BTreeSet<[i32; 3]> = x.removed.iter().copied().collect();
    let mut after: Vec<u8> = cells.iter().filter(|c| !removed.contains(&c.0)).map(|c| c.1).collect();
    after.extend(x.rim.iter().map(|r| r.1));
    let mut all = histogram(after.iter().copied());
    for (k, v) in histogram(x.thrown.iter().map(|t| t.palette)) {
        *all.entry(k).or_insert(0) += v;
    }
    assert_eq!(all, histogram(cells.iter().map(|c| c.1)), "no cell and no material is made or lost");
    let rim: BTreeSet<[i32; 3]> = x.rim.iter().map(|r| r.0).collect();
    assert_eq!(rim.len(), x.rim.len(), "no cell twice");
    for c in &rim {
        assert_eq!(before.get(*c), 0, "{c:?} was not empty");
        assert!(!removed.contains(c));
    }
    // each cell of the rim stands on the ground or on another cell of the rim: the cell under it, along minus the axis (which is +y here)
    let solid = |c: &[i32; 3]| (before.get(*c) != 0 && !removed.contains(c)) || rim.contains(c);
    for c in &rim {
        assert!(solid(&[c[0], c[1] + 1, c[2]]), "{c:?} has nothing under it");
    }
    // the rim has the shape of the law's: a level set of the height over the floor of the bowl in units of the height of the rim, up to the
    // level `rim_scale`, which is near 1 for the kernel that holds the volume (the law's own rim)
    let level = |c: &[i32; 3]| {
        let (r, a) = polar(*c);
        (a + law.kernel.bowl_depth_at(r)) / law.kernel.rim_height_at(r)
    };
    println!("VOXEL CRATER rim level {:.4}", x.rim_scale);
    assert!(x.rim_scale > 0.7 && x.rim_scale < 1.4, "{}", x.rim_scale);
    assert!(rim.iter().all(|c| level(c) <= x.rim_scale + 1e-9), "a cell over the level");
    // and every cell that stands on something and is under the level is in it: nothing is left out of the level set. The brute force looks at
    // every empty cell of the box that the rim could be in
    let spec = law.kernel.spec();
    let reach = ((spec.radius + spec.rim_width) / H).ceil() as i32 + 2;
    let mut left_out = 0;
    for k in -reach..=reach {
        for j in -reach..=reach {
            for i in -reach..=reach {
                let c = [i, j, k];
                let (r, _) = polar(c);
                if before.get(c) != 0
                    || removed.contains(&c)
                    || rim.contains(&c)
                    || r >= spec.radius + spec.rim_width
                    || law.kernel.rim_height_at(r) == 0.0
                {
                    continue;
                }
                if level(&c) < x.rim_scale - 1e-9 && solid(&[i, j + 1, k]) {
                    left_out += 1;
                }
            }
        }
    }
    assert_eq!(left_out, 0, "cells under the level that stand on something are not in the rim");
    // the lip is also inside the crest radius, as the law's surface has it (half of the rim stands within it)
    let inside = rim.iter().filter(|c| polar(**c).0 < spec.radius).count();
    assert!(inside > rim.len() / 4 && inside < 3 * rim.len() / 4, "{inside} of {} cells inside the crest", rim.len());
}

#[test]
fn the_thrown_cells_leave_with_the_speeds_of_the_law_the_shallowest_and_the_nearest_the_axis_first_and_the_same_every_time(
) {
    let law = law();
    let cells = ground();
    let before = occupancy(&cells);
    let e = ejection(&law);
    let x = excavate(&before, &law.kernel, H, &e).unwrap();
    let t = x.thrown.len();
    let speed = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    // the middle first: the distance from the axis does not fall; at one distance the shallower cell first, since the material that is launched
    // fastest is the nearest the surface (the Z-model of Maxwell: the speed falls with the depth of the launch)
    for w in x.thrown.windows(2) {
        let ((r0, a0), (r1, a1)) = (polar(w[0].cell), polar(w[1].cell));
        assert!(r1 >= r0 - 1e-12, "ordered by distance from the axis");
        if (r1 - r0).abs() < 1e-12 {
            assert!(a1 <= a0 + 1e-12, "at one distance the shallower first: {a0} then {a1}");
        }
        assert!(speed(w[1].velocity) <= speed(w[0].velocity) + 1e-9, "the speed does not rise along the order");
    }
    // the distribution of the thrown mass over speed is the law's: for several speeds, the share of the mass of the law's particles that is
    // faster against the share of the thrown cells that are
    let speeds: Vec<f64> = x.thrown.iter().map(|c| speed(c.velocity)).collect();
    let total: f64 = law.list.iter().map(|p| p.mass).sum();
    let mut sorted: Vec<(f64, f64)> = law.list.iter().map(|p| (speed(p.velocity), p.mass)).collect();
    sorted.sort_by(|a, b| b.0.total_cmp(&a.0));
    for q in [0.05, 0.2, 0.5, 0.8, 0.95] {
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
    // the fastest and the slowest cell have the extreme speeds of the law, to a cell
    assert!((speeds[0] - sorted[0].0).abs() / sorted[0].0 < 0.05, "{} against {}", speeds[0], sorted[0].0);
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
    assert!(x.rim.len() > x.thrown.len() && x.rim_scale > 1.0);
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
    // a crater in the units of the scene (100 to a metre) with cells in metres: a box of thousands of cells to a side, refused with the
    // reason, not scanned
    let big = Spec {
        radius: 665.0,
        depth: 279.0,
        rim_height: 48.0,
        rim_width: 153.0,
        influence_depth: 1330.0,
        ..law.kernel.spec()
    };
    let kernel = Crater::new(big).unwrap();
    let err = excavate(&before, &kernel, H, &ejection(&law)).unwrap_err();
    assert!(err.contains("units"), "{err}");
    // and a centre so far out that the box is not a box of cells
    let away = Spec { center: [1e12, 0.0, 0.0], ..law.kernel.spec() };
    let err = excavate(&before, &Crater::new(away).unwrap(), H, &ejection(&law)).unwrap_err();
    assert!(err.contains("range") || err.contains("units"), "{err}");
}
