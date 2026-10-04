//! What objects do to the water is attenuated by its depth (`bedResponse="depthFiltered"`), against
//! the long-wave answer of `hydrostatic`.
use sr_eval::{Evaluator, FrameGraph};

fn evaluator(xml: &str) -> Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    Evaluator::new(&doc, &Default::default()).unwrap()
}
fn sea(frame: &FrameGraph) -> &sr_eval::ocean::SimOcean {
    frame.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap()
}
fn at(ev: &Evaluator, t: f64) -> FrameGraph {
    let frame = ev.evaluate(t);
    assert!(
        frame.problems.is_empty() && frame.failures.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.failures,
        frame.problems
    );
    frame
}
/// The surface over the rest level in every column: how far its ordinate (the bed less the depth)
/// is above the level, 0 at rest.
fn elevation(frame: &FrameGraph) -> Vec<f64> {
    let f = &sea(frame).frame;
    f.cells.iter().zip(&f.bed).map(|(c, bed)| c.depth - bed).collect()
}
fn peak(frame: &FrameGraph) -> f64 {
    elevation(frame).into_iter().fold(f64::MIN, f64::max)
}

/// A rock of `radius` with its centre at `y` (scene y downward; the water starts at 0), held still, in water `depth` deep.
fn lake(response: &str, depth: f64, radius: f64, y: f64) -> String {
    format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
          <object3D id="rock" primitive="sphere" radius="{radius}" y="{y}"><rigidBody shape="sphere" mass="1000" restitution="0" linearDamping="0" angularDamping="0"/></object3D>
          <ocean id="sea" bedResponse="{response}" width="128" depth="128" cellSize="0.5" bottomDepth="{depth}" dt="0.0416666666666667" boundary="closed" colliders="rock" maxWork="100000000000"/>
        </composition><physics gravityY="0" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/></scene>"#
    )
}

#[test]
fn a_small_sphere_on_the_bed_of_deep_water_makes_a_wave_of_the_order_of_linear_theory() {
    // linear theory gives 0.024 m for the instant it appears (and 0.021 m at 0.5 s); the long-wave
    // answer lifts the surface by the sphere's thickness, 4 m
    let measure = |response: &str, t: f64| peak(&at(&evaluator(&lake(response, 20.0, 2.0, 18.0)), t));
    let (filtered, hydrostatic) = (measure("depthFiltered", 0.05), measure("hydrostatic", 0.05));
    println!("LAKE at 0.05 s: filtered {filtered:.4}, hydrostatic {hydrostatic:.4}");
    assert!(hydrostatic > 2.5, "the long-wave answer: {hydrostatic}");
    assert!(filtered < 0.06 && filtered > 0.005, "the order of the linear 0.024 m: {filtered}");
    let (filtered, hydrostatic) = (measure("depthFiltered", 0.5), measure("hydrostatic", 0.5));
    println!("LAKE at 0.5 s: filtered {filtered:.4}, hydrostatic {hydrostatic:.4}");
    assert!(filtered < 0.06 && hydrostatic > 4.0 * filtered, "{filtered} against {hydrostatic}");
}

#[test]
fn a_sphere_at_the_surface_is_attenuated_far_less_than_one_on_the_bed() {
    let ratio = |centre: f64| {
        let filtered = peak(&at(&evaluator(&lake("depthFiltered", 20.0, 2.0, centre)), 0.05));
        let hydrostatic = peak(&at(&evaluator(&lake("hydrostatic", 20.0, 2.0, centre)), 0.05));
        filtered / hydrostatic
    };
    let (surface, bed) = (ratio(0.0), ratio(18.0));
    println!("LAKE filtered over hydrostatic: at the surface {surface:.4}, on the bed {bed:.4}");
    assert!(surface > 0.3, "{surface}");
    assert!(bed < 0.03, "{bed}");
    assert!(surface > 10.0 * bed);
}

