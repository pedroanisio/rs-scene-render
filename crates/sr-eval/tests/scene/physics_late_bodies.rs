//! Rigid bodies in a group that starts after `physics@start` join the 2D world when their group starts.
//!
//! v0.1.4 built the world from the frame at `physics@start` only, so the bodies of a later group were silently left
//! out: in the Inova film (PALS-Notes/projects/inova-institucional/film/GATES.md, scene 2) 100 cards never fell into
//! their bins. A body that joins later does not touch the world before it exists.

use sr_eval::{EvalOptions, Evaluator};

const XML: &str = r##"<scene version="1.2"><project width="400" height="400" fps="30" duration="4"/>
<composition>
  <shape id="early" shape="rect" x="190" y="0" width="20" height="20" fill="#FFFFFF"><rigidBody/></shape>
  <group id="later" start="1">
    <shape id="floor" shape="rect" x="0" y="300" width="400" height="20" fill="#FFFFFF"><rigidBody type="static"/></shape>
    <shape id="card" shape="rect" x="50" y="100" width="20" height="20" fill="#FFFFFF"><rigidBody/></shape>
  </group>
</composition>
<physics pixelsPerMeter="100"/></scene>"##;

fn y_of(ev: &Evaluator, id: &str, t: f64) -> f64 {
    let f = ev.evaluate(t);
    let n = f.nodes.iter().find(|n| &*n.id == id).unwrap_or_else(|| panic!("{id} at {t} s"));
    n.world.apply([0.0, 0.0])[1]
}

#[test]
fn bodies_of_a_group_that_starts_later_fall_when_it_starts() {
    let doc = sr_model::load_str(XML, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = Evaluator::new(&doc, &EvalOptions::default()).unwrap();
    // the card is where its group puts it when the group starts
    assert!((y_of(&ev, "card", 1.0) - 100.0).abs() < 1.0, "{}", y_of(&ev, "card", 1.0));
    // it falls and comes to rest on the floor of its own group (top at 300 px, the card 20 px tall)
    let rest = y_of(&ev, "card", 3.5);
    assert!((rest - 280.0).abs() < 2.0, "card at 3.5 s: y = {rest}");
    // the floor did not exist while the early box fell through where it will be
    let early = y_of(&ev, "early", 1.5);
    assert!(early > 400.0, "early box at 1.5 s: y = {early}");
}

#[test]
fn a_body_that_is_in_the_frame_at_the_start_moves_as_before() {
    let doc = sr_model::load_str(XML, &sr_model::LoadOptions::without_assets()).unwrap();
    let ev = Evaluator::new(&doc, &EvalOptions::default()).unwrap();
    // free fall from rest: y = g t^2 / 2, 9.80665 m/s^2 at 100 px/m
    let y = y_of(&ev, "early", 0.5);
    assert!((y - 0.5 * 980.665 * 0.25).abs() < 3.0, "early box at 0.5 s: y = {y}");
}

/// A soft body joins the world only at its start: one in a group that starts later is reported, not silently dropped.
#[test]
fn a_soft_body_that_starts_later_is_reported() {
    let xml = r##"<scene version="1.2"><project width="400" height="400" fps="30" duration="2"/>
<composition>
  <shape id="early" shape="rect" x="0" y="0" width="20" height="20" fill="#FFFFFF"><rigidBody/></shape>
  <group id="later" start="1"><shape id="jelly" shape="rect" x="50" y="50" width="40" height="40" fill="#FFFFFF"><softBody/></shape></group>
</composition>
<physics pixelsPerMeter="100"/></scene>"##;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let g = Evaluator::new(&doc, &EvalOptions::default()).unwrap().evaluate(1.5);
    assert!(g.problems.iter().any(|m| m.contains("jelly") && m.contains("not simulated")), "{:?}", g.problems);
}
