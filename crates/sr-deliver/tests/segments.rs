//! Outputs with segments (SREP 13) and layouts that reframe: which composition time each output
//! frame shows, where the reframing focus puts the picture, and motion blur at segment speed.

mod common;
use std::path::{Path, PathBuf};

/// One device for the whole binary: tests run in parallel, and creating a device per test has
/// deadlocked and crashed the driver.
fn gpu() -> Option<sr_gpu::Gpu> {
    common::gpu()
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-segments-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Renders the document's first output (a PNG sequence) and returns its frames.
fn render(dir: &Path, xml: &str) -> (Vec<image::RgbaImage>, sr_deliver::pipeline::Report) {
    let path = dir.join("scene.xml");
    std::fs::write(&path, xml).unwrap();
    let d = sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let gpu = gpu().expect("gpu");
    let opts = sr_deliver::Options { hardware: sr_media::encode::Hardware::Software, ..Default::default() };
    let o = &d.scene.outputs[0];
    let r = sr_deliver::deliver(&d, o, Some(&gpu), &opts, &mut |_, _| {}).unwrap_or_else(|e| panic!("{e}"));
    let frames = r.files.iter().map(|f| image::open(f).unwrap().to_rgba8()).collect();
    (frames, r)
}

fn rgb(img: &image::RgbaImage, x: u32, y: u32) -> [u8; 3] {
    let p = img.get_pixel(x, y).0;
    [p[0], p[1], p[2]]
}

/// Whether pixel (x, y) is `want`, within 2 levels (colour conversion rounds).
fn near(img: &image::RgbaImage, x: u32, y: u32, want: [u8; 3]) -> bool {
    rgb(img, x, y).iter().zip(want).all(|(a, b)| (*a as i32 - b as i32).abs() <= 2)
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
fn segments_play_spans_in_order_at_their_speed() {
    if gpu().is_none() {
        return;
    }
    let d = dir("map");
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="3" background="#000000"/>
          <output path="{}/f_%03d.png" codec="png-sequence">
            <segment from="2" to="3"/>
            <segment from="0" to="1" speed="2"/>
          </output>
          <composition>{CLOCK}</composition></scene>"##,
        d.display()
    );
    let (frames, r) = render(&d, &xml);
    // 1 s + 0.5 s at 10 fps
    assert_eq!(frames.len(), 15);
    assert!(r.unsupported.is_empty(), "{:?}", r.unsupported);
    // output 0.5 s → composition 2.5 s (blue); 1.2 s → 0.4 s (red)
    assert!(near(&frames[5], 32, 18, [0, 0, 255]), "{:?}", rgb(&frames[5], 32, 18));
    assert!(near(&frames[12], 32, 18, [255, 0, 0]), "{:?}", rgb(&frames[12], 32, 18));
    // the frame exactly at the join (1.0 s) shows the incoming segment (red, composition 0)
    assert!(near(&frames[10], 32, 18, [255, 0, 0]), "{:?}", rgb(&frames[10], 32, 18));
    // the last frame of the first segment is still blue
    assert!(near(&frames[9], 32, 18, [0, 0, 255]), "{:?}", rgb(&frames[9], 32, 18));
}

/// Left half red, right half blue, in a 64 × 36 frame.
const HALVES: &str = r##"<shape id="l" shape="rect" width="32" height="36" x="0" y="0" fill="#FF0000"/>
    <shape id="r" shape="rect" width="32" height="36" x="32" y="0" fill="#0000FF"/>"##;

#[test]
fn layouts_crop_by_focus_and_segments_move_it() {
    if gpu().is_none() {
        return;
    }
    let d = dir("crop");
    // a square crop of the 64 × 36 frame: focusX 0 keeps the left (red), 1 the right (blue)
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="1" background="#000000"/>
          <output path="{}/f_%03d.png" codec="png-sequence" layout="sq">
            <segment from="0" to="0.2"/>
            <segment from="0" to="0.2" focusX="1"/>
            <segment from="0" to="0.2"><animate property="focusX"><key time="0" value="0"/><key time="0.2" value="1"/></animate></segment>
          </output>
          <layouts><layout id="sq" width="36" height="36" reframe="crop" focusX="0"/></layouts>
          <composition>{HALVES}</composition></scene>"##,
        d.display()
    );
    let (frames, _) = render(&d, &xml);
    assert_eq!(frames.len(), 6);
    assert_eq!(frames[0].dimensions(), (36, 36));
    // the layout's focus 0 shows frame columns 0‥35: column 10 is red
    assert!(near(&frames[0], 10, 18, [255, 0, 0]), "{:?}", rgb(&frames[0], 10, 18));
    // the segment's focus 1 shows columns 28‥63: column 10 (frame 38) is blue
    assert!(near(&frames[2], 10, 18, [0, 0, 255]), "{:?}", rgb(&frames[2], 10, 18));
    // animated 0 → 1 over the segment: at segment time 0.1 the crop is centred (columns 14‥49)
    assert!(near(&frames[5], 2, 18, [255, 0, 0]) && near(&frames[5], 33, 18, [0, 0, 255]));
}