#[test]
fn the_volume_a_body_displaces_is_the_volume_that_is_lifted() {
    let ev = evaluator(&lake("depthFiltered", 20.0, 2.0, 18.0));
    // at a canonical instant the bed of the frame is the bed of the sample
    let frame = at(&ev, 2.0 * 0.0416666666666667);
    let f = &sea(&frame).frame;
    let lifted: f64 = f.bed.iter().map(|b| (20.0 - b).max(0.0)).sum::<f64>() * 0.25;
    let sphere = 4.0 / 3.0 * std::f64::consts::PI * 8.0;
    println!("LAKE lifted volume {lifted:.2} of {sphere:.2}");
    assert!((lifted - sphere).abs() < 0.04 * sphere, "{lifted} against {sphere}");
}

#[test]
fn a_lake_with_no_body_in_the_water_is_the_same_in_both_responses() {
    // the rock hangs 30 m above the water: nothing is displaced, so nothing is filtered
    let a = at(&evaluator(&lake("depthFiltered", 20.0, 2.0, -30.0)), 1.0);
    let b = at(&evaluator(&lake("hydrostatic", 20.0, 2.0, -30.0)), 1.0);
    assert_eq!(sea(&a).frame, sea(&b).frame);
    assert_eq!(sea(&a).key, sea(&b).key);
    assert!(elevation(&a).iter().all(|e| *e == 0.0));
}

/// A seabed whose crater has radius `radius` and depth `depth`, under `water` of water.
fn crater(response: &str, water: f64, radius: f64, depth: f64) -> String {
    format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="6"/><composition>
          <object3D id="seabed" primitive="plane" width="400" height="400" segments="80" y="{water}" rotationX="-90">
            <crater radius="{radius}" depth="{depth}" rimHeight="0.2" rimWidth="{radius}" start="0.5" end="1.5"/>
          </object3D>
          <ocean id="sea" bedResponse="{response}" width="128" depth="128" cellSize="2" bottomDepth="{water}" dt="0.0416666666666667" boundary="closed" colliders="seabed"/>
        </composition></scene>"#
    )
}

#[test]
fn a_wide_crater_in_shallow_water_is_almost_what_it_was_and_a_narrow_one_in_deep_water_is_not() {
    // how far the surface sinks over the bowl at its deepest, over the run
    let trough = |response: &str, water: f64, radius: f64, depth: f64| {
        let ev = evaluator(&crater(response, water, radius, depth));
        (0..8)
            .map(|k| -elevation(&at(&ev, 1.5 + 0.125 * k as f64)).into_iter().fold(f64::MAX, f64::min))
            .fold(0.0, f64::max)
    };
    let (f, h) = (trough("depthFiltered", 6.0, 40.0, 4.0), trough("hydrostatic", 6.0, 40.0, 4.0));
    println!("CRATER wide and shallow: filtered {f:.4}, hydrostatic {h:.4}");
    assert!((f - h).abs() < 0.12 * h, "{f} against {h}");
    let (f, h) = (trough("depthFiltered", 40.0, 6.0, 4.0), trough("hydrostatic", 40.0, 6.0, 4.0));
    println!("CRATER narrow and deep: filtered {f:.4}, hydrostatic {h:.4}");
    assert!(f < 0.5 * h, "the wave of a narrow crater in deep water is smaller: {f} against {h}");
}

#[test]
fn the_filtered_water_replays_identically_and_does_not_depend_on_the_threads() {
    let xml = lake("depthFiltered", 20.0, 2.0, 18.0);
    let ev = evaluator(&xml);
    let first = at(&ev, 1.5);
    at(&ev, 0.3);
    at(&ev, 0.9);
    let again = at(&ev, 1.5);
    assert_eq!(sea(&first).frame, sea(&again).frame);
    assert_eq!(sea(&first).key, sea(&again).key);
    for threads in [1, 2, 8] {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        let other = pool.install(|| at(&evaluator(&xml), 1.5));
        assert_eq!(sea(&first).key, sea(&other).key, "{threads} threads");
    }
}
