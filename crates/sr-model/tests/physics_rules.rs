//! P01: soft bodies too stiff for their mass are rejected.

fn validate(soft: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="1.1"><project width="64" height="64" fps="30" duration="1"/><composition><shape id="s" kind="rect" width="10" height="10">{soft}</shape></composition></scene>"#
    );
    sr_model::validate_str(&xml, &sr_model::LoadOptions { verify_assets: false, base_dir: None })
        .diagnostics
        .iter()
        .map(|d| d.code.clone())
        .collect()
}

#[test]
fn soft_body_substep_limit() {
    assert!(!validate(r#"<softBody stiffness="20" mass="1"/>"#).contains(&"P01".to_string()));
    assert!(
        validate(r#"<softBody stiffness="1000000" mass="0.000001" rows="16" cols="16"/>"#).contains(&"P01".to_string())
    );
}
