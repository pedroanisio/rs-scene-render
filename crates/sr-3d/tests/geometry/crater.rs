use glam::{DMat3, DVec3};
use sr_3d::crater::{Budget, Crater, Spec};

fn spec() -> Spec {
    Spec {
        center: [0.; 3],
        outward: [0., 0., -1.],
        radius: 4.,
        depth: 2.,
        rim_height: 0.5,
        rim_width: 1.,
        influence_depth: 8.,
    }
}

#[test]
fn crater_excavates_center_and_raises_rim_with_compact_support() {
    let c = Crater::new(spec()).unwrap();
    assert_eq!(c.map([0.; 3], 1.).unwrap().position, [0., 0., 2.]);
    assert_eq!(c.map([4., 0., 0.], 1.).unwrap().position, [4., 0., -0.5]);
    for p in [[5., 0., 0.], [9., 0., 0.], [0., 0., 8.], [0., 0., -8.]] {
        let mapped = c.map(p, 1.).unwrap();
        assert_eq!(mapped.position, p);
        assert_eq!(mapped.jacobian, DMat3::IDENTITY);
    }
    // Half-growth has half radius, depth and rim height/width.
    assert_eq!(c.map([0.; 3], 0.5).unwrap().position, [0., 0., 1.]);
    assert_eq!(c.map([2., 0., 0.], 0.5).unwrap().position, [2., 0., -0.25]);
    for t in [0., 0.5, 1., 0.5, 0.] {
        let p = c.map([0.; 3], t).unwrap();
        assert_eq!(p.position[2], 2. * t);
    }
}

#[test]
fn crater_mapping_and_differential_rotate_with_its_authored_frame() {
    let base = Crater::new(spec()).unwrap();
    let rotation = DMat3::from_rotation_y(0.7) * DMat3::from_rotation_x(-0.4);
    let center = DVec3::new(10., -3., 7.);
    let mut s = spec();
    s.center = center.to_array();
    s.outward = (rotation * DVec3::NEG_Z * 100.).to_array();
    let moved = Crater::new(s).unwrap();
    let p = DVec3::new(2., 0.7, 1.);
    let a = base.map(p.to_array(), 0.8).unwrap();
    let b = moved.map((center + rotation * p).to_array(), 0.8).unwrap();
    assert!((DVec3::from(b.position) - center - rotation * DVec3::from(a.position)).length() < 1e-12);
    let want = rotation * a.jacobian * rotation.transpose();
    assert!(b.jacobian.to_cols_array().iter().zip(want.to_cols_array()).all(|(a, b)| (a - b).abs() < 1e-12));
}

#[test]
fn crater_normals_follow_the_deformed_surface_and_preserve_vertex_data() {
    let c = Crater::new(spec()).unwrap();
    let source = sr_3d::Vertex {
        pos: [2., 0., 0.],
        normal: [0., 0., -1.],
        tangent: [1., 0., 0., -1.],
        uv: [0.2, 0.4],
        map_uv: [[0.3, 0.7]; 4],
        color: [0.2, 0.3, 0.4, 0.5],
    };
    let out = c.deform(&[source], 1., 4096).unwrap()[0];
    let dx = DVec3::from(c.map([2.00001, 0., 0.], 1.).unwrap().position)
        - DVec3::from(c.map([1.99999, 0., 0.], 1.).unwrap().position);
    let dy = DVec3::from(c.map([2., 0.00001, 0.], 1.).unwrap().position)
        - DVec3::from(c.map([2., -0.00001, 0.], 1.).unwrap().position);
    let normal = -dx.cross(dy).normalize();
    assert!((DVec3::from(out.normal.map(f64::from)) - normal).length() < 1e-6);
    let tangent = DVec3::new(out.tangent[0] as f64, out.tangent[1] as f64, out.tangent[2] as f64);
    assert!(tangent.dot(normal).abs() < 1e-6);
    assert!((tangent.length() - 1.).abs() < 1e-6);
    assert_eq!(out.uv, source.uv);
    assert_eq!(out.map_uv, source.map_uv);
    assert_eq!(out.color, source.color);
    assert_eq!(out.tangent[3], source.tangent[3]);
    assert_eq!(c.deform(&[source], 0., 4096).unwrap(), [source]);
}

#[test]
fn crater_jacobian_matches_finite_differences_and_keeps_orientation() {
    let c = Crater::new(spec()).unwrap();
    for x in [0., 0.1, 2., 3.5, 4., 4.5, 4.999, 5., 6.] {
        for z in [-8., -4., 0., 3., 7.9, 8.] {
            let p = DVec3::new(x, 0., z);
            let mapped = c.map(p.to_array(), 1.).unwrap();
            assert!(mapped.jacobian.determinant() > 0.2);
            for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
                let a = DVec3::from(c.map((p + axis * 1e-6).to_array(), 1.).unwrap().position);
                let b = DVec3::from(c.map((p - axis * 1e-6).to_array(), 1.).unwrap().position);
                assert!(((a - b) / 2e-6 - mapped.jacobian * axis).length() < 1e-5, "at {p:?}");
            }
        }
    }
}

