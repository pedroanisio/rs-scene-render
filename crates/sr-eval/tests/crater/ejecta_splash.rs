//! The ejecta of a crater that fall into an ocean listed by `ocean@splash` are taken out of the particles and
//! given to the ocean by cell and canonical step, written once and read the same every time.

use sr_eval::Evaluator;

const GRAVITY: f64 = 9.80665;
const FLIGHT: f64 = 1.4888;
const MASS: f64 = 90478.0;

/// The rock of the impact scenes arriving at the origin at 100 m/s and `angle` degrees above the ground, the ejecta
/// that its crater throws, and an ocean 1 m under the ground that takes them. `emitter` is added to the emitter's
/// attributes and `ocean` to the ocean's.
fn scene(angle: f64, emitter: &str, ocean: &str) -> String {
    let (vx, vy) = (100.0 * angle.to_radians().cos(), 100.0 * angle.to_radians().sin());
    let launch = vy - GRAVITY * FLIGHT;
    let (x, y) = (-vx * FLIGHT, -2.0 - launch * FLIGHT - 0.5 * GRAVITY * FLIGHT * FLIGHT);
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="8"/><composition>
          <object3D id="rock" primitive="sphere" radius="2" x="{x}" y="{y}">
            <rigidBody shape="sphere" mass="{MASS}" velocityX="{vx}" velocityY="{launch}" restitution="0" linearDamping="0" angularDamping="0"/>
          </object3D>
          <object3D id="ground" primitive="plane" width="160" height="160" segments="160" y="0" rotationX="-90">
            <crater id="pit" source="rock" targetMaterial="softRock"/>
            <rigidBody type="static" shape="auto"/>
          </object3D>
          <particles3D id="debris" rate="0" lifetime="8" dt="0.0416666666666667" gravityY="9.80665" maxParticles="4000" {emitter}>
            <burst crater="pit" count="4000"/>
          </particles3D>
          <ocean id="sea" y="1" width="400" depth="400" cellSize="4" dt="0.0416666666666667" boundary="open" splash="debris" {ocean}/>
        </composition>
        <physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none" fixInternalEdges="true"/></scene>"##
    )
}

fn evaluator(xml: &str) -> Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

/// Everything given to the ocean by its canonical steps 0 to `last`: volume and momentum, and the cells.
fn total(ev: &Evaluator, last: u64) -> (f64, [f64; 2], Vec<u32>, usize) {
    let (mut volume, mut momentum, mut cells, mut entries) = (0.0, [0.0; 2], Vec::new(), 0);
    for step in 0..=last {
        let list = ev.splash_into("sea", step).unwrap_or_else(|e| panic!("step {step}: {e}"));
        assert!(list.windows(2).all(|w| w[0].cell < w[1].cell), "step {step}: sorted by cell, once each");
        for cell in &list {
            volume += cell.volume;
            momentum[0] += cell.momentum[0];
            momentum[1] += cell.momentum[1];
            cells.push(cell.cell);
        }
        entries += list.len();
    }
    (volume, momentum, cells, entries)
}

/// What the ocean is given in its canonical steps `first` to `last`.
fn total_between(ev: &Evaluator, first: u64, last: u64) -> (f64, [f64; 2]) {
    let (mut volume, mut momentum) = (0.0, [0.0; 2]);
    for step in first..=last {
        for cell in ev.splash_into("sea", step).unwrap() {
            volume += cell.volume;
            momentum[0] += cell.momentum[0];
            momentum[1] += cell.momentum[1];
        }
    }
    (volume, momentum)
}

fn alive(ev: &Evaluator, t: f64) -> usize {
    let frame = ev.evaluate(t);
    assert!(
        frame.problems.is_empty() && frame.failures.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.problems,
        frame.failures
    );
    frame.nodes.iter().find(|n| &*n.id == "debris").unwrap().particles3d.as_ref().unwrap().frame.particles.len()
}

/// The volume of the crater, cubic metres, and the density of the soft rock it is in.
fn crater_volume(ev: &Evaluator) -> f64 {
    let frame = ev.evaluate(3.0);
    frame.nodes.iter().find(|n| &*n.id == "ground").unwrap().crater_impact.as_deref().unwrap().law().volume
}

