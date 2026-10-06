//! A smoke that reads the rigid world fails when the rigid world cannot answer: a step computed from a world that
//! said "unstable" is not kept as if the world had stood still.

use sr_eval::Evaluator;

/// A box too light for the step in an ocean that loads it (the rigid world answers with a problem from its first
/// step), and a smoke that collides with it.
const SCENE: &str = r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="4"/><composition>
  <object3D id="float" primitive="box" width="2" height="1" depth="2" y="-0.2">
    <rigidBody shape="box" mass="0.1" linearDamping="0" angularDamping="0"/>
  </object3D>
  <ocean id="sea" bedResponse="hydrostatic" width="16" depth="16" cellSize="1" bottomDepth="20" dt="0.05" boundary="closed" colliders="float" bodyCoupling="buoyancy"/>
  <object3D id="cloud" primitive="volume" y="-6">
    <pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" boundary="open" colliders="float">
      <pyroSource shape="box" width="2" height="2" depth="2" densityRate="1"/>
    </pyro>
  </object3D>
</composition>
<physics gravityY="-9.80665" pixelsPerMeter="1" fixedStep="0.008333333333333333" bounds="none"/></scene>"##;

#[test]
fn a_smoke_whose_rigid_world_answered_with_a_problem_is_not_kept() {
    let doc = sr_model::load_str(SCENE, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    let ev = Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(1.0);
    let said = frame.problems.iter().chain(&frame.failures).any(|m| m.contains("omega * dt"));
    assert!(said, "the rigid world's problem is reported: {:?} {:?}", frame.problems, frame.failures);
    let cloud = frame.nodes.iter().find(|n| &*n.id == "cloud").unwrap();
    assert!(
        frame.failures.iter().any(|m| m.starts_with("cloud")) && cloud.sim_volume.is_none(),
        "the smoke fails with the world: {:?} {:?}",
        frame.failures,
        cloud.sim_volume.is_some()
    );
}
