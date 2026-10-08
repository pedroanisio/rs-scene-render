use super::common;
use common::*;

#[test]
fn text_and_clay_keep_their_rendered_solid_at_fracture_release() {
    let Some(gpu) = gpu() else { return };
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../sr-eval/tests/fixtures/solid.ttf");
    for mode in ["raster", "pathtrace"] {
        for (kind, attributes, children) in [
            ("text", r#"text="O" font="SR Solid Test" height="48" depth="12""#, ""),
            ("clay", r#"resolution="10""#, r#"<blob radius="18" blend="0"/>"#),
        ] {
            let xml = format!(
                r##"<scene version="1.3"><project width="96" height="96" fps="10" duration="2" background="#00000000"/>
                <assets><font id="font" family="SR Solid Test" src="{}"/></assets>
                <materials><material id="outside" baseColor="#00ff00" unlit="true"/><material id="inside" baseColor="#ff0000" unlit="true"/></materials>
                <composition><camera id="camera" projection="orthographic" orthoHeight="96" x="48" y="48" z="-200"
                renderer="{mode}" pathSamples="1" maxBounces="1" denoise="false"/>
                <object3D id="solid" primitive="{kind}" {attributes} x="48" y="48" material="outside">{children}
                <rigidBody mass="4" collidesWith="none"/><fracture at="1" pieces="4" seed="42" interiorMaterial="inside"/>
                </object3D></composition><physics gravityY="0" pixelsPerMeter="1" fixedStep="0.01"/></scene>"##,
                font.display()
            );
            let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
            let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
            let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
            let mut masks = Vec::new();
            for time in [0., 1., 0., 1.] {
                let graph = ev.evaluate(time);
                assert!(graph.problems.is_empty(), "{kind}: {:?}", graph.problems);
                let frame = renderer.render(&graph, ev.program());
                assert!(
                    frame.stats.errors.is_empty() && frame.stats.unsupported.is_empty(),
                    "{kind}: {:?}",
                    frame.stats
                );
                let pixels = renderer.read(&frame.texture);
                let mask: Vec<_> = pixels.iter().map(|p| p[1] > 0.5 && p[3] > 0.5).collect();
                assert!(mask.iter().filter(|&&p| p).count() > 200, "{mode}/{kind}: missing exterior");
                if kind == "text" {
                    assert!(!mask[48 * 96 + 48], "glyph counter must remain empty");
                }
                masks.push(mask);
            }
            let changed = masks[0].iter().zip(&masks[1]).filter(|(a, b)| a != b).count();
            assert!(changed < 12, "{mode}/{kind}: {changed} pixels changed at release");
            assert_eq!(masks[0], masks[2]);
            assert_eq!(masks[1], masks[3]);
        }
    }
}

#[test]
fn xml_fracture_renders_exterior_and_interior_in_both_modes_and_replays() {
    let Some(gpu) = gpu() else { return };
    for mode in ["raster", "pathtrace"] {
        let xml = format!(
            r##"<scene version="1.3"><project width="96" height="96" fps="10" duration="3" background="#00000000"/>
        <materials><material id="outside" baseColor="#00ff00" unlit="true"/><material id="inside" baseColor="#ff0000" unlit="true"/></materials>
        <composition><camera id="camera" projection="orthographic" orthoHeight="96" x="48" y="48" z="-200" renderer="{mode}" pathSamples="1" maxBounces="1" denoise="false"/>
        <group id="isolated" isolate="true"><object3D id="rock" primitive="box" width="24" height="24" depth="24" x="48" y="48" rotationY="20" rotationX="15" material="outside">
        <rigidBody mass="4" collidesWith="none" linearDamping="0" angularDamping="0"/>
        <fracture at="1" pieces="8" seed="42" interiorMaterial="inside" radialImpulse="80"/>
        </object3D></group></composition><physics gravityY="0" pixelsPerMeter="1" fixedStep="0.01"/></scene>"##
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        let mut images = Vec::new();
        for t in [0., 2., 0., 2.] {
            let graph = ev.evaluate(t);
            assert!(graph.problems.is_empty(), "{:?}", graph.problems);
            let frame = renderer.render(&graph, ev.program());
            assert!(frame.stats.errors.is_empty() && frame.stats.unsupported.is_empty(), "{:?}", frame.stats);
            images.push(renderer.read(&frame.texture));
        }
        let red = |image: &Vec<[f32; 4]>| image.iter().filter(|p| p[0] > 0.5 && p[1] < 0.1 && p[3] > 0.5).count();
        assert_eq!(red(&images[0]), 0);
        assert!(red(&images[1]) > 20, "{mode}: newly exposed interior must be red");
        assert!(images[1].iter().any(|p| p[1] > 0.5 && p[0] < 0.1), "exterior retains its material");
        assert_eq!(images[0], images[2], "{mode}: source replay");
        assert_eq!(images[1], images[3], "{mode}: fragment replay and group invalidation");
    }
}

#[test]
fn imported_fracture_keeps_texture_maps_and_separate_interior_material() {
    let Some(gpu) = gpu() else { return };
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Fixture(std::env::temp_dir().join(format!("sr-fracture-gpu-import-{}", std::process::id())));
    std::fs::create_dir(&dir.0).unwrap();
    let mesh = sr_3d::prim::cuboid(24., 24., 24.);
    let mut obj = "mtllib rock.mtl\nusemtl rock\n".to_string();
    for v in &mesh.vertices {
        obj.push_str(&format!(
            "v {} {} {}\nvt {} {}\nvn {} {} {}\n",
            v.pos[0], v.pos[1], v.pos[2], v.uv[0], v.uv[1], v.normal[0], v.normal[1], v.normal[2]
        ));
    }
    for t in mesh.indices.as_chunks::<3>().0 {
        obj.push_str(&format!("f {0}/{0}/{0} {1}/{1}/{1} {2}/{2}/{2}\n", t[0] + 1, t[1] + 1, t[2] + 1));
    }
    std::fs::write(dir.0.join("rock.obj"), obj).unwrap();
    std::fs::write(dir.0.join("rock.mtl"), "newmtl rock\nKd 1 1 1\nmap_Kd color.png\n").unwrap();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 255, 255])).save(dir.0.join("color.png")).unwrap();
    for mode in ["raster", "pathtrace"] {
        let xml = format!(
            r##"<scene version="1.3"><project width="96" height="96" fps="10" duration="3" background="#00000000"/>
        <assets><mesh id="source" src="rock.obj"/></assets><materials><material id="inside" baseColor="#ff0000" unlit="true"/></materials>
        <composition><camera id="camera" projection="orthographic" orthoHeight="96" x="48" y="48" z="-200" renderer="{mode}" pathSamples="1" maxBounces="1" denoise="false"/>
        <object3D id="rock" primitive="mesh" mesh="source" scaleX="0.01" scaleY="0.01" scaleZ="0.01" x="48" y="48" rotationY="20" rotationX="15">
        <rigidBody mass="4" collidesWith="none" linearDamping="0"/>
        <fracture at="1" pieces="8" seed="42" interiorMaterial="inside" radialImpulse="80"/>
        </object3D></composition><lights><light id="ambient" type="ambient" intensity="1"/></lights><physics gravityY="0" pixelsPerMeter="1" fixedStep="0.01"/></scene>"##
        );
        let doc =
            sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir.0.clone()) })
                .unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        let graph = ev.evaluate(2.);
        assert!(graph.problems.is_empty(), "{:?}", graph.problems);
        let frame = renderer.render(&graph, ev.program());
        assert!(frame.stats.errors.is_empty() && frame.stats.unsupported.is_empty(), "{:?}", frame.stats);
        let pixels = renderer.read(&frame.texture);
        assert!(
            pixels.iter().filter(|p| p[2] > 0.05 && p[2] > p[0] * 3. && p[2] > p[1] * 3.).count() > 20,
            "{mode}: imported blue texture missing"
        );
        assert!(
            pixels.iter().filter(|p| p[0] > 0.5 && p[1] < 0.1 && p[2] < 0.1).count() > 20,
            "{mode}: red interior missing"
        );
    }
}

