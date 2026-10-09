//! `output/@report` names the file a render report is written to (SREP 18): validation does not ask it to exist, as it
//! asks of every input file.

#[test]
fn a_report_not_written_yet_is_not_a_missing_file() {
    let dir =
        std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("sr-report-a01-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let xml = r#"<scene version="1.2"><project width="16" height="16" fps="1" duration="1"/>
      <output id="o" path="out/o.mp4" codec="h264" report="reports/never-written.json"/><composition/></scene>"#;
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir) };
    let r = sr_model::validate_str(xml, &opts);
    assert!(r.diagnostics.iter().all(|d| d.code != "A01"), "{r}");
    assert!(!r.has_errors(), "{r}");
}
