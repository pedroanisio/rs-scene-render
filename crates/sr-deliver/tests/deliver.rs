//! End to end: a scene with a video layer carrying audio, a music track, a
//! beat grid and four outputs, rendered, encoded, measured and delivered.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

fn ffmpeg(args: &[&str]) -> bool {
    Command::new(sr_media::ffmpeg())
        .args(["-v", "error", "-y"])
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn fixtures() -> Option<PathBuf> {
    static D: OnceLock<Option<PathBuf>> = OnceLock::new();
    D.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("sr-deliver-{}", std::process::id()));
        std::fs::create_dir_all(&d).ok()?;
        let p = |n: &str| d.join(n).display().to_string();
        let ok = ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=64x36:r=25:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=2",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
            &p("clip.mp4"),
        ]) && ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=110:sample_rate=48000:duration=3:beep_factor=4",
            "-ac",
            "2",
            &p("music.wav"),
        ]);
        ok.then_some(d)
    })
    .clone()
}

const SCENE: &str = r##"<scene version="1.1">
  <project width="64" height="36" fps="25" duration="2" background="#000000"/>
  <metadata title="Delivery test" author="scene-render"/>
  <output id="main" path="out/main.mp4" codec="h264" preset="ultrafast" colorSpace="rec709">
    <poster path="out/poster.jpg" time="1"/>
    <thumbnail path="out/thumb.png" format="png" width="32"/>
    <destination kind="file" uri="copies/"/>
  </output>
  <output id="sound" path="out/mix.wav" codec="audio-only"/>
  <output id="small" path="out/small.mp4" codec="h264" preset="ultrafast" maxFileSize="20000" audio="false"/>
  <output id="frames" path="out/seq/f_%03d.png" codec="png-sequence" end="0.2"/>
  <assets>
    <video id="clip" src="clip.mp4" width="64" height="36" fps="25" duration="2" hasAudio="true" colorSpace="rec709"/>
    <audio id="musicA" src="music.wav" bpm="120"/>
  </assets>
  <markers><beatGrid bpm="120" source="music"/></markers>
  <composition>
    <layer id="v" asset="clip"/>
  </composition>
  <audioMix sampleRate="48000">
    <audioTrack id="music" asset="musicA" volume="0.5" fadeIn="0.5" fadeCurve="equal-power"/>
    <master normalize="integrated" loudness="-16" truePeak="-1"/>
  </audioMix>
</scene>"##;

fn doc(dir: &std::path::Path, xml: &str) -> sr_model::Document {
    // tests run in parallel in one shared directory: each document gets its own file
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = dir.join(format!("scene-{n}.xml"));
    std::fs::write(&path, xml).unwrap();
    sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"))
}

fn gpu() -> Option<sr_gpu::Gpu> {
    sr_gpu::Gpu::new().map_err(|e| eprintln!("skipping: {e}")).ok()
}

#[test]
fn mix_and_analysis_table() {
    let Some(dir) = fixtures() else { return };
    // envelopes are computed for the tracks something reads: an expression and a link here
    let xml = SCENE.replace(
        r#"<layer id="v" asset="clip"/>"#,
        r#"<layer id="v" asset="clip"><expression property="opacity">audioAmplitude("v")</expression>
      <link property="rotation" source="audio:music" scale="0"/></layer>"#,
    );
    let d = doc(&dir, &xml);
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let sa = sr_deliver::audio::mix_scene(&ev, 25.0, None).unwrap().expect("scene has audio");
    assert_eq!(sa.mix.nodes.len(), 2, "music track and the video layer's audio");
    assert!((sa.mixed.loudness + 16.0).abs() < 0.5, "{}", sa.mixed.loudness);
    assert!(sa.mixed.true_peak <= -0.9);
    assert!(sa.analysis.tracks.contains_key("music") && sa.analysis.tracks.contains_key("v"));
    let beats = sa.analysis.beats.as_ref().unwrap();
    assert_eq!(beats.len(), 4, "{beats:?}");
    assert!(beats.windows(2).all(|w| (w[1] - w[0] - 0.5).abs() < 1e-9));
    // the evaluator reads the table
    let opts = sr_eval::EvalOptions { analysis: sa.analysis.clone(), ..Default::default() };
    let ev2 = sr_eval::Evaluator::new(&d, &opts).unwrap();
    assert!(ev2.program().analysis.amplitude("v", sr_eval::expr::vm::Band::Full, 1.0) > 0.3);
}

