use glam::{DMat3, DVec3};
use sr_3d::crater::{Budget, Crater, Deposit, Spec};

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

/// A deposit of 16 by 16 cells of 0.5, its corner at (-4, -4) of the plane of the crater, with a heap in the middle and some low
/// ground to one side.
fn heap() -> Deposit {
    let heights = (0..256)
        .map(|i| {
            let (x, y) = ((i % 16) as f64 - 7.5, (i / 16) as f64 - 7.5);
            (1.2 - 0.25 * x.hypot(y)).max(0.0) + if x > 4.0 { 0.1 } else { 0.0 }
        })
        .collect();
    Deposit::new([-4.0, -4.0], 0.5, [16, 16], heights).unwrap()
}

#[test]
fn a_deposit_raises_the_ground_where_it_lies_by_its_height_and_by_its_volume() {
    let (spec, budget) = authored();
    let plain = Crater::conserving(spec, budget).unwrap();
    let deposit = heap();
    let with = plain.clone().with_deposit(deposit.clone());
    let (u, v) = plain.plane_basis();
    let at = |crater: &Crater, a: f64, b: f64| {
        let p = [a * u[0] + b * v[0], a * u[1] + b * v[1], a * u[2] + b * v[2]];
        let moved = crater.map(p, 1.).unwrap().position;
        // along the outward axis, which is minus z here
        -(moved[2] - p[2])
    };
    for (a, b) in [(0.0, 0.0), (1.3, -0.7), (-2.2, 2.9), (3.1, 3.1), (-6.0, 6.0), (20.0, 20.0)] {
        let raised = at(&with, a, b) - at(&plain, a, b);
        let (wanted, _) = deposit.height(a, b);
        assert!((raised - wanted).abs() < 1e-12, "({a}, {b}): {raised} against {wanted}");
    }
    // outside the grid there is none, and at the middle there is
    assert_eq!(deposit.height(20.0, 20.0).0, 0.0);
    assert!(deposit.height(0.0, 0.0).0 > 1.0);
    // the ground has the volume of the deposit more: the integral of the rise over the plane, with a quadrature of the interpolant
    let n = 800;
    let h = 12.0 / n as f64;
    let mut volume = 0.0;
    for i in 0..n {
        for j in 0..n {
            let (a, b) = (-6.0 + (i as f64 + 0.5) * h, -6.0 + (j as f64 + 0.5) * h);
            volume += (at(&with, a, b) - at(&plain, a, b)) * h * h;
        }
    }
    println!("DEPOSIT volume of the heights {:.6}, integral of the rise {volume:.6}", deposit.volume());
    assert!((volume - deposit.volume()).abs() < 1e-3 * deposit.volume(), "{volume} against {}", deposit.volume());
}

#[test]
fn the_deposit_keeps_the_map_smooth_enough_for_its_normals_and_orientation() {
    let (spec, budget) = authored();
    let crater = Crater::conserving(spec, budget).unwrap().with_deposit(heap());
    let (u, v) = crater.plane_basis();
    let at =
        |a: f64, b: f64, c: f64| -> [f64; 3] { std::array::from_fn(|i| a * u[i] + b * v[i] + c * [0., 0., -1.][i]) };
    // inside cells, away from their borders, the jacobian is the derivative of the map
    for (a, b) in [(0.13, 0.21), (1.31, -0.77), (-2.21, 2.93), (3.11, 1.07), (-1.43, -3.1), (2.2, 2.7)] {
        let p = at(a, b, 0.0);
        let m = crater.map(p, 1.).unwrap();
        let e = 1e-6;
        for k in 0..3 {
            let mut d = [0.0; 3];
            d[k] = e;
            let (hi, lo) = (
                crater.map([p[0] + d[0], p[1] + d[1], p[2] + d[2]], 1.).unwrap(),
                crater.map([p[0] - d[0], p[1] - d[1], p[2] - d[2]], 1.).unwrap(),
            );
            for i in 0..3 {
                let numeric = (hi.position[i] - lo.position[i]) / (2.0 * e);
                assert!(
                    (numeric - m.jacobian.col(k)[i]).abs() < 1e-5,
                    "({a}, {b}) d{k} of {i}: {numeric} against {}",
                    m.jacobian.col(k)[i]
                );
            }
        }
        assert!(m.jacobian.determinant() > 0.0);
    }
}

#[test]
fn a_deposit_that_makes_no_sense_is_an_error_and_none_is_the_crater_as_it_was() {
    assert!(Deposit::new([0.0; 2], 0.5, [0, 4], vec![]).is_err());
    assert!(Deposit::new([0.0; 2], 0.5, [2, 2], vec![0.0; 3]).is_err());
    assert!(Deposit::new([0.0; 2], 0.0, [2, 2], vec![0.0; 4]).is_err());
    assert!(Deposit::new([f64::NAN, 0.0], 0.5, [2, 2], vec![0.0; 4]).is_err());
    assert!(Deposit::new([0.0; 2], 0.5, [2, 2], vec![0.0, 0.0, f64::NAN, 0.0]).is_err());
    assert!(Deposit::new([0.0; 2], 0.5, [2, 2], vec![0.0, 0.0, -0.1, 0.0]).is_err());
    let (spec, budget) = authored();
    let plain = Crater::conserving(spec, budget).unwrap();
    let empty = plain.clone().with_deposit(Deposit::new([-4.0; 2], 0.5, [16, 16], vec![0.0; 256]).unwrap());
    for p in [[0.0; 3], [3.0, 1.0, 0.0], [9.0, 0.0, 0.0], [1.0, 2.0, 0.5]] {
        let (a, b) = (plain.map(p, 1.).unwrap(), empty.map(p, 1.).unwrap());
        assert_eq!(a.position.map(f64::to_bits), b.position.map(f64::to_bits), "{p:?}");
    }
}

