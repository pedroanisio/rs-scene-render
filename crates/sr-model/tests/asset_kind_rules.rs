//! A reference that names an asset of a kind the renderer cannot use is rejected at validate, not at render.

fn codes(paints: &str, node: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="1.1"><project width="64" height="64" fps="24" duration="1"/>
<assets><image id="pic" src="pic.png" width="4" height="4"/><generator id="grid" kind="grid" width="8" height="8"/><text id="words" text="x" width="40" height="20" size="12"/></assets>
{paints}<composition>{node}</composition></scene>"#
    );
    let v = sr_model::validate_str(&xml, &sr_model::LoadOptions::without_assets());
    for d in &v.diagnostics {
        eprintln!("{d:?}");
    }
    v.diagnostics.into_iter().map(|d| d.code).collect()
}

#[test]
fn a_pattern_paint_tiles_an_image_asset_and_nothing_else() {
    let ok = codes(r#"<paints><pattern id="hatch" asset="pic"/></paints>"#, "");
    assert!(ok.is_empty(), "{ok:?}");
    for asset in ["grid", "words"] {
        let c = codes(&format!(r#"<paints><pattern id="hatch" asset="{asset}"/></paints>"#), "");
        assert!(c.contains(&"R42".into()), "{asset}: {c:?}");
    }
}

#[test]
fn pattern_rule_matches_only_paint_children_even_in_invalid_documents() {
    let outside = codes("", r#"<pattern id="bad" asset="grid"/>"#);
    assert!(!outside.contains(&"R42".into()), "{outside:?}");
    let missing = codes(r#"<paints><pattern id="bad"/></paints>"#, "");
    assert!(missing.contains(&"R42".into()), "{missing:?}");
}

#[test]
fn an_emitter_asset_is_an_image() {
    let n = |asset: &str| format!(r#"<particleEmitter id="e" emitterAsset="{asset}"/>"#);
    let ok = codes("", &n("pic"));
    assert!(ok.is_empty(), "{ok:?}");
    for asset in ["grid", "words"] {
        let c = codes("", &n(asset));
        assert!(c.contains(&"R43".into()), "{asset}: {c:?}");
    }
}

#[test]
fn an_effect_source_is_a_composition_node_not_an_asset() {
    // displacement-map, difference-key and shader effects sample another node; an asset id passed validate and
    // then failed at render ("must name a node (place an image asset on a hidden layer)")
    let effect = |source: &str| {
        format!(r#"<effects><effect id="dm" type="displacement-map" source="{source}" amount="4"/></effects>"#)
    };
    let nodes = r#"<layer id="map" asset="pic" visible="false"/><shape id="s" shape="rect" width="8" height="8" effects="dm"/>"#;
    let ok = codes_with_effects(&effect("map"), nodes);
    assert!(ok.is_empty(), "{ok:?}");
    for asset in ["grid", "pic"] {
        let c = codes_with_effects(&effect(asset), nodes);
        assert!(c.contains(&"R44".into()), "{asset}: {c:?}");
    }
}

fn codes_with_effects(effects: &str, node: &str) -> Vec<String> {
    let xml = format!(
        r#"<scene version="1.1"><project width="64" height="64" fps="24" duration="1"/>
<assets><image id="pic" src="pic.png" width="4" height="4"/><generator id="grid" kind="grid" width="8" height="8"/></assets>
<composition>{node}</composition>{effects}</scene>"#
    );
    let v = sr_model::validate_str(&xml, &sr_model::LoadOptions::without_assets());
    v.diagnostics.into_iter().map(|d| d.code).collect()
}

#[test]
fn line_width_belongs_to_grid_generators() {
    let g = |kind: &str| {
        let xml = format!(
            r#"<scene version="1.1"><project width="64" height="64" fps="24" duration="1"/><assets><generator id="g" kind="{kind}" width="8" height="8" lineWidth="1"/></assets><composition/></scene>"#
        );
        sr_model::validate_str(&xml, &sr_model::LoadOptions::without_assets())
            .diagnostics
            .into_iter()
            .map(|d| d.code)
            .collect::<Vec<_>>()
    };
    assert!(g("grid").is_empty(), "{:?}", g("grid"));
    for kind in ["checkerboard", "stripes", "fractal-noise"] {
        assert!(g(kind).contains(&"R45".into()), "{kind}: {:?}", g(kind));
    }
}
