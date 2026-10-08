//! Colliders named by an owner inside a symbol resolve in the scope of their own
//! instance: two instances of one symbol each see the collider of their own copy,
//! here told apart by an override that moves or shrinks the collider in one of them.
use sr_eval::{Evaluator, FrameGraph};

fn evaluator(inner: &str, override_b: &str) -> Evaluator {
    let xml = format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="10" duration="4"/><symbols><symbol id="s" width="64" height="64" duration="4">{inner}</symbol></symbols><composition>
          <instance id="a" symbol="s"/><instance id="b" symbol="s">{override_b}</instance></composition></scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    Evaluator::new(&doc, &Default::default()).unwrap()
}
fn node<'a>(frame: &'a FrameGraph, id: &str) -> &'a sr_eval::FrameNode {
    frame.nodes.iter().find(|n| &*n.id == id).unwrap_or_else(|| panic!("no node {id}"))
}

#[test]
fn smoke_colliders_in_a_symbol_belong_to_their_instance() {
    let ev = evaluator(
        r#"<object3D id="solid" primitive="sphere" radius="2"/>
           <object3D id="cloud" primitive="volume"><pyro width="8" height="8" depth="8" voxelSize="1" dt="0.1" boundary="open" colliders="solid">
             <pyroSource shape="box" width="10" height="10" depth="10" densityRate="10"/></pyro></object3D>"#,
        r#"<override target="solid" property="x" value="100"/>"#,
    );
    let frame = ev.evaluate(0.5);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    let centre = |id: &str| {
        node(&frame, id).sim_volume.as_ref().expect("volume").data.grid("density").unwrap().sample_world([0.5; 3])
    };
    // Instance a keeps its sphere at the middle of the domain, which excludes smoke;
    // instance b moved its own sphere away.
    assert_eq!(centre("a/cloud"), 0.0, "the sphere of a must exclude smoke");
    assert!(centre("b/cloud") > 0.5, "b must not see a's sphere: {}", centre("b/cloud"));
}

#[test]
fn particle_colliders_in_a_symbol_belong_to_their_instance() {
    let ev = evaluator(
        r#"<object3D id="wall" primitive="box" x="5" width="0.2" height="40" depth="40" visible="false"/>
           <particles3D id="p" rate="0" velocityX="10" gravityY="0" collisionRadius="0.1" bounce="1" dt="0.05" lifetime="3" colliders="wall"><burst time="0" count="1"/></particles3D>"#,
        r#"<override target="wall" property="x" value="100"/>"#,
    );
    let frame = ev.evaluate(1.0);
    assert!(frame.problems.is_empty(), "{:?}", frame.problems);
    let x = |id: &str| node(&frame, id).particles3d.as_ref().unwrap().frame.particles[0].position[0];
    // In a the wall at x = 5 turns the particle back; in b it flies on to x = 10.
    assert!(x("a/p") < 5.0, "a's particle should have bounced: {}", x("a/p"));
    assert!((x("b/p") - 10.0).abs() < 1e-6, "b's particle should be free: {}", x("b/p"));
}

/// A wave measured away from the sphere, in the 20 to 30 unit ring.
fn far_wave(ev: &Evaluator, id: &str) -> f64 {
    (4..=15)
        .map(|k| {
            let frame = ev.evaluate(k as f64 * 0.25);
            assert!(frame.problems.is_empty(), "{:?}", frame.problems);
            let cells = &node(&frame, id).sim_ocean.as_ref().unwrap().frame.cells;
            (0..cells.len())
                .filter(|&i| {
                    let (x, z) = ((i % 64) as f64 * 2.0 + 1.0 - 64.0, (i / 64) as f64 * 2.0 + 1.0 - 64.0);
                    (20.0..30.0).contains(&x.hypot(z))
                })
                .map(|i| (cells[i].depth - 12.0).abs())
                .fold(0.0, f64::max)
        })
        .fold(0.0, f64::max)
}

#[test]
fn ocean_colliders_in_a_symbol_belong_to_their_instance() {
    let ev = evaluator(
        r#"<object3D id="impactor" primitive="sphere" radius="7" segments="24"><animate property="y"><key time="0" value="-40"/><key time="1" value="6"/></animate></object3D>
           <ocean id="sea" bedResponse="hydrostatic" width="128" depth="128" cellSize="2" bottomDepth="12" dt="0.0416666666666667" boundary="closed" colliders="impactor"/>"#,
        r#"<override target="impactor" property="radius" value="3"/>"#,
    );
    let (a, b) = (far_wave(&ev, "a/sea"), far_wave(&ev, "b/sea"));
    println!("SCOPE far-field wave: instance a {a:.4} (radius 7), instance b {b:.4} (radius 3)");
    assert!(a > 1e-3 && b > 1e-3, "{a} {b}");
    assert!(a > 2.0 * b, "each instance must see its own sphere: {a} against {b}");
}
