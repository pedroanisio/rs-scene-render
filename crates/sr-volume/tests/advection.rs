use glam::{DMat4, DVec3};
use sr_volume::advection::Advection;
use sr_volume::medium::{Bounds, Medium, Optical};
use sr_volume::{SparseGrid, Transform};
use std::sync::Arc;

fn constant(value: f32) -> Arc<SparseGrid> {
    Arc::new(SparseGrid::new(Transform::identity(), value, 0).unwrap())
}
fn pulse(x: i32, value: f32) -> Arc<SparseGrid> {
    let mut grid = SparseGrid::new(Transform::identity(), 0., 1).unwrap();
    grid.set([x, 0, 0], value).unwrap();
    Arc::new(grid)
}
fn motion(v: [f32; 3], elapsed: f64) -> Advection {
    Advection::new(v.map(constant), elapsed).unwrap()
}

#[test]
fn translating_density_and_temperature_meet_between_frames_without_ghosts() {
    for blend in [0., 0.25, 0.5, 0.75, 1.] {
        let medium = Medium::new(pulse(0, 1.), None, Transform::identity(), Optical::default())
            .unwrap()
            .with_temperature(pulse(0, 2000.), 1., 1.)
            .unwrap()
            .with_next_frame(pulse(4, 1.), Some(pulse(4, 8000.)), blend)
            .unwrap()
            .with_advection(motion([4., 0., 0.], blend), motion([4., 0., 0.], blend - 1.))
            .unwrap();
        let p = [4. * blend, 0., 0.];
        assert_eq!(medium.sample_density(p), 1.);
        let expected = sr_volume::thermal::blackbody_rgb(2000. + blend * 6000.).unwrap();
        assert_eq!(medium.sample_emission(p), expected);
        for direction in [-1., 1.] {
            assert_eq!(medium.sample_density([p[0] + direction * 1.5, 0., 0.]), 0.);
        }
    }
}

#[test]
fn midpoint_trace_uses_spatial_velocity_and_asset_world_axes() {
    // v_x(x)=x. Midpoint backtrace x0=x(1-h+h²/2); x=2,h=1 => x0=1.
    // Euler would instead hit zero, so this distinguishes the integrator order.
    let mut vx = SparseGrid::new(Transform::identity(), 0., 1).unwrap();
    for x in 0..=4 {
        vx.set([x, 0, 0], x as f32).unwrap();
    }
    let trace = Advection::new([Arc::new(vx), constant(0.), constant(0.)], 1.).unwrap();
    assert_eq!(trace.backtrace([2., 0., 0.]), [1., 0., 0.]);
    let object = DMat4::from_scale_rotation_translation(
        DVec3::new(-2., 3., 1.),
        glam::DQuat::from_rotation_z(0.5),
        DVec3::new(20., 30., -1.),
    );
    let medium = Medium::new(pulse(1, 1.), None, Transform::new(object.to_cols_array()).unwrap(), Optical::default())
        .unwrap()
        .with_next_frame(pulse(1, 1.), None, 0.)
        .unwrap()
        .with_advection(trace.clone(), trace)
        .unwrap();
    assert_eq!(medium.sample_density(object.transform_point3(DVec3::new(2., 0., 0.)).to_array()), 1.);
}

#[test]
fn advected_support_expands_inferred_bounds_but_authored_bounds_clip() {
    let make = |bounds| {
        Medium::new(pulse(0, 1.), bounds, Transform::identity(), Optical::default())
            .unwrap()
            .with_next_frame(pulse(0, 1.), None, 0.5)
            .unwrap()
            .with_advection(motion([10., 0., 0.], 1.), motion([10., 0., 0.], 1.))
            .unwrap()
    };
    let inferred = make(None);
    assert_eq!(inferred.sample_density([10., 0., 0.]), 1.);
    assert!(inferred.bounds().unwrap().max()[0] >= 11.);
    let bounds = Bounds::new([-1.; 3], [1.; 3]).unwrap();
    let clipped = make(Some(bounds));
    assert_eq!(clipped.bounds().unwrap(), bounds);
    assert_eq!(clipped.sample_density([10., 0., 0.]), 0.);
}

#[test]
fn advection_rejects_invalid_time_mismatched_components_and_overflow() {
    for t in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(Advection::new([constant(1.), constant(0.), constant(0.)], t).is_err());
    }
    let matrix = Transform::new(DMat4::from_scale(DVec3::splat(2.)).to_cols_array()).unwrap();
    let changed = Arc::new(SparseGrid::new(matrix, 0., 0).unwrap());
    assert!(Advection::new([constant(0.), changed, constant(0.)], 1.).is_err());
    assert!(Advection::new([constant(f32::MAX), constant(0.), constant(0.)], f64::MAX).is_err());
    let medium = Medium::new(pulse(0, 1.), None, Transform::identity(), Optical::default()).unwrap();
    assert!(medium.with_advection(motion([0.; 3], 0.), motion([0.; 3], 0.)).is_err());
}
