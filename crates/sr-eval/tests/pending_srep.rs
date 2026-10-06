//! Accepted SREPs the engine does not implement yet are reported, not rendered silently wrong (E22).
//!
//! The fixtures are the sr-core conformance cases of each pending SREP (schema 1.3.0 accepts them). A SREP that is
//! implemented removes its row from `sr_eval::pending::PENDING` and its fixture from this list.

use sr_eval::{pending::PENDING, EvalOptions, Evaluator};

fn evaluate(xml: &str) -> Vec<sr_model::diag::Diagnostic> {
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    let ev = Evaluator::new(&doc, &EvalOptions::default()).unwrap_or_else(|r| panic!("{r}"));
    ev.warnings().to_vec()
}

fn fixture(srep: u32) -> String {
    let path = format!("{}/tests/fixtures/pending/srep-{srep}.xml", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn every_pending_srep_is_reported_as_a_warning_that_strict_counts() {
    for p in PENDING {
        let warnings = evaluate(&fixture(p.srep));
        let hit = warnings.iter().find(|d| d.code == "E22" && d.message.contains(&format!("SREP {}", p.srep)));
        let hit = hit.unwrap_or_else(|| panic!("SREP {}: no E22 in {warnings:?}", p.srep));
        // --strict counts every warning that is not information
        assert!(!hit.is_info(), "SREP {}: E22 must not be information", p.srep);
    }
}

#[test]
fn a_document_that_uses_none_of_them_has_no_e22() {
    let xml = r##"<scene version="1.2"><project width="64" height="64" fps="10" duration="1"/><composition>
        <shape id="s" shape="rect" width="10" height="10"/></composition></scene>"##;
    assert!(evaluate(xml).iter().all(|d| d.code != "E22"));
}

#[test]
fn an_attribute_left_at_its_default_is_not_a_use() {
    let xml = r##"<scene version="1.2"><project width="64" height="64" fps="10" duration="1" fontPolicy="system"/>
        <composition><shape id="s" shape="rect" width="10" height="10"/></composition></scene>"##;
    assert!(evaluate(xml).iter().all(|d| d.code != "E22"));
}

#[test]
fn an_ordinary_shape_has_no_e22_whatever_defaults_the_schema_gives_its_pending_attributes() {
    // regionPadding (SREP 17) has the default 0: reading the default is not a use
    let xml = r##"<scene version="1.2"><project width="64" height="64" fps="10" duration="1"/><composition>
        <shape id="s" shape="rect" width="10" height="10"/>
        <shape id="p" shape="rect" width="10" height="10" regionPadding="0"/></composition></scene>"##;
    assert!(evaluate(xml).iter().all(|d| d.code != "E22"));
}

#[test]
fn a_pending_attribute_on_a_typed_field_element_is_found_by_its_element_name() {
    // output, project, accessibility and captionTrack are reached as fields, not through a child enum
    for (what, xml) in [
        (
            "output/@report",
            r##"<scene version="1.2"><project width="64" height="64" fps="10" duration="1"/>
            <output id="o" path="out/o.mp4" codec="h264" report="out/r.json"/><composition/></scene>"##,
        ),
        (
            "project/@fontPolicy",
            r##"<scene version="1.2"><project width="64" height="64" fps="10" duration="1" fontPolicy="pinned"/>
            <composition/></scene>"##,
        ),
    ] {
        let w = evaluate(xml);
        assert!(w.iter().any(|d| d.code == "E22"), "{what}: {w:?}");
    }
}
