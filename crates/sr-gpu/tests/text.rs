//! Text layers with animators, charts, codes,
//! formulas, colour emoji and burned-in captions.

mod common;
use common::*;

fn doc_text(assets: &str, extra: &str, body: &str, w: u32, h: u32) -> sr_model::Document {
    let a = ASSETS.replace("</assets>", &format!("{assets}</assets>"));
    let xml = format!(
        r##"<scene version="1.1"><project width="{w}" height="{h}" fps="10" duration="4" background="#000000"/>{a}<composition>{body}</composition>{extra}</scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}"))
}

/// Sum of a channel over a rectangle.
fn sum(r: &Rendered, x0: u32, y0: u32, x1: u32, y1: u32, c: usize) -> f32 {
    let mut s = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            s += r.at(x, y)[c];
        }
    }
    s
}

#[test]
fn text_charts_codes_formulas_and_captions() {
    let d = doc_text(
        r##"<text id="t" text="Hello" width="200" height="60" size="40" color="#FF0000" font="DejaVu Sans"/>
            <text id="e" text="😀" width="60" height="60" size="40"/>
            <chart id="c" kind="column" width="200" height="100" labels="a,b" showAxes="false"><series name="s" values="1 2" color="#00FF00"/></chart>
            <code id="q" kind="qr" data="HELLO" width="100" height="100"/>
            <formula id="f" tex="\frac{a}{b}" width="100" height="100" color="#0000FF"/>"##,
        r##"<captions><captionTrack id="cc" language="en" preset="karaoke" activeColor="#FFFF00" y="95%"><cue start="0" end="4" text="sing along now"/></captionTrack></captions>"##,
        r##"<layer id="lt" asset="t" x="0" y="0"/>
            <layer id="le" asset="e" x="220" y="0"/>
            <layer id="lc" asset="c" x="0" y="100"/>
            <layer id="lq" asset="q" x="220" y="100"/>
            <layer id="lf" asset="f" x="320" y="100"/>"##,
        480,
        300,
    );
    let Some(r) = render_times(&d, &[0.0, 2.0]) else { return };
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    assert!(r.stats.unsupported.is_empty(), "{:?}", r.stats.unsupported);
    // red text in its box, nothing red elsewhere
    assert!(sum(&r, 0, 0, 200, 60, 0) > 300.0);
    assert!(sum(&r, 0, 0, 200, 60, 2) < 1.0);
    // the emoji is a colour bitmap: coloured pixels in its box
    let emoji: f32 = sum(&r, 220, 0, 280, 60, 0) + sum(&r, 220, 0, 280, 60, 1);
    assert!(emoji > 100.0, "{emoji}");
    // green columns
    assert!(sum(&r, 0, 100, 200, 200, 1) > 1000.0);
    // QR on white: a mix of dark and light modules
    let qr_light = sum(&r, 220, 100, 320, 200, 1);
    assert!(qr_light > 3000.0 && qr_light < 9500.0, "{qr_light}");
    // blue formula
    assert!(sum(&r, 320, 100, 420, 200, 2) > 100.0);
    // captions at the bottom: yellow (red + green) karaoke on the sung words
    let cap_r = sum(&r, 0, 220, 480, 300, 0);
    let cap_b = sum(&r, 0, 220, 480, 300, 2);
    assert!(cap_r > 200.0 && cap_r > cap_b * 1.5, "{cap_r} {cap_b}");
}