#[test]
fn unread_tracks_are_not_analysed() {
    let Some(dir) = fixtures() else { return };
    let d = doc(&dir, SCENE);
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let sa = sr_deliver::audio::mix_scene(&ev, 25.0, None).unwrap().expect("scene has audio");
    assert!(sa.analysis.tracks.is_empty(), "{:?}", sa.analysis.tracks.keys().collect::<Vec<_>>());
    assert_eq!(sa.analysis.beats.as_ref().map(Vec::len), Some(4), "beats still come from the grid's source");
}

/// Per-frame MD5 of the decoded video stream (`ffmpeg -f framemd5`).
fn framemd5(path: &std::path::Path) -> String {
    let out = std::process::Command::new(sr_media::ffmpeg())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-f", "framemd5", "-"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>().join("\n")
}

#[test]
fn parallel_segments_join_to_the_serial_frames() {
    let Some(dir) = fixtures() else { return };
    let Some(gpu) = gpu() else { return };
    // a cold start partway into the range (a video layer here) must give the serial frames
    let d = doc(&dir, SCENE);
    let mut frames = Vec::new();
    for (name, parallel, segments) in
        [("serial", sr_deliver::Parallel::Count(1), 1), ("parallel", sr_deliver::Parallel::Count(2), 2)]
    {
        // lossless, so the joined file must decode to exactly the serial frames
        let path = dir.join(format!("out/join-{name}.mkv"));
        let o = sr_deliver::adhoc_output(&path.display().to_string(), "ffv1").unwrap();
        let opts = sr_deliver::Options {
            hardware: sr_media::encode::Hardware::Software,
            parallel,
            upload: false,
            ..Default::default()
        };
        let r =
            sr_deliver::deliver(&d, &o, Some(&gpu), &opts, &mut |_, _| {}).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((r.frames, r.segments), (50, segments), "{name}");
        assert!(!sr_media::probe(&path).unwrap().audio.is_empty(), "{name}: the mix is muxed");
        frames.push(framemd5(&path));
    }
    assert_eq!(frames[0].lines().count(), 50);
    assert_eq!(frames[0], frames[1], "joined segments differ from the serial encode");
}

#[test]
fn outputs_render_encode_and_deliver() {
    let Some(dir) = fixtures() else { return };
    let Some(gpu) = gpu() else { return };
    let d = doc(&dir, SCENE);
    let opts = sr_deliver::Options { hardware: sr_media::encode::Hardware::Software, ..Default::default() };
    let mut reports = Vec::new();
    for o in &d.scene.outputs {
        let r =
            sr_deliver::deliver(&d, o, Some(&gpu), &opts, &mut |_, _| {}).unwrap_or_else(|e| panic!("{}: {e}", o.path));
        reports.push(r);
    }
    // main: H.264 with AAC, poster and thumbnail, copied to the file destination
    let main = sr_media::probe(&dir.join("out/main.mp4")).unwrap();
    let v = main.video.unwrap();
    assert_eq!((v.codec.as_str(), v.width, v.height), ("h264", 64, 36));
    assert!((main.duration - 2.0).abs() < 0.1, "{}", main.duration);
    assert_eq!(main.audio[0].codec, "aac");
    assert_eq!(reports[0].frames, 50);
    assert!(dir.join("out/poster.jpg").is_file());
    assert_eq!(sr_media::probe(&dir.join("out/thumb.png")).unwrap().video.unwrap().width, 32);
    assert!(dir.join("copies/main.mp4").is_file() && dir.join("copies/poster.jpg").is_file());
    assert_eq!(reports[0].uploads.len(), 3);
    // the decoded video frame at 1 s matches the source clip
    let mut src = sr_media::VideoDecoder::open(&dir.join("clip.mp4"), None, 2).unwrap();
    let mut out = sr_media::VideoDecoder::open(&dir.join("out/main.mp4"), None, 2).unwrap();
    let (a, b) = (src.frame(25).unwrap(), out.frame(25).unwrap());
    let n = a.planes[0].data.len() as f64;
    let err =
        a.planes[0].data.iter().zip(&b.planes[0].data).map(|(x, y)| (*x as f64 - *y as f64).abs()).sum::<f64>() / n;
    let bias = a.planes[0].data.iter().zip(&b.planes[0].data).map(|(x, y)| *y as f64 - *x as f64).sum::<f64>() / n;
    // a BT.1886 source delivered as BT.709 round-trips: only compression noise remains
    assert!(bias.abs() < 1.0 && err < 8.0, "mean luma bias {bias}, error {err}");
    // mix.wav: 24-bit PCM at the normalised loudness
    let wav = sr_media::decode_audio(&dir.join("out/mix.wav"), 0, 48000).unwrap();
    assert_eq!(sr_media::probe(&dir.join("out/mix.wav")).unwrap().audio[0].codec, "pcm_s24le");
    let planar: Vec<Vec<f32>> = (0..2).map(|c| wav.samples.iter().skip(c).step_by(2).copied().collect()).collect();
    let l = sr_audio::loudness::integrated(&planar, 48000.0, &[1.0, 1.0]);
    assert!((l + 16.0).abs() < 0.5, "delivered loudness {l}");
    // maxFileSize: two-pass encodes until the file fits
    let small = std::fs::metadata(dir.join("out/small.mp4")).unwrap().len();
    assert!(small <= 20000, "{small} bytes");
    assert!(reports[2].passes >= 2);
    // PNG sequence for 0..0.2 s: frames 0..4
    for k in 0..5 {
        assert!(dir.join(format!("out/seq/f_{k:03}.png")).is_file(), "frame {k}");
    }
    assert_eq!(reports[3].files.len(), 5);
}

