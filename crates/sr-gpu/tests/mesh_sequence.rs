mod common;
use common::*;

#[test]
fn topology_changing_mesh_cache_renders_and_replays_in_both_renderers() {
    let path = std::env::temp_dir().join(format!(
        "sr-gpu-mesh-cache-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir(&path).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(path.clone());
    std::fs::write(path.join("frame-0.obj"), "v -0.3 -0.3 0\nv 0.3 -0.3 0\nv 0 0.3 0\nf 1 2 3\n").unwrap();
    std::fs::write(
        path.join("frame-1.obj"),
        "v -0.3 -0.3 0\nv 0.3 -0.3 0\nv 0.3 0.3 0\nv -0.3 0.3 0\nf 1 2 3\nf 1 3 4\n",
    )
    .unwrap();
    let Some(gpu) = gpu() else { return };
    for mode in ["raster", "pathtrace"] {
        let xml = format!(
            r##"<scene version="1.3"><project width="128" height="128" fps="10" duration="2" background="#000000"/><assets><meshSequence id="frames" src="frame-%d.obj" first="0" last="1" fps="1"/></assets><materials><material id="white" baseColor="#FFFFFF" unlit="true" doubleSided="true"/></materials><composition><camera id="cam" x="64" y="64" z="-200" projection="orthographic" orthoHeight="128" renderer="{mode}" pathSamples="1" maxBounces="1" denoise="false"/><group id="isolate" opacity="0.8"><object3D id="cache" primitive="mesh" mesh="frames" material="white" x="64" y="64"/></group></composition></scene>"##
        );
        let doc =
            sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(path.clone()) })
                .unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        let mut images = Vec::new();
        for time in [0., 1., 0.] {
            let out = renderer.render(&ev.evaluate(time), ev.program());
            assert!(out.stats.errors.is_empty() && out.stats.unsupported.is_empty(), "{:?}", out.stats);
            images.push(renderer.read(&out.texture));
        }
        let lit = |img: &Vec<[f32; 4]>| img.iter().filter(|v| v[0] > 0.3).count();
        assert!(lit(&images[1]) > lit(&images[0]) * 3 / 2, "{mode}: {} vs {}", lit(&images[0]), lit(&images[1]));
        assert_eq!(images[0], images[2]);
    }
}