#[test]
fn text_animators_move_characters() {
    let body = |anim: &str| format!(r##"<layer id="lt" asset="t" x="0" y="0">{anim}</layer>"##);
    let asset = r##"<text id="t" text="ABCD" width="200" height="100" size="40" color="#FFFFFF" font="DejaVu Sans"/>"##;
    // before a fade-in starts the layer is empty; after it ends the text is fully drawn
    let d =
        doc_text(asset, "", &body(r#"<textAnimator preset="fade-in" presetStart="1" presetDuration="1"/>"#), 200, 100);
    let Some(r0) = render_times(&d, &[0.5]) else { return };
    assert!(sum(&r0, 0, 0, 200, 100, 0) < 1.0);
    let r1 = render_times(&d, &[2.5]).unwrap();
    let full = sum(&r1, 0, 0, 200, 100, 0);
    assert!(full > 300.0);
    // presetStart is on the layer's clock (composition time), not measured from the layer's start
    let late = r#"<layer id="lt" asset="t" x="0" y="0" start="2"><textAnimator preset="fade-in" presetStart="2.5" presetDuration="1"/></layer>"#;
    let d = doc_text(asset, "", late, 200, 100);
    assert!(sum(&render_times(&d, &[2.2]).unwrap(), 0, 0, 200, 100, 0) < 1.0);
    assert!(sum(&render_times(&d, &[4.0]).unwrap(), 0, 0, 200, 100, 0) > full * 0.95);
    // without presetStart the preset starts with the layer
    let own =
        r#"<layer id="lt" asset="t" x="0" y="0" start="2"><textAnimator preset="fade-in" presetDuration="1"/></layer>"#;
    let d = doc_text(asset, "", own, 200, 100);
    assert!(sum(&render_times(&d, &[2.1]).unwrap(), 0, 0, 200, 100, 0) < full * 0.5);
    assert!(sum(&render_times(&d, &[3.5]).unwrap(), 0, 0, 200, 100, 0) > full * 0.95);
    // a range selector moving y by +50 pushes the glyphs down
    let d = doc_text(asset, "", &body(r#"<textAnimator y="50"/>"#), 200, 100);
    let r = render_times(&d, &[0.0]).unwrap();
    assert!(sum(&r, 0, 0, 200, 45, 0) < full * 0.1);
    assert!((sum(&r, 0, 0, 200, 100, 0) - full).abs() < full * 0.05);
}

#[test]
fn per_character_blur_spreads_glyphs() {
    let body = |anim: &str| format!(r##"<layer id="lt" asset="t" x="0" y="0">{anim}</layer>"##);
    let asset = r##"<text id="t" text="I I" width="200" height="100" size="60" color="#FFFFFF" font="DejaVu Sans"/>"##;
    let d = doc_text(asset, "", &body(""), 200, 100);
    let Some(sharp) = render_times(&d, &[0.0]) else { return };
    let d = doc_text(asset, "", &body(r#"<textAnimator blur="8"/>"#), 200, 100);
    let soft = render_times(&d, &[0.0]).unwrap();
    assert!(!soft.stats.unsupported.iter().any(|m| m.contains("blur")), "{:?}", soft.stats.unsupported);
    // white text over the opaque background: measure the red channel
    let covered = |r: &Rendered| r.px.iter().filter(|p| p[0] > 0.02).count();
    let total = |r: &Rendered| sum(r, 0, 0, 200, 100, 0);
    assert!(covered(&soft) > covered(&sharp) * 2, "blur spreads: {} vs {}", covered(&soft), covered(&sharp));
    assert!(
        (total(&soft) - total(&sharp)).abs() < total(&sharp) * 0.1,
        "and keeps coverage: {} vs {}",
        total(&soft),
        total(&sharp)
    );
}

#[test]
fn text_animators_move_variable_font_axes() {
    let body = |anim: &str| format!(r##"<layer id="lt" asset="t" x="0" y="0">{anim}</layer>"##);
    let asset = r##"<text id="t" text="Heavy" width="200" height="100" size="48" color="#FFFFFF" font="Lora"/>"##;
    let Some(regular) = render_times(&doc_text(asset, "", &body(""), 200, 100), &[0.0]) else { return };
    let bold =
        render_times(&doc_text(asset, "", &body(r#"<textAnimator variation="wght 700"/>"#), 200, 100), &[0.0]).unwrap();
    assert!(bold.stats.unsupported.iter().all(|m| !m.contains("variation")), "{:?}", bold.stats.unsupported);
    let (a, b) = (sum(&regular, 0, 0, 200, 100, 0), sum(&bold, 0, 0, 200, 100, 0));
    assert!(b > a * 1.2, "wght 700 inks more than the default 400: {b} vs {a}");
}

#[test]
fn font_assets_are_loaded_even_though_no_layer_references_them() {
    // <font> assets are referenced from text (fontAsset), never by layers; they must still be loaded, not
    // silently replaced by the default family. A monospaced "i" is several times wider than a proportional one.
    let mono = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf";
    if !std::path::Path::new(mono).exists() {
        return;
    }
    let ink_right = |attrs: &str| {
        let d = doc_text(
            &format!(
                r##"<font id="fm" src="{mono}" family="DejaVu Sans Mono"/><text id="t" text="iiiiiiiiii" width="460" height="60" size="40" color="#FFFFFF" {attrs}/>"##
            ),
            "",
            r#"<layer id="lt" asset="t" x="0" y="0"/>"#,
            480,
            64,
        );
        let r = render(&d)?;
        assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
        (0..480u32).rev().find(|&x| (0..64).any(|y| r.at(x, y)[0] > 0.5))
    };
    let (Some(prop), Some(mono_w)) = (ink_right(r#"font="DejaVu Sans""#), ink_right(r#"fontAsset="fm""#)) else {
        return;
    };
    assert!(
        mono_w as f32 > prop as f32 * 1.8,
        "fontAsset must select the monospaced file: {mono_w} vs proportional {prop}"
    );
}