#[test]
fn what_leaves_the_particles_is_what_the_ocean_is_given_to_the_last_bit_of_the_volume() {
    let ev = evaluator(&scene(60.0, "", ""));
    let left = alive(&ev, 7.0);
    assert!(left < 4000, "some fell into the water: {left} of 4000 are left");
    // every particle of the burst shares 0.8 of the crater's volume equally; the ocean steps run to 7 s
    let steps = 167;
    let (volume, _, cells, _) = total(&ev, steps);
    let wanted = (4000 - left) as f64 / 4000.0 * 0.8 * crater_volume(&ev);
    println!(
        "SPLASH {} of 4000 fell in: {volume:.6} m3 against {wanted:.6}, in {} cell-steps",
        4000 - left,
        cells.len()
    );
    assert!(volume > 0.0 && (volume - wanted).abs() < 1e-9 * wanted, "{volume} against {wanted}");
    // all of it inside the 100 x 100 cells of the ocean
    assert!(cells.iter().all(|&c| (c as usize) < 100 * 100));
}

#[test]
fn an_oblique_impact_throws_momentum_downrange_and_a_vertical_one_throws_none_along_it() {
    let along: Vec<f64> = [90.0, 60.0, 30.0]
        .iter()
        .map(|&angle| {
            let ev = evaluator(&scene(angle, "", ""));
            let _ = alive(&ev, 7.0);
            let (volume, momentum, ..) = total(&ev, 167);
            println!("SPLASH {angle} degrees: volume {volume:.4} m3, momentum {momentum:.4?}");
            momentum[0] / volume
        })
        .collect();
    assert!(along[0].abs() < along[1] && along[1] < along[2], "momentum per volume along the rock's travel: {along:?}");
}

