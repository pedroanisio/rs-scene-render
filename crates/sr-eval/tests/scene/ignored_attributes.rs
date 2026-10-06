//! An attribute the schema accepts and this build does not read is reported (E19, a warning), not silently ignored.

fn warnings(composition: &str, effects: &str) -> Vec<(String, String)> {
    let xml = format!(
        r#"<scene version="1.1"><project width="64" height="64" fps="10" duration="1"/><composition>{composition}</composition>{effects}</scene>"#
    );
    let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
    let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
    ev.warnings().iter().map(|d| (d.code.clone(), d.message.clone())).collect()
}

#[test]
fn group_collapse_is_accepted_but_has_no_effect_and_says_so() {
    let w = warnings(r#"<group id="g" collapse="true" width="10" height="10"/>"#, "");
    assert!(w.iter().any(|(c, m)| c == "E19" && m.contains("collapse") && m.contains("no effect")), "{w:?}");
    // the default, and a group that does not set it, are silent
    assert!(warnings(r#"<group id="g" collapse="false" width="10" height="10"/>"#, "").is_empty());
    assert!(warnings(r#"<group id="g" width="10" height="10"/>"#, "").is_empty());
}

#[test]
fn selective_color_does_not_read_channel() {
    let fx = |attrs: &str| format!(r#"<effects><effect id="sc" type="selective-color" {attrs}/></effects>"#);
    let node = r#"<shape id="s" shape="rect" width="8" height="8" effects="sc"/>"#;
    let w = warnings(node, &fx(r#"hue="30" channel="red""#));
    assert!(w.iter().any(|(c, m)| c == "E19" && m.contains("channel")), "{w:?}");
    // once (SREP 34 rule I10), not once for the effect type and once for the effect's own reading
    assert_eq!(w.iter().filter(|(c, _)| c == "E19").count(), 1, "{w:?}");
    assert!(warnings(node, &fx(r#"hue="30""#)).is_empty());
}

#[test]
fn a_mask_that_misses_its_node_is_reported() {
    // masks are node-local: one given in canvas coordinates lies outside the node and the node vanishes silently
    let shape = |mask: &str| {
        format!(
            r##"<shape id="m" shape="rect" x="500" y="300" width="300" height="400" fill="#FF0000">{mask}</shape>"##
        )
    };
    let w = warnings(&shape(r#"<mask type="rect" x="500" y="300" width="300" height="400" mode="add"/>"#), "");
    assert!(w.iter().any(|(c, m)| c == "E20" && m.contains("outside")), "{w:?}");
    // local coordinates, an inverted mask, and a subtracting one that is outside are all fine
    assert!(warnings(&shape(r#"<mask type="rect" x="0" y="0" width="300" height="400" mode="add"/>"#), "").is_empty());
    assert!(
        warnings(&shape(r#"<mask type="rect" x="500" y="300" width="30" height="40" invert="true"/>"#), "").is_empty()
    );
    assert!(warnings(&shape(r#"<mask type="rect" x="500" y="300" width="30" height="40" mode="subtract"/>"#), "")
        .is_empty());
}

#[test]
fn an_effect_source_with_opacity_zero_is_reported() {
    // a node named as a displacement map or plate is drawn with its own opacity: at 0 it contributes nothing, and the
    // effect does nothing, silently. A hidden node (visible="false") at opacity 1 is the way to keep a map off screen.
    let nodes = |attrs: &str| {
        format!(
            r##"<layer id="map" asset="wide" {attrs}/><shape id="s" shape="rect" width="8" height="8" effects="dm"/>"##
        )
    };
    let fx = r#"<effects><effect id="dm" type="displacement-map" source="map" amount="4"/></effects>"#;
    let xml = |n: &str| {
        format!(
            r#"<scene version="1.1"><project width="64" height="64" fps="10" duration="1"/><assets><image id="wide" src="w.png" width="8" height="4"/></assets><composition>{n}</composition>{fx}</scene>"#
        )
    };
    let warn = |n: &str| {
        let doc =
            sr_model::load_str(&xml(n), &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
        sr_eval::Evaluator::new(&doc, &Default::default())
            .unwrap()
            .warnings()
            .iter()
            .map(|d| (d.code.clone(), d.message.clone()))
            .collect::<Vec<_>>()
    };
    let w = warn(&nodes(r#"opacity="0""#));
    assert!(w.iter().any(|(c, m)| c == "E19" && m.contains("opacity")), "{w:?}");
    assert!(warn(&nodes(r#"visible="false""#)).is_empty());
    assert!(warn(&nodes("")).is_empty());
}

#[test]
fn an_effect_attribute_its_type_does_not_read_is_reported_naming_it() {
    // effectType is one attribute bag: `intensity` is valid on a vignette, which reads amount, radius and softness
    let fx = |attrs: &str| format!(r#"<effects><effect id="vig" type="vignette" {attrs}/></effects>"#);
    let node = r#"<shape id="s" shape="rect" width="8" height="8" effects="vig"/>"#;
    let w = warnings(node, &fx(r#"intensity="0.55""#));
    let hit: Vec<_> = w.iter().filter(|(c, m)| c == "E19" && m.contains("@intensity")).collect();
    assert_eq!(hit.len(), 1, "{w:?}");
    assert!(
        hit[0].1.contains("@amount") && hit[0].1.contains("@radius") && hit[0].1.contains("@softness"),
        "{}",
        hit[0].1
    );
    // the attributes it does read, the always-allowed ones, and one set to its default are silent
    assert!(warnings(node, &fx(r#"amount="0.55" radius="700" softness="0.6" mix="0.5" enabled="true""#)).is_empty());
    assert!(warnings(node, &fx(r#"intensity="1""#)).is_empty(), "the default is not worth a warning");
    // a type with no list (a custom shader takes any parameter) is not checked
    let shader = r#"<effects><effect id="vig" type="shader" src="x.glsl" intensity="0.3"/></effects>"#;
    assert!(!warnings(node, shader).iter().any(|(c, m)| c == "E19" && m.contains("@intensity")));
}
