//! `explain` texts say what the engine does.

#[test]
fn sa01_names_the_audit_bound_the_engine_uses() {
    let text = sr_model::codes::lookup("SA01").expect("SA01 is explained").summary;
    // the audit looks at every frame of a short range and at most 240 of a long one (sr-eval safe_area::MAX_SAMPLES)
    assert!(text.contains("240"), "{text}");
    assert!(!text.contains("4800"), "{text}");
}