#[test]
fn the_same_entries_in_any_order_from_a_fresh_evaluator_and_from_particles_that_kept_no_checkpoint() {
    let xml = scene(60.0, "", "");
    let ev = evaluator(&xml);
    let _ = alive(&ev, 7.0);
    let reference: Vec<_> = (0..=167).map(|s| ev.splash_into("sea", s).unwrap()).collect();
    // another evaluator asked for frames in another order, and one whose particles keep only their first state
    let fresh = evaluator(&xml);
    let discarding = evaluator(&scene(60.0, r#"checkpointMemoryMiB="1""#, ""));
    for time in [7.0, 2.0, 5.5, 0.5, 7.0, 3.3] {
        let _ = alive(&fresh, time);
        let _ = alive(&discarding, time);
    }
    for step in [167, 40, 100, 7, 167, 80] {
        let step = step as usize;
        for (name, other) in [("fresh", &fresh), ("no checkpoint", &discarding)] {
            let got = other.splash_into("sea", step as u64);
            if let Ok(list) = got {
                assert_eq!(list, reference[step], "{name}, step {step}");
            }
        }
    }
    // a step the particles have not reached is an error that says so, not an empty splash
    let early = evaluator(&xml);
    let _ = alive(&early, 3.0);
    let error = early.splash_into("sea", 160).unwrap_err();
    assert!(error.contains("has not been computed"), "{error}");
}

#[test]
fn an_emitter_that_starts_later_gives_the_ocean_what_it_gives_at_the_same_instants_when_it_starts_at_once() {
    // the burst is born after the impact, so the emitter's start before it moves nothing but the clock the particles
    // are counted in; with a whole number of fixed steps in it the fixed steps are the same instants
    let at_once = evaluator(&scene(60.0, "", ""));
    let late = evaluator(&scene(60.0, r#"emissionStart="1""#, ""));
    let (left_at_once, left_late) = (alive(&at_once, 7.0), alive(&late, 7.0));
    assert_eq!(left_at_once, left_late, "the same particles fall");
    let (volume, ..) = total(&late, 167);
    let wanted = (4000 - left_late) as f64 / 4000.0 * 0.8 * crater_volume(&late);
    println!("SPLASH starts at 1 s: {volume:.6} m3 against {wanted:.6}");
    assert!((volume - wanted).abs() < 1e-9 * wanted, "{volume} against {wanted}");
    for step in 0..=167u64 {
        assert_eq!(late.splash_into("sea", step).unwrap(), at_once.splash_into("sea", step).unwrap(), "step {step}");
    }
}

#[test]
fn an_ocean_first_computed_when_its_emitter_is_not_in_the_frame_is_given_what_the_emitter_threw() {
    // the emitter leaves the composition at 3 s, with ejecta still in the air: an ocean whose first frame is after it
    // must not read that nothing fell because nobody said anything
    let xml = scene(60.0, r#"end="3""#, "");
    let early = evaluator(&xml);
    let _ = sea(&early, 2.9);
    let reference: Vec<_> = (0..=68u64).map(|s| early.splash_into("sea", s).unwrap()).collect();
    assert!(reference.iter().any(|l| !l.is_empty()), "something has fallen in by 2.9 s");
    let late = evaluator(&xml);
    let frame = late.evaluate(3.5);
    assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?} {:?}", frame.problems, frame.failures);
    for step in 0..=68u64 {
        match late.splash_into("sea", step) {
            Ok(list) => assert_eq!(list, reference[step as usize], "step {step}"),
            Err(error) => panic!("step {step}: {error}"),
        }
    }
}

#[test]
fn the_momentum_the_ejecta_bring_is_per_unit_of_the_density_of_the_water_they_fall_into() {
    let steps = 167;
    let thousand = evaluator(&scene(60.0, "", ""));
    let explicit = evaluator(&scene(60.0, "", r#"density="1000""#));
    let sea_water = evaluator(&scene(60.0, "", r#"density="1030""#));
    for ev in [&thousand, &explicit, &sea_water] {
        let _ = alive(ev, 7.0);
    }
    let (v0, m0, ..) = total(&thousand, steps);
    let (v1, m1, ..) = total(&explicit, steps);
    let (v2, m2, ..) = total(&sea_water, steps);
    // the default is 1000 to the bit, and the volume of the solid does not depend on the water
    assert_eq!((v0.to_bits(), m0.map(f64::to_bits)), (v1.to_bits(), m1.map(f64::to_bits)));
    assert!((v2 - v0).abs() < 1e-12 * v0, "{v2} against {v0}");
    // a particle falling into water of density 1030 brings the same momentum per unit of a density 1030 times as large
    for axis in 0..2 {
        assert!(
            (m2[axis] * 1030.0 - m0[axis] * 1000.0).abs() < 1e-9 * m0[axis].abs().max(1.0),
            "{m2:?} against {m0:?}"
        );
    }
}

#[test]
fn without_the_attribute_the_particles_are_as_they_were_and_with_it_they_are_fewer() {
    let with = evaluator(&scene(60.0, "", ""));
    let without = evaluator(&scene(60.0, "", "").replace(r#" splash="debris""#, ""));
    assert_eq!(alive(&without, 7.0), 4000, "nothing takes them out");
    assert!(alive(&with, 7.0) < 4000);
    // and until the first has fallen in they are the same particles, to the bit
    let bits = |ev: &Evaluator| {
        let frame = ev.evaluate(1.7);
        let p = &frame.nodes.iter().find(|n| &*n.id == "debris").unwrap().particles3d.as_ref().unwrap().frame.particles;
        p.iter().flat_map(|q| q.position.iter().chain(&q.velocity).map(|v| v.to_bits())).collect::<Vec<_>>()
    };
    assert_eq!(bits(&with), bits(&without));
}

#[test]
fn a_larger_crater_gives_the_water_more() {
    let volumes: Vec<f64> = [60.0, 100.0, 150.0]
        .iter()
        .map(|&speed| {
            let ev = evaluator(&scene_at(speed));
            let _ = alive(&ev, 7.0);
            total(&ev, 167).0
        })
        .collect();
    println!("SPLASH volume by speed: {volumes:?}");
    assert!(volumes.windows(2).all(|w| w[0] < w[1]), "{volumes:?}");
}

/// The scene with the rock arriving at `speed` m/s at 60 degrees.
fn scene_at(speed: f64) -> String {
    let (vx, vy) = (speed * 0.5, speed * 60f64.to_radians().sin());
    let launch = vy - GRAVITY * FLIGHT;
    let (x, y) = (-vx * FLIGHT, -2.0 - launch * FLIGHT - 0.5 * GRAVITY * FLIGHT * FLIGHT);
    let base = scene(60.0, "", "");
    let start = base.find(r#"<object3D id="rock""#).unwrap();
    let end = base[start..].find("</object3D>").unwrap() + start + "</object3D>".len();
    let rock = format!(
        r#"<object3D id="rock" primitive="sphere" radius="2" x="{x}" y="{y}">
            <rigidBody shape="sphere" mass="{MASS}" velocityX="{vx}" velocityY="{launch}" restitution="0" linearDamping="0" angularDamping="0"/>
          </object3D>"#
    );
    format!("{}{rock}{}", &base[..start], &base[end..])
}

#[test]
fn an_emitter_belongs_to_one_ocean_and_an_ocean_that_takes_particles_has_no_scale() {
    let two = scene(60.0, "", "").replace(
        "</composition>",
        r#"<ocean id="other" y="1" width="400" depth="400" cellSize="4" splash="debris"/></composition>"#,
    );
    let frame = evaluator(&two).evaluate(2.0);
    assert!(frame.failures.iter().any(|m| m.contains("debris") && m.contains("2 oceans")), "{:?}", frame.failures);
    let scaled = scene(60.0, "", r#"scaleX="2""#);
    let frame = evaluator(&scaled).evaluate(2.0);
    assert!(frame.failures.iter().any(|m| m.contains("no scale")), "{:?}", frame.failures);
}

// ---- the ocean's side: what the ocean does with what the particles bring ----

const CELLS: usize = 100;
const CELL: f64 = 4.0;
const STEP: f64 = 0.0416666666666667;

/// The same scene with a closed basin, so that all the water that is in it stays in it.
fn basin(angle: f64) -> String {
    scene(angle, "", "").replace(r#"boundary="open""#, r#"boundary="closed""#)
}

/// The depths and velocities of every cell of the ocean at `t`.
fn sea(ev: &Evaluator, t: f64) -> Vec<sr_sim::ocean::Cell> {
    let frame = ev.evaluate(t);
    assert!(
        frame.problems.is_empty() && frame.failures.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.problems,
        frame.failures
    );
    frame.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap().frame.cells.clone()
}

fn water(cells: &[sr_sim::ocean::Cell]) -> f64 {
    water_of(cells, CELL)
}

fn water_of(cells: &[sr_sim::ocean::Cell], cell: f64) -> f64 {
    cells.iter().map(|c| c.depth).sum::<f64>() * cell * cell
}

fn momentum(cells: &[sr_sim::ocean::Cell]) -> [f64; 2] {
    cells.iter().fold([0.0; 2], |m, c| {
        [m[0] + c.depth * c.velocity[0] * CELL * CELL, m[1] + c.depth * c.velocity[1] * CELL * CELL]
    })
}

/// A solver of the same basin that is given the entries of `ev` by canonical step, the way the evaluator does,
/// with `dense` giving every cell of the basin in every step, those that receive nothing with nothing.
fn reference(ev: &Evaluator, steps: u64, dense: bool) -> Vec<sr_sim::ocean::Cell> {
    use sr_sim::ocean::{Boundary, Cell, Forcing, Ocean, Order, Spec, SplashCell};
    let spec = Spec {
        cells: [CELLS, CELLS],
        origin: [-200.0, -200.0],
        cell_size: CELL,
        dt: STEP,
        gravity: 9.81,
        damping: 0.0,
        dry_tolerance: 1e-10,
        boundary: Boundary::Closed,
        order: Order::First,
        max_bytes: 256 << 20,
        checkpoint_bytes: 64 << 20,
        max_work: 100_000_000,
        moving_bed: true,
        ..Default::default()
    };
    let n = CELLS * CELLS;
    let mut ocean = Ocean::new(spec, vec![10.0; n], vec![Cell { depth: 10.0, velocity: [0.0; 2] }; n], vec![]).unwrap();
    let lists: Vec<Vec<SplashCell>> = (0..steps)
        .map(|s| {
            let list: Vec<SplashCell> = ev
                .splash_into("sea", s)
                .unwrap()
                .into_iter()
                .map(|c| SplashCell { cell: c.cell, volume: c.volume, momentum: c.momentum })
                .collect();
            if !dense {
                return list;
            }
            (0..n as u32)
                .map(|cell| {
                    list.iter().find(|e| e.cell == cell).copied().unwrap_or(SplashCell { cell, ..Default::default() })
                })
                .collect()
        })
        .collect();
    ocean
        .at_driven(steps as f64 * STEP, &mut |time: f64, f: &mut Forcing| {
            f.bed.fill(10.0);
            let k = (time / STEP).round() as i64 - 1;
            if k >= 0 && (time - (k + 1) as f64 * STEP).abs() < 1e-9 && (k as usize) < lists.len() {
                f.splash = lists[k as usize].clone();
            }
            Ok(())
        })
        .unwrap();
    ocean.frame().cells.clone()
}

#[test]
fn the_water_the_ocean_is_given_displaces_water_and_the_basin_keeps_all_of_it() {
    let ev = evaluator(&basin(60.0));
    let _ = alive(&ev, 7.0);
    let before = water(&sea(&ev, 0.0));
    let after_cells = sea(&ev, 7.0);
    let after = water(&after_cells);
    let (given, ..) = total(&ev, 167);
    let tallest = after_cells.iter().map(|c| (c.depth - 10.0).abs()).fold(0.0, f64::max);
    println!(
        "SPLASH basin {before:.6} m3 before, {after:.6} after; {given:.3} m3 given, the surface moved {tallest:.4} m"
    );
    assert!(given > 0.0 && tallest > 1e-3, "the particles moved the water: {tallest}");
    assert!((after - before).abs() < 1e-9 * before, "the volume is the same: {before} then {after}");
}

#[test]
fn the_horizontal_momentum_in_the_ocean_is_the_momentum_given_to_it() {
    for angle in [30.0, 60.0] {
        let ev = evaluator(&basin(angle));
        let _ = alive(&ev, 7.0);
        let (_, given, ..) = total(&ev, 167);
        let water = momentum(&sea(&ev, 7.0));
        println!("SPLASH {angle} degrees: given {given:.5?}, in the ocean {water:.5?}");
        // the waves have not reached the walls 200 m away and the bed is flat: nothing else gives it momentum
        assert!(given[0].abs() > 0.0, "{angle}: {given:?}");
        for axis in 0..2 {
            assert!(
                (water[axis] - given[axis]).abs() < 1e-6 * given[0].hypot(given[1]),
                "{angle} degrees, axis {axis}: {water:?} against {given:?}"
            );
        }
    }
}

#[test]
fn each_canonical_step_is_given_once_and_the_ocean_that_is_driven_with_the_log_is_the_same_to_the_bit() {
    let ev = evaluator(&basin(60.0));
    let _ = alive(&ev, 7.0);
    let bits = |cells: &[sr_sim::ocean::Cell]| {
        cells.iter().flat_map(|c| [c.depth, c.velocity[0], c.velocity[1]]).map(f64::to_bits).collect::<Vec<_>>()
    };
    // exactly at the end of canonical step 167, not between two states
    let got = bits(&sea(&ev, 168.0 * STEP));
    // 168 canonical steps; the entries of a step given twice, or in a step later, would make another ocean
    assert_eq!(bits(&reference(&ev, 168, false)), got, "sparse entries, one event per step");
    assert_eq!(bits(&reference(&ev, 168, true)), got, "every cell of the basin in every step, nothing in most");
    // one step late is another ocean
    assert_ne!(bits(&reference(&ev, 167, false)), got);
}

#[test]
fn the_ocean_is_the_same_at_any_time_asked_in_any_order() {
    let xml = basin(60.0);
    let ev = evaluator(&xml);
    let bits = |ev: &Evaluator, t: f64| {
        sea(ev, t).iter().flat_map(|c| [c.depth, c.velocity[0], c.velocity[1]]).map(f64::to_bits).collect::<Vec<_>>()
    };
    let forward: Vec<_> = [2.0, 4.5, 7.0].iter().map(|&t| bits(&ev, t)).collect();
    let other = evaluator(&xml);
    for (i, t) in [7.0, 2.0, 4.5, 7.0, 2.0].into_iter().enumerate() {
        let index = [2, 0, 1, 2, 0][i];
        assert_eq!(bits(&other, t), forward[index], "t = {t}");
    }
}

#[test]
fn a_time_inside_a_canonical_step_has_what_falls_until_the_step_ends() {
    // the ocean at a time that is not on a step needs the splash of the step that holds it, which ends later
    let ev = evaluator(&basin(60.0));
    for t in [1.013, 3.3333, 6.987, 7.01] {
        let cells = sea(&ev, t);
        assert!(water(&cells) > 0.0, "t = {t}");
    }
    let bits = |ev: &Evaluator, t: f64| {
        sea(ev, t).iter().flat_map(|c| [c.depth, c.velocity[0], c.velocity[1]]).map(f64::to_bits).collect::<Vec<_>>()
    };
    let first = bits(&ev, 5.017);
    let _ = alive(&ev, 7.5);
    assert_eq!(bits(&ev, 5.017), first, "after going later and back");
    assert_eq!(bits(&evaluator(&basin(60.0)), 5.017), first, "from a fresh evaluator");
}

// ---- a beach: the crater is on land and the ocean is next to it ----

/// The rock lands 25 m from the shore, on a ground that ends at the shore, thrown toward the water; the ejecta
/// fall on the ground and bounce on it unless they pass the shore, and the ocean 200 m square begins there.
fn beach(angle: f64) -> String {
    let (vx, vy) = (100.0 * angle.to_radians().cos(), 100.0 * angle.to_radians().sin());
    let launch = vy - GRAVITY * FLIGHT;
    let (x, y) = (-25.0 - vx * FLIGHT, -2.0 - launch * FLIGHT - 0.5 * GRAVITY * FLIGHT * FLIGHT);
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="8"/><composition>
          <object3D id="rock" primitive="sphere" radius="2" x="{x}" y="{y}">
            <rigidBody shape="sphere" mass="{MASS}" velocityX="{vx}" velocityY="{launch}" restitution="0" linearDamping="0" angularDamping="0"/>
          </object3D>
          <object3D id="ground" primitive="plane" width="160" height="160" segments="160" x="-80" y="0" rotationX="-90">
            <crater id="pit" source="rock" targetMaterial="softRock"/>
            <rigidBody type="static" shape="auto"/>
          </object3D>
          <particles3D id="debris" rate="0" lifetime="8" dt="0.0416666666666667" gravityY="9.80665" maxParticles="4000" size="0.34" colliders="ground">
            <burst crater="pit" count="4000"/>
          </particles3D>
          <ocean id="sea" x="100" y="1" width="200" depth="200" cellSize="2" dt="0.0416666666666667" boundary="closed" splash="debris"/>
        </composition>
        <physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none" fixInternalEdges="true"/></scene>"##
    )
}

#[test]
fn on_a_beach_what_lands_on_the_ground_stays_and_what_passes_the_shore_is_given_to_the_water() {
    let ev = evaluator(&beach(60.0));
    let left = alive(&ev, 7.0);
    let (volume, given, cells, _) = total(&ev, 167);
    println!("BEACH {left} of 4000 are on the ground, {} fell in: {volume:.4} m3, momentum {given:.4?}", 4000 - left);
    assert!(left > 0 && left < 4000, "some land and some fall in: {left}");
    // all given inside the ocean's 100 x 100 cells, thrown toward the water
    assert!(cells.iter().all(|&c| (c as usize) < 100 * 100));
    assert!(volume > 0.0 && given[0] > 0.0, "{volume} {given:?}");
    // the closed basin has the water it had
    let (before, after) = (water_of(&sea(&ev, 0.0), 2.0), water_of(&sea(&ev, 7.0), 2.0));
    assert!((after - before).abs() < 1e-9 * before, "{before} then {after}");
    // and, before the waves have been turned back by the shore wall that the first of them fall next to, the
    // momentum it was given: from the first step that has any to the second after it
    let first = (0..=167u64).find(|&s| !ev.splash_into("sea", s).unwrap().is_empty()).expect("some fell in");
    let (_, given) = total_between(&ev, first, first + 1);
    let time = (first + 2) as f64 * STEP;
    let moved = sea(&ev, time)
        .iter()
        .fold([0.0; 2], |m, c| [m[0] + c.depth * c.velocity[0] * 4.0, m[1] + c.depth * c.velocity[1] * 4.0]);
    println!("BEACH first fall in step {first}: given {given:.6?}, in the ocean {moved:.6?}");
    for axis in 0..2 {
        assert!((moved[axis] - given[axis]).abs() < 1e-3 * given[0].hypot(given[1]), "{moved:?} against {given:?}");
    }
}
