//! SREP 19's conformance cases through `scene-render encode` and `output/@report`.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn report(name: &str, scene: &str) -> Value {
    let d = std::env::temp_dir().join(format!("sr-leg-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let file = d.join("case.scene.xml");
    std::fs::write(&file, scene).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_scene-render")).args(["encode", file.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let p: PathBuf = d.join("r.json");
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap()
}

fn scene(check: &str, body: &str, track: &str, size: (u32, u32)) -> String {
    format!(
        r##"<scene version="1.2">
  <project width="{w}" height="{h}" fps="10" duration="2" background="#000000"/>
  <metadata><accessibility {check}/></metadata>
  <output id="o" path="f_%04d.png" codec="png-sequence" report="r.json"/>
  {body}
  <captions>{track}</captions>
</scene>"##,
        w = size.0,
        h = size.1
    )
}

fn leg(r: &Value) -> Vec<Value> {
    r["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["code"].as_str().unwrap().starts_with("LEG-"))
        .cloned()
        .collect()
}

const THIRTY: &str = "abcdefghij klmnopqrs tuvwxyz12";

#[test]
fn srep_0019_speed() {
    let track = format!(
        r#"<captionTrack id="cc" language="en" mode="sidecar" readingSpeed="20"><cue start="0" end="1" text="{THIRTY}"/></captionTrack>"#
    );
    let r = report("speed", &scene(r#"legibilityCheck="warn""#, "<composition/>", &track, (640, 360)));
    let f = leg(&r);
    assert_eq!(f.len(), 1, "{r:#}");
    assert_eq!((f[0]["code"].as_str(), f[0]["severity"].as_str()), (Some("LEG-SPEED"), Some("warning")));
    assert!((f[0]["measured"].as_f64().unwrap() - 30.0).abs() <= 0.1);
    assert_eq!(f[0]["limit"].as_f64(), Some(20.0));
}

#[test]
fn srep_0019_short() {
    let track =
        r#"<captionTrack id="cc" language="en" mode="sidecar"><cue start="0" end="0.5" text="Hi"/></captionTrack>"#;
    let r = report("short", &scene(r#"legibilityCheck="warn""#, "<composition/>", track, (640, 360)));
    let f = leg(&r);
    assert_eq!(f.len(), 1, "{r:#}");
    assert_eq!(f[0]["code"], "LEG-SHORT");
    assert!((f[0]["measured"].as_f64().unwrap() - 0.5).abs() < 1e-9);
}

#[test]
fn srep_0019_size() {
    let body = r#"<assets><text id="t" text="Small print" width="600" height="100" size="24" font="DejaVu Sans"/></assets><composition><layer id="small" asset="t"/></composition>"#;
    let r = report(
        "size",
        &scene(r#"legibilityCheck="warn" minTextSize="3vh""#, body, "", (1920, 1080))
            .replace("<captions></captions>", ""),
    );
    let f: Vec<Value> = leg(&r).into_iter().filter(|f| f["code"] == "LEG-SIZE").collect();
    assert_eq!(f.len(), 1, "{r:#}");
    assert!((f[0]["measured"].as_f64().unwrap() - 24.0).abs() < 1e-9);
    assert!((f[0]["limit"].as_f64().unwrap() - 32.4).abs() < 1e-9);
}

#[test]
fn srep_0019_off() {
    let track = format!(
        r#"<captionTrack id="cc" language="en" mode="sidecar" readingSpeed="20"><cue start="0" end="1" text="{THIRTY}"/></captionTrack>"#
    );
    let r = report("off", &scene(r#"legibilityCheck="off""#, "<composition/>", &track, (640, 360)));
    assert!(leg(&r).is_empty(), "{r:#}");
}
