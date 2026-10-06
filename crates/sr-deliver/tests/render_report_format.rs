//! SREP 18: the render report's codes, order, placement and determinism, without rendering.

use sr_deliver::render_report::{code, error_findings, report_code, Finding, OutputInfo, ReportWriter};
use sr_model::{Diagnostic, Loc, Severity};

const DOC: &str = r##"<scene version="1.2"><project width="64" height="48" fps="10" duration="2"/>
<assets><image id="img" src="a.png" width="4" height="4"/></assets>
<composition><group id="g"><shape id="a" shape="rect" width="4" height="4"/><shape id="b" shape="rect" width="4" height="4"/></group></composition></scene>"##;

fn output() -> OutputInfo {
    OutputInfo { id: Some("out".into()), width: 64, height: 48, start: 0.0, end: 2.0 }
}

fn offset_of(needle: &str) -> u32 {
    DOC.find(needle).unwrap() as u32
}

fn diag(code: &str, severity: Severity, path: &str, offset: u32) -> Diagnostic {
    let loc = Loc { line: 1, column: 1, offset };
    Diagnostic { severity, ..Diagnostic::error(code, format!("{code} message"), loc, path) }
}

#[test]
fn diagnostic_codes_map_to_the_registry() {
    let d = |c: &str, s: Severity| diag(c, s, "/scene", 0);
    assert_eq!(report_code(&d("S04", Severity::Error)), "XSD");
    assert_eq!(report_code(&d("XML", Severity::Error)), "XSD");
    assert_eq!(report_code(&d("C21", Severity::Error)), "SCH-C21");
    assert_eq!(report_code(&d("A01", Severity::Error)), "ASSET-MISSING");
    assert_eq!(report_code(&d("A02", Severity::Error)), "ASSET-MISSING");
    assert_eq!(report_code(&d("A04", Severity::Error)), "ASSET-MISSING");
    // a sequence that holds its missing frames is the engine's own warning
    assert_eq!(report_code(&d("A04", Severity::Warning)), "X-rs-scene-render-A04");
    assert_eq!(report_code(&d("SA01", Severity::Warning)), "SAFE-AREA");
    assert_eq!(report_code(&d("INERT-I2", Severity::Info)), "INERT-I2");
    assert_eq!(report_code(&d("E19", Severity::Info)), "X-rs-scene-render-E19");
    assert_eq!(report_code(&d("INERT-I13", Severity::Info)), "INERT-I13");
    assert_eq!(report_code(&d("MASK-MISS", Severity::Warning)), "MASK-MISS");
    assert_eq!(code::engine("RENDER"), "X-rs-scene-render-RENDER");
}

