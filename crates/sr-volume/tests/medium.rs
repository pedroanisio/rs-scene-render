use std::sync::Arc;

use glam::{DMat4, DVec3};
use sr_volume::medium::{integrate, phase_hg, Bounds, March, Medium, Optical, Ray};
use sr_volume::{SparseGrid, Transform};

fn slab(lo: f64, hi: f64, optical: Optical) -> Medium {
    Medium::new(
        Arc::new(SparseGrid::new(Transform::identity(), 1.0, 0).unwrap()),
        Some(Bounds::new([-1.0, -1.0, lo], [1.0, 1.0, hi]).unwrap()),
        Transform::identity(),
        optical,
    )
    .unwrap()
}

fn ray() -> Ray {
    Ray::new([0.0, 0.0, -5.0], [0.0, 0.0, 1.0], 20.0).unwrap()
}

fn dark(_: [f64; 3], _: [f64; 3], _: f64) -> [f64; 3] {
    [0.0; 3]
}

#[test]
fn homogeneous_slab_matches_beer_lambert_and_emission_integral() {
    let m = slab(0.0, 3.0, Optical { extinction: 0.7, emission: [2.0, 0.5, 0.0], ..Default::default() });
    for step_size in [0.125, 0.7, 20.0] {
        let r = integrate(std::slice::from_ref(&m), ray(), March { step_size, max_steps: 100 }, dark).unwrap();
        let t = (-2.1_f64).exp();
        assert!((r.transmittance - t).abs() < 1e-12);
        for (got, e) in r.radiance.into_iter().zip([2.0, 0.5, 0.0]) {
            assert!((got - e * (1.0 - t) / 0.7).abs() < 1e-12);
        }
    }
}

#[test]
fn vacuum_emission_uses_distance_and_surface_limit_clips_the_integral() {
    let m = slab(0.0, 3.0, Optical { extinction: 0.0, emission: [2.0, 0.0, 1.0], ..Default::default() });
    let clipped = Ray::new([0.0, 0.0, -5.0], [0.0, 0.0, 4.0], 6.0).unwrap();
    let r = integrate(&[m], clipped, March::default(), dark).unwrap();
    assert_eq!(r.transmittance, 1.0);
    assert_eq!(r.radiance, [2.0, 0.0, 1.0]);
}

#[test]
fn optically_thick_slabs_preserve_small_nonzero_transmittance() {
    let m = slab(0.0, 1.0, Optical { extinction: 40.0, ..Default::default() });
    let r = integrate(&[m], ray(), March { step_size: 2.0, max_steps: 10 }, dark).unwrap();
    assert!((r.transmittance / (-40.0_f64).exp() - 1.0).abs() < 1e-12);
}

#[test]
fn overlapping_media_combine_coefficients_independently_of_insertion_order() {
    let a = slab(0.0, 3.0, Optical { extinction: 1.0, emission: [2.0, 0.0, 0.0], ..Default::default() });
    let b = slab(1.0, 2.0, Optical { extinction: 2.0, emission: [0.0, 0.0, 3.0], ..Default::default() });
    let r = integrate(&[a.clone(), b.clone()], ray(), March::default(), dark).unwrap();
    let reversed = integrate(&[b, a], ray(), March::default(), dark).unwrap();
    assert_eq!(r, reversed);
    assert!((r.transmittance - (-5.0_f64).exp()).abs() < 1e-12);
    let e1 = (-1.0_f64).exp();
    let e3 = (-3.0_f64).exp();
    let red = 2.0 * (1.0 - e1) + e1 * (2.0 / 3.0) * (1.0 - e3) + e1 * e3 * 2.0 * (1.0 - e1);
    assert!((r.radiance[0] - red).abs() < 1e-12);
    assert!((r.radiance[2] - e1 * (1.0 - e3)).abs() < 1e-12);
}