type Requests = Arc<Mutex<Vec<(String, Vec<(String, String)>, usize)>>>;

/// A minimal HTTP/1.1 server that records requests and answers 200.
fn server() -> (String, Requests) {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", l.local_addr().unwrap());
    let log: Requests = Default::default();
    let log2 = log.clone();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            if r.read_line(&mut line).is_err() {
                continue;
            }
            let mut headers = Vec::new();
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                let h = h.trim_end().to_string();
                if h.is_empty() {
                    break;
                }
                if let Some((k, v)) = h.split_once(':') {
                    headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
                }
            }
            let mut w = s;
            if headers.iter().any(|(k, v)| k == "expect" && v.contains("100")) {
                w.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").unwrap();
            }
            let n: usize =
                headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
            let mut body = vec![0u8; n];
            r.read_exact(&mut body).unwrap();
            log2.lock().unwrap().push((line.trim().to_string(), headers, n));
            w.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
        }
    });
    (addr, log)
}

#[test]
fn network_destinations_sign_and_notify() {
    let Some(dir) = fixtures() else { return };
    let (url, log) = server();
    // SAFETY: tests in this binary do not read these variables concurrently
    unsafe {
        std::env::set_var("SR_CREDENTIALS_TEST_AWS_ACCESS_KEY_ID", "AKIDEXAMPLE");
        std::env::set_var("SR_CREDENTIALS_TEST_AWS_SECRET_ACCESS_KEY", "secret");
        std::env::set_var("SR_CREDENTIALS_TEST_S3_ENDPOINT", &url);
        std::env::set_var("SR_CREDENTIALS_TEST_GCS_TOKEN", "gtoken");
        std::env::set_var("SR_CREDENTIALS_TEST_GCS_ENDPOINT", &url);
        std::env::set_var("SR_CREDENTIALS_TEST_AZURE_SAS", "sv=2024&sig=abc");
        std::env::set_var("SR_CREDENTIALS_TEST_TOKEN", "bearer-token");
        std::env::set_var("SR_CREDENTIALS_TEST_WEBHOOK_SECRET", "hook-secret");
    }
    let xml = SCENE.replace(
        r#"<output id="sound" path="out/mix.wav" codec="audio-only"/>"#,
        &format!(
            r#"<output id="sound" path="net/mix.m4a" codec="audio-only">
                 <destination kind="s3" uri="s3://bucket/renders" credentials="test"/>
                 <destination kind="gcs" uri="gs://gbucket/p" credentials="test"/>
                 <destination kind="azure-blob" uri="{url}/container/p" credentials="test"/>
                 <destination kind="http-put" uri="{url}/upload/" credentials="test"/>
                 <destination kind="webhook" uri="{url}/hook" credentials="test"/>
               </output>"#
        ),
    );
    let d = doc(&dir, &xml);
    let o = d.scene.outputs.iter().find(|o| o.id.as_deref() == Some("sound")).unwrap();
    let r = sr_deliver::deliver(&d, o, None, &Default::default(), &mut |_, _| {}).unwrap();
    assert_eq!(
        r.uploads,
        vec![
            "s3://bucket/renders/mix.m4a".to_string(),
            "gs://gbucket/p/mix.m4a".into(),
            format!("{url}/container/p/mix.m4a"),
            format!("{url}/upload/mix.m4a")
        ]
    );
    let reqs = log.lock().unwrap().clone();
    let find = |prefix: &str| {
        reqs.iter().find(|r| r.0.starts_with(prefix)).unwrap_or_else(|| panic!("no request {prefix}: {reqs:?}")).clone()
    };
    let s3 = find("PUT /bucket/renders/mix.m4a");
    let auth = &s3.1.iter().find(|(k, _)| k == "authorization").unwrap().1;
    assert!(
        auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/")
            && auth.contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date"),
        "{auth}"
    );
    let gcs = find("POST /upload/storage/v1/b/gbucket/o?uploadType=media&name=p%2Fmix.m4a");
    assert!(gcs.1.iter().any(|(k, v)| k == "authorization" && v == "Bearer gtoken"));
    let az = find("PUT /container/p/mix.m4a?sv=2024&sig=abc");
    assert!(az.1.iter().any(|(k, v)| k == "x-ms-blob-type" && v == "BlockBlob"));
    let put = find("PUT /upload/mix.m4a");
    assert!(put.2 > 1000 && put.1.iter().any(|(k, v)| k == "authorization" && v == "Bearer bearer-token"));
    let hook = find("POST /hook");
    assert!(hook.1.iter().any(|(k, v)| k == "x-scene-render-signature" && v.starts_with("sha256=") && v.len() == 71));
    assert_eq!(reqs.iter().position(|r| r.0.starts_with("POST /hook")), Some(reqs.len() - 1), "webhook goes last");
}

