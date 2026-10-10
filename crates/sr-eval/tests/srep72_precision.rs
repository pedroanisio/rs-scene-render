//! SREP 72: `project/@precision="f32"` covers the 2D working textures; a document that also draws 3D content is told
//! that the 3D renderer keeps 16-bit buffers (E23, information).

use sr_model::Severity;

fn warnings(precision: &str, composition: &str) -> Vec<sr_model::Diagnostic> {
    let xml = format!(
        r##"<scene version="1.6"><project width="64" height="64" fps="24" duration="1" {precision}/>
  <materials><material id="m" baseColor="#808080"/></materials><composition>{composition}</composition></scene>"##
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap_or_else(|e| panic!("{e}"));
    ev.warnings().iter().filter(|d| d.code == "E23").cloned().collect()
}

const SPHERE: &str = r#"<object3D id="ball" primitive="sphere" radius="10" x="32" y="32" material="m"/>"#;
const RECT: &str = r##"<shape id="r" shape="rect" width="10" height="10" fill="#FFFFFF"/>"##;

#[test]
fn f32_with_3d_content_is_reported_once_as_information() {
    let w = warnings(r#"precision="f32""#, &format!("{RECT}{SPHERE}{}", SPHERE.replace("ball", "ball2")));
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0].severity, Severity::Info);
    assert!(w[0].message.contains("16-bit") && w[0].message.contains("ball"), "{}", w[0].message);
}

#[test]
fn no_report_without_3d_content_or_at_f16() {
    assert!(warnings(r#"precision="f32""#, RECT).is_empty());
    assert!(warnings("", SPHERE).is_empty());
    assert!(warnings(r#"precision="f16""#, SPHERE).is_empty());
}
