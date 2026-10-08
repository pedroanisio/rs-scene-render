use std::path::PathBuf;

use sr_model::{load_str, validate_str, LoadOptions, Severity};

fn options() -> LoadOptions {
    LoadOptions {
        verify_assets: true,
        base_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/media")),
    }
}

fn scene(assets: &str) -> String {
    format!(
        r#"<scene version="1.1"><project width="64" height="32" fps="30" duration="1"/>
<assets>{assets}</assets><composition/></scene>"#
    )
}

#[test]
fn image_dimensions_follow_exif_orientation() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sr-media/tests/fixtures/still/rotated.jpg");
    for (width, height, warns) in [(8, 12, false), (12, 8, true)] {
        let xml = scene(&format!(r#"<image id="photo" src="{}" width="{width}" height="{height}"/>"#, path.display()));
        let report = validate_str(&xml, &options());
        assert!(!report.has_errors(), "{report}");
        assert_eq!(report.diagnostics.iter().any(|d| d.code == "A07"), warns, "{width}x{height}: {report}");
    }
}

#[test]
fn image_dimension_mismatch_is_a_located_warning_without_a_digest() {
    let xml = scene(r#"<image id="im" src="logo.png" width="1536" height="1024"/>"#);
    let doc = load_str(&xml, &options()).expect("a mismatch does not prevent rendering");
    let warning = doc.warnings().iter().find(|d| d.code == "A07").expect("dimension warning");
    assert_eq!(warning.severity, Severity::Warning);
    assert!(warning.message.contains("1536x1024") && warning.message.contains("16x16"), "{warning:?}");
    assert!(warning.message.contains("logo.png"));
    assert!(warning.loc.line > 0 && warning.loc.column > 0);
    assert!(warning.path.contains("image"));
    assert!(warning.help.is_some());
    let skipped = validate_str(&xml, &LoadOptions::without_assets());
    assert!(!skipped.diagnostics.iter().any(|d| d.code == "A07"));
}

#[test]
fn matching_images_and_smaller_proxy_representations_do_not_warn() {
    let xml = scene(
        r#"<image id="im" src="logo.png" width="16" height="16">
<representation name="proxy" src="logo.png" width="8" height="8"/>
</image>"#,
    );
    let report = validate_str(&xml, &options());
    assert!(!report.has_errors(), "{report}");
    assert!(!report.diagnostics.iter().any(|d| d.code == "A07"), "{report}");
}

#[test]
fn missing_images_keep_the_existing_missing_file_diagnostic() {
    let report = validate_str(&scene(r#"<image id="im" src="absent.png" width="16" height="16"/>"#), &options());
    assert!(report.diagnostics.iter().any(|d| d.code == "A01"));
    assert!(!report.diagnostics.iter().any(|d| d.code == "A07"));
}

#[test]
fn image_headers_are_identified_by_content_and_each_declaration_is_checked() {
    let dir = std::env::temp_dir().join(format!("sr-model-image-headers-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(options().base_dir.unwrap().join("logo.png"), dir.join("image.data")).unwrap();
    let xml = scene(
        r#"<image id="correct" src="image.data" width="16" height="16"/>
<image id="wide" src="image.data" width="32" height="16"/>
<image id="tall" src="image.data" width="16" height="32"/>"#,
    );
    let report = validate_str(&xml, &LoadOptions { verify_assets: true, base_dir: Some(dir.clone()) });
    assert!(!report.has_errors(), "{report}");
    assert_eq!(report.diagnostics.iter().filter(|d| d.code == "A07").count(), 2, "{report}");
    std::fs::remove_dir_all(dir).unwrap();
}
