//! Decode tests against media generated with FFmpeg's lavfi sources.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use sr_media::{decode_audio, probe, PixelLayout, VideoDecoder};

/// Frame k of the counter videos has luma (k * 4) % 256.
fn fixtures() -> Option<PathBuf> {
    static D: OnceLock<Option<PathBuf>> = OnceLock::new();
    D.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("sr-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        let run = |args: &[&str]| {
            Command::new(sr_media::ffmpeg())
                .args(["-v", "error", "-y"])
                .args(args)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };
        let counter = "color=c=black:s=64x32:r=25:d=4,format=yuv420p,geq=lum='mod(N*4\\,256)':cb=128:cr=128";
        let ok = run(&["-f", "lavfi", "-i", counter, "-c:v", "ffv1", dir.join("counter.mkv").to_str()?])
            && run(&[
                "-f",
                "lavfi",
                "-i",
                counter,
                "-c:v",
                "libx264",
                "-qp",
                "0",
                "-g",
                "25",
                "-bf",
                "2",
                dir.join("counter.mp4").to_str()?,
            ])
            && run(&[
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=32x16:r=25:d=1,format=yuv420p10le",
                "-c:v",
                "ffv1",
                dir.join("deep.mkv").to_str()?,
            ])
            && run(&[
                "-f",
                "lavfi",
                "-i",
                "color=c=red@0.5:s=16x16:r=25:d=1,format=rgba",
                "-c:v",
                "png",
                dir.join("alpha.mov").to_str()?,
            ])
            && run(&[
                "-display_rotation",
                "90",
                "-i",
                dir.join("counter.mp4").to_str()?,
                "-c",
                "copy",
                dir.join("rotated.mp4").to_str()?,
            ])
            && run(&[
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=1000:sample_rate=44100:duration=2",
                "-ac",
                "2",
                "-c:a",
                "pcm_s16le",
                dir.join("tone.wav").to_str()?,
            ]);
        if !ok {
            eprintln!("skipping media tests: ffmpeg could not create fixtures");
            return None;
        }
        Some(dir)
    })
    .clone()
}

#[test]
fn probe_reports_streams_colour_and_rotation() {
    let Some(d) = fixtures() else { return };
    let i = probe(&d.join("counter.mp4")).unwrap();
    let v = i.video.unwrap();
    assert_eq!((v.width, v.height, v.fps, v.depth), (64, 32, 25.0, 8));
    assert!((i.duration - 4.0).abs() < 0.1);
    let deep = probe(&d.join("deep.mkv")).unwrap().video.unwrap();
    assert_eq!(PixelLayout::for_stream(&deep), PixelLayout::Yuv16(1, 1));
    let a = probe(&d.join("alpha.mov")).unwrap().video.unwrap();
    assert!(a.alpha && a.rgb);
    assert_eq!(PixelLayout::for_stream(&a), PixelLayout::Rgba8);
    assert_eq!(probe(&d.join("rotated.mp4")).unwrap().video.unwrap().rotation, 270);
    let t = probe(&d.join("tone.wav")).unwrap();
    assert_eq!((t.audio[0].sample_rate, t.audio[0].channels), (44100, 2));
}

#[test]
fn random_access_is_frame_accurate() {
    let Some(d) = fixtures() else { return };
    for file in ["counter.mkv", "counter.mp4"] {
        let mut dec = VideoDecoder::open(&d.join(file), None, 8).unwrap();
        for k in [0i64, 1, 2, 3, 50, 10, 11, 12, 99, 3, 60, 61, 200] {
            let f = dec.frame(k).unwrap();
            let want = ((k.min(99) * 4) % 256) as u8;
            let got = f.planes[0].data[64 * 16 + 32];
            assert!((got as i32 - want as i32).abs() <= 1, "{file}: frame {k} has luma {got}, want {want}");
        }
        // sequential access after a seek stays on one decoder process
        let before = dec.seeks;
        for k in 20..40 {
            dec.frame(k).unwrap();
        }
        assert!(dec.seeks <= before + 1, "{file}: {} seeks", dec.seeks - before);
    }
}

#[test]
fn frames_resample_to_a_requested_rate() {
    let Some(d) = fixtures() else { return };
    // at 50 fps each source frame appears twice
    let mut dec = VideoDecoder::open(&d.join("counter.mkv"), Some(50.0), 4).unwrap();
    let luma = |dec: &mut VideoDecoder, k| dec.frame(k).unwrap().planes[0].data[0];
    assert_eq!(luma(&mut dec, 20), 40);
    assert_eq!(luma(&mut dec, 21), 40);
    assert_eq!(luma(&mut dec, 22), 44);
}

#[test]
fn deep_and_alpha_layouts_carry_their_samples() {
    let Some(d) = fixtures() else { return };
    let mut dec = VideoDecoder::open(&d.join("deep.mkv"), None, 2).unwrap();
    let f = dec.frame(0).unwrap();
    assert_eq!(f.planes.len(), 3);
    assert_eq!(f.planes[0].data.len(), 32 * 16 * 2);
    let mut dec = VideoDecoder::open(&d.join("alpha.mov"), None, 2).unwrap();
    let f = dec.frame(0).unwrap();
    assert_eq!(&f.planes[0].data[..3], &[255, 0, 0]);
    assert!((127..=128).contains(&f.planes[0].data[3]));
}

#[test]
fn audio_decodes_and_resamples() {
    let Some(d) = fixtures() else { return };
    let a = decode_audio(&d.join("tone.wav"), 0, 48000).unwrap();
    assert_eq!((a.sample_rate, a.channels), (48000, 2));
    assert!((a.frames() as i64 - 96000).abs() < 50, "{}", a.frames());
    let peak = a.samples.iter().fold(0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.01 && peak <= 1.0, "{peak}");
    // 1 kHz: 2,000 sign changes per second in each channel
    let left: Vec<f32> = a.samples.iter().step_by(2).copied().collect();
    let crossings = left[..48000].windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
    assert!((crossings as i64 - 2000).abs() <= 2, "{crossings}");
}
