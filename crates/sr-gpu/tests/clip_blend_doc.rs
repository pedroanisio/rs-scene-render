//! `animationClipTo`, `animationBlend` and `animationOffsetTo`: two clips of an imported model blended by a weight.

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

/// A triangle of 1 m on node "A" with two clips: "left" holds x = 0 and "right" holds x = 0.6 m (two keys each).
fn model() -> String {
    let f = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let mut buf = f(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    buf.extend([0u8, 0, 1, 0, 2, 0, 0, 0]);
    buf.extend(f(&[0.0, 2.0]));
    buf.extend(f(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]));
    buf.extend(f(&[0.6, 0.0, 0.0, 0.6, 0.0, 0.0]));
    format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
 "nodes":[{{"name":"A","mesh":0}}],
 "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"indices":1}}]}}],
 "materials":[{{"name":"m","pbrMetallicRoughness":{{"baseColorFactor":[1,0,0,1]}},"doubleSided":true}}],
 "animations":[
  {{"name":"left","samplers":[{{"input":2,"output":3,"interpolation":"LINEAR"}}],"channels":[{{"sampler":0,"target":{{"node":0,"path":"translation"}}}}]}},
  {{"name":"right","samplers":[{{"input":2,"output":4,"interpolation":"LINEAR"}}],"channels":[{{"sampler":0,"target":{{"node":0,"path":"translation"}}}}]}}],
 "buffers":[{{"byteLength":{},"uri":"data:application/octet-stream;base64,{}"}}],
 "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":36}},{{"buffer":0,"byteOffset":36,"byteLength":6}},{{"buffer":0,"byteOffset":44,"byteLength":8}},{{"buffer":0,"byteOffset":52,"byteLength":24}},{{"buffer":0,"byteOffset":76,"byteLength":24}}],
 "accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},{{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}},
  {{"bufferView":2,"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[2]}},{{"bufferView":3,"componentType":5126,"count":2,"type":"VEC3"}},{{"bufferView":4,"componentType":5126,"count":2,"type":"VEC3"}}]}}"#,
        buf.len(),
        b64(&buf)
    )
}

fn scene(attrs: &str, children: &str) -> sr_model::Document {
    std::fs::write(fixtures().join("clips.gltf"), model()).unwrap();
    let xml = format!(
        r##"<scene version="1.2"><project width="256" height="128" fps="10" duration="2" background="#00000000"/>
        <assets><mesh id="m" src="clips.gltf"/></assets>
        <composition><object3D id="o" primitive="mesh" mesh="m" x="30" y="100" {attrs}>{children}</object3D></composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// The mean x of the covered pixels at time `t`.
fn centre(doc: &sr_model::Document, t: f64) -> Option<f32> {
    let r = render_times(doc, &[t])?;
    let problems: Vec<_> = r.stats.unsupported.iter().chain(&r.stats.errors).collect();
    assert!(problems.is_empty(), "{problems:?}");
    let (mut sum, mut n) = (0.0, 0.0);
    for (i, p) in r.px.iter().enumerate() {
        if p[3] > 0.5 {
            sum += (i % 256) as f32;
            n += 1.0;
        }
    }
    assert!(n > 100.0, "the model is drawn: {n} pixels");
    Some(sum / n)
}

#[test]
fn the_weight_places_the_object_between_the_two_clips() {
    let both = |w: &str| scene(&format!(r#"animationClip="left" animationClipTo="right" animationBlend="{w}""#), "");
    let Some(left) = centre(&both("0"), 0.5) else { return };
    let (half, right) = (centre(&both("0.5"), 0.5).unwrap(), centre(&both("1"), 0.5).unwrap());
    // 0.6 m is 60 px
    assert!((right - left - 60.0).abs() < 1.5, "{left} {right}");
    assert!((half - left - 30.0).abs() < 1.5, "{left} {half}");
    let only_left = centre(&scene(r#"animationClip="left""#, ""), 0.5).unwrap();
    assert!((only_left - left).abs() < 0.01, "weight 0 is the first clip alone: {only_left} {left}");
}

#[test]
fn a_keyed_weight_cross_fades_over_time() {
    let keyed = scene(
        r#"animationClip="left" animationClipTo="right""#,
        r#"<animate property="animationBlend"><key time="0" value="0"/><key time="1" value="1"/></animate>"#,
    );
    let Some(start) = centre(&keyed, 0.0) else { return };
    let (mid, end) = (centre(&keyed, 0.5).unwrap(), centre(&keyed, 1.0).unwrap());
    assert!((mid - start - 30.0).abs() < 2.0 && (end - start - 60.0).abs() < 2.0, "{start} {mid} {end}");
}

#[test]
fn an_unknown_second_clip_is_reported() {
    let doc = scene(r#"animationClip="left" animationClipTo="nope" animationBlend="0.5""#, "");
    let Some(r) = render_times(&doc, &[0.5]) else { return };
    assert!(r.stats.errors.iter().any(|m| m.contains("nope")), "{:?}", r.stats.errors);
}
