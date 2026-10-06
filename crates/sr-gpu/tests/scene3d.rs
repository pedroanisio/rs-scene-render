//! 3D objects in documents — primitives, materials, lights and shadows,
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
fn a_plane_is_a_grid_of_segments() {
    // a displacement map moves vertices, so a plane that ignored `segments` could never be displaced
    let dflt = scene(
        "",
        RED,
        r#"<object3D id="p" primitive="plane" width="80" height="80" x="64" y="64" material="rmat"/>"#,
        "",
    );
    let grid = scene(
        "",
        RED,
        r#"<object3D id="p" primitive="plane" width="80" height="80" segments="16" x="64" y="64" material="rmat"/>"#,
        "",
    );
    let (Some(a), Some(b)) = (render(&dflt), render(&grid)) else { return };
    assert!(problems(&a).is_empty() && problems(&b).is_empty());
    assert_eq!(a.stats.triangles, 2 * 32 * 32, "the schema's default of 32 segments");
    assert_eq!(b.stats.triangles, 2 * 16 * 16);
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

const MORPH_GLTF: &str = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
 "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":2,"material":0,"targets":[{"POSITION":1}]}],"weights":[0]}],
 "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[0,1,0,1]},"doubleSided":true}],
 "buffers":[{"byteLength":80,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAM3MTL8AAAAAAAABAAIAAAA="}],
 "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":36},{"buffer":0,"byteOffset":72,"byteLength":6}],
 "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},
   {"bufferView":1,"componentType":5126,"count":3,"type":"VEC3","min":[0,-0.8,0],"max":[0,0,0]},
   {"bufferView":2,"componentType":5123,"count":3,"type":"SCALAR"}]}"#;

