//! SREP 56: `text`, `strokeFont` and `fontSize` apply to `shape="stroke-text"` only; on another shape kind they have
//! no effect and are reported as inert, at information severity.

fn findings(shape: &str) -> Vec<sr_model::Diagnostic> {
    let xml = format!(
        r#"<scene version="1.2"><project width="64" height="64" fps="10" duration="1"/><assets><strokeFont id="hand" src="hand.jhf"/></assets><composition>{shape}</composition></scene>"#
    );
    let d =
        sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    // a stroke-text shape whose font file is absent still compiles; only the findings of SREP 56 matter here
    match sr_eval::Evaluator::new(&d, &Default::default()) {
        Ok(ev) => ev.warnings().iter().filter(|w| w.message.contains("SREP 56")).cloned().collect(),
        Err(r) => r.diagnostics.into_iter().filter(|w| w.message.contains("SREP 56")).collect(),
    }
}

#[test]
fn text_attributes_on_another_shape_kind_are_inert() {
    let f =
        findings(r#"<shape id="r" shape="rect" width="10" height="10" text="Hi" strokeFont="hand" fontSize="20"/>"#);
    let attrs: Vec<&str> =
        ["@text", "@strokeFont", "@fontSize"].into_iter().filter(|a| f.iter().any(|d| d.message.contains(a))).collect();
    assert_eq!(attrs, ["@text", "@strokeFont", "@fontSize"], "{f:?}");
    assert!(f.iter().all(|d| d.severity == sr_model::Severity::Info && d.code == "E19"), "{f:?}");
}

#[test]
fn the_default_font_size_and_stroke_text_shapes_are_silent() {
    assert!(findings(r#"<shape id="r" shape="ellipse" width="10" height="10" fontSize="48"/>"#).is_empty());
    assert!(findings(
        r#"<shape id="w" shape="stroke-text" width="10" height="10" text="Hi" strokeFont="hand" fontSize="20"/>"#
    )
    .is_empty());
}
