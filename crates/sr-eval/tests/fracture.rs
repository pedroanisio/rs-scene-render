fn xml(extra: &str, fracture: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="4"/>
      <materials><material id="inside" baseColor="#ff0000"/></materials>
      <composition><object3D id="rock" primitive="box" width="4" height="2" depth="2" {extra}>
        <rigidBody mass="4" velocityX="2" linearDamping="0" angularDamping="0" collidesWith="none"/>
        <fracture at="1" pieces="4" seed="18446744073709551615" interiorMaterial="inside" {fracture}/>
      </object3D></composition><physics gravityY="0" pixelsPerMeter="1" fixedStep="0.01"/></scene>"##
    )
}
fn evaluator(extra: &str, fracture: &str) -> sr_eval::Evaluator {
    let xml = xml(extra, fracture);
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
}

#[test]
fn xml_fracture_replaces_source_at_boundary_and_replays() {
    let ev = evaluator("", "impulseY=\"8\"");
    let before = ev.evaluate(0.99);
    assert!(before.problems.is_empty(), "{:?}", before.problems);
    assert!(before.nodes[0].fracture.is_none());
    let at = ev.evaluate(1.0);
    assert!(at.problems.is_empty(), "{:?}", at.problems);
    let split = at.nodes[0].fracture.as_ref().expect("XML fracture must reach the frame graph");
    assert_eq!(split.poses.len(), 4);
    assert!(split.enabled.iter().all(|v| *v));
    assert_eq!(split.geometry.interior_material, "inside");
    let total: f64 = split.geometry.pieces.iter().map(|p| p.mass).sum();
    assert!((total - 4.).abs() < 1e-9);
    let end = ev.evaluate(2.0);
    let moved = end.nodes[0].fracture.as_ref().unwrap();
    for (a, b) in split.poses.iter().zip(&moved.poses) {
        assert!((b.pos[0] - a.pos[0] - 2.).abs() < 1e-8);
        assert!((b.pos[1] - a.pos[1] - 2.).abs() < 1e-8);
    }
    assert!(ev.evaluate(0.5).nodes[0].fracture.is_none());
    assert_eq!(ev.evaluate(2.).nodes[0].fracture.as_ref().unwrap().poses, moved.poses);
}

#[test]
fn fracture_waits_for_source_visibility() {
    let ev = evaluator("start=\"2\"", "");
    assert!(ev.evaluate(1.5).nodes.is_empty());
    let at = ev.evaluate(2.);
    assert!(at.problems.is_empty(), "{:?}", at.problems);
    assert!(at.nodes[0].fracture.is_some());
}

#[test]
fn fracture_bake_preserves_replacement_flags_and_piece_poses() {
    let live = evaluator("", "radialImpulse=\"4\"");
    let bytes = live.physics_cache().unwrap();
    let path = std::env::temp_dir().join(format!("sr-fracture-cache-{}.bin", std::process::id()));
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(path.clone());
    std::fs::write(&path, bytes).unwrap();
    let xml = xml("", "radialImpulse=\"4\"").replace("<physics ", &format!("<physics cache=\"{}\" ", path.display()));
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let baked = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    for t in [0.99, 1., 2., 0.5, 3., 1.] {
        let expected = live.evaluate(t);
        let actual = baked.evaluate(t);
        assert!(actual.problems.is_empty(), "{:?}", actual.problems);
        let a = &actual.nodes[0].fracture;
        let b = &expected.nodes[0].fracture;
        assert_eq!(a.is_some(), b.is_some(), "t={t}");
        if let (Some(a), Some(b)) = (a, b) {
            assert_eq!(a.poses, b.poses);
            assert_eq!(a.enabled, b.enabled);
        }
    }
}

