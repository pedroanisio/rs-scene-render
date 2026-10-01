//! Malformed assets are import errors (or are repaired), never panics or runaway allocations.

use sr_3d::{anim, import, Animation, Asset, Channel, Interp, Model, Node, Path};

fn tmp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sr3d-malformed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

fn ply(props: &[&str], values: usize) -> Vec<u8> {
    let mut d = b"ply\nformat binary_little_endian 1.0\nelement vertex 1\n".to_vec();
    for p in props {
        d.extend_from_slice(format!("property float {p}\n").as_bytes());
    }
    d.extend_from_slice(b"end_header\n");
    for _ in 0..values {
        d.extend_from_slice(&0.5f32.to_le_bytes());
    }
    d
}

#[test]
fn splat_ply_without_the_sibling_properties_is_an_error() {
    let full =
        ["x", "y", "z", "f_dc_0", "f_dc_1", "f_dc_2", "scale_0", "scale_1", "scale_2", "rot_0", "rot_1", "rot_2"];
    // each header ends on a property whose siblings would be read past the row
    for props in [
        &["x", "y", "z", "rot_0", "scale_0", "f_dc_0"][..],
        &["x", "y", "z", "f_dc_0", "f_dc_1", "f_dc_2", "rot_0", "rot_1", "rot_2", "rot_3", "scale_0"][..],
        &full[..],
        &["x", "y", "z", "scale_0", "scale_1", "scale_2", "rot_0", "rot_1", "rot_2", "rot_3", "f_dc_0"][..],
    ] {
        let r = import::ply(&ply(props, props.len()));
        assert!(r.is_err(), "{props:?} must not import");
    }
}

#[test]
fn splat_ply_columns_are_found_by_name() {
    // siblings out of order: scale_2 before scale_0
    let props = [
        "x", "y", "z", "scale_2", "scale_1", "scale_0", "f_dc_0", "f_dc_1", "f_dc_2", "rot_0", "rot_1", "rot_2",
        "rot_3",
    ];
    let mut d = b"ply\nformat binary_little_endian 1.0\nelement vertex 1\n".to_vec();
    for p in props {
        d.extend_from_slice(format!("property float {p}\n").as_bytes());
    }
    d.extend_from_slice(b"end_header\n");
    for v in [0.0f32, 0.0, 0.0, 3f32.ln(), 2f32.ln(), 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    let Asset::Splats(s) = import::ply(&d).unwrap() else { panic!("splats") };
    for (got, want) in s.scale[0].iter().zip([1.0, 2.0, 3.0]) {
        assert!((got - want).abs() < 1e-5, "{:?}", s.scale[0]);
    }
}

#[test]
fn ply_normals_without_ny_nz_are_an_error() {
    let mut d =
        b"ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty float nx\n"
            .to_vec();
    d.extend_from_slice(b"element face 1\nproperty list uchar int vertex_indices\nend_header\n");
    d.extend_from_slice(b"0 0 0 0\n1 0 0 0\n0 1 0 0\n3 0 1 2\n");
    assert!(import::ply(&d).is_err());
}

#[test]
fn ply_list_counts_are_bounded_by_the_data() {
    // a list claiming 4 billion entries in a file of a few bytes
    let mut d =
        b"ply\nformat binary_little_endian 1.0\nelement face 1\nproperty list uint uint vertex_indices\nend_header\n"
            .to_vec();
    d.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(import::ply(&d).is_err());
    let d = b"ply\nformat ascii 1.0\nelement face 1\nproperty list uint uint vertex_indices\nend_header\n1e30 0 1 2\n";
    assert!(import::ply(d).is_err());
    // an element with no properties and a huge count reads nothing
    let d = b"ply\nformat ascii 1.0\nelement vertex 18446744073709551615\nend_header\n";
    assert!(import::ply(d).is_err());
}

#[test]
fn percent_escapes_before_multibyte_characters_do_not_split_them() {
    let json = r#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":4,"uri":"%aé.bin"}]}"#;
    let path = tmp("percent.gltf", json.as_bytes());
    let e = import::gltf(&path).expect_err("the buffer file does not exist");
    assert!(e.contains("%a\u{e9}.bin"), "the name is kept as written: {e}");
}

fn channel(times: Vec<f32>, values: Vec<f32>) -> Model {
    Model {
        nodes: vec![Node::default()],
        animations: vec![Animation {
            name: "a".into(),
            channels: vec![Channel { node: 0, path: Path::Translation, interp: Interp::Linear, times, values }],
            duration: 2.0,
        }],
        ..Default::default()
    }
}

#[test]
fn sampling_survives_nan_and_unsorted_key_times() {
    for times in [vec![f32::NAN, 1.0], vec![0.0, f32::NAN, 2.0], vec![2.0, 0.5, 1.0], vec![f32::NAN; 3]] {
        let n = times.len();
        let m = channel(times.clone(), (0..n * 3).map(|k| k as f32).collect());
        for t in [-1.0, 0.25, 0.75, 1.5, 3.0, f32::NAN] {
            let (locals, _) = anim::pose(&m, m.animations.first(), t);
            assert!(locals[0].t.to_array().iter().all(|v| !v.is_infinite()), "{times:?} at {t}");
        }
    }
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let v = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            out.push(if k <= c.len() { T[(v >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// A one-node glTF whose translation channel has the given key times; key k's value is (k, 0, 0).
fn gltf_keys(times: &[f32]) -> String {
    let mut buf = Vec::new();
    for t in times {
        buf.extend_from_slice(&t.to_le_bytes());
    }
    for k in 0..times.len() {
        for v in [k as f32, 0.0, 0.0] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
    }
    let (n, tb) = (times.len(), times.len() * 4);
    format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{}}],
 "buffers":[{{"byteLength":{},"uri":"data:application/octet-stream;base64,{}"}}],
 "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":{tb}}},{{"buffer":0,"byteOffset":{tb},"byteLength":{}}}],
 "accessors":[{{"bufferView":0,"componentType":5126,"count":{n},"type":"SCALAR"}},
   {{"bufferView":1,"componentType":5126,"count":{n},"type":"VEC3"}}],
 "animations":[{{"samplers":[{{"input":0,"output":1,"interpolation":"LINEAR"}}],"channels":[{{"sampler":0,"target":{{"node":0,"path":"translation"}}}}]}}]}}"#,
        buf.len(),
        b64(&buf),
        n * 12
    )
}

