//! sr-3d: primitives, importers, animation, lights, environments, cameras and MaterialX.

#![allow(unknown_lints, clippy::chunks_exact_to_as_chunks)]

use glam::{Vec2, Vec3};
use sr_3d::{anim, camera, env, import, light, mtlx, prim, Asset, Primitive};

/// Signed volume of a closed mesh (positive when triangles face outward).
fn volume(p: &Primitive) -> f32 {
    p.indices
        .chunks_exact(3)
        .map(|t| {
            let (a, b, c) = (
                Vec3::from(p.vertices[t[0] as usize].pos),
                Vec3::from(p.vertices[t[1] as usize].pos),
                Vec3::from(p.vertices[t[2] as usize].pos),
            );
            a.dot(b.cross(c)) / 6.0
        })
        .sum()
}

fn tmp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sr3d-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

#[test]
fn malformed_glb_headers_return_errors() {
    for length in [0u32, 1, 11, 12, 23, 25, u32::MAX] {
        let mut bytes = b"glTF\x02\x00\x00\x00".to_vec();
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.resize(24, 0);
        let path = tmp(&format!("bad-header-{length}.glb"), &bytes);
        assert!(import::load(&path, None).is_err(), "invalid length {length}");
    }
    for len in 0..12 {
        let path = tmp(&format!("short-header-{len}.glb"), &b"glTF\x02\0\0\0\x0c\0\0\0"[..len]);
        assert!(import::load(&path, None).is_err(), "truncated header of {len} bytes");
    }
}

#[test]
fn corpus_glb_contains_collision_geometry() {
    let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus/media/robot.glb"));
    let Asset::Model(model) = import::load(path, None).unwrap() else { panic!("model") };
    assert!(!model.nodes.is_empty());
    assert!(model.primitives.iter().any(|p| !p.vertices.is_empty() && p.indices.len() >= 3));
}

#[test]
fn primitives_are_closed_and_face_outward() {
    let pi = std::f32::consts::PI;
    let s = prim::sphere(50.0, 64);
    let want = 4.0 / 3.0 * pi * 50f32.powi(3);
    assert!((volume(&s) - want).abs() / want < 0.01, "sphere {} vs {want}", volume(&s));
    assert!((volume(&prim::cuboid(10.0, 20.0, 30.0)) - 6000.0).abs() < 1e-2);
    let c = prim::cylinder(10.0, 10.0, 40.0, 64);
    assert!((volume(&c) - pi * 100.0 * 40.0).abs() / (pi * 4000.0) < 0.01);
    let cone = prim::cylinder(0.0, 10.0, 30.0, 64);
    assert!((volume(&cone) - pi * 100.0 * 10.0).abs() / (pi * 1000.0) < 0.01);
    let t = prim::torus(40.0, 10.0, 64);
    let tv = 2.0 * pi * pi * 40.0 * 100.0;
    assert!((volume(&t) - tv).abs() / tv < 0.02, "torus {} vs {tv}", volume(&t));
    let cap = prim::capsule(10.0, 60.0, 48);
    let cv = pi * 100.0 * 40.0 + 4.0 / 3.0 * pi * 1000.0;
    assert!((volume(&cap) - cv).abs() / cv < 0.02, "capsule {} vs {cv}", volume(&cap));
    let pl = prim::plane(100.0, 50.0, 1);
    assert_eq!(pl.indices.len(), 6);
    for t in pl.indices.chunks_exact(3) {
        let (a, b, c) = (
            Vec3::from(pl.vertices[t[0] as usize].pos),
            Vec3::from(pl.vertices[t[1] as usize].pos),
            Vec3::from(pl.vertices[t[2] as usize].pos),
        );
        assert!((b - a).cross(c - a).z < 0.0, "the plane faces the camera (−z)");
    }
    for v in &s.vertices {
        assert!((Vec3::new(v.tangent[0], v.tangent[1], v.tangent[2]).length() - 1.0).abs() < 1e-3);
    }
}

