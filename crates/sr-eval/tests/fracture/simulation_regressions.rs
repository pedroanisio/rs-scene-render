use sr_eval::{EvalOptions, Evaluator};

fn doc(body: &str, physics: &str) -> sr_model::Document {
    let xml = format!(
        r#"<scene version="1.1"><project width="400" height="400" fps="30" duration="4"/><assets><image id="img" src="unused.png" width="10" height="10"/></assets><composition>{body}</composition>{physics}</scene>"#
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap()
}

#[test]
fn reject_cycles_through_containers_and_parent_links() {
    let d = doc(r#"<group id="g" parent="child"><layer id="child" asset="img"/></group>"#, "");
    assert!(Evaluator::new(&d, &EvalOptions::default()).is_err(), "combined parent cycle must be rejected");
}

#[test]
fn explicit_parent_can_break_a_container_transform_dependency() {
    let d = doc(
        r#"<group id="g" parent="child" x="10"><layer id="child" parent="outside" asset="img" x="20"/></group><layer id="outside" asset="img" x="100"/>"#,
        "",
    );
    let e = Evaluator::new(&d, &EvalOptions::default()).unwrap();
    // This is acyclic: g -> child -> outside. Containment still controls drawing.
    let f = e.evaluate(0.0);
    assert_eq!(f.nodes.iter().find(|n| &*n.id == "g").unwrap().world.apply([0.0, 0.0]), [130.0, 0.0]);
}

#[test]
fn particles_emit_at_the_simulated_parent_position() {
    let d = doc(
        r#"
      <layer id="body" asset="img" x="100" y="100"><rigidBody velocityX="100" linearDamping="0"/></layer>
      <particleEmitter id="p" parent="body" emitterShape="point" rate="0" speed="0" lifetime="10"><burst time="1" count="1"/></particleEmitter>
    "#,
        r#"<physics gravityY="0" bounds="none"/>"#,
    );
    let ev = Evaluator::new(&d, &EvalOptions::default()).unwrap();
    let f = ev.evaluate(1.01);
    let n = f.nodes.iter().find(|n| &*n.id == "p").unwrap();
    let origin = n.world.apply([0.0, 0.0]);
    let pos = &n.particles.as_ref().unwrap().pos;
    assert_eq!(pos.len(), 1);
    assert!(origin[0] > 190.0);
    assert!((pos[0][0] as f64 - origin[0]).abs() < 2.0, "particle {pos:?}, emitter {origin:?}");
    let late = ev.evaluate(2.0);
    let late_emitter = late.nodes.iter().find(|n| &*n.id == "p").unwrap();
    assert!(late_emitter.world.apply([0.0, 0.0])[0] > 290.0);
    assert_eq!(late_emitter.particles.as_ref().unwrap().pos, *pos, "old particles stay at their birth position");
    let back = ev.evaluate(1.01);
    assert_eq!(back.nodes.iter().find(|n| &*n.id == "p").unwrap().particles.as_ref().unwrap().pos, *pos);
    // A precomputed physics cache must give the same birth positions as a live world.
    use sha2::{Digest, Sha256};
    let bytes = ev.physics_cache().unwrap();
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let cache = std::env::temp_dir().join(format!("sr-parent-emitter-{}.physics", std::process::id()));
    std::fs::write(&cache, bytes).unwrap();
    let cached = doc(
        r#"
      <layer id="body" asset="img" x="100" y="100"><rigidBody velocityX="100" linearDamping="0"/></layer>
      <particleEmitter id="p" parent="body" emitterShape="point" rate="0" speed="0" lifetime="10"><burst time="1" count="1"/></particleEmitter>
    "#,
        &format!(r#"<physics gravityY="0" bounds="none" cache="{}" cacheSha256="{sha}"/>"#, cache.display()),
    );
    let cached = Evaluator::new(&cached, &EvalOptions::default()).unwrap().evaluate(2.0);
    assert!(cached.problems.is_empty(), "{:?}", cached.problems);
    assert_eq!(cached.nodes.iter().find(|n| &*n.id == "p").unwrap().particles.as_ref().unwrap().pos, *pos);
    std::fs::remove_file(cache).unwrap();
}

#[test]
fn remapped_instance_particles_do_not_depend_on_first_frame() {
    let xml = r#"<scene version="1.1"><project width="400" height="400" fps="30" duration="4"/><assets/>
      <symbols><symbol id="s" width="400" height="400" duration="4"><particleEmitter id="p" x="100" y="100" emitterShape="point" rate="0" speed="100" direction="0" lifetime="10"><burst time="0" count="1"/></particleEmitter></symbol></symbols>
      <composition><instance id="i" symbol="s"><timeRemap><key time="0" value="0"/><key time="2" value="4"/></timeRemap></instance></composition></scene>"#;
    let d = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let warm = Evaluator::new(&d, &EvalOptions::default()).unwrap();
    let cold = Evaluator::new(&d, &EvalOptions::default()).unwrap();
    warm.evaluate(0.0);
    let a = warm.evaluate(0.75);
    let b = cold.evaluate(0.75);
    let particles = |f: &sr_eval::FrameGraph| {
        f.nodes.iter().find(|n| n.kind == "particleEmitter").unwrap().particles.as_ref().unwrap().pos.clone()
    };
    assert!(!particles(&a).is_empty());
    assert_eq!(particles(&a), particles(&b));
}

#[test]
fn particle_source_history_handles_remaps_freezes_loops_and_reverse() {
    for (attrs, remap, t, expected_x) in [
        (
            "",
            r#"<timeRemap><key time="0" value="0" interpolation="quad-in"/><key time="1" value="2"/></timeRemap>"#,
            0.75,
            140.35534,
        ),
        (
            "",
            r#"<timeRemap><key time="0" value="0" interpolation="step"/><key time="1" value="2"/></timeRemap>"#,
            1.25,
            205.0,
        ),
        ("", r#"<timeRemap><key time="0" value="0"/><key time="1" value="2"/></timeRemap>"#, 0.75, 117.5),
        ("", r#"<timeRemap><key time="0" value="1"/></timeRemap>"#, 0.75, 180.0),
        (r#"clipIn="1""#, "", 0.75, 105.0),
        (r#"speed="0" clipIn="1""#, "", 0.75, 180.0),
        (r#"loop="1""#, "", 2.5, 330.0),
        (r#"reverse="true""#, "", 0.5, 280.0),
        (
            "",
            r#"<timeRemap><key time="0" value="0"/><key time="1" value="2"/><key time="2" value="0"/></timeRemap>"#,
            1.25,
            292.5,
        ),
    ] {
        let xml = format!(
            r#"<scene version="1.1"><project width="400" height="400" fps="30" duration="4"/><assets/>
        <symbols><symbol id="s" width="400" height="400" duration="2"><particleEmitter id="p" emitterShape="point" rate="0" speed="0" lifetime="10"><burst time="0.25" count="1"/>
        <animate property="x"><key time="0" value="0"/><key time="2" value="40"/></animate></particleEmitter></symbol></symbols>
        <composition><instance id="i" symbol="s" {attrs}>{remap}<animate property="x"><key time="0" value="100"/><key time="4" value="500"/></animate></instance></composition></scene>"#
        );
        let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let cold = Evaluator::new(&d, &EvalOptions::default()).unwrap();
        let warm = Evaluator::new(&d, &EvalOptions::default()).unwrap();
        warm.evaluate(0.0);
        warm.evaluate(3.0);
        let positions = |f: sr_eval::FrameGraph| {
            f.nodes.iter().find(|n| n.kind == "particleEmitter").unwrap().particles.as_ref().unwrap().pos.clone()
        };
        let a = positions(cold.evaluate(t));
        let b = positions(warm.evaluate(t));
        assert_eq!(a.len(), 1, "{attrs} {remap}: {a:?}");
        assert!((a[0][0] - expected_x).abs() < 0.01, "{attrs} {remap}: {a:?}, expected {expected_x}");
        assert_eq!(a, b, "{attrs} {remap}");
    }
}