#[test]
fn closed_procedural_sources_retain_outward_surfaces_under_reflection() {
    for primitive in ["box", "sphere", "cylinder", "cone", "capsule", "torus"] {
        let xml = xml("scaleX=\"-2\" scaleY=\"1.5\"", "")
            .replace("primitive=\"box\"", &format!("primitive=\"{primitive}\" radius=\"3\" segments=\"12\""));
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let frame = ev.evaluate(1.);
        assert!(frame.problems.is_empty(), "{primitive}: {:?}", frame.problems);
        let split = frame.nodes[0].fracture.as_ref().unwrap();
        for piece in &split.geometry.pieces {
            for surface in &piece.surfaces {
                for v in &surface.mesh.vertices {
                    assert!(v.pos.iter().chain(&v.normal).all(|v| v.is_finite()));
                }
            }
        }
    }
}

#[test]
fn fracture_source_freezes_animated_dimensions_at_first_visible_boundary() {
    let xml = xml("start=\"2\"", "")
        .replace("<rigidBody", "<animate property=\"width\" timeBase=\"composition\"><key time=\"0\" value=\"4\"/><key time=\"2\" value=\"8\"/></animate><rigidBody");
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(2.);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    let split = frame.nodes[0].fracture.as_ref().unwrap();
    let xs: Vec<_> = split
        .geometry
        .pieces
        .iter()
        .flat_map(|p| {
            p.surfaces.iter().flat_map(move |s| s.mesh.vertices.iter().map(move |v| v.pos[0] as f64 + p.offset[0]))
        })
        .collect();
    let min = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let max = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!((max - min - 8.).abs() < 1e-5, "release width: {}", max - min);
}

#[test]
fn clay_fracture_freezes_animated_blobs_at_release() {
    let xml = xml("", "maxMemoryMiB=\"8\"")
        .replace("primitive=\"box\"", "primitive=\"clay\" resolution=\"10\"")
        .replace("<rigidBody", "<blob radius=\"3\" blend=\"0\"><animate property=\"x\"><key time=\"0\" value=\"0\"/><key time=\"1\" value=\"2\"/><key time=\"2\" value=\"9\"/></animate></blob><rigidBody")
        .replace("velocityX=\"2\"", "velocityX=\"0\"");
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let f = ev.evaluate(1.);
    assert!(f.problems.is_empty(), "{:?}", f.problems);
    let fracture = f.nodes[0].fracture.as_ref().unwrap();
    let center: f64 = fracture.geometry.pieces.iter().map(|p| p.mass * p.offset[0]).sum::<f64>() / 4.;
    assert!((center - 2.).abs() < 1e-5, "release-time blob center: {center}");
    let later = ev.evaluate(2.);
    assert!(later.problems.is_empty(), "{:?}", later.problems);
    assert_eq!(fracture.poses, later.nodes[0].fracture.as_ref().unwrap().poses);
}