#[test]
fn extrusion_with_holes_and_bevels() {
    let sq = |s: f32, cw: bool| {
        let mut r = vec![Vec2::new(-s, -s), Vec2::new(s, -s), Vec2::new(s, s), Vec2::new(-s, s)];
        if cw {
            r.reverse();
        }
        r
    };
    let solid = prim::extrude(&[sq(10.0, false), sq(5.0, true)], 4.0, 0.0).unwrap();
    assert!((volume(&solid) - (400.0 - 100.0) * 4.0).abs() < 1e-2, "{}", volume(&solid));
    // the same hole wound the same way still reads as a hole under the renderer's convention? nonzero: no — it fills
    let beveled = prim::extrude(&[sq(10.0, false)], 4.0, 1.0).unwrap();
    let v = volume(&beveled);
    assert!(v > 0.0 && v < 1600.0 && v > 1300.0, "bevel trims the edges: {v}");
    let polys = prim::path_polygons("M0 0 L20 0 L20 20 Z", 0.1).unwrap();
    let tri = prim::extrude(&polys, 2.0, 0.0).unwrap();
    assert!((volume(&tri) - 400.0).abs() < 1e-2);
    assert!(prim::extrude(&[], 1.0, 0.0).is_err());
}

const TRI_GLTF: &str = r#"{
 "asset":{"version":"2.0"},
 "extensionsUsed":["KHR_materials_clearcoat","KHR_materials_variants"],
 "extensions":{"KHR_materials_variants":{"variants":[{"name":"red"}]}},
 "scene":0,"scenes":[{"nodes":[0]}],
 "nodes":[{"mesh":0,"translation":[1,0,0]}],
 "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1,"material":0,
   "extensions":{"KHR_materials_variants":{"mappings":[{"material":1,"variants":[0]}]}}}]}],
 "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[0.5,0.5,0.5,1],"metallicFactor":0.25,"roughnessFactor":0.75},
   "extensions":{"KHR_materials_clearcoat":{"clearcoatFactor":0.8,"clearcoatRoughnessFactor":0.1}}},
   {"pbrMetallicRoughness":{"baseColorFactor":[1,0,0,1]}}],
 "buffers":[{"byteLength":48,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAAABAAIAAAAAAIA/"}],
 "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":6},{"buffer":0,"byteOffset":44,"byteLength":4}],
 "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},
   {"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"},
   {"bufferView":2,"componentType":5126,"count":1,"type":"SCALAR","min":[1],"max":[1]},
   {"bufferView":0,"componentType":5126,"count":1,"type":"VEC3"}],
 "animations":[{"name":"move","samplers":[{"input":2,"output":3,"interpolation":"LINEAR"}],"channels":[{"sampler":0,"target":{"node":0,"path":"translation"}}]}]
}"#;

#[test]
fn gltf_node_morph_weights_override_mesh_defaults() {
    for (case, mesh, node, want) in [
        ("override", Some(0.0), Some(1.0), 1.0),
        ("zero", Some(1.0), Some(0.0), 0.0),
        ("fallback", Some(0.5), None, 0.5),
        ("node", None, Some(0.5), 0.5),
        ("none", None, None, 0.0),
    ] {
        let mut json: serde_json::Value = serde_json::from_str(TRI_GLTF).unwrap();
        json["meshes"][0]["primitives"][0]["targets"] = serde_json::json!([{"POSITION":0}]);
        if let Some(w) = mesh {
            json["meshes"][0]["weights"] = serde_json::json!([w]);
        }
        if let Some(w) = node {
            json["nodes"][0]["weights"] = serde_json::json!([w]);
        }
        let path = tmp(&format!("morph-{case}.gltf"), &serde_json::to_vec(&json).unwrap());
        let m = import::gltf(&path).unwrap();
        let (locals, weights) = anim::pose(&m, None, 0.0);
        let items = anim::draw_list(&m, &locals, &weights, None);
        let vs = items[0].vertices.as_ref().unwrap_or(&m.primitives[0].vertices);
        assert_eq!(vs[1].pos[0], 1.0 + want, "{case}: node weights {weights:?}");
    }
}

