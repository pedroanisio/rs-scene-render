//! Crafted documents: hostile values are diagnosed, without a panic or an unbounded allocation.

use sr_model::assets::sequence_frame;
use sr_model::parse::ParseValue;
use sr_model::values::Sha256;
use sr_model::{validate_str, LoadOptions, Report, Severity};

fn scene(sections: &str, nodes: &str) -> String {
    format!(
        r#"<scene version="1.1"><project width="64" height="64" fps="30" duration="1"/>{sections}<composition>{nodes}</composition></scene>"#
    )
}

fn report(xml: &str) -> Report {
    validate_str(xml, &LoadOptions { verify_assets: false, base_dir: None })
}

fn codes(xml: &str) -> Vec<String> {
    report(xml).diagnostics.iter().map(|d| d.code.clone()).collect()
}

fn has(xml: &str, code: &str) -> bool {
    codes(xml).iter().any(|c| c == code)
}

#[test]
fn a_digest_of_64_non_ascii_bytes_is_an_error() {
    let s = format!("a{}a", "é".repeat(31));
    assert_eq!(s.len(), 64);
    assert!(Sha256::parse_value(&s).is_err());
    assert!(Sha256::parse_value(&"0".repeat(64)).is_ok());
    // reached through asset verification
    let xml =
        scene(&format!(r#"<assets><image id="i" src="missing.png" width="1" height="1" sha256="{s}"/></assets>"#), "");
    let dir = std::env::temp_dir();
    let r = validate_str(&xml, &LoadOptions { verify_assets: true, base_dir: Some(dir) });
    assert!(r.diagnostics.iter().any(|d| d.code == "S06"), "{r}");
}

#[test]
fn frame_placeholders_have_a_bounded_width() {
    assert_eq!(sequence_frame("f_%9999999999d.png", 1), None);
    assert_eq!(sequence_frame("f_%99999999999999999999999999d.png", 1), None);
    assert_eq!(sequence_frame(&format!("f_{}.png", "#".repeat(70_000)), 1), None);
    assert_eq!(sequence_frame("f_%012d.png", 7).unwrap(), "f_000000000007.png");
    assert_eq!(sequence_frame("f_%04d.png", i64::MIN).unwrap(), "f_-9223372036854775808.png");
}

#[test]
fn a_sequence_spanning_every_integer_is_not_expanded() {
    for (src, step) in [("f_%04d.png", "1"), ("f_%9999999999d.png", "1"), ("f_%04d.png", "9223372036854775807")] {
        let xml = scene(
            &format!(
                r#"<assets><imageSequence id="s" src="{src}" first="-9223372036854775808" last="9223372036854775807" step="{step}" fps="12" width="4" height="4"/></assets>"#
            ),
            "",
        );
        let dir = std::env::temp_dir().join("sr-model-no-such-folder");
        let r = validate_str(&xml, &LoadOptions { verify_assets: true, base_dir: Some(dir) });
        assert!(r.diagnostics.iter().any(|d| d.code == "A04"), "{src} {step}: {r}");
    }
}

#[test]
fn numbers_that_overflow_to_infinity_are_reported() {
    let shape = |attrs: &str| scene("", &format!(r#"<shape id="s" shape="rect" width="10" height="10" {attrs}/>"#));
    assert!(!has(&shape(r#"x="50%" dash="4 2" opacity="0.5""#), "W01"));
    for attrs in [
        r#"rotation="1e400""#,
        r#"rotation="-1e400""#,
        r#"x="1e400""#,
        r#"x="-INF""#,
        r#"dash="4 1e400""#,
        r#"dash="NaN 2""#,
        r#"strokeWidth="1e400""#,
    ] {
        let r = report(&shape(attrs));
        let w: Vec<_> = r.diagnostics.iter().filter(|d| d.code == "W01").collect();
        assert_eq!(w.len(), 1, "{attrs}: {r}");
        assert_eq!(w[0].severity, Severity::Warning);
    }
    // digits alone overflow too
    assert!(has(&shape(&format!(r#"x="{}%""#, "9".repeat(400))), "W01"));
}

#[test]
fn deeply_nested_expressions_are_rejected() {
    let expr = |body: &str| {
        scene(
            "",
            &format!(
                r#"<shape id="s" shape="rect" width="10" height="10"><expression property="x">{body}</expression></shape>"#
            ),
        )
    };
    let nested = |n: usize| format!("{}1{}", "(".repeat(n), ")".repeat(n));
    assert!(codes(&expr(&nested(200))).is_empty(), "{}", report(&expr(&nested(200))));
    assert!(has(&expr(&nested(257)), "P04"));
    assert!(has(&expr(&"(".repeat(30_000)), "P04"));
    assert!(has(&expr(&"[".repeat(30_000)), "P04"));
    // brackets in strings do not nest
    assert!(!has(&expr(&format!("param('{}')", "(".repeat(1000))), "P04"));
    let cond = scene(
        "",
        &format!(r#"<shape id="s" shape="rect" width="10" height="10" condition="{}"/>"#, "(".repeat(30_000)),
    );
    assert!(has(&cond, "P04"));
}

#[test]
fn a_symbol_that_contains_itself_is_rejected() {
    let doc = |symbols: &str| scene(&format!("<symbols>{symbols}</symbols>"), r#"<instance id="i" symbol="a"/>"#);
    let ok = doc(
        r#"<symbol id="a" width="8" height="8"><instance id="ab" symbol="b"/><instance id="ab2" symbol="b"/></symbol>
           <symbol id="b" width="8" height="8"><shape id="s" shape="rect" width="1" height="1"/></symbol>"#,
    );
    assert!(codes(&ok).is_empty(), "{}", report(&ok));
    let direct =
        doc(r#"<symbol id="a" width="8" height="8"><group id="g"><instance id="aa" symbol="a"/></group></symbol>"#);
    assert!(has(&direct, "P03"), "{}", report(&direct));
    let indirect = doc(r#"<symbol id="a" width="8" height="8"><instance id="ab" symbol="b"/></symbol>
           <symbol id="b" width="8" height="8"><instance id="bc" symbol="c"/></symbol>
           <symbol id="c" width="8" height="8"><instance id="ca" symbol="a"/></symbol>"#);
    let r = report(&indirect);
    assert_eq!(r.diagnostics.iter().filter(|d| d.code == "P03").count(), 3, "{r}");
    assert!(r.has_errors());
}

#[test]
fn counts_that_allocate_are_bounded() {
    let node = |n: &str| scene("", n);
    let ok = [
        r#"<shape id="s" shape="star" points="64" width="10" height="10"><shapeModifier type="repeater" copies="500"/><shapeModifier type="zig-zag" ridges="200"/></shape>"#,
        r#"<repeat id="r" count="1000"><shape id="s" shape="rect" width="1" height="1"/></repeat>"#,
        r#"<object3D id="o" primitive="box" instances="10000"/>"#,
        r#"<particleEmitter id="p"><burst time="0" count="100000" repeat="100"/></particleEmitter>"#,
    ];
    for n in ok {
        assert!(codes(&node(n)).is_empty(), "{n}: {}", report(&node(n)));
    }
    let bad = [
        r#"<shape id="s" shape="star" points="4000000000" width="10" height="10"/>"#,
        r#"<shape id="s" shape="rect" width="10" height="10"><shapeModifier type="repeater" copies="1e12"/></shape>"#,
        r#"<shape id="s" shape="rect" width="10" height="10"><shapeModifier type="zig-zag" ridges="4000000000"/></shape>"#,
        r#"<repeat id="r" count="4000000000"><shape id="s" shape="rect" width="1" height="1"/></repeat>"#,
        r#"<object3D id="o" primitive="box" instances="4000000000"/>"#,
        r#"<particleEmitter id="p"><burst time="0" count="4000000000"/></particleEmitter>"#,
        r#"<particleEmitter id="p"><burst time="0" count="1" repeat="4000000000"/></particleEmitter>"#,
    ];
    for n in bad {
        let r = report(&node(n));
        assert_eq!(r.diagnostics.iter().filter(|d| d.code == "P02").count(), 1, "{n}: {r}");
        assert!(r.has_errors());
    }
}

#[test]
fn new_codes_are_catalogued() {
    for c in ["P02", "P03", "P04"] {
        assert!(sr_model::codes::lookup(c).is_some(), "{c}");
    }
}
