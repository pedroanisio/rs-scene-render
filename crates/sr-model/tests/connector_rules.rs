//! SREP 16: the `connector` node in the schema and its rules (V11, C60–C64, R48-from, R48-to, R49, R50).

fn codes_in(version: &str, body: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="{version}"><project width="640" height="360" fps="24" duration="2"/><assets><text id="t" text="Hi" width="40" height="20" size="12"/><image id="img" src="x.png" width="4" height="4"/></assets><symbols><symbol id="sym" width="100" height="100"><shape id="sa" shape="rect" width="10" height="10"/><shape id="sb" shape="rect" width="10" height="10"/><connector id="sc" from="sa" to="sb"/></symbol></symbols><composition><shape id="R" shape="rect" width="80" height="60"/><shape id="B" shape="rect" x="400" width="80" height="60"/>{body}</composition></scene>"#
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

fn codes(body: &str) -> Vec<String> {
    codes_in("1.2", body)
}

#[test]
fn valid_connectors() {
    for body in [
        r#"<connector id="c" from="R" to="B"/>"#,
        r#"<connector id="c" from="R" toX="10" toY="20%"/>"#,
        r#"<connector id="c" fromX="0" fromY="0" toX="1vw" toY="1vh"/>"#,
        r#"<connector id="c" from="R" to="B" fromAnchor="bottom" toAnchor="top-left" route="orthogonal" points="1,2 3,4"/>"#,
        r#"<connector id="c" from="R" to="B" fromX="50%" fromY="0" route="curved" bend="-45" label="t" labelOrient="along"/>"#,
        r#"<connector id="c" from="R" to="B" fromAnchor="auto" fromX="1" fromY="2"/>"#,
        r#"<connector id="c" from="R" to="B" markerEnd="arrow" trimEnd="0.5" dash="4 2"><animate property="bend"><key time="0" value="0"/><key time="1" value="30"/></animate><expression property="opacity">0.5</expression></connector>"#,
        r#"<group id="g"><shape id="in" shape="rect" width="4" height="4"/></group><connector id="c" from="g" to="in"/>"#,
        r#"<layer id="L" asset="img"/><instance id="I" symbol="sym"/><connector id="c" from="L" to="I"/>"#,
    ] {
        assert_eq!(codes(body), Vec::<String>::new(), "{body}");
    }
}

#[test]
fn v11_connectors_need_version_1_2() {
    assert_eq!(codes_in("1.1", r#"<connector id="c" from="R" to="B"/>"#), ["V11"]);
    // the template's symbol holds a connector too, so even an empty composition needs 1.2
    assert_eq!(codes_in("1.1", ""), ["V11"]);
    // and without any connector, a 1.1 document has no V11
    let plain = r#"<scene version="1.1"><project width="64" height="64" fps="24" duration="1"/><composition><shape id="s" shape="rect" width="4" height="4"/></composition></scene>"#;
    assert!(sr_model::load_str(plain, &sr_model::LoadOptions::without_assets()).is_ok());
    assert!(codes_in("1.3", r#"<connector id="c" from="R" to="B"/>"#).is_empty());
}

#[test]
fn ends_and_anchors() {
    for (body, rule) in [
        (r#"<connector id="c" from="R"/>"#, "C60"),
        (r#"<connector id="c" to="B"/>"#, "C60"),
        (r#"<connector id="c" from="R" toX="1"/>"#, "C60"),
        (r#"<connector id="c" from="R" to="B" fromAnchor="top" fromX="1" fromY="1"/>"#, "C61"),
        (r#"<connector id="c" from="R" to="B" toAnchor="left" toY="1" toX="1"/>"#, "C61"),
        (r#"<connector id="c" from="R" to="B" fromX="1"/>"#, "C62"),
        (r#"<connector id="c" from="R" to="B" toY="1"/>"#, "C62"),
        (r#"<connector id="c" from="R" to="B" route="curved" points="1,1"/>"#, "C63"),
        (r#"<connector id="c" from="R" to="B"><animate property="x"><key time="0" value="1"/></animate></connector>"#, "C64"),
        (r#"<connector id="c" from="R" to="B"><expression property="strokeWidth">2</expression></connector>"#, "C64"),
    ] {
        assert!(codes(body).contains(&rule.to_string()), "{body}: {:?}", codes(body));
    }
}

#[test]
fn r48_targets() {
    // not a node kind that has a box
    assert_eq!(
        codes(r#"<camera id="cam"/><connector id="c" from="cam" to="B"/>"#),
        ["R48-from"],
        "{:?}",
        codes(r#"<camera id="cam"/><connector id="c" from="cam" to="B"/>"#)
    );
    // inside a repeat
    assert_eq!(
        codes(r#"<repeat id="rp" count="2"><shape id="q" shape="rect" width="4" height="4"/></repeat><connector id="c" from="R" to="q"/>"#),
        ["R48-to"]
    );
    // 2.5D, itself or an ancestor
    assert_eq!(codes(r#"<shape id="d" shape="rect" width="4" height="4" threeD="true"/><connector id="c" from="d" to="B"/>"#), ["R48-from"]);
    assert_eq!(
        codes(r#"<group id="g" threeD="true"><shape id="d" shape="rect" width="4" height="4"/></group><connector id="c" from="R" to="d"/>"#),
        ["R48-to"]
    );
    // a composition connector cannot reach into a symbol, nor a symbol connector out of it
    assert_eq!(codes(r#"<connector id="c" from="sa" to="B"/>"#), ["R48-from"]);
}

#[test]
fn r48_inside_a_symbol() {
    let xml = |conn: &str| {
        format!(
            r#"<scene version="1.2"><project width="64" height="64" fps="24" duration="1"/><symbols><symbol id="s" width="10" height="10"><shape id="a" shape="rect" width="1" height="1"/>{conn}</symbol></symbols><composition><shape id="out" shape="rect" width="1" height="1"/></composition></scene>"#
        )
    };
    let codes = |conn: &str| match sr_model::load_str(&xml(conn), &sr_model::LoadOptions::without_assets()) {
        Ok(_) => Vec::new(),
        Err(sr_model::LoadError::Invalid(r)) => r.diagnostics.iter().map(|d| d.code.clone()).collect::<Vec<_>>(),
        Err(e) => panic!("{e:?}"),
    };
    assert!(codes(r#"<connector id="c" from="a" toX="0" toY="0"/>"#).is_empty());
    assert_eq!(codes(r#"<connector id="c" from="out" toX="0" toY="0"/>"#), ["R48-from"]);
}

#[test]
fn r49_nothing_is_positioned_by_a_connector() {
    let c = r#"<connector id="c" from="R" to="B"/>"#;
    assert!(codes(&format!(r#"{c}<shape id="p" shape="rect" width="4" height="4" parent="c"/>"#)).contains(&"R49".to_string()));
    assert!(codes(&format!(
        r#"{c}<shape id="p" shape="rect" width="4" height="4"><transformConstraint type="copy-position" target="c"/></shape>"#
    ))
    .contains(&"R49".to_string()));
    assert!(codes(&format!(r#"{c}<shape id="p" shape="rect" width="4" height="4"><link property="x" source="c.opacity"/></shape>"#))
        .contains(&"R49".to_string()));
    // a link from another node whose id merely starts with the connector's is fine
    assert!(!codes(&format!(
        r#"{c}<shape id="cc" shape="rect" width="4" height="4"/><shape id="p" shape="rect" width="4" height="4"><link property="x" source="cc.x"/></shape>"#
    ))
    .contains(&"R49".to_string()));
}

#[test]
fn r50_label_is_a_text_asset() {
    assert_eq!(codes(r#"<connector id="c" from="R" to="B" label="img"/>"#), ["R50"]);
    assert!(codes(r#"<connector id="c" from="R" to="B" label="t"/>"#).is_empty());
}

#[test]
fn structure() {
    for body in [
        r#"<connector id="c" from="R" to="B" x="3"/>"#,
        r#"<connector id="c" from="R" to="B" bend="91"/>"#,
        r#"<connector id="c" from="R" to="B" route="zigzag"/>"#,
        r#"<connector id="c" from="R" to="B" fromAnchor="middle"/>"#,
        r#"<connector id="c" from="R" to="B" parent="R"/>"#,
        r#"<connector id="c" from="R" to="B"><shape id="kid" shape="rect" width="1" height="1"/></connector>"#,
    ] {
        let c = codes(body);
        assert!(!c.is_empty() && c.iter().all(|c| c.starts_with('S')), "{body}: {c:?}");
    }
}
