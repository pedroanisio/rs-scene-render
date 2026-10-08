fn codes(node: &str, version: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="64" height="64" fps="24" duration="2"/><assets><mesh id="rock" src="rock.obj"/><image id="spark" src="spark.png" width="1" height="1"/></assets><composition>{node}</composition></scene>"#
    );
    let v = sr_model::validate_str(&xml, &sr_model::LoadOptions::without_assets());
    for d in &v.diagnostics {
        eprintln!("{d:?}");
    }
    v.diagnostics.into_iter().map(|d| d.code).collect()
}

#[test]
fn particles3d_is_versioned_and_distinguishes_emitting_meshes_from_particle_prototypes() {
    let n = r#"<particles3D id="dust" emitterShape="mesh" emitterMesh="rock" shape="mesh" mesh="rock" rate="0"><burst time="0" count="12"/></particles3D>"#;
    assert!(codes(n, "1.3").is_empty(), "{:?}", codes(n, "1.3"));
    assert!(codes(n, "1.2").contains(&"P3D1".into()));
    assert!(codes(&n.replace("emitterMesh=\"rock\"", "emitterMesh=\"spark\""), "1.3").contains(&"P3D2".into()));
    assert!(codes(&n.replace(" mesh=\"rock\"", ""), "1.3").contains(&"P3D2".into()));
}

#[test]
fn particle_ranges_and_static_solver_configuration_are_checked() {
    for n in [
        r#"<particles3D id="p" lifetime="1" lifetimeVariance="1"/>"#,
        r#"<particles3D id="p" speed="1" speedVariance="2"/>"#,
        r#"<particles3D id="p" directionX="0" directionY="0" directionZ="0"/>"#,
        r#"<particles3D id="p" emissionStart="1" emissionEnd="0.5"/>"#,
        r#"<particles3D id="p" emissionStart="1"><burst time="0" count="1"/></particles3D>"#,
    ] {
        assert!(codes(n, "1.3").contains(&"P3D3".into()), "{n}: {:?}", codes(n, "1.3"));
    }
    let n = r#"<particles3D id="p"><animate property="lifetime"><key time="0" value="2"/></animate></particles3D>"#;
    assert!(codes(n, "1.3").contains(&"P3D4".into()));
    assert!(codes(&n.replace("lifetime", "rate"), "1.3").is_empty());
}

#[test]
fn collider_geometry_animation_is_rejected_instead_of_freezing_it() {
    let n = r#"<object3D id="wall" primitive="box"><animate property="width"><key time="0" value="1"/><key time="1" value="2"/></animate></object3D><particles3D id="p" colliders="wall"/>"#;
    assert!(codes(n, "1.3").contains(&"P3D6".into()));
    assert!(codes(&n.replace("property=\"width\"", "property=\"x\""), "1.3").is_empty());
}