#[test]
fn text_fracture_uses_closed_extruded_glyphs_with_counters() {
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/solid.ttf");
    let xml = xml("", "maxMemoryMiB=\"1\"")
        .replace(
            "<materials>",
            &format!(
                "<assets><font id=\"face\" family=\"SR Solid Test\" src=\"{}\"/></assets><materials>",
                font.display()
            ),
        )
        .replace("primitive=\"box\"", "primitive=\"text\" text=\"O\" font=\"SR Solid Test\"")
        .replace("height=\"2\"", "height=\"20\"")
        .replace("velocityX=\"2\"", "velocityX=\"0\"");
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let f = ev.evaluate(1.);
    assert!(f.problems.is_empty(), "{:?}", f.problems);
    let fracture = f.nodes[0].fracture.as_ref().unwrap();
    assert_eq!(fracture.geometry.pieces.len(), 4);
    let total: f64 = fracture.geometry.pieces.iter().map(|p| p.mass).sum();
    assert!((total - 4.).abs() < 1e-8);
    for p in &fracture.geometry.pieces {
        assert!(p.surfaces.iter().flat_map(|s| &s.mesh.vertices).all(|v| v.pos.iter().all(|v| v.is_finite())));
        let mut points = Vec::new();
        let mut triangles = Vec::new();
        for s in &p.surfaces {
            let base = points.len() as u32;
            points.extend(s.mesh.vertices.iter().map(|v| std::array::from_fn(|i| v.pos[i] as f64 + p.offset[i])));
            triangles.extend(s.mesh.indices.as_chunks::<3>().0.iter().map(|t| t.map(|i| i + base)));
        }
        let region = sr_sim::pyro::mesh::Mesh::new(&points, &triangles, 16 << 20).unwrap();
        assert!(!region.contains([0.; 3]), "glyph counter must remain empty after fracture");
    }
}

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "sr-fracture-assets-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn imported_fracture_freezes_node_animation_and_authored_morph_weights() {
    use serde_json::json;
    let dir = Fixture::new();
    let mesh = sr_3d::prim::cuboid(2., 2., 2.);
    let mut bytes = Vec::new();
    let mut views = Vec::new();
    let mut add = |values: Vec<f32>| {
        let offset = bytes.len();
        for v in values {
            bytes.extend(v.to_le_bytes());
        }
        views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":bytes.len()-offset}));
    };
    add(mesh.vertices.iter().flat_map(|v| v.pos).collect());
    add(mesh.vertices.iter().flat_map(|v| v.normal).collect());
    add(mesh.vertices.iter().flat_map(|v| [v.pos[0], 0., 0.]).collect());
    add(vec![0., 2.]);
    add(vec![0., 0., 0., 4., 0., 0.]);
    let offset = bytes.len();
    for i in &mesh.indices {
        bytes.extend(i.to_le_bytes());
    }
    views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":bytes.len()-offset}));
    let v = mesh.vertices.len();
    let data = json!({"asset":{"version":"2.0"},"buffers":[{"uri":"shape.bin","byteLength":bytes.len()}],"bufferViews":views,
      "accessors":[{"bufferView":0,"componentType":5126,"count":v,"type":"VEC3","min":[-1,-1,-1],"max":[1,1,1]},
        {"bufferView":1,"componentType":5126,"count":v,"type":"VEC3"},
        {"bufferView":2,"componentType":5126,"count":v,"type":"VEC3"},
        {"bufferView":3,"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[2]},
        {"bufferView":4,"componentType":5126,"count":2,"type":"VEC3"},
        {"bufferView":5,"componentType":5125,"count":mesh.indices.len(),"type":"SCALAR"}],
      "meshes":[{"weights":[0],"primitives":[{"attributes":{"POSITION":0,"NORMAL":1},"indices":5,"targets":[{"POSITION":2}]}]}],
      "nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0,
      "animations":[{"name":"move","samplers":[{"input":3,"output":4,"interpolation":"LINEAR"}],"channels":[{"sampler":0,"target":{"node":0,"path":"translation"}}]}]});
    std::fs::write(dir.0.join("shape.gltf"), serde_json::to_vec(&data).unwrap()).unwrap();
    std::fs::write(dir.0.join("shape.bin"), bytes).unwrap();
    let xml = xml("scaleX=\"0.01\" scaleY=\"0.01\" scaleZ=\"0.01\" animationClip=\"move\" morphWeights=\"0.5\"", "")
        .replace("<materials>", "<assets><mesh id=\"source\" src=\"shape.gltf\"/></assets><materials>")
        .replace("primitive=\"box\"", "primitive=\"mesh\" mesh=\"source\"");
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir.0.clone()) })
        .unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(1.);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    let split = frame.nodes[0].fracture.as_ref().unwrap();
    let xs: Vec<_> = split
        .geometry
        .pieces
        .iter()
        .flat_map(|p| {
            p.surfaces.iter().flat_map(move |s| s.mesh.vertices.iter().map(move |v| v.pos[0] as f64 + p.offset[0]))
        })
        .collect();
    let min = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let max = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!((min - 0.5).abs() < 1e-5 && (max - 3.5).abs() < 1e-5, "frozen glTF x bounds {min}..{max}");
}

