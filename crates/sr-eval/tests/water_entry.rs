//! The cavity a body makes of the water it enters: nothing is authored about when, where or how
//! big, and the far wave grows with the energy it brings.
use sr_eval::{Evaluator, FrameGraph};

struct Setup {
    /// Speed toward the water, scene units (metres) per second.
    speed: f64,
    mass: f64,
    /// Rest depth of the water.
    depth: f64,
    source: &'static str,
    radius: f64,
}
impl Default for Setup {
    fn default() -> Self {
        Setup { speed: 20.0, mass: 2000.0, depth: 12.0, source: r#"<waterImpulse source="rock"/>"#, radius: 2.0 }
    }
}
impl Setup {
    fn xml(&self) -> String {
        format!(
            r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
              <object3D id="rock" primitive="sphere" radius="{radius}" y="-8">
                <rigidBody shape="sphere" mass="{mass}" velocityY="{speed}" restitution="0" linearDamping="0" angularDamping="0"/>
              </object3D>
              <ocean id="sea" bedResponse="hydrostatic" width="128" depth="128" cellSize="2" bottomDepth="{depth}" dt="0.0416666666666667" boundary="closed" colliders="rock">{source}</ocean>
            </composition><physics gravityY="0" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/></scene>"#,
            radius = self.radius,
            mass = self.mass,
            speed = self.speed,
            depth = self.depth,
            source = self.source,
        )
    }
    fn evaluator(&self) -> Evaluator {
        let doc =
            sr_model::load_str(&self.xml(), &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
        Evaluator::new(&doc, &Default::default()).unwrap()
    }
}
fn sea(frame: &FrameGraph) -> &sr_eval::ocean::SimOcean {
    frame.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap()
}
fn at(ev: &Evaluator, t: f64) -> FrameGraph {
    let frame = ev.evaluate(t);
    assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?} {:?}", frame.problems, frame.failures);
    frame
}
fn cells(frame: &FrameGraph) -> &[sr_sim::ocean::Cell] {
    &sea(frame).frame.cells
}
fn same(a: &FrameGraph, b: &FrameGraph) -> bool {
    cells(a) == cells(b) && sea(a).key == sea(b).key
}
const NO_SOURCE: &str = "";

/// The largest difference, over the ring 20 to 40 units out and up to `until` seconds, between
/// the water with the cavity and without it: the wave of the cavity alone.
fn cavity_wave(setup: Setup, until: f64) -> f64 {
    let with = Setup { source: r#"<waterImpulse source="rock"/>"#, ..setup }.evaluator();
    let without = Setup { source: NO_SOURCE, ..setup }.evaluator();
    let mut peak = 0.0f64;
    let mut t = 0.0;
    while t < until {
        t += 0.25;
        let (a, b) = (at(&with, t), at(&without, t));
        for i in 0..64 * 64 {
            let (x, z) = (((i % 64) as f64 + 0.5 - 32.0) * 2.0, ((i / 64) as f64 + 0.5 - 32.0) * 2.0);
            if (20.0..40.0).contains(&x.hypot(z)) {
                peak = peak.max((cells(&a)[i].depth - cells(&b)[i].depth).abs());
            }
        }
    }
    peak
}

#[test]
fn nothing_happens_before_the_body_enters_and_the_water_changes_at_the_instant_it_does() {
    let with = Setup::default().evaluator();
    let without = Setup { source: NO_SOURCE, ..Setup::default() }.evaluator();
    // the lowest point of the rock reaches the water at y = 0 after 6 / 20 s
    for t in [0.1, 0.25, 0.29] {
        assert!(same(&at(&with, t), &at(&without, t)), "before the entry, at {t}");
    }
    // the cavity forms over the law's time (about 0.4 s for this body), so the water over the entry point
    // sinks below what it would be without it more and more
    let centre = 32 * 64 + 32;
    let gap = |t: f64| cells(&at(&without, t))[centre].depth - cells(&at(&with, t))[centre].depth;
    let (early, late) = (gap(0.34), gap(0.6));
    assert!(!same(&at(&with, 0.34), &at(&without, 0.34)), "after the entry");
    assert!(late > 0.05, "the cavity is at the entry point: {late}");
    assert!(early < late, "it grows: {early} then {late}");
}

#[test]
fn the_cavity_moves_water_and_keeps_all_of_it() {
    let ev = Setup::default().evaluator();
    for t in [0.31, 0.6, 1.5, 3.0] {
        let total: f64 = cells(&at(&ev, t)).iter().map(|c| c.depth).sum();
        assert!((total - 12.0 * 4096.0).abs() < 1e-8 * total, "water at {t}: {total}");
    }
}

#[test]
fn the_far_wave_of_the_cavity_grows_with_the_speed_and_with_the_mass_of_the_body() {
    let by_speed: Vec<f64> = [10.0, 20.0, 30.0]
        .iter()
        .map(|&speed| cavity_wave(Setup { speed, depth: 100.0, ..Setup::default() }, 3.9))
        .collect();
    let by_mass: Vec<f64> = [500.0, 2000.0, 8000.0]
        .iter()
        .map(|&mass| cavity_wave(Setup { mass, depth: 100.0, ..Setup::default() }, 3.9))
        .collect();
    println!("CAVITY far wave by speed 10, 20, 30: {by_speed:?}; by mass 500, 2000, 8000: {by_mass:?}");
    assert!(by_speed.windows(2).all(|w| w[0] < w[1]), "{by_speed:?}");
    assert!(by_mass.windows(2).all(|w| w[0] < w[1]), "{by_mass:?}");
}

#[test]
fn a_body_that_starts_in_the_water_or_never_reaches_it_or_enters_elsewhere_makes_no_cavity() {
    let variants = [
        r#"y="2""#,          // already in the water
        r#"y="-8" x="500""#, // enters outside the ocean
    ];
    for placement in variants {
        let xml = Setup::default().xml().replacen(r#"y="-8""#, placement, 1);
        let with = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let xml = xml.replace(r#"<waterImpulse source="rock"/>"#, "");
        let without = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let (with, without) = (
            Evaluator::new(&with, &Default::default()).unwrap(),
            Evaluator::new(&without, &Default::default()).unwrap(),
        );
        for t in [0.5, 1.5] {
            assert!(same(&at(&with, t), &at(&without, t)), "{placement} at {t}");
        }
    }
    // moving away from the water
    let up = Setup { speed: -20.0, ..Setup::default() };
    let none = Setup { speed: -20.0, source: NO_SOURCE, ..Setup::default() };
    assert!(same(&at(&up.evaluator(), 1.0), &at(&none.evaluator(), 1.0)));
}

#[test]
fn a_body_that_enters_shallow_water_is_not_an_error_and_never_empties_a_column() {
    let ev = Setup { depth: 1.5, ..Setup::default() }.evaluator();
    for t in [0.31, 0.5, 1.0, 2.0] {
        let frame = at(&ev, t);
        assert!(cells(&frame).iter().all(|c| c.depth >= 0.0), "{t}");
        let total: f64 = cells(&frame).iter().map(|c| c.depth).sum();
        assert!((total - 1.5 * 4096.0).abs() < 1e-8 * total, "water at {t}: {total}");
    }
}

#[test]
fn scrubbing_gives_the_same_water() {
    let ev = Setup::default().evaluator();
    let first = at(&ev, 2.0);
    at(&ev, 0.31);
    at(&ev, 0.1);
    at(&ev, 1.0);
    let again = at(&ev, 2.0);
    assert!(same(&first, &again));
    assert!(same(&first, &at(&Setup::default().evaluator(), 2.0)), "an evaluator that did not scrub agrees");
}