#[test]
fn keyed_morph_weights_move_the_mesh() {
    // the triangle's apex is 100 px above the origin; the morph target lowers it by 80 px at weight 1
    std::fs::write(fixtures().join("morph.gltf"), MORPH_GLTF).unwrap();
    let asset = r#"<mesh id="morph" src="morph.gltf"/>"#;
    let obj = |weights: &str| {
        format!(r#"<object3D id="m" primitive="mesh" mesh="morph" x="30" y="100">{weights}</object3D>"#)
    };
    let keyed = obj(r#"<animate property="morphWeights"><key time="0" value="0"/><key time="1" value="1"/></animate>"#);
    let Some(start) = render_times(&scene(asset, "", &keyed, ""), &[0.0]) else { return };
    let Some(end) = render_times(&scene(asset, "", &keyed, ""), &[1.0]) else { return };
    // near the apex (30, 0): covered at weight 0, empty at weight 1; near the base: covered in both
    assert!(start.at(32, 12)[3] > 0.99, "weight 0 reaches the apex: {:?}", start.at(32, 12));
    assert_eq!(end.at(32, 12)[3], 0.0, "weight 1 lowers it");
    assert!(end.at(40, 95)[3] > 0.99 && start.at(40, 95)[3] > 0.99);
    // the static attribute still works, and a key agrees with it
    let Some(attr) = render(&scene(asset, "", &obj("").replace("y=\"100\"", "y=\"100\" morphWeights=\"1\""), ""))
    else {
        return;
    };
    assert_eq!(attr.at(32, 12)[3], 0.0, "the attribute at weight 1");
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
fn a_masked_group_follows_the_camera_while_its_3d_content_stands_still() {
    // a mask isolates the group into a cached layer; nothing in it moves, but the camera dollies in.
    // Rendered after an earlier frame (as an encode does), the layer kept the first frame's projection.
    let body = r##"<camera id="cam" x="64" y="64" z="-221.7"><animate property="z"><key time="0" value="-221.7" interpolation="linear"/><key time="2" value="-60"/></animate></camera>
        <group id="g"><object3D id="p" primitive="plane" width="40" height="40" x="64" y="64" z="200" material="w"/>
        <layer id="l" asset="red" threeD="true" zDepth="200" x="20" y="20" scaleX="2" scaleY="2"/>
        <mask type="rect" x="0" y="0" width="128" height="128"/></group>"##;
    let d = scene("", r##"<material id="w" baseColor="#FFFFFF" unlit="true"/>"##, body, "");
    let Some(fresh) = render_times(&d, &[1.8]) else { return };
    let after = render_times(&d, &[0.0, 1.8]).unwrap();
    assert!(problems(&after).is_empty(), "{:?}", problems(&after));
    let lit = |r: &Rendered| r.px.iter().filter(|p| p[3] > 0.5).count();
    assert!(lit(&fresh) > 0, "the plane is in view");
    assert_eq!(lit(&after), lit(&fresh), "the layer follows the camera after an earlier frame");
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
        let lights =
            format!(r#"<light id="sky" type="dome" environment="sky.png" environmentVisible="true" {angles}/>"#);
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
fn default_lights_are_the_neutral_rig_and_ambient_reflects_on_metals() {
    // conventions 5.20: without <lights>, ambient 0.35 plus a key whose irradiance π · 0.65 brings a white
    // Lambertian surface facing it to 1; ambient is a uniform environment, so a metal reflects it
    let mats = r##"<material id="matte" baseColor="#FFFFFF" roughness="1"/><material id="chrome" baseColor="#FFFFFF" metallic="1" roughness="0.3"/>"##;
    let body = r#"<object3D id="w" primitive="sphere" radius="30" x="32" y="64" material="matte"/>
        <object3D id="m" primitive="sphere" radius="30" x="96" y="64" material="chrome"/>"#;
    let Some(r) = render(&scene("", mats, body, "")) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    // the lit side of the white sphere (towards the key, up and left) is near 1, its unlit side the ambient 0.35
    let lit = (0..60).map(|k| lum(r.at(10 + k % 20, 42 + k / 20 * 3)) / 3.0).fold(0.0f32, f32::max);
    assert!(lit > 0.85 && lit < 1.15, "lit side {lit}");
    let shade = lum(r.at(50, 84)) / 3.0;
    assert!(shade > 0.2 && shade < 0.45, "ambient side {shade}");
    let metal = lum(r.at(96, 64)) / 3.0;
    assert!(metal > 0.2, "a metal reflects the ambient environment: {metal}");
    let lights = r#"<light id="a" type="ambient" intensity="0.5"/>"#;
    let r = render(&scene("", mats, body, lights)).unwrap();
    assert!(lum(r.at(96, 64)) / 3.0 > 0.3, "ambient only: {:?}", r.at(96, 64));
}

#[test]
fn the_torus_tube_defaults_to_0_35_of_the_radius() {
    let mats = r##"<material id="u" baseColor="#FFFFFF" unlit="true" doubleSided="true"/>"##;
    // seen along its axis (rotationX 90 turns the ring into the frame plane): ring radius 40, tube 14
    let body = |h: &str| {
        format!(r#"<object3D id="t" primitive="torus" radius="40" {h} x="64" y="64" rotationX="90" material="u"/>"#)
    };
    let Some(r) = render(&scene("", mats, &body(""), "")) else { return };
    let row: Vec<u32> = (0..128).filter(|x| r.at(*x, 64)[3] > 0.5).collect();
    // on the centre row: the tube spans 26..54 px from the centre on each side
    assert!(row.contains(&(64 + 40)) && row.contains(&(64 + 28)) && !row.contains(&(64 + 22)), "{row:?}");
    let r = render(&scene("", mats, &body(r#"height="10""#), "")).unwrap();
    let row: Vec<u32> = (0..128).filter(|x| r.at(*x, 64)[3] > 0.5).collect();
    assert!(row.contains(&(64 + 40)) && !row.contains(&(64 + 33)), "height/2 is the tube radius: {row:?}");
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
    // render as one pass and composite by depth, so the near red one stays in front of the far green one.
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

#[test]
fn objects_in_front_of_the_2d_plane_draw_over_earlier_layers() {
    // object3D z is a depth: z < 0 is nearer the camera (the eye is at z = -111 here), not lower
    // in the paint order
    let mats = r##"<material id="green" baseColor="#00FF00" roughness="1"/>"##;
    let body = r#"<layer id="bg" asset="red" scaleX="32" scaleY="32"/>
        <object3D id="near" primitive="sphere" radius="16" x="64" y="64" z="-40" material="green"/>"#;
    let Some(r) = render(&scene("", mats, body, r#"<light id="a" type="ambient" intensity="1"/>"#)) else { return };
    let c = r.at(64, 64);
    assert!(c[1] > 0.5 && c[0] < 0.2, "the near sphere covers the layer: {c:?}");
}

#[test]
fn glass_in_front_of_the_2d_plane_refracts_the_layers_behind_it() {
    // a tinted pane nearer the camera than the 2D plane: the backdrop shows through, tinted,
    // so the pixel is neither the bare backdrop nor missing the backdrop
    let mats =
        r##"<material id="glass" baseColor="#8080FF" transmission="1" roughness="0.05" ior="1.5" thickness="5"/>"##;
    let body = r#"<layer id="bg" asset="white" scaleX="32" scaleY="32"/>
        <object3D id="g" primitive="box" width="60" height="60" depth="5" x="64" y="64" z="-50" material="glass"/>"#;
    let lights = r#"<light id="sun" type="directional" intensity="1" yaw="45"/>"#;
    let Some(r) = render(&scene("", mats, body, lights)) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    let c = r.at(64, 64);
    let edge = r.at(4, 4);
    assert!(edge[0] > 0.95 && edge[1] > 0.95 && edge[2] > 0.95, "backdrop outside the pane: {edge:?}");
    assert!(c[2] > c[0] + 0.1 && c[0] > 0.2, "the pane tints the backdrop blue: {c:?}");
}

#[test]
fn reused_render_targets_leave_no_trace_between_frames() {
    // the 3D pass keeps its targets between calls; a frame rendered after others (shadows,
    // transmission and depth of field all on, the object moving) matches a cold render
    let mats = r##"<material id="chrome" baseColor="#D0D0D8" metallic="1" roughness="0.3"/>
        <material id="glass" transmission="1" roughness="0.05" ior="1.5" thickness="5"/>"##;
    let body = r#"<layer id="bg" asset="checker" scaleX="8" scaleY="8"/>
        <camera id="cam" fov="60" x="64" y="64" z="-111" depthOfField="true" fStop="1.4" focusTarget="ball"/>
        <object3D id="ball" primitive="sphere" radius="20" x="64" y="64" z="40" material="chrome">
          <animate property="x"><key time="0" value="30"/><key time="2" value="100"/></animate>
        </object3D>
        <object3D id="pane" primitive="box" width="50" height="50" depth="4" x="64" y="64" z="-20" material="glass"/>"#;
    let lights = r#"<light id="key" type="spot" x="64" y="-100" z="-100" pitch="-45" spotAngle="60" intensity="400" castShadow="true" range="2000"/>"#;
    let d = scene("", mats, body, lights);
    let Some(cold) = render_times(&d, &[1.0]) else { return };
    let warm = render_times(&d, &[0.0, 1.9, 0.4, 1.0]).unwrap();
    assert!(problems(&warm).is_empty(), "{:?}", problems(&warm));
    let diff = cold
        .px
        .iter()
        .zip(&warm.px)
        .map(|(a, b)| (0..4).map(|c| (a[c] - b[c]).abs()).fold(0.0, f32::max))
        .fold(0.0, f32::max);
    assert!(diff < 1e-4, "warm frame differs from a cold one by {diff}");
    let moved = render_times(&d, &[0.0]).unwrap();
    assert!(cold.px != moved.px, "the scene must change between the frames compared");
}

#[test]
fn clay_blobs_merge_split_and_animate() {
    // a blob slides out of a body: one silhouette at first, two apart later (topology changes)
    let mats = r##"<material id="c" baseColor="#C8643C" roughness="0.8"/>"##;
    let body = r#"<object3D id="clay" primitive="clay" material="c" x="64" y="64" resolution="48" fingerprints="0.3" boil="12">
          <blob radius="22"/>
          <blob radius="12" blend="10"><animate property="x"><key time="0" value="14"/><key time="2" value="50"/></animate></blob>
        </object3D>"#;
    let lights = r#"<light id="a" type="ambient" intensity="1"/>"#;
    let d = scene("", mats, body, lights);
    let runs = |r: &Rendered| {
        // covered runs along the middle row
        let mut n = 0;
        let mut inside = false;
        for x in 0..128 {
            let on = r.at(x, 64)[3] > 0.5;
            if on && !inside {
                n += 1;
            }
            inside = on;
        }
        n
    };
    let Some(a) = render_times(&d, &[0.0]) else { return };
    assert!(problems(&a).is_empty(), "{:?}", problems(&a));
    let b = render_times(&d, &[2.0]).unwrap();
    assert_eq!(runs(&a), 1, "merged at t = 0");
    assert_eq!(runs(&b), 2, "split at t = 2");
}

/// A 128 × 128 set: the default eye as an explicit camera (so it can take the screen-space
/// options), a floor plane at y = 100 receding in depth, and `body`.
fn stage(cam: &str, mats: &str, body: &str, lights: &str) -> sr_model::Document {
    stage_on("fl", cam, mats, body, lights)
}

fn stage_on(floor: &str, cam: &str, mats: &str, body: &str, lights: &str) -> sr_model::Document {
    let body = format!(
        r#"<camera id="cam" fov="60" x="64" y="64" z="-110.85" {cam}/>
        <object3D id="ground" primitive="box" width="400" height="4" depth="600" x="64" y="102" z="150" material="{floor}"/>{body}"#
    );
    scene("", &format!(r##"<material id="fl" baseColor="#A0A0A0" roughness="0.9"/>{mats}"##), &body, lights)
}

fn lum3(p: [f32; 4]) -> f32 {
    p[0] + p[1] + p[2]
}

#[test]
fn directional_shadows_fit_what_the_camera_sees() {
    // a 20 000-unit floor makes a whole-scene shadow map 40 units a texel at 512: a 6-unit pole
    // would cast nothing; fitted to the view, its shadow shows on the floor behind it
    let body = r#"<object3D id="huge" primitive="box" width="20000" height="2" depth="20000" x="64" y="100" z="5000" material="fl"/>
        <object3D id="pole" primitive="box" width="6" height="60" depth="6" x="64" y="70" z="40" material="fl"/>"#;
    let lights = r#"<light id="a" type="ambient" intensity="0.2"/>
        <light id="sun" type="directional" intensity="2" pitch="-35" yaw="70" castShadow="true" shadowMapSize="512"/>"#;
    let d = scene("", r##"<material id="fl" baseColor="#C0C0C0" roughness="1"/>"##, body, lights);
    let Some(r) = render(&d) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    // the sun comes from the side (yaw 70), so the pole's shadow runs to the right of its foot
    // along the floor (rows 86 to 91 of this view), where the pole does not hide it
    let dark =
        (86..91).flat_map(|y| (70..110).map(move |x| (x, y))).map(|(x, y)| lum3(r.at(x, y))).fold(f32::MAX, f32::min);
    let lit = (86..91).map(|y| lum3(r.at(30, y))).fold(0.0, f32::max);
    assert!(dark < lit * 0.6, "a pole shadow on the floor: darkest {dark}, lit {lit}");
}

#[test]
fn ambient_occlusion_darkens_contacts_only() {
    let mats = r##"<material id="redm" baseColor="#C04030" roughness="0.8"/>"##;
    let body =
        r#"<object3D id="ball" primitive="sphere" radius="18" segments="48" x="64" y="82" z="40" material="redm"/>"#;
    let lights = r#"<light id="a" type="ambient" intensity="1"/>"#;
    let Some(off) = render(&stage("", mats, body, lights)) else { return };
    let on = render(&stage(r#"ambientOcclusion="true" aoRadius="12""#, mats, body, lights)).unwrap();
    assert!(problems(&on).is_empty(), "{:?}", problems(&on));
    // around the ball's foot (row 90, x 51 to 77 in this view) the floor darkens
    let darker = (84..100u32)
        .flat_map(|y| (40..90u32).map(move |x| (x, y)))
        .filter(|&(x, y)| lum3(on.at(x, y)) < lum3(off.at(x, y)) * 0.95)
        .count();
    assert!(darker > 40, "{darker} pixels darkened around the contact");
    // far from it nothing changes
    let (far_off, far_on) = (lum3(off.at(10, 120)), lum3(on.at(10, 120)));
    assert!((far_on - far_off).abs() < far_off * 0.02, "open floor: {far_off} -> {far_on}");
}

#[test]
fn contact_shadows_catch_what_the_shadow_map_misses() {
    // no shadow map at all: the contact shadow alone darkens the floor beside a cube (the sun
    // comes from the side, so the shadow is not hidden behind the cube)
    let body =
        r#"<object3D id="cube" primitive="box" width="16" height="16" depth="16" x="64" y="92" z="30" material="fl"/>"#;
    let sun = |extra: &str| {
        format!(
            r#"<light id="a" type="ambient" intensity="0.1"/><light id="sun" type="directional" intensity="2" pitch="-25" yaw="70" {extra}/>"#
        )
    };
    let Some(off) = render(&stage("", "", body, &sun(""))) else { return };
    let on = render(&stage("", "", body, &sun(r#"contactShadows="true" contactShadowLength="30""#))).unwrap();
    assert!(problems(&on).is_empty(), "{:?}", problems(&on));
    let changed = (0..128u32)
        .flat_map(|y| (0..128u32).map(move |x| (x, y)))
        .filter(|&(x, y)| lum3(on.at(x, y)) < lum3(off.at(x, y)) * 0.7)
        .count();
    assert!(changed > 20, "{changed} pixels shadowed");
    assert!((lum3(on.at(10, 120)) - lum3(off.at(10, 120))).abs() < 0.02, "open floor unchanged");
}

#[test]
fn screen_space_reflections_mirror_objects_in_glossy_floors() {
    let mats = r##"<material id="redm" baseColor="#FF2010" roughness="0.8"/><material id="gloss" baseColor="#303030" roughness="0.05"/>"##;
    let body =
        r#"<object3D id="ball" primitive="sphere" radius="14" segments="48" x="64" y="70" z="60" material="redm"/>"#;
    let glossy = |cam: &str| stage_on("gloss", cam, mats, body, r#"<light id="a" type="ambient" intensity="1"/>"#);
    let Some(off) = render(&glossy("")) else { return };
    let on = render(&glossy(r#"screenSpaceReflections="true""#)).unwrap();
    assert!(problems(&on).is_empty(), "{:?}", problems(&on));
    // below the ball, where its mirror image falls on the floor, red rises
    let redness = |r: &Rendered, x: u32, y: u32| r.at(x, y)[0] - r.at(x, y)[1];
    let gain = (100..112).map(|y| redness(&on, 64, y) - redness(&off, 64, y)).fold(0.0f32, f32::max);
    assert!(gain > 0.05, "reflection of the red ball: {gain}");
    assert!((lum3(on.at(10, 120)) - lum3(off.at(10, 120))).abs() < 0.03, "empty floor unchanged");
}

fn traced(cam_extra: &str, mats: &str, body: &str, lights: &str) -> sr_model::Document {
    let body = format!(r#"<camera id="cam" fov="60" x="64" y="64" z="-110.85" {cam_extra}/>{body}"#);
    scene("", mats, &body, lights)
}

const PT: &str = r#"renderer="pathtrace" pathSamples="64" maxBounces="4" denoise="false""#;

#[test]
fn path_tracing_matches_the_rasteriser_where_both_are_exact() {
    // a grey (linear 0.5) diffuse sphere; (1) lit by ambient light only it is a white furnace:
    // every path leaves after one bounce, so it returns albedo × ambient like the rasteriser;
    // (2) lit by a directional light only, both evaluate the same BRDF
    let mats = r##"<material id="g" baseColor="#BCBCBC" roughness="1"/>"##;
    let body = r#"<object3D id="s" primitive="sphere" radius="30" segments="96" x="64" y="64" z="0" material="g"/>"#;
    for lights in [
        r#"<light id="a" type="ambient" intensity="1"/>"#,
        r#"<light id="sun" type="directional" intensity="2" yaw="30" pitch="-40"/>"#,
    ] {
        let Some(r) = render(&traced("", mats, body, lights)) else { return };
        let p = render(&traced(PT, mats, body, lights)).unwrap();
        assert!(problems(&p).is_empty(), "{:?}", problems(&p));
        for (x, y) in [(64, 64), (54, 54), (74, 60), (64, 80)] {
            let (a, b) = (r.at(x, y), p.at(x, y));
            for c in 0..3 {
                assert!(
                    (a[c] - b[c]).abs() < 0.03 + 0.04 * a[c],
                    "{lights} at ({x}, {y}): raster {a:?} vs traced {b:?}"
                );
            }
            assert!((b[3] - 1.0).abs() < 1e-3, "covered");
        }
        assert_eq!(p.at(2, 2)[3], 0.0, "a primary miss is transparent");
    }
}

#[test]
fn path_tracing_is_deterministic_and_shadows_and_refracts() {
    let mats = r##"<material id="w" baseColor="#D0D0D0" roughness="0.9"/><material id="gl" transmission="1" roughness="0.02" ior="1.5"/>"##;
    let body = r#"<layer id="bg" asset="red" scaleX="32" scaleY="32"/>
        <object3D id="floor" primitive="box" width="300" height="4" depth="300" x="64" y="102" z="100" material="w"/>
        <object3D id="ball" primitive="sphere" radius="14" segments="48" x="40" y="70" z="20" material="w"/>
        <object3D id="pane" primitive="sphere" radius="16" segments="64" x="96" y="50" z="-10" material="gl"/>"#;
    let lights = r#"<light id="sun" type="directional" intensity="2" pitch="-80" castShadow="true"/>"#;
    let Some(a) = render(&traced(PT, mats, body, lights)) else { return };
    let b = render(&traced(PT, mats, body, lights)).unwrap();
    assert!(a.px == b.px, "the same frame twice");
    // under the ball (the sun is nearly overhead) the floor is in shadow
    let under = lum(a.at(40, 96));
    let open = lum(a.at(10, 110));
    assert!(under < open * 0.5, "shadow {under} vs open floor {open}");
    // through the glass the red layer behind shows, refracted: red, not black
    let g = a.at(96, 50);
    assert!(g[0] > 0.3 && g[0] > g[1] * 2.0, "through glass: {g:?}");
}

#[test]
fn path_tracing_denoiser_smooths_noise_and_keeps_the_mean() {
    // a floor under a sphere-area light at 4 samples: soft, noisy light; the à-trous filter
    // lowers the pixel-to-pixel variation and keeps the average
    let mats = r##"<material id="w" baseColor="#C0C0C0" roughness="0.9"/>"##;
    let body = r#"<object3D id="floor" primitive="box" width="400" height="4" depth="400" x="64" y="102" z="100" material="w"/>
        <object3D id="ball" primitive="sphere" radius="12" segments="32" x="64" y="84" z="40" material="w"/>"#;
    let lights = r#"<light id="l" type="sphere-area" x="64" y="20" z="20" radius="25" intensity="60" castShadow="true" range="2000"/>"#;
    let cam = |dn: bool| format!(r#"renderer="pathtrace" pathSamples="4" maxBounces="3" denoise="{dn}""#);
    let Some(noisy) = render(&traced(&cam(false), mats, body, lights)) else { return };
    let clean = render(&traced(&cam(true), mats, body, lights)).unwrap();
    let region: Vec<(u32, u32)> = (104..124).flat_map(|y| (20..108).map(move |x| (x, y))).collect();
    let stats = |r: &Rendered| {
        let v: Vec<f32> = region.iter().map(|&(x, y)| lum(r.at(x, y))).collect();
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        let tv = region.windows(2).map(|w| (lum(r.at(w[0].0, w[0].1)) - lum(r.at(w[1].0, w[1].1))).abs()).sum::<f32>()
            / v.len() as f32;
        (mean, tv)
    };
    let ((m0, tv0), (m1, tv1)) = (stats(&noisy), stats(&clean));
    assert!(tv1 < tv0 * 0.5, "variation {tv0} -> {tv1}");
    assert!((m1 - m0).abs() < 0.05 * m0, "mean {m0} -> {m1}");
}

#[test]
fn a_2d_node_between_3d_objects_ends_their_block() {
    // The first sphere and the second are separate blocks, composited in document order around the
    // full-frame blue rectangle between them: the rectangle hides the first, and the second shows over it,
    // wherever the first lies (here far away, as a set parked off-screen)
    let body = r##"<object3D id="far" primitive="sphere" radius="5" x="20000" y="64" material="rmat"/>
        <object3D id="under" primitive="sphere" radius="20" x="32" y="64" material="rmat"/>
        <shape id="wall" shape="rect" x="0" y="0" width="128" height="128" fill="#0000FF"/>
        <object3D id="over" primitive="sphere" radius="20" x="96" y="64" material="rmat"/>"##;
    let Some(r) = render(&scene("", RED, body, "")) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    let (under, over) = (r.at(32, 64), r.at(96, 64));
    assert!(under[2] > under[0] * 5.0, "the wall covers the earlier block: {under:?}");
    assert!(over[0] > over[2] * 5.0, "the later block is drawn over the wall: {over:?}");
}

#[test]
fn draft_quality_360_renders_its_faces_at_the_tier_size() {
    let mats = r##"<material id="m-red" baseColor="#FF0000" unlit="true"/><material id="m-blue" baseColor="#0000FF" unlit="true"/><material id="m-green" baseColor="#00FF00" unlit="true"/>"##;
    let xml = format!(
        r##"<scene version="1.1"><project width="128" height="128" fps="10" duration="4" background="#00000000" quality="draft"/>{ASSETS}<materials>{mats}</materials><scene360 layout="equirectangular" width="256" height="128"/><composition>{SPHERES}</composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    let d = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    let Some(r) = render(&d) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    assert_eq!(r.size, [256, 128]);
    // half-size faces soften the spheres' edges
    let mostly = |p: [f32; 4], c: usize| p[3] > 0.5 && p[c] > 0.5 && (0..3).filter(|k| *k != c).all(|k| p[k] < 0.2);
    assert!(mostly(r.at(128, 64), 0), "front: {:?}", r.at(128, 64));
    assert!(mostly(r.at(1, 64), 2) && mostly(r.at(254, 64), 2), "back: {:?} {:?}", r.at(1, 64), r.at(254, 64));
    assert!(mostly(r.at(40, 1), 1), "up along the top row: {:?}", r.at(40, 1));
    assert_eq!(r.at(64, 100)[3], 0.0, "nothing to the lower left");
}
