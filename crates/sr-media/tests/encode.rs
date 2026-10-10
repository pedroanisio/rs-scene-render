//! Every codec and container end to end through the local FFmpeg.

use std::path::{Path, PathBuf};

use sr_media::encode::*;
use sr_media::probe;

fn dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-encode-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn have_ffmpeg() -> bool {
    std::process::Command::new(sr_media::ffmpeg()).arg("-version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn spec(path: &Path, codec: Codec, input: InputFormat) -> EncodeSpec {
    EncodeSpec {
        path: path.to_path_buf(),
        codec,
        container: None,
        width: 64,
        height: 36,
        fps: 25.0,
        input,
        pixel_format: "yuv420p".into(),
        preset: "ultrafast".into(),
        profile: None,
        level: None,
        prores_profile: None,
        crf: 23,
        bitrate: None,
        max_bitrate: None,
        buffer_size: None,
        pass: None,
        keyframe_interval: 2.0,
        b_frames: None,
        faststart: true,
        alpha: false,
        audio: None,
        loop_count: 0,
        start_number: 0,
        color: ColorTags {
            primaries: "bt709".into(),
            transfer: "bt709".into(),
            matrix: "bt709".into(),
            full_range: false,
        },
        hdr: Hdr::default(),
        metadata: vec![("title".into(), "scene-render test".into())],
        chapters: None,
        hardware: Hardware::Software,
        audio_bits: 24,
    }
}

#[test]
fn av1_two_pass_produces_and_consumes_statistics() {
    if !have_ffmpeg() || (!working_encoder("libsvtav1") && !working_encoder("libaom-av1")) {
        eprintln!("FFmpeg with a two-pass AV1 encoder unavailable; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut s = spec(&dir.path().join("two-pass.mp4"), Codec::Av1, InputFormat::Nv12);
    s.height = 64;
    s.bitrate = Some(100_000);
    let log = dir.path().join("stats");
    s.pass = Some((1, log.clone()));
    let (_, encoder) = s.args().unwrap();
    assert!(matches!(encoder.as_str(), "libsvtav1" | "libaom-av1"));
    encode(&s, 25);
    let stats: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("stats"))
        .collect();
    assert!(!stats.is_empty(), "successful pass 1 must write statistics");
    assert!(stats.iter().any(|p| std::fs::metadata(p).unwrap().len() > 0));
    assert!(!s.path.exists(), "pass 1 must not publish output");
    s.pass = Some((2, log));
    encode(&s, 25);
    assert_eq!(probe(&s.path).unwrap().video.unwrap().codec, "av1");
    // A second pass must depend on its first pass's data, not silently encode independently.
    for path in stats {
        std::fs::remove_file(path).unwrap();
    }
    s.path = dir.path().join("missing-stats.mp4");
    let mut e = Encoder::start(&s).unwrap();
    let result = e.write(&frame(s.input, 0, s.width as usize, s.height as usize)).and_then(|_| e.finish());
    assert!(result.is_err(), "pass 2 without statistics must fail");
    assert!(!s.path.exists());
}

#[cfg(unix)]
#[test]
fn av1_two_pass_checks_encoder_statistics_before_selection() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    if let Ok(mode) = std::env::var("SR_AV1_PROBE_TEST") {
        let dir = tempfile::tempdir().unwrap();
        let mut s = spec(&dir.path().join("out.mp4"), Codec::Av1, InputFormat::Nv12);
        s.height = 64;
        s.bitrate = Some(100_000);
        s.pass = Some((1, dir.path().join("pass")));
        if mode == "unsupported" {
            assert!(s.args().is_err(), "an encoder that ignores pass flags must not be selected");
        } else {
            let expected = if mode == "modern" { "libsvtav1" } else { "libaom-av1" };
            assert_eq!(s.args().unwrap().1, expected);
            s.pass.as_mut().unwrap().0 = 2;
            assert_eq!(s.args().unwrap().1, expected);
        }
        s.pass = None;
        assert_eq!(s.args().unwrap().1, "libsvtav1", "single-pass SVT remains available");
        let calls = std::fs::read_to_string(std::env::var("SR_AV1_PROBE_CALLS").unwrap()).unwrap_or_default();
        assert_eq!(calls.lines().count(), 1, "two-pass capability is probed once");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg = dir.path().join("ffmpeg");
    std::fs::write(
        &ffmpeg,
        r#"#!/usr/bin/env python3
import os, sys
a = sys.argv[1:]
codec = a[a.index('-c:v') + 1]
mode = os.environ['SR_AV1_PROBE_TEST']
if '-passlogfile' in a:
    with open(os.environ['SR_AV1_PROBE_CALLS'], 'a') as f: f.write('probe\n')
    path = a[a.index('-passlogfile') + 1] + '-0.log'
    with open(path, 'w') as f: f.write('statistics' if mode == 'modern' else '')
sys.exit(1 if codec == 'libaom-av1' and mode == 'unsupported' else 0)
"#,
    )
    .unwrap();
    std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).unwrap();
    for mode in ["fallback", "unsupported", "modern"] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "av1_two_pass_checks_encoder_statistics_before_selection", "--nocapture"])
            .env("SR_FFMPEG", &ffmpeg)
            .env("SR_AV1_PROBE_TEST", mode)
            .env("SR_AV1_PROBE_CALLS", dir.path().join(format!("calls-{mode}")))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{mode}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn frame(input: InputFormat, k: u32, w: usize, h: usize) -> Vec<u8> {
    match input {
        InputFormat::Nv12 => {
            let mut v = vec![0u8; input.frame_bytes(w as u32, h as u32)];
            for y in 0..h {
                for x in 0..w {
                    v[y * w + x] = (16 + (x * 3 + k as usize * 4) % 200) as u8;
                }
            }
            for c in v[w * h..].iter_mut() {
                *c = 128;
            }
            v
        }
        InputFormat::Rgba8 => (0..w * h).flat_map(|i| [(i % 256) as u8, (k * 8 % 256) as u8, 100, 128]).collect(),
        InputFormat::Rgba16 => (0..w * h)
            .flat_map(|i| {
                [(i as u16).to_le_bytes(), 3000u16.to_le_bytes(), 20000u16.to_le_bytes(), 65535u16.to_le_bytes()]
                    .concat()
            })
            .collect(),
        InputFormat::Gbrapf32 => {
            (0..4).flat_map(|p| (0..w * h).flat_map(move |_| (0.25f32 * p as f32).to_le_bytes())).collect()
        }
        InputFormat::P010 => vec![0x40; input.frame_bytes(w as u32, h as u32)],
    }
}