#[test]
fn crater_rejects_invalid_domains_and_admits_memory_before_copying() {
    for case in 0..7 {
        let mut s = spec();
        match case {
            0 => s.outward = [0.; 3],
            1 => s.radius = 0.,
            2 => s.depth = -1.,
            3 => s.rim_width = 5.,
            4 => s.influence_depth = 3.,
            5 => s.center[0] = f64::NAN,
            _ => s.rim_height = f64::INFINITY,
        }
        assert!(Crater::new(s).is_err(), "case {case}");
    }
    let c = Crater::new(spec()).unwrap();
    assert!(c.map([0.; 3], f64::NAN).is_err());
    assert!(c.map([f64::INFINITY, 0., 0.], 0.).is_err());
    let source = sr_3d::prim::plane(10., 10., 4).vertices;
    let bytes = source.len() * std::mem::size_of::<sr_3d::Vertex>();
    assert!(c.deform(&source, 1., bytes - 1).is_err());
    assert_eq!(c.deform(&source, 1., bytes).unwrap().len(), source.len());
    assert_eq!(source[0].pos, [-5., -5., 0.]);
}

#[test]
fn crater_deformation_respects_imported_mesh_basis_and_node_transforms() {
    let c = Crater::new(spec()).unwrap();
    let transform =
        glam::DMat4::from_translation(DVec3::new(0.7, 0.2, -0.1)) * glam::DMat4::from_scale(DVec3::new(-2., 3., 4.));
    let source =
        sr_3d::Vertex { pos: [0.4, 0., 0.], normal: [0., 0., -1.], tangent: [1., 0., 0., 1.], ..Default::default() };
    let result = c.deform_in(&[source], transform, 0.8, 4096).unwrap()[0];
    let point = transform.transform_point3(DVec3::from(source.pos.map(f64::from)));
    let mapped = c.map(point.to_array(), 0.8).unwrap();
    assert!(
        (transform.transform_point3(DVec3::from(result.pos.map(f64::from))) - DVec3::from(mapped.position)).length()
            < 1e-6
    );
    let normal_to_object = DMat3::from_mat4(transform).inverse().transpose();
    let expected = (mapped.jacobian.inverse().transpose() * normal_to_object * DVec3::NEG_Z).normalize();
    let actual = (normal_to_object * DVec3::from(result.normal.map(f64::from))).normalize();
    assert!((actual - expected).length() < 1e-6);
    assert!(c.deform_in(&[source], glam::DMat4::ZERO, 0.8, 4096).is_err());
}

/// The crater of the authored rock in soft rock (the numbers of `sr_sim::cratering::crater`, in metres): the bowl and the rim of the law
/// and the volume it excavates and throws out.
fn authored() -> (Spec, Budget) {
    let (radius, rim_radius) = (5.121485192042297, 6.6579307496549855);
    (
        Spec {
            center: [0.; 3],
            outward: [0., 0., -1.],
            radius: rim_radius,
            depth: 2.793537377477616,
            rim_height: 0.4793710139751589,
            rim_width: rim_radius - radius,
            influence_depth: 2. * rim_radius,
        },
        Budget { volume: 100.92754480774914, ejecta: 80.74203584619931, bulking: None },
    )
}

/// The change of height, integrated over the plane of the ground, of a crater that has grown: by Simpson's rule on the radius.
fn net_volume(crater: &Crater) -> f64 {
    let reach = crater.reach();
    let intervals = 40_000;
    let h = reach / intervals as f64;
    let rise = |r: f64| -crater.map([r, 0., 0.], 1.).unwrap().position[2];
    let f = |r: f64| 2. * std::f64::consts::PI * r * rise(r);
    let mut sum = f(0.) + f(reach);
    for i in 1..intervals {
        sum += f(i as f64 * h) * if i % 2 == 1 { 4. } else { 2. };
    }
    sum * h / 3.
}

