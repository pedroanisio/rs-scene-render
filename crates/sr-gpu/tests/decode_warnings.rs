//! A video that stops decoding part-way is a frame error, not a silently held frame. In a process
//! of its own: it replaces the FFmpeg executable.
#![cfg(unix)]

mod common;
use common::*;

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn a_clip_that_stops_decoding_is_reported_on_the_frames_it_cannot_show() {
    let dir = std::env::temp_dir().join(format!("sr-gpu-decode-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let made = Command::new(sr_media::ffmpeg())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=c=red:s=32x16:r=25:d=1,format=yuv420p", "-c:v", "ffv1"])
        .arg(dir.join("clip.mkv"))
        .status()
        .is_ok_and(|s| s.success());
    if !made {
        eprintln!("skipping: ffmpeg could not create the fixture");
        return;
    }
    let xml = r##"<scene version="1.1"><project width="64" height="32" fps="25" duration="1" background="#000000"/>
        <assets><video id="c" src="clip.mkv" width="32" height="16" fps="25" duration="1" colorSpace="rec709"/></assets>
        <composition><layer id="v" asset="c"/></composition></scene>"##;
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir.clone()) };
    let d = sr_model::load_str(xml, &opts).unwrap_or_else(|e| panic!("{e:?}"));
    // stands in for FFmpeg: three frames (32×16 yuv420p), then a failure
    let fake = dir.join("ffmpeg");
    std::fs::write(
        &fake,
        "#!/bin/sh\nhead -c 2304 /dev/zero\necho 'clip.mkv: Invalid data found when processing input' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("SR_FFMPEG", &fake);
    let Some(r) = render_times(&d, &[0.0]) else { return };
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    let r = render_times(&d, &[0.0, 0.4]).unwrap();
    assert!(
        r.stats.errors.iter().any(|e| e.starts_with("v: ") && e.contains("Invalid data found")),
        "{:?}",
        r.stats.errors
    );
    for times in [&[0.0, 0.4, 0.4][..], &[0.0, 0.4, 0.44, 0.44][..]] {
        let r = render_times(&d, times).unwrap();
        assert!(
            r.stats.errors.iter().any(|e| e.starts_with("v: ") && e.contains("Invalid data found")),
            "cached failure was lost: {:?}",
            r.stats.errors
        );
    }
    let two = xml.replace("</composition>", "<layer id=\"other\" asset=\"c\"/></composition>");
    let two = sr_model::load_str(&two, &opts).unwrap();
    let r = render_times(&two, &[0.0, 0.4, 0.4]).unwrap();
    for id in ["v", "other"] {
        assert!(r.stats.errors.iter().any(|e| e.starts_with(&format!("{id}: "))));
    }
    // A normal EOF may hold the final frame without treating it as a decode failure.
    std::fs::write(&fake, "#!/bin/sh\nhead -c 2304 /dev/zero\nexit 0\n").unwrap();
    let r = render_times(&d, &[0.0, 0.4, 0.4]).unwrap();
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    let _ = std::fs::remove_dir_all(&dir);
}
