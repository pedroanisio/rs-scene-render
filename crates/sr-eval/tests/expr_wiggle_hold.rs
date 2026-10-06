//! `wiggle(freq, amp, octaves, ampMult, t, hold)`: the sixth argument holds the wiggle for `hold` seconds at a time.

use sr_eval::{EvalOptions, Evaluator};

fn x_at(expression: &str, t: f64) -> f64 {
    let xml = format!(
        r#"<scene version="1.2"><project width="400" height="100" fps="60" duration="6" seed="3"/><composition>
  <shape id="s" shape="rect" x="100" width="10" height="10"><expression property="x">{expression}</expression></shape>
</composition></scene>"#
    );
    let d =
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    let f = Evaluator::new(&d, &EvalOptions::default()).unwrap_or_else(|r| panic!("{r}")).evaluate(t);
    f.nodes.iter().find(|n| &*n.id == "s").unwrap().world.apply([0.0, 0.0])[0]
}

#[test]
fn the_wiggle_is_constant_within_a_hold_and_changes_between_holds() {
    let w = |t: f64| x_at("wiggle(3, 20, 1, 0.5, time, 0.25)", t);
    // 0.25 s holds: 1.00 to 1.25 is one value
    let a = w(1.0);
    for t in [1.01, 1.1, 1.2, 1.249] {
        assert_eq!(w(t), a, "t = {t}");
    }
    assert_ne!(w(1.26), a, "the next hold differs");
    // the value of a hold is the wiggle at the start of the hold
    assert_eq!(a, x_at("wiggle(3, 20, 1, 0.5, 1.0)", 1.2));
}

#[test]
fn without_a_hold_or_with_none_the_wiggle_is_unchanged() {
    let plain = |t| x_at("wiggle(3, 20)", t);
    for t in [0.3, 1.234, 2.5] {
        assert_eq!(x_at("wiggle(3, 20, 1, 0.5, time)", t), plain(t));
        assert_eq!(x_at("wiggle(3, 20, 1, 0.5, time, 0)", t), plain(t), "a hold of 0 holds nothing");
    }
}
