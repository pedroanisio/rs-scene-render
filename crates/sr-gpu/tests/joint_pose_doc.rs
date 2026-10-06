//! `<joint name rotation… lookAt …>`: pose and aim a joint of an imported model from the document.

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
 "nodes":[{{"name":"A","children":[1]}},{{"name":"head","mesh":0}}],
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

fn scene(joints: &str, target: &str) -> sr_model::Document {
    std::fs::write(fixtures().join("joint-pose.gltf"), model()).unwrap();
    let xml = format!(
        r##"<scene version="1.2"><project width="256" height="256" fps="10" duration="2" background="#00000000"/>
        <assets><mesh id="m" src="joint-pose.gltf"/></assets>
        <composition>
          <object3D id="o" primitive="mesh" mesh="m" x="130" y="200">{joints}</object3D>
          <object3D id="ball" primitive="sphere" radius="8" parent="o" parentJoint="head" x="40"/>
          {target}
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
        if p[3] > 0.5 && !(p[0] > p[1] + 0.2) {
            sx += (i % 256) as f32;
            sy += (i / 256) as f32;
            n += 1.0;
        }
    }
    assert!(n > 50.0, "the ball is drawn: {n} pixels");
    Some((sx / n, sy / n))
}

/// The centre of the red triangle, which is drawn with the joint (it is the head node's mesh).
fn triangle(doc: &sr_model::Document, t: f64) -> Option<(f32, f32)> {
    let r = render_times(doc, &[t])?;
    let problems: Vec<_> = r.stats.unsupported.iter().chain(&r.stats.errors).collect();
    assert!(problems.is_empty(), "{problems:?}");
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
    for (i, p) in r.px.iter().enumerate() {
        if p[3] > 0.5 && p[0] > p[1] + 0.2 {
            sx += (i % 256) as f32;
            sy += (i / 256) as f32;
            n += 1.0;
        }
    }
    assert!(n > 100.0, "the triangle is drawn: {n} pixels");
    Some((sx / n, sy / n))
}

/// The triangle's centre (33.3, 33.3 up) turned by `deg` degrees about the joint at (130, 200).
fn turned(deg: f32) -> (f32, f32) {
    let (c, s) = (deg.to_radians().cos(), deg.to_radians().sin());
    let (x, y) = (100.0 / 3.0, 100.0 / 3.0);
    (130.0 + x * c - y * s, 200.0 - (x * s + y * c))
}

/// Where the ball sits relative to the joint, which is at the object's origin (30, 200).
fn offset(b: (f32, f32)) -> (f32, f32) {
    (b.0 - 130.0, b.1 - 200.0)
}

const TARGET_ABOVE: &str =
    r##"<shape id="t" shape="rect" x="130" y="100" width="2" height="2" opacity="0" fill="#FFFFFF"/>"##;

#[test]
fn a_joint_without_a_pose_leaves_the_ball_on_its_axis() {
    let Some(b) = ball(&scene("", ""), 0.0) else { return };
    let (dx, dy) = offset(b);
    assert!((dx - 40.0).abs() < 1.5 && dy.abs() < 1.5, "{b:?}");
}

#[test]
fn a_rotation_turns_the_joint_about_its_own_axes() {
    // a quarter turn about z: the ball 40 px along the joint's x goes to 40 px along its y (up the screen is -y)
    let doc = scene(r#"<joint name="head" rotation="90"/>"#, "");
    let Some(b) = ball(&doc, 0.0) else { return };
    let (dx, dy) = offset(b);
    assert!(dx.abs() < 1.5 && (dy.abs() - 40.0).abs() < 1.5, "{b:?}");
    // 45 degrees: both
    let (dx, dy) = offset(ball(&scene(r#"<joint name="head" rotation="45"/>"#, ""), 0.0).unwrap());
    assert!((dx - 28.28).abs() < 1.5 && (dy.abs() - 28.28).abs() < 1.5, "({dx}, {dy})");
}

#[test]
fn the_rotation_animates() {
    let doc = scene(
        r#"<joint name="head"><animate property="rotation"><key time="0" value="0"/><key time="1" value="90"/></animate></joint>"#,
        "",
    );
    let Some(start) = ball(&doc, 0.0) else { return };
    let mid = offset(ball(&doc, 0.5).unwrap());
    assert!((offset(start).0 - 40.0).abs() < 1.5);
    assert!((mid.0 - 28.28).abs() < 1.5 && (mid.1.abs() - 28.28).abs() < 1.5, "{mid:?}");
}

#[test]
fn look_at_points_the_joints_axis_at_the_target() {
    // the joint is at (130, 200) and the target at (130, 100): straight up the screen, a quarter turn from +x
    let doc = scene(r#"<joint name="head" lookAt="t" lookAxis="x"/>"#, TARGET_ABOVE);
    let Some(b) = triangle(&doc, 0.0) else { return };
    let want = turned(90.0);
    assert!((b.0 - want.0).abs() < 2.0 && (b.1 - want.1).abs() < 2.0, "{b:?} want {want:?}");
}

#[test]
fn influence_and_max_angle_limit_the_turn() {
    let at = |attrs: &str| {
        triangle(&scene(&format!(r#"<joint name="head" lookAt="t" lookAxis="x" {attrs}/>"#), TARGET_ABOVE), 0.0)
            .unwrap()
    };
    for (attrs, deg) in [(r#"influence="0.5""#, 45.0), (r#"maxAngle="30""#, 30.0), (r#"influence="0""#, 0.0)] {
        let (b, want) = (at(attrs), turned(deg));
        assert!((b.0 - want.0).abs() < 2.0 && (b.1 - want.1).abs() < 2.0, "{attrs}: {b:?} want {want:?}");
    }
}

#[test]
fn an_unknown_joint_is_reported_and_an_unknown_target_does_not_validate() {
    let Some(r) = render_times(&scene(r#"<joint name="nope" rotation="30"/>"#, ""), &[0.0]) else { return };
    let notes: Vec<_> = r.stats.unsupported.iter().chain(&r.stats.errors).collect();
    assert!(notes.iter().any(|m| m.contains("nope") && m.contains("head")), "{notes:?}");
    // @lookAt is an IDREF: a target that is no element is rejected by validation
    let xml = r##"<scene version="1.2"><project width="64" height="64" fps="10" duration="1"/><assets><mesh id="m" src="joint-pose.gltf"/></assets><composition><object3D id="o" primitive="mesh" mesh="m"><joint name="head" lookAt="missing"/></object3D></composition></scene>"##;
    match sr_model::load_str(xml, &sr_model::LoadOptions { verify_assets: false, base_dir: None }) {
        Err(sr_model::LoadError::Invalid(r)) => assert!(r.diagnostics.iter().any(|d| d.code == "S10"), "{r}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn joint_needs_version_1_2() {
    let xml = r##"<scene version="1.1"><project width="64" height="64" fps="10" duration="1"/><assets><mesh id="m" src="joint-pose.gltf"/></assets><composition><object3D id="o" primitive="mesh" mesh="m"><joint name="head" rotation="30"/></object3D></composition></scene>"##;
    match sr_model::load_str(xml, &sr_model::LoadOptions { verify_assets: false, base_dir: None }) {
        Err(sr_model::LoadError::Invalid(r)) => assert!(r.diagnostics.iter().any(|d| d.code == "V5"), "{r}"),
        other => panic!("{other:?}"),
    }
}