fn encode(s: &EncodeSpec, frames: u32) {
    let mut e = Encoder::start(s).unwrap_or_else(|err| panic!("{:?}: {err}", s.codec));
    for k in 0..frames {
        e.write(&frame(s.input, k, s.width as usize, s.height as usize)).unwrap();
    }
    e.finish().unwrap_or_else(|err| panic!("{:?} {:?}: {err}", s.codec, s.path));
}

fn tone(d: &Path) -> PathBuf {
    let p = d.join("tone.wav");
    let ok = std::process::Command::new(sr_media::ffmpeg())
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=1",
            "-ac",
            "2",
            "-c:a",
            "pcm_f32le",
        ])
        .arg(&p)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    p
}

#[test]
fn every_video_codec_produces_a_readable_file() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir();
    let cases: Vec<(Codec, &str, InputFormat, &str)> = vec![
        (Codec::H264, "a.mp4", InputFormat::Nv12, "h264"),
        (Codec::H265, "a.mov", InputFormat::Nv12, "hevc"),
        (Codec::Av1, "a.mkv", InputFormat::Nv12, "av1"),
        (Codec::Vp9, "a.webm", InputFormat::Rgba8, "vp9"),
        (Codec::Prores, "p.mov", InputFormat::Rgba16, "prores"),
        (Codec::Dnxhr, "a.mxf", InputFormat::Rgba8, "dnxhd"),
        (Codec::Ffv1, "a.mkv", InputFormat::P010, "ffv1"),
        (Codec::Gif, "a.gif", InputFormat::Rgba8, "gif"),
        (Codec::Apng, "a.png", InputFormat::Rgba8, "apng"),
        (Codec::Webp, "a.webp", InputFormat::Rgba8, "webp"),
    ];
    for (codec, name, input, want) in cases {
        let p = d.join(format!("{codec:?}-{name}"));
        let mut s = spec(&p, codec, input);
        if codec == Codec::Ffv1 {
            s.pixel_format = "yuv420p10le".into();
        }
        if codec == Codec::Dnxhr {
            (s.width, s.height) = (256, 128);
        }
        encode(&s, 10);
        if codec == Codec::Webp {
            // FFmpeg cannot decode animated WebP; check the RIFF chunks instead
            let b = std::fs::read(&p).unwrap();
            assert!(&b[..4] == b"RIFF" && &b[8..12] == b"WEBP", "WebP header");
            assert!(
                b.windows(4).any(|w| w == b"ANIM") && b.windows(4).filter(|w| *w == b"ANMF").count() >= 2,
                "animated WebP"
            );
            continue;
        }
        let info = probe(&p).unwrap_or_else(|e| panic!("{codec:?}: {e}"));
        let v = info.video.unwrap_or_else(|| panic!("{codec:?}: no video"));
        assert_eq!(v.codec, want, "{codec:?}");
        assert_eq!((v.width, v.height), (s.width, s.height), "{codec:?}");
    }
}

