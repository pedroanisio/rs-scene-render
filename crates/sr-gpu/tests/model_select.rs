//! `object3D/@node` and `@materialOverride` on an imported model.

mod common;
use common::*;

/// Two triangles of 1 m: node A (material "red") 0.6 m to the right of node B (material "green").
const MODEL: &str = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0,1]}],
 "nodes":[{"name":"A","mesh":0,"translation":[0.6,0,0]},{"name":"B","mesh":1}],
 "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1,"material":0}]},{"primitives":[{"attributes":{"POSITION":0},"indices":1,"material":1}]}],
 "materials":[{"name":"red","pbrMetallicRoughness":{"baseColorFactor":[1,0,0,1]},"doubleSided":true},{"name":"green","pbrMetallicRoughness":{"baseColorFactor":[0,1,0,1]},"doubleSided":true}],
 "buffers":[{"byteLength":48,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAAABAAIAAAAAAIA/"}],
 "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":6}],
 "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}]}"#;

fn scene(attrs: &str) -> sr_model::Document {
    std::fs::write(fixtures().join("two.gltf"), MODEL).unwrap();
    let xml = format!(
        r##"<scene version="1.2"><project width="256" height="128" fps="10" duration="2" background="#00000000"/>
        <assets><mesh id="two" src="two.gltf"/></assets>
        <materials><material id="blue" baseColor="#0000FF"/></materials>
        <composition><object3D id="m" primitive="mesh" mesh="two" x="30" y="100" {attrs}/></composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn dominant(c: [f32; 4]) -> char {
    if c[3] < 0.5 {
        '-'
    } else if c[0] > c[1] && c[0] > c[2] {
        'r'
    } else if c[1] > c[0] && c[1] > c[2] {
        'g'
    } else {
        'b'
    }
}

fn problems(r: &Rendered) -> Vec<String> {
    r.stats.unsupported.iter().chain(&r.stats.errors).cloned().collect()
}

#[test]
fn the_whole_model_draws_both_nodes() {
    let Some(r) = render(&scene("")) else { return };
    // B (green) at the object's origin, A (red) 60 px to the right
    assert_eq!(dominant(r.at(45, 85)), 'g');
    assert_eq!(dominant(r.at(150, 95)), 'r');
}

#[test]
fn node_draws_only_that_node_at_the_objects_origin() {
    let Some(a) = render(&scene(r#"node="A""#)) else { return };
    assert!(problems(&a).is_empty(), "{:?}", problems(&a));
    // A is drawn where B was: red at the origin, nothing 60 px to the right of it
    assert_eq!(dominant(a.at(45, 85)), 'r');
    assert_eq!(dominant(a.at(150, 95)), '-');
    let Some(b) = render(&scene(r#"node="B""#)) else { return };
    assert_eq!(dominant(b.at(45, 85)), 'g');
    assert_eq!(dominant(b.at(150, 95)), '-');
}

#[test]
fn an_unknown_node_draws_nothing_and_is_reported() {
    let Some(r) = render(&scene(r#"node="Nope""#)) else { return };
    assert!(r.px.iter().all(|p| p[3] < 0.01), "nothing is drawn");
    assert!(problems(&r).iter().any(|m| m.contains("Nope")), "{:?}", problems(&r));
}

#[test]
fn material_override_replaces_one_material_by_name() {
    let Some(r) = render(&scene(r#"materialOverride="red:blue""#)) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    assert_eq!(dominant(r.at(150, 95)), 'b', "A took the document material");
    assert_eq!(dominant(r.at(45, 85)), 'g', "B kept its own");
}

#[test]
fn material_takes_precedence_over_material_override() {
    let Some(r) = render(&scene(r#"material="blue" materialOverride="red:blue green:blue""#)) else { return };
    assert_eq!(dominant(r.at(150, 95)), 'b');
    let Some(s) = render(&scene(r#"material="blue" materialOverride="green:blue""#)) else { return };
    assert_eq!(dominant(s.at(45, 85)), 'b', "the override has no effect under @material");
}

#[test]
fn an_override_naming_no_material_of_the_model_warns_and_lists_the_names() {
    let Some(r) = render(&scene(r#"materialOverride="nosuch:blue red:blue""#)) else { return };
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    let warned: Vec<_> = r.stats.unsupported.iter().filter(|m| m.contains("nosuch")).collect();
    assert_eq!(warned.len(), 1, "one warning for the one unknown name: {:?}", r.stats.unsupported);
    assert!(warned[0].contains("red") && warned[0].contains("green"), "it lists the model's materials: {}", warned[0]);
    // the name that exists took effect
    assert_eq!(dominant(r.at(150, 95)), 'b');
    // no warning when every name exists
    let Some(ok) = render(&scene(r#"materialOverride="red:blue""#)) else { return };
    assert!(ok.stats.unsupported.iter().all(|m| !m.contains("materialOverride")), "{:?}", ok.stats.unsupported);
}
