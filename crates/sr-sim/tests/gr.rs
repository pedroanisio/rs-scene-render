//! Light around a black hole that does not rotate: the integration of its null geodesics against the closed
//! forms and the quadratures, and its determinism and symmetry.
use sr_sim::gr::{self, oracle, Config, Outcome};

const M: f64 = 1.0;
/// An observer so far away that the part of the ray before it is under a part in 1e8 of the deflection.
const FAR: f64 = 1e9;

fn fine(step: f64) -> Config {
    Config { step, max_steps: 4_000_000 }
}

/// The smallest impact parameter, found by bisection, of the rays from `FAR` that escape with this step.
fn separating_b(config: Config) -> f64 {
    let (mut low, mut high) = (4.0 * M, 7.0 * M);
    for _ in 0..60 {
        let mid = 0.5 * (low + high);
        match gr::trace_with(config, M, FAR, mid, true, 0.0, 0).outcome {
            Outcome::Captured => low = mid,
            Outcome::Escaped => high = mid,
        }
    }
    0.5 * (low + high)
}

#[test]
fn the_impact_parameter_that_separates_capture_from_escape_converges_to_the_shadow_radius() {
    let exact = oracle::shadow_radius(M);
    assert_eq!(exact, 27f64.sqrt() * M);
    let errors: Vec<f64> =
        [0.08, 0.04, 0.02, 0.01].iter().map(|&h| (separating_b(fine(h)) - exact).abs() / exact).collect();
    println!("SHADOW relative errors of the separating b by step 0.08, 0.04, 0.02, 0.01: {errors:?}");
    assert!(errors.windows(2).all(|w| w[1] < w[0]), "{errors:?}");
    assert!(errors[3] < 1e-4, "{errors:?}");
}

#[test]
fn the_deflection_of_the_integration_is_the_exact_one_and_goes_to_four_m_over_b() {
    for b in [10.0 * M, 20.0 * M, 100.0 * M] {
        let exact = oracle::deflection(M, b).expect("a ray that escapes");
        let traced = gr::trace_with(fine(2e-4), M, FAR, b, true, 0.0, 0);
        assert_eq!(traced.outcome, Outcome::Escaped, "b = {b}");
        let found = traced.phi_inf - std::f64::consts::PI;
        println!("DEFLECTION b = {b}: integration {found:.9}, quadrature {exact:.9}, 4M/b {:.9}", 4.0 * M / b);
        assert!((found - exact).abs() < 1e-5, "b = {b}: {found} against {exact}");
        // the weak-field series accounts for what 4M/b leaves, up to the fifth order in M/b
        let series = oracle::weak_field_deflection_series(M, b);
        let fifth = (exact - series).abs() * (b / M).powi(5);
        println!("SERIES b = {b}: residual of the fourth-order series times (b/M)^5 = {fifth:.1}");
        assert!(fifth < 3000.0, "b = {b}: {exact} against {series}");
    }
    // and the limit: the ratio to 4M/b goes to one as b grows
    let ratio = |b: f64| oracle::deflection(M, b).unwrap() / oracle::weak_field_deflection(M, b);
    assert!(
        (ratio(1e3 * M) - 1.0).abs() < 4e-3 && (ratio(1e5 * M) - 1.0).abs() < 4e-5,
        "{} {}",
        ratio(1e3),
        ratio(1e5)
    );
    // a ray inside the shadow has no deflection to speak of
    assert!(oracle::deflection(M, 5.0 * M).is_none());
}

#[test]
fn the_error_of_the_integration_falls_as_the_fourth_power_of_the_step() {
    let b = 10.0 * M;
    let exact = oracle::deflection(M, b).unwrap();
    let error =
        |h: f64| (gr::trace_with(fine(h), M, FAR, b, true, 0.0, 0).phi_inf - std::f64::consts::PI - exact).abs();
    let (e1, e2, e3) = (error(0.04), error(0.02), error(0.01));
    println!("ORDER errors of the deflection at b = 10 M by step 0.04, 0.02, 0.01: {e1:e} {e2:e} {e3:e}");
    // the angle at the end is found by a linear interpolation, which is of the second order: the whole falls at
    // least as the square of the step, and the part the Runge-Kutta contributes is far below it
    assert!(e2 < e1 / 3.5 && e3 < e2 / 3.5, "{e1:e} {e2:e} {e3:e}");
    // at the step of the shader the angle is good to a few parts in a thousand
    assert!(error(gr::STEP) < 2e-3, "{:e}", error(gr::STEP));
}

