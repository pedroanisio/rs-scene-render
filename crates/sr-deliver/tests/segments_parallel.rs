//! Parallel encoding of an output with segments. It runs in its own test binary: each worker opens its
//! own GPU device, which is not safe alongside the other GPU tests of one process.

use std::path::{Path, PathBuf};

fn gpu() -> Option<sr_gpu::Gpu> {
    sr_gpu::Gpu::new().map_err(|e| eprintln!("skipping: {e}")).ok()
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-segments-parallel-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A full-frame rectangle red on [0, 1), green on [1, 2), blue on [2, 3).
const CLOCK: &str = r##"<shape id="c" shape="rect" width="64" height="36" x="0" y="0" fill="#FF0000">
      <animate property="fill">
        <key time="0" value="#FF0000" interpolation="hold"/>
        <key time="1" value="#00FF00" interpolation="hold"/>
        <key time="2" value="#0000FF" interpolation="hold"/>
      </animate>
    </shape>"##;

#[test]
fn segments_render_the_same_serially_and_in_parallel() {
    if gpu().is_none() {
        return;
    }
    // a lossless output with a speed change, a crossfade and an overlay: two workers, each with its own
    // chunk of output frames, decode to exactly the serial frames
    let decode = |path: &Path| {
        let out = std::process::Command::new(sr_media::ffmpeg())
            .args(["-v", "error", "-i"])
            .arg(path)
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .output()
            .unwrap();
        out.stdout
    };
    let mut frames = Vec::new();
    for (name, parallel) in [("serial", sr_deliver::Parallel::Count(1)), ("parallel", sr_deliver::Parallel::Count(2))] {
        let d = dir(&format!("par-{name}"));
        let xml = format!(
            r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="3" background="#000000"/>
              <output path="{}/v.mkv" codec="ffv1" overlay="tag" audio="false">
                <segment from="2" to="3"><transition type="crossfade" duration="0.4"/></segment>
                <segment from="0" to="2" speed="1.5"/>
              </output>
              <symbols><symbol id="tag"><shape id="w" shape="rect" width="8" height="8" y="2" fill="#FFFFFF">
                <animate property="x"><key time="0" value="0"/><key time="1.6" value="56"/></animate></shape></symbol></symbols>
              <composition>{CLOCK}</composition></scene>"##,
            d.display()
        );
        let path = d.join("scene.xml");
        std::fs::write(&path, xml).unwrap();
        let doc = sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
        let opts = sr_deliver::Options { parallel, ..Default::default() };
        let r = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu().unwrap()), &opts, &mut |_, _| {}).unwrap();
        assert_eq!(r.segments, if name == "serial" { 1 } else { 2 });
        frames.push(decode(&r.path));
    }
    // 1 s + 2/1.5 s at 10 fps: 24 frames of 64 × 36
    assert_eq!(frames[0].len(), 24 * 64 * 36 * 3);
    assert!(frames[0] == frames[1], "the parallel render differs");
}
