//! MOV1: `object3D/@materialOverride` is a list of name:id pairs.

fn codes(object: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="1.2"><project width="64" height="64" fps="24" duration="1"/><composition>{object}</composition></scene>"#
    );
    match sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()) {
        Ok(_) => Vec::new(),
        Err(sr_model::LoadError::Invalid(r)) => r.diagnostics.iter().map(|d| d.code.clone()).collect(),
        Err(e) => panic!("{e:?}"),
    }
}

#[test]
fn material_override_is_a_list_of_pairs() {
    let object = |v: &str| format!(r#"<object3D id="m" primitive="box" materialOverride="{v}"/>"#);
    assert!(codes(&object("red:blue")).is_empty());
    assert!(codes(&object("  red:blue   green:grey ")).is_empty());
    for bad in ["red", "red:", ":blue", "red:blue green", "  "] {
        assert!(codes(&object(bad)).contains(&"MOV1".to_string()), "{bad:?}");
    }
}
