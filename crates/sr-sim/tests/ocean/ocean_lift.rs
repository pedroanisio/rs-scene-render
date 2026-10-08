//! The depth response of the surface to a displacement of its fluid: Kajiura's 1/cosh(kh) for the
//! bed, cosh(k z0)/cosh(kh) for a volume at height z0, checked against numerical integration of the
//! same transforms, and what the engine adds: conservation, dry columns, the edges of the domain.
use sr_sim::ocean::lift::depth_response;

fn j0(x: f64) -> f64 {
    // the power series, good for the arguments used here (below 12)
    let (mut term, mut sum) = (1.0, 1.0);
    for m in 1..60 {
        term *= -(x * x / 4.0) / (m * m) as f64;
        sum += term;
    }
    sum
}

/// `integral_0^inf k R(k) F(k) dk` for a radial `profile(r)` up to radius `edge`, where
/// `F(k) = integral profile(r) J0(k r) r dr` and `R` the response: the surface height at the
/// centre, per the transform of a radial function.
fn centre(profile: &dyn Fn(f64) -> f64, edge: f64, response: &dyn Fn(f64) -> f64, k_max: f64) -> f64 {
    let (nk, nr) = (4000, 800);
    let mut total = 0.0;
    for i in 0..nk {
        let k = (i as f64 + 0.5) * k_max / nk as f64;
        let mut f = 0.0;
        for j in 0..nr {
            let r = (j as f64 + 0.5) * edge / nr as f64;
            f += profile(r) * j0(k * r) * r * edge / nr as f64;
        }
        total += k * response(k) * f * k_max / nk as f64;
    }
    total
}

fn sphere_thickness(a: f64) -> impl Fn(f64) -> f64 {
    move |r| if r < a { 2.0 * (a * a - r * r).sqrt() } else { 0.0 }
}

/// A radial profile on a grid of 0.5-unit cells, centred on a cell centre.
fn grid(n: usize, cell: f64, profile: &dyn Fn(f64) -> f64) -> Vec<f64> {
    let c = (n / 2) as f64 + 0.5;
    (0..n * n)
        .map(|i| {
            let (x, z) = (((i % n) as f64 + 0.5 - c) * cell, ((i / n) as f64 + 0.5 - c) * cell);
            profile(x.hypot(z))
        })
        .collect()
}

fn filtered(n: usize, cell: f64, depth: f64, height: f64, field: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; n * n];
    assert!(depth_response([n, n], cell, depth, height, field, &mut out, &|_| true).unwrap());
    out
}

#[test]
fn a_sphere_on_the_bed_of_deep_water_raises_the_surface_as_kajiura_says() {
    let (n, cell, depth, a) = (320, 0.5, 20.0, 2.0);
    // the cells are about the same size as the column spacing the integral would need, so the
    // sampled sphere has a little less volume than the real one: compare with the sampled volume
    let field = grid(n, cell, &sphere_thickness(a));
    let out = filtered(n, cell, depth, 0.0, &field);
    let middle = (n / 2) * n + n / 2;
    // the centre of the grid is a cell corner: the four cells around it are the centre
    let measured = (out[middle - 1] + out[middle] + out[middle - n - 1] + out[middle - n]) / 4.0;
    let theory = centre(&sphere_thickness(a), a, &|k| 1.0 / (k * depth).cosh(), 40.0 / depth);
    println!("LIFT sphere on the bed: {measured:.5} against {theory:.5}");
    assert!((theory - 0.0241).abs() < 0.0015, "the reference itself: {theory}");
    assert!((measured - theory).abs() < 0.06 * theory, "{measured} against {theory}");
    // the model's own long-wave answer would have been the thickness, 4 here
    assert!(measured < 0.05);
}

#[test]
fn a_volume_at_the_surface_is_not_attenuated_and_one_between_is() {
    let (n, cell, depth) = (256, 0.5, 20.0);
    let field = grid(n, cell, &sphere_thickness(2.0));
    let top = filtered(n, cell, depth, depth, &field);
    let worst = field.iter().zip(&top).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(worst < 1e-9, "z0 = h gives the response 1: {worst}");
    let middle = (n / 2) * n + n / 2;
    let heights: Vec<f64> =
        [0.0, 5.0, 10.0, 15.0, 20.0].iter().map(|&z| filtered(n, cell, depth, z, &field)[middle]).collect();
    assert!(heights.windows(2).all(|w| w[0] < w[1]), "higher volumes are attenuated less: {heights:?}");
    // the derivation's other check: against the integral for a volume at the middle depth
    let theory = centre(&sphere_thickness(2.0), 2.0, &|k| (k * 10.0).cosh() / (k * depth).cosh(), 40.0 / depth);
    let measured = heights[2];
    assert!((measured - theory).abs() < 0.08 * theory, "{measured} against {theory}");
}

