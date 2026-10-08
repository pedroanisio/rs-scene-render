fn evaluator(nodes: &str) -> sr_eval::Evaluator {
    let xml = format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="10" duration="4"/><composition>{nodes}</composition></scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
}

#[test]
fn native_particles_follow_parent_clocks_but_keep_world_space_after_birth() {
    let ev = evaluator(
        r#"<group id="clock" start="0.5" timeScale="2"><particles3D id="dust" start="1" rate="0" velocityZ="4" x="10" dt="0.1"><burst time="0" count="1"/><animate property="x" timeBase="local"><key time="0" value="10"/><key time="1" value="110"/></animate></particles3D></group>"#,
    );
    assert!(ev.has_simulation());
    for (t, z) in [(0.85, 0.8), (1.1, 2.8), (0.85, 0.8)] {
        let f = ev.evaluate(t);
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        let n = f.nodes.iter().find(|n| &*n.id == "dust").unwrap();
        let p = n.particles3d.as_ref().unwrap();
        assert_eq!(p.frame.particles.len(), 1);
        assert!((p.frame.particles[0].position[0] - 10.).abs() < 1e-10);
        assert!((p.frame.particles[0].position[2] - z).abs() < 1e-10, "{:?}", p.frame);
    }
}

#[test]
fn emission_windows_are_separate_from_node_visibility_and_failures_are_reported() {
    let ev = evaluator(
        r#"<particles3D id="dust" start="0.5" rate="10" emissionStart="0.2" emissionEnd="0.4" dt="0.1" lifetime="2"/>"#,
    );
    for (t, count) in [(0.6, 0), (0.8, 1), (1.5, 2), (0.8, 1)] {
        let f = ev.evaluate(t);
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        assert_eq!(f.nodes[0].particles3d.as_ref().unwrap().frame.particles.len(), count);
    }
    let ev = evaluator(r#"<particles3D id="huge" maxParticles="1000000" maxMemoryMiB="1"/>"#);
    let f = ev.evaluate(0.1);
    assert!(f.problems.iter().any(|p| p.contains("memory")), "{:?}", f.problems);
    assert!(f.nodes[0].particles3d.is_none());
}

#[test]
fn particles_hit_hidden_moving_scene_colliders() {
    let ev = evaluator(
        r#"<object3D id="floor" primitive="box" x="10" y="2" width="30" height="0.1" depth="30" visible="false"><animate property="y"><key time="0" value="2"/><key time="2" value="2.2"/></animate></object3D><particles3D id="dust" x="10" rate="0" velocityY="20" gravityY="10" collisionRadius="0.1" bounce="0" dt="0.01" lifetime="3" colliders="floor"><burst time="0" count="1"/></particles3D>"#,
    );
    for t in [1., 2., 1.] {
        let f = ev.evaluate(t);
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        let p = &f.nodes.iter().find(|n| &*n.id == "dust").unwrap().particles3d.as_ref().unwrap().frame.particles[0];
        assert!((p.position[1] - (1.85 + 0.1 * t)).abs() < 1e-4, "{p:?}");
        assert!((p.velocity[1] - 0.1).abs() < 1e-4, "{p:?}");
    }
}

#[test]
fn selected_force_fields_use_particle_class_three_axes_and_activation_windows() {
    let xml = r#"<scene version="1.3"><project width="64" height="64" fps="10" duration="2"/><composition><particles3D id="p" forceFields="wind" rate="0" dt="0.05"><burst time="0" count="1"/></particles3D></composition><physics pixelsPerMeter="1"><forceField id="wind" type="directional" forceZ="2" start="0.2" end="0.4" affects="particles"/></physics></scene>"#;
    for (xml, z) in [(xml.to_string(), -0.28), (xml.replace("affects=\"particles\"", "affects=\"bodies\""), 0.)] {
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        for t in [1., 0.1, 1.] {
            let f = ev.evaluate(t);
            assert!(f.problems.is_empty(), "{:?}", f.problems);
            let particle = &f.nodes[0].particles3d.as_ref().unwrap().frame.particles[0];
            assert!((particle.position[2] - if t == 1. { z } else { 0. }).abs() < 1e-8, "{particle:?}");
        }
    }
}

#[test]
fn adjacent_large_seeds_and_conditional_emission_are_preserved() {
    let sample = |seed| {
        let ev = evaluator(&format!(
            r#"<particles3D id="p" condition="time &gt;= 0.2" rate="10" speed="1" spread="180" dt="0.1" seed="{seed}"/>"#
        ));
        let f = ev.evaluate(0.5);
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        f.nodes[0].particles3d.as_ref().unwrap().clone()
    };
    let a = sample(9007199254740992u64);
    let b = sample(9007199254740993u64);
    assert_eq!(a.frame.emitted, 3);
    assert_ne!(a.key, b.key);
}

#[test]
fn collider_links_and_particle_parents_resolve_in_each_symbol_instance() {
    let xml = r#"<scene version="1.3"><project width="128" height="32" fps="10" duration="2"/><symbols><symbol id="assembly" width="32" height="32"><object3D id="floor" primitive="box" y="2" width="30" height="0.1" depth="30" visible="false"/><particles3D id="p" parent="floor" y="-2" velocityY="20" rate="0" collisionRadius="0.1" bounce="0" dt="0.05" colliders="floor"><burst time="0" count="1"/></particles3D></symbol></symbols><composition><instance id="a" symbol="assembly"/><instance id="b" symbol="assembly" x="100"/></composition></scene>"#;
    let d = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let f = ev.evaluate(0.2);
    assert!(f.problems.is_empty(), "{:?}", f.problems);
    for (id, x) in [("a/p", 0.), ("b/p", 100.)] {
        let p = &f.nodes.iter().find(|n| &*n.id == id).unwrap().particles3d.as_ref().unwrap().frame.particles[0];
        assert!((p.position[0] - x).abs() < 1e-8, "{p:?}");
        assert!((p.position[1] - 1.85).abs() < 1e-5, "{p:?}");
    }
}
