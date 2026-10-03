fn scene(consumers: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="32" height="32" fps="10" duration="1.2"/>
        <materials><material id="inside" baseColor="#ff0000"/></materials>
        <composition><object3D id="solid" primitive="box" width="2" height="2" depth="2" visible="false">
        <rigidBody type="static" mass="4" linearDamping="0" angularDamping="0" collidesWith="none"/>
        <fracture at="0.2" pieces="4" seed="42" interiorMaterial="inside" impulseX="16"/>
        </object3D>{consumers}</composition>
        <physics gravityY="0" pixelsPerMeter="1" fixedStep="0.01" bounds="none"/></scene>"##
    )
}

fn evaluator(xml: &str) -> sr_eval::Evaluator {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
}

#[test]
fn particles_hit_released_pieces_and_pass_through_the_retired_source() {
    let ev = evaluator(&scene(
        r#"<particles3D id="old" x="0" y="0.23" z="-3" rate="0" velocityZ="10" lifetime="2"
        collisionRadius="0.1" bounce="0" dt="0.01" colliders="solid"><burst time="0.6" count="1"/></particles3D>
        <particles3D id="moved" x="3" y="0.23" z="-3" rate="0" velocityZ="10" lifetime="2"
        collisionRadius="0.1" bounce="0" dt="0.01" colliders="solid"><burst time="0.6" count="1"/></particles3D>"#,
    ));
    for time in [1., 0.1, 1.] {
        let f = ev.evaluate(time);
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        if time < 0.6 {
            continue;
        }
        let particle =
            |id| &f.nodes.iter().find(|n| &*n.id == id).unwrap().particles3d.as_ref().unwrap().frame.particles[0];
        assert!((particle("old").position[2] - 1.).abs() < 1e-6, "retired source: {:?}", particle("old"));
        assert!((particle("moved").position[2] + 1.1).abs() < 1e-5, "moving pieces: {:?}", particle("moved"));
    }
}

#[test]
fn pyro_uses_piece_occupancy_and_boundary_velocity_after_release() {
    let ev = evaluator(&scene(
        r#"<object3D id="cloud" primitive="volume"><pyro width="12" height="8" depth="8" voxelSize="1"
        dt="0.1" boundary="open" pressureIterations="500" colliders="solid">
        <pyroSource shape="box" width="20" height="20" depth="20" densityRate="10"/>
        </pyro></object3D>"#,
    ));
    let mut key = None;
    for time in [0.1, 1., 0.1, 1.] {
        let f = ev.evaluate(time);
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        let v = f.nodes.iter().find(|n| &*n.id == "cloud").unwrap().sim_volume.as_ref().unwrap();
        let density = v.data.grid("density").unwrap();
        if time == 0.1 {
            assert_eq!(density.sample_world([-0.5, 0.5, 0.5]), 0.);
        } else {
            assert!(density.sample_world([-0.5, 0.5, 0.5]) > 0.5, "retired source must admit smoke");
            assert_eq!(density.sample_world([2.5, 0.5, 0.5]), 0., "moving pieces must exclude smoke");
            let velocity = v.data.grid("velocity.x").unwrap().sample_world([2.5, 0.5, 0.5]);
            assert!((velocity - 4.).abs() < 1e-5, "piece boundary velocity: {velocity}");
            if let Some(key) = key {
                assert_eq!(v.key, key, "backward replay");
            }
            key = Some(v.key);
        }
    }
}

#[test]
fn baked_physics_preserves_fragment_collisions_for_both_consumers() {
    let xml = scene(
        r#"<particles3D id="dust" x="3" y="0.23" z="-3" rate="0" velocityZ="10" lifetime="2"
        collisionRadius="0.1" bounce="0" dt="0.01" colliders="solid"><burst time="0.6" count="1"/></particles3D>
        <object3D id="cloud" primitive="volume"><pyro width="12" height="8" depth="8" voxelSize="1"
        dt="0.1" boundary="open" pressureIterations="500" colliders="solid">
        <pyroSource shape="box" width="20" height="20" depth="20" densityRate="10"/>
        </pyro></object3D>"#,
    );
    let live = evaluator(&xml);
    let path = std::env::temp_dir().join(format!("sr-fracture-collider-cache-{}.bin", std::process::id()));
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(path.clone());
    std::fs::write(&path, live.physics_cache().unwrap()).unwrap();
    let baked = evaluator(&xml.replace("<physics ", &format!("<physics cache=\"{}\" ", path.display())));
    for time in [0.1, 1., 0.3, 1.] {
        let a = live.evaluate(time);
        let b = baked.evaluate(time);
        assert!(a.problems.is_empty(), "{:?}", a.problems);
        assert!(b.problems.is_empty(), "{:?}", b.problems);
        for id in ["dust", "cloud"] {
            let a = a.nodes.iter().find(|n| &*n.id == id).unwrap();
            let b = b.nodes.iter().find(|n| &*n.id == id).unwrap();
            if id == "dust" {
                assert_eq!(a.particles3d.as_ref().unwrap().key, b.particles3d.as_ref().unwrap().key, "t={time}");
            } else {
                assert_eq!(a.sim_volume.as_ref().unwrap().key, b.sim_volume.as_ref().unwrap().key, "t={time}");
            }
        }
    }
}

#[test]
fn fracture_collider_allocations_respect_consumer_mesh_budgets() {
    for consumer in [
        r#"<particles3D id="dust" x="3" rate="0" dt="0.1" meshMemoryMiB="1" colliders="solid">
        <burst time="0" count="1"/></particles3D>"#,
        r#"<object3D id="cloud" primitive="volume"><pyro width="4" height="4" depth="4" voxelSize="1"
        dt="0.1" meshMemoryMiB="1" colliders="solid"/></object3D>"#,
    ] {
        let ev = evaluator(&scene(consumer).replace("pieces=\"4\"", "pieces=\"64\""));
        let before = ev.evaluate(0.1);
        assert!(before.problems.is_empty(), "{:?}", before.problems);
        let after = ev.evaluate(0.4);
        assert!(after.problems.iter().any(|p| p.contains("memory") || p.contains("budget")), "{:?}", after.problems);
        let before = ev.evaluate(0.1);
        assert!(before.problems.is_empty(), "a failed future request must preserve replay: {:?}", before.problems);
    }
}

#[test]
fn fragment_smoke_colliders_use_domain_axes_under_rotation_and_reflection() {
    for (transform, point, channel) in
        [(r#"rotation="90""#, [0.5, -2.5, 0.5], "velocity.y"), (r#"scaleX="-1""#, [-2.5, 0.5, 0.5], "velocity.x")]
    {
        let ev = evaluator(&scene(&format!(
            r#"<object3D id="cloud" primitive="volume" {transform}><pyro width="12" height="12" depth="8"
            voxelSize="1" dt="0.1" boundary="open" pressureIterations="500" colliders="solid">
            <pyroSource shape="box" width="20" height="20" depth="20" densityRate="10"/>
            </pyro></object3D>"#,
        )));
        let f = ev.evaluate(1.);
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        let v = f.nodes.iter().find(|n| &*n.id == "cloud").unwrap().sim_volume.as_ref().unwrap();
        assert_eq!(v.data.grid("density").unwrap().sample_world(point), 0., "{transform}");
        let speed = v.data.grid(channel).unwrap().sample_world(point);
        assert!((speed + 4.).abs() < 1e-5, "{transform}: {speed}");
    }
}
