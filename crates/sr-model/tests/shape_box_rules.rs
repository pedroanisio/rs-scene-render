fn dimension_errors(attributes: &str) -> Vec<sr_model::Diagnostic> {
    let xml = format!(
        r#"<scene version="1.2">
<project width="64" height="64" fps="24" duration="1"/>
<composition><shape id="box" shape="rect" {attributes}/></composition>
</scene>"#
    );
    sr_model::validate_str(&xml, &sr_model::LoadOptions::without_assets())
        .diagnostics
        .into_iter()
        .filter(|d| d.code == "C69")
        .collect()
}

#[test]
fn shape_needs_both_dimensions_without_a_region() {
    for attributes in ["", r#"width="8""#, r#"height="8""#] {
        let errors = dimension_errors(attributes);
        assert_eq!(errors.len(), 1, "{attributes}: {errors:?}");
        assert_eq!(errors[0].loc.line, 3);
    }
    assert!(dimension_errors(r#"width="8" height="8""#).is_empty());
}

#[test]
fn region_shape_uses_the_earlier_schematron_rule() {
    for attributes in [r#"region="page-box""#, r#"region="""#] {
        assert!(dimension_errors(attributes).is_empty());
    }
}

#[test]
fn dimension_rule_checks_presence_even_when_values_are_invalid() {
    assert!(dimension_errors(r#"width="" height="""#).is_empty());
}