#[test]
fn fit_blur_fills_the_margins_with_the_picture() {
    if gpu().is_none() {
        return;
    }
    let d = dir("fitblur");
    // 64 × 36 into 36 × 64: fitted in a 36 × 20 band, the rest a blurred cover of the frame
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="0.1" background="#000000"/>
          <output path="{}/f_%03d.png" codec="png-sequence" layout="tall"/>
          <layouts><layout id="tall" width="36" height="64" reframe="fit-blur"/></layouts>
          <composition>{HALVES}</composition></scene>"##,
        d.display()
    );
    let (frames, _) = render(&d, &xml);
    let f = &frames[0];
    assert_eq!(f.dimensions(), (36, 64));
    // the band is sharp: red left, blue right
    assert!(near(f, 2, 32, [255, 0, 0]) && near(f, 33, 32, [0, 0, 255]));
    // the margin above it is not black: blurred picture, red towards the left
    let top = rgb(f, 4, 4);
    assert!(top[0] > 100 && top[1] < 30, "{top:?}");
    // and a fit leaves it black
    let d2 = dir("fit");
    let (frames, _) =
        render(&d2, &xml.replace("fit-blur", "fit").replace(&d.display().to_string(), &d2.display().to_string()));
    assert!(near(&frames[0], 4, 4, [0, 0, 0]));
}

#[test]
fn motion_blur_spans_the_composition_time_of_the_shutter() {
    if gpu().is_none() {
        return;
    }
    // a white bar moving 40 px/s with a 180° shutter: at speed 1 a frame smears over 2 px of motion
    // (0.05 s), at speed 4 over 8 px (0.2 s of composition time)
    let smear = |speed: &str, to: &str, name: &str| {
        let d = dir(name);
        let xml = format!(
            r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="1" background="#000000" motionBlur="true" shutterAngle="180" shutterPhase="0" motionBlurSamples="16"/>
              <output path="{}/f_%03d.png" codec="png-sequence"><segment from="0" to="{to}" speed="{speed}"/></output>
              <composition><shape id="b" shape="rect" width="4" height="36" y="0" fill="#FFFFFF">
                <animate property="x"><key time="0" value="4"/><key time="1" value="44"/></animate></shape></composition></scene>"##,
            d.display()
        );
        let (frames, _) = render(&d, &xml);
        // columns touched by the bar in the second frame
        let f = &frames[1];
        (0..64).filter(|&x| f.get_pixel(x, 18).0[0] > 8).count()
    };
    let (slow, fast) = (smear("1", "0.4", "blur1"), smear("4", "1", "blur4"));
    assert!(fast >= slow + 4, "speed 4 smears wider: {fast} vs {slow} columns");
}

