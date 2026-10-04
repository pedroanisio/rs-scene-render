//! The crater an impact makes, from Holsapple's pi-group scaling law, in SI units.

use sr_sim::cratering::{crater, Impact, Material, Target};

const G: f64 = 9.80665;

fn impact(mass: f64, density: f64, speed: f64) -> Impact {
    Impact { mass, density, normal_speed: speed }
}

fn target(material: Material) -> Target {
    Target { material, density: None, strength: None, gravity: G }
}

/// Radius of the sphere of `mass` and `density`.
fn radius(mass: f64, density: f64) -> f64 {
    (3.0 * mass / (4.0 * std::f64::consts::PI * density)).cbrt()
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance * b.abs()
}

const ALL: [Material; 8] = [
    Material::Water,
    Material::DrySand,
    Material::DrySoil,
    Material::WetSoil,
    Material::SoftRock,
    Material::HardRock,
    Material::Regolith,
    Material::ColdIce,
];

#[test]
fn water_in_the_gravity_regime_follows_the_published_efficiency() {
    // rho V / m = 0.98 (g a / U^2)^(-3 mu / (2 + mu)) for water, mu = 0.55, at equal densities
    let (mass, density, speed) = (500.0, 1000.0, 3000.0);
    let a = radius(mass, density);
    let c = crater(&impact(mass, density, speed), &target(Material::Water)).unwrap();
    let pi2 = G * a / (speed * speed);
    let want = 0.98 * pi2.powf(-3.0 * 0.55 / 2.55) * mass / 1000.0;
    assert!(close(c.volume, want, 1e-9), "{} vs {want}", c.volume);
    // a water crater is nearly a hemisphere: Kr 0.8, Kd 0.75
    assert!(close(c.radius, 0.8 * want.cbrt(), 1e-9));
    assert!(close(c.depth, 0.75 * want.cbrt(), 1e-9));
}

#[test]
fn dry_sand_has_no_strength_and_follows_its_own_exponent() {
    let (mass, density, speed) = (20.0, 2700.0, 5000.0);
    let a = radius(mass, density);
    let c = crater(&impact(mass, density, speed), &target(Material::DrySand)).unwrap();
    let pi2 = G * a / (speed * speed);
    let want = 0.132 * pi2.powf(-3.0 * 0.41 / 2.41) * (1700.0f64 / density).powf((2.0 + 0.41 - 6.0 * 0.33) / 2.41);
    assert!(close(c.volume * 1700.0 / mass, want, 1e-9), "{} vs {want}", c.volume * 1700.0 / mass);
}

#[test]
fn strength_takes_over_when_gravity_is_negligible() {
    // rho V / m = K1 (K2 Y / (rho U^2))^(-3 mu / 2) times the density ratio factor, for g -> 0
    let (mass, density, speed) = (2.0, 2700.0, 4000.0);
    let t = Target { gravity: 1e-9, ..target(Material::HardRock) };
    let c = crater(&impact(mass, density, speed), &t).unwrap();
    let (rho, y) = (3200.0, 1.0e7);
    let pi3 = y / (rho * speed * speed);
    let want =
        0.095 * (0.257 * pi3 * (rho / density).powf((6.0 * 0.33 - 2.0) / (3.0 * 0.55))).powf(-3.0 * 0.55 / 2.0) * mass
            / rho;
    assert!(close(c.volume, want, 1e-6), "{} vs {want}", c.volume);
    // and a stronger target makes a smaller crater
    let stronger = Target { strength: Some(2.0e7), ..t };
    assert!(crater(&impact(mass, density, speed), &stronger).unwrap().volume < c.volume);
}

#[test]
fn every_dimension_grows_with_energy_and_shrinks_with_gravity_and_strength() {
    for material in ALL {
        let base = impact(100.0, 3000.0, 4000.0);
        let reference = crater(&base, &target(material)).unwrap();
        let grows = |bigger: &Impact, label: &str| {
            let c = crater(bigger, &target(material)).unwrap();
            for (name, a, b) in [
                ("volume", c.volume, reference.volume),
                ("radius", c.radius, reference.radius),
                ("depth", c.depth, reference.depth),
                ("rim radius", c.rim_radius, reference.rim_radius),
                ("rim height", c.rim_height, reference.rim_height),
                ("ejecta volume", c.ejecta_volume, reference.ejecta_volume),
            ] {
                assert!(a > b, "{material:?}: {label} must enlarge the {name}: {a} vs {b}");
            }
        };
        grows(&impact(200.0, 3000.0, 4000.0), "mass");
        grows(&impact(100.0, 3000.0, 6000.0), "speed");
        // energy at fixed speed and at fixed mass, together
        grows(&impact(400.0, 3000.0, 5000.0), "energy");
        let weaker = crater(&base, &Target { gravity: G / 4.0, ..target(material) }).unwrap();
        assert!(weaker.volume > reference.volume, "{material:?}: lower gravity");
        assert!(weaker.duration > reference.duration, "{material:?}: lower gravity is slower");
    }
}

