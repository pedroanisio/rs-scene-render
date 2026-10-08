fn codes(version: &str, attributes: &str) -> Vec<String> {
    let xml = format!(
        r##"<scene version="{version}"><project width="64" height="64" fps="10" duration="1"/><assets><tiles id="dem" src="world.pmtiles"/><map id="m" width="64" height="32" background="#FFFFFF"/></assets><composition><object3D id="earth" {attributes}/></composition></scene>"##
    );
    sr_model::validate_str(&xml, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}
#[test]
fn globe_relief_and_its_options_have_versioned_typed_semantics() {
    let attrs = r#"primitive="globe" map="m" terrain="dem" planetRadius="6378137" terrainTileSize="256" terrainZoom="1" terrainMissing="error" terrainMemoryMiB="64""#;
    assert!(codes("1.3", attrs).is_empty(), "{:?}", codes("1.3", attrs));
    assert!(codes("1.2", attrs).contains(&"GEO1".into()));
    assert!(codes("1.3", r#"primitive="sphere" planetRadius="1""#).contains(&"GEO2".into()));
    assert!(codes("1.3", r#"primitive="globe" map="m" terrainTileSize="260""#).contains(&"GEO2".into()));
    for size in [" +260 ", "00260"] {
        let attrs = format!(r#"primitive="globe" map="m" terrain="dem" terrainTileSize="{size}""#);
        assert!(codes("1.3", &attrs).contains(&"GEO2".into()), "{size}: {:?}", codes("1.3", &attrs));
    }
}

#[test]
fn terrain_sampling_is_static_and_animated_collider_relief_is_rejected() {
    let xml = |property: &str, collider: &str| {
        format!(
            r##"<scene version="1.3"><project width="64" height="64" fps="10" duration="1"/><assets><tiles id="dem" src="world.pmtiles"/><map id="m" width="64" height="32" background="#FFFFFF"/></assets><composition><object3D id="earth" primitive="globe" map="m" terrain="dem"><animate property="{property}"><key time="0" value="1"/></animate>{collider}</object3D></composition></scene>"##
        )
    };
    for (property, body) in
        [("terrainZoom", ""), ("planetRadius", ""), ("exaggeration", r#"<rigidBody type="static"/>"#)]
    {
        let report = sr_model::validate_str(&xml(property, body), &sr_model::LoadOptions::without_assets());
        assert!(report.diagnostics.iter().any(|d| d.code == "GEO3"), "{:?}", report.diagnostics);
    }
    assert!(!sr_model::validate_str(&xml("exaggeration", ""), &sr_model::LoadOptions::without_assets()).has_errors());
    for (other, code) in [
        (r#"<particles3D id="dust" colliders="earth"/>"#, "P3D6"),
        (
            r#"<object3D id="cloud" primitive="volume"><pyro width="4" height="4" depth="4" voxelSize="1" colliders="earth"/></object3D>"#,
            "PYRO8",
        ),
    ] {
        let scene = xml("exaggeration", "").replace("</composition>", &format!("{other}</composition>"));
        let report = sr_model::validate_str(&scene, &sr_model::LoadOptions::without_assets());
        assert!(report.diagnostics.iter().any(|d| d.code == code), "{:?}", report.diagnostics);
    }
}

#[test]
fn globe_sampling_rules_preserve_version_1_2_flat_map_animation() {
    let xml = r##"<scene version="1.2"><project width="64" height="64" fps="10" duration="1"/><assets><tiles id="dem" src="world.pmtiles"/><map id="m" width="64" height="32" background="#FFFFFF"/></assets><composition><object3D id="ground" primitive="map" map="m" terrain="dem"><animate property="terrainEncoding"><key time="0" value="terrarium"/><key time="1" value="mapbox"/></animate></object3D></composition></scene>"##;
    let report = sr_model::validate_str(xml, &sr_model::LoadOptions::without_assets());
    assert!(!report.has_errors(), "{:?}", report.diagnostics);
}
