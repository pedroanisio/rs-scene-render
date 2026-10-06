//! A crater that grows from an impact has its size from the impact, so its `influenceDepth` is checked against that
//! size when the crater is made, not against the defaults of an authored one.

use sr_eval::Evaluator;

fn scene(influence: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><composition>
          <object3D id="rock" primitive="sphere" radius="1" y="-8"><rigidBody mass="5000" velocityY="40"/></object3D>
          <object3D id="ground" primitive="plane" width="40" height="40" segments="40" y="2" rotationX="-90">
            <crater id="pit" source="rock" targetMaterial="softRock" {influence}/><rigidBody type="static"/>
          </object3D></composition><physics pixelsPerMeter="1" gravityY="-9.80665"/></scene>"##
    )
}

fn failures(xml: &str) -> Vec<String> {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    let ev = Evaluator::new(&doc, &Default::default()).unwrap();
    let frame = ev.evaluate(2.5);
    frame.problems.iter().chain(&frame.failures).cloned().collect()
}

#[test]
fn an_envelope_that_holds_the_impacts_crater_is_accepted_and_one_that_does_not_is_an_error_when_it_is_made() {
    // the crater of a 5000 kg rock at 40 m/s in soft rock is a few metres deep
    assert!(failures(&scene("influenceDepth=\"60\"")).is_empty(), "{:?}", failures(&scene("influenceDepth=\"60\"")));
    let small = failures(&scene("influenceDepth=\"0.5\""));
    assert!(small.iter().any(|m| m.contains("envelope")), "{small:?}");
}