#[test]
fn a_crater_that_keeps_its_volume_puts_back_what_it_took_out_as_a_rim_and_a_mantle() {
    let (spec, budget) = authored();
    for bulking in [None, Some(1.0), Some(1.3)] {
        let crater = Crater::conserving(spec, Budget { bulking, ..budget }).unwrap();
        let v = crater.volumes();
        println!(
            "MANTLE bulking {bulking:?}: bowl {:.3}, rim {:.3}, mantle {:.3} of V = {:.3}",
            v.bowl, v.rim, v.mantle, budget.volume
        );
        // the bowl excavates exactly the volume of the law, and the mantle holds the ejecta that the law throws out
        assert!((v.bowl - budget.volume).abs() < 1e-9 * budget.volume, "{v:?}");
        assert!((v.mantle - budget.ejecta).abs() < 1e-6 * budget.volume, "{v:?}");
        // what is put back is the volume times the bulking: the rim and the mantle together; with none given, the bulking that the
        // law's own rim height asks for (the rim of the law and 0.8 V of mantle)
        let wanted = bulking.unwrap_or((v.rim + budget.ejecta) / budget.volume);
        assert!(((v.rim + v.mantle) / budget.volume - wanted).abs() < 1e-6, "{v:?}");
        if bulking.is_none() {
            assert!((crater.spec().rim_height - spec.rim_height).abs() < 1e-12, "the rim of the law stays");
            assert!(wanted > 1.1 && wanted < 1.15, "{wanted}");
        }
        // and the ground that the map moves is the sum of the three, whatever the profile: the integral of the change of height
        let net = net_volume(&crater);
        assert!(
            (net - (wanted - 1.) * budget.volume).abs() < 1e-6 * budget.volume,
            "net {net} against {}",
            (wanted - 1.) * budget.volume
        );
    }
}

#[test]
fn the_mantle_falls_off_as_the_inverse_cube_outside_the_rim_and_stops_at_twenty_crest_radii() {
    let (spec, budget) = authored();
    let crater = Crater::conserving(spec, budget).unwrap();
    let t0 = crater.mantle_thickness();
    // the law's blanket is 0.2899 m at the crest for all of the ejecta outside it; with the ramp over the rim and the cut at twenty
    // radii the same volume needs a little more
    assert!(t0 > 0.2898 && t0 < 0.33, "{t0}");
    for factor in [1.5, 2.0, 5.0, 12.0] {
        let r = factor * spec.radius;
        let rise = -crater.map([r, 0., 0.], 1.).unwrap().position[2];
        assert!((rise - t0 / (factor * factor * factor)).abs() < 1e-9 * t0, "r = {factor} crests: {rise}");
    }
    assert_eq!(crater.reach(), 20. * spec.radius);
    assert_eq!(crater.map([20.5 * spec.radius, 0., 0.], 1.).unwrap().position, [20.5 * spec.radius, 0., 0.]);
    // a crater that is half grown has half the height at half the crest radius, as every part of the map does
    let half = -crater.map([2. * spec.radius, 0., 0.], 0.5).unwrap().position[2];
    assert!((half - 0.5 * t0 / 64.).abs() < 1e-9 * t0, "{half}");
}

#[test]
fn the_conserving_crater_keeps_the_depth_and_the_radius_of_the_law_and_the_map_keeps_its_orientation() {
    let (spec, budget) = authored();
    let crater = Crater::conserving(spec, budget).unwrap();
    assert!(
        (-crater.map([0., 0., 0.], 1.).unwrap().position[2] + spec.depth).abs() < 1e-12,
        "the depth at the centre is the law's"
    );
    // finite differences of the map against its differential, in the bowl, on the rim and in the mantle
    for r in [1.0, 4.0, 6.2, 7.0, 9.0, 25.0, 80.0] {
        let p = [r * 0.8, r * 0.6, 0.];
        let m = crater.map(p, 1.).unwrap();
        assert!(m.jacobian.determinant() > 0.2, "r = {r}: {}", m.jacobian.determinant());
        let e = 1e-6;
        for axis in 0..3 {
            let (mut a, mut b) = (p, p);
            a[axis] += e;
            b[axis] -= e;
            let (ma, mb) = (crater.map(a, 1.).unwrap(), crater.map(b, 1.).unwrap());
            let column = m.jacobian.col(axis);
            for k in 0..3 {
                let numeric = (ma.position[k] - mb.position[k]) / (2. * e);
                assert!(
                    (numeric - column[k]).abs() < 1e-5,
                    "r = {r}, axis {axis}, component {k}: {numeric} against {}",
                    column[k]
                );
            }
        }
    }
}

#[test]
fn a_budget_that_cannot_be_met_or_makes_no_sense_is_an_error() {
    let (spec, budget) = authored();
    for bad in [
        Budget { volume: 0., ..budget },
        Budget { volume: f64::NAN, ..budget },
        Budget { ejecta: -1., ..budget },
        Budget { bulking: Some(0.5), ..budget },
        Budget { bulking: Some(f64::INFINITY), ..budget },
    ] {
        assert!(Crater::conserving(spec, bad).is_err(), "{bad:?}");
    }
    // a volume that the bump of the exponent 2 cannot hold at the law's depth is held by a deeper bowl, with the volume exact
    let more = Budget { volume: 150., ..budget };
    let crater = Crater::conserving(spec, more).unwrap();
    assert!(crater.spec().depth > spec.depth && (crater.volumes().bowl - 150.).abs() < 1e-9 * 150.);
    // and one that the envelope cannot carry is refused
    assert!(Crater::conserving(spec, Budget { volume: 400., ..budget }).is_err());
}