#[test]
fn gltf_import_drops_nan_keys_and_sorts_the_rest() {
    let m = import::gltf(&tmp("nan-keys.gltf", gltf_keys(&[2.0, f32::NAN, 1.0]).as_bytes())).unwrap();
    let ch = &m.animations[0].channels[0];
    assert_eq!(ch.times, vec![1.0, 2.0]);
    assert_eq!(ch.values, vec![2.0, 0.0, 0.0, 0.0, 0.0, 0.0], "values follow their keys");
    assert_eq!(m.animations[0].duration, 2.0);
    assert!(m.warnings.iter().any(|w| w.contains("key times")), "{:?}", m.warnings);
    let (locals, _) = anim::pose(&m, m.animations.first(), 1.5);
    assert!((locals[0].t.x - 1.0).abs() < 1e-6, "halfway from key 2 (x = 2) to key 0 (x = 0): {}", locals[0].t);
    // well-formed keys are untouched and unremarked
    let m = import::gltf(&tmp("good-keys.gltf", gltf_keys(&[0.0, 1.0]).as_bytes())).unwrap();
    assert_eq!(m.animations[0].channels[0].times, vec![0.0, 1.0]);
    assert!(m.warnings.is_empty(), "{:?}", m.warnings);
}

/// A one-triangle glTF: `count` positions declared over three stored ones, and the given indices.
fn gltf_triangle(count: usize, indices: [u16; 3]) -> String {
    let mut buf = Vec::new();
    for p in [[0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for v in p {
            buf.extend_from_slice(&v.to_le_bytes());
        }
    }
    for i in indices {
        buf.extend_from_slice(&i.to_le_bytes());
    }
    format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0}}],
 "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"indices":1}}]}}],
 "buffers":[{{"byteLength":{},"uri":"data:application/octet-stream;base64,{}"}}],
 "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":36}},{{"buffer":0,"byteOffset":36,"byteLength":6}}],
 "accessors":[{{"bufferView":0,"componentType":5126,"count":{count},"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},
   {{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}}]}}"#,
        buf.len(),
        b64(&buf)
    )
}

#[test]
fn gltf_accessors_reaching_past_their_data_are_an_error() {
    assert!(import::gltf(&tmp("ok.gltf", gltf_triangle(3, [0, 1, 2]).as_bytes())).is_ok());
    let e = import::gltf(&tmp("long.gltf", gltf_triangle(4000, [0, 1, 2]).as_bytes())).unwrap_err();
    assert!(e.contains("accessor 0"), "{e}");
}

#[test]
fn gltf_indices_past_the_vertices_are_an_error() {
    let e = import::gltf(&tmp("far.gltf", gltf_triangle(3, [0, 1, 9]).as_bytes())).unwrap_err();
    assert!(e.contains("index"), "{e}");
}
