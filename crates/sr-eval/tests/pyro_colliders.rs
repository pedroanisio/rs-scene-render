fn evaluator(collider: &str, pyro: &str) -> sr_eval::Evaluator {
    let boundary = if pyro.contains("boundary=") { "" } else { r#"boundary="open""# };
    let xml = format!(
        r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/>
      <assets><map id="atlas" width="32" height="16"/></assets><composition>{collider}<object3D id="cloud" primitive="volume">
      <pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" {boundary} pressureIterations="500" colliders="solid" {pyro}>
      <pyroSource shape="box" width="10" height="10" depth="10" densityRate="10"/>
      </pyro></object3D></composition><physics gravityY="0" bounds="none"/></scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
}

fn volume(ev: &sr_eval::Evaluator, time: f64) -> std::sync::Arc<sr_eval::pyro::SimVolume> {
    let frame = ev.evaluate(time);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    frame.nodes.iter().find(|n| &*n.id == "cloud").unwrap().sim_volume.as_ref().unwrap().clone()
}

#[test]
fn crater_pyro_excavates_the_solid_region_and_replays_local_time() {
    let ev = evaluator(
        r#"<object3D id="solid" primitive="plane" width="8" height="8" segments="16" start="0.1">
        <crater radius="4" depth="2" rimHeight="0" rimWidth="1" start="0.1" end="0.3" curve="linear"/>
        </object3D>"#,
        r#"colliderThickness="2""#,
    );
    let density = |t| volume(&ev, t).data.grid("density").unwrap().sample_world([0.5, 0.5, 0.5]);
    assert_eq!(density(0.2), 0.0, "before local growth the plane slab excludes smoke");
    assert!(density(0.6) > 0.5, "excavated cells must admit smoke");
    let key = volume(&ev, 0.6).key;
    assert_eq!(density(0.2), 0.0);
    assert_eq!(volume(&ev, 0.6).key, key);
}

#[test]
fn crater_pyro_boundary_motion_is_nonzero_and_budgeted() {
    let ev = evaluator(
        r#"<object3D id="solid" primitive="plane" width="8" height="8" segments="16">
        <crater radius="4" depth="2" rimHeight="0" rimWidth="1" end="1" curve="linear"/>
        </object3D>"#,
        r#"colliderThickness="4""#,
    );
    let v = volume(&ev, 0.7);
    let speed = v.data.grid("velocity.z").unwrap().sample_world([0.5, 0.5, 1.5]);
    assert!(speed > 0.1, "excavation must prescribe solid velocity, got {speed}");
    let ev = evaluator(
        r#"<object3D id="solid" primitive="plane" segments="256">
        <crater maxMemoryMiB="1"/></object3D>"#,
        "",
    );
    let frame = ev.evaluate(0.1);
    assert!(frame.problems.iter().any(|e| e.contains("budget")), "{:?}", frame.problems);
}

#[test]
fn crater_pyro_primitives_have_closed_regions_without_uv_seam_failures() {
    for (attrs, inside, outside) in [
        (r#"primitive="plane" width="8" height="8""#, [0.5, 0.5, -0.5], [0.5, 0.5, -1.5]),
        (r#"primitive="box" width="4" height="4" depth="4""#, [0.5; 3], [2.5, 0.5, 0.5]),
        (r#"primitive="sphere" radius="2""#, [0.5; 3], [2.5, 0.5, 0.5]),
        (r#"primitive="globe" map="atlas" radius="2""#, [0.5; 3], [2.5, 0.5, 0.5]),
        (r#"primitive="cylinder" radius="2" height="6""#, [0.5, 2.5, 0.5], [2.5, 0.5, 0.5]),
        (r#"primitive="cone" radius="3" height="6""#, [1.5, 2.5, 0.5], [1.5, -2.5, 0.5]),
        (r#"primitive="capsule" radius="1" height="6""#, [0.5, 2.5, 0.5], [1.5, 0.5, 0.5]),
        (r#"primitive="capsule" radius="2" height="4""#, [0.5; 3], [2.5, 0.5, 0.5]),
        (r#"primitive="torus" radius="2" height="2""#, [2.5, 0.5, 0.5], [0.5; 3]),
    ] {
        let ev = evaluator(
            &format!(r#"<object3D id="solid" {attrs} segments="16"><crater start="0.5" end="1"/></object3D>"#),
            "",
        );
        let v = volume(&ev, 0.1);
        let density = v.data.grid("density").unwrap();
        assert_eq!(density.sample_world(inside), 0., "{attrs}");
        assert!(density.sample_world(outside) > 0.5, "{attrs}");
    }
}

#[test]
fn crater_pyro_budget_uses_actual_primitive_topology_counts() {
    // A cylinder grows linearly in segments; charging a square grid would
    // reject this small, valid surface under its 3 MiB component allowance.
    let ev = evaluator(
        r#"<object3D id="solid" primitive="cylinder" radius="2" height="4" segments="512">
        <crater depth="0" rimHeight="0" maxMemoryMiB="3"/></object3D>"#,
        "",
    );
    assert_eq!(volume(&ev, 0.1).data.grid("density").unwrap().sample_world([0.5; 3]), 0.0);
}

#[test]
fn crater_pyro_uses_relative_parent_motion_under_reflection_and_rotation() {
    let make = |parent: &str, motion: &str| {
        let xml = format!(
            r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/>
        <composition><object3D id="carrier" primitive="box" {parent}>{motion}</object3D>
        <object3D id="solid" parent="carrier" primitive="plane" width="8" height="8" segments="16">
        <crater radius="3" depth="1" rimHeight="0.2" rimWidth="1" end="0.5"/></object3D>
        <object3D id="cloud" parent="carrier" primitive="volume">
        <pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" boundary="open" pressureIterations="500" colliders="solid">
        <pyroSource shape="box" width="10" height="10" depth="10" densityRate="10"/></pyro></object3D>
        </composition></scene>"#
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
    };
    let reference = make("", "");
    let transformed = make(
        r#"x="32" rotationY="30" scaleX="-2" scaleY="3""#,
        r#"<animate property="x"><key time="0" value="32"/><key time="1" value="64"/></animate>"#,
    );
    for t in [0.3, 0.1, 0.3] {
        let a = volume(&reference, t);
        let b = volume(&transformed, t);
        for channel in ["density", "velocity.x", "velocity.y", "velocity.z"] {
            for z in -3..3 {
                for y in -3..3 {
                    for x in -3..3 {
                        let p = [x, y, z].map(|v| f64::from(v) + 0.5);
                        let av = a.data.grid(channel).unwrap().sample_world(p);
                        let bv = b.data.grid(channel).unwrap().sample_world(p);
                        assert!((av - bv).abs() < 1e-4, "{channel} {p:?}: {av} {bv}");
                    }
                }
            }
        }
    }
}

#[test]
fn collider_windows_and_hidden_proxies_are_sampled_during_replay() {
    let ev = evaluator(
        r#"<object3D id="solid" primitive="box" width="2" height="2" depth="2" visible="false" start="0.1" end="0.2"/>"#,
        "",
    );
    for (t, density) in [(0.1, 1.0), (0.2, 0.0), (0.3, 1.0), (0.1, 1.0), (0.3, 1.0)] {
        assert_eq!(volume(&ev, t).data.grid("density").unwrap().sample_world([0.5; 3]), density, "t={t}");
    }
}

#[test]
fn rotating_box_moves_boundary_faces_without_artificial_expansion() {
    let ev = evaluator(
        r#"<object3D id="solid" primitive="box" width="2" height="2" depth="2">
      <animate property="rotation"><key time="0" value="0"/><key time="1" value="30"/></animate></object3D>"#,
        r#"boundary="closed""#,
    );
    let v = volume(&ev, 0.1);
    // A rotating solid's x velocity changes sign across its y axis. Both
    // sample centres are solid, so this tests prescribed boundary velocities.
    let lo = v.data.grid("velocity.x").unwrap().sample_world([0.5, -0.5, 0.5]);
    let hi = v.data.grid("velocity.x").unwrap().sample_world([0.5, 0.5, 0.5]);
    assert!(lo > 0.2 && hi < -0.2, "{lo} {hi}");
    assert!((lo + hi).abs() < 1e-5);
}

#[test]
fn collider_primitive_mapping_preserves_curved_regions_and_plane_thickness() {
    for (attrs, inside, outside) in [
        (r#"primitive="sphere" radius="1""#, [0.5; 3], [1.5, 0.5, 0.5]),
        (r#"primitive="globe" map="atlas" radius="1""#, [0.5; 3], [1.5, 0.5, 0.5]),
        (r#"primitive="cylinder" radius="1" height="6""#, [0.5, 2.5, 0.5], [0.5, 3.5, 0.5]),
        (r#"primitive="cone" radius="3" height="6""#, [2.5, 2.5, 0.5], [0.5, -2.5, 0.5]),
        (r#"primitive="capsule" radius="1" height="6""#, [0.5, 2.5, 0.5], [0.5, 3.5, 0.5]),
        (r#"primitive="torus" radius="2" height="2""#, [2.5, 0.5, 0.5], [0.5; 3]),
        (r#"primitive="plane" width="8" height="8""#, [2.5, 2.5, 0.5], [2.5, 2.5, 1.5]),
    ] {
        let ev = evaluator(&format!(r#"<object3D id="solid" {attrs}/>"#), "");
        let v = volume(&ev, 0.1);
        assert_eq!(v.data.grid("density").unwrap().sample_world(inside), 0.0, "{attrs}");
        assert_eq!(v.data.grid("density").unwrap().sample_world(outside), 1.0, "{attrs}");
    }
}

#[test]
fn physics_collider_motion_matches_authored_motion_and_replays() {
    let make = |motion: &str| {
        evaluator(
            &format!(r#"<object3D id="solid" primitive="box" width="2" height="2" depth="2">{motion}</object3D>"#),
            "",
        )
    };
    let simulated = make(r#"<rigidBody velocityX="1" linearDamping="0" angularDamping="0"/>"#);
    let authored = make(r#"<animate property="x"><key time="0" value="0"/><key time="1" value="1"/></animate>"#);
    for time in [0.1, 0.3, 0.2, 0.1] {
        let a = volume(&simulated, time);
        let b = volume(&authored, time);
        for name in ["density", "velocity.x", "velocity.y", "velocity.z"] {
            let av = a.data.grid(name).unwrap().sample_world([0.5; 3]);
            let bv = b.data.grid(name).unwrap().sample_world([0.5; 3]);
            assert!((av - bv).abs() < 1e-5, "time={time}, {name}: {av} != {bv}");
        }
        assert!((a.data.grid("velocity.x").unwrap().sample_world([0.5; 3]) - 1.0).abs() < 1e-5);
    }
}

#[test]
fn instanced_colliders_use_relative_domain_motion() {
    let xml = r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/>
      <symbols><symbol id="assembly" width="32" height="32">
      <object3D id="carrier" primitive="box"><animate property="x"><key time="0" value="0"/><key time="1" value="10"/></animate></object3D>
      <object3D id="solid" primitive="box" parent="carrier" width="2" height="2" depth="2"/>
      <object3D id="cloud" primitive="volume" parent="carrier"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" colliders="solid">
      <pyroSource shape="box" width="10" height="10" depth="10" densityRate="10"/></pyro></object3D>
      </symbol></symbols><composition><instance id="a" symbol="assembly"/><instance id="b" symbol="assembly" x="100"/></composition></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    for time in [0.2, 0.1, 0.3] {
        let frame = ev.evaluate(time);
        assert!(frame.problems.is_empty(), "{:?}", frame.problems);
        for id in ["a/cloud", "b/cloud"] {
            let v = &frame.nodes.iter().find(|n| &*n.id == id).unwrap().sim_volume.as_ref().unwrap().data;
            assert_eq!(v.grid("density").unwrap().sample_world([0.5; 3]), 0.0);
            assert_eq!(v.grid("velocity.x").unwrap().sample_world([0.5; 3]), 0.0);
        }
    }
}

#[test]
fn mesh_colliders_coexist_with_mesh_sources_and_reject_open_geometry() {
    struct Temp(std::path::PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let temp = (0..10_000)
        .find_map(|i| {
            let path = std::env::temp_dir().join(format!("sr-pyro-collider-{}-{i}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => Some(Temp(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => panic!("temporary directory: {e}"),
            }
        })
        .expect("temporary directory namespace exhausted");
    let points = [[-1, -1, -1], [1, -1, -1], [1, 1, -1], [-1, 1, -1], [-1, -1, 1], [1, -1, 1], [1, 1, 1], [-1, 1, 1]];
    let faces = [
        [1, 3, 2],
        [1, 4, 3],
        [5, 6, 7],
        [5, 7, 8],
        [1, 2, 6],
        [1, 6, 5],
        [4, 8, 7],
        [4, 7, 3],
        [1, 5, 8],
        [1, 8, 4],
        [2, 3, 7],
        [2, 7, 6],
    ];
    let mut obj = String::new();
    for [x, y, z] in points {
        obj.push_str(&format!("v {} {} {}\n", x as f64 * 0.02, y as f64 * 0.02, z as f64 * 0.02));
    }
    for [a, b, c] in faces {
        obj.push_str(&format!("f {a} {b} {c}\n"));
    }
    std::fs::write(temp.0.join("cube.obj"), &obj).unwrap();
    let xml = r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/>
      <assets><mesh id="cube" src="cube.obj"/></assets><composition>
      <object3D id="solid" primitive="mesh" mesh="cube" x="2" visible="false"/>
      <object3D id="cloud" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" colliders="solid" meshMemoryMiB="1">
      <pyroSource shape="mesh" mesh="cube" densityRate="10"/></pyro></object3D></composition></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(temp.0.clone()) })
        .unwrap();
    let make = || sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let v = volume(&make(), 0.1);
    assert_eq!(v.data.grid("density").unwrap().sample_world([-0.5; 3]), 1.0);
    assert_eq!(v.data.grid("density").unwrap().sample_world([0.5; 3]), 0.0);
    std::fs::write(temp.0.join("cube.obj"), obj.replace("f 2 7 6\n", "")).unwrap();
    let frame = make().evaluate(0.1);
    assert!(frame.problems.iter().any(|e| e.contains("closed")), "{:?}", frame.problems);
    assert!(frame.nodes.iter().find(|n| &*n.id == "cloud").unwrap().sim_volume.is_none());
}

#[test]
fn collapsing_and_singular_midpoint_motion_report_errors_without_losing_initial_state() {
    for scale in ["0", "-1"] {
        let ev = evaluator(
            &format!(
                r#"<object3D id="solid" primitive="box" width="2" height="2" depth="2">
          <animate property="scaleX"><key time="0" value="1"/><key time="0.1" value="{scale}"/></animate></object3D>"#
            ),
            "",
        );
        let initial = volume(&ev, 0.0).key;
        let frame = ev.evaluate(0.1);
        assert!(!frame.problems.is_empty(), "scale={scale}");
        assert!(frame.nodes.iter().find(|n| &*n.id == "cloud").unwrap().sim_volume.is_none());
        assert_eq!(volume(&ev, 0.0).key, initial);
    }
}