#[test]
fn a_wide_shallow_displacement_is_nearly_what_it_was_and_a_narrow_deep_one_is_not() {
    let (n, cell) = (256, 0.5);
    let bump = |sigma: f64| move |r: f64| (-r * r / (2.0 * sigma * sigma)).exp();
    // 20 units across in 2 units of water: kh is small
    let wide = grid(n, cell, &bump(10.0));
    let out = filtered(n, cell, 2.0, 0.0, &wide);
    let worst = wide.iter().zip(&out).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(worst < 0.05, "a wide displacement in shallow water keeps its shape: {worst}");
    // 2 units across in 20 of water, against the integral of the Gaussian
    let narrow = grid(n, cell, &bump(2.0));
    let out = filtered(n, cell, 20.0, 0.0, &narrow);
    let middle = (n / 2) * n + n / 2;
    let measured = (out[middle - 1] + out[middle] + out[middle - n - 1] + out[middle - n]) / 4.0;
    let theory = centre(&bump(2.0), 14.0, &|k| 1.0 / (k * 20.0).cosh(), 40.0 / 20.0);
    assert!((measured - theory).abs() < 0.04 * theory, "{measured} against {theory}");
    assert!(measured < 0.05, "a bump of height 1 over water ten times as deep is 20 times lower: {measured}");
}

#[test]
fn what_is_displaced_is_what_is_lifted_with_dry_columns_edges_and_both_signs() {
    let (n, cell, depth) = (128, 0.5, 6.0);
    // near a corner of the domain, so that the window runs off it, with a dry strip
    let field: Vec<f64> = (0..n * n)
        .map(|i| {
            let (x, z) = ((i % n) as f64, (i / n) as f64);
            let r2 = (x - 6.0).powi(2) + (z - 5.0).powi(2);
            3.0 * (-r2 / 18.0).exp() - 1.0 * (-((x - 14.0).powi(2) + (z - 5.0).powi(2)) / 8.0).exp()
        })
        .collect();
    let wet = |c: usize| c % n >= 3 || c / n >= 3;
    let mut out = vec![0.0; n * n];
    depth_response([n, n], cell, depth, 1.0, &field, &mut out, &wet).unwrap();
    let (total_in, total_out): (f64, f64) = (field.iter().sum(), out.iter().sum());
    assert!((total_in - total_out).abs() < 1e-9 * total_in.abs(), "{total_in} against {total_out}");
    assert!(out.iter().enumerate().all(|(c, v)| wet(c) || *v == 0.0), "nothing lands on a dry column");
    // each sign keeps its own total
    let (positive, negative): (f64, f64) = field.iter().fold((0.0, 0.0), |a, v| (a.0 + v.max(0.0), a.1 + v.min(0.0)));
    assert!(positive > 0.0 && negative < 0.0);
}

#[test]
fn nothing_displaced_lifts_nothing_and_the_result_does_not_depend_on_the_threads() {
    let (n, cell) = (64, 1.0);
    let mut out = vec![0.0; n * n];
    assert!(depth_response([n, n], cell, 5.0, 0.0, &vec![0.0; n * n], &mut out, &|_| true).unwrap());
    assert!(out.iter().all(|v| *v == 0.0));
    let field = grid(n, cell, &|r: f64| (-r * r / 8.0).exp());
    let reference = filtered(n, cell, 5.0, 2.0, &field);
    for threads in [1, 2, 8] {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        let again = pool.install(|| filtered(n, cell, 5.0, 2.0, &field));
        assert_eq!(
            again.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            reference.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
    }
}

#[test]
fn bad_input_is_an_error_and_too_wide_a_displacement_is_left_alone() {
    let n = 8;
    let mut out = vec![0.0; n * n];
    assert!(depth_response([n, n], 1.0, 0.0, 0.0, &vec![1.0; n * n], &mut out, &|_| true).is_err());
    assert!(depth_response([n, n], 0.0, 1.0, 0.0, &vec![1.0; n * n], &mut out, &|_| true).is_err());
    assert!(depth_response([n, n], 1.0, 1.0, 0.0, &[1.0; 3], &mut out, &|_| true).is_err());
    // wider than a window: added as it is, and said so
    let wide = sr_sim::ocean::lift::MAX_WINDOW + 4;
    let field: Vec<f64> = (0..wide).map(|i| if i == 0 || i == wide - 1 { 1.0 } else { 0.0 }).collect();
    let mut out = vec![0.0; wide];
    assert!(!depth_response([wide, 1], 1.0, 1.0, 0.0, &field, &mut out, &|_| true).unwrap());
    assert_eq!(out, field);
}

#[test]
fn a_depth_that_no_window_can_hold_is_an_error_and_never_an_overflow() {
    let n = 8;
    let field = vec![1.0; n * n];
    let window = sr_sim::ocean::lift::MAX_WINDOW as f64;
    for depth in [window * 1.5, 1e9, 1e300, f64::MAX] {
        let mut out = vec![0.0; n * n];
        let dense = depth_response([n, n], 1.0, depth, 0.0, &field, &mut out, &|_| true);
        assert!(matches!(dense, Err(sr_sim::ocean::Error::Invalid(_))), "depth {depth}: {dense:?}");
        let sparse = sr_sim::ocean::lift::depth_response_sparse([n, n], 1.0, depth, 0.0, &field, &|_| true);
        assert!(matches!(sparse, Err(sr_sim::ocean::Error::Invalid(_))), "depth {depth}: {sparse:?}");
    }
    // the deepest water a window holds, in cells, is accepted
    let mut out = vec![0.0; n * n];
    assert!(depth_response([n, n], 1.0, window, 0.0, &field, &mut out, &|_| true).is_ok());
}
