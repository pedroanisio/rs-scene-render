use glam::{Mat4, Vec3};
use sr_3d::{anim, Model, Node, Primitive, Skin, Trs, Vertex};

#[test]
fn skinned_normals_stay_perpendicular_to_scaled_surface() {
    let points = [Vec3::ZERO, Vec3::new(1.0, -1.0, 0.0), Vec3::new(0.0, 0.0, -1.0)];
    let normal = (points[1] - points[0]).cross(points[2] - points[0]).normalize();
    let primitive = Primitive {
        vertices: points
            .into_iter()
            .map(|p| Vertex { pos: p.to_array(), normal: normal.to_array(), ..Default::default() })
            .collect(),
        indices: vec![0, 1, 2],
        joints: vec![[0, 0, 0, 0]; 3],
        weights: vec![[1.0, 0.0, 0.0, 0.0]; 3],
        ..Default::default()
    };
    let mut model = Model {
        nodes: vec![
            Node { primitives: vec![0], skin: Some(0), ..Default::default() },
            Node { local: Trs { s: Vec3::new(2.0, 1.0, 1.0), ..Default::default() }, ..Default::default() },
        ],
        primitives: vec![primitive],
        skins: vec![Skin { joints: vec![1], inverse_bind: vec![Mat4::IDENTITY] }],
        ..Default::default()
    };
    for scale in [Vec3::new(2.0, 1.0, 1.0), Vec3::new(-2.0, 1.0, 1.0), Vec3::new(0.0, 1.0, 1.0), Vec3::ZERO] {
        model.nodes[1].local.s = scale;
        let (locals, weights) = anim::pose(&model, None, 0.0);
        let draw = anim::draw_list(&model, &locals, &weights, None);
        let vertices = draw[0].vertices.as_ref().unwrap();
        let edge = Vec3::from(vertices[1].pos) - Vec3::from(vertices[0].pos);
        let n = Vec3::from(vertices[0].normal);
        assert!(n.is_finite(), "scale {scale:?}: {n:?}");
        assert!(edge.dot(n).abs() < 1e-6, "normal {n:?} is not perpendicular to edge {edge:?}: dot={}", edge.dot(n));
        if scale.x != 0.0 {
            let expected = Mat4::from_scale(scale).inverse().transpose().transform_vector3(normal).normalize();
            assert!(n.abs_diff_eq(expected, 1e-6), "mirrored normals preserve the inverse-transpose direction");
        }
    }
}