#[test]
fn fractured_globe_preserves_map_drape() {
    let Some(gpu) = gpu() else { return };
    for mode in ["raster", "pathtrace"] {
        let xml = format!(
            r##"<scene version="1.3"><project width="96" height="96" fps="10" duration="3" background="#00000000"/>
        <assets><map id="map" width="64" height="32" background="#0000ff"/></assets><materials><material id="inside" baseColor="#ff0000" unlit="true"/></materials>
        <composition><camera id="camera" projection="orthographic" orthoHeight="96" x="48" y="48" z="-200" renderer="{mode}" pathSamples="1" maxBounces="1" denoise="false"/>
        <object3D id="rock" primitive="globe" map="map" radius="12" segments="24" textureSize="64" x="48" y="48">
        <rigidBody mass="4" collidesWith="none" linearDamping="0"/><fracture at="1" pieces="4" seed="42" interiorMaterial="inside" radialImpulse="80"/>
        </object3D></composition><lights><light id="ambient" type="ambient" intensity="1"/></lights><physics gravityY="0" pixelsPerMeter="1" fixedStep="0.01"/></scene>"##
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        let graph = ev.evaluate(2.);
        assert!(graph.problems.is_empty(), "{:?}", graph.problems);
        let frame = renderer.render(&graph, ev.program());
        assert!(frame.stats.errors.is_empty() && frame.stats.unsupported.is_empty(), "{:?}", frame.stats);
        let pixels = renderer.read(&frame.texture);
        assert!(
            pixels.iter().filter(|p| p[2] > 0.05 && p[2] > p[0] * 3. && p[2] > p[1] * 3.).count() > 20,
            "{mode}: blue map drape missing"
        );
    }
}