#[test]
fn gltf_draws_the_selected_scene_and_keeps_node_indices() {
    for (case, scene, scenes, want) in [
        ("default", Some(1), true, vec![1]),
        ("first", None, true, vec![2]),
        ("empty", Some(2), true, vec![]),
        ("sceneless", None, false, vec![1, 2]),
    ] {
        let mut json: serde_json::Value = serde_json::from_str(TRI_GLTF).unwrap();
        json["nodes"] = serde_json::json!([
            {"children":[1]}, {"mesh":0,"translation":[1,0,0]}, {"mesh":0,"translation":[10,0,0]}
        ]);
        json["scenes"] = serde_json::json!([{"nodes":[2]}, {"nodes":[0]}, {"nodes":[]}]);
        json.as_object_mut().unwrap().remove("scene");
        if let Some(s) = scene {
            json["scene"] = s.into();
        }
        if !scenes {
            json.as_object_mut().unwrap().remove("scenes");
        }
        let path = tmp(&format!("scenes-{case}.gltf"), &serde_json::to_vec(&json).unwrap());
        let m = import::gltf(&path).unwrap();
        assert_eq!(m.nodes.len(), 3, "animation and skin indices must remain valid");
        assert_eq!(m.nodes[1].parent, Some(0));
        assert_eq!(m.animations[0].channels[0].node, 0);
        let (locals, weights) = anim::pose(&m, None, 0.0);
        let items = anim::draw_list(&m, &locals, &weights, None);
        assert_eq!(items.iter().map(|i| i.node).collect::<Vec<_>>(), want, "{case}");
        let (_, hi) = m.bounds();
        let want_max = if want.contains(&2) {
            1100.0
        } else if want.is_empty() {
            0.0
        } else {
            200.0
        };
        assert!((hi.x - want_max).abs() < 1e-3, "{case}: inactive geometry must not enlarge bounds");
    }
}

#[test]
fn gltf_with_extensions_variants_and_animation() {
    let p = tmp("tri.gltf", TRI_GLTF.as_bytes());
    let Asset::Model(m) = import::load(&p, None).unwrap() else { panic!("model") };
    assert_eq!(m.primitives.len(), 1);
    assert_eq!(m.primitives[0].indices, vec![0, 1, 2]);
    let mat = &m.materials[0].params;
    assert_eq!((mat.metallic, mat.roughness, mat.clearcoat), (0.25, 0.75, 0.8));
    assert_eq!(m.primitives[0].variants, vec![("red".to_string(), 1)]);
    // normals computed facing +z (counter-clockwise in glTF's y-up space)
    assert!((m.primitives[0].vertices[0].normal[2] - 1.0).abs() < 1e-5);
    let (locals, weights) = anim::pose(&m, m.animations.first(), 0.5);
    assert_eq!(locals[0].t, Vec3::ZERO, "the only key (t = 1) holds before it: translation 0,0,0");
    let items = anim::draw_list(&m, &locals, &weights, None);
    assert_eq!(items.len(), 1);
    // basis: 100 px per metre, y and z flipped
    let (lo, hi) = m.bounds();
    assert!((hi.x - 200.0).abs() < 1e-3 && (lo.y + 100.0).abs() < 1e-3, "{lo} {hi}");
}

#[test]
fn obj_ply_and_splats() {
    tmp("m.mtl", b"newmtl gold\nKd 1 0.8 0.2\nNs 100\nd 1\n");
    let obj = tmp("q.obj", b"mtllib m.mtl\no quad\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl gold\nf 1/1 2/2 3/3 4/4\n");
    let Asset::Model(m) = import::load(&obj, None).unwrap() else { panic!() };
    assert_eq!(m.primitives[0].indices.len(), 6);
    assert_eq!(m.materials[0].params.base_color[0], 1.0);
    assert!(m.materials[0].params.roughness < 0.2);
    let ply = tmp("t.ply", b"ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n");
    let Asset::Model(m) = import::load(&ply, None).unwrap() else { panic!() };
    assert_eq!(m.primitives[0].indices, vec![0, 1, 2]);
    // binary 3DGS PLY
    let mut gs = b"ply\nformat binary_little_endian 1.0\nelement vertex 1\n".to_vec();
    for p in [
        "x", "y", "z", "f_dc_0", "f_dc_1", "f_dc_2", "opacity", "scale_0", "scale_1", "scale_2", "rot_0", "rot_1",
        "rot_2", "rot_3",
    ] {
        gs.extend_from_slice(format!("property float {p}\n").as_bytes());
    }
    gs.extend_from_slice(b"end_header\n");
    for v in [1.0f32, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0f32.ln(), 2.0f32.ln(), 1.0, 0.0, 0.0, 0.0] {
        gs.extend_from_slice(&v.to_le_bytes());
    }
    let Asset::Splats(s) = import::load(&tmp("g.ply", &gs), None).unwrap() else { panic!("splats") };
    assert_eq!(s.pos[0], [1.0, 2.0, 3.0]);
    assert!((s.scale[0][1] - 1.0).abs() < 1e-5 && (s.scale[0][2] - 2.0).abs() < 1e-5);
    assert!((s.color[0][3] - 0.5).abs() < 1e-5, "sigmoid(0) = 0.5");
    assert!((s.color[0][0] - sr_3d::material::srgb_to_linear(0.5)).abs() < 1e-5);
    let mut sp = Vec::new();
    for v in [1.0f32, 2.0, 3.0, 0.5, 0.5, 0.5] {
        sp.extend_from_slice(&v.to_le_bytes());
    }
    sp.extend_from_slice(&[255, 0, 0, 128, 255, 128, 128, 128]);
    let s = import::splat(&sp).unwrap();
    assert_eq!(s.len(), 1);
    assert!((s.rot[0][3] - 1.0).abs() < 0.01, "w ≈ 1: {:?}", s.rot[0]);
    assert!(import::splat(&sp[..31]).is_err());
}

