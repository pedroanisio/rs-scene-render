//! Batch 8: 3D objects in documents — primitives, materials, lights and shadows,
//! glass over 2D layers, imported meshes, cameras, 2.5D agreement and depth of field.

mod common;
use common::*;

/// A 128×128 scene with extra assets, materials, composition body and lights.
fn scene(assets: &str, materials: &str, body: &str, lights: &str) -> sr_model::Document {
    let assets_xml = ASSETS.replace("</assets>", &format!("{assets}</assets>"));
    let mats = if materials.is_empty() { String::new() } else { format!("<materials>{materials}</materials>") };
    let ls = if lights.is_empty() { String::new() } else { format!("<lights>{lights}</lights>") };
    let xml = format!(
        r##"<scene version="1.1"><project width="128" height="128" fps="10" duration="4" background="#00000000"/>{assets_xml}{mats}<composition>{body}</composition>{ls}</scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn problems(r: &Rendered) -> Vec<String> {
    r.stats.unsupported.iter().chain(&r.stats.errors).cloned().collect()
}

fn lum(p: [f32; 4]) -> f32 {
    p[0] + p[1] + p[2]
}

/// Centroid x of pixels with alpha above ½ on row `y`.
fn centre_x(r: &Rendered, y: u32) -> f32 {
    let xs: Vec<u32> = (0..r.size[0]).filter(|x| r.at(*x, y)[3] > 0.5).collect();
    xs.iter().sum::<u32>() as f32 / xs.len().max(1) as f32
}

const RED: &str = r##"<material id="rmat" baseColor="#FF0000" roughness="0.6"/>"##;

#[test]
fn a_sphere_with_a_material_under_default_lights() {
    let d = scene("", RED, r#"<object3D id="s" primitive="sphere" radius="40" x="64" y="64" material="rmat"/>"#, "");
    let Some(r) = render(&d) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    let c = r.at(64, 64);
    assert!(c[3] > 0.99 && c[0] > 0.1 && c[0] > c[1] * 5.0, "red and opaque: {c:?}");
    assert_eq!(r.at(2, 2)[3], 0.0);
    assert_eq!(r.stats.objects3d, 1);
    assert!(r.stats.triangles > 1000);
}

#[test]
fn lights_cast_shadows_on_other_objects() {
    let body = r#"<object3D id="s" primitive="sphere" radius="15" x="64" y="64"/>
        <object3D id="p" primitive="plane" width="400" height="400" x="64" y="64" z="60"/>"#;
    let lights = r#"<light id="sun" type="directional" intensity="3" yaw="45" castShadow="true"/><light id="amb" type="ambient" intensity="0.02"/>"#;
    // the sample points were laid out for a 39.6° camera on the frame axis (absolute position)
    let body = format!(r#"<camera id="cam" x="64" y="64" z="-177.77" fov="39.6"/>{body}"#);
    let Some(r) = render(&scene("", "", &body, lights)) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    let (shadow, open) = (lum(r.at(64 + 55, 64)), lum(r.at(64 - 45, 64)));
    assert!(open > 0.1 && shadow < open * 0.5, "shadow {shadow} vs open {open}");
}

#[test]
fn glass_refracts_the_layers_behind_it() {
    let mats = r##"<material id="glass" transmission="1" roughness="0.05" ior="1.5" thickness="5"/>"##;
    let body = r#"<layer id="bg" asset="red" scaleX="32" scaleY="32"/>
        <object3D id="g" primitive="box" width="60" height="60" depth="5" x="64" y="64" material="glass"/>"#;
    let lights = r#"<light id="sun" type="directional" intensity="1" yaw="45"/>"#;
    let Some(r) = render(&scene("", mats, body, lights)) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    let c = r.at(64, 64);
    assert!(c[0] > 0.5 && c[1] < 0.2 && c[3] > 0.99, "{c:?}");
}

const TRI_GLTF: &str = r#"{"asset":{"version":"2.0"},"extensionsUsed":["KHR_materials_variants"],
 "extensions":{"KHR_materials_variants":{"variants":[{"name":"blue"}]}},
 "scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
 "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1,"material":0,
   "extensions":{"KHR_materials_variants":{"mappings":[{"material":1,"variants":[0]}]}}}]}],
 "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[0,1,0,1]},"doubleSided":true},{"pbrMetallicRoughness":{"baseColorFactor":[0,0,1,1]},"doubleSided":true}],
 "buffers":[{"byteLength":48,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAAABAAIAAAAAAIA/"}],
 "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":6}],
 "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}]}"#;

