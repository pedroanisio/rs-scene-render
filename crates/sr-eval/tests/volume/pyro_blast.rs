//! A blast in the smoke of a document: the units of the scene, the open domain it needs, and what it does to a puff of smoke.

/// A puff of smoke `distance` metres from the centre of a pyro of 32 metres, and a blast of `energy` joules at the centre half a step after the start,
/// all in a scene of `ppm` scene units to the metre (every length of the scene is the length in metres times `ppm`).
fn scene(ppm: f64, energy: &str, extra_object: &str, boundary: &str) -> String {
    let m = |v: f64| v * ppm;
    format!(
        r##"<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/>
        <composition><object3D id="cloud" primitive="volume" {extra_object}>
          <pyro width="{}" height="{}" depth="{}" voxelSize="{}" dt="0.0005" boundary="{boundary}" pressureIterations="2000" pressureTolerance="1e-8">
            <pyroImpulse shape="sphere" radius="{}" x="{}" time="0" density="1"/>
            <pyroBlast time="0.0005" energy="{energy}"/>
          </pyro><medium blackbody="true"/></object3D></composition>
        <physics pixelsPerMeter="{ppm}"/></scene>"##,
        m(32.0),
        m(32.0),
        m(32.0),
        m(0.5),
        m(1.5),
        m(6.0)
    )
}

fn evaluator(xml: &str) -> sr_eval::Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

/// The density-weighted centre of the smoke along the x axis, in metres, and the smoke on that line (the puff is on it).
fn centroid(ev: &sr_eval::Evaluator, time: f64, ppm: f64) -> (f64, f64) {
    let frame = ev.evaluate(time);
    assert!(frame.problems.is_empty() && frame.failures.is_empty(), "{:?} {:?}", frame.problems, frame.failures);
    let volume =
        frame.nodes.iter().find(|n| &*n.id == "cloud").unwrap().sim_volume.as_ref().expect("a native pyro volume");
    let density = volume.data.grid("density").unwrap();
    let (mut mass, mut moment) = (0.0, 0.0);
    for i in 0..640 {
        let x = (-16.0 + 0.05 * i as f64 + 0.025) * ppm;
        let d = f64::from(density.sample_world([x, 0.0, 0.0]));
        mass += d;
        moment += d * x / ppm;
    }
    (moment / mass, mass)
}

#[test]
fn a_blast_displaces_a_puff_of_smoke_by_the_volume_the_front_swept_in_metres_whatever_the_scene_units() {
    // 3.75e9 J: the front at the end of the step it is released in is at 3.9 m, the puff is at 6 m (r1^3 = 6^3 + 3.9^3: 6.5 m)
    let after = 0.0015;
    let mut moved = Vec::new();
    for ppm in [1.0, 100.0, 37.0] {
        let plain = evaluator(&scene(ppm, "0", "", "open"));
        let blast = evaluator(&scene(ppm, "3.75e9", "", "open"));
        let (x0, _) = centroid(&plain, after, ppm);
        let (x1, _) = centroid(&blast, after, ppm);
        moved.push(x1 - x0);
    }
    // the exact displacement is 0.5 m (to a cell, half a metre, by the advection), and the same in metres in every scene unit
    assert!((moved[0] - 0.5).abs() < 0.25, "{moved:?}");
    assert!((moved[1] - moved[0]).abs() < 1e-3 && (moved[2] - moved[0]).abs() < 1e-3, "{moved:?}");
}

#[test]
fn a_blast_of_no_energy_is_a_document_with_no_blast() {
    let with = evaluator(&scene(1.0, "0", "", "open"));
    let without = evaluator(&scene(1.0, "0", "", "open").replace(r#"<pyroBlast time="0.0005" energy="0"/>"#, ""));
    let key = |ev: &sr_eval::Evaluator| {
        ev.evaluate(0.002).nodes.iter().find(|n| &*n.id == "cloud").unwrap().sim_volume.as_ref().unwrap().key
    };
    assert_eq!(key(&with), key(&without));
}

#[test]
fn a_blast_needs_an_open_domain_and_a_volume_that_is_scaled_the_same_on_every_axis() {
    // the rule of the schema: a closed domain is refused at load
    let closed = sr_model::load_str(&scene(1.0, "1e6", "", "closed"), &sr_model::LoadOptions::without_assets());
    let Err(report) = closed else { panic!("a closed pyro with a blast is refused") };
    assert!(format!("{report:?}").contains("PYC5"), "{report:?}");
    // a volume that is stretched has no sphere in its own units
    let stretched = evaluator(&scene(1.0, "3.75e9", r#"scaleX="2""#, "open"));
    let frame = stretched.evaluate(0.002);
    let said = frame.failures.iter().chain(&frame.problems).any(|f| f.contains("uniformly scaled"));
    assert!(said, "{:?} {:?}", frame.failures, frame.problems);
}