const USDA: &str = r#"#usda 1.0
(
    upAxis = "Z"
    metersPerUnit = 1
)
def Xform "World"
{
    double3 xformOp:translate = (0, 0, 1)
    uniform token[] xformOpOrder = ["xformOp:translate"]
    def Mesh "Quad" (
        prepend apiSchemas = ["MaterialBindingAPI"]
    )
    {
        point3f[] points = [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]
        int[] faceVertexCounts = [4]
        int[] faceVertexIndices = [0, 1, 2, 3]
        rel material:binding = </World/Mat>
    }
    def Material "Mat"
    {
        token outputs:surface.connect = </World/Mat/S.outputs:surface>
        def Shader "S"
        {
            uniform token info:id = "UsdPreviewSurface"
            color3f inputs:diffuseColor = (0.1, 0.2, 0.3)
            float inputs:roughness = 0.25
            token outputs:surface
        }
    }
}
"#;

/// A stored (uncompressed) zip with one entry.
fn zip_one(name: &str, data: &[u8]) -> Vec<u8> {
    let crc = {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
            }
        }
        !c
    };
    let mut z = Vec::new();
    let le16 = |v: u16| v.to_le_bytes();
    let le32 = |v: u32| v.to_le_bytes();
    z.extend_from_slice(&le32(0x0403_4b50));
    for v in [20u16, 0, 0, 0, 0] {
        z.extend_from_slice(&le16(v));
    }
    for v in [crc, data.len() as u32, data.len() as u32] {
        z.extend_from_slice(&le32(v));
    }
    z.extend_from_slice(&le16(name.len() as u16));
    z.extend_from_slice(&le16(0));
    z.extend_from_slice(name.as_bytes());
    z.extend_from_slice(data);
    let cd = z.len() as u32;
    z.extend_from_slice(&le32(0x0201_4b50));
    for v in [20u16, 20, 0, 0, 0, 0] {
        z.extend_from_slice(&le16(v));
    }
    for v in [crc, data.len() as u32, data.len() as u32] {
        z.extend_from_slice(&le32(v));
    }
    for v in [name.len() as u16, 0, 0, 0, 0] {
        z.extend_from_slice(&le16(v));
    }
    z.extend_from_slice(&le32(0));
    z.extend_from_slice(&le32(0));
    z.extend_from_slice(name.as_bytes());
    let cd_len = z.len() as u32 - cd;
    z.extend_from_slice(&le32(0x0605_4b50));
    for v in [0u16, 0, 1, 1] {
        z.extend_from_slice(&le16(v));
    }
    z.extend_from_slice(&le32(cd_len));
    z.extend_from_slice(&le32(cd));
    z.extend_from_slice(&le16(0));
    z
}

#[test]
fn usd_stages_convert_axes_units_and_materials() {
    for (name, bytes) in [("q.usda", USDA.as_bytes().to_vec()), ("q.usdz", zip_one("q.usda", USDA.as_bytes()))] {
        let Asset::Model(m) = import::load(&tmp(name, &bytes), None).unwrap_or_else(|e| panic!("{name}: {e}")) else {
            panic!()
        };
        assert_eq!(m.primitives[0].indices.len(), 6);
        let mat = &m.materials[m.primitives[0].material.expect("bound material")].params;
        assert_eq!(mat.base_color[..3], [0.1, 0.2, 0.3]);
        assert_eq!(mat.roughness, 0.25);
        // Z-up, 1 m per unit: the quad lies 1 m above the ground (−100 px in y), 1 m deep
        let (lo, hi) = m.bounds();
        assert!((lo.y + 100.0).abs() < 1e-3 && (hi.y + 100.0).abs() < 1e-3, "{lo} {hi}");
        assert!((hi.x - 100.0).abs() < 1e-3);
    }
    assert!(import::load(&tmp("b.usdc", b"PXR-USDC\0\0"), None).unwrap_err().contains("usdc"));
}

