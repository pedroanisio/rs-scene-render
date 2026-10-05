//! `key/@overshoot` (back-* curves) and `key/@period` (elastic-* curves) tune the fixed Penner constants; a key that
//! sets one on a curve that does not read it is reported (E19, a warning).

use sr_eval::{EvalOptions, Evaluator};

fn document(key: &str, interpolation: &str) -> sr_model::Document {
    let xml = format!(
        r#"<scene version="1.2"><project width="400" height="100" fps="10" duration="2"/><composition>
  <shape id="s" shape="rect" width="10" height="10"><animate property="x">
    <key time="0" value="0" interpolation="{interpolation}" {key}/><key time="1" value="100"/></animate></shape>
</composition></scene>"#
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn x_at(d: &sr_model::Document, t: f64) -> f64 {
    let f = Evaluator::new(d, &EvalOptions::default()).unwrap_or_else(|r| panic!("{r}")).evaluate(t);
    f.nodes.iter().find(|n| &*n.id == "s").unwrap().world.apply([0.0, 0.0])[0]
}

fn warnings(d: &sr_model::Document) -> Vec<(String, String)> {
    let ev = Evaluator::new(d, &EvalOptions::default()).unwrap();
    ev.warnings().iter().map(|w| (w.code.clone(), w.message.clone())).collect()
}

#[test]
fn back_out_overshoot_scales_the_peak_and_the_default_is_unchanged() {
    // back-out(u) = 1 + (c+1)(u-1)^3 + c(u-1)^2 at u = 0.5: c = 1.70158 -> 1.08770, c = 0 -> 0.875, c = 3 -> 1.25
    let at_half = |attrs: &str| x_at(&document(attrs, "back-out"), 0.5);
    assert!((at_half("") - 108.7704).abs() < 1e-3, "{}", at_half(""));
    assert!((at_half(r#"overshoot="1.70158""#) - at_half("")).abs() < 1e-9);
    assert!((at_half(r#"overshoot="0""#) - 87.5).abs() < 1e-9, "{}", at_half(r#"overshoot="0""#));
    assert!((at_half(r#"overshoot="3""#) - 125.0).abs() < 1e-9, "{}", at_half(r#"overshoot="3""#));
}

#[test]
fn back_in_and_in_out_read_it_too() {
    // back-in(u) = (c+1)u^3 - c u^2: at u = 0.5, c = 3 -> 0.5 - 0.75 = -0.25 (a dip before the move)
    assert!((x_at(&document(r#"overshoot="3""#, "back-in"), 0.5) - -25.0).abs() < 1e-9);
    // back-in-out is symmetric about the midpoint whatever the constant
    let io = |c: &str| x_at(&document(&format!(r#"overshoot="{c}""#), "back-in-out"), 0.5);
    assert!((io("0.5") - 50.0).abs() < 1e-9 && (io("4") - 50.0).abs() < 1e-9);
    // and a larger constant digs deeper before the start: u = 0.2 is negative and more so at 4 than at 0.5
    let early = |c: &str| x_at(&document(&format!(r#"overshoot="{c}""#), "back-in-out"), 0.2);
    assert!(early("4") < early("0.5") && early("0.5") < 0.0, "{} {}", early("4"), early("0.5"));
}

#[test]
fn elastic_period_sets_the_ringing_and_the_default_is_unchanged() {
    // elastic-out(u) = 2^(-10u) sin((10u - p*10/4) * 2pi*0.1/p) + 1, default p = 0.3 (easings.net C4 = 2pi/3)
    let at = |attrs: &str, t: f64| x_at(&document(attrs, "elastic-out"), t);
    let reference = |u: f64, p: f64| {
        100.0 * (2f64.powf(-10.0 * u) * ((10.0 * u - p * 2.5) * 2.0 * std::f64::consts::PI * 0.1 / p).sin() + 1.0)
    };
    for u in [0.1, 0.25, 0.4, 0.7] {
        assert!((at("", u) - reference(u, 0.3)).abs() < 1e-9, "default at {u}");
        assert!((at(r#"period="0.3""#, u) - at("", u)).abs() < 1e-9, "explicit default at {u}");
        assert!((at(r#"period="0.6""#, u) - reference(u, 0.6)).abs() < 1e-9, "period 0.6 at {u}");
    }
    assert!((at(r#"period="0.6""#, 0.25) - at("", 0.25)).abs() > 1.0, "the period must change the curve");
}

#[test]
fn a_parameter_on_a_curve_that_ignores_it_is_reported() {
    let w = warnings(&document(r#"overshoot="2""#, "ease-out"));
    assert!(w.iter().any(|(c, m)| c == "E19" && m.contains("overshoot") && m.contains("back")), "{w:?}");
    let w = warnings(&document(r#"period="0.4""#, "back-out"));
    assert!(w.iter().any(|(c, m)| c == "E19" && m.contains("period") && m.contains("elastic")), "{w:?}");
    // read by their own curves: silent
    assert!(warnings(&document(r#"overshoot="2""#, "back-in-out")).is_empty());
    assert!(warnings(&document(r#"period="0.4""#, "elastic-in")).is_empty());
    assert!(warnings(&document("", "back-out")).is_empty());
}

#[test]
fn the_curve_a_key_inherits_from_default_interpolation_counts() {
    let xml = |default: &str| {
        format!(
            r#"<scene version="1.2"><project width="400" height="100" fps="10" duration="2"/><composition>
  <shape id="s" shape="rect" width="10" height="10"><animate property="x" defaultInterpolation="{default}">
    <key time="0" value="0" overshoot="3"/><key time="1" value="100"/></animate></shape>
</composition></scene>"#
        )
    };
    let load = |d: &str| sr_model::load_str(&xml(d), &sr_model::LoadOptions::without_assets()).unwrap();
    assert!(warnings(&load("back-out")).is_empty());
    assert!(!warnings(&load("linear")).is_empty());
    assert!((x_at(&load("back-out"), 0.5) - 125.0).abs() < 1e-9);
}
