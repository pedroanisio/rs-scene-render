use sr_resolve::{resolve, Options, Status};
use std::os::unix::fs::PermissionsExt;

#[test]
fn shared_store_distinguishes_relative_local_models() {
    let root = std::env::temp_dir().join(format!("sr-local-models-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let exe = root.join("fake-piper");
    std::fs::write(
        &exe,
        r#"#!/usr/bin/env python3
import sys, wave, struct
from pathlib import Path
args = sys.argv
model = Path(args[args.index('-m') + 1]).read_text()
with wave.open(args[args.index('-f') + 1], 'wb') as w:
    w.setparams((1, 2, 48000, 0, 'NONE', 'not compressed'))
    w.writeframes(struct.pack('<h', int(model)) * 4800)
"#,
    )
    .unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let old_piper = std::env::var_os("SR_PIPER");
    std::env::set_var("SR_PIPER", &exe);
    let opts = Options { store: Some(root.join("store")), ..Default::default() };
    for (name, model, expected) in
        [("a", "100", Status::Made), ("b", "200", Status::Made), ("c", "100", Status::Restored)]
    {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("voice.onnx"), model).unwrap();
        std::fs::write(dir.join("scene.xml"), format!(r#"<scene version="1.2"><project width="64" height="64" fps="10" duration="1"/><assets><generated id="voice" kind="speech" provider="piper" model="voice.onnx" prompt="Hello" cache="voice.wav" cacheSha256="{}"/></assets><composition/></scene>"#, "0".repeat(64))).unwrap();
        let rows = resolve(&dir.join("scene.xml"), &opts).unwrap();
        assert_eq!(rows[0].status, expected, "{name}: {rows:?}");
        assert_eq!(resolve(&dir.join("scene.xml"), &opts).unwrap()[0].status, Status::UpToDate);
    }
    assert_ne!(std::fs::read(root.join("a/voice.wav")).unwrap(), std::fs::read(root.join("b/voice.wav")).unwrap());
    assert_eq!(std::fs::read(root.join("a/voice.wav")).unwrap(), std::fs::read(root.join("c/voice.wav")).unwrap());
    // Editing a model in place must also invalidate the project's sidecar.
    std::fs::write(root.join("b/voice.onnx"), "300").unwrap();
    assert_eq!(
        resolve(&root.join("b/scene.xml"), &Options { check: true, ..opts.clone() }).unwrap()[0].status,
        Status::Stale
    );
    assert_eq!(resolve(&root.join("b/scene.xml"), &opts).unwrap()[0].status, Status::Made);
    // Piper's adjacent model configuration participates in the identity too.
    std::fs::write(root.join("b/voice.onnx.json"), "{}").unwrap();
    assert_eq!(resolve(&root.join("b/scene.xml"), &opts).unwrap()[0].status, Status::Made);
    std::fs::write(root.join("b/voice.onnx.json"), r#"{"speaker_id_map":{"other":1}}"#).unwrap();
    assert_eq!(resolve(&root.join("b/scene.xml"), &opts).unwrap()[0].status, Status::Made);
    // Named voices use the same identity rules after searching SR_PIPER_VOICES.
    let old_voices = std::env::var_os("SR_PIPER_VOICES");
    let named = sr_resolve::protocol::Request {
        provider: "piper".into(),
        model: "voice".into(),
        base_dir: root.display().to_string(),
        ..Default::default()
    };
    std::env::set_var("SR_PIPER_VOICES", root.join("a"));
    let a = named.key();
    std::env::set_var("SR_PIPER_VOICES", root.join("b"));
    assert_ne!(a, named.key());
    std::env::set_var("SR_PIPER_VOICES", root.join("c"));
    assert_eq!(a, named.key());
    match old_voices {
        Some(v) => std::env::set_var("SR_PIPER_VOICES", v),
        None => std::env::remove_var("SR_PIPER_VOICES"),
    }
    match old_piper {
        Some(v) => std::env::set_var("SR_PIPER", v),
        None => std::env::remove_var("SR_PIPER"),
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn whisper_model_content_participates_in_request_keys() {
    use sr_resolve::protocol::Request;
    let root = std::env::temp_dir().join(format!("sr-whisper-identity-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let model = root.join("custom.bin");
    let req = Request {
        provider: "whisper".into(),
        model: "custom.bin".into(),
        base_dir: root.display().to_string(),
        ..Default::default()
    };
    std::fs::write(&model, b"model A").unwrap();
    let first = req.key();
    std::fs::write(&model, b"model B").unwrap();
    assert_ne!(first, req.key());
    std::fs::remove_dir_all(root).unwrap();
}
