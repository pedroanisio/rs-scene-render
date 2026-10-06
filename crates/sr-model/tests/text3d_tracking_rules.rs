//! `object3D/@tracking` (TXT2): extruded text only, in every version.

fn codes(version: &str, object: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="24" duration="1"/><composition>{object}</composition></scene>"#
    );
    match sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()) {
        Ok(_) => Vec::new(),
        Err(sr_model::LoadError::Invalid(r)) => r.diagnostics.iter().map(|d| d.code.clone()).collect(),
        Err(e) => panic!("{e:?}"),
    }
}

#[test]
fn tracking_needs_text_and_no_version() {
    let text = r#"<object3D id="t" primitive="text" text="Hi" tracking="100"/>"#;
    for v in ["1.0", "1.1", "1.2", "1.3"] {
        assert!(!codes(v, text).iter().any(|c| c.starts_with("TXT")), "{v}: {:?}", codes(v, text));
    }
    assert!(codes("1.3", r#"<object3D id="b" primitive="box" width="2" height="2" depth="2" tracking="100"/>"#)
        .contains(&"TXT2".to_string()));
}