#[test]
fn fbx_through_ufbx() {
    let fbx = "; FBX 7.4.0 project file\nFBXHeaderExtension:  {\n\tFBXHeaderVersion: 1003\n\tFBXVersion: 7400\n}\nObjects:  {\n\tGeometry: 1000, \"Geometry::tri\", \"Mesh\" {\n\t\tVertices: *9 {\n\t\t\ta: 0,0,0,1,0,0,0,1,0\n\t\t}\n\t\tPolygonVertexIndex: *3 {\n\t\t\ta: 0,1,-3\n\t\t}\n\t}\n\tModel: 2000, \"Model::tri\", \"Mesh\" {\n\t}\n}\nConnections:  {\n\tC: \"OO\",1000,2000\n\tC: \"OO\",2000,0\n}\n";
    let Asset::Model(m) = import::load(&tmp("t.fbx", fbx.as_bytes()), None).unwrap() else { panic!() };
    assert_eq!(m.primitives.len(), 1);
    assert_eq!(m.primitives[0].indices.len(), 3);
}

#[test]
fn lights_environments_and_cameras() {
    let d65 = light::kelvin_to_rgb(6504.0);
    assert!(d65.iter().all(|c| *c > 0.9), "{d65:?}");
    let warm = light::kelvin_to_rgb(2000.0);
    assert!(warm[0] == 1.0 && warm[2] < 0.3, "{warm:?}");
    let ies = light::Ies::parse("IESNA:LM-63-2002\nTILT=NONE\n1 1000 1 3 1 1 2 0 0 0\n1 1 100\n0 45 90\n0\n100 50 0\n")
        .unwrap();
    assert!((ies.sample(22.5, 0.0) - 75.0).abs() < 1e-3);
    assert_eq!(ies.sample(120.0, 0.0), 0.0);
    let table = ies.bake(16, 4);
    assert_eq!(table[0], 1.0);
    // a constant environment stays constant through every mip and the SH
    let e = env::Equirect { width: 64, height: 32, rgb: vec![[0.5, 0.25, 1.0]; 64 * 32] };
    let pf = env::prefilter(&e, 64, 4);
    assert_eq!(pf.mips.len(), 4);
    for (_, _, px) in &pf.mips {
        for p in px.iter().step_by(37) {
            assert!((p[0] - 0.5).abs() < 1e-3 && (p[2] - 1.0).abs() < 1e-3, "{p:?}");
        }
    }
    let irr = env::sh_eval(&pf.sh, Vec3::new(0.3, -0.8, 0.5));
    assert!((irr.x - 0.5).abs() < 0.02 && (irr.z - 1.0).abs() < 0.04, "{irr}");
    let lut = env::brdf_lut(16);
    let smooth = lut[15];
    assert!((smooth[0] + smooth[1] - 1.0).abs() < 0.05, "{smooth:?}");
    // the default camera maps the z = 0 plane 1:1 onto the frame
    let cam = camera::resolve(&camera::CameraParams::default(), 1920.0, 1080.0);
    let ndc = |p: Vec3| {
        let c = cam.view_proj() * p.extend(1.0);
        c.truncate() / c.w
    };
    let a = ndc(Vec3::new(0.0, 0.0, 0.0));
    let b = ndc(Vec3::new(1920.0, 1080.0, 0.0));
    assert!((a.x + 1.0).abs() < 1e-4 && (a.y - 1.0).abs() < 1e-4, "{a}");
    assert!((b.x - 1.0).abs() < 1e-4 && (b.y + 1.0).abs() < 1e-4, "{b}");
    let near = ndc(cam.eye + Vec3::new(0.0, 0.0, 0.1));
    let far = ndc(cam.eye + Vec3::new(0.0, 0.0, 10000.0));
    assert!((near.z - 1.0).abs() < 1e-3 && far.z.abs() < 1e-3, "reverse z: {near} {far}");
    let look = camera::resolve(
        &camera::CameraParams {
            target: Some(Vec3::new(960.0, 540.0, 500.0)),
            offset: Vec3::new(300.0, 0.0, 0.0),
            ..Default::default()
        },
        1920.0,
        1080.0,
    );
    let c = look.view_proj() * Vec3::new(960.0, 540.0, 500.0).extend(1.0);
    assert!((c.x / c.w).abs() < 1e-4 && (c.y / c.w).abs() < 1e-4, "the target is centred");
    // positive pitch looks up (−y), positive yaw towards +x
    let up = camera::resolve(&camera::CameraParams { pitch: 30.0, ..Default::default() }, 100.0, 100.0);
    assert!(up.view.inverse().transform_vector3(Vec3::Z).y < -0.4);
    let right = camera::resolve(&camera::CameraParams { yaw: 30.0, ..Default::default() }, 100.0, 100.0);
    assert!(right.view.inverse().transform_vector3(Vec3::Z).x > 0.4);
    assert!((camera::fov_of_lens(camera::lens_of_fov(39.6, 36.0), 36.0) - 39.6).abs() < 1e-3);
    let (x, y, r, z) = camera::shake(10.0, 5.0, 0.0, [0.5, -0.25, 0.1, 0.3]);
    assert_eq!((x, y, r, z), (5.0, -2.5, 0.5, 1.0));
}