#[test]
fn an_emitter_where_the_ray_is_perpendicular_to_its_motion_has_only_the_gravitational_redshift() {
    for r in [6.0 * M, 8.0 * M, 20.0 * M] {
        let gravitational = (1.0 - 3.0 * M / r).sqrt();
        assert_eq!(gr::redshift(M, r, 9.0 * M, 0.0), gravitational);
        // Luminet's form at the sky angle zero, at any inclination
        for inclination in [0.2, 1.0, 1.5] {
            assert!((oracle::luminet_redshift(M, r, 9.0 * M, inclination, 0.0) - gravitational).abs() < 1e-15);
        }
        // the receding side is redder and the approaching side bluer, and the two forms agree
        let omega = oracle::kepler_omega(M, r);
        let (b, inclination) = (9.0 * M, 1.2);
        for alpha in [-1.3, -0.4, 0.4, 1.3] {
            let by_luminet = oracle::luminet_redshift(M, r, b, inclination, alpha);
            let by_shader = gr::redshift(M, r, b, inclination.sin() * alpha.sin());
            assert!((by_luminet - by_shader).abs() < 1e-15, "{by_luminet} {by_shader}");
            assert_eq!(by_luminet < gravitational, alpha > 0.0, "r = {r}, alpha = {alpha}");
            let one_plus_z = 1.0 / by_luminet;
            assert!((one_plus_z - (1.0 + omega * b * inclination.sin() * alpha.sin()) / gravitational).abs() < 1e-12);
        }
    }
}

#[test]
fn the_orbit_is_symmetric_in_phi() {
    // the equation is the same when phi runs backwards and w = du/dphi changes sign: a step of -h from the turning
    // point (where w = 0) mirrors the step of h, to the bit
    for (u, h) in [(0.02, 0.02), (0.1, 0.005), (0.3, 0.01)] {
        let (up, wp) = gr::rk4_step(M, u, 0.0, h);
        let (un, wn) = gr::rk4_step(M, u, 0.0, -h);
        assert_eq!(up.to_bits(), un.to_bits(), "u = {u}");
        assert_eq!(wp.to_bits(), (-wn).to_bits(), "u = {u}");
    }
    // and a ray from the turning point out sweeps half of what the whole ray does: the orbit is the same either way
    let b = 12.0 * M;
    let u_min = oracle::turning_point(M, b).unwrap();
    let out = gr::trace_with(fine(1e-3), M, (1.0 + 1e-12) / u_min, b, false, 0.0, 0);
    assert_eq!(out.outcome, Outcome::Escaped);
    let whole = 2.0 * out.phi_inf - std::f64::consts::PI;
    let exact = oracle::deflection(M, b).unwrap();
    assert!((whole - exact).abs() < 1e-5, "{whole} against {exact}");
}

#[test]
fn the_same_arguments_give_the_same_bits() {
    let run = || gr::trace(M, 40.0 * M, 7.0 * M, true, 0.9, 4);
    let (a, b) = (run(), run());
    assert_eq!(a, b);
    let bits = |t: &gr::Trace| {
        t.crossings.iter().flat_map(|c| [c.0.to_bits(), c.1.to_bits()]).chain([t.phi_inf.to_bits()]).collect::<Vec<_>>()
    };
    assert_eq!(bits(&a), bits(&b));
    let single = gr::f32::trace(1.0, 40.0, 7.0, true, 0.9, 4);
    assert_eq!(single, gr::f32::trace(1.0, 40.0, 7.0, true, 0.9, 4));
}

#[test]
fn a_ray_far_from_the_hole_crosses_the_disc_where_a_straight_line_would() {
    // from r_obs = 1000 M with b = 300 M the deflection is 1e-2 rad: the crossings are those of the straight line
    // u = sin(phi + delta) / b to the order M / b
    let (r_obs, b, phi0) = (1000.0 * M, 300.0 * M, 0.6);
    let delta = (b / r_obs).asin();
    let traced = gr::trace(M, r_obs, b, true, phi0, 3);
    assert_eq!(traced.outcome, Outcome::Escaped);
    // the straight line sweeps pi - delta in all: only the first of the three planes is in front of it
    assert_eq!(traced.crossings.len(), 1);
    for (phi, r) in &traced.crossings {
        let straight = b / (phi + delta).sin();
        assert!((r / straight - 1.0).abs() < 0.03, "phi {phi}: r {r} against {straight}");
    }
    // the crossings are where the plane is: phi0 + k pi, at phi > 0, in order
    for (k, (phi, _)) in traced.crossings.iter().enumerate() {
        assert!((phi - (phi0 + k as f64 * std::f64::consts::PI)).abs() < 1e-12);
    }
    // a ray that is captured crosses the plane only outside the horizon
    let captured = gr::trace(M, 30.0 * M, 4.0 * M, true, 0.3, 4);
    assert_eq!(captured.outcome, Outcome::Captured);
    assert_eq!(captured.phi_inf, 0.0);
    assert!(captured.crossings.iter().all(|(_, r)| *r > 2.0 * M));
}

