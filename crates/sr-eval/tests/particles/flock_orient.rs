//! SREP 73: `flock/@orientToVelocity`. `true` (the default) turns each agent's disc or sprite along its velocity;
//! `false` keeps rotation 0 in the flock's own space, so the node's transform still turns the whole flock. The SREP's
//! kit cases (`srep-0073-*`) checked at the evaluator: the rotation each agent is drawn with, and where it is.

use sr_eval::Evaluator;

fn evaluator(flock_attrs: &str) -> Evaluator {
    let xml = format!(
        r##"<scene version="1.6"><project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
  <assets><image id="spr" src="srep73-halves.png" width="8" height="8"/></assets>
  <composition><flock id="f" seed="3" x="320" y="180" sprite="spr" size="40" {flock_attrs}/></composition></scene>"##
    );
    let doc =
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"))
}

/// `(position, velocity, rotation in degrees)` of every agent at time `t`.
fn agents(ev: &Evaluator, t: f64) -> Vec<([f32; 2], [f32; 2], f32)> {
    let frame = ev.evaluate(t);
    assert!(frame.failures.is_empty() && frame.problems.is_empty(), "{:?} {:?}", frame.failures, frame.problems);
    let p =
        frame.nodes.iter().find(|n| &*n.id == "f").and_then(|n| n.particles.clone()).expect("the flock's particles");
    (0..p.pos.len()).map(|k| (p.pos[k], p.vel[k], p.rot[k])).collect()
}

/// The difference of two angles in degrees, folded into [0, 180].
fn angle_gap(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

#[test]
fn srep_0073_default_turns_each_agent_along_its_velocity() {
    let ev = evaluator(r#"count="12" width="200" height="120" shape="sprite""#);
    for t in [0.0, 0.5, 0.9] {
        for (_, v, rot) in agents(&ev, t) {
            let along = v[1].atan2(v[0]).to_degrees();
            assert!(angle_gap(rot, along) < 1e-3, "t {t}: rotation {rot} is not the velocity's angle {along}");
        }
    }
}

#[test]
fn srep_0073_upright_sprite_keeps_rotation_zero() {
    let ev = evaluator(r#"count="12" width="200" height="120" shape="sprite" orientToVelocity="false""#);
    for t in [0.0, 0.5, 0.9] {
        let a = agents(&ev, t);
        assert_eq!(a.len(), 12);
        assert!(a.iter().any(|(_, v, _)| v[1].atan2(v[0]).abs() > 0.1), "t {t}: some agent heads off the +x axis");
        for (_, _, rot) in a {
            assert!(angle_gap(rot, 0.0) < 1e-3, "t {t}: rotation {rot}, not 0");
        }
    }
}

#[test]
fn srep_0073_upright_sprite_node_rotation_still_turns_the_flock() {
    let ev = evaluator(r#"count="12" width="200" height="120" shape="sprite" orientToVelocity="false" rotation="90""#);
    for t in [0.0, 0.5] {
        for (_, _, rot) in agents(&ev, t) {
            assert!(angle_gap(rot, 90.0) < 1e-3, "t {t}: rotation {rot}, not the node's 90");
        }
    }
}

#[test]
fn srep_0073_kit_placement_one_agent_sits_at_the_node_origin() {
    // the kit's assumption: a one-agent flock in a 1 × 1 box has its agent within 1 px of the node's origin at time 0
    for attrs in [
        r#"count="1" width="1" height="1" shape="sprite" orientToVelocity="false""#,
        r#"count="1" width="1" height="1" shape="sprite" orientToVelocity="false" rotation="90""#,
    ] {
        let a = agents(&evaluator(attrs), 0.0);
        assert_eq!(a.len(), 1);
        let [x, y] = a[0].0;
        assert!((x - 320.0).abs() <= 1.0 && (y - 180.0).abs() <= 1.0, "{attrs}: agent at ({x}, {y})");
    }
}