#[test]
fn transformed_density_uses_world_distance_and_sparse_support() {
    let mut grid = SparseGrid::new(Transform::identity(), 0.0, 1).unwrap();
    grid.set([0, 0, 0], 1.0).unwrap();
    let transform = Transform::new(DMat4::from_scale(DVec3::new(1.0, 1.0, 3.0)).to_cols_array()).unwrap();
    let m = Medium::new(Arc::new(grid), None, transform, Optical::default()).unwrap();
    let r = integrate(&[m], ray(), March { step_size: 0.125, max_steps: 100 }, dark).unwrap();
    // One trilinear voxel is a triangular density profile of integral 1;
    // stretching its z axis by 3 multiplies its optical depth by 3.
    assert!((r.transmittance - (-3.0_f64).exp()).abs() < 1e-7);
}

#[test]
fn bounded_marching_rejects_insufficient_budget_instead_of_dropping_tail() {
    let a = slab(0.0, 1.0, Optical::default());
    let b = slab(10.0, 11.0, Optical::default());
    let r = integrate(&[a.clone(), b.clone()], ray(), March { step_size: 0.25, max_steps: 8 }, dark).unwrap();
    assert_eq!(r.steps, 8, "empty space between domains must not consume samples");
    assert!((r.transmittance - (-2.0_f64).exp()).abs() < 1e-12);
    assert!(integrate(&[a, b], ray(), March { step_size: 0.25, max_steps: 7 }, dark).is_err());
}

#[test]
fn single_scattering_has_correct_albedo_and_phase_normalization() {
    let m = slab(0.0, 2.0, Optical { albedo: [0.5, 0.25, 0.0], ..Default::default() });
    let r = integrate(&[m], ray(), March::default(), |_, _, _| [2.0; 3]).unwrap();
    let lost = 1.0 - (-2.0_f64).exp();
    assert!((r.radiance[0] - lost).abs() < 1e-12);
    assert!((r.radiance[1] - lost * 0.5).abs() < 1e-12);
    assert_eq!(r.radiance[2], 0.0);
    for g in [-0.8, 0.0, 0.8] {
        let integral: f64 = (0..20000).map(|i| phase_hg(-1.0 + (i as f64 + 0.5) / 10000.0, g)).sum::<f64>()
            * 2.0
            * std::f64::consts::PI
            / 10000.0;
        assert!((integral - 1.0).abs() < 1e-5);
    }
    assert!(phase_hg(1.0, 0.5) > phase_hg(-1.0, 0.5));
}

#[test]
fn invalid_domain_coefficients_and_density_fail_before_transport() {
    assert!(Bounds::new([0.0; 3], [0.0; 3]).is_err());
    assert!(Bounds::new([f64::NAN; 3], [1.0; 3]).is_err());
    assert!(Ray::new([0.0; 3], [0.0; 3], 1.0).is_err());
    assert!(Ray::new([0.0; 3], [0.0, 0.0, 1.0], f64::INFINITY).is_err());
    let bg = Arc::new(SparseGrid::new(Transform::identity(), 1.0, 0).unwrap());
    assert!(Medium::new(bg.clone(), None, Transform::identity(), Optical::default()).is_err());
    let bounds = Some(Bounds::new([0.0; 3], [1.0; 3]).unwrap());
    for optical in [
        Optical { extinction: -1.0, ..Default::default() },
        Optical { albedo: [1.1; 3], ..Default::default() },
        Optical { anisotropy: 1.0, ..Default::default() },
        Optical { emission: [f64::INFINITY; 3], ..Default::default() },
    ] {
        assert!(Medium::new(bg.clone(), bounds, Transform::identity(), optical).is_err());
    }
    let negative = Arc::new(SparseGrid::new(Transform::identity(), -1.0, 0).unwrap());
    assert!(Medium::new(negative, bounds, Transform::identity(), Optical::default()).is_err());
    let empty = Medium::new(
        Arc::new(SparseGrid::new(Transform::identity(), 0.0, 0).unwrap()),
        None,
        Transform::identity(),
        Optical::default(),
    )
    .unwrap();
    let r = integrate(&[empty], ray(), March::default(), dark).unwrap();
    assert_eq!(r.transmittance, 1.0);
    assert_eq!(r.steps, 0);
}
