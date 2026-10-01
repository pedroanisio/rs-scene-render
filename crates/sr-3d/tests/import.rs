use sr_3d::import;

#[test]
fn gltf_texture_transforms_are_implemented() {
    let dir = std::env::temp_dir().join(format!("sr-gltf-transform-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255])).save(dir.join("tex.png")).unwrap();
    let scene = serde_json::json!({
        "asset": {"version":"2.0"},
        "extensionsUsed": ["KHR_texture_transform"],
        "images": [{"uri":"tex.png"}], "textures":[{"source":0}],
        "materials": [{
            "pbrMetallicRoughness": {"baseColorTexture": {"index":0,"texCoord":0,
                "extensions":{"KHR_texture_transform":{"offset":[0.25,0.5],"scale":[2,3],"rotation":std::f64::consts::FRAC_PI_2,"texCoord":2}}}},
            "normalTexture": {"index":0,"texCoord":1,"extensions":{"KHR_texture_transform":{"offset":[0.75,0]}}},
            "occlusionTexture": {"index":0,"extensions":{"KHR_texture_transform":{"scale":[0.5,0.5]}}}
        }]
    });
    let path = dir.join("scene.gltf");
    std::fs::write(&path, scene.to_string()).unwrap();
    let model = import::gltf(&path).unwrap();
    assert!(model.warnings.is_empty(), "{:?}", model.warnings);
    let material = &model.materials[0];
    assert_eq!(material.texture_transforms[0].tex_coord, 2);
    assert_eq!(material.texture_transforms[1].tex_coord, 1);
    let mut primitive = sr_3d::Primitive {
        vertices: vec![sr_3d::Vertex { uv: [0.2, 0.4], ..Default::default() }],
        ..Default::default()
    };
    primitive.tex_coords.insert(1, vec![[0.1, 0.2]]);
    primitive.tex_coords.insert(2, vec![[0.3, 0.4]]);
    let mut vertices = primitive.vertices.clone();
    material.apply_texture_coordinates(&primitive, &mut vertices);
    let close = |a: [f32; 2], b: [f32; 2]| {
        for i in 0..2 {
            assert!((a[i] - b[i]).abs() < 1e-5, "{a:?} != {b:?}");
        }
    };
    close(vertices[0].uv, [-0.95, 1.1]);
    close(vertices[0].map_uv[0], [0.85, 0.2]);
    close(vertices[0].map_uv[1], [0.2, 0.4]);
    close(vertices[0].map_uv[2], [0.1, 0.2]);
    close(vertices[0].map_uv[3], [0.2, 0.4]);
    assert_eq!(primitive.vertices[0].uv, [0.2, 0.4], "variants retain the source UVs");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ply_vertex_colours_are_imported() {
    let bytes=b"ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0 255 0 0\n1 0 0 0 255 0\n0 1 0 0 0 255\n3 0 1 2\n";
    let sr_3d::Asset::Model(model) = import::ply(bytes).unwrap() else { panic!("mesh") };
    assert!(model.warnings.is_empty(), "{:?}", model.warnings);
    assert_eq!(
        model.primitives[0].vertices.iter().map(|v| v.color).collect::<Vec<_>>(),
        vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]]
    );
}
