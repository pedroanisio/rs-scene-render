//! PEN1: a stroke-text shape needs @text and @strokeFont; PEN2: @strokeFont names a strokeFont asset; V5: version 1.2.

fn codes(version: &str, assets: &str, shape: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="24" duration="1"/>{assets}<composition>{shape}</composition></scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    match sr_model::load_str(&xml, &opts) {
        Ok(_) => Vec::new(),
        Err(sr_model::LoadError::Invalid(r)) => r.diagnostics.iter().map(|d| d.code.clone()).collect(),
        Err(e) => panic!("{e:?}"),
    }
}

const FONT: &str = r#"<assets><strokeFont id="hand" src="hand.jhf"/><mesh id="m" src="m.glb"/></assets>"#;

fn shape(attrs: &str) -> String {
    format!(r#"<shape id="w" shape="stroke-text" width="50" height="20" {attrs}/>"#)
}

#[test]
fn a_complete_stroke_text_shape_is_valid() {
    assert!(codes("1.2", FONT, &shape(r#"text="Hi" strokeFont="hand" fontSize="20""#)).is_empty());
}

#[test]
fn text_and_font_are_required() {
    assert_eq!(codes("1.2", FONT, &shape(r#"strokeFont="hand""#)), vec!["PEN1"]);
    assert_eq!(codes("1.2", FONT, &shape(r#"text="Hi""#)), vec!["PEN1"]);
}

#[test]
fn the_font_must_be_a_stroke_font_asset() {
    assert_eq!(codes("1.2", FONT, &shape(r#"text="Hi" strokeFont="m""#)), vec!["PEN2"]);
}

#[test]
fn stroke_text_and_the_asset_need_version_1_2() {
    assert_eq!(codes("1.1", FONT, &shape(r#"text="Hi" strokeFont="hand""#)), vec!["V5"]);
    // the asset alone is gated too
    assert_eq!(codes("1.1", FONT, r#"<shape id="r" shape="rect" width="5" height="5"/>"#), vec!["V5"]);
}
