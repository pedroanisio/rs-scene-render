//! SREP 21, conformance case `srep-0021-pinned-font`: under `fontPolicy="pinned"`, with SREP 20's test font as the
//! only asset, a character outside it draws as the font's `.notdef`, a hollow box.

mod common;
use common::*;

#[path = "../../sr-text/tests/support/test_font.rs"]
mod test_font;

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn srep_0021_pinned_font_draws_notdef() {
    let bytes = test_font::build("SREP Test", test_font::Metrics::default());
    std::fs::write(fixtures().join("srep21.ttf"), &bytes).unwrap();
    let xml = format!(
        r##"<scene version="1.2"><project width="640" height="360" fps="10" duration="1" background="#000000" fontPolicy="pinned"/>
  <assets>
    <font id="tf" family="SREP Test" src="srep21.ttf" sha256="{}"/>
    <text id="t" text="H&#233;" width="400" height="300" size="100" lineHeight="1.5" color="#FFFFFF" font="SREP Test"/>
  </assets>
  <composition><layer id="l" asset="t" x="100" y="30"/></composition></scene>"##,
        sha256(&bytes)
    );
    let d = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) })
        .unwrap_or_else(|e| panic!("{e:?}"));
    let Some(r) = render(&d) else { return };
    // H: a full rectangle, columns 105..=154, rows 65..=134
    assert!(r.at(130, 100)[0] > 0.5);
    // é is .notdef: the outer box 165..=214 × 65..=134 with the hole 170..=209 × 70..=129
    assert!(r.at(167, 100)[0] > 0.5, "left side of the box");
    assert!(r.at(190, 100)[0] < 0.1, "the hole");
    assert!(r.at(190, 67)[0] > 0.5, "top side of the box");
}
