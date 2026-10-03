//! Volume instances must resolve the selected primitive's asset and clock.

#[test]
fn volume_primitive_selects_volume_even_with_an_unused_mesh_reference() {
    let xml = r#"<scene version="1.3"><project width="16" height="16" fps="24" duration="3"/>
      <assets><mesh id="rock" src="rock.obj"/><volume id="smoke" src="smoke.srvol"/></assets>
      <composition><object3D id="cloud" primitive="volume" volume="smoke" mesh="rock" start="1"/></composition></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let evaluator = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = evaluator.evaluate(1.5);
    let node = frame.nodes.iter().find(|node| &*node.id == "cloud").unwrap();
    assert_eq!(node.asset.as_deref(), Some("smoke"));
    assert_eq!(node.local_time, 0.5);
}