#[test]
fn sequences_alpha_and_hdr_metadata() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir();
    for (codec, pat, input) in [
        (Codec::PngSequence, "png/f_%04d.png", InputFormat::Rgba8),
        (Codec::JpegSequence, "jpg/f_%04d.jpg", InputFormat::Rgba8),
        (Codec::ExrSequence, "exr/f_%04d.exr", InputFormat::Gbrapf32),
        (Codec::TiffSequence, "tif/f_%04d.tif", InputFormat::Rgba16),
    ] {
        let mut s = spec(&d.join(pat), codec, input);
        s.start_number = 100;
        encode(&s, 3);
        for k in 100..103 {
            let f = d.join(pat.replace("%04d", &format!("{k:04}")));
            assert!(f.is_file(), "{codec:?}: {} missing", f.display());
        }
    }
    // alpha survives in ProRes 4444 and PNG
    let p = d.join("alpha.mov");
    let mut s = spec(&p, Codec::Prores, InputFormat::Rgba8);
    s.alpha = true;
    encode(&s, 3);
    assert!(probe(&p).unwrap().video.unwrap().alpha);
    // HDR10 static metadata reaches the H.265 bitstream
    let p = d.join("hdr.mkv");
    let mut s = spec(&p, Codec::H265, InputFormat::P010);
    s.pixel_format = "yuv420p10le".into();
    s.color = ColorTags {
        primaries: "bt2020".into(),
        transfer: "smpte2084".into(),
        matrix: "bt2020nc".into(),
        full_range: false,
    };
    s.hdr = Hdr {
        max_cll: Some(1000),
        max_fall: Some(400),
        mastering_display: Some("G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1)".into()),
    };
    encode(&s, 3);
    let out = std::process::Command::new(sr_media::ffprobe())
        .args(["-v", "error", "-show_frames", "-read_intervals", "%+#1", "-of", "json"])
        .arg(&p)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Mastering display metadata") && text.contains("Content light level metadata"), "{text}");
    let v = probe(&p).unwrap().video.unwrap();
    assert_eq!((v.transfer.as_str(), v.primaries.as_str(), v.depth), ("smpte2084", "bt2020", 10));
}

#[test]
fn audio_muxing_audio_only_and_two_pass() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir();
    let wav = tone(&d);
    let p = d.join("av.mp4");
    let mut s = spec(&p, Codec::H264, InputFormat::Nv12);
    s.audio = Some((wav.clone(), "aac".into(), 128000));
    encode(&s, 25);
    let i = probe(&p).unwrap();
    assert_eq!(i.audio.len(), 1);
    assert_eq!(i.audio[0].codec, "aac");
    for (name, want) in [("a.wav", "pcm_s24le"), ("a.m4a", "aac"), ("a.mp3", "mp3")] {
        let p = d.join(name);
        let mut s = spec(&p, Codec::AudioOnly, InputFormat::Nv12);
        s.audio = Some((wav.clone(), "aac".into(), 128000));
        Encoder::start(&s).unwrap().finish().unwrap();
        assert_eq!(probe(&p).unwrap().audio[0].codec, want, "{name}");
    }
    // two-pass ABR lands near its target
    let p = d.join("2pass.mp4");
    let log = d.join("2pass");
    // noise frames, so the encoder needs every bit it is given
    let mut seed = 1u32;
    let noisy: Vec<Vec<u8>> = (0..50)
        .map(|_| {
            let mut f = vec![128u8; InputFormat::Nv12.frame_bytes(64, 36)];
            for v in f[..64 * 36].iter_mut() {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                *v = 16 + (seed >> 24) as u8 % 200;
            }
            f
        })
        .collect();
    for pass in [1u8, 2] {
        let mut s = spec(&p, Codec::H264, InputFormat::Nv12);
        s.bitrate = Some(200_000);
        s.pass = Some((pass, log.clone()));
        let mut e = Encoder::start(&s).unwrap();
        for f in &noisy {
            e.write(f).unwrap();
        }
        e.finish().unwrap();
    }
    let size = std::fs::metadata(&p).unwrap().len() as f64;
    let kbps = size * 8.0 / 2.0 / 1000.0;
    assert!((kbps - 200.0).abs() < 40.0, "{kbps} kbit/s");
}

