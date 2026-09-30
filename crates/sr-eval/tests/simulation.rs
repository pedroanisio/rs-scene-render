use sr_eval::{EvalOptions, Evaluator};

fn evaluator(body: &str, physics: &str) -> Evaluator {
    let xml = format!(
        r#"<scene version="1.1"><project width="400" height="400" fps="30" duration="4"/><assets><image id="img" src="unused.png" width="10" height="10"/></assets><composition>{body}</composition>{physics}</scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    Evaluator::new(&doc, &EvalOptions::default()).unwrap()
}

fn node<'a>(f: &'a sr_eval::FrameGraph, id: &str) -> &'a sr_eval::FrameNode {
    f.nodes.iter().find(|n| &*n.id == id).unwrap()
}

#[test]
fn scaled_group_particles_do_not_depend_on_first_frame() {
    let xml = r#"<scene version="1.1"><project width="400" height="400" fps="30" duration="4"/><assets/><composition>
      <group id="g" timeScale="2"><particleEmitter id="p" x="100" y="100" emitterShape="point" seed="1" rate="0" speed="100" direction="0" lifetime="10"><burst time="0" count="1"/></particleEmitter></group>
    </composition></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let warm = Evaluator::new(&doc, &EvalOptions::default()).unwrap();
    let cold = Evaluator::new(&doc, &EvalOptions::default()).unwrap();
    warm.evaluate(0.0);
    let a = warm.evaluate(0.75);
    let b = cold.evaluate(0.75);
    let pos = |f: &sr_eval::FrameGraph| {
        f.nodes.iter().find(|n| &*n.id == "p").unwrap().particles.as_ref().unwrap().pos.clone()
    };
    assert!(!pos(&a).is_empty());
    assert_eq!(pos(&a), pos(&b));
}

#[test]
fn nested_particle_clocks_match_local_timeline_and_replay() {
    let emitter = r#"<particleEmitter id="p" start="0.5" emitterShape="point" seed="1" rate="16" speed="100" direction="0" lifetime="0.5" preroll="0.125">
        <burst time="0.125" count="3" repeat="2" interval="0.125"/>
        <animate property="x"><key time="0" value="20"/><key time="2" value="220"/></animate>
        <animate property="rate"><key time="0" value="16"/><key time="2" value="32"/></animate>
      </particleEmitter>"#;
    let nested = format!(
        r#"<group id="outer" timeOffset="0.5" timeScale="2"><group id="inner" timeOffset="0.25" timeScale="1.5">{emitter}</group></group>"#
    );
    let direct = evaluator(&nested, "").evaluate(1.0);
    let reference = evaluator(emitter, "").evaluate(1.125);
    let a = node(&direct, "p").particles.as_ref().unwrap();
    let b = node(&reference, "p").particles.as_ref().unwrap();
    assert!(!a.pos.is_empty());
    assert_eq!(a.pos, b.pos);
    assert_eq!(a.alpha, b.alpha);
    let replay = evaluator(&nested, "");
    replay.evaluate(0.9);
    replay.evaluate(2.0);
    let back = replay.evaluate(1.0);
    assert_eq!(a.pos, node(&back, "p").particles.as_ref().unwrap().pos);
}

#[test]
fn physics_parent_chains_follow_rotation_without_changing_containers() {
    let ev = evaluator(
        r#"
      <group id="attached" parent="body" x="20"><layer id="child" asset="img" x="5"/><layer id="detached" parent="fixed" asset="img" x="7"/></group>
      <layer id="fixed" asset="img" x="300"/>
      <layer id="body" asset="img" x="100" y="100"><rigidBody angularVelocity="90" linearDamping="0" angularDamping="0"/></layer>
    "#,
        r#"<physics gravityY="0" bounds="none"/>"#,
    );
    let f = ev.evaluate(1.0);
    assert!(f.problems.is_empty(), "{:?}", f.problems);
    let expected = node(&f, "body").world.apply([25.0, 0.0]);
    let actual = node(&f, "child").world.apply([0.0, 0.0]);
    assert!((expected[0] - actual[0]).abs() < 1e-8 && (expected[1] - actual[1]).abs() < 1e-8);
    assert_eq!(node(&f, "detached").world.apply([0.0, 0.0]), [307.0, 0.0]);
    let attached_index = f.nodes.iter().position(|n| &*n.id == "attached").unwrap() as u32;
    assert_eq!(node(&f, "detached").parent, Some(attached_index));
}

#[test]
fn simulated_children_keep_their_own_pose_when_parent_is_drawn_later() {
    let ev = evaluator(
        r#"
      <layer id="child" parent="body" asset="img" x="30"><rigidBody type="static"/></layer>
      <layer id="body" asset="img" x="100" y="100"><rigidBody velocityX="100" linearDamping="0" collidesWith="1"/></layer>
    "#,
        r#"<physics gravityY="0" bounds="none"/>"#,
    );
    let f = ev.evaluate(0.5);
    assert!(node(&f, "body").world.apply([0.0, 0.0])[0] > 140.0);
    assert_eq!(node(&f, "child").world.apply([0.0, 0.0]), [130.0, 100.0]);
}

#[test]
fn parented_layers_follow_rigid_body_parent() {
    let xml = r#"<scene version="1.1"><project width="400" height="400" fps="30" duration="4"/><assets><image id="img" src="unused.png" width="10" height="10"/></assets><composition>
      <layer id="body" asset="img" x="100" y="100"><rigidBody velocityX="100" linearDamping="0"/></layer>
      <layer id="rider" parent="body" asset="img" x="20"/>
    </composition><physics gravityY="0" bounds="none"/></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = Evaluator::new(&doc, &EvalOptions::default()).unwrap();
    ev.evaluate(0.0);
    let f = ev.evaluate(1.01);
    let rider = f.nodes.iter().find(|n| &*n.id == "rider").unwrap().world.apply([0.0, 0.0]);
    let body = f.nodes.iter().find(|n| &*n.id == "body").unwrap().world.apply([0.0, 0.0]);
    assert!(body[0] > 190.0, "body moved: {body:?}; problems {:?}", f.problems);
    assert!((rider[0] - body[0] - 20.0).abs() < 0.01, "body {body:?}, rider {rider:?}");
}
