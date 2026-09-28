//! Spherical video metadata written into MP4 files, read back by us and by ffprobe.

use sr_media::spherical::{inject, read, Projection, Stereo};
use std::process::Command;

fn encode(path: &std::path::Path, faststart: bool) -> bool {
    let mut args = vec![
        "-y",
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=64x32:rate=10:duration=1",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
    ];
    if faststart {
        args.extend(["-movflags", "+faststart"]);
    }
    let out = path.to_str().unwrap();
    args.push(out);
    Command::new(sr_media::ffmpeg()).args(&args).status().map(|s| s.success()).unwrap_or(false)
}

fn probe(path: &std::path::Path) -> String {
    let o = Command::new(sr_media::ffprobe())
        .args(["-v", "error", "-count_frames", "-show_streams", "-of", "json", path.to_str().unwrap()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn v1_and_v2_metadata_survive_both_box_orders() {
    let dir = std::env::temp_dir().join(format!("sr-spherical-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (faststart, projection, stereo) in
        [(false, Projection::Equirectangular, Stereo::TopBottom), (true, Projection::Cubemap, Stereo::Mono)]
    {
        let p = dir.join(format!("v{faststart}.mp4"));
        if !encode(&p, faststart) {
            eprintln!("ffmpeg with libx264 unavailable; skipping");
            return;
        }
        inject(&p, projection, stereo).unwrap();
        assert_eq!(read(&p).unwrap(), Some((projection, stereo)));
        let info = probe(&p);
        assert!(info.contains("\"nb_read_frames\": \"10\""), "the file still decodes: {info}");
        assert!(info.contains("Spherical Mapping"), "ffprobe sees sv3d: {info}");
        if stereo == Stereo::TopBottom {
            assert!(info.contains("Stereo 3D"), "ffprobe sees st3d: {info}");
        }
        // injecting twice replaces rather than duplicates
        inject(&p, projection, stereo).unwrap();
        assert!(probe(&p).contains("\"nb_read_frames\": \"10\""));
    }
}