#[test]
fn a_slower_normal_component_makes_a_smaller_crater() {
    for material in ALL {
        let mut last = f64::INFINITY;
        // the normal component of a 6 km/s impact at 90, 60, 45, 30 and 15 degrees from the surface
        for degrees in [90.0f64, 60.0, 45.0, 30.0, 15.0] {
            let speed = 6000.0 * degrees.to_radians().sin();
            let c = crater(&impact(100.0, 3000.0, speed), &target(material)).unwrap();
            assert!(c.radius < last, "{material:?} at {degrees} degrees");
            last = c.radius;
        }
    }
}

#[test]
fn strength_and_density_overrides_replace_the_table() {
    let i = impact(100.0, 3000.0, 4000.0);
    let table = crater(&i, &target(Material::SoftRock)).unwrap();
    let same = Target { density: Some(2100.0), strength: Some(1.0e6), ..target(Material::SoftRock) };
    assert!(close(crater(&i, &same).unwrap().volume, table.volume, 1e-12), "the table entry for soft rock");
    let weaker = Target { strength: Some(1.0e3), ..same };
    assert!(crater(&i, &weaker).unwrap().volume > table.volume);
    let denser = Target { density: Some(4000.0), ..same };
    assert!(crater(&i, &denser).unwrap().volume * 4000.0 != table.volume * 2100.0);
}

#[test]
fn the_crater_has_the_shape_the_literature_gives() {
    let c = crater(&impact(100.0, 3000.0, 4000.0), &target(Material::SoftRock)).unwrap();
    assert!(close(c.rim_radius, 1.3 * c.radius, 1e-12));
    assert!(close(c.rim_height, 0.036 * 2.0 * c.rim_radius, 1e-12));
    assert!(close(c.depth / c.radius, 0.6 / 1.1, 1e-12));
    assert!(close(c.ejecta_volume, 0.8 * c.volume, 1e-12));
    assert!(close(c.duration, 0.8 * (c.volume.cbrt() / G).sqrt(), 1e-12));
    // the blanket's thickness at the rim, with t = c (rim / r)^3, holds the ejecta volume
    let hold = 2.0 * std::f64::consts::PI * c.blanket_thickness * c.rim_radius.powi(2);
    assert!(close(hold, c.ejecta_volume, 1e-12), "{hold} vs {}", c.ejecta_volume);
}

#[test]
fn the_size_is_of_the_order_the_impact_effects_calculator_gives() {
    // A 40 m iron body at 20 km/s and 45 degrees into rock: Collins, Melosh & Marcus give a
    // transient crater near 1.3 km. This only catches a slip of units or of scale.
    let mass = 8000.0 * std::f64::consts::PI / 6.0 * 40.0f64.powi(3);
    let speed = 20_000.0 * 45.0f64.to_radians().sin();
    let c = crater(&impact(mass, 8000.0, speed), &target(Material::HardRock)).unwrap();
    let diameter = 2.0 * c.radius;
    assert!((300.0..5000.0).contains(&diameter), "a crater {diameter} m across for a 1.3 km reference");
}

#[test]
fn inputs_that_have_no_crater_are_errors() {
    let i = impact(100.0, 3000.0, 4000.0);
    for bad in [
        Target { gravity: 0.0, ..target(Material::HardRock) },
        Target { gravity: f64::NAN, ..target(Material::HardRock) },
        Target { gravity: -1.0, ..target(Material::HardRock) },
        Target { density: Some(0.0), ..target(Material::HardRock) },
        Target { strength: Some(-1.0), ..target(Material::HardRock) },
    ] {
        assert!(crater(&i, &bad).is_err(), "{bad:?}");
    }
    for bad in [
        impact(0.0, 3000.0, 4000.0),
        impact(100.0, 0.0, 4000.0),
        impact(100.0, 3000.0, 0.0),
        impact(f64::NAN, 3000.0, 4000.0),
    ] {
        assert!(crater(&bad, &target(Material::SoftRock)).is_err(), "{bad:?}");
    }
}

#[test]
fn materials_have_names() {
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
