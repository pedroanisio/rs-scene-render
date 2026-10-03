fn xml(body: &str) -> String {
    format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><composition><object3D id="ground" primitive="plane" width="100" height="100" segments="32">{body}</object3D></composition></scene>"#
    )
}
fn codes(xml: &str) -> Vec<String> {
    sr_model::validate_str(xml, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}
#[test]
fn crater_has_versioned_owned_geometry_and_explicit_timing() {
    let valid = xml(r#"<crater radius="20" depth="5" rimWidth="4" rimHeight="1" start="1" end="2"/>"#);
    assert!(codes(&valid).is_empty(), "{:?}", codes(&valid));
    assert!(codes(&valid.replace("1.3", "1.2")).contains(&"CRT1".into()));
    assert!(codes(&xml("<crater/><crater/>")).contains(&"CRT2".into()));
    assert!(codes(&xml("<crater/>").replace("primitive=\"plane\"", "primitive=\"volume\"")).contains(&"CRT2".into()));
    assert!(codes(&xml(r#"<crater start="2" end="1"/>"#)).contains(&"CRT3".into()));
}
#[test]
fn crater_rejects_invalid_direction_envelope_and_rim() {
    for attrs in [r#"normalZ="0""#, r#"radius="2" rimWidth="3""#, r#"depth="5" influenceDepth="9""#] {
        let got = codes(&xml(&format!("<crater {attrs}/>")));
        assert!(got.contains(&"CRT4".into()), "{attrs}: {got:?}");
    }
    for body in [r#"<crater/><rigidBody type="dynamic"/>"#, r#"<crater/><rigidBody type="static" shape="sphere"/>"#] {
        assert!(codes(&xml(body)).contains(&"CRT5".into()));
    }
    assert!(codes(&xml(r#"<crater/><rigidBody type="static"/>"#)).is_empty());
}
