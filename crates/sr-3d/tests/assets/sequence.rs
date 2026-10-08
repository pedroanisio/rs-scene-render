use sr_3d::{
    sequence::{Interpolation, Missing, Sequence},
    Model, Node, Primitive, Vertex,
};
use std::sync::Arc;

fn model(x: f32) -> Arc<Model> {
    Arc::new(Model {
        basis: glam::Mat4::IDENTITY,
        nodes: vec![Node { primitives: vec![0], ..Default::default() }],
        primitives: vec![Primitive {
            vertices: [[x, 0., 0.], [x + 1., 0., 0.], [x, 1., 0.]]
                .map(|pos| Vertex { pos, normal: [0., 0., 1.], tangent: [1., 0., 0., 1.], ..Default::default() })
                .to_vec(),
            indices: vec![0, 1, 2],
            ..Default::default()
        }],
        ..Default::default()
    })
}

#[test]
fn sequence_sampling_clamps_replays_and_preserves_topology_changes_in_hold_mode() {
    let a = model(0.);
    let mut b = (*model(2.)).clone();
    b.primitives[0].indices.extend([0, 2, 1]);
    let b = Arc::new(b);
    let seq = Sequence::new(-3, -2, 2., Interpolation::Hold, Missing::Error).unwrap();
    for (time, want) in [(-1., &a), (0.49, &a), (0.5, &b), (10., &b), (0., &a)] {
        let frame =
            seq.sample(time, 1 << 20, |label| Ok(Some(if label == -3 { a.clone() } else { b.clone() }))).unwrap();
        assert!(Arc::ptr_eq(frame.model.as_ref().unwrap(), want));
        assert_eq!(frame.opacity, 1.);
    }
    assert!(seq.sample(f64::NAN, 1 << 20, |_| unreachable!()).is_err());
    assert!(Sequence::new(0, 1_000_000, 24., Interpolation::Hold, Missing::Error).is_err());
}

#[test]
fn linear_mesh_samples_interpolate_geometry_and_normalize_direction_attributes() {
    let a = model(0.);
    let mut b = (*model(2.)).clone();
    for v in &mut b.primitives[0].vertices {
        v.normal = [0., 1., 0.];
        v.tangent = [1., 0., 0., 1.];
        v.uv = [1., 1.];
        v.color = [0., 0., 0., 1.];
    }
    let b = Arc::new(b);
    let seq = Sequence::new(0, 1, 2., Interpolation::Linear, Missing::Error).unwrap();
    for time in [0.25, 0.5, 0.25] {
        let out = seq.sample(time, 1 << 20, |i| Ok(Some(if i == 0 { a.clone() } else { b.clone() }))).unwrap();
        let v = out.model.as_ref().unwrap().primitives[0].vertices[0];
        assert!((v.pos[0] - time as f32 * 4.).abs() < 1e-6);
        assert!((glam::Vec3::from(v.normal).length() - 1.).abs() < 1e-6);
        assert_eq!(v.uv, [time as f32 * 2.; 2]);
        assert_eq!(v.color[0], 1. - time as f32 * 2.);
        assert!(glam::Vec3::from(v.normal).dot(glam::Vec3::from_slice(&v.tangent[..3])).abs() < 1e-6);
    }
    let mut bad = (*b).clone();
    bad.primitives[0].indices = [0, 2, 1].to_vec();
    assert!(seq
        .sample(0.25, 1 << 20, |i| Ok(Some(if i == 0 { a.clone() } else { Arc::new(bad.clone()) })))
        .unwrap_err()
        .contains("topology"));
    assert!(seq.sample(0.25, 1, |_| Ok(Some(a.clone()))).unwrap_err().contains("budget"));
}

#[test]
fn missing_mesh_frames_distinguish_holes_from_decode_errors_and_fade_explicit_transparency() {
    let a = model(0.);
    let seq = Sequence::new(0, 3, 1., Interpolation::Linear, Missing::Hold).unwrap();
    let mut calls = Vec::new();
    let out = seq
        .sample(2.5, 1 << 20, |i| {
            calls.push(i);
            Ok((i == 0).then(|| a.clone()))
        })
        .unwrap();
    assert!(Arc::ptr_eq(out.model.as_ref().unwrap(), &a));
    assert!(calls.len() <= 5, "{calls:?}");
    let transparent = Sequence::new(0, 1, 1., Interpolation::Linear, Missing::Transparent).unwrap();
    let out = transparent.sample(0.25, 1 << 20, |i| Ok((i == 0).then(|| a.clone()))).unwrap();
    assert_eq!(out.opacity, 0.75);
    assert!(transparent.sample(0.25, 1 << 20, |_| Err("corrupt input".into())).unwrap_err().contains("corrupt"));
    assert!(seq.sample(0., 1 << 20, |_| Ok(None)).is_err());
}

#[test]
fn mesh_cache_hold_rejects_cycles_and_invalid_texture_or_skin_references() {
    let seq = Sequence::new(0, 0, 1., Interpolation::Hold, Missing::Error).unwrap();
    for invalid in 0..3 {
        let mut m = (*model(0.)).clone();
        match invalid {
            0 => m.nodes[0].parent = Some(0),
            1 => m.nodes[0].skin = Some(0),
            _ => m.materials.push(sr_3d::ImportedMaterial {
                maps: sr_3d::MaterialMaps { base_color: Some(0), ..Default::default() },
                ..Default::default()
            }),
        }
        assert!(seq.sample(0., 1 << 20, |_| Ok(Some(Arc::new(m.clone())))).is_err(), "case {invalid} was accepted");
    }
}

#[test]
fn display_names_do_not_define_mesh_interpolation_topology() {
    let a = model(0.);
    let mut b = (*model(2.)).clone();
    b.nodes[0].name = "second exported frame".into();
    let seq = Sequence::new(0, 1, 1., Interpolation::Linear, Missing::Error).unwrap();
    let out = seq.sample(0.5, 1 << 20, |i| Ok(Some(if i == 0 { a.clone() } else { Arc::new(b.clone()) }))).unwrap();
    assert_eq!(out.model.unwrap().primitives[0].vertices[0].pos[0], 1.);
}

#[test]
fn mesh_hierarchy_depth_limit_is_independent_of_node_order() {
    let seq = Sequence::new(0, 0, 1., Interpolation::Hold, Missing::Error).unwrap();
    for count in [512usize, 513] {
        for parent_first in [true, false] {
            let mut m = (*model(0.)).clone();
            m.nodes = (0..count)
                .map(|i| Node {
                    parent: if parent_first { i.checked_sub(1) } else { (i + 1 < count).then_some(i + 1) },
                    ..Default::default()
                })
                .collect();
            let result = seq.sample(0., 1 << 20, |_| Ok(Some(Arc::new(m.clone()))));
            assert_eq!(result.is_ok(), count == 512, "count={count}, parent_first={parent_first}: {result:?}");
        }
    }
}
