use glam::Vec3;
use sr_3d::terrain::{globe, Globe};

#[test]
fn elevations_are_radial_and_convert_metres_to_scene_units() {
    let spec = Globe { radius: 10., planet_radius: 1000., exaggeration: 2., segments: 32, ..Default::default() };
    let m = globe(&spec, |_, _| Ok(100.)).unwrap();
    for v in &m.vertices {
        assert!((Vec3::from(v.pos).length() - 12.).abs() < 2e-6);
        assert!(Vec3::from(v.normal).dot(Vec3::from(v.pos).normalize()) > 0.99);
        assert!(v.tangent.iter().all(|v| v.is_finite()));
    }
    let front = m.vertices.iter().find(|v| v.uv == [0.5, 0.5]).unwrap();
    assert!((front.pos[2] + 12.).abs() < 1e-6 && front.pos[0].abs() < 1e-6);
    assert!(m.vertices.iter().filter(|v| v.uv[1] == 0.).all(|v| v.pos == [0., -12., 0.]));
}

#[test]
fn rough_globes_have_exact_seams_watertight_poles_and_outward_nonzero_faces() {
    let spec = Globe { radius: 10., planet_radius: 1000., segments: 32, ..Default::default() };
    let m = globe(&spec, |lon, lat| Ok(100. * lon.to_radians().cos() * lat.to_radians().cos())).unwrap();
    for row in m.vertices.as_chunks::<33>().0 {
        assert_eq!(row[0].pos, row[32].pos);
        assert_eq!(row[0].normal, row[32].normal);
        assert_eq!(row[0].tangent, row[32].tangent);
    }
    let mut edges = std::collections::BTreeMap::new();
    let mut points = std::collections::BTreeMap::new();
    let mut ids = Vec::new();
    for v in &m.vertices {
        let next = points.len();
        ids.push(*points.entry(v.pos.map(|x| if x == 0. { 0 } else { x.to_bits() })).or_insert(next));
    }
    for t in m.indices.as_chunks::<3>().0 {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(m.vertices[i as usize].pos));
        assert!((b - a).cross(c - a).dot(a + b + c) > 0.);
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let (a, b) = (ids[a as usize], ids[b as usize]);
            let e = edges.entry((a.min(b), a.max(b))).or_insert((0, 0));
            e.0 += 1;
            e.1 += if a < b { 1 } else { -1 };
        }
    }
    assert!(edges.values().all(|e| *e == (2, 0)));
}

#[test]
fn relief_changes_normals_and_limits_fail_before_sampling() {
    let spec = Globe { radius: 10., planet_radius: 1000., segments: 32, ..Default::default() };
    let flat = globe(&spec, |_, _| Ok(0.)).unwrap();
    let raised = globe(&spec, |lon, lat| Ok(500. * (-(lon * lon + lat * lat) / 400.).exp())).unwrap();
    assert!(flat
        .vertices
        .iter()
        .zip(&raised.vertices)
        .any(|(a, b)| Vec3::from(a.normal).distance(Vec3::from(b.normal)) > 0.1));
    assert!(globe(&Globe { max_bytes: 1, ..spec.clone() }, |_, _| panic!("budget must be checked first")).is_err());
    assert!(globe(&spec, |_, _| Ok(-1000.)).is_err());
    assert!(globe(&spec, |_, _| Ok(f64::NAN)).is_err());
    assert!(globe(&spec, |_, _| Err("DEM corrupt".into())).unwrap_err().contains("DEM corrupt"));
}
