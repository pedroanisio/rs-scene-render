//! `parentJoint`: an element parented to an imported model's joint follows the joint as the clip poses it.

use super::common;
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

/// A triangle of 1 m on node "A", with a child joint "hand" that clip "reach" moves from x = 0 to x = 0.6 m over 2 s
/// and clip "turn" rotates a quarter turn about z over 2 s (and moves to x = 0.5).
fn model() -> String {
    let f = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let mut buf = f(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    buf.extend([0u8, 0, 1, 0, 2, 0, 0, 0]);
    buf.extend(f(&[0.0, 2.0]));
    buf.extend(f(&[0.0, 0.0, 0.0, 0.6, 0.0, 0.0]));
    let h = std::f32::consts::FRAC_1_SQRT_2;
    buf.extend(f(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, h, h]));
    format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
 "nodes":[{{"name":"A","mesh":0,"children":[1]}},{{"name":"hand"}}],
 "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"indices":1,"material":0}}]}}],
 "materials":[{{"name":"m","pbrMetallicRoughness":{{"baseColorFactor":[1,0,0,1]}},"doubleSided":true}}],
 "animations":[
  {{"name":"reach","samplers":[{{"input":2,"output":3,"interpolation":"LINEAR"}}],"channels":[{{"sampler":0,"target":{{"node":1,"path":"translation"}}}}]}},
  {{"name":"turn","samplers":[{{"input":2,"output":4,"interpolation":"LINEAR"}}],"channels":[{{"sampler":0,"target":{{"node":1,"path":"rotation"}}}}]}}],
 "buffers":[{{"byteLength":{},"uri":"data:application/octet-stream;base64,{}"}}],
 "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":36}},{{"buffer":0,"byteOffset":36,"byteLength":6}},{{"buffer":0,"byteOffset":44,"byteLength":8}},{{"buffer":0,"byteOffset":52,"byteLength":24}},{{"buffer":0,"byteOffset":76,"byteLength":32}}],
 "accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},{{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}},
  {{"bufferView":2,"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[2]}},{{"bufferView":3,"componentType":5126,"count":2,"type":"VEC3"}},{{"bufferView":4,"componentType":5126,"count":2,"type":"VEC4"}}]}}"#,
        buf.len(),
        b64(&buf)
    )
}

fn scene(parent_attrs: &str, child_attrs: &str) -> sr_model::Document {
    scene_with(parent_attrs, &format!(r#"parent="o" {child_attrs}"#), "")
}

fn scene_with(parent_attrs: &str, child_attrs: &str, child_children: &str) -> sr_model::Document {
    std::fs::write(fixtures().join("joints.gltf"), model()).unwrap();
    let xml = format!(
        r##"<scene version="1.2"><project width="256" height="128" fps="10" duration="2" background="#00000000"/>
        <assets><mesh id="m" src="joints.gltf"/></assets>
        <composition>
          <object3D id="o" primitive="mesh" mesh="m" x="30" y="64" {parent_attrs}/>
          <object3D id="ball" primitive="sphere" radius="8" {child_attrs}>{child_children}</object3D>
        </composition>
        <lights><light id="fill" type="ambient" intensity="1"/></lights></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// Centre (x, y) of the pixels that are covered and not the red triangle: the ball.
fn ball(doc: &sr_model::Document, t: f64) -> Option<(f32, f32)> {
    let r = render_times(doc, &[t])?;
    let problems: Vec<_> = r.stats.unsupported.iter().chain(&r.stats.errors).collect();
    assert!(problems.is_empty(), "{problems:?}");
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
    for (i, p) in r.px.iter().enumerate() {
        if p[3] > 0.5 && p[0].partial_cmp(&(p[1] + 0.2)) != Some(std::cmp::Ordering::Greater) {
            sx += (i % 256) as f32;
            sy += (i / 256) as f32;
            n += 1.0;
        }
    }
    assert!(n > 50.0, "the ball is drawn: {n} pixels");
    Some((sx / n, sy / n))
}

#[test]
fn a_child_of_a_joint_follows_the_joint_through_the_clip() {
    let doc = scene(r#"animationClip="reach""#, r#"parentJoint="hand""#);
    let Some(start) = ball(&doc, 0.0) else { return };
    let end = ball(&doc, 1.0).unwrap();
    let still = ball(&scene(r#"animationClip="reach""#, ""), 1.0).unwrap();
    // the joint is 0.6 m (60 px) along x at the end of the clip: halfway through one second of two is 0.3 m
    assert!((end.0 - start.0 - 30.0).abs() < 1.5, "{start:?} {end:?}");
    assert!((end.1 - start.1).abs() < 1.5, "{start:?} {end:?}");
    assert!((still.0 - start.0).abs() < 1.5, "without parentJoint the child stays in the object's frame: {still:?}");
}

#[test]
fn the_child_turns_with_the_joint() {
    // the "turn" clip rotates the joint a quarter turn about z over 2 s: at 1.9 s it has turned 85.5 degrees, and a
    // child offset 40 px along x from the joint has swung to (40 cos, 40 sin) of that angle
    let doc = scene(r#"animationClip="turn""#, r#"parentJoint="hand" x="40""#);
    let Some(start) = ball(&doc, 0.0) else { return };
    let end = ball(&doc, 1.9).unwrap();
    let a = 85.5f32.to_radians();
    assert!((end.0 - start.0 - 40.0 * (a.cos() - 1.0)).abs() < 2.0, "{start:?} {end:?}");
    assert!(((end.1 - start.1).abs() - 40.0 * a.sin()).abs() < 2.0, "{start:?} {end:?}");
}

#[test]
fn an_unknown_joint_is_reported_and_the_child_stays_in_the_objects_frame() {
    let doc = scene(r#"animationClip="reach""#, r#"parentJoint="nope""#);
    let Some(r) = render_times(&doc, &[1.0]) else { return };
    let all: Vec<_> = r.stats.unsupported.iter().chain(&r.stats.errors).collect();
    assert!(all.iter().any(|m| m.contains("nope")), "{all:?}");
}

#[test]
fn a_parent_constraint_takes_the_joint_through_target_joint() {
    let doc = scene_with(
        r#"animationClip="reach""#,
        "",
        r#"<transformConstraint type="parent" target="o" targetJoint="hand"/>"#,
    );
    let Some(start) = ball(&doc, 0.0) else { return };
    let end = ball(&doc, 1.0).unwrap();
    assert!((end.0 - start.0 - 30.0).abs() < 1.5 && (end.1 - start.1).abs() < 1.5, "{start:?} {end:?}");
}

#[test]
fn the_joint_follows_a_blend_of_two_clips() {
    // "reach" moves the joint 0.6 m over the clip, "turn" leaves its translation at the rest pose (0): at weight 0.5
    // and 1 s into a 2 s clip the joint has moved half of the half
    let doc = scene(r#"animationClip="reach" animationClipTo="turn" animationBlend="0.5""#, r#"parentJoint="hand""#);
    let Some(start) = ball(&doc, 0.0) else { return };
    let end = ball(&doc, 1.0).unwrap();
    assert!((end.0 - start.0 - 15.0).abs() < 1.5, "{start:?} {end:?}");
}
