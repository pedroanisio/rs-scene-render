//! A key parameter that the key's curve does not read is reported (E19, a warning), as `overshoot` and `period` are.

use sr_eval::{EvalOptions, Evaluator};

fn doc(default: &str, k1: &str, k2: &str) -> sr_model::Document {
    let xml = format!(
        r#"<scene version="1.2"><project width="400" height="100" fps="10" duration="2"/><composition>
  <shape id="s" shape="rect" width="10" height="10"><animate property="x" defaultInterpolation="{default}">
    <key time="0" value="0" {k1}/><key time="1" value="100" {k2}/></animate></shape>
</composition></scene>"#
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn warned(d: &sr_model::Document) -> Vec<String> {
    let ev = Evaluator::new(d, &EvalOptions::default()).unwrap();
    ev.warnings().iter().filter(|w| w.code == "E19").map(|w| w.message.clone()).collect()
}

fn says(d: &sr_model::Document, attr: &str, family: &str) -> bool {
    warned(d).iter().any(|m| m.contains(&format!("@{attr}")) && m.contains(family))
}

#[test]
fn bezier_and_handles_are_read_by_cubic_bezier_only() {
    assert!(says(&doc("linear", r#"bezier="0.2,0,0.2,1""#, ""), "bezier", "cubic-bezier"));
    assert!(says(&doc("linear", r#"interpolation="ease-out" easeOut="0.3,1""#, ""), "easeOut", "cubic-bezier"));
    // the first key's easeIn is never read; a later key's is read by the segment that ends in it
    assert!(says(
        &doc("cubic-bezier", r#"easeIn="0.3,1" easeOut="0.3,1""#, r#"easeIn="0.3,1""#),
        "easeIn",
        "cubic-bezier"
    ));
    assert!(says(&doc("linear", "", r#"easeIn="0.3,1""#), "easeIn", "cubic-bezier"));
    // @bezier wins over the handles of the same key and of the key that follows
    assert!(says(&doc("cubic-bezier", r#"bezier="0.2,0,0.2,1" easeOut="0.3,1""#, ""), "easeOut", "bezier"));
    assert!(says(&doc("cubic-bezier", r#"bezier="0.2,0,0.2,1""#, r#"easeIn="0.3,1""#), "easeIn", "bezier"));
    // read: silent
    assert!(warned(&doc("cubic-bezier", r#"bezier="0.2,0,0.2,1""#, "")).is_empty());
    assert!(warned(&doc("cubic-bezier", r#"easeOut="0.3,1""#, r#"easeIn="0.3,1""#)).is_empty());
    assert!(warned(&doc("linear", r#"interpolation="cubic-bezier" easeOut="0.3,1""#, r#"easeIn="0.3,1""#)).is_empty());
}

#[test]
fn steps_are_read_by_the_steps_curve_only() {
    assert!(says(&doc("linear", r#"steps="4""#, ""), "steps", "steps"));
    assert!(says(&doc("ease-in", r#"stepPosition="start""#, ""), "stepPosition", "steps"));
    assert!(warned(&doc("steps", r#"steps="4" stepPosition="start""#, "")).is_empty());
    assert!(warned(&doc("linear", "", "")).is_empty());
}

#[test]
fn spring_and_tcb_parameters_are_read_by_their_curves_only() {
    assert!(says(&doc("linear", r#"stiffness="200""#, ""), "stiffness", "spring"));
    assert!(says(&doc("linear", r#"damping="4" mass="2""#, ""), "damping", "spring"));
    assert!(says(&doc("linear", r#"tension="0.5""#, ""), "tension", "tcb"));
    assert!(says(&doc("catmull-rom", r#"bias="0.5""#, ""), "bias", "tcb"));
    assert!(warned(&doc("spring", r#"stiffness="200" damping="4" mass="2""#, "")).is_empty());
    assert!(warned(&doc("tcb", r#"tension="0.5" continuity="0.2" bias="-0.3""#, "")).is_empty());
    // the tangent of a key that follows a tcb key is shaped by the segment before it
    assert!(warned(&doc("linear", r#"interpolation="tcb""#, r#"tension="0.5""#)).is_empty());
    // the values a key has when it does not set them are not parameters
    assert!(warned(&doc("linear", r#"stiffness="100" damping="10" mass="1" tension="0" continuity="0" bias="0""#, ""))
        .is_empty());
}

#[test]
fn a_key_that_takes_cubic_bezier_from_the_default_without_handles_is_reported() {
    let said = |d: &sr_model::Document| {
        warned(d).iter().any(|m| m.contains("defaultInterpolation") && m.contains("default handles"))
    };
    assert!(said(&doc("cubic-bezier", "", "")), "no handles anywhere");
    assert!(!said(&doc("cubic-bezier", r#"bezier="0.2,0,0.2,1""#, "")));
    assert!(!said(&doc("cubic-bezier", r#"easeOut="0.3,1""#, "")));
    assert!(!said(&doc("cubic-bezier", "", r#"easeIn="0.3,1""#)), "the next key's easeIn is a handle of the segment");
    // the last key starts no segment; a key that names the curve is C40's business and not repeated here
    assert_eq!(warned(&doc("cubic-bezier", r#"bezier="0.2,0,0.2,1""#, "")).len(), 0);
    assert!(!said(&doc("linear", r#"interpolation="cubic-bezier" bezier="0.2,0,0.2,1""#, "")));
    // it is reported once, for the key that starts the segment
    assert_eq!(warned(&doc("cubic-bezier", "", "")).len(), 1);
    // and it is only a warning: the animation still evaluates with the default handles
    let d = doc("cubic-bezier", "", "");
    let ev = Evaluator::new(&d, &EvalOptions::default()).unwrap();
    let x = ev.evaluate(0.5).nodes.iter().find(|n| &*n.id == "s").unwrap().world.apply([0.0, 0.0])[0];
    assert!(x > 0.0 && x < 100.0, "{x}");
}
