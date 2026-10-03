use sr_model::{validate_str, LoadOptions, Severity};
use std::path::Path;

fn diagnostics(dir: &Path, attrs: &str) -> Vec<sr_model::Diagnostic> {
    let xml = format!(
        r#"<scene version="1.3"><project width="4" height="4" fps="24" duration="1"/><assets><volume id="v" src="frame-%02d.srvol" first="0" last="2" {attrs}/></assets><composition/></scene>"#
    );
    validate_str(&xml, &LoadOptions { verify_assets: true, base_dir: Some(dir.to_owned()) }).diagnostics
}

#[test]
fn volume_asset_verification_expands_all_frames_and_applies_missing_policy() {
    let dir = std::env::temp_dir().join(format!("sr-volume-sequence-assets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for n in [0, 2] {
        std::fs::write(dir.join(format!("frame-{n:02}.srvol")), b"existence fixture").unwrap();
    }
    let strict = diagnostics(&dir, "");
    assert!(strict.iter().any(|d| d.code == "A04" && d.severity == Severity::Error), "{strict:?}");
    assert!(!strict.iter().any(|d| d.code == "A01"), "must not stat the unexpanded pattern: {strict:?}");
    for policy in ["hold", "transparent"] {
        let result = diagnostics(&dir, &format!("missingFrame=\"{policy}\""));
        assert!(result.iter().all(|d| d.severity != Severity::Error), "{result:?}");
        assert!(result.iter().any(|d| d.code == "A04"));
    }
    std::fs::write(dir.join("frame-01.srvol"), b"existence fixture").unwrap();
    assert!(diagnostics(&dir, "").is_empty());
    std::fs::remove_file(dir.join("frame-00.srvol")).unwrap();
    let result = diagnostics(&dir, "missingFrame=\"hold\"");
    assert!(result.iter().any(|d| d.code == "A04" && d.severity == Severity::Error), "{result:?}");
    std::fs::remove_dir_all(dir).unwrap();
}
