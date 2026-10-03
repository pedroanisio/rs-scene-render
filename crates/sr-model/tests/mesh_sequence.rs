fn xml(asset: &str, body: &str) -> String {
    format!(
        r#"<scene version="1.3"><project width="32" height="32" fps="10" duration="2"/><assets>{asset}</assets><composition><object3D id="cache" primitive="mesh" mesh="frames">{body}</object3D></composition></scene>"#
    )
}
const ASSET: &str = r#"<meshSequence id="frames" src="cache-%04d.obj" first="-1" last="2" fps="24000/1001" interpolation="linear" missingFrame="error" maxMemoryMiB="64"/>"#;
fn codes(scene: &str) -> Vec<String> {
    sr_model::validate_str(scene, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}
#[test]
fn mesh_sequences_have_versioned_typed_timing_and_bounded_ranges() {
    assert!(codes(&xml(ASSET, "")).is_empty(), "{:?}", codes(&xml(ASSET, "")));
    assert!(codes(&xml(ASSET, "").replace("version=\"1.3\"", "version=\"1.2\"")).contains(&"MSQ1".into()));
    for bad in [
        ASSET.replace("last=\"2\"", "last=\"-2\""),
        ASSET.replace("cache-%04d.obj", "cache.obj"),
        ASSET.replace("last=\"2\"", "last=\"1000000\""),
    ] {
        assert!(codes(&xml(&bad, "")).contains(&"MSQ2".into()));
    }
    let hash = ASSET.replace("/>", &format!(" sha256=\"{}\"/>", "0".repeat(64)));
    assert!(codes(&xml(&hash, "")).contains(&"MSQ3".into()));
}
#[test]
fn mesh_sequences_do_not_silently_become_static_rigid_colliders() {
    let unused = xml(ASSET, r#"<rigidBody type="static"/>"#).replace("primitive=\"mesh\"", "primitive=\"sphere\"");
    assert!(codes(&unused).is_empty(), "{:?}", codes(&unused));
    assert!(codes(&xml(ASSET, r#"<rigidBody type="static"/>"#)).contains(&"MSQ4".into()));
    for consumer in [
        r#"<particles3D id="p" colliders="cache"/>"#,
        r#"<object3D id="smoke" primitive="volume"><pyro width="4" height="4" depth="4" voxelSize="1" colliders="cache"/></object3D>"#,
    ] {
        let scene = xml(ASSET, "").replace("</composition>", &format!("{consumer}</composition>"));
        assert!(codes(&scene).contains(&"MSQ4".into()), "{:?}", codes(&scene));
    }
}
