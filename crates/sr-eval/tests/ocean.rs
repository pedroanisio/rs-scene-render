fn evaluator(nodes: &str) -> sr_eval::Evaluator {
    let xml = format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="10" duration="4"/><composition>{nodes}</composition></scene>"#
    );
    let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    sr_eval::Evaluator::new(&d, &Default::default()).unwrap()
}
#[test]
fn ocean_impulse_follows_nested_clock_and_replays_identically() {
    let ev = evaluator(
        r#"<group id="clock" start="0.5" timeScale="2"><ocean id="sea" start="1" width="8" depth="8" cellSize="0.5" bottomDepth="2" dt="0.05"><waterImpulse time="0.2" radius="2" amplitude="0.3"/></ocean></group>"#,
    );
    assert!(ev.has_simulation());
    let get = |t| {
        let f = ev.evaluate(t);
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        f.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap().clone()
    };
    let before = get(0.8);
    assert!(before.frame.cells.iter().all(|c| c.depth == 2.0));
    let after = get(1.0);
    assert!(after.frame.cells.iter().any(|c| (c.depth - 2.0).abs() > 0.01));
    assert!((after.frame.cells.iter().map(|c| c.depth).sum::<f64>() - 512.0).abs() < 1e-9);
    get(0.8);
    assert_eq!(get(1.0).frame, after.frame);
}
#[test]
fn ocean_mesh_has_upward_normals_finite_tangents_and_omits_dry_cells() {
    let ev = evaluator(r#"<ocean id="sea" width="4" depth="6" bottomDepth="2"/>"#);
    let f = ev.evaluate(0.0);
    assert!(f.problems.is_empty(), "{:?}", f.problems);
    let s = f.nodes[0].sim_ocean.as_ref().unwrap();
    assert_eq!(s.mesh.indices.len(), 4 * 6 * 6);
    assert!(s
        .mesh
        .vertices
        .iter()
        .all(|v| v.normal[1] < -0.99 && v.pos[1] == 0.0 && v.tangent.iter().all(|x| x.is_finite())));
    let dry = evaluator(r#"<ocean id="sea" width="4" depth="6" bottomDepth="0"/>"#).evaluate(0.0);
    assert!(dry.nodes[0].sim_ocean.as_ref().unwrap().mesh.indices.is_empty());
}

#[test]
fn bathymetry_images_are_numeric_data_and_meshes_sample_the_top_surface() {
    let dir = std::env::temp_dir().join(format!(
        "sr-ocean-bed-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    image::RgbImage::from_fn(2, 1, |x, _| image::Rgb([if x == 0 { 128 } else { 255 }, 0, 0]))
        .save(dir.join("bed.png"))
        .unwrap();
    std::fs::write(
        dir.join("bed.obj"),
        "v -0.01 -0.02 -0.01\nv 0.01 -0.02 -0.01\nv -0.01 -0.02 0.01\nv 0.01 -0.02 0.01\nf 1 2 3\nf 2 4 3\n",
    )
    .unwrap();
    for (asset, opts, expected) in [
        (
            r#"<image id="bed" src="bed.png" width="2" height="1" transfer="srgb"/>"#,
            r#"bathymetryScale="2" bathymetryOffset="1""#,
            vec![1.0 + 256.0 / 255.0, 3.0, 1.0 + 256.0 / 255.0, 3.0],
        ),
        (r#"<mesh id="bed" src="bed.obj"/>"#, r#"bathymetryOffset="0.5""#, vec![2.5; 4]),
    ] {
        let xml = format!(
            r#"<scene version="1.3"><project width="64" height="64" fps="10" duration="1"/><assets>{asset}</assets><composition><ocean id="sea" width="2" depth="2" bathymetry="bed" {opts}/></composition></scene>"#
        );
        let d = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir.clone()) })
            .unwrap();
        let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
        let f = ev.evaluate(0.2);
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        let sea = f.nodes[0].sim_ocean.as_ref().unwrap();
        for (c, h) in sea.frame.cells.iter().zip(expected) {
            assert!((c.depth - h).abs() < 1e-6, "{c:?}, expected {h}");
        }
    }
}

#[test]
fn seeded_waves_preserve_all_seed_bits_and_explicit_phase_is_stable() {
    let sample = |seed, phase: &str| {
        let ev = evaluator(&format!(
            r#"<ocean id="sea" width="8" depth="4" cellSize="0.5" bottomDepth="2" seed="{seed}"><wave wavelength="4" amplitude="0.1" {phase}/></ocean>"#
        ));
        let f = ev.evaluate(0.4);
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        f.nodes[0].sim_ocean.as_ref().unwrap().frame.clone()
    };
    let a = sample(9_007_199_254_740_992u64, "");
    let b = sample(9_007_199_254_740_993u64, "");
    assert_ne!(a, b);
    assert_eq!(sample(1, r#"phase="0""#), sample(2, r#"phase="0""#));
}

#[test]
fn surface_budgets_report_errors_and_visibility_does_not_reset_the_water() {
    let limited = evaluator(r#"<ocean id="sea" width="128" depth="128" surfaceMemoryMiB="1"/>"#).evaluate(0.);
    assert!(limited.problems.iter().any(|s| s.contains("surface") && s.contains("memory")), "{:?}", limited.problems);
    assert!(limited.nodes[0].sim_ocean.is_none());
    let scene = r#"<ocean id="sea" width="8" depth="8" condition="time &gt;= 0.5"><waterImpulse time="0.2" radius="2" amplitude="0.2"/></ocean>"#;
    let conditional = evaluator(scene);
    let plain = evaluator(&scene.replace(r#" condition="time &gt;= 0.5""#, ""));
    assert!(conditional.evaluate(0.1).nodes.is_empty());
    let a = conditional.evaluate(0.6);
    let b = plain.evaluate(0.6);
    assert!(a.problems.is_empty() && b.problems.is_empty());
    assert_eq!(a.nodes[0].sim_ocean.as_ref().unwrap().frame, b.nodes[0].sim_ocean.as_ref().unwrap().frame);
}

#[test]
fn whitewater_is_clocked_replayable_batched_geometry_without_changing_water() {
    let nodes = r#"<group id="g" start="0.2" timeScale="2"><ocean id="sea" width="8" depth="4" bottomDepth="2" dt="0.1" initialVelocityX="2" boundary="periodic"><whitewater emissionRate="20" threshold="0.1" sprayFraction="0.5" radius="0.1" seed="9007199254740993"/></ocean></group>"#;
    let ev = evaluator(nodes);
    let plain = evaluator(&nodes.replace(
        r#"<whitewater emissionRate="20" threshold="0.1" sprayFraction="0.5" radius="0.1" seed="9007199254740993"/>"#,
        "",
    ));
    let f = ev.evaluate(0.4);
    assert!(f.problems.is_empty(), "{:?}", f.problems);
    let sea = f.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap();
    let white = sea.whitewater.as_ref().unwrap();
    // Group clocks preserve their start anchor: .2 + (.4-.2)*2 = .6.
    assert!((white.time - 0.6).abs() < 1e-12);
    assert!(!white.particles.is_empty());
    assert!(sea.whitewater_mesh.iter().all(|m| !m.indices.is_empty()));
    assert!(sea.whitewater_mesh.iter().flat_map(|m| &m.vertices).all(|v| v
        .pos
        .iter()
        .chain(&v.normal)
        .chain(&v.tangent)
        .all(|v| v.is_finite())));
    let f_plain = plain.evaluate(0.4);
    assert_eq!(sea.frame, f_plain.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap().frame);
    ev.evaluate(0.25);
    let again = ev.evaluate(0.4);
    let sea_again = again.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap();
    assert_eq!(sea_again.whitewater, sea.whitewater);
    assert_eq!(sea_again.key, sea.key);
    let other_seed = evaluator(&nodes.replace("9007199254740993", "9007199254740992")).evaluate(0.4);
    let other = other_seed.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap();
    assert_ne!(sea.whitewater, other.whitewater, "adjacent full-u64 seeds must not round together");
}

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "sr-ocean-data-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        Self(dir)
    }
    fn evaluate(&self, asset: &str, attrs: &str) -> sr_eval::FrameGraph {
        let xml = format!(
            r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="1"/><assets>{asset}</assets><composition><ocean id="sea" width="2" depth="2" bathymetry="bed" {attrs}/></composition></scene>"#
        );
        let d =
            sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(self.0.clone()) })
                .unwrap();
        sr_eval::Evaluator::new(&d, &Default::default()).unwrap().evaluate(0.)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn packed_bathymetry_converts_upward_elevation_before_scale_and_offset() {
    let dir = Fixture::new();
    for (encoding, rgb) in [("terrarium", [127, 253, 128]), ("mapbox", [1, 134, 135])] {
        image::RgbImage::from_pixel(1, 1, image::Rgb(rgb)).save(dir.0.join("bed.png")).unwrap();
        let f = dir.evaluate(
            r#"<image id="bed" src="bed.png" width="1" height="1"/>"#,
            &format!(r#"bathymetryEncoding="{encoding}" bathymetryScale="2" bathymetryOffset="0.5" waterLevel="1""#),
        );
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        assert!(f.nodes[0].sim_ocean.as_ref().unwrap().frame.cells.iter().all(|c| (c.depth - 4.5).abs() < 1e-9));
    }
}

#[test]
fn bathymetry_rejects_large_decoded_rasters_and_uncovered_meshes() {
    let dir = Fixture::new();
    image::RgbImage::from_pixel(512, 512, image::Rgb([128, 0, 0])).save(dir.0.join("bed.png")).unwrap();
    let f = dir.evaluate(r#"<image id="bed" src="bed.png" width="512" height="512"/>"#, r#"meshMemoryMiB="1""#);
    assert!(f.problems.iter().any(|e| e.contains("decoded image") && e.contains("budget")), "{:?}", f.problems);
    std::fs::write(dir.0.join("bed.obj"), "v 0 -0.02 0\nv 0.001 -0.02 0\nv 0 -0.02 0.001\nf 1 2 3\n").unwrap();
    let f = dir.evaluate(r#"<mesh id="bed" src="bed.obj"/>"#, "");
    assert!(f.problems.iter().any(|e| e.contains("does not cover")), "{:?}", f.problems);
}

#[test]
fn order_selects_the_solver_and_changes_the_published_frame_and_its_cache_key() {
    let at_one_second = |order: &str| {
        let ev = evaluator(&format!(
            r#"<ocean id="sea" width="16" depth="16" cellSize="0.5" bottomDepth="2" dt="0.05" {order}><waterImpulse time="0.2" radius="2" amplitude="0.3"/></ocean>"#
        ));
        let f = ev.evaluate(1.0);
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        f.nodes.iter().find(|n| &*n.id == "sea").unwrap().sim_ocean.as_ref().unwrap().clone()
    };
    let (absent, first, second) = (at_one_second(""), at_one_second(r#"order="1""#), at_one_second(r#"order="2""#));
    assert_eq!((&absent.frame, absent.key), (&first.frame, first.key));
    assert_ne!(first.frame, second.frame);
    assert_ne!(first.key, second.key);
    // Both conserve water; only the numerical scheme differs.
    for s in [&first, &second] {
        assert!((s.frame.cells.iter().map(|c| c.depth).sum::<f64>() - 32.0 * 32.0 * 2.0).abs() < 1e-9);
    }
}

/// A cold seek to 20 s on 64 x 64 first-order cells with a 0.0005 s step costs
/// 40000 steps x 32768 units = 1.31e9 units, more than the old ceiling of 1e9. It
/// runs about ten seconds in release, so it runs on request:
/// `cargo test --release -p sr-eval --test ocean -- --ignored`.
#[test]
#[ignore = "about ten seconds of solver work in release"]
fn work_allowance_above_one_billion_reaches_the_solver_without_loss() {
    let sea = |work: u64| {
        let xml = format!(
            r#"<scene version="1.3"><project width="64" height="64" fps="10" duration="21"/><composition><ocean id="sea" width="64" depth="64" bottomDepth="1" dt="0.0005" maxWork="{work}"><waterImpulse time="0.1" radius="4" amplitude="0.2"/></ocean></composition></scene>"#
        );
        let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        sr_eval::Evaluator::new(&d, &Default::default()).unwrap().evaluate(20.0)
    };
    let frame = sea(1_000_000_000_000);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    assert!(frame.nodes[0].sim_ocean.is_some());
}
