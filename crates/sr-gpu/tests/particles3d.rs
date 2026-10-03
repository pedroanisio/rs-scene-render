mod common;

#[test]
fn particles_share_native_depth_and_redraw_inside_cached_groups() {
    let Some(gpu) = common::gpu() else {
        return;
    };
    let xml = r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="3"/><materials><material id="red" baseColor="#FF0000" unlit="true"/><material id="blue" baseColor="#0000FF" unlit="true"/></materials><composition>
      <camera id="cam" projection="orthographic" x="32" y="32" z="-100"/>
      <group id="g" isolate="true">
        <particles3D id="dust" material="red" rate="0" size="20" x="32" y="32" z="10" velocityX="10" lifetime="3" dt="0.1"><burst time="0" count="1"/></particles3D>
        <object3D id="cover" primitive="box" material="blue" x="32" y="32" z="0" width="8" height="8" depth="2"/>
      </group></composition></scene>"##;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let mut renderer = sr_gpu::Renderer::new(gpu, ev.program());
    let mut images = Vec::new();
    for t in [0., 1., 0.] {
        let g = ev.evaluate(t);
        assert!(g.problems.is_empty(), "{:?}", g.problems);
        let f = renderer.render(&g, ev.program());
        assert!(f.stats.errors.is_empty() && f.stats.unsupported.is_empty(), "{:?}", f.stats);
        images.push(renderer.read(&f.texture));
    }
    assert!(images[0][32 * 64 + 32][2] > 0.8, "occluder should remain blue");
    assert!(
        images[0].iter().filter(|p| p[0] > 0.8 && p[1] < 0.1 && p[2] < 0.1).count() > 40,
        "native red particle missing"
    );
    assert_ne!(images[0], images[1]);
    assert_eq!(images[0], images[2]);
}

#[test]
fn billboard_streak_and_mesh_particles_render_native_geometry_and_sprites() {
    let Some(gpu) = common::gpu() else {
        return;
    };
    let dir = common::fixtures();
    std::fs::write(dir.join("particle.obj"), "v -0.005 -0.005 0\nv 0.005 -0.005 0\nv 0 0.005 0\nf 1 3 2\n").unwrap();
    for shape in ["billboard", "streak", "mesh"] {
        let extra = match shape {
            "mesh" => r#"mesh="rock" emitterShape="mesh" emitterMesh="rock""#,
            "billboard" => r#"sprite="sprite""#,
            _ => r#"velocityX="1" trail="20""#,
        };
        let xml = format!(
            r##"<scene version="1.3"><project width="32" height="32" fps="10" duration="2"/><assets><mesh id="rock" src="particle.obj"/><image id="sprite" src="red.png" width="4" height="4"/></assets><materials><material id="mat" baseColor="#FFFFFF" unlit="true" doubleSided="true"/></materials><composition><camera id="cam" projection="orthographic" x="16" y="16" z="-100"/><particles3D id="p" shape="{shape}" material="mat" {extra} x="16" y="16" rate="0" size="12"><burst time="0" count="1"/></particles3D></composition></scene>"##
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions { base_dir: Some(dir.clone()), verify_assets: true })
            .unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        let graph = ev.evaluate(0.);
        assert!(graph.problems.is_empty(), "{:?}", graph.problems);
        let f = renderer.render(&graph, ev.program());
        assert!(f.stats.errors.is_empty() && f.stats.unsupported.is_empty(), "{shape}: {:?}", f.stats);
        match shape {
            "mesh" => assert_eq!(f.stats.triangles, 1),
            "billboard" => assert_eq!(f.stats.triangles, 2),
            _ => assert!(f.stats.triangles > 2),
        }
        let pixels = renderer.read(&f.texture);
        assert!(
            pixels.iter().filter(|p| p[..3].iter().copied().fold(0f32, f32::max) > 0.5).count() > 15,
            "{shape}: no geometry; stats={:?}, particle={:?}, max={}",
            f.stats,
            graph.nodes.last().unwrap().particles3d,
            pixels.iter().flat_map(|p| &p[..3]).copied().fold(0f32, f32::max)
        );
        if shape == "billboard" {
            assert!(pixels.iter().any(|p| p[0] > 0.8 && p[1] < 0.1 && p[3] > 0.8));
        }
    }
}

#[test]
fn particles_use_age_curves_and_native_shutter_samples() {
    let xml = r##"<scene version="1.3"><project width="64" height="32" fps="10" duration="3" motionBlur="true" shutterAngle="360" shutterPhase="0" motionBlurSamples="4"/><materials><material id="mat" baseColor="#FFFFFF" unlit="true"/></materials><composition><camera id="cam" projection="orthographic" x="32" y="16" z="-100"/><particles3D id="p" x="10" y="16" shape="billboard" material="mat" rate="0" velocityX="100" size="8" sizeEnd="4" color="#FF0000" colorEnd="#0000FF" lifetime="2"><burst time="0" count="1"/></particles3D></composition></scene>"##;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let Some(blurred) = common::render_sub(&doc, 0.2) else {
        return;
    };
    assert!(blurred.stats.errors.is_empty() && blurred.stats.unsupported.is_empty(), "{:?}", blurred.stats);
    assert!(blurred.stats.subframes >= 4);
    let mut sharp_doc = doc.clone();
    sharp_doc.scene.project.motion_blur = false;
    let sharp = common::render_sub(&sharp_doc, 0.2).unwrap();
    assert_ne!(sharp.px, blurred.px);
    let occupied = |p: &[[f32; 4]]| p.iter().filter(|p| p[0] > 0.01).count();
    assert!(occupied(&blurred.px) > occupied(&sharp.px), "shutter must spread colored geometry across more pixels");
    assert!(sharp.px.iter().any(|p| p[0] > p[2] && p[2] > 0.05 && p[3] > 0.9), "age color must blend toward blue");
}

#[test]
fn particle_sprites_honor_the_image_transfer_declaration() {
    let Some(gpu) = common::gpu() else {
        return;
    };
    let mut values = Vec::new();
    for transfer in ["srgb", "linear"] {
        let xml = format!(
            r##"<scene version="1.3"><project width="16" height="16" fps="10" duration="1"/><assets><image id="sprite" src="gray.png" width="4" height="4" transfer="{transfer}" colorProfile="declared"/></assets><materials><material id="mat" unlit="true" baseColor="#FFFFFF"/></materials><composition><camera id="cam" projection="orthographic" x="8" y="8" z="-50"/><particles3D id="p" shape="billboard" material="mat" sprite="sprite" x="8" y="8" size="8" rate="0"><burst time="0" count="1"/></particles3D></composition></scene>"##
        );
        let doc = sr_model::load_str(
            &xml,
            &sr_model::LoadOptions { base_dir: Some(common::fixtures()), verify_assets: true },
        )
        .unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let mut r = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        let f = r.render(&ev.evaluate(0.), ev.program());
        assert!(f.stats.errors.is_empty(), "{:?}", f.stats);
        values.push(r.read(&f.texture)[8 * 16 + 8][0]);
    }
    assert!(values[1] > values[0] + 0.15, "declared linear input must be brighter: {values:?}");
}