#[test]
fn imported_fracture_freezes_two_clips_blended_by_animation_blend() {
    use serde_json::json;
    let dir = Fixture::new();
    let mesh = sr_3d::prim::cuboid(2., 2., 2.);
    let mut bytes = Vec::new();
    let mut views = Vec::new();
    let mut add = |values: Vec<f32>| {
        let offset = bytes.len();
        for v in values {
            bytes.extend(v.to_le_bytes());
        }
        views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":bytes.len()-offset}));
    };
    add(mesh.vertices.iter().flat_map(|v| v.pos).collect());
    add(mesh.vertices.iter().flat_map(|v| v.normal).collect());
    add(mesh.vertices.iter().flat_map(|v| [v.pos[0], 0., 0.]).collect());
    add(vec![0., 2.]);
    add(vec![0., 0., 0., 4., 0., 0.]);
    add(vec![0., 0., 0., 8., 0., 0.]);
    let offset = bytes.len();
    for i in &mesh.indices {
        bytes.extend(i.to_le_bytes());
    }
    views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":bytes.len()-offset}));
    let v = mesh.vertices.len();
    let data = json!({"asset":{"version":"2.0"},"buffers":[{"uri":"shape.bin","byteLength":bytes.len()}],"bufferViews":views,
      "accessors":[{"bufferView":0,"componentType":5126,"count":v,"type":"VEC3","min":[-1,-1,-1],"max":[1,1,1]},
        {"bufferView":1,"componentType":5126,"count":v,"type":"VEC3"},
        {"bufferView":2,"componentType":5126,"count":v,"type":"VEC3"},
        {"bufferView":3,"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[2]},
        {"bufferView":4,"componentType":5126,"count":2,"type":"VEC3"},
        {"bufferView":5,"componentType":5126,"count":2,"type":"VEC3"},
        {"bufferView":6,"componentType":5125,"count":mesh.indices.len(),"type":"SCALAR"}],
      "meshes":[{"weights":[0],"primitives":[{"attributes":{"POSITION":0,"NORMAL":1},"indices":6,"targets":[{"POSITION":2}]}]}],
      "nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0,
      "animations":[{"name":"move","samplers":[{"input":3,"output":4,"interpolation":"LINEAR"}],"channels":[{"sampler":0,"target":{"node":0,"path":"translation"}}]},
        {"name":"far","samplers":[{"input":3,"output":5,"interpolation":"LINEAR"}],"channels":[{"sampler":0,"target":{"node":0,"path":"translation"}}]}]});
    std::fs::write(dir.0.join("shape.gltf"), serde_json::to_vec(&data).unwrap()).unwrap();
    std::fs::write(dir.0.join("shape.bin"), bytes).unwrap();
    let xml = xml("scaleX=\"0.01\" scaleY=\"0.01\" scaleZ=\"0.01\" animationClip=\"move\" animationClipTo=\"far\" animationBlend=\"0.5\" morphWeights=\"0.5\"", "")
        .replace("<materials>", "<assets><mesh id=\"source\" src=\"shape.gltf\"/></assets><materials>")
        .replace("primitive=\"box\"", "primitive=\"mesh\" mesh=\"source\"");
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir.0.clone()) })
        .unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(1.);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    let split = frame.nodes[0].fracture.as_ref().unwrap();
    let xs: Vec<_> = split
        .geometry
        .pieces
        .iter()
        .flat_map(|p| {
            p.surfaces.iter().flat_map(move |s| s.mesh.vertices.iter().map(move |v| v.pos[0] as f64 + p.offset[0]))
        })
        .collect();
    let min = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let max = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    // translation at t = 1: 2 in "move", 4 in "far"; halfway 3, and the half morph stretches the cube to +-1.5
    assert!((min - 1.5).abs() < 1e-5 && (max - 4.5).abs() < 1e-5, "frozen blended x bounds {min}..{max}");
}

