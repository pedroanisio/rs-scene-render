//! `key/@carry`: a spring segment that starts with the velocity the previous segment arrives with.

use sr_eval::{EvalOptions, Evaluator};

fn doc(carry: &str, k1_extra: &str) -> sr_model::Document {
    let xml = format!(
        r#"<scene version="1.2"><project width="400" height="100" fps="60" duration="6"/><composition>
  <shape id="s" shape="rect" width="10" height="10"><animate property="x">
    <key time="0" value="0" interpolation="linear"/>
    <key time="1" value="100" interpolation="spring" stiffness="100" damping="10" mass="1" {carry} {k1_extra}/>
    <key time="3" value="0"/></animate></shape>
</composition></scene>"#
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn x_at(d: &sr_model::Document, t: f64) -> f64 {
    let f = Evaluator::new(d, &EvalOptions::default()).unwrap_or_else(|r| panic!("{r}")).evaluate(t);
    f.nodes.iter().find(|n| &*n.id == "s").unwrap().world.apply([0.0, 0.0])[0]
}

fn infos(d: &sr_model::Document) -> Vec<String> {
    Evaluator::new(d, &EvalOptions::default()).unwrap().warnings().iter().filter(|w| w.is_info()).map(|w| w.message.clone()).collect()
}

/// Spring response (k = 100, c = 10, m = 1: w0 = 10, zeta = 0.5) of the displacement x - 0 starting at 100 with
/// velocity `v0`, toward 0: x(u) = 100 * (1 - s(u)) + v0 * g(u), with s the step response and g the release response.
fn analytic(u: f64, v0: f64) -> f64 {
    let (w0, z) = (10.0f64, 0.5f64);
    let wd = w0 * (1.0 - z * z).sqrt();
    let s = 1.0 - (-z * w0 * u).exp() * ((wd * u).cos() + z * w0 / wd * (wd * u).sin());
    let g = (-z * w0 * u).exp() * (wd * u).sin() / wd;
    100.0 * (1.0 - s) + v0 * g
}

#[test]
fn without_carry_the_spring_starts_from_rest_as_before() {
    let d = doc("", "");
    for u in [0.05, 0.2, 0.5, 1.0] {
        assert!((x_at(&d, 1.0 + u) - analytic(u, 0.0)).abs() < 0.2, "u = {u}: {} vs {}", x_at(&d, 1.0 + u), analytic(u, 0.0));
    }
    assert_eq!(x_at(&doc(r#"carry="false""#, ""), 1.3), x_at(&d, 1.3));
}

#[test]
fn carry_starts_the_spring_with_the_velocity_of_the_segment_before() {
    // the first segment runs 0 -> 100 in 1 s: 100 per second when the spring is released at 1 s
    let d = doc(r#"carry="true""#, "");
    for u in [0.02, 0.05, 0.1, 0.2, 0.4, 0.8] {
        let (got, want) = (x_at(&d, 1.0 + u), analytic(u, 100.0));
        assert!((got - want).abs() < 0.3, "u = {u}: {got} vs {want}");
    }
    // it carries on past the key instead of stopping dead: the incoming 100 per second takes it above the key's value
    // before the spring pulls it back (a spring from rest only ever moves toward the next key)
    assert!(x_at(&d, 1.005) > 100.2, "{}", x_at(&d, 1.005));
    assert!(x_at(&doc("", ""), 1.005) < 100.0);
    // and it still lands on the next key at its time
    assert!(x_at(&d, 3.0).abs() < 1e-6, "{}", x_at(&d, 3.0));
}

#[test]
fn carry_takes_the_velocity_of_a_spring_before_it_too() {
    // chained: a spring whose segment ends in motion hands that motion to the next spring
    let xml = r#"<scene version="1.2"><project width="400" height="100" fps="60" duration="6"/><composition>
  <shape id="s" shape="rect" width="10" height="10"><animate property="x">
    <key time="0" value="0" interpolation="spring" stiffness="100" damping="10" mass="1"/>
    <key time="0.5" value="100" interpolation="spring" stiffness="100" damping="10" mass="1" carry="true"/>
    <key time="2" value="0"/></animate></shape></composition></scene>"#;
    let d = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let plain = xml.replace(r#" carry="true""#, "");
    let p = sr_model::load_str(&plain, &sr_model::LoadOptions::without_assets()).unwrap();
    // the first spring is still moving at 0.5 s, so the carried second spring differs from the one from rest
    assert!((x_at(&d, 0.6) - x_at(&p, 0.6)).abs() > 1.0, "{} {}", x_at(&d, 0.6), x_at(&p, 0.6));
    // before the key nothing changes
    assert_eq!(x_at(&d, 0.4), x_at(&p, 0.4));
}

#[test]
fn carry_that_cannot_act_is_reported_as_information() {
    let on_linear = r#"<scene version="1.2"><project width="400" height="100" fps="60" duration="6"/><composition>
  <shape id="s" shape="rect" width="10" height="10"><animate property="x">
    <key time="0" value="0" carry="true"/><key time="1" value="100" carry="true" interpolation="linear"/><key time="2" value="0"/></animate></shape></composition></scene>"#;
    let d = sr_model::load_str(on_linear, &sr_model::LoadOptions::without_assets()).unwrap();
    let notes = infos(&d);
    assert!(notes.iter().filter(|m| m.contains("carry")).count() >= 2, "{notes:?}");
    assert!(infos(&doc(r#"carry="true""#, "")).iter().all(|m| !m.contains("carry")), "a carried spring is read");
}