#[test]
fn findings_are_placed_by_path_offset_or_id() {
    let w = ReportWriter::new(DOC.as_bytes(), std::path::Path::new("."));
    let by_path = Finding::of_diagnostic(&diag(
        "INERT-I2",
        Severity::Info,
        "/scene/composition/group/shape[2]",
        offset_of(r#"<shape id="b""#),
    ));
    // the evaluator names nodes by id
    let by_id = Finding::of_diagnostic(&diag("INERT-I8", Severity::Info, "a", offset_of(r#"<shape id="a""#)));
    let unknown = Finding::node("TXT-FIT", Severity::Warning, "nowhere", "m");
    let r = w.report(output(), vec![by_path, by_id, unknown]);
    let placed: Vec<(&str, Option<&str>)> = r.findings.iter().map(|f| (f.path.as_str(), f.node.as_deref())).collect();
    assert_eq!(
        placed,
        [
            ("/scene/composition/group/shape[2]", Some("b")),
            ("/scene/composition/group/shape[1]", Some("a")),
            ("/scene", None)
        ]
    );
}

#[test]
fn findings_are_sorted_by_time_then_code_then_path() {
    let w = ReportWriter::new(DOC.as_bytes(), std::path::Path::new("."));
    let f = |c: &str, id: &str| Finding::node(c, Severity::Warning, id, "m");
    let r = w.report(
        output(),
        vec![
            f("TXT-FIT", "b").at_time(1.0, 1.5),
            f("TXT-CUT", "a").at_time(1.0, 1.0),
            f("SAFE-AREA", "a").at_time(0.5, 0.5),
            f("INERT-I2", "b"),
            f("INERT-I2", "a"),
            f("ACC-CAPTIONS", "g"),
            // an exact duplicate is listed once
            f("ACC-CAPTIONS", "g"),
        ],
    );
    let got: Vec<(String, String)> = r.findings.iter().map(|f| (f.code.clone(), f.node.clone().unwrap())).collect();
    let want = [
        ("ACC-CAPTIONS", "g"),
        ("INERT-I2", "a"),
        ("INERT-I2", "b"),
        ("SAFE-AREA", "a"),
        ("TXT-CUT", "a"),
        ("TXT-FIT", "b"),
    ];
    assert_eq!(got, want.map(|(a, b)| (a.to_string(), b.to_string())));
}

#[test]
fn the_report_serialises_the_fields_of_the_format() {
    let w = ReportWriter::new(DOC.as_bytes(), std::path::Path::new("."));
    let f =
        Finding::node("TXT-FIT", Severity::Warning, "a", "too tall").at_time(0.5, 1.0).measuring(12.5, Some(0.0), "px");
    let v = serde_json::to_value(w.report(output(), vec![f])).unwrap();
    assert_eq!(v["format"], "scene-render-report/1");
    assert_eq!(v["engine"]["name"], "rs-scene-render");
    assert_eq!(v["scene"]["version"], "1.2");
    assert_eq!(v["output"], serde_json::json!({"id": "out", "width": 64, "height": 48, "start": 0.0, "end": 2.0}));
    assert_eq!(
        v["findings"][0],
        serde_json::json!({"code": "TXT-FIT", "severity": "warning", "path": "/scene/composition/group/shape[1]",
            "node": "a", "time": [0.5, 1.0], "measured": 12.5, "limit": 0.0, "unit": "px", "message": "too tall"})
    );
    // the same input gives the same bytes
    let a = serde_json::to_string(&w.report(output(), vec![])).unwrap();
    let b = serde_json::to_string(&w.report(output(), vec![])).unwrap();
    assert_eq!(a, b);
}

#[test]
fn messages_keep_no_path_outside_the_document_folder() {
    let root =
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("sr-report-clean-{}", std::process::id()));
    let folder = root.join("scene");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(root.join("shared.png"), b"x").unwrap();
    let folder = std::fs::canonicalize(&folder).unwrap();
    let inside = folder.join("a.png");
    let outside = std::fs::canonicalize(root.join("shared.png")).unwrap();
    let msg = format!(
        "{} does not exist; \"{}\" differs; /scene/composition/layer[2] is fine",
        inside.display(),
        outside.display()
    );
    let w = ReportWriter::new(DOC.as_bytes(), &folder);
    let r = w.report(output(), vec![Finding::scene("ASSET-MISSING", Severity::Error, msg)]);
    let m = &r.findings[0].message;
    assert!(m.starts_with("a.png does not exist"), "{m}");
    assert!(!m.contains(&root.display().to_string()), "{m}");
    assert!(m.contains("…/shared.png"), "{m}");
    assert!(m.contains("/scene/composition/layer[2]"), "{m}");
}

#[test]
fn a_failed_delivery_reports_why() {
    let caps = error_findings(&sr_deliver::DeliverError::Accessibility(
        "requireCaptions: the output has no caption track".into(),
    ));
    assert_eq!(caps.len(), 1);
    assert_eq!((caps[0].code.as_str(), caps[0].severity), ("ACC-CAPTIONS", Severity::Error));
    let render = error_findings(&sr_deliver::DeliverError::Render { time: 1.5, message: "bad frame".into() });
    assert_eq!(render[0].code, "X-rs-scene-render-RENDER");
    assert_eq!(render[0].time, Some([1.5, 1.5]));
    // a check set to error is already a finding of the delivery
    assert!(error_findings(&sr_deliver::DeliverError::Accessibility("contrastCheck: …".into())).is_empty());
}

#[test]
fn a_document_that_cannot_load_still_names_its_outputs() {
    let doc = r#"<scene version="1.2"><project width="64" height="48" fps="10" duration="2"/><output id="m" path="m.mp4" report="m.report.json" height="720"/><composition/></scene>"#;
    let w = ReportWriter::new(doc.as_bytes(), std::path::Path::new("."));
    let outs = w.raw_outputs();
    assert_eq!(outs.len(), 1);
    assert_eq!(outs[0].report.as_deref(), Some("m.report.json"));
    assert_eq!(outs[0].info, OutputInfo { id: Some("m".into()), width: 64, height: 720, start: 0.0, end: 2.0 });
}
