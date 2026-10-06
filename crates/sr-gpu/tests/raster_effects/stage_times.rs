use super::common;

/// `RenderStats` reports where a frame's time went: the simulations that advanced it, the CPU
/// work that prepared its 3D draws and, on the path tracer, scene assembly, BVH construction
/// and buffer packing. Each figure is measured, not derived, so a stage that ran is positive
/// and one the scene does not have costs only the bookkeeping of finding nothing to do.
#[test]
fn stage_times_cover_simulation_draw_preparation_and_path_tracer_build() {
    let Some(gpu) = common::gpu() else { return };
    let xml = r##"<scene version="1.3"><project width="16" height="16" fps="10" duration="2" background="#00000000"/>
        <materials><material id="red" baseColor="#FF0000" unlit="true"/></materials>
        <composition><camera id="camera" projection="orthographic" x="2" y="2" z="-20" renderer="pathtrace" pathSamples="1" maxBounces="1" denoise="false"/>
        <object3D id="ball" primitive="sphere" radius="1" x="2" y="2" material="red"/>
        <particles3D id="dust" material="red" rate="0" size="0.2" x="2" y="2" z="1" velocityX="1" lifetime="2" dt="0.1"><burst time="0" count="3"/></particles3D>
        <group id="isolated" isolate="true"><object3D id="cloud" primitive="volume" x="2" y="2">
        <pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" boundary="open" pressureIterations="20">
        <pyroSource shape="box" width="4" height="4" depth="2" densityRate="10"/></pyro>
        <medium extinction="0.1" emissionColor="#FF4000" emissionScale="1" albedo="#000000"/></object3D></group>
        </composition></scene>"##;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let mut renderer = sr_gpu::Renderer::new(gpu, ev.program());
    let graph = ev.evaluate(0.5);
    assert!(graph.problems.is_empty(), "{:?}", graph.problems);
    let stats = renderer.render(&graph, ev.program()).stats;
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    for (name, seconds) in [
        ("sim_smoke_seconds", stats.sim_smoke_seconds),
        ("sim_particles_seconds", stats.sim_particles_seconds),
        ("draw_prep_seconds", stats.draw_prep_seconds),
        ("volume_prep_seconds", stats.volume_prep_seconds),
        ("pt_assemble_seconds", stats.pt_assemble_seconds),
        ("pt_bvh_seconds", stats.pt_bvh_seconds),
        ("pt_pack_seconds", stats.pt_pack_seconds),
    ] {
        assert!(seconds > 0.0 && seconds.is_finite(), "{name} = {seconds}");
    }
    // the scene has no ocean and no rigid body: those stages cost nothing
    assert!(stats.sim_ocean_seconds < 1e-3, "no ocean exists: {}", stats.sim_ocean_seconds);
    assert!(stats.sim_rigid_seconds < 1e-3, "no rigid world exists: {}", stats.sim_rigid_seconds);
    // the statistics serialise with the new fields, as `render --stats` prints them
    let json = serde_json::to_value(&stats).unwrap();
    for key in
        ["sim_rigid_seconds", "sim_ocean_seconds", "sim_smoke_seconds", "sim_particles_seconds", "pt_bvh_seconds"]
    {
        assert!(json.get(key).is_some_and(|v| v.is_number()), "{key} missing from the statistics JSON");
    }
}
