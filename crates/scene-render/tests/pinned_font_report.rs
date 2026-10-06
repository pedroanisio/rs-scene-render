//! SREP 21, conformance case `srep-0021-pinned-font`, through `scene-render encode` and `output/@report`: under
//! `fontPolicy="pinned"`, with SREP 20's test font as the only asset, a character outside it is `FONT-GLYPH` at error
//! severity, measured as its code point; under `system` the same document's host fonts draw it.

#[path = "../../sr-text/tests/support/test_font.rs"]
mod test_font;

use std::process::Command;

use serde_json::Value;

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn report(name: &str, policy: &str) -> Value {
    let d = std::env::temp_dir().join(format!("sr-pinned-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let font = test_font::build("SREP Test", test_font::Metrics::default());
    std::fs::write(d.join("srep21.ttf"), &font).unwrap();
    let scene = format!(
        r##"<scene version="1.2">
  <project width="320" height="160" fps="10" duration="0.2" background="#000000" fontPolicy="{policy}"/>
  <output id="o" path="f_%04d.png" codec="png-sequence" report="r.json"/>
  <assets>
    <font id="tf" family="SREP Test" src="srep21.ttf" sha256="{}"/>
    <text id="t" text="H&#233;" width="300" height="150" size="100" lineHeight="1.5" color="#FFFFFF" font="SREP Test"/>
  </assets>
  <composition><layer id="l" asset="t"/></composition>
</scene>"##,
        sha256(&font)
    );
    let file = d.join("case.scene.xml");
    std::fs::write(&file, scene).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_scene-render")).args(["encode", file.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_str(&std::fs::read_to_string(d.join("r.json")).unwrap()).unwrap()
}

fn glyph_findings(r: &Value) -> Vec<Value> {
    r["findings"].as_array().unwrap().iter().filter(|f| f["code"] == "FONT-GLYPH").cloned().collect()
}

#[test]
fn srep_0021_pinned_font_reports_the_missing_glyph_as_an_error() {
    let r = report("pinned", "pinned");
    let g = glyph_findings(&r);
    assert_eq!(g.len(), 1, "{r:#}");
    assert_eq!(g[0]["severity"], "error");
    assert_eq!(g[0]["measured"].as_f64(), Some(233.0), "U+00E9");
    assert_eq!(g[0]["node"], "l");
}

#[test]
fn under_the_system_policy_the_host_draws_it() {
    // DejaVu Sans (the engine's text tests need it) has é: no face is missing it
    let r = report("system", "system");
    assert!(glyph_findings(&r).is_empty(), "{r:#}");
}

#[test]
fn strict_fails_on_a_missing_glyph_under_the_pinned_policy() {
    // FONT-GLYPH at error severity is a finding --strict counts (SREP 18: only information is left out)
    let d = std::env::temp_dir().join(format!("sr-pinned-strict-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let font = test_font::build("SREP Test", test_font::Metrics::default());
    std::fs::write(d.join("srep21.ttf"), &font).unwrap();
    let scene = format!(
        r##"<scene version="1.2">
  <project width="320" height="160" fps="10" duration="0.2" background="#000000" fontPolicy="pinned"/>
  <assets>
    <font id="tf" family="SREP Test" src="srep21.ttf" sha256="{}"/>
    <text id="t" text="H&#233;" width="300" height="150" size="100" lineHeight="1.5" color="#FFFFFF" font="SREP Test"/>
  </assets>
  <composition><layer id="l" asset="t"/></composition>
</scene>"##,
        sha256(&font)
    );
    let file = d.join("case.scene.xml");
    std::fs::write(&file, scene).unwrap();
    let frames = d.join("f_%04d.png");
    let run = |strict: bool| {
        let mut args = vec!["encode", file.to_str().unwrap(), "-o", frames.to_str().unwrap()];
        if strict {
            args.push("--strict");
        }
        Command::new(env!("CARGO_BIN_EXE_scene-render")).args(&args).output().unwrap()
    };
    assert_eq!(run(false).status.code(), Some(0));
    let o = run(true);
    assert_eq!(o.status.code(), Some(1), "{}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn a_pinned_document_without_font_assets_reports_every_character() {
    let d = std::env::temp_dir().join(format!("sr-pinned-none-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let scene = r##"<scene version="1.2">
  <project width="320" height="160" fps="10" duration="0.2" background="#000000" fontPolicy="pinned"/>
  <output id="o" path="f_%04d.png" codec="png-sequence" report="r.json"/>
  <assets><text id="t" text="Hi" width="300" height="150" size="100" color="#FFFFFF"/></assets>
  <composition><layer id="l" asset="t"/></composition>
</scene>"##;
    let file = d.join("case.scene.xml");
    std::fs::write(&file, scene).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_scene-render")).args(["encode", file.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let r: Value = serde_json::from_str(&std::fs::read_to_string(d.join("r.json")).unwrap()).unwrap();
    let g = glyph_findings(&r);
    let measured: Vec<f64> = g.iter().map(|f| f["measured"].as_f64().unwrap()).collect();
    assert_eq!(measured, [72.0, 105.0], "H and i: {r:#}");
    assert!(g.iter().all(|f| f["severity"] == "error"), "{r:#}");
}
