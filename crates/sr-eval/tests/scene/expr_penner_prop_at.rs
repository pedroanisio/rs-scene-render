//! Expressions: `penner("curve", u)` evaluates any named curve, `propAtTime("id.prop", t)` reads another property at
//! another composition time.

use sr_eval::{EvalOptions, Evaluator};

fn doc(expression: &str) -> sr_model::Document {
    let xml = format!(
        r#"<scene version="1.2"><project width="400" height="100" fps="60" duration="6"/><composition>
  <shape id="a" shape="rect" width="10" height="10"><animate property="x"><key time="0" value="0"/><key time="2" value="100"/></animate></shape>
  <shape id="c" shape="rect" width="10" height="10"><expression property="x">100 * time</expression></shape>
  <shape id="b" shape="rect" width="10" height="10"><expression property="x">{expression}</expression></shape>
</composition></scene>"#
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn b_at(d: &sr_model::Document, t: f64) -> f64 {
    let f = Evaluator::new(d, &EvalOptions::default()).unwrap_or_else(|r| panic!("{r}")).evaluate(t);
    f.nodes.iter().find(|n| &*n.id == "b").unwrap().world.apply([0.0, 0.0])[0]
}

fn compile_error(expression: &str) -> String {
    match Evaluator::new(&doc(expression), &EvalOptions::default()) {
        Err(r) => r.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("; "),
        Ok(_) => panic!("{expression} compiled"),
    }
}

#[test]
fn penner_evaluates_a_named_curve() {
    // quad-in(0.5) = 0.25; back-out(0.5) = 1 + 2.70158 (-0.5)^3 + 1.70158 (-0.5)^2
    assert!((b_at(&doc(r#"100 * penner("quad-in", 0.5)"#), 1.0) - 25.0).abs() < 1e-9);
    let back = 1.0 + 2.70158 * -0.125 + 1.70158 * 0.25;
    assert!((b_at(&doc(r#"100 * penner("back-out", 0.5)"#), 1.0) - 100.0 * back).abs() < 1e-6);
    // driven by time: an ease over the first second of the node's own time
    let eased = doc(r#"100 * penner("cubic-out", Math.min(time, 1))"#);
    assert!((b_at(&eased, 0.5) - 87.5).abs() < 1e-9 && (b_at(&eased, 3.0) - 100.0).abs() < 1e-9);
    // a spring is its progress over a one-second segment: lands on 1, and overshoots on the way
    let spring = |u: f64| b_at(&doc(&format!(r#"penner("spring", {u})"#)), 1.0);
    assert!((spring(1.0) - 1.0).abs() < 1e-9 && spring(0.3) > 1.0, "{} {}", spring(1.0), spring(0.3));
    // the curves without parameters of a key: the endpoints are the endpoints
    for name in ["linear", "ease-in-out", "elastic-out", "bounce-out", "spring", "sine-in-out"] {
        let d = doc(&format!(r#"penner("{name}", 1)"#));
        assert!((b_at(&d, 1.0) - 1.0).abs() < 1e-9, "{name}");
    }
}

#[test]
fn an_unknown_curve_is_a_compile_error_with_a_suggestion() {
    let e = compile_error(r#"penner("quad-inn", 0.5)"#);
    assert!(e.contains("unknown curve") && e.contains("quad-in"), "{e}");
    assert!(compile_error(r#"penner(1, 0.5)"#).contains("string literal"));
}

#[test]
fn prop_at_time_reads_another_property_at_another_time() {
    // a.x runs 0 -> 100 over 2 s: at composition time 1.5, a second ago it was 25
    let d = doc(r#"propAtTime("a.x", time - 1)"#);
    assert!((b_at(&d, 1.5) - 25.0).abs() < 1e-9, "{}", b_at(&d, 1.5));
    assert!((b_at(&d, 1.0) - 0.0).abs() < 1e-9);
    // the full value of the other property, here itself an expression: c.x = 100 * time
    let via_expression = doc(r#"propAtTime("c.x", 0.5)"#);
    assert!((b_at(&via_expression, 3.0) - 50.0).abs() < 1e-9);
    // and it is what prop() reads now when asked for now
    let now = doc(r#"propAtTime("a.x", time)"#);
    assert!((b_at(&now, 1.0) - 50.0).abs() < 1e-9);
}

#[test]
fn prop_at_time_needs_a_known_property() {
    let e = compile_error(r#"propAtTime("nobody.x", 1)"#);
    assert!(e.contains("nobody"), "{e}");
}
