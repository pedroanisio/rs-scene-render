//! `--debug-gpu` is a global flag.

use std::process::Command;

#[test]
fn debug_gpu_is_accepted_and_documented() {
    let exe = env!("CARGO_BIN_EXE_scene-render");
    let help = Command::new(exe).arg("--help").env("NO_COLOR", "1").output().unwrap();
    assert!(String::from_utf8_lossy(&help.stdout).contains("--debug-gpu"));
    let corpus =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/valid/kitchen-sink.scene.xml");
    let o = Command::new(exe).args(["--debug-gpu", "validate"]).arg(&corpus).env("NO_COLOR", "1").output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn the_environment_variable_takes_1_0_and_true() {
    let exe = env!("CARGO_BIN_EXE_scene-render");
    let corpus =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/valid/kitchen-sink.scene.xml");
    for v in ["1", "0", "true"] {
        let o = Command::new(exe).arg("validate").arg(&corpus).env("SR_GPU_DEBUG", v).env("NO_COLOR", "1").output().unwrap();
        assert_eq!(o.status.code(), Some(0), "SR_GPU_DEBUG={v}: {}", String::from_utf8_lossy(&o.stderr));
    }
}
