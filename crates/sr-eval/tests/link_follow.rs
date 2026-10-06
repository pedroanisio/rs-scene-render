//! `link/@follow`: a first-order or spring follower of the source's history, evaluated at any time without state.

use sr_eval::{EvalOptions, Evaluator};

fn doc(link: &str) -> sr_model::Document {
    let xml = format!(
        r#"<scene version="1.2"><project width="400" height="100" fps="60" duration="6"/><composition>
  <shape id="a" shape="rect" width="10" height="10"><animate property="x" defaultInterpolation="hold">
    <key time="0" value="0"/><key time="1" value="100"/></animate></shape>
  <shape id="b" shape="rect" width="10" height="10"><link property="x" source="a.x" {link}/></shape>
</composition></scene>"#
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn b_at(d: &sr_model::Document, t: f64) -> f64 {
    let f = Evaluator::new(d, &EvalOptions::default()).unwrap_or_else(|r| panic!("{r}")).evaluate(t);
    f.nodes.iter().find(|n| &*n.id == "b").unwrap().world.apply([0.0, 0.0])[0]
}

fn warnings(d: &sr_model::Document) -> Vec<String> {
    Evaluator::new(d, &EvalOptions::default()).unwrap().warnings().iter().map(|w| w.message.clone()).collect()
}

#[test]
fn without_follow_the_link_is_unchanged() {
    let d = doc("");
    assert_eq!(b_at(&d, 0.5), 0.0);
    assert_eq!(b_at(&d, 1.5), 100.0);
    assert_eq!(b_at(&doc(r#"follow="none" timeConstant="0.5""#), 1.5), 100.0);
}

#[test]
fn the_exponential_follower_charges_like_a_first_order_lag() {
    let d = doc(r#"follow="exponential" timeConstant="0.2""#);
    for t in [0.5, 0.99] {
        assert!(b_at(&d, t).abs() < 1e-6, "before the step: {}", b_at(&d, t));
    }
    // a unit step through h(u) = e^(-u/T)/T: f(t) = 100 (1 - e^(-(t-1)/T))
    for dt in [0.1, 0.2, 0.5, 1.0, 2.0] {
        let want = 100.0 * (1.0 - (-dt / 0.2f64).exp());
        let got = b_at(&d, 1.0 + dt);
        assert!((got - want).abs() < 1.0, "t = 1 + {dt}: {got} vs {want}");
    }
}

#[test]
fn the_spring_follower_overshoots_and_settles() {
    // k = 100, c = 4, m = 1: w0 = 10, zeta = 0.2; step response 1 - e^(-zeta w0 u)(cos wd u + zeta/sqrt(1-zeta^2) sin wd u)
    let d = doc(r#"follow="spring" stiffness="100" damping="4" mass="1""#);
    let (w0, z) = (10.0f64, 0.2f64);
    let wd = w0 * (1.0 - z * z).sqrt();
    let step = |u: f64| 100.0 * (1.0 - (-z * w0 * u).exp() * ((wd * u).cos() + z / (1.0 - z * z).sqrt() * (wd * u).sin()));
    for u in [0.05, 0.15, 0.3, 0.6, 1.0, 2.0] {
        let got = b_at(&d, 1.0 + u);
        assert!((got - step(u)).abs() < 1.5, "u = {u}: {got} vs {}", step(u));
    }
    // first peak at u = pi / wd, height 100 (1 + e^(-pi z / sqrt(1 - z^2)))
    let peak = 100.0 * (1.0 + (-std::f64::consts::PI * z / (1.0 - z * z).sqrt()).exp());
    let got = b_at(&d, 1.0 + std::f64::consts::PI / wd);
    assert!(got > 100.0 && (got - peak).abs() < 1.5, "{got} vs {peak}");
    assert!((b_at(&d, 5.0) - 100.0).abs() < 0.5, "settled: {}", b_at(&d, 5.0));
}

#[test]
fn overdamped_and_critical_springs_do_not_overshoot() {
    for damping in ["20", "40"] {
        let d = doc(&format!(r#"follow="spring" stiffness="100" damping="{damping}" mass="1""#));
        let top = (0..400).map(|k| b_at(&d, 1.0 + k as f64 * 0.01)).fold(0.0, f64::max);
        assert!(top <= 100.0 + 0.5, "damping {damping}: peak {top}");
        assert!((b_at(&d, 5.0) - 100.0).abs() < 0.5);
    }
    // critical (c = 20): step response 1 - e^(-w0 u)(1 + w0 u)
    let d = doc(r#"follow="spring" stiffness="100" damping="20" mass="1""#);
    let want = 100.0 * (1.0 - (-10.0f64 * 0.3).exp() * (1.0 + 3.0));
    assert!((b_at(&d, 1.3) - want).abs() < 1.0, "{} vs {want}", b_at(&d, 1.3));
}

#[test]
fn a_constant_source_gives_that_constant_and_a_follower_is_a_pure_function_of_time() {
    let d = doc(r#"follow="spring" stiffness="100" damping="4" mass="1""#);
    assert!((b_at(&d, 0.0) - 0.0).abs() < 1e-9);
    assert!((b_at(&d, 0.5) - 0.0).abs() < 1e-9);
    // evaluating out of order gives the same values
    let late = b_at(&d, 1.4);
    let _ = b_at(&d, 3.0);
    assert_eq!(b_at(&d, 1.4), late);
    // the output is scaled, offset and clamped after the follower: a spring overshoot is cut by max
    let clipped = doc(r#"follow="spring" stiffness="100" damping="4" mass="1" max="100""#);
    let top = (0..300).map(|k| b_at(&clipped, 1.0 + k as f64 * 0.01)).fold(0.0, f64::max);
    assert!(top <= 100.0 + 1e-9, "{top}");
}

#[test]
fn smoothing_wins_and_follow_is_reported_ignored_and_unread_parameters_are_reported() {
    let both = doc(r#"follow="exponential" smoothing="0.2""#);
    assert!(warnings(&both).iter().any(|m| m.contains("follow") && m.contains("smoothing")), "{:?}", warnings(&both));
    let inert = doc(r#"timeConstant="0.4""#);
    assert!(warnings(&inert).iter().any(|m| m.contains("timeConstant")), "{:?}", warnings(&inert));
    let spring_params = doc(r#"follow="exponential" stiffness="200""#);
    assert!(warnings(&spring_params).iter().any(|m| m.contains("stiffness")), "{:?}", warnings(&spring_params));
    assert!(warnings(&doc(r#"follow="spring" stiffness="200""#)).iter().all(|m| !m.contains("stiffness")));
}
