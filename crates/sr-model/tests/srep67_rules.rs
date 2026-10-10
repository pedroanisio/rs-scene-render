//! SREP 67 in the schema: compute, tonemap, iterate, the serial effect types, and the rules V13, ITR1, ITR2, SRT1 and
//! CMP13. The cases follow sr-core's kit (conformance/srep_cases/srep-0067.json): compute-version-gate and
//! iterate-check-every.

fn codes(version: &str, body: &str, effects: &str) -> Vec<String> {
    let xml = format!(
        r##"<scene version="{version}"><project width="64" height="64" fps="10" duration="1"/><composition>{body}</composition>{effects}</scene>"##
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

const COMPUTE: &str = r##"<compute id="c" src="a.wgsl" width="400" height="200" invocations="1110"><tonemap ramp="#000000FF #FFFFFFFF"/></compute>"##;
const ITERATE: &str =
    r##"<iterate id="it" steps="10"><shape id="s" shape="rect" width="10" height="10" fill="#FFFFFF"/></iterate>"##;

#[test]
fn the_new_syntax_is_valid_at_1_6() {
    assert_eq!(codes("1.6", COMPUTE, ""), Vec::<String>::new());
    assert_eq!(codes("1.6", ITERATE, ""), Vec::<String>::new());
    let all = r##"<compute id="c" src="a.wgsl" sha256="0000000000000000000000000000000000000000000000000000000000000000" width="4" height="4" invocations="4294967295" channels="4" fractionBits="24" seed="7" blend="screen"><param name="a" value="1"/><tonemap scale="linear" gain="2" gamma="2.2" reference="2000" ramp="#000000FF #FF0000FF #FFFFFFFF" alpha="density"/></compute>"##;
    assert_eq!(codes("1.6", all, ""), Vec::<String>::new());
    let fx = r##"<shape id="s" shape="rect" width="10" height="10" fill="#FFFFFF" effects="a b"/>"##;
    let e = r##"<effects><effect id="a" type="error-diffusion" kernel="stucki" palette="#000000FF #FFFFFFFF" serpentine="true"/><effect id="b" type="segmented-sort" direction="vertical" order="descending" sortKey="green" low="0.2" high="0.8"/></effects>"##;
    assert_eq!(codes("1.6", fx, e), Vec::<String>::new());
}

#[test]
fn before_1_6_the_new_syntax_is_v13() {
    // kit srep-0067-compute-version-gate
    assert_eq!(codes("1.5", COMPUTE, ""), ["V13"]);
    assert_eq!(codes("1.3", ITERATE, ""), ["V13"]);
    let fx = r##"<shape id="s" shape="rect" width="10" height="10" fill="#FFFFFF" effects="a"/>"##;
    assert_eq!(codes("1.5", fx, r#"<effects><effect id="a" type="error-diffusion"/></effects>"#), ["V13"]);
    assert_eq!(codes("1.5", fx, r#"<effects><effect id="a" type="segmented-sort"/></effects>"#), ["V13"]);
}

#[test]
fn iterate_rules() {
    // kit srep-0067-iterate-check-every
    let check = r##"<iterate id="it" steps="10" checkEvery="20"><shape id="s" shape="rect" width="10" height="10" fill="#FFFFFF"/></iterate>"##;
    assert_eq!(codes("1.6", check, ""), ["ITR1"]);
    let ok = check.replace(r#"checkEvery="20""#, r#"checkEvery="10""#);
    assert_eq!(codes("1.6", &ok, ""), Vec::<String>::new());
    let nested = r##"<iterate id="a" steps="10"><iterate id="b" steps="5"><shape id="s" shape="rect" width="10" height="10" fill="#FFFFFF"/></iterate></iterate>"##;
    assert_eq!(codes("1.6", nested, ""), ["ITR2"]);
    assert!(!codes("1.6", r#"<iterate id="it" steps="0"/>"#, "").is_empty(), "steps is positive");
    assert!(!codes("1.6", r#"<iterate id="it" steps="10000001"/>"#, "").is_empty(), "the ceiling is 10^7");
}

#[test]
fn sort_and_compute_rules() {
    let fx = r##"<shape id="s" shape="rect" width="10" height="10" fill="#FFFFFF" effects="a"/>"##;
    let e = r#"<effects><effect id="a" type="segmented-sort" low="0.8" high="0.2"/></effects>"#;
    assert_eq!(codes("1.6", fx, e), ["SRT1"]);
    let two = r##"<compute id="c" src="a.wgsl" width="4" height="4" invocations="1"><tonemap/><tonemap/></compute>"##;
    assert_eq!(codes("1.6", two, ""), ["CMP13"]);
    for bad in [
        r#"<compute id="c" src="a.wgsl" width="4" height="4" invocations="1" channels="2"/>"#,
        r#"<compute id="c" src="a.wgsl" width="4" height="4" invocations="1" fractionBits="25"/>"#,
        r#"<compute id="c" src="a.wgsl" width="4" height="4" invocations="4294967296"/>"#,
        r#"<compute id="c" src="a.wgsl" width="4" height="4" invocations="1"><tonemap reference="0"/></compute>"#,
        r#"<compute id="c" src="a.wgsl" width="4" height="4" invocations="1"><tonemap reference="min"/></compute>"#,
    ] {
        assert!(!codes("1.6", bad, "").is_empty(), "{bad} is accepted");
    }
}
