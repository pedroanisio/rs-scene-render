//! SREP 74: a viewport3D at the evaluator. It is a node of the viewport's width × height that clips (§4), its children
//! are its own 3D scene (§5), its cameras never become the document's camera, and its camera is the one `camera` names,
//! else its last active camera child, else none: the implicit camera (§2).

use sr_eval::{viewport_camera, viewport_of, Evaluator, FrameGraph};

fn graph(composition: &str, t: f64) -> FrameGraph {
    let xml = format!(
        r##"<scene version="1.6"><project width="640" height="360" fps="24" duration="2" background="#000000FF" seed="1"/>
  <materials><material id="m" baseColor="#FF0000FF" unlit="true" doubleSided="true"/></materials>
  <composition>{composition}</composition></scene>"##
    );
    let doc =
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    let ev = Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"));
    let g = ev.evaluate(t);
    assert!(g.failures.is_empty() && g.problems.is_empty(), "{:?} {:?}", g.failures, g.problems);
    g
}

fn index(g: &FrameGraph, id: &str) -> usize {
    g.nodes.iter().position(|n| &*n.id == id).unwrap_or_else(|| panic!("no node {id}"))
}

const PLANE: &str = r#"<object3D id="p" primitive="plane" width="40" height="20" x="100" y="50" material="m"/>"#;

#[test]
fn a_viewport_is_a_clipping_node_of_its_size_holding_its_scene() {
    let g = graph(&format!(r#"<viewport3D id="v" width="200" height="100" x="100" y="100">{PLANE}</viewport3D>"#), 0.0);
    let v = index(&g, "v");
    assert_eq!(g.nodes[v].kind, "viewport3D");
    assert_eq!(g.nodes[v].size, Some([200.0, 100.0]));
    assert!(g.nodes[v].clip);
    assert_eq!(g.nodes[v].world.apply([0.0, 0.0]), [100.0, 100.0]);
    let p = index(&g, "p");
    assert_eq!(g.nodes[p].parent, Some(v as u32));
    assert_eq!(viewport_of(&g.nodes, p), Some(v));
    assert_eq!(viewport_of(&g.nodes, v), None);
    // no camera: the implicit one (§2)
    assert_eq!(viewport_camera(&g, v), None);
    assert_eq!(g.camera, None);
}

#[test]
fn srep_0074_viewport_camera_stays_inside() {
    // the kit's case: the camera inside the viewport does not become the document's camera
    let g = graph(
        &format!(
            r#"<object3D id="q" primitive="plane" width="40" height="20" x="500" y="300" material="m"/>
            <viewport3D id="v" width="200" height="100" x="100" y="100"><camera id="vc" x="900" y="900" z="-50"/>{PLANE}</viewport3D>"#
        ),
        0.0,
    );
    assert_eq!(g.camera, None);
    assert_eq!(viewport_camera(&g, index(&g, "v")), Some(index(&g, "vc")));
}

#[test]
fn srep_0074_main_camera_does_not_reach() {
    // the document's camera stays the document's and the viewport keeps its implicit camera
    let g = graph(
        &format!(
            r#"<camera id="c" x="1320" y="180" z="-554.256"/><viewport3D id="v" width="200" height="100" x="100" y="100">{PLANE}</viewport3D>"#
        ),
        0.0,
    );
    assert_eq!(g.camera, Some(index(&g, "c") as u32));
    assert_eq!(viewport_camera(&g, index(&g, "v")), None);
}

#[test]
fn a_later_document_camera_is_not_hidden_by_a_viewport_camera_after_it() {
    let g = graph(
        r#"<camera id="c" x="0" y="0" z="-100"/><viewport3D id="v" width="20" height="10"><camera id="vc" x="0" y="0" z="-5"/></viewport3D>"#,
        0.0,
    );
    assert_eq!(g.camera, Some(index(&g, "c") as u32));
}

#[test]
fn the_viewport_camera_is_the_named_one_else_the_last_active_child() {
    let cams = r#"<camera id="a" x="1" y="0" z="-10"/><camera id="b" x="2" y="0" z="-10" start="1"/>"#;
    // b is active from t = 1: the last active camera child
    let g = graph(&format!(r#"<viewport3D id="v" width="20" height="10">{cams}</viewport3D>"#), 0.5);
    assert_eq!(viewport_camera(&g, index(&g, "v")), Some(index(&g, "a")));
    let g = graph(&format!(r#"<viewport3D id="v" width="20" height="10">{cams}</viewport3D>"#), 1.5);
    assert_eq!(viewport_camera(&g, index(&g, "v")), Some(index(&g, "b")));
    // @camera wins over the rule, and an inactive camera (active="false") is no candidate
    let g = graph(&format!(r#"<viewport3D id="v" width="20" height="10" camera="a">{cams}</viewport3D>"#), 1.5);
    assert_eq!(viewport_camera(&g, index(&g, "v")), Some(index(&g, "a")));
    let off = cams.replace(r#"start="1""#, r#"active="false""#);
    let g = graph(&format!(r#"<viewport3D id="v" width="20" height="10">{off}</viewport3D>"#), 1.5);
    assert_eq!(viewport_camera(&g, index(&g, "v")), Some(index(&g, "a")));
}
