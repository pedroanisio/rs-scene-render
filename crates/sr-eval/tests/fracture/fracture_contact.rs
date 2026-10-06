//! A body that breaks when another hits it: the fracture is a consequence of the contact, nothing in the document says
//! when, and the pieces are the same in any order of frames and from a baked cache.

fn xml(fracture: &str, speed: f64) -> String {
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="20" duration="4"/>
      <materials><material id="inside" baseColor="#ff0000"/></materials>
      <composition>
        <object3D id="ball" primitive="sphere" radius="1" x="-8">
          <rigidBody mass="1" velocityX="{speed}" linearDamping="0" angularDamping="0" restitution="0"/>
        </object3D>
        <object3D id="rock" primitive="box" width="4" height="4" depth="4">
          <rigidBody mass="8" linearDamping="0" angularDamping="0" restitution="0"/>
          <fracture pieces="4" seed="7" interiorMaterial="inside" {fracture}/>
        </object3D></composition><physics gravityY="0" pixelsPerMeter="1" fixedStep="0.005"/></scene>"##
    )
}

fn evaluator(fracture: &str, speed: f64) -> sr_eval::Evaluator {
    let doc = sr_model::load_str(&xml(fracture, speed), &sr_model::LoadOptions::without_assets())
        .unwrap_or_else(|e| panic!("{e}"));
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

fn broken(ev: &sr_eval::Evaluator, t: f64) -> bool {
    let frame = ev.evaluate(t);
    assert!(
        frame.problems.is_empty() && frame.failures.is_empty(),
        "t = {t}: {:?} {:?}",
        frame.problems,
        frame.failures
    );
    frame.nodes.iter().find(|n| &*n.id == "rock").unwrap().fracture.is_some()
}

#[test]
fn the_body_breaks_at_the_impact_of_its_projectile_and_not_before() {
    let ev = evaluator(r#"source="ball""#, 20.);
    // the ball reaches the box, 5 away, at 0.25 s
    assert!(!broken(&ev, 0.1) && !broken(&ev, 0.2));
    assert!(broken(&ev, 0.4) && broken(&ev, 2.0));
    let frame = ev.evaluate(1.0);
    let split = frame.nodes.iter().find(|n| &*n.id == "rock").unwrap().fracture.as_ref().unwrap();
    assert_eq!(split.poses.len(), 4);
    let mass: f64 = split.geometry.pieces.iter().map(|p| p.mass).sum();
    assert!((mass - 8.).abs() < 1e-9, "{mass}");
}

#[test]
fn a_weak_impact_and_no_impact_break_nothing() {
    // the weight of 1 kg for a step is small; this threshold is far above the impulse of a ball at 20 m/s
    let hard = evaluator(r#"source="ball" minImpulse="1000000""#, 20.);
    assert!(!broken(&hard, 2.0));
    // a ball that does not move never touches it
    assert!(!broken(&evaluator(r#"source="ball""#, 0.), 3.0));
}

#[test]
fn the_pieces_are_the_same_in_any_order_of_frames_and_from_a_baked_cache() {
    let live = evaluator(r#"source="ball""#, 20.);
    let order = [2.0, 0.1, 0.4, 1.0, 0.3, 2.0];
    let first: Vec<_> = order
        .iter()
        .map(|&t| {
            live.evaluate(t).nodes.iter().find(|n| &*n.id == "rock").unwrap().fracture.as_ref().map(|f| f.poses.clone())
        })
        .collect();
    let fresh = evaluator(r#"source="ball""#, 20.);
    for k in [3, 0, 5, 1, 2, 4] {
        let again = fresh
            .evaluate(order[k])
            .nodes
            .iter()
            .find(|n| &*n.id == "rock")
            .unwrap()
            .fracture
            .as_ref()
            .map(|f| f.poses.clone());
        assert_eq!(first[k], again, "t = {}", order[k]);
    }
    // baked: the cache carries the contacts and the pieces
    let bytes = live.physics_cache().unwrap();
    let path = std::env::temp_dir().join(format!("sr-fracture-contact-{}.bin", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    let with_cache =
        xml(r#"source="ball""#, 20.).replace("<physics ", &format!("<physics cache=\"{}\" ", path.display()));
    let doc = sr_model::load_str(&with_cache, &sr_model::LoadOptions::without_assets()).unwrap();
    let baked = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    for &t in &order {
        let a = baked.evaluate(t);
        assert!(a.problems.is_empty(), "t = {t}: {:?}", a.problems);
        let b = live.evaluate(t);
        let get = |f: &sr_eval::FrameGraph| {
            f.nodes
                .iter()
                .find(|n| &*n.id == "rock")
                .unwrap()
                .fracture
                .as_ref()
                .map(|s| (s.poses.clone(), s.enabled.clone()))
        };
        assert_eq!(get(&a), get(&b), "t = {t}");
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_fracture_at_a_time_is_as_it_was() {
    let ev = evaluator(r#"at="1""#, 0.);
    assert!(!broken(&ev, 0.5) && broken(&ev, 1.5));
}
