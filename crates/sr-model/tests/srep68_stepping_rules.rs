//! SREP 68: `effect/@stepsPerFrame` and `@prewarm` in the schema, and STP1 (they apply to shader effects only).
//! The cases follow sr-core's kit `conformance/srep_cases/srep-0068.json` (srep-0068-not-a-stateful-effect).

fn codes(effect: &str) -> Vec<String> {
    let xml = format!(
        r##"<scene version="1.5"><project width="64" height="64" fps="10" duration="1"/><composition><shape id="s" shape="rect" width="10" height="10" fill="#FFFFFF" effects="fx"/></composition><effects>{effect}</effects></scene>"##
    );
    match sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()) {
        Ok(_) => Vec::new(),
        Err(sr_model::LoadError::Invalid(r)) => {
            let mut c: Vec<String> = r.diagnostics.iter().filter(|d| d.is_error()).map(|d| d.code.clone()).collect();
            c.sort();
            c.dedup();
            c
        }
        Err(e) => panic!("{e:?}"),
    }
}

#[test]
fn a_shader_effect_takes_steps_and_prewarm() {
    for e in [
        r#"<effect id="fx" type="shader" src="x.fs" stepsPerFrame="10"/>"#,
        r#"<effect id="fx" type="shader" src="x.fs" prewarm="9"/>"#,
        r#"<effect id="fx" type="shader" src="x.fs" stepsPerFrame="3" prewarm="2"/>"#,
        r#"<effect id="fx" type="shader" src="x.fs" stepsPerFrame="10000" prewarm="10000000"/>"#,
    ] {
        assert_eq!(codes(e), Vec::<String>::new(), "{e}");
    }
}

#[test]
fn another_effect_type_is_stp1() {
    assert_eq!(codes(r#"<effect id="fx" type="blur" stepsPerFrame="2"/>"#), ["STP1"]);
    assert_eq!(codes(r#"<effect id="fx" type="blur" prewarm="2"/>"#), ["STP1"]);
}

#[test]
fn out_of_range_values_fail_the_xsd() {
    for e in [
        r#"<effect id="fx" type="shader" src="x.fs" stepsPerFrame="0"/>"#,
        r#"<effect id="fx" type="shader" src="x.fs" stepsPerFrame="10001"/>"#,
        r#"<effect id="fx" type="shader" src="x.fs" prewarm="-1"/>"#,
        r#"<effect id="fx" type="shader" src="x.fs" prewarm="10000001"/>"#,
    ] {
        assert!(!codes(e).is_empty(), "{e} is accepted");
    }
}
