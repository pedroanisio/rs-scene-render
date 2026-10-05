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
fn an_emitter_asset_is_an_image() {
    let n = |asset: &str| format!(r#"<particleEmitter id="e" emitterAsset="{asset}"/>"#);
    let ok = codes("", &n("pic"));
    assert!(ok.is_empty(), "{ok:?}");
    for asset in ["grid", "words"] {
        let c = codes("", &n(asset));
        assert!(c.contains(&"R43".into()), "{asset}: {c:?}");
    }
}
