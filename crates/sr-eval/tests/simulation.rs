use sr_eval::{EvalOptions, Evaluator};

fn evaluator(body: &str, physics: &str) -> Evaluator {
    let xml = format!(
        r#"<scene version="1.2"><project width="400" height="400" fps="30" duration="4"/><assets><image id="img" src="unused.png" width="10" height="10"/></assets><composition>{body}</composition>{physics}</scene>"#
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

#[test]
fn bursts_happen_at_their_composition_time_like_animation_keys() {
    // an emitter that starts at 1 s with a burst at 1 s bursts at 1 s, not at 2 s: burst times
    // are on the same clock as the emitter's keys (composition time), not offset by its start
    let emitter = r#"<particleEmitter id="p" start="1" x="100" y="100" emitterShape="point" seed="1" rate="0" speed="50" lifetime="3">
        <burst time="1" count="5"/><burst time="1.5" count="7"/></particleEmitter>"#;
    let ev = evaluator(emitter, "");
    let count = |t: f64| {
        ev.evaluate(t).nodes.iter().find(|n| &*n.id == "p").unwrap().particles.as_ref().map_or(0, |p| p.pos.len())
    };
    assert_eq!(count(1.1), 5);
    assert_eq!(count(1.4), 5);
    assert_eq!(count(1.6), 12);
}

const RAIN: &str = r#"<particleEmitter id="p" x="200" y="60" emitterShape="line" emitterWidth="300" seed="1" rate="120" speed="300" direction="90" spread="10" lifetime="3" collide="true"/>"#;

#[test]
fn particles_bounce_off_rigid_bodies() {
    let ev = evaluator(
        &format!(
            r##"<shape id="slab" shape="rect" x="100" y="300" width="200" height="40" fill="#FFFFFF"><rigidBody type="static"/></shape>{RAIN}"##
        ),
        r#"<physics gravityY="0" bounds="none"/>"#,
    );
    let f = ev.evaluate(2.0);
    assert!(f.problems.is_empty(), "{:?}", f.problems);
    let slab = node(&f, "slab");
    let (a, b) = (slab.world.apply([0.0, 0.0]), slab.world.apply([200.0, 40.0]));
    assert_eq!((a, b), ([100.0, 300.0], [300.0, 340.0]));
    let parts = node(&f, "p").particles.as_ref().unwrap();
    let inside = |q: &[f32; 2]| q[0] > 101.0 && q[0] < 299.0 && q[1] > 301.0 && q[1] < 339.0;
    assert_eq!(parts.pos.iter().filter(|q| inside(q)).count(), 0, "nothing passes through the slab");
    let rising = parts.pos.iter().zip(&parts.vel).filter(|(q, v)| v[1] < 0.0 && q[1] < 300.0).count();
    assert!(rising > 20, "drops bounce back up off it: {rising}");
    assert!(parts.pos.iter().any(|q| q[1] > 345.0 && (q[0] < 95.0 || q[0] > 305.0)), "and fall past its sides");
}

#[test]
fn particle_collisions_do_not_depend_on_the_frames_rendered_before() {
    // a body crossing under the rain: every catch-up step meets it where it was at that step
    let scene = format!(
        r##"<shape id="cart" shape="rect" x="20" y="300" width="120" height="40" fill="#FFFFFF"><rigidBody velocityX="150" linearDamping="0"/></shape>{RAIN}"##
    );
    let physics = r#"<physics gravityY="0" bounds="none"/>"#;
    let parts = |f: &sr_eval::FrameGraph| {
        let p = node(f, "p").particles.as_ref().unwrap();
        (p.pos.clone(), p.vel.clone())
    };
    let in_order = evaluator(&scene, physics);
    let mut last = None;
    for k in 0..=45 {
        last = Some(parts(&in_order.evaluate(k as f64 / 30.0)));
    }
    let direct = parts(&evaluator(&scene, physics).evaluate(1.5));
    assert!(direct.1.iter().filter(|v| v[1] < 0.0).count() > 20, "drops bounce off the cart");
    assert!(last.unwrap() == direct, "frames 0..45 in order and frame 45 alone differ");
    // and going back gives what was there
    let early = parts(&evaluator(&scene, physics).evaluate(0.5));
    assert!(parts(&in_order.evaluate(0.5)) == early, "going back differs");
    let skipping = evaluator(&scene, physics);
    skipping.evaluate(1.0);
    assert!(parts(&skipping.evaluate(0.5)) == early && parts(&skipping.evaluate(1.5)) == direct, "skipping differs");
}

#[test]
fn delayed_3d_bodies_spawn_at_their_time_and_replay_identically() {
    for body in [
        r#"<object3D id="b" start="1" primitive="sphere" radius="10" x="100" y="100"><rigidBody linearDamping="0"/></object3D>"#,
        r#"<group id="g" timeOffset="0.5" timeScale="2"><object3D id="b" start="1" primitive="sphere" radius="10" x="100" y="100"><rigidBody linearDamping="0"/></object3D></group>"#,
        r#"<group id="g" start="1"><object3D id="b" primitive="sphere" radius="10" x="100" y="100"><rigidBody linearDamping="0"/></object3D></group>"#,
    ] {
        let make = || evaluator(body, r#"<physics gravityY="-1" bounds="none"/>"#);
        let ev = make();
        assert!(ev.evaluate(0.5).nodes.iter().all(|n| &*n.id != "b"));
        let at_start = ev.evaluate(1.0);
        let pose = node(&at_start, "b").pose3.expect("delayed body must be registered");
        assert!((pose[13] - 100.0).abs() < 1e-9);
        let later = ev.evaluate(1.5);
        let pose = node(&later, "b").pose3.unwrap();
        assert!((pose[13] - 112.5).abs() < 0.1, "{pose:?}");
        assert_eq!(node(&make().evaluate(1.5), "b").pose3, Some(pose));
        ev.evaluate(3.0);
        assert_eq!(node(&ev.evaluate(1.5), "b").pose3, Some(pose));
        assert!(!ev.physics_cache().unwrap().is_empty());
    }
}

#[test]
fn activation_and_force_windows_defer_3d_motion() {
    let body = r#"<object3D id="b" primitive="sphere" radius="10" x="100" y="100"><rigidBody activateAt="1" linearDamping="0"/></object3D>"#;
    let ev = evaluator(body, r#"<physics gravityY="-1" bounds="none"/>"#);
    assert!((node(&ev.evaluate(1.0), "b").pose3.unwrap()[13] - 100.0).abs() < 1e-9);
    assert!((node(&ev.evaluate(1.5), "b").pose3.unwrap()[13] - 112.5).abs() < 0.1);
    let ev = evaluator(
        &body.replace("activateAt=\"1\"", ""),
        r#"<physics gravityY="0" bounds="none"><forceField id="f" type="directional" forceX="1" start="1" end="2"/></physics>"#,
    );
    let x = |t| node(&ev.evaluate(t), "b").pose3.unwrap()[12];
    assert_eq!(x(1.0), 100.0);
    assert!((x(1.5) - 112.5).abs() < 0.1);
    assert!((x(2.5) - x(2.0) - 50.0).abs() < 0.1);
    assert!((x(3.0) - x(2.5) - 50.0).abs() < 0.1);
}

#[test]
fn delayed_3d_bodies_sample_birth_animation_and_do_not_collide_early() {
    let ev = evaluator(
        r#"<object3D id="b" start="1" primitive="sphere" radius="10" x="100" y="100">
      <animate property="x" timeBase="composition"><key time="0" value="100"/><key time="1" value="200"/></animate>
      <rigidBody activateAt="2" velocityX="20" linearDamping="0"/>
    </object3D>"#,
        r#"<physics gravityY="-1" bounds="none"/>"#,
    );
    let at = |t| node(&ev.evaluate(t), "b").pose3.unwrap();
    assert!((at(1.0)[12] - 200.0).abs() < 1e-9);
    assert!((at(2.0)[13] - 100.0).abs() < 1e-9);
    assert!((at(2.5)[12] - 210.0).abs() < 0.1);
    assert!((at(2.5)[13] - 112.5).abs() < 0.1);

    let ev = evaluator(
        r#"<object3D id="b" primitive="sphere" radius="10" x="100" y="100"><rigidBody linearDamping="0"/></object3D>
        <object3D id="late" start="1" primitive="sphere" radius="10" x="105" y="100"><rigidBody type="static"/></object3D>"#,
        r#"<physics gravityY="0" bounds="none"/>"#,
    );
    let at = |t| node(&ev.evaluate(t), "b").pose3.unwrap();
    assert!((at(0.5)[12] - 100.0).abs() < 1e-9, "unborn collider affected the body");
    assert!(at(1.5)[12] < 95.0, "spawned collider never participated: {:?}", at(1.5));
}
