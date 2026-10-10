//! SREP 74: validation of viewport3D. V15 (version 1.6), VP1 (the named camera is a child of the viewport) and VP3 (no
//! bodies inside a viewport), with the documents of the kit (`conformance/srep_cases/srep-0074.json`): its findings
//! cases fail with their code and its pixel cases are valid.
//!
//! Also the scope of the black-hole rules BH5 to BH8 of the engine (sr-core draft SREP 76): a viewport3D holds a 3D
//! scene of its own, so objects inside it do not refuse a geodesic camera of the document (BH6), a geodesic camera inside
//! it needs a hole in the same viewport (BH5), and each scope has at most one geodesic camera (BH8). A black hole needs
//! version 1.3 (BH1) and a viewport3D 1.6 (V15), so these documents also report V15 today; the rules run regardless.

use sr_model::model::{Node, Viewport3DChild};
use sr_model::{validate_str, LoadOptions};

fn codes(xml: &str) -> Vec<String> {
    let r = validate_str(xml, &LoadOptions::without_assets());
    let mut c: Vec<String> = r.diagnostics.iter().map(|d| d.code.clone()).collect();
    c.sort();
    c.dedup();
    c
}

/// A kit document: the scene of `srep-0074.json`, with `body` as its composition and `version`.
fn kit(version: &str, body: &str) -> String {
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<scene version="{version}">
<project width="640" height="360" fps="24" duration="1" background="#000000FF" seed="1"/>
<output id="still" path="out/frame_%04d.png" codec="png-sequence"/>
<materials><material id="m-red" baseColor="#FF0000FF" unlit="true" doubleSided="true"/><material id="m-green" baseColor="#00FF00FF" unlit="true" doubleSided="true"/></materials>
<composition>
{body}
</composition>
</scene>
"##
    )
}

const PLANE: &str = r#"<object3D id="p" primitive="plane" width="40" height="20" x="100" y="50" material="m-red"/>"#;