#[test]
fn failures_are_reported() {
    let Some(dir) = fixtures() else { return };
    // an unreachable destination fails the delivery with its URI
    let xml = SCENE.replace(r#"<output id="sound" path="out/mix.wav" codec="audio-only"/>"#, r#"<output id="sound" path="bad/mix.wav" codec="audio-only"><destination kind="http-put" uri="http://127.0.0.1:9/x/"/></output>"#);
    let d = doc(&dir, &xml);
    let o = d.scene.outputs.iter().find(|o| o.id.as_deref() == Some("sound")).unwrap();
    let e = sr_deliver::deliver(&d, o, None, &Default::default(), &mut |_, _| {}).unwrap_err();
    assert!(e.to_string().contains("http://127.0.0.1:9/x/"), "{e}");
    // invalid codec/container combinations are caught before rendering
    let xml = SCENE.replace(r#"path="out/small.mp4" codec="h264""#, r#"path="out/small.webm" codec="h264""#);
    let d = doc(&dir, &xml);
    let o = d.scene.outputs.iter().find(|o| o.id.as_deref() == Some("small")).unwrap();
    if let Some(g) = gpu() {
        let e = sr_deliver::deliver(&d, o, Some(&g), &Default::default(), &mut |_, _| {}).unwrap_err();
        assert!(e.to_string().contains("cannot hold"), "{e}");
    }
}

#[test]
fn caption_sidecars_are_written_next_to_outputs() {
    let Some(dir) = fixtures() else { return };
    let xml = SCENE
        .replace(r#"<output id="sound" path="out/mix.wav" codec="audio-only"/>"#, r#"<output id="sound" path="cap/mix.wav" codec="audio-only" captions="cc"/>"#)
        .replace("</scene>", r#"<captions><captionTrack id="cc" language="en" mode="burn"><cue start="0.5" end="1.5" text="hello there"/><cue start="5" end="6" text="after the end"/></captionTrack><captionTrack id="sd" language="de" mode="sidecar"><cue start="0" end="1" text="hallo"/></captionTrack></captions></scene>"#);
    let d = doc(&dir, &xml);
    let o = d.scene.outputs.iter().find(|o| o.id.as_deref() == Some("sound")).unwrap();
    let r = sr_deliver::deliver(&d, o, None, &Default::default(), &mut |_, _| {}).unwrap();
    let vtt = std::fs::read_to_string(dir.join("cap/mix.cc.en.vtt")).unwrap();
    assert_eq!(vtt, "WEBVTT\n\n00:00:00.500 --> 00:00:01.500\nhello there\n\n");
    assert!(dir.join("cap/mix.sd.de.vtt").exists(), "sidecar-mode tracks are always written");
    assert!(r.files.iter().any(|f| f.ends_with("mix.cc.en.vtt")));
}
