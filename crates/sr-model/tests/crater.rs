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

fn impact_xml(crater: &str) -> String {
    format!(
        r#"<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/><composition><object3D id="rock" primitive="sphere" radius="1" y="-8"><rigidBody mass="5"/></object3D><object3D id="ground" primitive="plane" width="100" height="100" segments="32" y="2">{crater}<rigidBody type="static"/></object3D></composition><physics pixelsPerMeter="1"/></scene>"#
    )
}

#[test]
fn a_crater_can_grow_from_the_impact_of_a_body() {
    let valid = impact_xml(
        r#"<crater source="rock" targetMaterial="softRock" targetDensity="2100" strength="1000000" gravity="9.8" curve="linear"/>"#,
    );
    assert!(codes(&valid).is_empty(), "{:?}", codes(&valid));
    for material in ["water", "drySand", "drySoil", "wetSoil", "softRock", "hardRock", "regolith", "ice"] {
        let xml = impact_xml(&format!(r#"<crater source="rock" targetMaterial="{material}"/>"#));
        assert!(codes(&xml).is_empty(), "{material}: {:?}", codes(&xml));
    }
    assert!(!codes(&impact_xml(r#"<crater source="rock" targetMaterial="granite"/>"#)).is_empty());
}

#[test]
fn a_crater_from_a_source_derives_what_it_would_otherwise_be_given() {
    for attr in ["radius", "depth", "rimHeight", "rimWidth", "start", "end", "centerX", "normalY"] {
        let xml = impact_xml(&format!(r#"<crater source="rock" targetMaterial="softRock" {attr}="1"/>"#));
        assert!(codes(&xml).contains(&"CRT6".into()), "{attr}: {:?}", codes(&xml));
    }
}

#[test]
fn the_target_belongs_to_a_crater_with_a_source_and_is_required_there() {
    assert!(codes(&impact_xml(r#"<crater source="rock"/>"#)).contains(&"CRT7".into()));
    for attr in [r#"targetMaterial="softRock""#, r#"targetDensity="2000""#, r#"strength="1""#, r#"gravity="9.8""#] {
        assert!(codes(&impact_xml(&format!("<crater {attr}/>"))).contains(&"CRT7".into()), "{attr}");
    }
    assert!(!codes(&impact_xml(r#"<crater source="rock" targetMaterial="softRock" targetDensity="0"/>"#)).is_empty());
}

#[test]
fn the_source_is_another_object_with_a_dynamic_rigid_body() {
    let crater = r#"<crater source="rock" targetMaterial="softRock"/>"#;
    for (xml, why) in [
        (impact_xml(crater).replace(r#"<rigidBody mass="5"/>"#, r#"<rigidBody type="static"/>"#), "static source"),
        (
            impact_xml(crater).replace(r#"<rigidBody mass="5"/>"#, r#"<rigidBody type="kinematic"/>"#),
            "kinematic source",
        ),
        (impact_xml(crater).replace(r#"<rigidBody mass="5"/>"#, ""), "no rigid body"),
        (impact_xml(r#"<crater source="ground" targetMaterial="softRock"/>"#), "the owner itself"),
    ] {
        assert!(codes(&xml).contains(&"CRT8".into()), "{why}: {:?}", codes(&xml));
    }
    assert!(!codes(&impact_xml(r#"<crater source="nobody" targetMaterial="softRock"/>"#)).is_empty());
}

#[test]
fn a_crater_can_be_named_and_its_name_is_an_id_like_any_other() {
    let named = impact_xml(r#"<crater id="pit" source="rock" targetMaterial="softRock"/>"#);
    assert!(codes(&named).is_empty(), "{:?}", codes(&named));
    assert!(codes(&xml(r#"<crater id="pit" radius="4" rimWidth="1"/>"#)).is_empty());
    // the same id twice is the duplicate-id error, whatever the element
    let twice = impact_xml(r#"<crater id="rock" source="rock" targetMaterial="softRock"/>"#);
    assert!(codes(&twice).iter().any(|c| c == "S09"), "{:?}", codes(&twice));
}
