//! SREP 34 (amends SREP 18): inert rules I9 to I13 at information severity, and `MASK-MISS`, a warning with the
//! mask's offset measured. The SREP's conformance cases, case by case.

use sr_model::Severity;

fn doc(composition: &str, effects: &str, assets: &str) -> sr_model::Document {
    let assets = if assets.is_empty() { String::new() } else { format!("<assets>{assets}</assets>") };
    let xml = format!(
        r#"<scene version="1.2"><project width="640" height="640" fps="10" duration="1"/>{assets}<composition>{composition}</composition>{effects}</scene>"#
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn findings(composition: &str, effects: &str, assets: &str) -> Vec<sr_model::Diagnostic> {
    let d = doc(composition, effects, assets);
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    // validation's own inert findings (I1 to I7) and the evaluator's
    d.warnings().iter().chain(ev.warnings()).cloned().collect()
}

fn codes(composition: &str, effects: &str) -> Vec<String> {
    findings(composition, effects, "").into_iter().map(|d| d.code).collect()
}

const RECT: &str = r#"shape="rect" width="8" height="8""#;

#[test]
fn srep_0034_inert_collapse() {
    let f = findings(&format!(r#"<group id="g" collapse="true"><shape id="s" {RECT}/></group>"#), "", "");
    assert_eq!(f.iter().map(|d| d.code.as_str()).collect::<Vec<_>>(), ["INERT-I9"], "exactly I9: {f:?}");
    assert_eq!(f[0].severity, Severity::Info);
}

#[test]
fn srep_0034_inert_channel() {
    let node = format!(r#"<shape id="s" {RECT} effects="sc"/>"#);
    let fx = |c: &str| format!(r#"<effects><effect id="sc" type="selective-color" hue="30" channel="{c}"/></effects>"#);
    assert_eq!(codes(&node, &fx("red")), ["INERT-I10"], "once, and not also as I13");
    assert!(codes(&node, &fx("rgb")).is_empty());
}

#[test]
fn srep_0034_inert_key_param() {
    let k = |interp: &str| {
        format!(
            r#"<shape id="s" {RECT}><animate property="x"><key time="0" value="0" interpolation="{interp}" overshoot="2"/><key time="1" value="10"/></animate></shape>"#
        )
    };
    assert_eq!(codes(&k("ease-out"), ""), ["INERT-I12"]);
    assert!(codes(&k("back-out"), "").is_empty());
}

#[test]
fn srep_0034_inert_effect_param() {
    let node = format!(r#"<shape id="s" {RECT} effects="v"/>"#);
    let fx = |a: &str| format!(r#"<effects><effect id="v" type="vignette" {a}/></effects>"#);
    assert_eq!(codes(&node, &fx(r#"intensity="0.5""#)), ["INERT-I13"]);
    assert!(codes(&node, &fx(r#"amount="0.5" radius="200" softness="0.4""#)).is_empty());
}

#[test]
fn srep_0034_inert_source_hidden() {
    let img = r#"<image id="wide" src="w.png" width="8" height="4"/>"#;
    let fx = r#"<effects><effect id="dm" type="displacement-map" source="map" amount="4"/></effects>"#;
    let nodes = |a: &str| format!(r#"<layer id="map" asset="wide" {a}/><shape id="s" {RECT} effects="dm"/>"#);
    let c = |a: &str| findings(&nodes(a), fx, img).into_iter().map(|d| d.code).collect::<Vec<_>>();
    assert_eq!(c(r#"opacity="0""#), ["INERT-I11"]);
    assert!(c(r#"visible="false" opacity="1""#).is_empty());
    // an opacity that is animated up is not 0 for the whole window
    let animated = r#"opacity="0"><animate property="opacity"><key time="0" value="0"/><key time="1" value="1"/></animate></layer><shape id="x" shape="rect" width="1" height="1""#;
    assert!(c(animated).is_empty(), "{:?}", c(animated));
}

#[test]
fn srep_0034_mask_miss() {
    let shape = |mask: &str| {
        format!(
            r##"<shape id="m" shape="rect" x="500" y="300" width="300" height="400" fill="#FF0000">{mask}</shape>"##
        )
    };
    // canvas coordinates: 200 px to the right of the node's 300 px width
    let f = findings(&shape(r#"<mask type="rect" x="500" y="300" width="300" height="400" mode="add"/>"#), "", "");
    let miss: Vec<_> = f.iter().filter(|d| d.code == "MASK-MISS").collect();
    assert_eq!(miss.len(), 1, "{f:?}");
    assert_eq!(miss[0].severity, Severity::Warning);
    let m = miss[0].measured.as_ref().expect("the offset is measured");
    assert_eq!((m.value, m.unit), (200.0, "px"));
    // the default mode (intersect) misses too
    assert_eq!(codes(&shape(r#"<mask type="ellipse" x="-50" y="0" width="40" height="40"/>"#), ""), ["MASK-MISS"]);
    // local coordinates, an inverted mask, a subtracting one, and modes that do not add or intersect: none
    for m in [
        r#"<mask type="rect" x="0" y="0" width="300" height="400" mode="add"/>"#,
        r#"<mask type="rect" x="500" y="300" width="30" height="40" invert="true"/>"#,
        r#"<mask type="rect" x="500" y="300" width="30" height="40" mode="subtract"/>"#,
        r#"<mask type="rect" x="500" y="300" width="30" height="40" mode="lighten"/>"#,
        r#"<mask type="rect" x="500" y="300" width="30" height="40" mode="none"/>"#,
    ] {
        assert!(codes(&shape(m), "").is_empty(), "{m}");
    }
}
