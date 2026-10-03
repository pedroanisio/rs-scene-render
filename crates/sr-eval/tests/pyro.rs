fn evaluator(extra: &str) -> sr_eval::Evaluator {
    evaluator_size(extra, 8)
}

fn evaluator_size(extra: &str, size: usize) -> sr_eval::Evaluator {
    let xml = format!(
        r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="4"/>
      <composition><group id="clock" start="0.5" timeScale="2"><object3D id="cloud" primitive="volume" start="1">
        <pyro width="{size}" height="{size}" depth="{size}" voxelSize="1" dt="0.1" {extra}>
          <pyroSource shape="sphere" radius="2" densityRate="10" temperatureRate="1000" start="0.15" end="0.25"/>
        </pyro><medium blackbody="true"/></object3D></group></composition></scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
}

#[test]
fn native_pyro_obeys_object_parent_clocks_and_backward_seeking() {
    let ev = evaluator("");
    assert!(ev.has_simulation());
    for (time, expected) in [(0.85, 0.5), (0.9, 1.0), (1.5, 1.0), (0.85, 0.5)] {
        let frame = ev.evaluate(time);
        assert!(frame.problems.is_empty(), "{:?}", frame.problems);
        let node = frame.nodes.iter().find(|n| &*n.id == "cloud").unwrap();
        let volume = node.sim_volume.as_ref().expect("native pyro volume");
        let got = volume.data.grid("density").unwrap().sample_world([0.5; 3]);
        assert!(
            (got - expected).abs() < 1e-5,
            "time={time}, expected={expected}, got={got}, parts={:?}",
            frame.nodes[0].parts
        );
        assert!(
            (volume.data.grid("temperature").unwrap().sample_world([0.5; 3]) - (300.0 + 100.0 * expected)).abs() < 1e-3
        );
    }
}

#[test]
fn failed_pyro_budget_is_reported_instead_of_an_empty_success() {
    let ev = evaluator("maxMemoryMiB=\"1\" checkpointMemoryMiB=\"1\"");
    // A small domain fits even this budget.
    assert!(ev.evaluate(0.9).problems.is_empty());
    let large = evaluator_size("maxMemoryMiB=\"1\" checkpointMemoryMiB=\"1\"", 32);
    let frame = large.evaluate(0.9);
    assert!(frame.problems.iter().any(|e| e.contains("memory budget")), "{:?}", frame.problems);
    assert!(frame.nodes.iter().find(|n| &*n.id == "cloud").unwrap().sim_volume.is_none());
}

#[test]
fn large_adjacent_seeds_keep_all_u64_bits() {
    let a = evaluator("seed=\"9007199254740992\" turbulence=\"1\"");
    let b = evaluator("seed=\"9007199254740993\" turbulence=\"1\"");
    let fa = a.evaluate(0.9);
    let fb = b.evaluate(0.9);
    assert!(fa.problems.is_empty() && fb.problems.is_empty());
    let key =
        |f: &sr_eval::FrameGraph| f.nodes.iter().find(|n| &*n.id == "cloud").unwrap().sim_volume.as_ref().unwrap().key;
    assert_ne!(key(&fa), key(&fb));
}

#[test]
fn animated_source_rates_are_sampled_on_simulation_time() {
    let xml = r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="2"/><composition>
      <object3D id="cloud" primitive="volume" start="0.5" animationSpeed="2">
        <pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1">
          <pyroSource radius="2" start="0.1" end="0.3"><animate property="densityRate" timeBase="local"><key time="0" value="0"/><key time="1" value="10"/></animate></pyroSource>
        </pyro></object3D></composition></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    for (time, expected) in [(0.6, 0.1), (0.65, 0.3), (0.6, 0.1)] {
        let frame = ev.evaluate(time);
        assert!(frame.problems.is_empty(), "{:?}", frame.problems);
        let volume = frame.nodes[0].sim_volume.as_ref().unwrap();
        let got = volume.data.grid("density").unwrap().sample_world([0.5; 3]);
        assert!(
            (got - expected).abs() < 1e-5,
            "time={time}, expected={expected}, got={got}, parts={:?}",
            frame.nodes[0].parts
        );
    }
}

#[test]
fn conditionally_hidden_source_history_does_not_abort_visible_pyro() {
    let xml = r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/><composition>
      <object3D id="cloud" primitive="volume" condition="time &gt;= 0.2">
        <pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1"><pyroSource radius="2" densityRate="1"/></pyro>
      </object3D></composition></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(0.4);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    let density = frame.nodes[0].sim_volume.as_ref().unwrap().data.grid("density").unwrap().sample_world([0.5; 3]);
    assert!((density - 0.2).abs() < 1e-6, "{density}");
}

#[test]
fn mesh_sources_emit_inside_imported_geometry_and_reject_open_surfaces() {
    let temp = TempDir::new();
    let dir = &temp.0;
    let file = dir.join("cube.obj");
    let obj = MESH_CUBE;
    std::fs::write(&file, obj).unwrap();
    let xml = r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/><assets><mesh id="shape" src="cube.obj"/></assets><composition>
      <object3D id="cloud" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1"><pyroSource shape="mesh" mesh="shape" densityRate="10"/></pyro></object3D>
      </composition></scene>"#;
    let doc =
        sr_model::load_str(xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir.clone()) }).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(0.1);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    let density = frame.nodes[0].sim_volume.as_ref().unwrap().data.grid("density").unwrap();
    assert_eq!(density.sample_world([0.5; 3]), 1.0);
    assert_eq!(density.sample_world([3.5; 3]), 0.0);
    std::fs::write(&file, obj.replace("f 2 7 6\n", "")).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(0.1);
    assert!(frame.problems.iter().any(|e| e.contains("closed")), "{:?}", frame.problems);
    assert!(frame.nodes[0].sim_volume.is_none());
}

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        for id in 0..10_000 {
            let dir = std::env::temp_dir().join(format!("sr-pyro-mesh-{}-{id}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => return Self(dir),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("temporary directory: {e}"),
            }
        }
        panic!("temporary directory namespace exhausted");
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const MESH_CUBE: &str = "v -0.02 -0.02 -0.02\nv 0.02 -0.02 -0.02\nv 0.02 0.02 -0.02\nv -0.02 0.02 -0.02\nv -0.02 -0.02 0.02\nv 0.02 -0.02 0.02\nv 0.02 0.02 0.02\nv -0.02 0.02 0.02\nf 1 3 2\nf 1 4 3\nf 5 6 7\nf 5 7 8\nf 1 2 6\nf 1 6 5\nf 4 8 7\nf 4 7 3\nf 1 5 8\nf 1 8 4\nf 2 3 7\nf 2 7 6\n";

#[test]
fn pyro_force_fields_obey_activation_selection_and_domain_axes() {
    let xml = r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/>
      <composition>
      <object3D id="cloud" primitive="volume" rotation="90" scaleX="2" scaleY="2" scaleZ="2">
      <pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" boundary="open" forceFields="wind"/>
      </object3D></composition><physics pixelsPerMeter="10" gravityY="0" bounds="none">
      <forceField id="wind" type="directional" forceX="1" forceZ="1" start="0.2" end="0.3"/>
      <forceField id="excluded" type="directional" forceY="100"/></physics></scene>"#;
    for disabled in [false, true] {
        let xml = if disabled {
            xml.replace("forceFields=\"wind\"", "forceFields=\"wind\" useForceFields=\"false\"")
        } else {
            xml.to_string()
        };
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        for (time, active) in [(0.2, false), (0.3, true), (0.4, true), (0.2, false), (0.3, true)] {
            let frame = ev.evaluate(time);
            assert!(frame.problems.is_empty(), "{:?}", frame.problems);
            let volume = &frame.nodes[0].sim_volume.as_ref().unwrap().data;
            let v = ["velocity.x", "velocity.y", "velocity.z"]
                .map(|name| volume.grid(name).unwrap().sample_world([0.5; 3]));
            let expected = if active && !disabled { [0.0, -0.5, -0.5] } else { [0.0; 3] };
            // The first forced step is exact; subsequent open-boundary
            // advection loses a small amount of momentum to the exterior.
            let tolerance = if time > 0.3 && !disabled { 0.003 } else { 1e-5 };
            for a in 0..3 {
                assert!((v[a] - expected[a]).abs() < tolerance, "time {time}, disabled {disabled}: {v:?}");
            }
        }
    }
}

#[test]
fn pyro_drag_uses_current_velocity_and_replays_deterministically() {
    let xml = r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/>
      <composition><object3D id="cloud" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" boundary="open">
      <pyroSource shape="box" width="10" height="10" depth="10" velocityRateX="1"/>
      </pyro></object3D></composition><physics pixelsPerMeter="1" gravityY="0" bounds="none"><forceField id="drag" type="drag" strength="1" affects="particles"/><forceField id="bodies-only" type="directional" forceX="100" affects="bodies"/></physics></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let mut replay_key = None;
    for (t, expected) in [(0.1, 0.1), (0.2, 0.19), (0.3, 0.271), (0.2, 0.19)] {
        let frame = ev.evaluate(t);
        assert!(frame.problems.is_empty(), "{:?}", frame.problems);
        let vx = frame.nodes[0].sim_volume.as_ref().unwrap().data.grid("velocity.x").unwrap().sample_world([0.5; 3]);
        // v[n+1] = 0.9*v[n] + 0.1, with a small open-boundary advection loss.
        assert!((vx - expected).abs() < 5e-4, "t={t}: {vx} != {expected}");
        if t == 0.2 {
            let key = frame.nodes[0].sim_volume.as_ref().unwrap().key;
            if let Some(previous) = replay_key {
                assert_eq!(key, previous);
            }
            replay_key = Some(key);
        }
    }
}

#[test]
fn pyro_fields_follow_a_simulated_parent_at_each_substep() {
    let template = r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/><composition>
      <object3D id="carrier" primitive="box" width="1" height="1" depth="1" x="-1">MOTION</object3D>
      <object3D id="cloud" primitive="volume" parent="carrier"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" boundary="open"/></object3D>
      </composition><physics pixelsPerMeter="10" gravityY="0" bounds="none">
      <forceField id="radial" type="radial" strength="1" affects="particles"/></physics></scene>"#;
    let make = |motion: &str| {
        let doc =
            sr_model::load_str(&template.replace("MOTION", motion), &sr_model::LoadOptions::without_assets()).unwrap();
        sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
    };
    let simulated = make(r#"<rigidBody velocityX="20" linearDamping="0" angularDamping="0"/>"#);
    let animated = make(r#"<animate property="x"><key time="0" value="-1"/><key time="1" value="19"/></animate>"#);
    for time in [0.2, 0.3, 0.1, 0.2] {
        let a = simulated.evaluate(time);
        let b = animated.evaluate(time);
        assert!(a.problems.is_empty() && b.problems.is_empty(), "{:?} {:?}", a.problems, b.problems);
        let volume = |g: &sr_eval::FrameGraph| {
            g.nodes.iter().find(|n| &*n.id == "cloud").unwrap().sim_volume.as_ref().unwrap().clone()
        };
        for name in ["velocity.x", "velocity.y", "velocity.z"] {
            let va = volume(&a).data.grid(name).unwrap().sample_world([0.5; 3]);
            let vb = volume(&b).data.grid(name).unwrap().sample_world([0.5; 3]);
            assert!((va - vb).abs() < 1e-4, "time={time}, {name}: {va} != {vb}");
        }
    }
}

#[test]
fn included_pyro_fields_resolve_three_dimensional_parents_in_scope() {
    let temp = TempDir::new();
    for link in [r#"parent="carrier""#, "constraint"] {
        let (parent, constraint) = if link == "constraint" {
            ("", r#"<transformConstraint type="parent" target="carrier" influence="0.5"/>"#)
        } else {
            (link, "")
        };
        let body = format!(
            r#"<object3D id="carrier" primitive="box" rotationY="90"/>
          <object3D id="cloud" primitive="volume" {parent}>{constraint}
          <pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" boundary="open"/></object3D>"#
        );
        let project = r#"<project width="32" height="32" fps="10" duration="1"/>"#;
        let physics = r#"<physics pixelsPerMeter="10" gravityY="0" bounds="none"><forceField id="wind" type="directional" forceX="1" affects="particles"/></physics>"#;
        std::fs::write(temp.0.join("library.xml"), format!(r#"<scene version="1.3">{project}<symbols><symbol id="assembly" width="8" height="8">{body}</symbol></symbols><composition/></scene>"#)).unwrap();
        let evaluate = |composition: &str| {
            let xml =
                format!(r#"<scene version="1.3">{project}<composition>{composition}</composition>{physics}</scene>"#);
            let doc = sr_model::load_str(
                &xml,
                &sr_model::LoadOptions { verify_assets: false, base_dir: Some(temp.0.clone()) },
            )
            .unwrap();
            sr_eval::Evaluator::new(&doc, &Default::default()).unwrap().evaluate(0.1)
        };
        let direct = evaluate(&body);
        let included = evaluate(r#"<include id="inc" src="library.xml" symbol="assembly"/>"#);
        assert!(
            direct.problems.is_empty() && included.problems.is_empty(),
            "{:?} {:?}",
            direct.problems,
            included.problems
        );
        let volume = |g: &sr_eval::FrameGraph, id: &str| {
            g.nodes.iter().find(|n| &*n.id == id).unwrap().sim_volume.as_ref().unwrap().clone()
        };
        for name in ["velocity.x", "velocity.y", "velocity.z"] {
            let a = volume(&direct, "cloud").data.grid(name).unwrap().sample_world([0.5; 3]);
            let b = volume(&included, "inc/cloud").data.grid(name).unwrap().sample_world([0.5; 3]);
            assert!((a - b).abs() < 1e-6, "{link}, {name}: direct={a}, included={b}");
        }
    }
}

#[test]
fn included_mesh_source_uses_its_document_and_animated_transform_on_replay() {
    let temp = TempDir::new();
    let library = temp.0.join("library");
    std::fs::create_dir(&library).unwrap();
    std::fs::write(library.join("cube.obj"), MESH_CUBE).unwrap();
    // A same-named main-document asset must not shadow the library's mesh.
    let main = r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/>
      <assets><mesh id="shape" src="deliberately-missing.obj"/></assets><composition>
      <include id="inc" src="library/smoke.xml" symbol="plume"/></composition></scene>"#;
    std::fs::write(
        library.join("smoke.xml"),
        r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/>
      <assets><mesh id="shape" src="cube.obj"/></assets><symbols><symbol id="plume" width="8" height="8">
      <object3D id="cloud" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1">
      <pyroSource shape="mesh" mesh="shape" scaleX="-0.5" scaleY="0.5" scaleZ="0.5" densityRate="10">
      <animate property="x"><key time="0" value="-2"/><key time="0.1" value="2"/></animate>
      </pyroSource></pyro></object3D></symbol></symbols><composition/></scene>"#,
    )
    .unwrap();
    let doc = sr_model::load_str(main, &sr_model::LoadOptions { verify_assets: false, base_dir: Some(temp.0.clone()) })
        .unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    for (time, right) in [(0.1, 0.0), (0.2, 1.0), (0.1, 0.0)] {
        let frame = ev.evaluate(time);
        assert!(frame.problems.is_empty(), "{:?}", frame.problems);
        let density = frame
            .nodes
            .iter()
            .find(|n| &*n.id == "inc/cloud")
            .unwrap()
            .sim_volume
            .as_ref()
            .unwrap()
            .data
            .grid("density")
            .unwrap();
        assert_eq!(density.sample_world([-1.5, 0.5, 0.5]), 1.0);
        assert_eq!(density.sample_world([2.5, 0.5, 0.5]), right);
        assert_eq!(density.sample_world([0.5, 0.5, 0.5]), 0.0);
    }
}