#[test]
fn a_crossfade_joins_segments_each_through_its_own_clamped_map() {
    if gpu().is_none() {
        return;
    }
    let d = dir("join");
    // blue (2..3) into red (0..1), a linear crossfade of 0.5 s centred on the join at 1 s: 0.75‥1.25
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="3" background="#000000"/>
          <output path="{}/f_%03d.png" codec="png-sequence">
            <segment from="2" to="3"><transition type="crossfade" duration="0.5" curve="linear"/></segment>
            <segment from="0" to="1"/>
          </output>
          <composition>{CLOCK}</composition></scene>"##,
        d.display()
    );
    let (frames, r) = render(&d, &xml);
    assert_eq!(frames.len(), 20);
    assert!(r.unsupported.is_empty(), "{:?}", r.unsupported);
    // outside the window each segment is alone
    assert!(near(&frames[7], 32, 18, [0, 0, 255]), "{:?}", rgb(&frames[7], 32, 18));
    assert!(near(&frames[13], 32, 18, [255, 0, 0]), "{:?}", rgb(&frames[13], 32, 18));
    // before the join the incoming side is extended before its start and clamped at 0 (red), after
    // it the outgoing side runs on past 3 s and is clamped there (blue): no frame is empty
    let mix = |k: usize| {
        let c = rgb(&frames[k], 32, 18);
        assert!(c[1] < 3 && c[0] as u32 + c[2] as u32 > 200, "frame {k}: {c:?}");
        c
    };
    let (a, b, c) = (mix(8), mix(10), mix(12));
    assert!(a[2] > a[0] && c[0] > c[2], "{a:?} {c:?}");
    assert!(b[0].abs_diff(b[2]) < 3, "{b:?}");
}

#[test]
fn the_overlay_is_drawn_in_output_time_over_the_layout() {
    if gpu().is_none() {
        return;
    }
    let d = dir("overlay");
    // a white square keyed on at output time 0.4 in the corner of a 36 × 36 cropped layout, over
    // segments that play the composition at other times and speeds
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="3" background="#000000"/>
          <output path="{}/f_%03d.png" codec="png-sequence" layout="sq" overlay="tag">
            <segment from="2" to="3" speed="2"/>
            <segment from="0" to="1"/>
          </output>
          <layouts><layout id="sq" width="36" height="36" reframe="crop"/></layouts>
          <symbols><symbol id="tag">
            <shape id="w" shape="rect" width="8" height="8" x="28" y="0" fill="#FFFFFF" opacity="0">
              <animate property="opacity"><key time="0" value="0" interpolation="hold"/><key time="0.4" value="1"/></animate>
            </shape>
          </symbol></symbols>
          <composition>{CLOCK}</composition></scene>"##,
        d.display()
    );
    let (frames, r) = render(&d, &xml);
    assert_eq!(frames.len(), 15);
    assert!(r.unsupported.is_empty(), "{:?}", r.unsupported);
    assert!(near(&frames[3], 32, 4, [0, 0, 255]), "{:?}", rgb(&frames[3], 32, 4));
    assert!(near(&frames[4], 32, 4, [255, 255, 255]), "{:?}", rgb(&frames[4], 32, 4));
    // and in the second segment, still over the picture, which shows red below it
    assert!(near(&frames[12], 32, 4, [255, 255, 255]) && near(&frames[12], 10, 20, [255, 0, 0]));
}

/// A 160 × 90 scene for captions (large enough to read them in pixels).
fn captioned(dir: &Path, output: &str, captions: &str) -> String {
    format!(
        r##"<scene version="1.2"><project width="160" height="90" fps="10" duration="3" background="#203040"/>
          {output}
          <markers><marker id="m1" time="0.5"/></markers>
          <composition><shape id="c" shape="rect" width="160" height="90" x="0" y="0" fill="#406080"/></composition>
          {captions}</scene>"##
    )
    .replace("DIR", &dir.display().to_string())
}

fn same(a: &image::RgbaImage, b: &image::RgbaImage) -> bool {
    a.pixels().zip(b.pixels()).all(|(x, y)| x.0.iter().zip(y.0).all(|(p, q)| p.abs_diff(q) <= 1))
}

