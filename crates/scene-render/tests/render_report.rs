//! SREP 18: render reports. The conformance cases `srep-0018-report`, `-failed` and `-deterministic`, run through
//! `scene-render encode`, and the shape of `scene-render-report/1` as the SREP's field table gives it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scene-render")).args(args).env("NO_COLOR", "1").output().expect("binary runs")
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-report-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// One I2 shape (a stroke width without a stroke paint) and one I8 node (starting after the composition ends).
const REPORT_SCENE: &str = r##"<scene version="1.2">
  <project width="64" height="48" fps="10" duration="1" background="#000000"/>
  <composition>
    <shape id="chip" shape="rect" width="20" height="10" fill="#FF0000" strokeWidth="2"/>
    <shape id="late" shape="rect" width="8" height="8" fill="#00FF00" start="5"/>
  </composition>
</scene>"##;

fn encode(dir: &Path, scene: &str, report: &str) -> Output {
    let file = dir.join("case.scene.xml");
    std::fs::write(&file, scene).unwrap();
    let frames = dir.join("frames/f_%04d.png");
    run(&[
        "encode",
        file.to_str().unwrap(),
        "-o",
        frames.to_str().unwrap(),
        "--end",
        "0.2",
        "--report",
        dir.join(report).to_str().unwrap(),
    ])
}

fn read(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}\n{text}", path.display()))
}

fn codes(report: &Value) -> Vec<String> {
    report["findings"].as_array().unwrap().iter().map(|f| f["code"].as_str().unwrap().to_string()).collect()
}

/// The registry codes (SREP 18 and the SREPs that add to it) and the engine-specific namespace.
fn known_code(c: &str) -> bool {
    const FIXED: [&str; 17] = [
        "XSD",
        "ASSET-MISSING",
        "TXT-FIT",
        "TXT-CUT",
        "ACC-FLASH",
        "ACC-CAPTIONS",
        "LEG-CONTRAST",
        "LEG-SPEED",
        "LEG-SHORT",
        "LEG-SIZE",
        "SAFE-AREA",
        "MASK-MISS",
        "FONT-SUB",
        "FONT-GLYPH",
        "SUP-APPROX",
        "SUP-REPORTED",
        "INERT-I1",
    ];
    let inert = c.strip_prefix("INERT-I").is_some_and(|n| n.parse::<u32>().is_ok_and(|n| (1..=13).contains(&n)));
    let sch = c.strip_prefix("SCH-").is_some_and(|id| !id.is_empty());
    let engine = c.strip_prefix("X-rs-scene-render-").is_some_and(|rest| !rest.is_empty());
    FIXED.contains(&c) || inert || sch || engine
}

/// The report's shape, field by field, as SREP 18's Specification 2 and 3 give it.
fn check_shape(r: &Value) {
    let o = r.as_object().expect("one JSON object");
    let mut keys: Vec<&str> = o.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["engine", "findings", "format", "output", "scene"]);
    assert_eq!(r["format"], "scene-render-report/1");
    assert_eq!(r["engine"]["name"], "rs-scene-render");
    assert!(r["engine"]["version"].as_str().is_some_and(|v| !v.is_empty()));
    let sha = r["scene"]["sha256"].as_str().unwrap();
    assert!(sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()), "{sha}");
    for k in ["width", "height"] {
        assert!(r["output"][k].as_u64().is_some(), "output.{k}: {}", r["output"]);
    }
    for k in ["start", "end"] {
        assert!(r["output"][k].as_f64().is_some(), "output.{k}: {}", r["output"]);
    }
    let findings = r["findings"].as_array().expect("findings array");
    let allowed = ["code", "severity", "path", "node", "time", "measured", "limit", "unit", "message"];
    for f in findings {
        for k in f.as_object().unwrap().keys() {
            assert!(allowed.contains(&k.as_str()), "unknown field {k} in {f}");
        }
        let code = f["code"].as_str().unwrap();
        assert!(known_code(code), "code {code} is neither in the registry nor engine-specific");
        assert!(["error", "warning", "info"].contains(&f["severity"].as_str().unwrap()), "{f}");
        assert!(f["path"].as_str().unwrap().starts_with("/scene"), "{f}");
        assert!(f["message"].is_string(), "{f}");
        if let Some(t) = f.get("time") {
            let t = t.as_array().unwrap();
            assert!(t.len() == 2 && t[0].as_f64().unwrap() <= t[1].as_f64().unwrap(), "{f}");
        }
        for k in ["measured", "limit"] {
            assert!(f.get(k).is_none_or(Value::is_number), "{f}");
        }
    }
    // sorted: findings without a time first, then by start time, code and path
    let key = |f: &Value| {
        let t = f.get("time").map(|t| t[0].as_f64().unwrap());
        (
            u8::from(t.is_some()),
            t.unwrap_or(0.0),
            f["code"].as_str().unwrap().to_string(),
            f["path"].as_str().unwrap().to_string(),
        )
    };
    for w in findings.windows(2) {
        let order = key(&w[0]).partial_cmp(&key(&w[1]));
        assert!(order != Some(std::cmp::Ordering::Greater), "findings out of order: {} then {}", w[0], w[1]);
    }
}

