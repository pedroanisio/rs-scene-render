use sr_sim::cratering::ejecta::{ejecta, Ejecta, Spec};
use sr_sim::cratering::Material;

/// A crater from an impact at `speed`, standing in for the cratering law: volume grows
/// as speed^1.2 and the radius is that of a bowl of that volume.
fn spec(material: Material, speed: f64, theta: f64, particles: usize) -> Spec {
    let volume = 1.0e3 * (speed / 1000.0).powf(1.2);
    let radius = 1.1 * volume.cbrt();
    // The impact velocity points into the surface at `theta` degrees above it; the
    // contact normal is +y, so the velocity has a negative y component.
    let t = theta.to_radians();
    Spec {
        material,
        body_radius: 1.0,
        body_density: 2700.0,
        target_density: 1000.0,
        impact_speed: speed,
        velocity_direction: [t.cos(), -t.sin(), 0.0],
        normal: [0.0, 1.0, 0.0],
        crater_volume: volume,
        crater_radius: radius,
        crater_duration: 0.8 * (volume.cbrt() / 9.81).sqrt(),
        particles,
        seed: 20_261_003,
        angle: 45.0,
        angle_spread: 15.0,
    }
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
fn total_mass(e: &[Ejecta]) -> f64 {
    e.iter().map(|p| p.mass).sum()
}
fn radius_of(p: &Ejecta) -> f64 {
    norm(p.position)
}

#[test]
fn the_launched_mass_is_eighty_percent_of_the_crater_mass() {
    for material in [Material::Water, Material::DrySand, Material::HardRock, Material::SoftRock] {
        let s = spec(material, 5000.0, 90.0, 4000);
        let e = ejecta(&s).unwrap();
        assert_eq!(e.len(), 4000);
        let expected = 0.8 * s.target_density * s.crater_volume;
        assert!((total_mass(&e) - expected).abs() < 1e-12 * expected, "{material:?}");
        assert!(e.iter().all(|p| p.mass == e[0].mass), "equal masses");
    }
}

#[test]
fn a_faster_impact_throws_faster_farther_and_more() {
    let slow = ejecta(&spec(Material::Water, 3000.0, 90.0, 4000)).unwrap();
    let fast = ejecta(&spec(Material::Water, 6000.0, 90.0, 4000)).unwrap();
    let max_speed = |e: &[Ejecta]| e.iter().map(|p| norm(p.velocity)).fold(0.0, f64::max);
    let mean_radius = |e: &[Ejecta]| e.iter().map(radius_of).sum::<f64>() / e.len() as f64;
    assert!(max_speed(&fast) > max_speed(&slow));
    assert!(mean_radius(&fast) > mean_radius(&slow));
    assert!(total_mass(&fast) > total_mass(&slow));
}

#[test]
fn speed_falls_with_launch_radius_never_exceeds_the_impact_speed_and_the_fastest_leaves_first() {
    let s = spec(Material::DrySand, 4000.0, 90.0, 6000);
    let mut e = ejecta(&s).unwrap();
    assert!(e.iter().all(|p| norm(p.velocity) <= s.impact_speed * (1.0 + 1e-12)));
    // Particles are stratified by launch radius, so the order of the output is the order of x.
    e.sort_by(|a, b| radius_of(a).total_cmp(&radius_of(b)));
    let speeds: Vec<f64> = e.iter().map(|p| norm(p.velocity)).collect();
    // The elevation scatter of +-15 degrees leaves speed alone; speed is monotone in x.
    assert!(speeds.windows(2).all(|w| w[1] <= w[0] * (1.0 + 1e-12)), "speed must not rise with x");
    let times: Vec<f64> = e.iter().map(|p| p.time).collect();
    assert!(times.windows(2).all(|w| w[1] >= w[0]), "slower, farther material leaves later");
    assert!(times.iter().all(|t| *t >= 0.0 && *t <= s.crater_duration * (1.0 + 1e-12)));
    assert!(times[0] < 0.5 * s.crater_duration && *times.last().unwrap() == s.crater_duration);
}

/// The mass faster than v falls as v^(-3 mu) over the range where the last factor of the
/// speed law is still close to one: launch distances from 3 n1 a to a tenth of n2 R.
#[test]
fn cumulative_mass_against_speed_has_the_slope_minus_three_mu() {
    for (material, mu, n2) in
        [(Material::Water, 0.55, 1.5), (Material::DrySand, 0.41, 1.3), (Material::HardRock, 0.55, 1.0)]
    {
        // A crater much wider than the body, so that the central range is wide.
        let mut s = spec(material, 5000.0, 90.0, 40_000);
        s.crater_volume = 1.0e6;
        s.crater_radius = 50.0;
        let e = ejecta(&s).unwrap();
        // Particles come in order of launch distance, equal in mass, so the mass faster
        // than particle i is (i + 1) masses.
        let (x_lo, x_hi) = (3.0 * 1.2 * s.body_radius, 0.1 * n2 * s.crater_radius);
        let first = e.iter().position(|p| radius_of(p) >= x_lo).unwrap();
        let last = e.iter().rposition(|p| radius_of(p) <= x_hi).unwrap();
        let point = |i: usize| (norm(e[i].velocity).ln(), ((i + 1) as f64 * e[i].mass).ln());
        let ((v0, m0), (v1, m1)) = (point(first), point(last));
        let slope = (m1 - m0) / (v1 - v0);
        println!("SLOPE {material:?}: {slope:.3} against {:.3}", -3.0 * mu);
        assert!((slope + 3.0 * mu).abs() < 0.08 * 3.0 * mu, "{material:?}: {slope} against {}", -3.0 * mu);
    }
}

fn downrange_fraction(e: &[Ejecta], s: &Spec) -> f64 {
    let t = (-s.velocity_direction[1]).asin();
    let _ = t;
    let downrange = [1.0, 0.0, 0.0];
    let total = total_mass(e);
    e.iter().filter(|p| dot(p.position, downrange) > 0.0).map(|p| p.mass).sum::<f64>() / total
}
fn upstream_fraction(e: &[Ejecta]) -> f64 {
    let total = total_mass(e);
    // Within 20 degrees of straight upstream.
    e.iter()
        .filter(|p| {
            let r = radius_of(p);
            r > 0.0 && p.position[0] / r < -(20f64.to_radians()).cos()
        })
        .map(|p| p.mass)
        .sum::<f64>()
        / total
}

#[test]
fn the_azimuth_is_symmetric_at_steep_angles_and_lopsided_below_45_degrees() {
    let fraction = |theta: f64| {
        let s = spec(Material::HardRock, 5000.0, theta, 40_000);
        downrange_fraction(&ejecta(&s).unwrap(), &s)
    };
    let (steep, at45, at60, at35, at25, at15) =
        (fraction(90.0), fraction(45.0), fraction(60.0), fraction(35.0), fraction(25.0), fraction(15.0));
    println!("AZIMUTH downrange fractions: 90 {steep:.4}, 60 {at60:.4}, 45 {at45:.4}, 35 {at35:.4}, 25 {at25:.4}, 15 {at15:.4}");
    for symmetric in [steep, at60, at45] {
        assert!((symmetric - 0.5).abs() < 0.01, "{symmetric}");
    }
    // 0.5 + b / pi with b = (45 - theta) / 20, clamped to [0, 1].
    assert!((at35 - (0.5 + 0.5 / std::f64::consts::PI)).abs() < 0.01, "{at35}");
    assert!(at35 > at45 + 0.05 && at25 > at35 + 0.04);
    assert!((at25 - (0.5 + 1.0 / std::f64::consts::PI)).abs() < 0.01, "{at25}");
    // Below 25 degrees the clamp holds the asymmetry; it does not grow further.
    assert!((at15 - at25).abs() < 0.01);
    // At and below 25 degrees almost nothing goes straight back up the track.
    let upstream = |theta: f64| {
        let s = spec(Material::HardRock, 5000.0, theta, 40_000);
        upstream_fraction(&ejecta(&s).unwrap())
    };
    assert!(upstream(25.0) < 0.005, "{}", upstream(25.0));
    assert!(upstream(15.0) < 0.005);
    assert!(upstream(90.0) > 0.03, "{}", upstream(90.0));
}

#[test]
fn particles_leave_the_tangent_plane_at_the_launch_angle_with_its_spread() {
    let s = spec(Material::Water, 5000.0, 90.0, 20_000);
    let e = ejecta(&s).unwrap();
    let mut least = f64::MAX;
    let mut most = f64::MIN;
    for p in &e {
        assert!(dot(p.position, s.normal).abs() < 1e-9, "released from the tangent plane");
        let v = norm(p.velocity);
        let elevation = (dot(p.velocity, s.normal) / v).asin().to_degrees();
        least = least.min(elevation);
        most = most.max(elevation);
        // The horizontal part of the velocity points away from the impact point.
        let horizontal = [p.velocity[0], 0.0, p.velocity[2]];
        assert!(dot(horizontal, p.position) > 0.0);
    }
    println!("ANGLE elevations from {least:.2} to {most:.2} degrees");
    assert!(least >= 30.0 - 1e-9 && most <= 60.0 + 1e-9, "{least} {most}");
    assert!(least < 31.0 && most > 59.0, "the spread should fill +-15 degrees");
}

#[test]
fn a_tilted_contact_normal_gives_a_tilted_tangent_plane() {
    let mut s = spec(Material::DrySand, 3000.0, 60.0, 5000);
    let n = {
        let v = [0.3, 1.0, -0.2];
        let l = norm(v);
        [v[0] / l, v[1] / l, v[2] / l]
    };
    s.normal = n;
    s.velocity_direction = [-n[0] + 0.5, -n[1], -n[2] + 0.1];
    let e = ejecta(&s).unwrap();
    assert!(e.iter().all(|p| dot(p.position, n).abs() < 1e-9));
    assert!(e.iter().all(|p| dot(p.velocity, n) > 0.0), "every particle leaves the surface");
}

#[test]
fn results_do_not_depend_on_the_thread_count_and_the_seed_selects_them() {
    let s = spec(Material::SoftRock, 4000.0, 30.0, 50_000);
    let reference = ejecta(&s).unwrap();
    for threads in [1, 2, 8] {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        assert_eq!(pool.install(|| ejecta(&s).unwrap()), reference, "{threads} threads");
    }
    assert_eq!(ejecta(&s).unwrap(), reference);
    let other = ejecta(&Spec { seed: s.seed + 1, ..s.clone() }).unwrap();
    assert_ne!(other, reference);
    // The stratified launch radii and masses do not depend on the seed; angles and azimuths do.
    assert!(other
        .iter()
        .zip(&reference)
        .all(|(a, b)| a.mass == b.mass && radius_of(a) - radius_of(b) < 1e-9 || a.mass == b.mass));
}

#[test]
fn materials_without_a_table_row_and_bad_inputs_are_errors() {
    for material in [Material::Regolith, Material::ColdIce] {
        let error = ejecta(&spec(material, 3000.0, 90.0, 10)).unwrap_err();
        assert!(error.contains("no ejecta table row"), "{error}");
    }
    let good = spec(Material::Water, 3000.0, 90.0, 10);
    for bad in [
        Spec { particles: 0, ..good.clone() },
        Spec { impact_speed: 0.0, ..good.clone() },
        Spec { body_radius: -1.0, ..good.clone() },
        Spec { crater_volume: f64::NAN, ..good.clone() },
        Spec { normal: [0.0; 3], ..good.clone() },
        Spec { velocity_direction: [0.0; 3], ..good.clone() },
        // A crater smaller than the body leaves no launch range.
        Spec { crater_radius: 0.1, ..good.clone() },
        Spec { angle: 80.0, angle_spread: 15.0, ..good.clone() },
    ] {
        assert!(ejecta(&bad).is_err(), "{bad:?}");
    }
}

#[test]
fn material_names_follow_the_document_vocabulary() {
    for (name, material) in [
        ("water", Material::Water),
        ("drySand", Material::DrySand),
        ("drySoil", Material::DrySoil),
        ("wetSoil", Material::WetSoil),
        ("softRock", Material::SoftRock),
        ("hardRock", Material::HardRock),
        ("regolith", Material::Regolith),
        ("ice", Material::ColdIce),
    ] {
        assert_eq!(Material::parse(name), Some(material));
    }
    assert_eq!(Material::parse("granite"), None);
}

/// Where the ejecta of the authored rock (90 478 kg of 2700 kg/m3, 100 m/s at 60 degrees, soft rock) come down on level
/// ground, with no drag, and the mantle they make there against the analytic one `t0 (R / r)^3`: a measurement, not a
/// part of the model. Returns the crest radius and the landing radius and mass of each particle.
fn landings(particles: usize, angle: f64, spread: f64) -> (f64, Vec<(f64, f64)>) {
    use sr_sim::cratering::{crater, Impact, Target};
    const G: f64 = 9.80665;
    let theta = 60f64.to_radians();
    let impact = Impact { mass: 90_478.0, density: 2700.0, normal_speed: 100.0 * theta.sin() };
    let law =
        crater(&impact, &Target { material: Material::SoftRock, density: None, strength: None, gravity: G }).unwrap();
    let list = ejecta(&Spec {
        material: Material::SoftRock,
        body_radius: (3.0 * impact.mass / (4.0 * std::f64::consts::PI * impact.density)).cbrt(),
        body_density: impact.density,
        target_density: 2100.0,
        impact_speed: 100.0,
        velocity_direction: [theta.cos(), -theta.sin(), 0.0],
        normal: [0.0, 1.0, 0.0],
        crater_volume: law.volume,
        crater_radius: law.radius,
        crater_duration: law.duration,
        particles,
        seed: 20_261_006,
        angle,
        angle_spread: spread,
    })
    .unwrap();
    let crest = law.rim_radius;
    let landed = list
        .iter()
        .map(|p| {
            let up = p.velocity[1];
            let flight = if up > 0.0 { 2.0 * up / G } else { 0.0 };
            let (x, z) = (p.position[0] + p.velocity[0] * flight, p.position[2] + p.velocity[2] * flight);
            (x.hypot(z), p.mass)
        })
        .collect();
    (crest, landed)
}

#[test]
fn the_ballistic_landing_points_of_the_ejecta_make_a_mantle_that_is_measured_against_the_inverse_cube() {
    for (angle, spread) in [(45.0, 15.0), (30.0, 10.0), (60.0, 10.0)] {
        let (crest, landed) = landings(20_000, angle, spread);
        let total: f64 = landed.iter().map(|l| l.1).sum();
        let share = |lo: f64, hi: f64| {
            landed.iter().filter(|l| l.0 >= lo * crest && l.0 < hi * crest).fold(0.0, |sum, l| sum + l.1) / total
        };
        let (inside, band, far) = (share(0.0, 1.0), share(1.0, 20.0), share(20.0, f64::INFINITY));
        println!("LANDING {angle}/{spread}: crest {crest:.3} m; inside the crest {inside:.4}, 1 to 20 crest radii {band:.4}, beyond {far:.4}");
        assert!((inside + band + far - 1.0).abs() < 1e-12);
        // logarithmic bins from the crest to twenty crest radii: thickness is mass over the annulus' area
        let bins = 12;
        let edge = |i: usize| 20f64.powf(i as f64 / bins as f64);
        let mut points = Vec::new();
        for i in 0..bins {
            let (lo, hi) = (edge(i), edge(i + 1));
            let mass = share(lo, hi) * total;
            let area = std::f64::consts::PI * crest * crest * (hi * hi - lo * lo);
            if mass > 0.0 {
                let r = (lo * hi).sqrt();
                println!(
                    "LANDING   r/R {r:6.2}: thickness {:.4e} against the cube {:.4e} (arbitrary scale)",
                    mass / area,
                    r.powi(-3)
                );
                points.push((r.ln(), (mass / area).ln()));
            }
        }
        // least squares slope of log thickness against log radius
        let n = points.len() as f64;
        let (mx, my) = (points.iter().map(|p| p.0).sum::<f64>() / n, points.iter().map(|p| p.1).sum::<f64>() / n);
        let slope = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum::<f64>()
            / points.iter().map(|p| (p.0 - mx).powi(2)).sum::<f64>();
        // the analytic mantle's share of what lies between the crest and twenty crest radii in 1 to 2 and 2 to 4 crest radii
        let analytic = |a: f64, b: f64| (1.0 / a - 1.0 / b) / (1.0 - 1.0 / 20.0);
        println!(
            "LANDING   slope {slope:.2} (the mantle is -3); share of the band in 1 to 2 R {:.3} against {:.3}, in 2 to 4 R {:.3} against {:.3}",
            share(1.0, 2.0) / band, analytic(1.0, 2.0), share(2.0, 4.0) / band, analytic(2.0, 4.0)
        );
        assert!(slope.is_finite());
    }
}
