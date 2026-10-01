//! A failing decoder says why. In a process of its own: it replaces the FFmpeg executable.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use sr_media::VideoDecoder;

#[test]
fn a_failing_decoder_says_why() {
    let dir = std::env::temp_dir().join(format!("sr-media-errors-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let clip = dir.join("clip.mkv");
    let made = Command::new(sr_media::ffmpeg())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=c=red:s=32x16:r=25:d=1,format=yuv420p", "-c:v", "ffv1"])
        .arg(&clip)
        .status()
        .is_ok_and(|s| s.success());
    if !made {
        eprintln!("skipping: ffmpeg could not create the fixture");
        return;
    }
    // stands in for FFmpeg: SR_TEST_BYTES of frames, then a failure
    let fake = dir.join("ffmpeg");
    std::fs::write(
        &fake,
        "#!/bin/sh\nhead -c \"$SR_TEST_BYTES\" /dev/zero\necho 'clip.mkv: Invalid data found when processing input' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("SR_FFMPEG", &fake);
    std::env::set_var("SR_TEST_BYTES", "0");
    let mut dec = VideoDecoder::open(&clip, None, 4).unwrap();
    let e = dec.frame(0).unwrap_err().to_string();
    assert!(e.contains("Invalid data found"), "{e}");
    // three frames (32×16 yuv420p), then the failure: the last good frame is held, with a warning
    std::env::set_var("SR_TEST_BYTES", (3 * 768).to_string());
    let mut dec = VideoDecoder::open(&clip, None, 4).unwrap();
    assert_eq!(dec.frame(0).unwrap().index, 0);
    assert!(dec.warnings.is_empty());
    assert_eq!(dec.frame(5).unwrap().index, 2);
    assert!(dec.warnings.len() == 1 && dec.warnings[0].contains("Invalid data found"), "{:?}", dec.warnings);
}
