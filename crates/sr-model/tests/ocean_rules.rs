fn codes(node: &str, version: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="24" duration="2"/><assets><image id="bed" src="bed.png" width="8" height="8"/><mesh id="terrain" src="terrain.obj"/><audio id="sound" src="sound.wav"/></assets><composition>{node}</composition></scene>"#
    );
    sr_model::validate_str(&xml, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn ocean_has_typed_versioned_schema_and_owned_impulses_and_waves() {
    let node = r#"<ocean id="sea" width="16" depth="12" cellSize="0.5" bottomDepth="2"><waterImpulse time="0.3" radius="2" amplitude="0.1"/><wave wavelength="8" amplitude="0.1" speed="2"/></ocean>"#;
    assert!(codes(node, "1.3").is_empty(), "{:?}", codes(node, "1.3"));
    assert!(codes(node, "1.2").contains(&"OCN1".into()));
}

#[test]
fn ocean_order_is_a_one_or_two_enumeration() {
    for order in ["1", "2"] {
        let node = format!(r#"<ocean id="o" order="{order}"/>"#);
        assert!(codes(&node, "1.3").is_empty(), "{:?}", codes(&node, "1.3"));
    }
    for order in ["0", "3", "second", "2.0", ""] {
        let node = format!(r#"<ocean id="o" order="{order}"/>"#);
        assert!(codes(&node, "1.3").contains(&"S06".into()), "{order:?}: {:?}", codes(&node, "1.3"));
    }
}

#[test]
fn ocean_references_dimensions_and_static_inputs_are_checked() {
    for node in [
        r#"<ocean id="o" bathymetry="sound"/>"#,
        r#"<ocean id="o" material="bed"/>"#,
        r#"<ocean id="o" bathymetryEncoding="terrarium"/>"#,
    ] {
        assert!(codes(node, "1.3").contains(&"OCN2".into()), "{:?}", codes(node, "1.3"));
    }
    for node in [
        r#"<ocean id="o" width="0.5" cellSize="1"/>"#,
        r#"<ocean id="o" width="4.2" cellSize="1"/>"#,
        r#"<ocean id="o"><wave wavelength="1"/></ocean>"#,
        r#"<ocean id="o"><waterImpulse type="add-water" amplitude="-1"/></ocean>"#,
    ] {
        assert!(codes(node, "1.3").contains(&"OCN3".into()), "{:?}", codes(node, "1.3"));
    }
    let node = r#"<ocean id="o"><animate property="cellSize"><key time="0" value="1"/></animate></ocean>"#;
    assert!(codes(node, "1.3").contains(&"OCN4".into()));
    assert!(codes(&node.replace("cellSize", "y"), "1.3").is_empty());
}

#[test]
fn whitewater_is_a_single_owned_source_with_typed_materials_and_time_window() {
    assert!(codes(
        r#"<ocean id="o"><whitewater emissionRate="2" threshold="0.2" seed="18446744073709551615"/></ocean>"#,
        "1.3"
    )
    .is_empty());
    for child in [
        r#"<whitewater/><whitewater/>"#,
        r#"<whitewater start="2" end="1"/>"#,
        r#"<whitewater foamMaterial="bed"/>"#,
        r#"<whitewater sprayMaterial="terrain"/>"#,
    ] {
        let c = codes(&format!(r#"<ocean id="o">{child}</ocean>"#), "1.3");
        assert!(c.contains(&"OCN5".into()), "{c:?}");
    }
}

#[test]
fn numeric_bathymetry_does_not_silently_ignore_an_image_layer_selection() {
    let xml = r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="1"/><assets><image id="bed" src="elevation.exr" width="8" height="8" layer="depth"/></assets><composition><ocean id="sea" bathymetry="bed"/></composition></scene>"#;
    let result = sr_model::validate_str(xml, &sr_model::LoadOptions::without_assets());
    assert!(result.diagnostics.iter().any(|d| d.code == "OCN2"));
    let primary = xml.replace(r#" layer="depth""#, "");
    assert!(!sr_model::validate_str(&primary, &sr_model::LoadOptions::without_assets()).has_errors());
}
