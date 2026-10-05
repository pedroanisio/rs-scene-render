//! C65: stroke markers need an open outline (SREP 15).

fn codes(shape: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="1.1"><project width="64" height="64" fps="24" duration="1"/><composition>{shape}</composition></scene>"#
    );
    match sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()) {
        Ok(_) => Vec::new(),
        Err(sr_model::LoadError::Invalid(r)) => r.diagnostics.iter().map(|d| d.code.clone()).collect(),
        Err(e) => panic!("{e:?}"),
    }
}

#[test]
fn markers_need_a_path_or_a_line() {
    assert!(
        codes(r#"<shape id="r" shape="rect" width="8" height="8" markerEnd="arrow"/>"#).contains(&"C65".to_string())
    );
    assert!(codes(r#"<shape id="e" shape="ellipse" width="8" height="8" markerStart="circle"/>"#)
        .contains(&"C65".to_string()));
    assert!(codes(r#"<shape id="p" shape="path" path="M0 0 L8 8" width="8" height="8" markerEnd="arrow"/>"#).is_empty());
    assert!(
        codes(r#"<shape id="l" shape="line" width="8" height="8" markerStart="bar" markerEnd="arrow"/>"#).is_empty()
    );
    // none draws nothing and is allowed anywhere
    assert!(codes(r#"<shape id="r" shape="rect" width="8" height="8" markerEnd="none"/>"#).is_empty());
}

#[test]
fn a_marker_size_must_be_positive() {
    assert!(!codes(
        r#"<shape id="p" shape="path" path="M0 0 L8 8" width="8" height="8" markerEnd="arrow" markerSize="0"/>"#
    )
    .is_empty());
}
