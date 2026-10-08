//! `object3D/@tracking` spaces the glyphs of extruded text, in thousandths of an em of its height.

use super::common;
use common::*;

fn text(version: &str, attrs: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="{version}"><project width="128" height="128" fps="10" duration="2" background="#00000000"/>
        <composition><object3D id="t" primitive="text" text="III" height="30" depth="4" x="64" y="64" {attrs}/></composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// The columns [first, last] holding covered pixels.
fn columns(r: &Rendered) -> (u32, u32) {
    let covered: Vec<u32> = (0..r.size[0]).filter(|&x| (0..r.size[1]).any(|y| r.at(x, y)[3] > 0.5)).collect();
    (*covered.first().unwrap_or(&0), *covered.last().unwrap_or(&0))
}

#[test]
fn tracking_widens_extruded_text() {
    let Some(plain) = render(&text("1.1", "")) else { return };
    let Some(open) = render(&text("1.1", r#"tracking="500""#)) else { return };
    let (a, b) = (columns(&plain), columns(&open));
    assert!(a.1 > a.0, "the plain text is drawn: {a:?}");
    // glyph 2 moves 2 · 500/1000 · 30 = 30 px at the object's depth; the block stays centred on the object's origin,
    // so it widens by 30 px, half on each side (the view's scale at that depth is close to 1)
    let (wa, wb) = ((a.1 - a.0) as f64, (b.1 - b.0) as f64);
    assert!(wb - wa > 20.0 && wb - wa < 40.0, "width grew by {}", wb - wa);
    let (ca, cb) = ((a.0 + a.1) as f64 / 2.0, (b.0 + b.1) as f64 / 2.0);
    assert!((ca - cb).abs() <= 1.5, "the block stays centred: {ca} {cb}");
}
