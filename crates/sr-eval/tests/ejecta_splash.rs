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
