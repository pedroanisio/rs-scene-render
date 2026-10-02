//! A delivery that fails leaves nothing behind. One test, in a process of its own: it looks for this process's
//! temporary directories.

mod common;
fn leftovers() -> Vec<std::path::PathBuf> {
    let prefix = format!("scene-render-{}-", std::process::id());
    std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .map(|e| e.path())
        .collect()
}

#[test]
fn a_failed_delivery_leaves_no_temporary_directory_or_truncated_output() {
    let dir = std::env::temp_dir().join(format!("sr-deliver-cleanup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let load = |name: &str, xml: &str| {
        let path = dir.join(name);
        std::fs::write(&path, xml).unwrap();
        sr_model::load_file(path, &Default::default()).unwrap()
    };
    // fails after the temporary directory is made, without a GPU
    let doc = load(
        "silent.xml",
        r#"<scene version="1.2"><project width="32" height="32" fps="10" duration="1"/>
      <output path="out/silent.m4a" codec="audio-only"/><composition/></scene>"#,
    );
    let r = sr_deliver::deliver(&doc, &doc.scene.outputs[0], None, &Default::default(), &mut |_, _| {});
    assert!(matches!(r, Err(sr_deliver::DeliverError::Invalid(_))), "{r:?}");
    assert_eq!(leftovers(), Vec::<std::path::PathBuf>::new());

    // a frame that cannot be rendered half way: serial, and through the two-pass intermediate
    let Some(gpu) = common::gpu() else { return };
    std::fs::write(dir.join("bad.png"), b"not an image").unwrap();
    for (name, extra) in [("serial", ""), ("twopass", r#"twoPass="true" bitrate="200000""#)] {
        let doc = load(
            "bad.xml",
            &format!(
                r##"<scene version="1.2"><project width="32" height="32" fps="10" duration="2" background="#204060"/>
          <output path="out/{name}.mp4" codec="h264" preset="ultrafast" audio="false" {extra}/>
          <assets><image id="im" src="bad.png" width="32" height="32"/></assets>
          <composition><layer id="bad" asset="im" start="1"/></composition></scene>"##
            ),
        );
        let opts = sr_deliver::Options {
            hardware: sr_media::encode::Hardware::Software,
            parallel: sr_deliver::Parallel::Count(1),
            ..Default::default()
        };
        let r = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {});
        assert!(matches!(r, Err(sr_deliver::DeliverError::Render { .. })), "{name}: {r:?}");
        assert_eq!(leftovers(), Vec::<std::path::PathBuf>::new(), "{name}");
        let out = dir.join(format!("out/{name}.mp4"));
        assert!(!out.exists(), "{name}: a truncated {} is left as if delivered", out.display());
    }
}
