fn xml(body: &str) -> String {
    format!(
        r##"<scene version="1.3"><project width="64" height="64" fps="24" duration="3"/>
    <materials><material id="inside" baseColor="#cc6633"/></materials>
    <composition><object3D id="rock" primitive="box"><rigidBody mass="10"/>{body}</object3D></composition></scene>"##
    )
}
fn codes(s: &str) -> Vec<String> {
    sr_model::validate_str(s, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .map(|d| d.code)
        .collect()
}
#[test]
fn fracture_has_typed_defaults_full_seed_precision_and_version_gate() {
    let s = xml(r#"<fracture interiorMaterial="inside" seed="18446744073709551615"/>"#);
    let doc = sr_model::load_str(&s, &sr_model::LoadOptions::without_assets()).unwrap();
    use sr_model::element::children;
    let owner = children(&doc.scene.composition)[0];
    let f = children(owner).into_iter().find(|e| e.element_name() == "fracture").unwrap();
    // The typed model preserves all bits; the generic animation attribute
    // interface intentionally converts numbers to f64 and is not a seed API.
    let typed = f.as_any().downcast_ref::<sr_model::model::Fracture>().unwrap();
    assert_eq!(typed.seed, u64::MAX);
    assert_eq!(f.get_attr("pieces").unwrap().to_string(), "8");
    assert!(codes(&s.replace("1.3", "1.2")).contains(&"FRX1".into()));
}
#[test]
fn fracture_requires_one_closed_surface_owner_body_and_interior_material() {
    let f = r#"<fracture interiorMaterial="inside"/>"#;
    assert!(codes(&xml(f)).is_empty());
    for s in [
        xml(&format!("{f}{f}")),
        xml(f).replace(r#"<rigidBody mass="10"/>"#, ""),
        xml(f).replace(r#"primitive="box""#, r#"primitive="plane""#),
        xml(f).replace(r#"primitive="box""#, r#"primitive="volume""#),
    ] {
        assert!(codes(&s).contains(&"FRX2".into()), "{:?}", codes(&s));
    }
    assert!(codes(&xml(r#"<fracture interiorMaterial="rock"/>"#)).contains(&"FRX3".into()));
    assert!(!codes(&xml("<fracture/>")).is_empty());
}
#[test]
fn fracture_rejects_nonfinite_impulses_and_unbounded_counts() {
    for attr in [
        r#"pieces="0""#,
        r#"pieces="4097""#,
        r#"seed="18446744073709551616""#,
        r#"at="-1""#,
        r#"interiorUvScale="0""#,
        r#"maxMemoryMiB="4097""#,
    ] {
        assert!(!codes(&xml(&format!(r#"<fracture interiorMaterial="inside" {attr}/>"#))).is_empty(), "{attr}");
    }
    for attr in [r#"impulseX="NaN""#, r#"impulseY="INF""#, r#"impulseZ="-INF""#] {
        assert!(codes(&xml(&format!(r#"<fracture interiorMaterial="inside" {attr}/>"#))).contains(&"FRX4".into()));
    }
}

#[test]
fn numbered_mesh_fracture_requires_a_deformable_kinematic_or_static_source() {
    let s = xml(r#"<fracture at="1" interiorMaterial="inside"/>"#)
        .replace(
            "<materials>",
            r#"<assets><meshSequence id="frames" src="frame-%d.obj" first="0" last="1" fps="1"/></assets><materials>"#,
        )
        .replace(r#"primitive="box""#, r#"primitive="mesh" mesh="frames""#);
    assert!(codes(&s).contains(&"MSQ4".into()));
    for kind in ["kinematic", "static"] {
        let s = s.replace("<rigidBody ", &format!("<rigidBody type=\"{kind}\" "));
        assert!(codes(&s).is_empty(), "{:?}", codes(&s));
    }
}
