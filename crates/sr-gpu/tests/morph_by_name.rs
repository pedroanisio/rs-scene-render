//! `<morph name weight>` on an object3D: morph targets by name, from the glTF's `extras.targetNames`.

mod common;
use common::*;

fn b64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= c.len() {
                s.push(T[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// A triangle of 1 m with two morph targets: "right" moves it +0.4 m in x, "up" +0.4 m in y; weights 0 at rest.
fn model(names: &str) -> String {
    let f = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let mut buf = f(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    buf.extend([0u8, 0, 1, 0, 2, 0, 0, 0]);
    buf.extend(f(&[0.4, 0.0, 0.0, 0.4, 0.0, 0.0, 0.4, 0.0, 0.0]));
    buf.extend(f(&[0.0, 0.4, 0.0, 0.0, 0.4, 0.0, 0.0, 0.4, 0.0]));
    format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
 "nodes":[{{"name":"A","mesh":0}}],
 "meshes":[{{"weights":[0,0],{names}"primitives":[{{"attributes":{{"POSITION":0}},"indices":1,"material":0,"targets":[{{"POSITION":2}},{{"POSITION":3}}]}}]}}],
 "materials":[{{"name":"m","pbrMetallicRoughness":{{"baseColorFactor":[1,0,0,1]}},"doubleSided":true}}],
 "buffers":[{{"byteLength":{},"uri":"data:application/octet-stream;base64,{}"}}],
 "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":36}},{{"buffer":0,"byteOffset":36,"byteLength":6}},{{"buffer":0,"byteOffset":44,"byteLength":36}},{{"buffer":0,"byteOffset":80,"byteLength":36}}],
 "accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},{{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}},
  {{"bufferView":2,"componentType":5126,"count":3,"type":"VEC3","min":[0.4,0,0],"max":[0.4,0,0]}},{{"bufferView":3,"componentType":5126,"count":3,"type":"VEC3","min":[0,0.4,0],"max":[0,0.4,0]}}]}}"#,
        buf.len(),
        b64(&buf)
    )
}

const NAMES: &str = r#""extras":{"targetNames":["right","up"]},"#;

fn scene(object_attrs: &str, morphs: &str) -> sr_model::Document {
    scene_with(NAMES, object_attrs, morphs)
}

fn scene_with(names: &str, object_attrs: &str, morphs: &str) -> sr_model::Document {
    // one file per variant: the tests run in parallel
    let file = if names.is_empty() { "morphs-plain.gltf" } else { "morphs-named.gltf" };
    std::fs::write(fixtures().join(file), model(names)).unwrap();
    let xml = format!(
        r##"<scene version="1.2"><project width="256" height="256" fps="10" duration="2" background="#00000000"/>
        <assets><mesh id="m" src="{file}"/></assets>
        <composition><object3D id="o" primitive="mesh" mesh="m" x="30" y="200" {object_attrs}>{morphs}</object3D></composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// The mean (x, y) of the covered pixels at time `t`.
fn centre(doc: &sr_model::Document, t: f64) -> Option<(f32, f32)> {
    let r = render_times(doc, &[t])?;
    let problems: Vec<_> = r.stats.unsupported.iter().chain(&r.stats.errors).collect();
    assert!(problems.is_empty(), "{problems:?}");
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
    for (i, p) in r.px.iter().enumerate() {
        if p[3] > 0.5 {
            sx += (i % 256) as f32;
            sy += (i / 256) as f32;
            n += 1.0;
        }
    }
    assert!(n > 100.0, "the model is drawn: {n} pixels");
    Some((sx / n, sy / n))
}

#[test]
fn a_named_morph_sets_its_target_and_leaves_the_others() {
    let Some(rest) = centre(&scene("", ""), 0.0) else { return };
    let right = centre(&scene("", r#"<morph name="right" weight="1"/>"#), 0.0).unwrap();
    let up = centre(&scene("", r#"<morph name="up" weight="1"/>"#), 0.0).unwrap();
    // 0.4 m is 40 px; y grows downward on screen, so "up" (+y in the model's axes) moves the other way
    assert!((right.0 - rest.0 - 40.0).abs() < 1.5 && (right.1 - rest.1).abs() < 1.5, "{rest:?} {right:?}");
    assert!((up.0 - rest.0).abs() < 1.5 && (up.1 - rest.1).abs() > 38.0, "{rest:?} {up:?}");
    let both = centre(&scene("", r#"<morph name="right" weight="0.5"/><morph name="up" weight="1"/>"#), 0.0).unwrap();
    assert!((both.0 - rest.0 - 20.0).abs() < 1.5 && (both.1 - up.1).abs() < 1.5, "{both:?}");
}

#[test]
fn the_weight_animates() {
    let keyed = scene(
        "",
        r#"<morph name="right"><animate property="weight"><key time="0" value="0"/><key time="1" value="1"/></animate></morph>"#,
    );
    let Some(start) = centre(&keyed, 0.0) else { return };
    let (mid, end) = (centre(&keyed, 0.5).unwrap(), centre(&keyed, 1.0).unwrap());
    assert!((mid.0 - start.0 - 20.0).abs() < 1.5 && (end.0 - start.0 - 40.0).abs() < 1.5, "{start:?} {mid:?} {end:?}");
}

#[test]
fn a_named_weight_wins_over_the_positional_list() {
    // morphWeights sets both targets; the named weight replaces its own target only
    let Some(list) = centre(&scene(r#"morphWeights="1 1""#, ""), 0.0) else { return };
    let named = centre(&scene(r#"morphWeights="1 1""#, r#"<morph name="right" weight="0"/>"#), 0.0).unwrap();
    assert!((list.0 - named.0 - 40.0).abs() < 1.5 && (list.1 - named.1).abs() < 1.5, "{list:?} {named:?}");
}

#[test]
fn an_unknown_name_is_reported_with_the_names_there_are() {
    let doc = scene("", r#"<morph name="blink" weight="1"/>"#);
    let Some(r) = render_times(&doc, &[0.0]) else { return };
    let notes: Vec<_> = r.stats.unsupported.iter().chain(&r.stats.errors).collect();
    assert!(notes.iter().any(|m| m.contains("blink") && m.contains("right") && m.contains("up")), "{notes:?}");
}

#[test]
fn a_model_without_target_names_reports_that_it_has_none() {
    let doc = scene_with("", "", r#"<morph name="right" weight="1"/>"#);
    let Some(r) = render_times(&doc, &[0.0]) else { return };
    let notes: Vec<_> = r.stats.unsupported.iter().chain(&r.stats.errors).collect();
    assert!(notes.iter().any(|m| m.contains("right") && m.contains("no named morph targets")), "{notes:?}");
}

#[test]
fn morph_needs_version_1_2() {
    std::fs::write(fixtures().join("morphs-named.gltf"), model(NAMES)).unwrap();
    let xml = r##"<scene version="1.1"><project width="64" height="64" fps="10" duration="1"/><assets><mesh id="m" src="morphs-named.gltf"/></assets><composition><object3D id="o" primitive="mesh" mesh="m"><morph name="right" weight="1"/></object3D></composition></scene>"##;
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: Some(fixtures()) };
    match sr_model::load_str(xml, &opts) {
        Err(sr_model::LoadError::Invalid(r)) => assert!(r.diagnostics.iter().any(|d| d.code == "V5"), "{r}"),
        other => panic!("{other:?}"),
    }
}
