struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "sr-mesh-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn obj(&self, n: i32, x: f32) {
        std::fs::write(
            self.0.join(format!("frame-{n}.obj")),
            format!("v {x} 0 0\nv {} 0 0\nv {x} 1 0\nf 1 2 3\n", x + 1.),
        )
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn mesh_cache_playback_uses_local_clocks_and_backward_requests_reuse_frames() {
    let f = Fixture::new();
    f.obj(0, 0.);
    f.obj(1, 2.);
    let xml = r#"<scene version="1.3"><project width="64" height="64" fps="10" duration="3"/><assets><meshSequence id="frames" src="frame-%d.obj" first="0" last="1" fps="1" interpolation="linear"/></assets><composition><object3D id="o" primitive="mesh" mesh="frames" start="1" animationSpeed="2"/></composition></scene>"#;
    let doc =
        sr_model::load_str(xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(f.0.clone()) }).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let mut keys = Vec::new();
    for (time, x) in [(1., 0.), (1.25, 1.), (1.5, 2.), (1.25, 1.)] {
        let frame = ev.evaluate(time);
        assert!(frame.problems.is_empty(), "{:?}", frame.problems);
        let n = frame.nodes.iter().find(|n| &*n.id == "o").unwrap();
        let s = sr_eval::mesh_sequence::sample(ev.program(), n).unwrap().unwrap();
        assert!((s.frame.model.unwrap().primitives[0].vertices[0].pos[0] - x).abs() < 1e-6);
        keys.push(s.key);
    }
    assert_eq!(keys[1], keys[3]);
    assert_ne!(keys[0], keys[1]);
}

#[test]
fn recompiling_a_mesh_sequence_does_not_reuse_stale_external_materials() {
    let f = Fixture::new();
    f.obj(0, 0.);
    let path = f.0.join("frame-0.obj");
    let obj = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("mtllib surface.mtl\nusemtl surface\n{obj}")).unwrap();
    let xml = r#"<scene version="1.3"><project width="64" height="64" fps="10" duration="1"/><assets><meshSequence id="frames" src="frame-%d.obj" first="0" last="0" fps="1"/></assets><composition><object3D id="o" primitive="mesh" mesh="frames"/></composition></scene>"#;
    for (color, want) in [("1 0 0", [1., 0., 0., 1.]), ("0 0 1", [0., 0., 1., 1.])] {
        std::fs::write(f.0.join("surface.mtl"), format!("newmtl surface\nKd {color}\n")).unwrap();
        let doc = sr_model::load_str(xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(f.0.clone()) })
            .unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let frame = ev.evaluate(0.);
        let sample = sr_eval::mesh_sequence::sample(ev.program(), &frame.nodes[0]).unwrap().unwrap();
        assert_eq!(sample.frame.model.unwrap().materials[0].params.base_color, want);
    }
}

#[test]
fn included_mesh_sequence_resolves_its_own_assets_and_directory() {
    let main = Fixture::new();
    let library = Fixture(main.0.join("library"));
    std::fs::create_dir(&library.0).unwrap();
    library.obj(0, 7.);
    main.obj(0, 99.);
    let project = r#"<project width="64" height="64" fps="1" duration="1"/>"#;
    let asset = r#"<assets><meshSequence id="frames" src="frame-%d.obj" first="0" last="0" fps="1"/></assets>"#;
    std::fs::write(library.0.join("cache.xml"), format!(r#"<scene version="1.3">{project}{asset}<symbols><symbol id="clip" width="64" height="64"><object3D id="mesh" primitive="mesh" mesh="frames"/></symbol></symbols><composition/></scene>"#)).unwrap();
    let xml = format!(
        r#"<scene version="1.3">{project}{asset}<composition><include id="inc" src="library/cache.xml" symbol="clip"/></composition></scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(main.0.clone()) })
        .unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(0.);
    let node = frame.nodes.iter().find(|n| &*n.id == "inc/mesh").unwrap();
    let sampled = sr_eval::mesh_sequence::sample(ev.program(), node).unwrap().unwrap();
    assert_eq!(sampled.frame.model.unwrap().primitives[0].vertices[0].pos[0], 7.);
}
