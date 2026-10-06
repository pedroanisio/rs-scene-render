//! MOV1: `object3D/@materialOverride` is a list of name:id pairs; MOV2: each id names a material of the document.

fn codes(object: &str) -> Vec<String> {
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="64" fps="24" duration="1"/><materials><material id="blue" baseColor="#0000FF"/><material id="grey" baseColor="#888888"/></materials><composition>{object}</composition></scene>"##
    );
    match sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()) {
        Ok(_) => Vec::new(),
        Err(sr_model::LoadError::Invalid(r)) => r.diagnostics.iter().map(|d| d.code.clone()).collect(),
        Err(e) => panic!("{e:?}"),
    }
}

fn object(v: &str) -> String {
    format!(r#"<object3D id="m" primitive="box" materialOverride="{v}"/>"#)
}

#[test]
fn material_override_is_a_list_of_pairs() {
    assert!(codes(&object("red:blue")).is_empty());
    assert!(codes(&object("  red:blue   green:grey ")).is_empty());
    for bad in ["red", "red:", ":blue", "red:blue green", "  "] {
        assert!(codes(&object(bad)).contains(&"MOV1".to_string()), "{bad:?}");
    }
}

#[test]
fn every_override_target_must_be_a_document_material() {
    assert!(codes(&object("stone:blue old:grey")).is_empty());
    for (bad, why) in [
        ("stone:nosuchmaterial", "unknown id"),
        ("stone:blue old:nope", "second pair unknown"),
        ("stone:Blue", "ids are case sensitive"),
    ] {
        let c = codes(&object(bad));
        assert_eq!(c, vec!["MOV2".to_string()], "{why}: {bad:?} gave {c:?}");
    }
    // a malformed pair has no id to find, so it is MOV1 and MOV2 both
    let c = codes(&object("red"));
    assert!(c.contains(&"MOV1".to_string()) && c.contains(&"MOV2".to_string()), "{c:?}");
    // one diagnostic per object however many pairs are wrong
    assert_eq!(codes(&object("a:x b:y")), vec!["MOV2".to_string()]);
}

#[test]
fn the_check_has_no_limit_on_the_number_of_pairs() {
    let pairs = |n: usize, bad: Option<usize>| {
        (0..n).map(|k| format!("n{k}:{}", if Some(k) == bad { "nope" } else { "blue" })).collect::<Vec<_>>().join(" ")
    };
    for n in [1, 2, 40, 200] {
        assert!(codes(&object(&pairs(n, None))).is_empty(), "{n} good pairs");
        assert_eq!(codes(&object(&pairs(n, Some(n - 1)))), vec!["MOV2".to_string()], "{n} pairs, the last wrong");
    }
}
