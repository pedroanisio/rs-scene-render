//! A compiled document says whether it has anything the 3D pass draws, so the GPU adapter can be
//! chosen before the first frame.

use sr_eval::THREE_D_DRAWN;

fn program(nodes: &str) -> sr_eval::Evaluator {
    let xml = format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="10" duration="1"/><composition>{nodes}</composition></scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    sr_eval::Evaluator::new(&doc, &Default::default()).unwrap()
}

/// One minimal element per kind the 3D pass draws. The test below fails when the pass learns a kind this table lacks.
const KINDS: &[(&str, &str)] = &[
    ("object3D", r#"<object3D id="b" primitive="sphere" radius="10"/>"#),
    ("particles3D", r#"<particles3D id="p" rate="5" lifetime="1" dt="0.1"/>"#),
    ("ocean", r#"<ocean id="sea" width="4" depth="6" bottomDepth="2"/>"#),
];

#[test]
fn every_kind_the_3d_pass_draws_is_covered_here_and_asks_for_it() {
    let mut have: Vec<&str> = KINDS.iter().map(|k| k.0).collect();
    let mut want: Vec<&str> = THREE_D_DRAWN.to_vec();
    have.sort_unstable();
    want.sort_unstable();
    assert_eq!(have, want, "a kind was added to THREE_D_DRAWN without a case here (or the reverse)");
    for (kind, xml) in KINDS {
        assert!(program(xml).program().uses_3d(), "{kind}");
        assert!(sr_eval::draws_in_3d(kind), "{kind}");
    }
}

#[test]
fn a_document_with_3d_elements_says_so_wherever_they_sit() {
    for nodes in [
        r#"<group id="g"><group id="h"><object3D id="b" primitive="sphere" radius="10"/></group></group>"#,
        r#"<group id="g"><ocean id="sea" width="4" depth="6" bottomDepth="2"/></group>"#,
    ] {
        assert!(program(nodes).program().uses_3d(), "{nodes}");
    }
}

#[test]
fn a_3d_object_inside_an_instanced_symbol_counts() {
    let xml = r#"<scene version="1.3"><project width="64" height="64" fps="10" duration="1"/>
<symbols><symbol id="ball" width="32" height="32"><object3D id="b" primitive="sphere" radius="5"/></symbol></symbols>
<composition><instance id="i" symbol="ball"/></composition></scene>"#;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    assert!(sr_eval::Evaluator::new(&doc, &Default::default()).unwrap().program().uses_3d());
    // a symbol that is defined but never instanced draws nothing
    let unused = xml.replace(r#"<instance id="i" symbol="ball"/>"#, "");
    let doc = sr_model::load_str(&unused, &sr_model::LoadOptions::without_assets()).unwrap();
    assert!(!sr_eval::Evaluator::new(&doc, &Default::default()).unwrap().program().uses_3d());
}

#[test]
fn a_3d_object_brought_in_by_include_counts() {
    let dir = std::env::temp_dir().join(format!("sr-uses3d-include-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("lib.scene.xml"),
        r#"<scene version="1.3"><project width="64" height="64" fps="10" duration="1"/>
<symbols><symbol id="sea" width="32" height="32"><ocean id="o" width="4" depth="6" bottomDepth="2"/></symbol></symbols><composition/></scene>"#,
    )
    .unwrap();
    let main = r#"<scene version="1.3"><project width="64" height="64" fps="10" duration="1"/>
<composition><include id="inc" src="lib.scene.xml" symbol="sea"/></composition></scene>"#;
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: Some(dir.clone()) };
    let doc = sr_model::load_str(main, &opts).unwrap();
    assert!(sr_eval::Evaluator::new(&doc, &Default::default()).unwrap().program().uses_3d());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_flat_document_does_not_ask_for_the_3d_pass() {
    let ev = program(r##"<shape id="s" shape="rect" width="8" height="8" fill="#ffffff"/><group id="g"/>"##);
    assert!(!ev.program().uses_3d());
}
