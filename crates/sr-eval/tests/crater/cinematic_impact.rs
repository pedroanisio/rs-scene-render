use sr_eval::{Evaluator, FrameGraph, FrameNode};

fn node<'a>(frame: &'a FrameGraph, id: &str) -> &'a FrameNode {
    frame.nodes.iter().find(|node| &*node.id == id).unwrap()
}

#[test]
fn shipped_impact_scene_runs_combined_systems_and_replays_after_late_seek() {
    let xml = include_str!("../../../../examples/cinematic-impact/impact.scene.xml");
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let evaluator = Evaluator::new(&doc, &Default::default()).unwrap();
    let sample = |time| {
        let frame = evaluator.evaluate(time);
        assert!(frame.problems.is_empty(), "{time}: {:?}", frame.problems);
        frame
    };
    let before = sample(0.5);
    assert!(node(&before, "ejecta").particles3d.as_ref().unwrap().frame.particles.is_empty());
    let crater = sr_eval::crater::at(node(&before, "seabed")).unwrap().unwrap();
    assert_eq!(crater.progress, 0.);
    let after = sample(1.5);
    assert_eq!(node(&after, "ejecta").particles3d.as_ref().unwrap().frame.particles.len(), 1500);
    let plume = node(&after, "plume").sim_volume.as_ref().unwrap();
    assert!(plume.data.grid("density").unwrap().bricks().any(|(_, b)| b.iter().any(|&v| v > 0.)));
    assert!(plume.data.grid("temperature").unwrap().bricks().any(|(_, b)| b.iter().any(|&v| v > 1000.)));
    assert_ne!(
        node(&before, "sea").sim_ocean.as_ref().unwrap().frame,
        node(&after, "sea").sim_ocean.as_ref().unwrap().frame
    );
    let late = sample(4.5);
    let crater = sr_eval::crater::at(node(&late, "seabed")).unwrap().unwrap();
    assert_eq!(crater.progress, 1.);
    assert!((crater.kernel.map([0.; 3], 1.).unwrap().position[2] - 25.).abs() < 1e-9);
    sample(0.5);
    let replay = sample(1.5);
    assert_eq!(
        node(&replay, "sea").sim_ocean.as_ref().unwrap().frame,
        node(&after, "sea").sim_ocean.as_ref().unwrap().frame
    );
    assert_eq!(
        node(&replay, "ejecta").particles3d.as_ref().unwrap().frame,
        node(&after, "ejecta").particles3d.as_ref().unwrap().frame
    );
    assert_eq!(node(&replay, "plume").sim_volume.as_ref().unwrap().key, plume.key);
}