fn viewport(attrs: &str, inner: &str) -> String {
    format!(r#"<viewport3D id="v" width="200" height="100" x="100" y="100"{attrs}>{inner}</viewport3D>"#)
}

#[test]
fn the_kits_pixel_documents_are_valid() {
    let cases = [
        ("implicit-camera", viewport("", PLANE)),
        (
            "main-camera-does-not-reach",
            format!(r#"<camera id="c" x="1320" y="180" z="-554.256"/>{}"#, viewport("", PLANE)),
        ),
        (
            "own-camera",
            viewport(r#" camera="vc""#, &format!(r#"<camera id="vc" x="130.0" y="50.0" z="-173.205"/>{PLANE}"#)),
        ),
        (
            "viewport-camera-stays-inside",
            format!(
                r#"<object3D id="p" primitive="plane" width="40" height="20" x="500" y="300" material="m-red"/>{}"#,
                viewport(
                    "",
                    r#"<camera id="vc" x="900" y="900" z="-50"/><object3D id="q" primitive="plane" width="40" height="20" x="100" y="50" material="m-green"/>"#
                )
            ),
        ),
        (
            "clipped",
            viewport(
                "",
                r#"<object3D id="p" primitive="plane" width="100" height="20" x="0" y="50" material="m-red"/>"#,
            ),
        ),
    ];
    for (name, body) in cases {
        assert_eq!(codes(&kit("1.6", &body)), Vec::<String>::new(), "srep-0074-{name}");
    }
}

#[test]
fn srep_0074_version_gate() {
    assert_eq!(codes(&kit("1.5", &viewport("", PLANE))), ["V15"]);
    // and every earlier version
    for v in ["1.0", "1.1", "1.2", "1.3", "1.4"] {
        assert!(codes(&kit(v, &viewport("", PLANE))).contains(&"V15".to_string()), "{v}");
    }
}

#[test]
fn srep_0074_camera_must_be_inside() {
    let body = format!(r#"<camera id="c" x="0" y="0" z="-100"/>{}"#, viewport(r#" camera="c""#, PLANE));
    assert_eq!(codes(&kit("1.6", &body)), ["VP1"]);
}

#[test]
fn the_named_camera_is_a_child_of_that_viewport_not_of_another() {
    let a = viewport(r#" camera="vb""#, PLANE);
    let b = r#"<viewport3D id="w" width="10" height="10"><camera id="vb" x="5" y="5" z="-10"/></viewport3D>"#;
    assert_eq!(codes(&kit("1.6", &format!("{a}{b}"))), ["VP1"]);
}

#[test]
fn a_body_inside_a_viewport_is_refused() {
    let body = viewport(
        "",
        r#"<object3D id="p" primitive="sphere" radius="5" x="100" y="50"><rigidBody mass="1"/></object3D>"#,
    );
    assert_eq!(codes(&kit("1.6", &body)), ["VP3"]);
}

#[test]
fn a_viewport_holds_only_3d_nodes_and_cannot_nest() {
    // the content model: object3D, camera, particles3D and node behaviours; a layer, a shape or a nested viewport is a
    // structural error (the reason SREP 74 dropped VP2)
    for inner in [
        r##"<shape id="s" shape="rect" width="4" height="4" fill="#FFFFFF"/>"##,
        r#"<viewport3D id="w" width="10" height="10"/>"#,
        r#"<group id="g"/>"#,
    ] {
        assert!(codes(&kit("1.6", &viewport("", inner))).iter().any(|c| c.starts_with('S')), "{inner}");
    }
    let animated =
        viewport("", r#"<animate property="opacity"><key time="0" value="0"/><key time="1" value="1"/></animate>"#);
    assert_eq!(codes(&kit("1.6", &animated)), Vec::<String>::new());
}

#[test]
fn the_model_reaches_a_viewports_nodes_as_child_nodes() {
    let xml = kit(
        "1.6",
        &viewport(r#" camera="vc" lights="key""#, &format!(r#"<camera id="vc" x="1" y="2" z="-3"/>{PLANE}"#)),
    )
    .replace("</composition>", r#"</composition><lights><light id="key" type="ambient"/></lights>"#);
    let doc = sr_model::load_str(&xml, &LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    let Some(Node::Viewport3D(v)) = doc.node("v") else { panic!("no viewport3D") };
    assert_eq!((v.width, v.height, v.camera.as_deref()), (200, 100, Some("vc")));
    assert_eq!(v.lights.as_deref(), Some(&["key".to_string()][..]));
    assert!(v.children.iter().all(|c| matches!(c, Viewport3DChild::Node(_))));
    let kids: Vec<_> = Node::Viewport3D(v.clone()).child_nodes().map(|n| n.id().unwrap_or("").to_string()).collect();
    assert_eq!(kids, ["vc", "p"]);
    // the ids inside a viewport resolve like any node's
    assert!(matches!(doc.node("p"), Some(Node::Object3D(_))));
    assert!(matches!(doc.node("vc"), Some(Node::Camera(_))));
}

// ------------------------------------------------------------------ BH5 to BH8 in the scope of a viewport3D

/// The engine's black-hole document (version 1.3) with `inner` inside a viewport3D.
fn hole_with_viewport(inner: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="2"/><composition>
  <shape id="sky" shape="rect" x="0" y="0" width="64" height="64" fill="#000000"/>
  <camera id="eye" x="0" y="0" z="-60" geodesics="true"/>
  <viewport3D id="v" width="16" height="16">{inner}</viewport3D>
  <blackHole id="hole" mass="1" x="0" y="0" z="0"/>
  <accretionDisk id="disk" blackHole="hole" outerRadius="20" temperatureScale="6000"/>
</composition></scene>"##
    )
}

#[test]
fn bh6_ignores_the_objects_inside_a_viewport() {
    // without the scope, BH6 refused the document's geodesic camera for an object it never draws
    let xml =
        hole_with_viewport(r#"<object3D id="ball" primitive="sphere" radius="1"/><particles3D id="dust" rate="1"/>"#);
    assert_eq!(codes(&xml), ["V15"]);
    // the same object outside the viewport is still refused
    let outside = xml.replace("<viewport3D", r#"<object3D id="out" primitive="sphere" radius="1"/><viewport3D"#);
    assert_eq!(codes(&outside), ["BH6", "V15"]);
}

#[test]
fn bh5_needs_the_hole_in_the_cameras_own_viewport() {
    // the hole of the document is not in the viewport's scene, so a geodesic camera there has nothing to trace
    let xml = hole_with_viewport(r#"<camera id="eye2" x="0" y="0" z="-80" geodesics="true"/>"#);
    assert_eq!(codes(&xml), ["BH5", "V15"]);
}

#[test]
fn bh6_counts_the_objects_in_the_cameras_own_viewport() {
    let xml = hole_with_viewport(
        r#"<camera id="eye2" x="0" y="0" z="-80" geodesics="true"/><object3D id="ball" primitive="sphere" radius="1"/>"#,
    );
    assert_eq!(codes(&xml), ["BH5", "BH6", "V15"]);
}

#[test]
fn bh8_counts_the_geodesic_cameras_of_one_scope() {
    // one geodesic camera in the document and one in the viewport: one per scope, no BH8
    let one = hole_with_viewport(r#"<camera id="eye2" x="0" y="0" z="-80" geodesics="true"/>"#);
    assert!(!codes(&one).contains(&"BH8".to_string()));
    let two = hole_with_viewport(
        r#"<camera id="eye2" x="0" y="0" z="-80" geodesics="true"/><camera id="eye3" x="0" y="0" z="-90" geodesics="true"/>"#,
    );
    assert_eq!(codes(&two), ["BH5", "BH8", "V15"]);
}

#[test]
fn bh7_measures_from_the_hole_in_scope_only() {
    // the viewport camera sits on the document's hole; that hole is not in its scope, so BH5 refuses it, not BH7
    let xml = hole_with_viewport(r#"<camera id="eye2" x="0" y="0" z="1" geodesics="true"/>"#);
    assert_eq!(codes(&xml), ["BH5", "V15"]);
}

#[test]
fn w03_needs_a_geodesic_camera_in_the_holes_scope() {
    // the document's hole is drawn only by a geodesic camera of the document
    let xml = hole_with_viewport(r#"<camera id="eye2" x="0" y="0" z="-80" geodesics="true"/>"#)
        .replace(r#"<camera id="eye" x="0" y="0" z="-60" geodesics="true"/>"#, "");
    assert_eq!(codes(&xml), ["BH5", "V15", "W03"]);
}
