fn sources() -> [(&'static str, &'static str); 3] {
    [
        (r#"primitive="text" text="O" font="SR Solid Test" height="10" depth="2""#, ""),
        (r#"primitive="extrude" depth="2" path="M-3 -4 L3 -4 L3 4 L-3 4 Z M-1 -2 L-1 2 L1 2 L1 -2 Z""#, ""),
        (
            r#"primitive="clay" resolution="16""#,
            r#"<blob shape="box" width="6" height="8" depth="2" blend="0"/>
            <blob shape="box" width="2" height="4" depth="4" blend="0" subtract="true"/>"#,
        ),
    ]
}

fn scene(attributes: &str, children: &str, consumer: &str) -> sr_eval::Evaluator {
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/solid.ttf");
    let xml = format!(
        r##"<scene version="1.3"><project width="32" height="32" fps="10" duration="2"/>
        <assets><font id="face" family="SR Solid Test" src="{}"/></assets>
        <materials><material id="inside" baseColor="#ff0000"/></materials><composition>
        <object3D id="solid" {attributes} start="0.2" condition="time &gt;= 0.2" visible="false">{children}</object3D>
        {consumer}</composition><physics gravityY="0" pixelsPerMeter="1" fixedStep="0.01" bounds="none"/></scene>"##,
        font.display()
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
}

#[test]
fn particles_pass_solid_holes_and_hit_their_walls() {
    for (attributes, children) in sources() {
        let ev = scene(
            attributes,
            children,
            r#"<particles3D id="hole" z="-3" rate="0" velocityZ="10" lifetime="2"
            collisionRadius="0.1" bounce="0" dt="0.05" colliders="solid"><burst time="0.3" count="1"/></particles3D>
            <particles3D id="wall" x="2" z="-3" rate="0" velocityZ="10" lifetime="2"
            collisionRadius="0.1" bounce="0" dt="0.05" colliders="solid"><burst time="0.3" count="1"/></particles3D>"#,
        );
        for time in [0.9, 0.1, 0.9] {
            let f = ev.evaluate(time);
            assert!(f.problems.is_empty(), "{attributes}: {:?}", f.problems);
            if time == 0.1 {
                continue;
            }
            let particle =
                |id| &f.nodes.iter().find(|n| &*n.id == id).unwrap().particles3d.as_ref().unwrap().frame.particles[0];
            assert!((particle("hole").position[2] - 3.).abs() < 1e-6, "{attributes}: {:?}", particle("hole"));
            assert!(particle("wall").position[2] < -0.9, "{attributes}: {:?}", particle("wall"));
            // Surface nets round the clay wall's corners, so contact may
            // retain tangential velocity while preventing passage through it.
        }
    }
}

#[test]
fn smoke_enters_holes_and_is_excluded_by_solid_walls() {
    for (attributes, children) in sources() {
        let ev = scene(
            attributes,
            children,
            r#"<object3D id="cloud" primitive="volume">
            <pyro width="8" height="10" depth="6" voxelSize="1" dt="0.1" boundary="open" pressureIterations="500" colliders="solid">
            <pyroSource shape="box" width="20" height="20" depth="20" densityRate="10"/></pyro></object3D>"#,
        );
        for time in [0.5, 0.1, 0.5] {
            let f = ev.evaluate(time);
            assert!(f.problems.is_empty(), "{attributes}: {:?}", f.problems);
            let v = f.nodes.iter().find(|n| &*n.id == "cloud").unwrap().sim_volume.as_ref().unwrap();
            let d = v.data.grid("density").unwrap();
            assert!(d.sample_world([0.5, 0.5, 0.5]) > 0.5, "{attributes}: hole");
            if time > 0.2 {
                assert_eq!(d.sample_world([2.5, 0.5, 0.5]), 0., "{attributes}: wall");
            } else {
                assert!(d.sample_world([2.5, 0.5, 0.5]) > 0.5, "before source window");
            }
        }
    }
}

#[test]
fn rigid_bodies_pass_procedural_holes_before_fracture() {
    for (attributes, children) in sources() {
        let children = format!(
            r#"{children}<rigidBody type="static" mass="4"/>
            <fracture at="1" pieces="4" seed="42" interiorMaterial="inside"/>"#
        );
        let ev = scene(
            attributes,
            &children,
            r#"<object3D id="hole" primitive="sphere" radius="0.1" z="-3" start="0.2">
            <rigidBody velocityZ="10" linearDamping="0" restitution="0"/></object3D>
            <object3D id="wall" primitive="sphere" radius="0.1" x="2.5" z="-3" start="0.2">
            <rigidBody velocityZ="10" linearDamping="0" restitution="0"/></object3D>"#,
        );
        for time in [0.8, 0.1, 0.8] {
            let f = ev.evaluate(time);
            assert!(f.problems.is_empty(), "{attributes}: {:?}", f.problems);
            if time == 0.1 {
                continue;
            }
            let z = |id| f.nodes.iter().find(|n| &*n.id == id).unwrap().pose3.as_ref().unwrap()[14];
            assert!((z("hole") - 3.).abs() < 1e-5, "{attributes}: hole body z={}", z("hole"));
            assert!(z("wall") < -0.8, "{attributes}: wall body z={}", z("wall"));
        }
    }
}

#[test]
fn procedural_colliders_replace_the_source_with_moving_fracture_pieces() {
    for (attributes, children) in sources() {
        let children = format!(
            r#"{children}<rigidBody type="static" mass="4" linearDamping="0" collidesWith="none"/>
            <fracture at="0.4" pieces="4" seed="42" interiorMaterial="inside" impulseX="16"/>"#
        );
        let ev = scene(
            attributes,
            &children,
            r#"<particles3D id="old" x="-2" z="-3" rate="0" velocityZ="10" lifetime="2"
            collisionRadius="0.1" bounce="0" dt="0.01" colliders="solid"><burst time="1.1" count="1"/></particles3D>
            <particles3D id="moved" x="6" z="-3" rate="0" velocityZ="10" lifetime="2"
            collisionRadius="0.1" bounce="0" dt="0.01" colliders="solid"><burst time="1.1" count="1"/></particles3D>
            <object3D id="cloud" primitive="volume"><pyro width="20" height="10" depth="6" voxelSize="1" dt="0.1"
            boundary="open" pressureIterations="500" colliders="solid"><pyroSource shape="box" width="30" height="20" depth="20"
            densityRate="10"/></pyro></object3D>"#,
        );
        let mut key = None;
        for time in [1.5, 0.3, 1.5] {
            let f = ev.evaluate(time);
            assert!(f.problems.is_empty(), "{attributes}: {:?}", f.problems);
            let v = f.nodes.iter().find(|n| &*n.id == "cloud").unwrap().sim_volume.as_ref().unwrap();
            let d = v.data.grid("density").unwrap();
            if time == 0.3 {
                assert_eq!(d.sample_world([-2.5, 0.5, 0.5]), 0., "source wall before release");
                continue;
            }
            assert!(d.sample_world([-2.5, 0.5, 0.5]) > 0.5, "{attributes}: retired source");
            assert_eq!(d.sample_world([6.5, 0.5, 0.5]), 0., "{attributes}: moving pieces");
            let particle =
                |id| &f.nodes.iter().find(|n| &*n.id == id).unwrap().particles3d.as_ref().unwrap().frame.particles[0];
            assert!((particle("old").position[2] - 1.).abs() < 1e-5, "{attributes}: {:?}", particle("old"));
            assert!(particle("moved").position[2] < -0.8, "{attributes}: {:?}", particle("moved"));
            if let Some(k) = key {
                assert_eq!(v.key, k);
            }
            key = Some(v.key);
        }
    }
}