#[test]
fn composition_captions_burn_once_in_output_time() {
    if gpu().is_none() {
        return;
    }
    // at half speed the cue at 0.2‥0.8 s shows at output 0.4‥1.6 s: burned mapped over the output's
    // frame, exactly as an output-time track with those times, and not burned again by the composition
    let out =
        r#"<output path="DIR/f_%03d.png" codec="png-sequence"><segment from="0" to="1" speed="0.5"/>OWN</output>"#;
    let style = r#"language="en" preset="boxed-line""#;
    let (da, db, dc) = (dir("cap-a"), dir("cap-b"), dir("cap-c"));
    let a = captioned(
        &da,
        &out.replace("OWN", ""),
        &format!(
            r#"<captions><captionTrack id="cc" {style}><cue start="0.2" end="0.8" text="HELLO"/></captionTrack></captions>"#
        ),
    );
    let b = captioned(
        &db,
        &out.replace(
            "OWN",
            &format!(r#"<captionTrack id="own" {style}><cue start="0.4" end="1.6" text="HELLO"/></captionTrack>"#),
        ),
        "",
    );
    let c = captioned(&dc, &out.replace("OWN", ""), "");
    let ((fa, _), (fb, _), (fc, _)) = (render(&da, &a), render(&db, &b), render(&dc, &c));
    assert_eq!(fa.len(), 20);
    for k in 0..20 {
        assert!(same(&fa[k], &fb[k]), "frame {k}");
    }
    assert!(same(&fa[2], &fc[2]) && !same(&fa[4], &fc[4]) && !same(&fa[15], &fc[15]) && same(&fa[16], &fc[16]));
}

#[test]
fn stills_sidecars_and_chapters_are_in_output_time() {
    if gpu().is_none() {
        return;
    }
    let d = dir("stills");
    // segments 0..1 and 2..3: chapters at 0.5 and 2.5 land at 0.5 and 1.5, the one at 1.5 is skipped
    // (Matroska keeps chapter times as given; FFmpeg's MP4 muxer starts the first chapter at 0);
    // the poster's marker (0.5) is the output's frame at 0.5
    let xml = captioned(
        &d,
        r#"<output path="DIR/short.mkv" codec="h264" preset="ultrafast" audio="false">
             <poster path="DIR/poster.png" format="png" marker="m1"/>
             <segment from="0" to="1"/><segment from="2" to="3"/>
           </output>"#,
        r#"<captions><captionTrack id="cc" language="en" mode="sidecar">
             <cue start="0.2" end="0.8" text="first"/><cue start="1.2" end="1.9" text="skipped"/><cue start="2.1" end="2.9" text="second"/>
           </captionTrack></captions>"#,
    )
    .replace(
        r#"<marker id="m1" time="0.5"/>"#,
        r#"<marker id="m1" time="0.5"/><marker time="0.5" kind="chapter" label="Intro"/><marker time="1.5" kind="chapter" label="Gone"/><marker time="2.5" kind="chapter" label="Main"/>"#,
    );
    let path = d.join("scene.xml");
    std::fs::write(&path, xml).unwrap();
    let doc = sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let gpu = gpu().unwrap();
    let opts = sr_deliver::Options { hardware: sr_media::encode::Hardware::Software, ..Default::default() };
    let r = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {}).unwrap();
    let vtt = std::fs::read_to_string(d.join("short.cc.en.vtt")).unwrap();
    assert_eq!(vtt, "WEBVTT\n\n00:00:00.200 --> 00:00:00.800\nfirst\n\n00:00:01.100 --> 00:00:01.900\nsecond\n\n");
    // the first cue shows for 0.6 s
    assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
    let probe = std::process::Command::new(sr_media::ffprobe())
        .args(["-v", "error", "-show_chapters", "-of", "json"])
        .arg(d.join("short.mkv"))
        .output()
        .unwrap();
    let ch: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    let ch = ch["chapters"].as_array().unwrap();
    let got: Vec<(String, String)> = ch
        .iter()
        .map(|c| (c["start_time"].as_str().unwrap().to_string(), c["tags"]["title"].as_str().unwrap().to_string()))
        .collect();
    assert_eq!(got, vec![("0.500000".into(), "Intro".into()), ("1.500000".into(), "Main".into())]);
    assert_eq!(image::open(d.join("poster.png")).unwrap().to_rgba8().dimensions(), (160, 90));
}

#[test]
fn a_segment_on_an_empty_frame_is_reported() {
    if gpu().is_none() {
        return;
    }
    let d = dir("empty");
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="3" background="#000000"/>
          <output path="{}/f_%03d.png" codec="png-sequence"><segment id="late" from="1" to="3"/></output>
          <composition><shape id="c" shape="rect" width="10" height="10" start="0" end="2" fill="#FFFFFF"/></composition></scene>"##,
        d.display()
    );
    let (_, r) = render(&d, &xml);
    assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
    assert!(r.warnings[0].starts_with("late ends"), "{:?}", r.warnings);
}
