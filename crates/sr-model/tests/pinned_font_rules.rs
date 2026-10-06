//! SREP 21, rules C70 to C73 (pattern p74): with `project/@fontPolicy="pinned"` every face comes from a font asset
//! pinned by `sha256`. Each rule rejects a document built to break it; a pinned document with valid `font` and
//! `fallback` passes; the rules do not apply under the default policy.
//!
//! The rules run here on the parsed document, as the Schematron does, whatever the structure stage says: until the
//! engine's schema copy declares `project/@fontPolicy` (schema 1.2), the structure stage rejects the attribute itself.

fn rule_codes(project: &str, assets: &str, body: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="1.2"><project width="64" height="64" fps="10" duration="1" {project}/><styles><textStyle id="ts" font="Brand" fallback="Brand, Latin"/></styles><assets>{assets}</assets><composition>{body}</composition></scene>"#
    );
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let mut out = Vec::new();
    sr_model::rules::validate(&doc, &mut out);
    let mut c: Vec<String> = out.into_iter().map(|d| d.code).filter(|c| c.starts_with("C7")).collect();
    c.sort();
    c
}

const SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn fonts() -> String {
    format!(
        r#"<font id="b" src="b.ttf" family="Brand" sha256="{SHA}"/><font id="l" src="l.ttf" family="Latin" sha256="{SHA}"/><text id="t" text="Hi" width="10" height="10" size="10" font="Brand" fallback="Latin,Brand"/>"#
    )
}

const PINNED: &str = r#"fontPolicy="pinned""#;

#[test]
fn a_pinned_document_with_valid_fonts_passes() {
    assert!(rule_codes(PINNED, &fonts(), "").is_empty());
}

#[test]
fn c70_no_font_file() {
    let assets = fonts().replace(r#"font="Brand" fallback"#, r#"font="Brand" fontFile="x.ttf" fallback"#);
    assert_eq!(rule_codes(PINNED, &assets, ""), ["C70"]);
}

#[test]
fn c71_every_font_asset_is_pinned_by_sha256() {
    let assets = fonts().replacen(&format!(r#" sha256="{SHA}""#), "", 1);
    assert_eq!(rule_codes(PINNED, &assets, ""), ["C71"]);
}

#[test]
fn c72_every_font_names_a_font_asset_family() {
    let assets = fonts().replace(
        r#"text="Hi" width="10" height="10" size="10" font="Brand""#,
        r#"text="Hi" width="10" height="10" size="10" font="Helvetica""#,
    );
    assert_eq!(rule_codes(PINNED, &assets, ""), ["C72"]);
    // object3D/@font too, which has no fontAsset
    let body = r#"<object3D id="o" primitive="text" text="3D" font="Helvetica"/>"#;
    assert_eq!(rule_codes(PINNED, &fonts(), body), ["C72"]);
}

#[test]
fn c73_every_fallback_family_names_a_font_asset() {
    let assets = fonts().replace(r#"fallback="Latin,Brand""#, r#"fallback="Latin, Arial""#);
    assert_eq!(rule_codes(PINNED, &assets, ""), ["C73"]);
    // tokens are space-normalised, and empty tokens are not tokens
    let spaced = fonts().replace(r#"fallback="Latin,Brand""#, r#"fallback="  Latin  ,,Brand""#);
    assert!(rule_codes(PINNED, &spaced, "").is_empty());
}

#[test]
fn the_default_policy_asks_nothing() {
    let assets = r#"<font id="b" src="b.ttf" family="Brand"/><text id="t" text="Hi" width="10" height="10" size="10" font="Helvetica" fontFile="x.ttf"/>"#;
    assert!(rule_codes("", assets, "").is_empty());
    assert!(rule_codes(r#"fontPolicy="system""#, assets, "").is_empty());
}
