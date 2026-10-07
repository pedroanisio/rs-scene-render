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
fn whitewater_foam_in_the_albedo_mode_draws_no_foam_triangles_and_whitens_the_water_by_the_documents_albedo() {
    let Some(gpu) = gpu() else { return };
    // the particles mode, the albedo mode, and the albedo mode with a foam that is black
    let mut shots = Vec::new();
    for mode in ["", r#"foamMode="albedo""#, r#"foamMode="albedo" foamAlbedo="0""#] {
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
    let green = |px: &[[f32; 4]]| px.iter().filter(|p| p[1] > 0.3 && p[1] > p[0] * 3.).count();
    // light and grey: the foam's white under the ambient light and the sun (the water is dark blue, the spray red)
    let white = |px: &[[f32; 4]]| px.iter().filter(|p| p[0] > 0.45 && p[2] < p[0] * 1.2 && p[1] > p[0] * 0.8).count();
    let (particles, albedo, black) = (&shots[0], &shots[1], &shots[2]);
    println!(
        "green {} / {} / {}, white {} / {} / {}",
        green(&particles.1),
        green(&albedo.1),
        green(&black.1),
        white(&particles.1),
        white(&albedo.1),
        white(&black.1)
    );
    assert!(albedo.0 < particles.0, "the foam is still drawn as triangles: {} against {}", albedo.0, particles.0);
    assert!(green(&particles.1) > 0, "the green of the foam material is visible in the particles mode");
    assert_eq!(green(&albedo.1), 0, "the foam material is drawn although the foam is the water's own");
    assert!(
        white(&albedo.1) > white(&black.1) + 20,
        "foam of albedo 0.9 whitens the water, one of albedo 0 does not: {} against {}",
        white(&albedo.1),
        white(&black.1)
    );
}

#[test]
fn whitewater_foam_in_the_albedo_mode_is_refused_for_a_raster_camera_and_for_water_that_is_not_opaque_lit_and_dark() {
    let Some(gpu) = gpu() else { return };
    // the mix is the path tracer's, of an opaque lit water that does not shine by itself: each of the others is an error that names
    // the attribute and what is wrong, and the water is not drawn (a raster camera discards the whole 3D pass, as its error says)
    let path = r#"renderer="pathtrace" pathSamples="2" maxBounces="1""#;
    let lit = r##"baseColor="#102040""##;
    let cases = [
        ("", lit.to_string(), "path tracer"),
        (path, format!(r#"{lit} alphaMode="blend""#), "opaque"),
        (path, format!(r#"{lit} alphaMode="mask""#), "opaque"),
        (path, format!(r#"{lit} unlit="true""#), "lit"),
        (path, format!(r##"{lit} emissive="#FFFFFF" emissiveStrength="2""##), "emission"),
    ];
    // the background #101020 as a linear colour
    let linear = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    let expected = [linear(16.0 / 255.0), linear(16.0 / 255.0), linear(32.0 / 255.0)];
    for (camera, water, wants) in cases {
        let xml = format!(
            r##"<scene version="1.3"><project width="32" height="32" fps="10" duration="2" background="#101020"/><materials><material id="water" {water} roughness="0.3" doubleSided="true"/></materials><composition><camera id="cam" x="0" y="-6" z="-8" target="sea" {camera}/><ocean id="sea" width="8" depth="4" bottomDepth="2" initialVelocityX="2" boundary="periodic" dt="0.1" material="water"><whitewater emissionRate="30" threshold="0.1" radius="0.2" sprayFraction="0" foamMode="albedo"/></ocean></composition><lights><light id="sun" type="directional" intensity="3" yaw="45"/></lights></scene>"##
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
        // and nothing of the water is drawn: every pixel is the background, and the background is not black
        let frame = renderer.read(&out.texture);
        for p in &frame {
            for k in 0..3 {
                assert!(
                    (p[k] - expected[k]).abs() < 2e-3,
                    "{camera} / {water}: a pixel {p:?} is not the background {expected:?}"
                );
            }
        }
    }
}

/// The picture of a sea with foam in the albedo mode, seen from `y` over it by a path tracer, whose whitewater has `foam` besides
/// `foamMode="albedo"`: the sum of the luminance of its pixels.
fn foamy_sea(gpu: &sr_gpu::Gpu, y: f32, foam: &str) -> Vec<[f32; 4]> {
    foamy_sea_of(gpu, y, foam, r##"baseColor="#102040" roughness="0.3""##)
}

/// The same sea of a water material `water`.
fn foamy_sea_of(gpu: &sr_gpu::Gpu, y: f32, foam: &str, water: &str) -> Vec<[f32; 4]> {
    let xml = format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="2" background="#101020"/><materials><material id="water" {water} doubleSided="true"/></materials><composition><camera id="cam" x="0" y="{y}" z="-8" target="sea" renderer="pathtrace" pathSamples="16" maxBounces="2"/><ocean id="sea" width="8" depth="4" bottomDepth="2" initialVelocityX="2" boundary="periodic" dt="0.1" material="water"><whitewater emissionRate="30" threshold="0.1" radius="0.2" sprayFraction="0" foamMode="albedo" {foam}/></ocean></composition><lights><light id="sun" type="directional" intensity="3" yaw="45"/><light id="fill" type="ambient" intensity="1"/></lights></scene>"##
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(0.5);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
    let out = renderer.render(&frame, ev.program());
    assert!(out.stats.errors.is_empty() && out.stats.unsupported.is_empty(), "{:?}", out.stats);
    renderer.read(&out.texture)
}

fn light(px: &[[f32; 4]]) -> f64 {
    px.iter().map(|p| f64::from(0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])).sum()
}

#[test]
fn the_documents_foam_albedo_and_roughness_reach_the_water_with_their_values_and_their_defaults() {
    let Some(gpu) = gpu() else { return };
    // the magnitude of the albedo: what the foam adds to a sea is in proportion to the albedo (a diffuse surface under the
    // ambient light and the sun), so half the albedo adds half
    let (black, half, full) = (
        light(&foamy_sea(&gpu, -6.0, r#"foamAlbedo="0""#)),
        light(&foamy_sea(&gpu, -6.0, r#"foamAlbedo="0.45""#)),
        light(&foamy_sea(&gpu, -6.0, "")),
    );
    println!("albedo 0: {black:.2}, 0.45: {half:.2}, default: {full:.2}");
    assert!(full > black + 5.0, "the foam brightens the sea: {full} against {black}");
    let ratio = (half - black) / (full - black);
    assert!((ratio - 0.5).abs() <= 0.08, "half the albedo adds half the light: {ratio}");
    // the defaults are the documented ones, bit for bit
    let explicit = foamy_sea(&gpu, -6.0, r#"foamAlbedo="0.9" foamRoughness="0.8""#);
    assert!(explicit == foamy_sea(&gpu, -6.0, ""), "the defaults are 0.9 and 0.8");
    // the roughness: from 10 degrees over the water a smooth foam has a brighter lobe than a rough one
    let (smooth, rough) = (
        light(&foamy_sea(&gpu, -1.2, r#"foamRoughness="0.1""#)),
        light(&foamy_sea(&gpu, -1.2, r#"foamRoughness="0.9""#)),
    );
    println!("grazing: roughness 0.1: {smooth:.2}, 0.9: {rough:.2}");
    assert!(smooth > 1.2 * rough, "{smooth} against {rough}");
    // and the default is the value of 0.8, not another
    let default = foamy_sea(&gpu, -1.2, "");
    assert!(default == foamy_sea(&gpu, -1.2, r#"foamRoughness="0.8""#));
    assert!(default != foamy_sea(&gpu, -1.2, r#"foamRoughness="0.2""#), "the roughness reaches the shading");
}

#[test]
fn the_documents_foam_reaches_a_transmissive_water_through_the_water_variant_of_the_shader() {
    let Some(gpu) = gpu() else { return };
    // a water that lets the light through is the shader with the refracted shadow rays and the foam together (the variants of the
    // pipelines that no document with an opaque water reaches): foam of albedo 0, 0.45 and 0.9 adds light in proportion to the albedo
    let clear = r##"baseColor="#102040" roughness="0.05" transmission="1" ior="1.333""##;
    let level = |foam: &str| light(&foamy_sea_of(&gpu, -6.0, foam, clear));
    let (black, half, full) = (level(r#"foamAlbedo="0""#), level(r#"foamAlbedo="0.45""#), level(""));
    println!("transmissive: albedo 0: {black:.2}, 0.45: {half:.2}, default: {full:.2}");
    assert!(full > black + 5.0, "the foam brightens the sea: {full} against {black}");
    let ratio = (half - black) / (full - black);
    assert!((ratio - 0.5).abs() <= 0.1, "half the albedo adds half the light: {ratio}");
    // and a foam of no share (no tracers: births that start after the frame) is the picture of the water with no foam mode, bit for bit
    let none = |foam: &str| foamy_sea_of(&gpu, -6.0, foam, clear);
    assert!(
        none(r#"start="100""#) == none(r#"start="100" foamAlbedo="0.5" foamRoughness="0.2""#),
        "no foam, no change"
    );
}
