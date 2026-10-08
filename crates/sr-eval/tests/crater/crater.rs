#[test]
fn crater_grows_at_the_objects_local_clock_and_seeks_without_accumulating() {
    let xml = r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="4"/><composition><object3D id="ground" primitive="plane" start="1"><crater radius="4" depth="2" rimWidth="1" rimHeight="0.5" start="0.5" end="1.5" curve="linear"/></object3D></composition></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    for (time, depth) in [(1., 0.), (1.5, 0.), (2., 1.), (2.5, 2.), (3., 2.), (2., 1.)] {
        let frame = ev.evaluate(time);
        let crater = sr_eval::crater::at(&frame.nodes[0]).unwrap().unwrap();
        assert_eq!(crater.kernel.map([0.; 3], crater.progress).unwrap().position[2], depth);
    }
}

#[test]
fn rigid_body_falls_into_growing_crater_and_replays() {
    let xml = r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="5"/>
    <composition>
      <object3D id="ground" primitive="plane" width="20" height="20" segments="40" y="5" rotationX="-90">
        <rigidBody type="static"/>
        <crater radius="4" depth="3" rimWidth="1" rimHeight="0.5" start="1" end="2" curve="linear"/>
      </object3D>
      <object3D id="ball" primitive="sphere" radius="0.5" y="3"><rigidBody restitution="0"/></object3D>
    </composition><physics pixelsPerMeter="1" fixedStep="0.01" gravityY="-9.81"/></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let sample = |t| {
        let frame = ev.evaluate(t);
        assert!(frame.problems.is_empty(), "{:?}", frame.problems);
        frame.nodes.iter().find(|n| &*n.id == "ball").unwrap().pose3.unwrap()
    };
    let early = sample(0.9);
    let late = sample(4.0);
    assert!(late[13] > 7.2 && late[13] < 7.6, "body must rest in excavated bowl: y={}", late[13]);
    assert!(early[13] > 4.3 && early[13] < 4.6, "initial surface: y={}", early[13]);
    assert_eq!(sample(0.9), early);
    assert_eq!(sample(4.0), late);

    // A reflected, stretched kinematic surface has the same local profile;
    // object start shifts the crater's local clock, while depth follows scaleZ.
    let transformed = xml
        .replace("type=\"static\"", "type=\"kinematic\"")
        .replace("segments=\"40\"", "segments=\"40\" scaleX=\"-1\" scaleZ=\"2\" start=\"0.5\"");
    let doc = sr_model::load_str(&transformed, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let late = ev.evaluate(4.0);
    assert!(late.problems.is_empty(), "{:?}", late.problems);
    let y = late.nodes.iter().find(|n| &*n.id == "ball").unwrap().pose3.unwrap()[13];
    assert!(y > 10.2 && y < 10.6, "scaled kinematic crater: y={y}");
    assert!(!ev.physics_cache().unwrap().is_empty());
}

#[test]
fn crater_collider_budget_failure_reaches_evaluation_and_baking() {
    let xml = r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="1"/>
    <composition><object3D id="ground" primitive="plane" segments="64">
    <crater maxMemoryMiB="1"/><rigidBody type="static"/></object3D></composition></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    assert!(ev.evaluate(0.5).problems.iter().any(|p| p.contains("crater collider exceeds memory budget")));
    assert!(ev.physics_cache().unwrap_err().contains("crater collider exceeds memory budget"));
}

fn particle_crater(crater: &str, particle: &str) -> sr_eval::Evaluator {
    let xml = format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="5"/>
        <composition><object3D id="ground" primitive="plane" width="20" height="20" segments="40" y="5" rotationX="-90">{crater}</object3D>
        <particles3D id="dust" rate="0" lifetime="5" bounce="0" dt="0.01" colliders="ground" {particle}><burst time="0" count="1"/></particles3D>
        </composition></scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
}

#[test]
fn particles_follow_excavation_and_replay() {
    let ev = particle_crater(
        r#"<crater radius="4" depth="3" rimWidth="1" rimHeight="0.5" start="1" end="2" curve="linear"/>"#,
        r#"y="3" gravityY="9.81" collisionRadius="0.5""#,
    );
    let sample = |t| {
        let f = ev.evaluate(t);
        assert!(f.problems.is_empty(), "t={t}: {:?}", f.problems);
        f.nodes.iter().find(|n| &*n.id == "dust").unwrap().particles3d.as_ref().unwrap().frame.particles[0].clone()
    };
    let early = sample(0.9);
    let late = sample(4.0);
    assert!((early.position[1] - 4.5).abs() < 0.05, "{early:?}");
    // This frictionless particle may slide on the tessellated bowl; it must
    // remain near the excavated floor, rather than the former y=5 surface.
    assert!(late.position[1] > 7.2 && late.position[1] < 7.6, "{late:?}");
    assert_eq!(sample(0.9), early);
    assert_eq!(sample(4.0), late);
}

#[test]
fn particles_receive_rim_velocity() {
    let ev = particle_crater(
        r#"<crater radius="4" depth="0" rimWidth="1" rimHeight="2" end="1" curve="linear"/>"#,
        r#"x="4" y="4.4" collisionRadius="0.1""#,
    );
    let f = ev.evaluate(1.0);
    assert!(f.problems.is_empty(), "{:?}", f.problems);
    let p = &f.nodes.iter().find(|n| &*n.id == "dust").unwrap().particles3d.as_ref().unwrap().frame.particles[0];
    // The outer slope's normal points upward and outward. With zero friction
    // the particle slides outward; it is not attached to the top of the rim.
    assert!(
        p.position[0] > 4.1 && p.position[1] < 4.2 && p.velocity[0] > 0.5 && p.velocity[1] < -0.5,
        "rising rim must transfer outward/upward velocity: {p:?}"
    );
}
