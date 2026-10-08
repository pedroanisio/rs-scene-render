//! Video layers: YUV conversion, depth, rotation, frame blending and optical flow.

use super::common;
use common::*;

use std::process::Command;

fn ffmpeg(args: &[&str]) -> bool {
    Command::new(sr_media::ffmpeg())
        .args(["-v", "error", "-y"])
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Video fixtures next to the image fixtures; None when FFmpeg is missing.
fn videos() -> Option<std::path::PathBuf> {
    static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let dir = fixtures();
    let ok = *D.get_or_init(|| {
        let p = |n: &str| dir.join(n).display().to_string();
        // tagged limited-range BT.709 in the graph: FFmpeg 8.1 would otherwise treat the generated
        // frames as full range and squeeze the codes into 16-235 while encoding
        let counter = "color=c=black:s=32x16:r=25:d=4,format=yuv420p,geq=lum='mod(N*4\\,256)':cb=128:cr=128,\
                       setparams=range=tv:color_primaries=bt709:color_trc=bt709:colorspace=bt709";
        ffmpeg(&["-f", "lavfi", "-i", counter, "-c:v", "libx264", "-qp", "0", &p("counter.mp4")])
            && ffmpeg(&[
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=32x16:r=25:d=1,format=yuv420p10le",
                "-c:v",
                "ffv1",
                &p("red10.mkv"),
            ])
            && ffmpeg(&[
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=16x16:r=25:d=1",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=16x16:r=25:d=1",
                "-filter_complex",
                "[0][1]hstack,format=yuv444p",
                "-c:v",
                "ffv1",
                &p("halves.mkv"),
            ])
            && ffmpeg(&[
                "-f",
                "lavfi",
                "-i",
                "color=c=white@0.5:s=16x16:r=25:d=1,format=rgba",
                "-c:v",
                "png",
                &p("alpha.mov"),
            ])
    });
    ok.then_some(dir)
}

const VIDEOS: &str = r#"
  <video id="counter" src="counter.mp4" width="32" height="16" fps="25" duration="4" colorSpace="rec709"/>
  <video id="red10" src="red10.mkv" width="32" height="16" fps="25" duration="1" colorSpace="rec709"/>
  <video id="halves" src="halves.mkv" width="16" height="32" fps="25" duration="1" colorSpace="rec709" rotation="90"/>
  <video id="alpha" src="alpha.mov" width="16" height="16" fps="25" duration="1" alpha="straight"/>"#;

fn video_doc(body: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.1"><project width="64" height="32" fps="25" duration="2" background="#000000"/><assets>{VIDEOS}</assets><composition>{body}</composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}"))
}

/// Linear value of a BT.709 limited-range luma code with neutral chroma.
fn y_to_linear(y: f32) -> f32 {
    ((y - 16.0) / 219.0).max(0.0).powf(2.4)
}

#[test]
fn video_frames_follow_source_time_and_decode_bt709() {
    if videos().is_none() {
        return;
    }
    let d = video_doc(r#"<layer id="v" asset="counter"/>"#);
    // frame 30 at t = 1.2 s has luma code 120
    let Some(r) = render_times(&d, &[1.2]) else { return };
    let want = y_to_linear(120.0);
    let got = r.at(4, 4);
    assert!(
        (got[0] - want).abs() < 4e-3 && (got[1] - want).abs() < 4e-3 && (got[2] - want).abs() < 4e-3,
        "{got:?} vs {want}"
    );
    assert_eq!(r.stats.video_frames, 1);
    assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
}

#[test]
fn ten_bit_rotation_and_alpha() {
    if videos().is_none() {
        return;
    }
    let d = video_doc(
        r#"<layer id="r" asset="red10"/><layer id="h" asset="halves" x="40"/><layer id="a" asset="alpha" y="16"/>"#,
    );
    let Some(r) = render(&d) else { return };
    let red = r.at(4, 4);
    assert!(red[0] > 0.97 && red[1] < 0.02 && red[2] < 0.02, "10-bit red {red:?}");
    // rotated 90° clockwise: the left (red) half becomes the top
    let top = r.at(48, 4);
    let bottom = r.at(48, 28);
    assert!(top[0] > 0.9 && top[2] < 0.1, "top {top:?}");
    assert!(bottom[2] > 0.9 && bottom[0] < 0.1, "bottom {bottom:?}");
    // straight alpha 0.5 white over black
    let a = r.at(4, 20);
    assert!((a[0] - 0.5).abs() < 0.01 && (a[3] - 1.0).abs() < 1e-3, "alpha {a:?}");
}

#[test]
fn frame_mix_blends_neighbouring_source_frames() {
    if videos().is_none() {
        return;
    }
    // speed 0.5 at t = 0.44 s → source 0.22 s → frame 5.5 at 25 fps
    let hold = video_doc(r#"<layer id="v" asset="counter" speed="0.5"/>"#);
    let mix = video_doc(r#"<layer id="v" asset="counter" speed="0.5" frameBlend="frame-mix"/>"#);
    let Some(h) = render_times(&hold, &[0.44]) else { return };
    let m = render_times(&mix, &[0.44]).unwrap();
    let (a, b) = (y_to_linear(20.0), y_to_linear(24.0));
    assert!((h.at(4, 4)[0] - a).abs() < 2e-3, "hold {:?} vs {a}", h.at(4, 4));
    assert!((m.at(4, 4)[0] - (a + b) / 2.0).abs() < 2e-3, "mix {:?} vs {}", m.at(4, 4), (a + b) / 2.0);
}

#[test]
fn dis_optical_flow_recovers_a_translation() {
    let Some(gpu) = gpu() else { return };
    let (w, h) = (128u32, 96u32);
    // smooth texture; frame b is frame a moved by (+6, +4) pixels
    let tex = |x: f32, y: f32| 0.5 + 0.25 * (x * 0.21).sin() * (y * 0.17).cos() + 0.2 * ((x + y) * 0.07).sin();
    let img = |dx: f32, dy: f32| -> Vec<[f32; 4]> {
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (x as f32, y as f32)))
            .map(|(x, y)| {
                let v = tex(x - dx, y - dy);
                [v, v, v, 1.0]
            })
            .collect()
    };
    let layout = sr_gpu::resources::source_layout(&gpu.device);
    let up = |px: Vec<[f32; 4]>| {
        sr_gpu::resources::upload(
            &gpu.device,
            &gpu.queue,
            &layout,
            &sr_gpu::resources::Decoded { levels: vec![(w, h, px)], note: None },
            "t",
        )
    };
    let (a, b, mid) = (up(img(0.0, 0.0)), up(img(6.0, 4.0)), img(3.0, 2.0));
    let engine = sr_gpu::video::VideoEngine::new(gpu.device.clone(), gpu.queue.clone());
    let flow = engine.flow(&a, &b);
    let (fw, fh, f) = engine.read_flow(&flow);
    let mut xs: Vec<f32> = Vec::new();
    let mut ys: Vec<f32> = Vec::new();
    for y in fh / 4..3 * fh / 4 {
        for x in fw / 4..3 * fw / 4 {
            xs.push(f[(y * fw + x) as usize][0]);
            ys.push(f[(y * fw + x) as usize][1]);
        }
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let (mx, my) = (xs[xs.len() / 2], ys[ys.len() / 2]);
    // half-resolution flow: (3, 2)
    assert!((mx - 3.0).abs() < 0.25 && (my - 2.0).abs() < 0.25, "median flow ({mx}, {my})");
    // the warped midpoint is closer to the true midpoint than a plain mix
    let ev = sr_eval::Evaluator::new(&doc("", "", ""), &Default::default()).unwrap();
    let r = sr_gpu::Renderer::new(gpu.clone(), ev.program());
    let warped = r.read(&engine.interpolate(&a, &b, 0.5, &layout));
    let mixed = r.read(&engine.mix(&a, &b, 0.5, &layout));
    let err = |img: &[[f32; 4]]| -> f64 {
        let mut s = 0.0;
        for y in 16..h - 16 {
            for x in 16..w - 16 {
                let i = (y * w + x) as usize;
                s += ((img[i][0] - mid[i][0]) as f64).powi(2);
            }
        }
        s
    };
    let (ew, em) = (err(&warped), err(&mixed));
    assert!(ew < em * 0.2, "optical flow error {ew} vs frame mix {em}");
}

#[test]
fn identical_decoded_frames_reuse_conversion_but_changed_planes_do_not() {
    if videos().is_none() {
        return;
    }
    let static_doc = video_doc(r#"<layer id="v" asset="red10"/>"#);
    let Some(first) = render_times(&static_doc, &[0.0]) else { return };
    let reused = render_times(&static_doc, &[0.0, 0.04]).unwrap();
    assert_eq!(first.px, reused.px);
    assert_eq!(first.stats.video_frames, 1);
    assert_eq!(reused.stats.video_frames, 0, "identical planes need no new GPU conversion");
    let moving_doc = video_doc(r#"<layer id="v" asset="counter"/>"#);
    let initial = render_times(&moving_doc, &[1.2]).unwrap();
    let changed = render_times(&moving_doc, &[1.2, 1.24]).unwrap();
    assert_ne!(initial.px, changed.px);
    assert_eq!(changed.stats.video_frames, 1, "changed planes must be converted");
}

#[test]
fn video_cache_separates_input_transfer_and_decoder_frame_rate() {
    if videos().is_none() {
        return;
    }
    for (attributes, equal) in [(r#"fps="25" transfer="linear""#, false), (r#"fps="50""#, true)] {
        let xml = format!(
            r##"<scene version="1.1"><project width="64" height="32" fps="25" duration="2" background="#000000"/>
            <assets>{VIDEOS}<video id="other" src="counter.mp4" width="32" height="16" duration="4" colorSpace="rec709" {attributes}/></assets>
            <composition><layer id="left" asset="counter"/><layer id="right" asset="other" x="32"/></composition></scene>"##
        );
        let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
        let d = sr_model::load_str(&xml, &opts).unwrap();
        let Some(r) = render_times(&d, &[1.2]) else { return };
        let (left, right) = (r.at(4, 4), r.at(36, 4));
        assert_eq!(left == right, equal, "{attributes}: {left:?} vs {right:?}");
        if !equal {
            assert!((right[0] - (120.0 - 16.0) / 219.0).abs() < 0.002);
        }
        assert!(r.stats.errors.is_empty(), "{:?}", r.stats.errors);
    }
}
