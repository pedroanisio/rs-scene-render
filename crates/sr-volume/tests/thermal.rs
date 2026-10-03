use std::sync::Arc;

use sr_volume::medium::{integrate, Bounds, March, Medium, Optical, Ray};
use sr_volume::thermal::{blackbody_rgb, spectral_radiance};
use sr_volume::{SparseGrid, Transform};

#[test]
fn planck_radiance_has_physical_units_and_temperature_dependence() {
    // Planck's law, SI constants, independently evaluated at 500 nm and 5800 K.
    let b = spectral_radiance(500.0, 5800.0).unwrap();
    assert!((b / 2.688219962592932e13 - 1.0).abs() < 1e-10, "{b}");
    assert_eq!(spectral_radiance(500.0, 0.0).unwrap(), 0.0);
    for t in [-1.0, f64::INFINITY, f64::NAN, 50001.0] {
        assert!(blackbody_rgb(t).is_err());
    }
    for wavelength in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(spectral_radiance(wavelength, 5000.0).is_err());
    }
}

#[test]
fn visible_blackbody_is_dark_when_cold_and_preserves_brightness_growth() {
    assert_eq!(blackbody_rgb(0.0).unwrap(), [0.0; 3]);
    assert!(blackbody_rgb(300.0).unwrap().into_iter().all(|v| v < 1e-20));
    let warm = blackbody_rgb(2000.0).unwrap();
    let hot = blackbody_rgb(6500.0).unwrap();
    assert!(warm[0] > 3.0 * warm[1] && warm[1] > warm[2], "{warm:?}");
    let y = 0.2126 * hot[0] + 0.7152 * hot[1] + 0.0722 * hot[2];
    assert!((y - 1.0).abs() < 0.001, "{hot:?}");
    // Independent Planckian-locus chromaticity reference, with tolerance for CIE fits.
    let xyz =
        [0.4124 * hot[0] + 0.3576 * hot[1] + 0.1805 * hot[2], y, 0.0193 * hot[0] + 0.1192 * hot[1] + 0.9505 * hot[2]];
    let sum: f64 = xyz.iter().sum();
    assert!((xyz[0] / sum - 0.3135).abs() < 0.003);
    assert!((xyz[1] / sum - 0.3237).abs() < 0.003);
    let hotter = blackbody_rgb(13000.0).unwrap();
    assert!(hotter.into_iter().zip(hot).all(|(a, b)| a > 4.0 * b));
}

fn grid(background: f32) -> Arc<SparseGrid> {
    Arc::new(SparseGrid::new(Transform::identity(), background, 8).unwrap())
}

#[test]
fn temperature_is_sampled_in_its_own_space_before_emission_and_density() {
    let domain = Bounds::new([-1.0; 3], [1.0; 3]).unwrap();
    let object =
        Transform::new(glam::DMat4::from_translation(glam::DVec3::new(10.0, 0.0, 0.0)).to_cols_array()).unwrap();
    let base = Medium::new(
        grid(2.0),
        Some(domain),
        object,
        Optical { extinction: 0.0, emission: [0.1; 3], ..Default::default() },
    )
    .unwrap();
    let transform = Transform::new(glam::DMat4::from_scale(glam::DVec3::splat(2.0)).to_cols_array()).unwrap();
    let mut temp = SparseGrid::new(transform, 0.0, 8).unwrap();
    temp.set([0, 0, 0], 4000.0).unwrap();
    temp.set([1, 0, 0], 8000.0).unwrap();
    let medium = base.clone().with_temperature(Arc::new(temp), 1.0, 0.5).unwrap();
    let expected = blackbody_rgb(6000.0).unwrap().map(|v| 0.1 + v * 0.5);
    let actual = medium.sample_emission([11.0, 0.0, 0.0]);
    for c in 0..3 {
        assert!((actual[c] - expected[c]).abs() < 1e-12);
    }
    let homogeneous = base.with_temperature(grid(3000.0), 2.0, 0.5).unwrap();
    let result = integrate(
        &[homogeneous],
        Ray::new([10.0, 0.0, -2.0], [0.0, 0.0, 1.0], 4.0).unwrap(),
        March::default(),
        |_, _, _| [0.0; 3],
    )
    .unwrap();
    for (actual, expected) in result.radiance.into_iter().zip(expected) {
        assert!((actual - expected * 4.0).abs() < 1e-12);
    }
}

#[test]
fn temperature_fields_and_scales_are_validated_before_transport() {
    let base = Medium::new(
        grid(1.0),
        Some(Bounds::new([0.0; 3], [1.0; 3]).unwrap()),
        Transform::identity(),
        Optical::default(),
    )
    .unwrap();
    for (t, scale, strength) in [
        (-1.0, 1.0, 1.0),
        (25001.0, 2.0, 1.0),
        (5000.0, 0.0, 1.0),
        (5000.0, f64::NAN, 1.0),
        (5000.0, 1.0, -1.0),
        (5000.0, 1.0, f64::INFINITY),
    ] {
        assert!(base.clone().with_temperature(grid(t), scale, strength).is_err());
    }
    let mut negative = SparseGrid::new(Transform::identity(), 0.0, 1).unwrap();
    negative.set([0, 0, 0], -1.0).unwrap();
    assert!(base.with_temperature(Arc::new(negative), 1.0, 1.0).is_err());
}

#[test]
fn bounded_thermal_table_tracks_direct_spectral_integration() {
    use sr_volume::thermal::{blackbody_table, MAX_KELVIN, TABLE_INTERVALS};
    let table = blackbody_table();
    for kelvin in (0..=50000).step_by(137).chain([50000]) {
        let t = f64::from(kelvin);
        let x = (t / MAX_KELVIN).sqrt() * TABLE_INTERVALS as f64;
        let lo = (x as usize).min(TABLE_INTERVALS - 1);
        let f = x - lo as f64;
        let exact = blackbody_rgb(t).unwrap();
        for c in 0..3 {
            let interpolated = table[lo][c] * (1.0 - f) + table[lo + 1][c] * f;
            assert!((interpolated - exact[c]).abs() <= 1e-7 + 0.005 * exact[c], "{kelvin}K, channel {c}");
        }
    }
}
