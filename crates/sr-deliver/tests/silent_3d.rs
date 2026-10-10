//! The render report's two 3D findings, on the corpus documents that show them: an `object3D` never inside the camera's
//! view over the frames rendered (`tests/corpus/valid/object3d-off-camera`), and a metal with no environment to reflect
//! (`metal-without-environment`). Both are warnings; neither changes the picture.

mod common;

use sr_deliver::render_report::Finding;

const OFF_CAMERA: &str = "X-rs-scene-render-3D-OFF-CAMERA";
const NO_ENVIRONMENT: &str = "X-rs-scene-render-3D-METAL-NO-ENVIRONMENT";

/// Delivers the corpus document `name` as a PNG sequence, with a report; its findings and warnings.
fn deliver(name: &str) -> Option<(Vec<Finding>, Vec<String>)> {
    let gpu = common::gpu()?;
    let corpus = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/valid");
    let d = sr_model::load_file(corpus.join(format!("{name}.scene.xml")), &sr_model::LoadOptions::default())
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    let out = std::env::temp_dir().join(format!("sr-silent-3d-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let o = sr_deliver::adhoc_output(&out.join("f_%03d.png").to_string_lossy(), "png-sequence").unwrap();
    let opts = sr_deliver::Options { report: true, ..Default::default() };
    let r = sr_deliver::deliver(&d, &o, Some(&gpu), &opts, &mut |_, _| {}).unwrap_or_else(|e| panic!("{name}: {e}"));
    let _ = std::fs::remove_dir_all(&out);
    Some((r.findings, r.warnings))
}

fn node(f: &Finding) -> Option<&str> {
    f.at.as_ref().and_then(|a| a.id.as_deref())
}

#[test]
fn an_object_never_in_view_is_a_warning_over_its_window() {
    let Some((findings, warnings)) = deliver("object3d-off-camera") else { return };
    let off: Vec<&Finding> = findings.iter().filter(|f| f.code == OFF_CAMERA).collect();
    // "behind" sits behind the default camera for the whole second; "seen" is in front of it
    assert_eq!(off.len(), 1, "{findings:#?}");
    assert_eq!(node(off[0]), Some("behind"));
    assert_eq!(off[0].severity, sr_model::Severity::Warning);
    assert_eq!(off[0].time, Some([0.0, 0.9]), "drawn at frames 0 to 9 of 10 fps");
    assert!(off[0].message.contains("camera's view"), "{}", off[0].message);
    assert!(warnings.iter().any(|w| w.contains("behind") && w.contains("camera's view")), "{warnings:?}");
    assert!(!findings.iter().any(|f| f.code == NO_ENVIRONMENT), "{findings:#?}");
}

#[test]
fn a_metal_with_no_environment_is_a_warning_with_its_metallic() {
    let Some((findings, warnings)) = deliver("metal-without-environment") else { return };
    let metal: Vec<&Finding> = findings.iter().filter(|f| f.code == NO_ENVIRONMENT).collect();
    assert_eq!(metal.len(), 1, "{findings:#?}");
    assert_eq!(node(metal[0]), Some("ball"));
    assert_eq!(metal[0].severity, sr_model::Severity::Warning);
    assert_eq!((metal[0].measured, metal[0].limit, metal[0].unit.as_deref()), (Some(1.0), Some(0.5), Some("metallic")));
    assert!(metal[0].message.contains("environment"), "{}", metal[0].message);
    assert!(warnings.iter().any(|w| w.contains("ball") && w.contains("environment")), "{warnings:?}");
    assert!(!findings.iter().any(|f| f.code == OFF_CAMERA), "{findings:#?}");
}