#[test]
fn fracture_samples_numbered_mesh_geometry_at_release() {
    let dir = Fixture::new();
    for (frame, width) in [(0, 2.), (1, 4.)] {
        let mesh = sr_3d::prim::cuboid(width, 2., 2.);
        let mut obj = String::new();
        for v in &mesh.vertices {
            obj.push_str(&format!("v {} {} {}\n", v.pos[0], v.pos[1], v.pos[2]));
        }
        for t in mesh.indices.as_chunks::<3>().0 {
            obj.push_str(&format!("f {} {} {}\n", t[0] + 1, t[1] + 1, t[2] + 1));
        }
        std::fs::write(dir.0.join(format!("shape-{frame}.obj")), obj).unwrap();
    }
    let xml = xml("scaleX=\"0.01\" scaleY=\"0.01\" scaleZ=\"0.01\"","")
        .replace("<materials>","<assets><meshSequence id=\"source\" src=\"shape-%d.obj\" first=\"0\" last=\"1\" fps=\"1\"/></assets><materials>")
        .replace("primitive=\"box\"","primitive=\"mesh\" mesh=\"source\"")
        .replace("<rigidBody ","<rigidBody type=\"kinematic\" ");
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir.0.clone()) })
        .unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(1.);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    let split = frame.nodes[0].fracture.as_ref().unwrap();
    let xs: Vec<_> = split
        .geometry
        .pieces
        .iter()
        .flat_map(|p| {
            p.surfaces.iter().flat_map(move |s| s.mesh.vertices.iter().map(move |v| v.pos[0] as f64 + p.offset[0]))
        })
        .collect();
    let min = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let max = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!((max - min - 4.).abs() < 1e-5, "release mesh sequence width {}", max - min);
}

#[test]
fn fracture_freezes_crater_deformation_before_cutting() {
    let xml = xml("", "").replace("<rigidBody ", "<rigidBody type=\"kinematic\" ").replace(
        "<fracture ",
        "<crater radius=\"10\" depth=\"2\" rimHeight=\"0\" rimWidth=\"1\" influenceDepth=\"8\" end=\"0.5\"/><fracture ",
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(1.);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    let n = &frame.nodes[0];
    let crater = sr_eval::crater::at(n).unwrap().unwrap();
    let split = n.fracture.as_ref().unwrap();
    let points: Vec<_> = split
        .geometry
        .pieces
        .iter()
        .flat_map(|p| {
            p.surfaces.iter().flat_map(move |s| s.mesh.vertices.iter().map(move |v| v.pos[2] as f64 + p.offset[2]))
        })
        .collect();
    let min = points.iter().copied().fold(f64::INFINITY, f64::min);
    let max = points.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let lo = crater.kernel.map([2., 1., -1.], 1.).unwrap().position[2];
    let hi = crater.kernel.map([2., 1., 1.], 1.).unwrap().position[2];
    assert!((min - lo).abs() < 1e-5 && (max - hi).abs() < 1e-5, "crater split z={min}..{max}, want {lo}..{hi}");
}

#[test]
fn fracture_uses_globe_elevation_instead_of_a_spherical_proxy() {
    let dir = Fixture::new();
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(2, 2, image::Rgb([128, 100, 0])).write_to(&mut png, image::ImageFormat::Png).unwrap();
    let tiles = vec![((0, 0, 0), png.into_inner())];
    std::fs::write(
        dir.0.join("dem.pmtiles"),
        sr_geo::pmtiles::write(&tiles, sr_geo::pmtiles::TileType::Png, 1, &serde_json::json!({})),
    )
    .unwrap();
    let xml = xml("","")
        .replace("<materials>","<assets><tiles id=\"dem\" src=\"dem.pmtiles\"/><map id=\"map\" width=\"64\" height=\"32\"/></assets><materials>")
        .replace("primitive=\"box\"","primitive=\"globe\" map=\"map\" radius=\"4\" segments=\"24\" terrain=\"dem\" planetRadius=\"100\" terrainTileSize=\"2\" terrainZoom=\"0\"");
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir.0.clone()) })
        .unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(1.);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    let split = frame.nodes[0].fracture.as_ref().unwrap();
    let extent = split
        .geometry
        .pieces
        .iter()
        .flat_map(|p| {
            p.surfaces
                .iter()
                .flat_map(move |s| s.mesh.vertices.iter().map(move |v| (v.pos[0] as f64 + p.offset[0]).abs()))
        })
        .fold(0., f64::max);
    assert!((extent - 8.).abs() < 1e-4, "equatorial radius with elevation: {extent}");
}