#[test]
fn validation_hardware_fallback_and_bitrate_fitting() {
    let d = dir();
    let mut s = spec(&d.join("x.webm"), Codec::H264, InputFormat::Nv12);
    assert!(s.validate().is_err(), "h264 in webm");
    s.path = d.join("x.mp4");
    s.alpha = true;
    assert!(s.validate().is_err(), "alpha in h264");
    let s = spec(&d.join("seq.png"), Codec::PngSequence, InputFormat::Rgba8);
    assert!(s.validate().is_err(), "sequence without a pattern");
    if have_ffmpeg() {
        // with no GPU encoder present, auto falls back to software
        let e = choose_encoder(Codec::H264, Hardware::Auto).unwrap();
        assert!(e == "libx264" || e.contains('_'), "{e}");
    }
    // 10 MB over 60 s with 192 kbit/s audio
    assert_eq!(fit_bitrate(10_000_000, 60.0, 192_000), 1_101_333);
}

#[test]
fn stills_in_four_formats() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir();
    let rgba: Vec<u8> = (0..64 * 36).flat_map(|i| [(i % 256) as u8, 50, 200, 255]).collect();
    for (f, name) in [
        (StillFormat::Jpeg, "p.jpg"),
        (StillFormat::Png, "p.png"),
        (StillFormat::Webp, "p.webp"),
        (StillFormat::Avif, "p.avif"),
    ] {
        let p = d.join(name);
        write_still(&p, &rgba, 64, 36, f, Some(32), 0.8).unwrap_or_else(|e| panic!("{name}: {e}"));
        let v = probe(&p).unwrap().video.unwrap();
        assert_eq!(v.width, 32, "{name}");
    }
}

#[test]
fn a_rejected_stream_reports_ffmpegs_reason_and_leaves_no_file() {
    if !have_ffmpeg() {
        return;
    }
    let p = dir().join("odd.mp4");
    let mut s = spec(&p, Codec::H264, InputFormat::Rgba8);
    // yuv420p needs an even width
    s.width = 63;
    let mut e = Encoder::start(&s).unwrap();
    let f = frame(s.input, 0, 63, 36);
    let mut err = None;
    for _ in 0..500 {
        if let Err(x) = e.write(&f) {
            err = Some(x.to_string());
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let err = err.expect("the encoder stops accepting frames");
    assert!(err.contains("divisible by 2"), "{err}");
    drop(e);
    assert!(!p.exists(), "a truncated file is left at {}", p.display());
    // an encode abandoned half way leaves nothing either
    let p = dir().join("abandoned.mkv");
    let s = spec(&p, Codec::Ffv1, InputFormat::Rgba8);
    let mut e = Encoder::start(&s).unwrap();
    for k in 0..3 {
        e.write(&frame(s.input, k, 64, 36)).unwrap();
    }
    drop(e);
    assert!(!p.exists(), "a truncated file is left at {}", p.display());
}

#[test]
fn a_percent_sign_in_the_directory_is_not_a_frame_pattern() {
    if !have_ffmpeg() {
        return;
    }
    let pattern = dir().join("50%/f_%04d.png");
    encode(&spec(&pattern, Codec::PngSequence, InputFormat::Rgba8), 2);
    for k in 0..2 {
        assert!(dir().join(format!("50%/f_{k:04}.png")).is_file(), "frame {k}");
    }
}

#[test]
fn test_encoder_preserves_existing_output_until_success() {
    if !have_ffmpeg() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("existing.mp4");
    let old = b"previous successful deliverable";
    std::fs::write(&p, old).unwrap();
    let mut s = spec(&p, Codec::H264, InputFormat::Rgba8);
    s.width = 63;
    let mut e = Encoder::start(&s).unwrap();
    let _ = e.write(&frame(s.input, 0, 63, 36));
    assert!(e.finish().is_err());
    assert_eq!(std::fs::read(&p).unwrap(), old);
    s.width = 64;
    let mut e = Encoder::start(&s).unwrap();
    e.write(&frame(s.input, 0, 64, 36)).unwrap();
    drop(e);
    assert_eq!(std::fs::read(&p).unwrap(), old);
    encode(&s, 2);
    assert_ne!(std::fs::read(&p).unwrap(), old);
    assert!(probe(&p).unwrap().video.is_some());
    assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
}

#[test]
fn test_failed_segment_join_preserves_existing_output() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let segment = dir.path().join("segment.mp4");
    encode(&spec(&segment, Codec::H264, InputFormat::Rgba8), 2);
    let output = dir.path().join("existing.webm");
    std::fs::write(&output, b"previous render").unwrap();
    let s = spec(&output, Codec::H264, InputFormat::Rgba8);
    assert!(s.join(&[segment], &dir.path().join("segments.txt")).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), b"previous render");
}

