//! SREP 18, rule I8: a node whose window lies wholly outside [0, project duration) is never drawn, and is reported
//! once, at information severity, with its ancestors' clocks applied.

fn findings(composition: &str) -> Vec<sr_model::Diagnostic> {
    let xml = format!(
        r#"<scene version="1.2"><project width="64" height="64" fps="10" duration="10"/><composition>{composition}</composition></scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    ev.warnings().iter().filter(|d| d.code == "INERT-I8").cloned().collect()
}

fn ids(composition: &str) -> Vec<String> {
    findings(composition).into_iter().map(|d| d.path).collect()
}

const RECT: &str = r#"shape="rect" width="4" height="4""#;

#[test]
fn a_node_that_starts_after_the_end_is_never_drawn() {
    let f = findings(&format!(r#"<shape id="late" {RECT} start="12"/>"#));
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].severity, sr_model::Severity::Info);
    assert_eq!(f[0].path, "late");
    assert!(f[0].message.contains("never drawn"), "{}", f[0].message);
    // exactly at the end: the window [10, ∞) misses [0, 10)
    assert_eq!(ids(&format!(r#"<shape id="edge" {RECT} start="10"/>"#)), ["edge"]);
}

#[test]
fn a_node_that_ends_before_zero_is_never_drawn() {
    assert_eq!(ids(&format!(r#"<shape id="early" {RECT} start="-3" end="0"/>"#)), ["early"]);
}

#[test]
fn a_node_that_overlaps_the_composition_is_drawn() {
    assert!(
        ids(&format!(r#"<shape id="a" {RECT} start="9.9"/><shape id="b" {RECT} start="-3" end="0.1"/>"#)).is_empty()
    );
    assert!(ids(&format!(r#"<shape id="c" {RECT}/>"#)).is_empty());
}

#[test]
fn the_ancestors_clocks_place_the_window() {
    // timeOffset 5: the child's 0 is the composition's 5, so a child starting at 6 starts at 11
    let g = |child_start: f64| {
        format!(r#"<group id="g" timeOffset="5"><shape id="kid" {RECT} start="{child_start}"/></group>"#)
    };
    assert_eq!(ids(&g(6.0)), ["kid"]);
    assert!(ids(&g(4.0)).is_empty());
    // a group that is itself outside is reported, its children are not: their own windows are inside
    assert_eq!(ids(&format!(r#"<group id="out" start="20"><shape id="in" {RECT}/></group>"#)), ["out"]);
}

#[test]
fn a_symbol_child_is_reported_once_however_many_instances() {
    let xml = format!(
        r#"<scene version="1.2"><project width="64" height="64" fps="10" duration="10"/><symbols><symbol id="sym" width="8" height="8"><shape id="dot" {RECT} start="50"/></symbol></symbols><composition><instance id="i1" symbol="sym"/><instance id="i2" symbol="sym"/></composition></scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    let f: Vec<_> = ev.warnings().iter().filter(|d| d.code == "INERT-I8").collect();
    assert_eq!(f.len(), 1, "{f:?}");
}
