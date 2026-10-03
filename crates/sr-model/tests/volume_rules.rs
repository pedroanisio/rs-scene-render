fn codes(version: &str, asset: &str, object: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="32" height="32" fps="30" duration="1"/><assets>{asset}</assets><composition>{object}</composition></scene>"#
    );
    sr_model::validate_str(&xml, &sr_model::LoadOptions { verify_assets: false, base_dir: None })
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}

const ASSET: &str = r#"<volume id="smoke" src="smoke.srvol"/>"#;
const OBJECT: &str =
    r#"<object3D id="v" primitive="volume" volume="smoke"><medium extinction="0.5" stepSize="0.25"/></object3D>"#;

#[test]
fn openvdb_assets_accept_explicit_format_and_existing_sequence_contract() {
    let asset = r#"<volume id="smoke" src="smoke.vdb" format="openvdb"/>"#;
    assert!(codes("1.3", asset, OBJECT).is_empty(), "{:?}", codes("1.3", asset, OBJECT));
    let sequence =
        asset.replace("smoke.vdb", "smoke-%03d.vdb").replace("/>", r#" first="0" last="3" interpolation="linear"/>"#);
    assert!(codes("1.3", &sequence, OBJECT).is_empty(), "{:?}", codes("1.3", &sequence, OBJECT));
    assert!(codes("1.2", asset, OBJECT).contains(&"V8".into()));
    assert!(codes("1.3", &sequence.replace(" last=\"3\"", ""), OBJECT).contains(&"VOL6".into()));
    assert!(!codes("1.3", &asset.replace("openvdb", "guess"), OBJECT).is_empty());
}

#[test]
fn baked_sequence_requires_a_digest_and_owns_its_timing() {
    let asset = format!(
        r#"<volume id="smoke" src="take/manifest.srvseq" format="srvseq" sha256="{}" interpolation="linear" temperatureGrid="temperature"/>"#,
        "a".repeat(64)
    );
    assert!(codes("1.3", &asset, OBJECT).is_empty(), "{:?}", codes("1.3", &asset, OBJECT));
    for bad in [
        asset.replace(&format!(r#"sha256="{}""#, "a".repeat(64)), ""),
        asset.replace("/>", r#" fps="10"/>"#),
        asset.replace("/>", r#" first="0" last="2"/>"#),
        asset.replace("/>", r#" missingFrame="hold"/>"#),
    ] {
        assert!(codes("1.3", &bad, OBJECT).contains(&"VOL8".into()), "{:?}", codes("1.3", &bad, OBJECT));
    }
}

#[test]
fn thermal_schema_requires_a_declared_temperature_channel() {
    let asset = r#"<volume id="smoke" src="smoke.srvol" temperatureGrid="temperature"/>"#;
    let object = r#"<object3D id="v" primitive="volume" volume="smoke"><medium blackbody="true" temperatureScale="2" emissionScale="0.5"/></object3D>"#;
    assert!(codes("1.3", asset, object).is_empty(), "{:?}", codes("1.3", asset, object));
    assert!(codes("1.3", ASSET, object).contains(&"VOL5".into()));
    assert!(codes("1.3", ASSET, &object.replace("blackbody=\"true\"", "blackbody=\"false\"")).is_empty());
    assert!(!codes("1.3", asset, &object.replace("temperatureScale=\"2\"", "temperatureScale=\"0\"")).is_empty());
}

#[test]
fn volume_schema_is_versioned_and_requires_correct_asset_kind() {
    assert!(codes("1.3", ASSET, OBJECT).is_empty(), "{:?}", codes("1.3", ASSET, OBJECT));
    for version in ["1.0", "1.1", "1.2"] {
        assert!(codes(version, ASSET, OBJECT).contains(&"V8".into()));
    }
    assert!(codes("1.3", ASSET, r#"<object3D id="v" primitive="volume"/>"#).contains(&"VOL1".into()));
    assert!(codes("1.3", r#"<mesh id="smoke" src="x.obj"/>"#, OBJECT).contains(&"VOL2".into()));
    assert!(codes("1.3", ASSET, r#"<object3D id="v" primitive="box"><medium/></object3D>"#).contains(&"VOL3".into()));
    assert!(codes("1.3", ASSET, r#"<object3D id="v" primitive="volume" volume="smoke"><medium/><medium/></object3D>"#)
        .contains(&"VOL3".into()));
}

#[test]
fn volume_bounds_are_all_or_none_and_ordered() {
    for extra in [
        r#"boundsMinX="0""#,
        r#"boundsMinX="0" boundsMinY="0" boundsMinZ="0" boundsMaxX="0" boundsMaxY="1" boundsMaxZ="1""#,
    ] {
        let a = format!(r#"<volume id="smoke" src="s.srvol" {extra}/>"#);
        assert!(codes("1.3", &a, OBJECT).contains(&"VOL4".into()));
    }
    for extra in [r#"densityGrid="bad name""#, r#"densityGrid="""#] {
        assert!(!codes("1.3", &format!(r#"<volume id="smoke" src="s.srvol" {extra}/>"#), OBJECT).is_empty());
    }
}

#[test]
fn volume_sequence_contract_requires_bounded_paired_labels_and_a_pattern() {
    let asset = r#"<volume id="smoke" src="frame-%04d.srvol" first="-2" last="12" fps="24000/1001" interpolation="linear" missingFrame="hold"/>"#;
    assert!(codes("1.3", asset, OBJECT).is_empty(), "{:?}", codes("1.3", asset, OBJECT));
    for bad in [
        asset.replace(" last=\"12\"", ""),
        asset.replace("last=\"12\"", "last=\"-3\""),
        asset.replace("last=\"12\"", "last=\"1000000\""),
        asset.replace("frame-%04d.srvol", "single.srvol"),
    ] {
        assert!(codes("1.3", &bad, OBJECT).contains(&"VOL6".into()), "{:?}", codes("1.3", &bad, OBJECT));
    }
    for attr in [r#"fps="24""#, r#"interpolation="linear""#, r#"missingFrame="hold""#] {
        let asset = format!(r#"<volume id="smoke" src="single.srvol" {attr}/>"#);
        assert!(codes("1.3", &asset, OBJECT).contains(&"VOL6".into()));
    }
    assert!(!codes("1.3", &asset.replace("missingFrame=\"hold\"", "missingFrame=\"black\""), OBJECT).is_empty());
    // A whole-sequence hash is undefined; an asset hash belongs to a single file.
    let hash = format!(" sha256=\"{}\"", "0".repeat(64));
    assert!(codes("1.3", &asset.replace("/>", &format!("{hash}/>")), OBJECT).contains(&"VOL7".into()));
}

#[test]
fn volume_frame_labels_accept_xsd_integer_whitespace_and_leading_signs() {
    for (first, last) in [(" -2 ", " +12 "), (" +0 ", " 0000 ")] {
        let asset = format!(r#"<volume id="smoke" src="frame-%02d.srvol" first="{first}" last="{last}"/>"#);
        assert!(codes("1.3", &asset, OBJECT).is_empty(), "{:?}", codes("1.3", &asset, OBJECT));
    }
}

#[test]
fn motion_interpolation_requires_all_velocity_components_and_a_sequence() {
    let valid = r#"<volume id="smoke" src="f-%d.srvol" first="0" last="1" interpolation="advect" velocityGridX="vel.x" velocityGridY="vel.y" velocityGridZ="vel.z"/>"#;
    assert!(codes("1.3", valid, OBJECT).is_empty(), "{:?}", codes("1.3", valid, OBJECT));
    for attr in ["velocityGridX=\"vel.x\"", "velocityGridY=\"vel.y\"", "velocityGridZ=\"vel.z\""] {
        assert!(codes("1.3", &valid.replace(attr, ""), OBJECT).contains(&"VOL9".into()));
    }
    for interp in ["hold", "linear"] {
        assert!(codes("1.3", &valid.replace("advect", interp), OBJECT).contains(&"VOL9".into()));
    }
    let single = valid.replace(" first=\"0\" last=\"1\"", "").replace("f-%d.srvol", "f.srvol");
    assert!(codes("1.3", &single, OBJECT).contains(&"VOL6".into()));
    let baked = valid.replace(" first=\"0\" last=\"1\"", &format!(" format=\"srvseq\" sha256=\"{}\"", "0".repeat(64)));
    assert!(codes("1.3", &baked, OBJECT).is_empty());
    // The mesh type previously shared the same interpolation enum with volumes.
    let mesh = r#"<meshSequence id="mesh" src="m-%d.obj" first="0" last="1" interpolation="advect"/>"#;
    assert!(!codes("1.3", mesh, "").is_empty());
}