#[cfg(unix)]
#[test]
fn staging_is_private_and_publication_preserves_permissions() {
    use std::os::unix::fs::PermissionsExt;
    if !have_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    for mode in [0o600, 0o640] {
        let output = dir.path().join(format!("private-{mode}.mkv"));
        std::fs::write(&output, b"old").unwrap();
        std::fs::set_permissions(&output, std::fs::Permissions::from_mode(mode)).unwrap();
        let s = spec(&output, Codec::Ffv1, InputFormat::Rgba8);
        let mut encoder = Encoder::start(&s).unwrap();
        let stage = PathBuf::from(encoder.command.last().unwrap());
        assert_eq!(std::fs::metadata(&stage).unwrap().permissions().mode() & 0o777, 0o600);
        encoder.write(&frame(s.input, 0, 64, 36)).unwrap();
        encoder.finish().unwrap();
        assert_eq!(std::fs::metadata(&output).unwrap().permissions().mode() & 0o777, mode);
        assert!(!stage.exists());
    }
}

#[test]
fn ffmpegs_aac_encoder_runs_without_the_tools_that_click() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir();
    let after_codec = |s: &EncodeSpec| {
        let (args, _) = s.args().unwrap();
        let at = args.iter().position(|a| a == "-c:a").unwrap();
        args[at + 1..].iter().take_while(|a| *a != "-shortest" && !a.starts_with("-map")).cloned().collect::<Vec<_>>()
    };
    // muxed under a picture, and alone in an M4A file
    let mut video = spec(&d.join("pns.mp4"), Codec::H264, InputFormat::Nv12);
    video.audio = Some((d.join("pns.wav"), "aac".into(), 192_000));
    assert_eq!(after_codec(&video)[..7], ["aac", "-b:a", "192000", "-aac_pns", "0", "-aac_tns", "0"]);
    let mut alone = spec(&d.join("pns.m4a"), Codec::AudioOnly, InputFormat::Nv12);
    alone.audio = Some((d.join("pns.wav"), "aac".into(), 128_000));
    assert_eq!(after_codec(&alone)[..7], ["aac", "-b:a", "128000", "-aac_pns", "0", "-aac_tns", "0"]);
    // the switches belong to that encoder alone
    video.audio = Some((d.join("pns.wav"), "libopus".into(), 96_000));
    assert!(!after_codec(&video).iter().any(|a| a.starts_with("-aac_")));
}

/// libx264 and libx265 choose their thread counts from the host's cores, and their output depends on those counts: the
/// same binary gave different MP4 bytes on an 8-core Intel VM (style3) and 6-core AMD VMs (Lula 3.0.0 rebuild, 6 of 32
/// segments; reproduced 2026-10-09 with `ffmpeg -c:v libx264` at `-threads` auto, 6, 8, 9 and 12, where each fixed count
/// gave the same bytes on both hosts). A fixed count makes an encode the same on every host.
#[test]
fn the_software_encoders_use_a_fixed_number_of_threads() {
    if !have_ffmpeg() {
        return;
    }
    let d = dir();
    let value_after = |args: &[String], flag: &str| args.iter().position(|a| a == flag).map(|k| args[k + 1].clone());
    let h264 = spec(&d.join("threads.mp4"), Codec::H264, InputFormat::Nv12);
    let (args, encoder) = h264.args().unwrap();
    if encoder == "libx264" {
        assert_eq!(value_after(&args, "-threads").as_deref(), Some(SOFTWARE_ENCODER_THREADS.to_string().as_str()));
    }
    let h265 = spec(&d.join("threads-hevc.mp4"), Codec::H265, InputFormat::Nv12);
    if let Ok((args, "libx265")) = h265.args().as_ref().map(|(a, e)| (a.clone(), e.as_str())) {
        let params = value_after(&args, "-x265-params").unwrap_or_default();
        assert!(params.contains(&format!("pools={SOFTWARE_ENCODER_THREADS}")), "{params}");
        assert!(params.contains("frame-threads="), "{params}");
    }
}
