//! `eval --format json` keeps standard output for the JSON: warnings go to standard error.

use std::process::Command;

#[test]
fn json_output_parses_while_the_document_has_warnings() {
    let dir = std::env::temp_dir().join(format!("sr-eval-json-warn-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("warn.scene.xml");
    // a group with collapse="true" is accepted and has no effect: an evaluator finding (INERT-I9, information)
    std::fs::write(
        &file,
        r#"<scene version="1.1"><project width="32" height="32" fps="10" duration="1"/><composition>
        <group id="g" collapse="true" width="10" height="10"/></composition></scene>"#,
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_scene-render"))
            .arg("eval")
            .arg(&file)
            .args(args)
            .env("NO_COLOR", "1")
            .output()
            .unwrap()
    };
    let o = run(&["--frame", "0", "--format", "json"]);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", String::from_utf8_lossy(&o.stdout)));
    assert_eq!(v["frame"], 0);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("info[INERT-I9]") && err.contains("collapse"), "the warning is on stderr: {err}");
    // the summary format still shows warnings with their source line, on standard output
    let s = run(&["--frame", "0"]);
    assert!(String::from_utf8_lossy(&s.stdout).contains("info[INERT-I9]"), "{}", String::from_utf8_lossy(&s.stdout));
}