#[test]
fn materialx_standard_surface() {
    let doc = r#"<?xml version="1.0"?>
<materialx version="1.38" fileprefix="tex/">
  <image name="albedo" type="color3"><input name="file" type="filename" value="wood.png"/></image>
  <standard_surface name="S" type="surfaceshader">
    <input name="base_color" type="color3" nodename="albedo"/>
    <input name="metalness" type="float" value="0.9"/>
    <input name="specular_roughness" type="float" value="0.15"/>
    <input name="coat" type="float" value="1"/>
  </standard_surface>
  <surfacematerial name="M" type="material"><input name="surfaceshader" type="surfaceshader" nodename="S"/></surfacematerial>
</materialx>"#;
    let m = mtlx::parse(doc, std::path::Path::new("/assets")).unwrap();
    assert_eq!((m.params.metallic, m.params.roughness, m.params.clearcoat), (0.9, 0.15, 1.0));
    assert_eq!(m.base_color_map.as_deref(), Some(std::path::Path::new("/assets/tex/wood.png")));
    assert!(mtlx::parse("<materialx/>", std::path::Path::new(".")).is_err());
}

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

/// Bounding box and centroid of the distinct vertex positions of every named mesh node, in the
/// model's own space (Y-up metres), with the model posed at `t` in `clip`.
fn posed(m: &sr_3d::Model, clip: Option<&sr_3d::Animation>, t: f32) -> std::collections::BTreeMap<String, [Vec3; 3]> {
    let (locals, weights) = anim::pose(m, clip, t);
    let mut pts: std::collections::BTreeMap<String, std::collections::BTreeSet<[i64; 3]>> = Default::default();
    for item in anim::draw_list(m, &locals, &weights, None) {
        let vs = item.vertices.as_ref().unwrap_or(&m.primitives[item.prim].vertices);
        let set = pts.entry(m.nodes[item.node].name.clone()).or_default();
        for v in vs {
            let p = item.matrix.transform_point3(Vec3::from(v.pos));
            set.insert([p.x, p.y, p.z].map(|c| (c as f64 * 1e4).round() as i64));
        }
    }
    pts.into_iter()
        .map(|(name, set)| {
            let ps: Vec<Vec3> = set.iter().map(|p| Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32) / 1e4).collect();
            let lo = ps.iter().fold(Vec3::splat(f32::MAX), |a, b| a.min(*b));
            let hi = ps.iter().fold(Vec3::splat(f32::MIN), |a, b| a.max(*b));
            let c = ps.iter().copied().sum::<Vec3>() / ps.len() as f32;
            (name, [lo, hi, c])
        })
        .collect()
}