#[test]
fn single_precision_follows_double_precision() {
    for (r_obs, b, ingoing) in [(40.0, 7.0, true), (60.0, 15.0, true), (25.0, 9.0, false), (50.0, 5.0, true)] {
        let double = gr::trace(M, r_obs, b, ingoing, 0.7, 4);
        let single = gr::f32::trace(1.0, r_obs as f32, b as f32, ingoing, 0.7, 4);
        println!(
            "SINGLE b = {b}: phi_inf {:e} relative, crossings {:?}",
            ((single.phi_inf as f64 - double.phi_inf) / double.phi_inf.max(1.0)).abs(),
            single
                .crossings
                .iter()
                .zip(&double.crossings)
                .map(|(s, d)| (s.1 as f64 / d.1 - 1.0).abs())
                .collect::<Vec<_>>()
        );
        assert_eq!(single.outcome, double.outcome, "b = {b}");
        assert_eq!(single.crossings.len(), double.crossings.len(), "b = {b}");
        assert!((single.phi_inf as f64 - double.phi_inf).abs() <= 1e-4 * double.phi_inf.max(1.0), "b = {b}");
        for (s, d) in single.crossings.iter().zip(&double.crossings) {
            assert!((s.1 as f64 / d.1 - 1.0).abs() < 1e-4, "b = {b}: {s:?} {d:?}");
        }
    }
}

#[test]
fn the_impact_parameter_of_a_static_observer_and_the_shadow_it_sees() {
    // at a great distance the angle of the shadow's edge is b_c / r
    let r_obs = 1e6 * M;
    let alpha = (oracle::shadow_radius(M) / r_obs).asin();
    let b = gr::impact_parameter(M, r_obs, alpha);
    assert!((b / oracle::shadow_radius(M) - 1.0).abs() < 1e-5, "{b}");
    // a static observer at the photon sphere sees light from every outgoing direction fall back
    assert_eq!(gr::impact_parameter(M, 10.0 * M, 0.0), 0.0);
}

#[test]
fn the_closed_forms_of_the_orbits_and_the_disc() {
    assert_eq!(oracle::isco(M), 6.0 * M);
    assert_eq!(oracle::isco(2.5), 15.0);
    assert!((oracle::kepler_omega(M, 6.0 * M) - (1.0f64 / 216.0).sqrt()).abs() < 1e-16);
    assert_eq!(oracle::horizon(M), 2.0 * M);
    assert_eq!(oracle::photon_sphere(M), 3.0 * M);
    // the temperature of the disc is zero at the inner edge, peaks at 49/36 of it, and falls as r^(-3/4) far out
    let r_in = 6.0 * M;
    assert_eq!(oracle::disc_temperature(r_in, r_in, 1.0), 0.0);
    assert_eq!(oracle::disc_temperature(0.5 * r_in, r_in, 1.0), 0.0);
    let peak = r_in * 49.0 / 36.0;
    for factor in [0.9, 0.99, 1.01, 1.1] {
        assert!(oracle::disc_temperature(peak, r_in, 1.0) > oracle::disc_temperature(peak * factor, r_in, 1.0));
    }
    let far = |r: f64| oracle::disc_temperature(r, r_in, 3.0);
    assert!((far(1e6) / far(2e6) - 2f64.powf(0.75)).abs() < 1e-3);
    // the turning point of a ray is at r_min = b for a great b, and at 3M for the critical one
    assert!((1.0 / oracle::turning_point(M, 1e4 * M).unwrap() / (1e4 * M) - 1.0).abs() < 1e-3);
    assert!(oracle::turning_point(M, oracle::shadow_radius(M) * (1.0 + 1e-9)).unwrap() * 3.0 * M < 1.0 + 1e-3);
    assert!(oracle::turning_point(M, oracle::shadow_radius(M) * 0.999).is_none());
}
