use super::common;
use common::*;

#[test]
fn crater_excavation_opens_a_native_smoke_emission_region() {
    let Some(gpu) = gpu() else { return };
    for mode in ["raster", "pathtrace"] {
        let xml = format!(
            r##"<scene version="1.3"><project width="4" height="4" fps="10" duration="1" background="#00000000"/>
        <composition><camera id="camera" projection="orthographic" x="2" y="2" z="-20" renderer="{mode}" pathSamples="1" maxBounces="1" denoise="false"/>
        <object3D id="solid" primitive="plane" x="2" y="2" width="8" height="8" segments="16" visible="false">
        <crater radius="4" depth="2" rimHeight="0" rimWidth="1" start="0.1" end="0.3" curve="linear"/></object3D>
        <group id="isolated" isolate="true"><object3D id="cloud" primitive="volume" x="2" y="2">
        <pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" boundary="open" pressureIterations="500" colliders="solid" colliderThickness="2">
        <pyroSource shape="box" width="8" height="8" depth="2" densityRate="10"/></pyro>
        <medium extinction="0.1" emissionColor="#FF4000" emissionScale="1" albedo="#000000"/></object3D></group>
        </composition></scene>"##
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        let mut images = Vec::new();
        for t in [0.1, 0.5, 0.1] {
            let graph = ev.evaluate(t);
            assert!(graph.problems.is_empty(), "{:?}", graph.problems);
            let frame = renderer.render(&graph, ev.program());
            assert!(frame.stats.errors.is_empty() && frame.stats.unsupported.is_empty(), "{:?}", frame.stats);
            images.push(renderer.read(&frame.texture));
        }
        assert!(images[0].iter().all(|p| p[3] < 1e-6), "{mode}: initial slab excludes the source");
        assert!(images[1].iter().any(|p| p[3] > 0.01 && p[0] > 0.01), "{mode}: crater permits visible smoke");
        assert_eq!(images[0], images[2], "{mode}: backward replay");
    }
}

#[test]
fn native_crater_deforms_both_renderers_and_replays_inside_isolated_groups() {
    let Some(gpu) = gpu() else { return };
    for mode in ["raster", "pathtrace"] {
        let scene = |crater: &str| {
            format!(
                r##"<scene version="1.3"><project width="128" height="96" fps="10" duration="2" background="#000000"/><materials><material id="clay" baseColor="#FFFFFF" roughness="1" doubleSided="true"/></materials><composition><camera id="cam" x="64" y="48" z="-200" projection="orthographic" orthoHeight="96" renderer="{mode}" pathSamples="1" maxBounces="1" denoise="false"/><group id="isolate" opacity="0.8"><object3D id="ground" primitive="plane" width="100" height="70" segments="40" material="clay" x="64" y="48">{crater}</object3D></group></composition><lights><light id="ambient" type="ambient" intensity="0.1"/><light id="sun" type="directional" yaw="35" pitch="-35" intensity="2"/></lights></scene>"##
            )
        };
        let crater = r#"<crater radius="24" depth="14" rimWidth="6" rimHeight="4" end="1" curve="linear"/>"#;
        let mut images = Vec::new();
        for body in ["", crater] {
            let doc = sr_model::load_str(&scene(body), &sr_model::LoadOptions::without_assets()).unwrap();
            let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
            let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
            let mut frames = Vec::new();
            for t in [0., 1., 0.] {
                let out = renderer.render(&ev.evaluate(t), ev.program());
                assert!(out.stats.errors.is_empty() && out.stats.unsupported.is_empty(), "{:?}", out.stats);
                frames.push(renderer.read(&out.texture));
            }
            assert_eq!(frames[0], frames[2], "{mode}: backward seek");
            images.push(frames);
        }
        assert_eq!(images[0][0], images[1][0], "{mode}: crater has not started");
        let changed = images[0][1].iter().zip(&images[1][1]).filter(|(a, b)| (a[0] - b[0]).abs() > 0.05).count();
        assert!(changed > 100, "{mode}: crater changed only {changed} pixels");
    }
}