#[test]
fn fbx_animation_stacks_skins_and_blend_shapes() {
    // tools/fixtures/make_fbx_rig.py: Blender's evaluated vertices are the reference
    let Asset::Model(m) = import::load(&fixture("rig.fbx"), None).unwrap() else { panic!() };
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    assert_eq!(m.animations.len(), 1, "{:?}", m.animations.iter().map(|a| &a.name).collect::<Vec<_>>());
    let clip = &m.animations[0];
    assert!((clip.duration - 1.0).abs() < 1e-3, "{}", clip.duration);
    assert_eq!(m.skins.len(), 1);
    let want: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("rig.expected.json")).unwrap()).unwrap();
    let v3 = |v: &serde_json::Value| {
        Vec3::new(v[0].as_f64().unwrap() as f32, v[1].as_f64().unwrap() as f32, v[2].as_f64().unwrap() as f32)
    };
    for (key, t) in [("0.0", 0.0), ("0.5", 0.5), ("1.0", 1.0)] {
        let got = posed(&m, Some(clip), t);
        for (name, w) in want[key].as_object().unwrap() {
            let [lo, hi, c] = got.get(name).unwrap_or_else(|| panic!("no mesh node {name}: {:?}", got.keys()));
            for (what, g, e) in
                [("min", lo, v3(&w["min"])), ("max", hi, v3(&w["max"])), ("centroid", c, v3(&w["centroid"]))]
            {
                assert!((*g - e).abs().max_element() < 2e-3, "{name} {what} at t = {t}: {g} vs Blender {e}");
            }
        }
    }
    // past the end the last pose holds
    let (end, after) = (posed(&m, Some(clip), 1.0), posed(&m, Some(clip), 3.0));
    for (name, e) in &end {
        assert!((after[name][2] - e[2]).abs().max_element() < 1e-4, "{name}");
    }
}

#[test]
fn usd_text_binary_and_packaged_import_alike() {
    // tools/fixtures/make_usd_stage.py writes one stage three ways, and USD's own world-space points
    let want: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("stage.expected.json")).unwrap()).unwrap();
    let load = |name: &str| match import::load(&fixture(name), None).unwrap_or_else(|e| panic!("{name}: {e}")) {
        Asset::Model(m) => m,
        _ => panic!("{name}: not a model"),
    };
    let text = load("stage.usda");
    for (name, m) in [("stage.usda", &text), ("stage.usdc", &load("stage.usdc")), ("stage.usdz", &load("stage.usdz"))] {
        assert!(m.warnings.iter().any(|w| w.contains("time-sampled")), "{name}: {:?}", m.warnings);
        // every mesh's points, as USD places them in the world (stage units, before the basis)
        let locals: Vec<sr_3d::Trs> = m.nodes.iter().map(|n| n.local).collect();
        let world = m.world_matrices(&locals);
        for (path, pts) in want.as_object().unwrap() {
            let ni = m.nodes.iter().position(|n| &n.name == path).unwrap_or_else(|| panic!("{name}: no {path}"));
            let got: Vec<Vec3> = m.nodes[ni]
                .primitives
                .iter()
                .flat_map(|&p| m.primitives[p].vertices.iter().map(|v| world[ni].transform_point3(Vec3::from(v.pos))))
                .collect();
            for p in pts.as_array().unwrap() {
                let e = Vec3::new(
                    p[0].as_f64().unwrap() as f32,
                    p[1].as_f64().unwrap() as f32,
                    p[2].as_f64().unwrap() as f32,
                );
                let d = got.iter().map(|g| (*g - e).length()).fold(f32::MAX, f32::min);
                assert!(d < 1e-3, "{name} {path}: USD point {e} is {d} from the nearest imported vertex");
            }
            for g in &got {
                assert!(
                    pts.as_array().unwrap().iter().any(|p| (Vec3::new(
                        p[0].as_f64().unwrap() as f32,
                        p[1].as_f64().unwrap() as f32,
                        p[2].as_f64().unwrap() as f32
                    ) - *g)
                        .length()
                        < 1e-3),
                    "{name} {path}: imported vertex {g} is not a USD point"
                );
            }
        }
        // and the binary layers import exactly as the text one
        assert_eq!(m.basis, text.basis, "{name}");
        assert_eq!(m.primitives.len(), text.primitives.len(), "{name}");
        for (a, b) in m.primitives.iter().zip(&text.primitives) {
            assert_eq!(a.indices, b.indices, "{name}");
            assert_eq!(
                a.material.map(|k| &m.materials[k].params),
                b.material.map(|k| &text.materials[k].params),
                "{name}"
            );
            for (u, v) in a.vertices.iter().zip(&b.vertices) {
                let d = (0..3)
                    .map(|k| (u.pos[k] - v.pos[k]).abs().max((u.normal[k] - v.normal[k]).abs()))
                    .fold(0.0, f32::max);
                let duv = (0..2).map(|k| (u.uv[k] - v.uv[k]).abs()).fold(0.0, f32::max);
                assert!(d < 1e-5 && duv < 1e-6, "{name}: {u:?} vs {v:?}");
            }
        }
    }
}

