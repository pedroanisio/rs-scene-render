//! Spherical video metadata written into MP4 files, read back by us and by ffprobe.

use sr_media::spherical::{inject, read, Projection, Stereo};
use std::process::Command;

#[test]
#[cfg(unix)]
fn metadata_staging_does_not_follow_symlinks_or_replace_existing_temporary_files() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("movie.mp4");
    assert!(encode(&path, true));
    let victim = dir.path().join("unrelated.txt");
    std::fs::write(&victim, b"unrelated data").unwrap();
    let old_temp = path.with_extension("spherical.tmp");
    symlink(&victim, &old_temp).unwrap();
    inject(&path, Projection::Cubemap, Stereo::Mono).unwrap();
    assert_eq!(std::fs::read(&victim).unwrap(), b"unrelated data");
    assert!(!std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_link(&old_temp).unwrap(), victim);
    std::fs::remove_file(&old_temp).unwrap();
    std::fs::write(&old_temp, b"another existing file").unwrap();
    inject(&path, Projection::Equirectangular, Stereo::TopBottom).unwrap();
    assert_eq!(std::fs::read(&old_temp).unwrap(), b"another existing file");
    assert_eq!(read(&path).unwrap(), Some((Projection::Equirectangular, Stereo::TopBottom)));
}

#[test]
#[cfg(unix)]
fn metadata_replacement_preserves_output_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("movie.mp4");
    assert!(encode(&path, true));
    for mode in [0o600, 0o640] {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        inject(&path, Projection::Cubemap, Stereo::Mono).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, mode);
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "staging files must be cleaned up");
}

#[test]
fn oversized_extended_boxes_return_errors_without_panicking() {
    let dir = std::env::temp_dir().join(format!("sr-spherical-malformed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bad.mp4");
    for size in [u64::MAX, u64::MAX - 7, 32, 15] {
        let mut data = vec![0, 0, 0, 8];
        data.extend_from_slice(b"free");
        data.extend_from_slice(&[0, 0, 0, 1]);
        data.extend_from_slice(b"mdat");
        data.extend_from_slice(&size.to_be_bytes());
        std::fs::write(&path, &data).unwrap();
        assert!(read(&path).is_err(), "size={size}");
        assert!(inject(&path, Projection::Cubemap, Stereo::Mono).is_err(), "size={size}");
        assert_eq!(std::fs::read(&path).unwrap(), data);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn changing_projection_removes_old_v1_projection_and_stereo() {
    let dir = std::env::temp_dir().join(format!("sr-spherical-switch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("switch.mp4");
    assert!(encode(&path, true), "ffmpeg with libx264 required for this regression");
    inject(&path, Projection::Equirectangular, Stereo::TopBottom).unwrap();
    let has_v1 = || {
        let data = std::fs::read(&path).unwrap();
        data.windows(b"GSpherical:ProjectionType".len()).any(|b| b == b"GSpherical:ProjectionType")
    };
    assert!(has_v1());
    inject(&path, Projection::Cubemap, Stereo::Mono).unwrap();
    assert_eq!(read(&path).unwrap(), Some((Projection::Cubemap, Stereo::Mono)));
    assert!(!has_v1(), "cubemap must not retain equirectangular/stereo V1 metadata");
    assert!(probe(&path).contains("\"nb_read_frames\": \"10\""));
    inject(&path, Projection::Equirectangular, Stereo::LeftRight).unwrap();
    assert!(has_v1());
    assert_eq!(read(&path).unwrap(), Some((Projection::Equirectangular, Stereo::LeftRight)));
    assert!(probe(&path).contains("\"nb_read_frames\": \"10\""));
    std::fs::remove_dir_all(dir).unwrap();
}

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