#[test]
fn imported_meshes_with_variants() {
    std::fs::write(fixtures().join("tri.gltf"), TRI_GLTF).unwrap();
    let asset = r#"<mesh id="tri" src="tri.gltf"/>"#;
    // 1 m triangle = 100 px, y up in glTF → pointing up from the origin
    let obj = |variant: &str| format!(r#"<object3D id="t" primitive="mesh" mesh="tri" x="30" y="100" {variant}/>"#);
    let Some(r) = render(&scene(asset, "", &obj(""), "")) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    let c = r.at(45, 85);
    assert!(c[3] > 0.99 && c[1] > c[2] && c[1] > c[0], "green default material: {c:?}");
    assert_eq!(r.at(100, 20)[3], 0.0, "outside the triangle");
    let Some(r) = render(&scene(asset, "", &obj(r#"materialVariant="blue""#), "")) else { return };
    let c = r.at(45, 85);
    assert!(c[2] > c[1] && c[2] > c[0], "blue variant: {c:?}");
}

#[test]
fn the_camera_moves_3d_objects_and_2_5d_layers_together() {
    let body = |cam: &str| {
        format!(
            r#"{cam}<object3D id="s" primitive="sphere" radius="12" x="40" y="40" z="200"/>
            <layer id="l" asset="red" threeD="true" zDepth="200" x="84" y="84" scaleX="4" scaleY="4"/>"#
        )
    };
    let Some(a) = render(&scene("", "", &body(""), "")) else { return };
    // the implicit camera's eye moved 30 px right; camera positions are absolute (conventions 2.3)
    let eye_z = -64.0 / (30f64.to_radians()).tan();
    let Some(b) = render(&scene("", "", &body(&format!(r#"<camera id="cam" x="94" y="64" z="{eye_z:.3}"/>"#)), ""))
    else {
        return;
    };
    assert!(problems(&b).is_empty(), "{:?}", problems(&b));
    // moving the camera right moves everything left, by the same amount at the same depth
    let cam = sr_3d::camera::resolve(&Default::default(), 128.0, 128.0);
    let row = |x: f32, y: f32| {
        let p = cam.view_proj() * glam::Vec4::new(x, y, 200.0, 1.0);
        ((0.5 - p.y / p.w * 0.5) * 128.0).round() as u32
    };
    let (s0, s1) = (centre_x(&a, row(40.0, 40.0)), centre_x(&b, row(40.0, 40.0)));
    let (l0, l1) = (centre_x(&a, row(86.0, 86.0)), centre_x(&b, row(86.0, 86.0)));
    assert!(s1 < s0 - 5.0, "sphere {s0} → {s1}");
    assert!(((s0 - s1) - (l0 - l1)).abs() < 1.5, "sphere shift {} vs layer shift {}", s0 - s1, l0 - l1);
}

#[test]
fn a_3d_object_and_a_2_5d_layer_at_the_same_depth_coincide() {
    let body = r#"<object3D id="s" primitive="sphere" radius="6" x="96" y="32" z="300"/>
        <layer id="l" asset="red" threeD="true" zDepth="300" x="88" y="88" scaleX="4" scaleY="4"/>"#;
    let Some(r) = render(&scene("", "", body, "")) else { return };
    // both centres sit on the same ray through the default camera: equal offsets from the frame centre
    let cam = sr_3d::camera::resolve(&Default::default(), 128.0, 128.0);
    let p = cam.view_proj() * glam::Vec4::new(96.0, 32.0, 300.0, 1.0);
    let (px, py) = ((p.x / p.w * 0.5 + 0.5) * 128.0, (0.5 - p.y / p.w * 0.5) * 128.0);
    let c = r.at(px as u32, py as u32);
    assert!(c[3] > 0.99, "the sphere projects where the camera says: ({px}, {py}) {c:?}");
    let q = cam.view_proj() * glam::Vec4::new(96.0, 96.0, 300.0, 1.0);
    let (qx, qy) = ((q.x / q.w * 0.5 + 0.5) * 128.0, (0.5 - q.y / q.w * 0.5) * 128.0);
    let l = (0..9).map(|k| r.at(qx as u32 + k % 3 - 1, qy as u32 + k / 3 - 1)).fold([0.0f32; 4], |m, p| {
        if p[3] > m[3] {
            p
        } else {
            m
        }
    });
    assert!(l[0] > 0.5 && l[3] > 0.99, "the 2.5D layer projects through the same camera: ({qx}, {qy}) {l:?}");
}

#[test]
fn extruded_text() {
    let body = r#"<object3D id="t" primitive="text" text="HI" height="60" depth="10" bevel="1" x="64" y="64"/>"#;
    let Some(r) = render(&scene("", "", body, "")) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    let covered = r.px.iter().filter(|p| p[3] > 0.5).count();
    assert!(covered > 600 && covered < 128 * 128 / 2, "{covered}");
}

#[test]
fn group_opacity_applies_to_3d_objects() {
    let body = r#"<group id="g" opacity="0.5"><object3D id="s" primitive="sphere" radius="30" x="64" y="64"/></group>"#;
    let Some(r) = render(&scene("", "", body, "")) else { return };
    let a = r.at(64, 64)[3];
    assert!((a - 0.5).abs() < 0.02, "{a}");
}

#[test]
fn camera_depth_of_field_blurs_out_of_focus_objects() {
    let body = |dof: &str| {
        format!(
            r#"<camera id="cam" {dof} focusDistance="100" fStop="1.4" focalLength="50"/><object3D id="s" primitive="sphere" radius="10" x="64" y="64" z="800"/>"#
        )
    };
    let Some(sharp) = render(&scene("", "", &body(""), "")) else { return };
    let Some(soft) = render(&scene("", "", &body(r#"depthOfField="true""#), "")) else { return };
    let cover = |r: &Rendered| r.px.iter().filter(|p| p[3] > 0.02).count();
    assert!(cover(&soft) > cover(&sharp) + 20, "{} vs {}", cover(&soft), cover(&sharp));
}

/// A 128×128 project rendered as 360 video (`scene360` attributes given).
fn scene360(s360: &str, body: &str) -> sr_model::Document {
    let mats = r##"<material id="m-red" baseColor="#FF0000" unlit="true"/><material id="m-blue" baseColor="#0000FF" unlit="true"/><material id="m-green" baseColor="#00FF00" unlit="true"/>"##;
    let xml = format!(
        r##"<scene version="1.1"><project width="128" height="128" fps="10" duration="4" background="#00000000"/>{ASSETS}<materials>{mats}</materials><scene360 {s360}/><composition>{body}</composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

// the default eye sits at (64, 64, −110.85) (60° horizontal): red ahead, blue behind, green above
const SPHERES: &str = r#"<object3D id="front" primitive="sphere" radius="60" x="64" y="64" z="300" material="m-red"/>
    <object3D id="back" primitive="sphere" radius="60" x="64" y="64" z="-700" material="m-blue"/>
    <object3D id="up" primitive="sphere" radius="60" x="64" y="-450" z="-111" material="m-green"/>"#;

fn is(p: [f32; 4], c: usize) -> bool {
    p[3] > 0.9 && p[c] > 0.5 && (0..3).filter(|k| *k != c).all(|k| p[k] < 0.2)
}

#[test]
fn equirectangular_360() {
    let Some(r) = render(&scene360(r#"layout="equirectangular" width="256" height="128""#, SPHERES)) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    assert_eq!(r.size, [256, 128]);
    assert!(is(r.at(128, 64), 0), "front: {:?}", r.at(128, 64));
    assert!(is(r.at(2, 64), 2) && is(r.at(253, 64), 2), "back at both edges: {:?} {:?}", r.at(2, 64), r.at(253, 64));
    assert!(is(r.at(40, 1), 1), "up along the top row: {:?}", r.at(40, 1));
    assert_eq!(r.at(64, 100)[3], 0.0, "nothing to the lower left");
    assert_eq!(r.stats.objects3d, 18, "three objects on each of six faces");
}

#[test]
fn stereo_top_bottom_has_parallax() {
    // 16 px in front of the default eye at z = −110.85
    let body = r#"<object3D id="near" primitive="sphere" radius="4" x="64" y="64" z="-95" material="m-red"/>"#;
    let Some(r) = render(&scene360(
        r#"layout="equirectangular" width="256" height="256" stereo="top-bottom" interpupillary="0.064""#,
        body,
    )) else {
        return;
    };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    let cx = |y: u32| {
        let xs: Vec<u32> = (0..256).filter(|x| r.at(*x, y)[3] > 0.5).collect();
        xs.iter().sum::<u32>() as f32 / xs.len().max(1) as f32
    };
    let (top, bottom) = (cx(64), cx(192));
    assert!(top > 100.0 && top < 156.0 && bottom > 100.0 && bottom < 156.0, "{top} {bottom}");
    assert!(top - bottom > 3.0, "the left eye (top) sees the near sphere further right: {top} vs {bottom}");
}

#[test]
fn fisheye_cubemap_and_eac_layouts() {
    let Some(f) = render(&scene360(r#"layout="fisheye-180" width="128" height="128""#, SPHERES)) else { return };
    assert!(is(f.at(64, 64), 0), "fisheye centre looks ahead: {:?}", f.at(64, 64));
    assert_eq!(f.at(1, 1)[3], 0.0, "outside the circle");
    let Some(c) = render(&scene360(r#"layout="cubemap" width="192" height="128""#, SPHERES)) else { return };
    assert!(is(c.at(96, 96), 0), "cubemap front tile (row 2, column 2): {:?}", c.at(96, 96));
    assert!(is(c.at(160, 96), 2), "cubemap back tile: {:?}", c.at(160, 96));
    assert!(is(c.at(160, 32), 1), "cubemap up tile: {:?}", c.at(160, 32));
    let Some(e) = render(&scene360(r#"layout="eac" width="192" height="128""#, SPHERES)) else { return };
    assert!(is(e.at(96, 32), 0), "EAC front tile (row 1, column 2): {:?}", e.at(96, 32));
    assert!(is(e.at(96, 96), 2), "EAC back tile: {:?}", e.at(96, 96));
}

#[test]
fn flat_layers_float_in_front_of_the_360_viewer() {
    let body = r#"<layer id="card" asset="red" x="48" y="48" scaleX="8" scaleY="8"/>"#;
    let Some(r) = render(&scene360(r#"layout="equirectangular" width="256" height="128""#, body)) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    let c = r.at(128, 64);
    assert!(c[3] > 0.9 && c[0] > 0.5, "the card is ahead: {c:?}");
    assert_eq!(r.at(20, 64)[3], 0.0, "and nowhere behind");
}

const UNLIT: &str = r##"<material id="u" baseColor="#00FF00" unlit="true" doubleSided="true"/>"##;

fn green(p: [f32; 4]) -> bool {
    p[3] > 0.99 && p[1] > 0.5 && p[0] < 0.1
}

#[test]
fn objects_parented_to_a_camera_take_its_3d_pose() {
    // on the camera's view axis, 100 units ahead: the frame centre, wherever the camera is and however it turns
    let body = r#"<camera id="cam" x="30" y="90" z="-150" yaw="20" pitch="10" roll="15"/>
        <object3D id="hud" primitive="plane" parent="cam" width="10" height="10" z="100" material="u"/>"#;
    let Some(r) = render(&scene("", UNLIT, body, "")) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    assert!(green(r.at(64, 64)), "{:?}", r.at(64, 64));
    assert!(!green(r.at(64, 80)) && !green(r.at(80, 64)), "10 units at 100 cover about 11 px");
}

#[test]
fn parented_cameras_are_placed_in_their_parents_frame() {
    // the rig turns 30° towards +x; the camera 50 units along the rig's forward axis sees the plane
    // 150 further along that axis at the frame centre
    let (s, c) = (30f32.to_radians().sin(), 30f32.to_radians().cos());
    let at = |d: f32| (64.0 + d * s, -200.0 + d * c);
    let (px, pz) = at(200.0);
    let body = format!(
        r#"<object3D id="rig" primitive="sphere" radius="1" visible="false" x="64" y="64" z="-200" rotationY="30"/>
        <camera id="cam" z="50"><transformConstraint type="parent" target="rig"/></camera>
        <object3D id="p" primitive="plane" width="10" height="10" x="{px}" y="64" z="{pz}" rotationY="30" material="u"/>"#
    );
    let Some(r) = render(&scene("", UNLIT, &body, "")) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    assert!(green(r.at(64, 64)), "{:?}", r.at(64, 64));
}

#[test]
fn the_dome_turns_by_its_yaw_and_pitch_and_decodes_srgb() {
    let dome = |angles: &str| {
        let lights = format!(r#"<light id="sky" type="dome" environment="sky.png" environmentVisible="true" {angles}/>"#);
        render(&scene("", "", r#"<object3D id="s" primitive="sphere" radius="1" x="0" y="0" z="-500"/>"#, &lights))
    };
    let red = |p: [f32; 4]| p[0] > 0.5 && p[1] < 0.1;
    let Some(r) = dome("") else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    assert!(red(r.at(64, 64)), "the image centre is +z: {:?}", r.at(64, 64));
    // 8-bit sky: code 128 is sRGB-decoded
    let g = r.at(10, 64);
    assert!((g[1] - lin8(128)).abs() < 0.02, "{g:?} vs {}", lin8(128));
    // positive yaw turns the environment towards +x: the patch (±17°) moves right by 30°
    let r = dome(r#"yaw="30""#).unwrap();
    assert!(!red(r.at(64, 64)) && red(r.at(120, 64)) && !red(r.at(10, 64)));
    // positive pitch turns it up by 20°: tan(20°)·110.85 ≈ 40 px above the centre
    let r = dome(r#"pitch="20""#).unwrap();
    assert!(!red(r.at(64, 64)) && red(r.at(64, 24)), "{:?} {:?}", r.at(64, 64), r.at(64, 24));
}

#[test]
fn look_at_constraints_aim_lights() {
    let body = r#"<object3D id="s" primitive="sphere" radius="30" x="64" y="64" z="0"/>"#;
    // a narrow spot to the left, pointing straight ahead (+z): it misses the sphere unless aimed
    let spot = |c: &str| {
        format!(r#"<light id="spot" type="spot" x="-300" y="64" z="-200" spotAngle="10" intensity="3000">{c}</light>"#)
    };
    let Some(off) = render(&scene("", "", body, &spot(""))) else { return };
    let on = render(&scene("", "", body, &spot(r#"<transformConstraint type="look-at" target="s"/>"#))).unwrap();
    assert!(problems(&on).is_empty(), "{:?}", problems(&on));
    let (a, b) = (lum(off.at(64, 64)), lum(on.at(64, 64)));
    assert!(b > a * 5.0 + 0.05, "aimed at the sphere: {b} vs {a}");
}

#[test]
fn motion_blurred_3d_objects_share_one_pass() {
    // two overlapping spheres, both moving under motion blur; the near red one is listed first. Blurred objects
    // used to render one pass each and composite in document order, so the far green one painted over it.
    let mats = r##"<material id="rm" baseColor="#FF0000" roughness="0.6"/><material id="gm" baseColor="#00FF00" roughness="0.6"/>"##;
    let body = r#"<object3D id="near" primitive="sphere" radius="14" x="64" y="64" z="-30" material="rm">
            <animate property="x"><key time="0" value="60"/><key time="4" value="68"/></animate></object3D>
        <object3D id="far" primitive="sphere" radius="40" x="64" y="64" z="60" material="gm">
            <animate property="x"><key time="0" value="68"/><key time="4" value="60"/></animate></object3D>"#;
    let mut d = scene("", mats, body, "");
    d.scene.project.motion_blur = true;
    d.scene.project.motion_blur_samples = 4;
    d.scene.project.shutter_angle = 180.0;
    let Some(r) = render_sub(&d, 1.0) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    assert!(r.stats.subframes >= 4, "motion blur ran: {}", r.stats.subframes);
    let c = r.at(64, 64);
    assert!(c[0] > c[1] * 3.0, "the near red sphere occludes the far green one: {c:?}");
    let edge = r.at(64, 64 - 24);
    assert!(edge[1] > edge[0], "the far sphere shows around the near one: {edge:?}");
}