#[test]
fn usdc_values_decode_as_usd_reads_them() {
    // tools/fixtures/make_usdc_values.py: one attribute per coding (inlined scalars, vectors and
    // diagonal matrices, compressed int, int64 and float arrays, tables, strings, tokens, assets)
    use sr_3d::usdc::{self, Val};
    let specs = usdc::read(&std::fs::read(fixture("values.usdc")).unwrap()).unwrap();
    let want: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("values.expected.json")).unwrap()).unwrap();
    for (name, w) in want.as_object().unwrap() {
        let spec = specs.iter().find(|s| s.path == format!("/V.{name}")).unwrap_or_else(|| panic!("no /V.{name}"));
        let got = spec.get("default").unwrap_or_else(|| panic!("{name}: no default"));
        match (got, w) {
            (Val::Str(s) | Val::Token(s) | Val::Asset(s), serde_json::Value::String(e)) => assert_eq!(s, e, "{name}"),
            (Val::Tokens(t), serde_json::Value::Array(e)) => {
                assert_eq!(t, &e.iter().map(|x| x.as_str().unwrap().to_string()).collect::<Vec<_>>(), "{name}")
            }
            (Val::Nums(n) | Val::Array(n), serde_json::Value::Array(e)) => {
                assert_eq!(n.len(), e.len(), "{name}: {n:?}");
                for (g, e) in n.iter().zip(e) {
                    let e = e.as_f64().unwrap();
                    assert!((g - e).abs() <= 1e-12 * e.abs().max(1.0), "{name}: {g} vs {e}");
                }
            }
            (Val::Bool(b), serde_json::Value::Array(e)) => {
                assert_eq!(*b as u8 as f64, e[0].as_f64().unwrap(), "{name}")
            }
            (g, e) => panic!("{name}: {g:?} vs {e}"),
        }
    }
}

#[test]
fn splat_ply_spherical_harmonics_import_in_coefficient_order() {
    // 3DGS writes f_rest channel-major (15 red, then 15 green, then 15 blue coefficients)
    for (rest, degree) in [(9usize, 1u32), (24, 2), (45, 3)] {
        let mut gs = b"ply\nformat binary_little_endian 1.0\nelement vertex 1\n".to_vec();
        let mut names: Vec<String> =
            ["x", "y", "z", "nx", "ny", "nz", "f_dc_0", "f_dc_1", "f_dc_2"].map(String::from).to_vec();
        names.extend((0..rest).map(|k| format!("f_rest_{k}")));
        names
            .extend(["opacity", "scale_0", "scale_1", "scale_2", "rot_0", "rot_1", "rot_2", "rot_3"].map(String::from));
        for p in &names {
            gs.extend_from_slice(format!("property float {p}\n").as_bytes());
        }
        gs.extend_from_slice(b"end_header\n");
        let per = rest / 3;
        let mut vals = vec![0.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.1, 0.2, 0.3];
        // f_rest value encodes (channel, coefficient) so the mapping can be read back
        vals.extend((0..rest).map(|k| (k / per) as f32 * 100.0 + (k % per) as f32 + 1.0));
        vals.extend([0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
        for v in vals {
            gs.extend_from_slice(&v.to_le_bytes());
        }
        let Asset::Splats(s) = import::load(&tmp(&format!("sh{rest}.ply"), &gs), None).unwrap() else { panic!() };
        assert_eq!(s.sh_degree, degree);
        let c = s.sh[0];
        assert_eq!([c[0], c[1], c[2]], [0.1, 0.2, 0.3], "DC first");
        for j in 1..=per {
            for ch in 0..3 {
                assert_eq!(
                    c[j * 3 + ch],
                    ch as f32 * 100.0 + j as f32,
                    "degree {degree}: coefficient {j} channel {ch}"
                );
            }
        }
    }
}