#[test]
fn srep_0018_report_lists_exactly_the_inert_findings() {
    let d = dir("report");
    let o = encode(&d, REPORT_SCENE, "case.report.json");
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let r = read(&d.join("case.report.json"));
    check_shape(&r);
    assert_eq!(codes(&r), ["INERT-I2", "INERT-I8"], "{r:#}");
    let f = &r["findings"];
    assert_eq!(f[0]["severity"], "info");
    assert_eq!(f[0]["path"], "/scene/composition/shape[1]");
    assert_eq!(f[0]["node"], "chip");
    assert_eq!(f[1]["severity"], "info");
    assert_eq!(f[1]["path"], "/scene/composition/shape[2]");
    assert_eq!(f[1]["node"], "late");
    // the document as read, the version it declares, and the output as rendered
    let bytes = std::fs::read(d.join("case.scene.xml")).unwrap();
    assert_eq!(r["scene"]["sha256"], sha256_hex(&bytes));
    assert_eq!(r["scene"]["version"], "1.2");
    assert_eq!((r["output"]["width"].as_u64(), r["output"]["height"].as_u64()), (Some(64), Some(48)));
    assert_eq!((r["output"]["start"].as_f64(), r["output"]["end"].as_f64()), (Some(0.0), Some(0.2)));
    // no path outside the document's folder, no host name
    let text = std::fs::read_to_string(d.join("case.report.json")).unwrap();
    assert!(!text.contains(d.to_str().unwrap()), "{text}");
}

#[test]
fn srep_0018_failed_render_still_writes_its_report() {
    let d = dir("failed");
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255])).save(d.join("red.png")).unwrap();
    let wrong = "0".repeat(64);
    let scene = REPORT_SCENE.replace(
        "<composition>",
        &format!(
            r#"<assets><image id="img" src="red.png" width="4" height="4" sha256="{wrong}"/></assets>
  <composition><layer id="pic" asset="img"/>"#
        ),
    );
    let o = encode(&d, &scene, "case.report.json");
    assert_eq!(o.status.code(), Some(1), "the render fails: {}", String::from_utf8_lossy(&o.stdout));
    let r = read(&d.join("case.report.json"));
    check_shape(&r);
    let c = codes(&r);
    assert!(c.contains(&"ASSET-MISSING".to_string()), "{r:#}");
    let missing = r["findings"].as_array().unwrap().iter().find(|f| f["code"] == "ASSET-MISSING").unwrap();
    assert_eq!(missing["severity"], "error");
    assert_eq!(missing["node"], "img");
    // the message names the file relative to the document, never by an absolute path
    assert!(!missing["message"].as_str().unwrap().contains(d.to_str().unwrap()), "{missing}");
}

#[test]
fn srep_0018_reports_are_deterministic() {
    let d = dir("deterministic");
    let first = encode(&d, REPORT_SCENE, "first.json");
    assert_eq!(first.status.code(), Some(0), "{}", String::from_utf8_lossy(&first.stderr));
    let second = encode(&d, REPORT_SCENE, "second.json");
    assert_eq!(second.status.code(), Some(0), "{}", String::from_utf8_lossy(&second.stderr));
    let (a, b) = (std::fs::read(d.join("first.json")).unwrap(), std::fs::read(d.join("second.json")).unwrap());
    assert!(!a.is_empty());
    assert_eq!(a, b, "the same document, engine and output give the same bytes");
}

#[test]
fn text_that_overflows_its_box_is_reported_with_the_overflow() {
    let d = dir("txt-fit");
    let scene = r##"<scene version="1.2">
  <project width="200" height="120" fps="10" duration="1" background="#000000"/>
  <assets>
    <text id="t" text="a&#10;b&#10;c" width="180" height="50" size="20" lineHeight="1.5" font="DejaVu Sans"/>
    <text id="cut" text="ab&#10;cd&#10;ef" width="180" height="100" size="20" maxLines="1" font="DejaVu Sans"/>
  </assets>
  <composition><layer id="tall" asset="t"/><layer id="short" asset="cut" y="60"/></composition>
</scene>"##;
    let o = encode(&d, scene, "r.json");
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let r = read(&d.join("r.json"));
    check_shape(&r);
    let find = |code: &str| r["findings"].as_array().unwrap().iter().find(|f| f["code"] == code).cloned();
    let fit = find("TXT-FIT").unwrap_or_else(|| panic!("no TXT-FIT: {r:#}"));
    assert_eq!(fit["severity"], "warning");
    assert_eq!(fit["node"], "tall");
    // three 30 px lines in a 50 px box
    assert!((fit["measured"].as_f64().unwrap() - 40.0).abs() < 1e-6, "{fit}");
    assert_eq!(fit["unit"], "px");
    assert_eq!(fit["time"][0].as_f64(), Some(0.0));
    let cut = find("TXT-CUT").unwrap_or_else(|| panic!("no TXT-CUT: {r:#}"));
    assert_eq!(cut["node"], "short");
    assert_eq!(cut["measured"].as_f64(), Some(4.0), "{cut}");
}

#[test]
fn several_outputs_need_an_id_in_the_report_path() {
    let d = dir("several");
    let scene = r##"<scene version="1.2">
  <project width="32" height="32" fps="10" duration="0.2" background="#000000"/>
  <output id="a" path="a/f_%04d.png" codec="png-sequence"/>
  <output id="b" path="b/f_%04d.png" codec="png-sequence"/>
  <composition><shape id="s" shape="rect" width="8" height="8"/></composition>
</scene>"##;
    let file = d.join("case.scene.xml");
    std::fs::write(&file, scene).unwrap();
    let o = run(&["encode", file.to_str().unwrap(), "--report", d.join("r.json").to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2), "{}", String::from_utf8_lossy(&o.stderr));
    let o = run(&["encode", file.to_str().unwrap(), "--report", d.join("r-{id}.json").to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    for id in ["a", "b"] {
        let r = read(&d.join(format!("r-{id}.json")));
        check_shape(&r);
        assert_eq!(r["output"]["id"], id);
        assert!(codes(&r).is_empty(), "{r:#}");
    }
}
