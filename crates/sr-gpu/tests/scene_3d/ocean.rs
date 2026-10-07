use super::common;
use common::*;

#[test]
fn native_ocean_renders_changes_inside_an_isolated_group_and_replays() {
    let xml = r##"<scene version="1.3"><project width="96" height="96" fps="10" duration="2" background="#101020"/><materials><material id="water" baseColor="#206FC0" roughness="0.25" metallic="0.2" doubleSided="true"/></materials><composition><camera id="cam" x="0" y="-9" z="-12" target="sea"/><group id="isolated" opacity="0.9"><ocean id="sea" width="8" depth="8" cellSize="0.5" bottomDepth="3" material="water" dt="0.05"><waterImpulse time="0.2" radius="3" amplitude="1"/></ocean></group></composition></scene>"##;
    let d = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let Some(gpu) = gpu() else { return };
    let mut renderer = sr_gpu::Renderer::new(gpu, ev.program());
    let mut snapshots = Vec::new();
    for t in [0.1, 0.5, 0.1] {
        let f = ev.evaluate(t);
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        let r = renderer.render(&f, ev.program());
        assert!(r.stats.errors.is_empty() && r.stats.unsupported.is_empty(), "{:?}", r.stats);
        assert_eq!(r.stats.triangles, 512);
        let px = renderer.read(&r.texture);
        assert!(px.iter().any(|c| c[2] > 0.08 && c[2] > c[0] * 2.0), "water is absent");
        snapshots.push(px);
    }
    assert_ne!(snapshots[0], snapshots[1]);
    assert_eq!(snapshots[0], snapshots[2]);
}