#[test]
fn the_profile_of_the_bowl_and_of_the_rim_are_readable_and_hold_the_volumes_of_the_crater() {
    let (spec, budget) = authored();
    let crater = Crater::conserving(spec, budget).unwrap();
    // the floor of the bowl under the centre is the depth, and it meets the original surface at the crest radius
    assert!((crater.bowl_depth_at(0.0) - spec.depth).abs() < 1e-12);
    assert_eq!(crater.bowl_depth_at(spec.radius), 0.0);
    assert_eq!(crater.bowl_depth_at(2.0 * spec.radius), 0.0);
    assert!(
        crater.bowl_depth_at(0.5 * spec.radius) < crater.bowl_depth_at(0.25 * spec.radius),
        "it rises towards the wall"
    );
    // what the profile encloses is the volume the crater says it excavates (Simpson over the radius)
    let simpson = |f: &dyn Fn(f64) -> f64, lo: f64, hi: f64| {
        let n = 20_000;
        let h = (hi - lo) / n as f64;
        let mut sum = f(lo) + f(hi);
        for i in 1..n {
            sum += f(lo + i as f64 * h) * if i % 2 == 1 { 4. } else { 2. };
        }
        sum * h / 3.
    };
    let two_pi = 2. * std::f64::consts::PI;
    let bowl = simpson(&|r| two_pi * r * crater.bowl_depth_at(r), 0.0, spec.radius);
    assert!(
        (bowl - crater.volumes().bowl).abs() < 1e-6 * crater.volumes().bowl,
        "{bowl} against {}",
        crater.volumes().bowl
    );
    // the rim stands over the original surface between the crest radius less its width and the crest radius plus it, highest at the crest
    let rim =
        simpson(&|r| two_pi * r * crater.rim_height_at(r), spec.radius - spec.rim_width, spec.radius + spec.rim_width);
    assert!((rim - crater.volumes().rim).abs() < 1e-6 * crater.volumes().rim, "{rim} against {}", crater.volumes().rim);
    assert!((crater.rim_height_at(spec.radius) - spec.rim_height).abs() < 1e-12);
    assert_eq!(crater.rim_height_at(spec.radius + 1.01 * spec.rim_width), 0.0);
    // the axis is the unit vector the crater points along
    let axis = crater.axis();
    assert!((axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2] - 1.0).abs() < 1e-15);
    assert_eq!(axis, [0.0, 0.0, -1.0]);
    // a crater that has the exponent 2 (not conserving) has it readable too
    let plain = Crater::new(spec).unwrap();
    assert_eq!(plain.bowl_exponent(), 2.0);
    assert!(crater.bowl_exponent() >= 2.0);
}

#[test]
fn the_readable_profile_is_the_surface_the_map_gives_to_a_point_on_the_original_ground() {
    // with no mantle the grown surface over a point of the original ground is the rim less the bowl, and `map` is what draws it: the two
    // accessors are that surface, which is what a voxel crater has to follow
    let (spec, budget) = authored();
    let crater = Crater::conserving(spec, Budget { ejecta: 0.0, bulking: Some(1.0), ..budget }).unwrap();
    for r in [0.0, 1.0, 3.3, 5.0, 5.6, 6.0, 6.6, 7.0, 7.9, 8.1, 9.5, 12.0] {
        let moved = crater.map([r, 0.0, 0.0], 1.0).unwrap().position;
        // the axis is minus z: the height gained along it is minus the change of z
        let height = -moved[2];
        let wanted = -crater.bowl_depth_at(r) + crater.rim_height_at(r);
        assert!((height - wanted).abs() < 1e-12, "r = {r}: the map gives {height}, the profile {wanted}");
    }
}

#[test]
fn with_a_mantle_the_map_is_the_bowl_the_rim_and_the_mantle_and_the_voxel_crater_follows_the_first_two() {
    // the kernel that the evaluator uses has a mantle (the law's ejecta volume): the surface that the map gives over the original ground is the
    // bowl, the rim AND the mantle's height, which is half its thickness at the crest. A crater in cells follows the bowl and the rim only: the
    // ejecta are particles there and settle as cells, so the mantle is not part of the ground it excavates. Both are stated by the accessors.
    let (spec, budget) = authored();
    let crater = Crater::conserving(spec, Budget { bulking: Some(1.0), ..budget }).unwrap();
    assert!(crater.mantle_thickness() > 0.0);
    let mut at_crest = 0.0;
    for r in [0.0, 3.3, 5.6, 6.0, 6.6579307496549855, 7.0, 7.9, 8.1, 9.5, 12.0, 30.0, 100.0, 133.0] {
        let moved = crater.map([r, 0.0, 0.0], 1.0).unwrap().position;
        let height = -moved[2];
        let wanted = -crater.bowl_depth_at(r) + crater.rim_height_at(r) + crater.mantle_height_at(r);
        assert!((height - wanted).abs() < 1e-12, "r = {r}: the map gives {height}, the three accessors {wanted}");
        if (r - spec.radius).abs() < 1e-9 {
            at_crest = crater.mantle_height_at(r);
        }
    }
    // half the thickness at the crest, and nothing under the crest or beyond the reach
    assert!((at_crest - 0.5 * crater.mantle_thickness()).abs() < 1e-12, "{at_crest}");
    assert_eq!(crater.mantle_height_at(0.5 * spec.radius), 0.0);
    assert_eq!(crater.mantle_height_at(crater.reach() * 1.01), 0.0);
    // without a mantle the accessor says nothing
    assert_eq!(Crater::new(spec).unwrap().mantle_height_at(spec.radius), 0.0);
}