#[test]
fn ocean_default_material_matches_explicit_clear_water_optics() {
    let Some(gpu) = gpu() else { return };
    let mut pixels = Vec::new();
    for material in ["", r#"material="water""#] {
        let xml = format!(
            r##"<scene version="1.3"><project width="32" height="32" fps="10" duration="1" background="#123040"/><materials><material id="water" baseColor="#FFFFFF" roughness="0.05" transmission="1" ior="1.333" doubleSided="true"/><material id="floor" baseColor="#F03010" unlit="true" doubleSided="true"/></materials><composition><camera id="cam" x="0" y="-6" z="-8" target="sea"/><ocean id="sea" width="8" depth="8" bottomDepth="2" {material}/><object3D id="floor-plane" primitive="plane" width="8" height="8" y="1" rotationX="90" material="floor"/></composition></scene>"##
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        let out = renderer.render(&ev.evaluate(0.), ev.program());
        assert!(out.stats.errors.is_empty() && out.stats.unsupported.is_empty(), "{:?}", out.stats);
        pixels.push(renderer.read(&out.texture));
    }
    assert_eq!(pixels[0], pixels[1]);
}

#[test]
fn whitewater_batches_render_typed_materials_in_raster_and_pathtrace() {
    let Some(gpu) = gpu() else { return };
    for mode in ["", r#"renderer="pathtrace" pathSamples="2" maxBounces="1""#] {
        let xml = format!(
            r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="2" background="#101020"/><materials><material id="water" baseColor="#102040" unlit="true" doubleSided="true"/><material id="foam" baseColor="#10FF10" unlit="true" doubleSided="true"/><material id="spray" baseColor="#FF1010" unlit="true" doubleSided="true"/></materials><composition><camera id="cam" x="0" y="-6" z="-8" target="sea" {mode}/><ocean id="sea" width="8" depth="4" bottomDepth="2" initialVelocityX="2" boundary="periodic" dt="0.1" material="water"><whitewater emissionRate="30" threshold="0.1" radius="0.2" sprayFraction="0.5" foamMaterial="foam" sprayMaterial="spray"/></ocean></composition></scene>"##
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let frame = ev.evaluate(0.5);
        assert!(frame.problems.is_empty(), "{:?}", frame.problems);
        let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        let out = renderer.render(&frame, ev.program());
        assert!(out.stats.errors.is_empty() && out.stats.unsupported.is_empty(), "{:?}", out.stats);
        assert!(out.stats.triangles > 64, "whitewater was omitted: {:?}", out.stats);
        let pixels = renderer.read(&out.texture);
        assert!(pixels.iter().any(|p| p[0] > 0.3 && p[0] > p[1] * 3.), "red spray absent in {mode}");
        assert!(pixels.iter().any(|p| p[1] > 0.3 && p[1] > p[0] * 3.), "green foam absent in {mode}");
    }
}

#[test]
fn whitewater_foam_in_the_albedo_mode_draws_no_foam_triangles_and_renders_in_the_path_tracer() {
    let Some(gpu) = gpu() else { return };
    let mut shots = Vec::new();
    for mode in ["", r#"foamMode="albedo""#] {
        let xml = format!(
            r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="2" background="#101020"/><materials><material id="water" baseColor="#102040" roughness="0.3" doubleSided="true"/><material id="foam" baseColor="#10FF10" unlit="true" doubleSided="true"/><material id="spray" baseColor="#FF1010" unlit="true" doubleSided="true"/></materials><composition><camera id="cam" x="0" y="-6" z="-8" target="sea" renderer="pathtrace" pathSamples="4" maxBounces="2"/><ocean id="sea" width="8" depth="4" bottomDepth="2" initialVelocityX="2" boundary="periodic" dt="0.1" material="water"><whitewater emissionRate="30" threshold="0.1" radius="0.2" sprayFraction="0.5" foamMaterial="foam" sprayMaterial="spray" {mode}/></ocean></composition><lights><light id="sun" type="directional" intensity="3" yaw="45"/><light id="fill" type="ambient" intensity="1"/></lights></scene>"##
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let frame = ev.evaluate(0.5);
        assert!(frame.problems.is_empty(), "{:?}", frame.problems);
        let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        let out = renderer.render(&frame, ev.program());
        assert!(out.stats.errors.is_empty() && out.stats.unsupported.is_empty(), "{:?}", out.stats);
        shots.push((out.stats.triangles, renderer.read(&out.texture)));
    }
    let (particles, albedo) = (&shots[0], &shots[1]);
    assert!(albedo.0 < particles.0, "the foam is still drawn as triangles: {} against {}", albedo.0, particles.0);
    assert!(
        !albedo.1.iter().any(|p| p[1] > 0.3 && p[1] > p[0] * 3.),
        "the foam material is drawn although the foam is the water's own"
    );
    assert!(albedo.1.iter().any(|p| p[2] > 0.02), "the water is absent");
    assert_ne!(particles.1, albedo.1);
}

#[test]
fn whitewater_foam_in_the_albedo_mode_is_refused_by_the_raster_renderer_and_by_a_blended_water() {
    let Some(gpu) = gpu() else { return };
    // the mix is the path tracer's: the raster renderer says so instead of drawing no foam, and the water must be opaque
    for (camera, water, wants) in [
        ("", r##"baseColor="#102040""##, "path tracer"),
        (
            r#"renderer="pathtrace" pathSamples="2" maxBounces="1""#,
            r##"baseColor="#102040" alphaMode="blend""##,
            "opaque",
        ),
    ] {
        let xml = format!(
            r##"<scene version="1.3"><project width="32" height="32" fps="10" duration="2" background="#101020"/><materials><material id="water" {water} roughness="0.3" doubleSided="true"/></materials><composition><camera id="cam" x="0" y="-6" z="-8" target="sea" {camera}/><ocean id="sea" width="8" depth="4" bottomDepth="2" initialVelocityX="2" boundary="periodic" dt="0.1" material="water"><whitewater emissionRate="30" threshold="0.1" radius="0.2" foamMode="albedo"/></ocean></composition><lights><light id="sun" type="directional" intensity="3" yaw="45"/></lights></scene>"##
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let frame = ev.evaluate(0.5);
        assert!(frame.problems.is_empty(), "{:?}", frame.problems);
        let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        let out = renderer.render(&frame, ev.program());
        assert!(
            out.stats.errors.iter().any(|e| e.contains("foamMode") && e.contains(wants)),
            "{camera} / {water}: no error says why: {:?}",
            out.stats.errors
        );
    }
}
