//! Accessibility: flash analysis, text contrast and required captions.

mod common;
use sr_deliver::access::{FlashDetector, COLS, ROWS};

// As in the segment tests, share one device: concurrent device creation can crash
// the software Vulkan driver. Each test still owns its renderers and outputs.
fn gpu() -> Option<sr_gpu::Gpu> {
    common::gpu()
}

fn frame(v: [f64; 3], cells: Option<usize>) -> Vec<[f64; 3]> {
    (0..COLS * ROWS).map(|k| if cells.map(|n| k < n).unwrap_or(true) { v } else { [0.0; 3] }).collect()
}

fn run(fps: f64, secs: f64, hz: f64, on: [f64; 3], cells: Option<usize>) -> FlashDetector {
    let mut d = FlashDetector::default();
    let n = (fps * secs) as usize;
    for k in 0..n {
        let t = k as f64 / fps;
        let lit = ((t * hz * 2.0).floor() as u64).is_multiple_of(2);
        d.push(t, &frame(if lit { on } else { [0.0; 3] }, cells));
    }
    d
}

#[test]
fn flash_thresholds() {
    let fast = run(30.0, 2.0, 15.0, [1.0; 3], None);
    assert!(fast.general_frames > 0 && fast.verdict().is_some(), "15 Hz full-frame flicker fails");
    let slow = run(30.0, 3.0, 1.0, [1.0; 3], None);
    assert_eq!(slow.general_frames, 0, "1 Hz passes");
    let small = run(30.0, 2.0, 15.0, [1.0; 3], Some(20));
    assert_eq!(small.general_frames, 0, "a 20-cell stripe covers at most 16 of a 10° field's 144 cells, under 25 %");
    let band = run(30.0, 2.0, 15.0, [1.0; 3], Some(COLS * 3));
    assert!(band.general_frames > 0, "three full rows cover 48 of 144 cells, over 25 %");
    let red = run(30.0, 2.0, 10.0, [0.9, 0.02, 0.02], None);
    assert!(red.red_frames > 0, "saturated red flicker fails the red test");
    // a dim flicker below the 10 % luminance swing is harmless
    let dim = run(30.0, 2.0, 15.0, [0.05; 3], None);
    assert_eq!(dim.general_frames, 0);
}

fn fixtures() -> Option<std::path::PathBuf> {
    let ok = std::process::Command::new(sr_media::ffmpeg())
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("skipping: FFmpeg missing");
        return None;
    }
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("sr-access-{}-{id}", std::process::id()));
    std::fs::create_dir_all(d.join("out")).unwrap();
    Some(d)
}

fn doc(dir: &std::path::Path, access: &str, body: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.1"><project width="64" height="64" fps="30" duration="2" background="#000000"/><metadata>{access}</metadata><output id="o" path="out/a.mp4" codec="h264" preset="ultrafast" audio="false"/><composition>{body}</composition></scene>"##
    );
    let path = dir.join("access.scene.xml");
    std::fs::write(&path, xml).unwrap();
    sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"))
}

fn flicker() -> String {
    let keys: String = (0..=60)
        .map(|k| format!(r#"<key time="{}" value="{}" interpolation="hold"/>"#, k as f64 / 30.0, k % 2))
        .collect();
    format!(
        r##"<shape id="flash" shape="rect" width="64" height="64" fill="#FFFFFF"><animate property="opacity">{keys}</animate></shape>"##
    )
}

fn deliver(d: &sr_model::Document) -> Result<sr_deliver::pipeline::Report, sr_deliver::DeliverError> {
    let gpu = gpu();
    let opts = sr_deliver::Options { hardware: sr_media::encode::Hardware::Software, ..Default::default() };
    sr_deliver::deliver(d, &d.scene.outputs[0], gpu.as_ref(), &opts, &mut |_, _| {})
}

#[test]
fn segment_flash_checks_use_output_time() {
    let Some(dir) = fixtures() else { return };
    if gpu().is_none() {
        return;
    }
    for (name, speed, step, should_fail) in [("fast", 4.0, 0.2, true), ("slow", 0.5, 0.1, false)] {
        let keys: String = (0..=(2.0 / step) as usize)
            .map(|k| format!(r#"<key time="{}" value="{}" interpolation="hold"/>"#, k as f64 * step, k % 2))
            .collect();
        let xml = format!(
            r##"<scene version="1.2"><project width="64" height="64" fps="25" duration="2" background="#000000"/>
          <metadata><accessibility flashCheck="error"/></metadata>
          <output path="out/{name}.mp4" codec="h264" preset="ultrafast" audio="false"><segment from="0" to="2" speed="{speed}"/></output>
          <composition><shape id="flash" shape="rect" width="64" height="64" fill="#FFFFFF"><animate property="opacity">{keys}</animate></shape></composition></scene>"##
        );
        let path = dir.join(format!("{name}.scene.xml"));
        std::fs::write(&path, xml).unwrap();
        let d = sr_model::load_file(path, &Default::default()).unwrap();
        let result = deliver(&d);
        if should_fail {
            assert!(
                matches!(result, Err(sr_deliver::DeliverError::Accessibility(ref m)) if m.contains("flashCheck")),
                "{name}: {result:?}"
            );
        } else {
            assert!(result.unwrap().accessibility.is_empty(), "slowing 5 Hz to 2.5 Hz must pass");
        }
    }
}

#[test]
fn flash_checks_include_overlays_and_ignore_covered_flashes() {
    let Some(dir) = fixtures() else { return };
    if gpu().is_none() {
        return;
    }
    let flash = flicker();
    let cover = r##"<shape id="cover" shape="rect" width="64" height="64" fill="#000000"/>"##;
    for (body, over, should_fail) in [("", flash.as_str(), true), (flash.as_str(), cover, false)] {
        let xml = format!(
            r##"<scene version="1.2"><project width="64" height="64" fps="30" duration="2" background="#000000"/>
              <metadata><accessibility flashCheck="error" contrastCheck="off"/></metadata>
              <output path="out/overlay.mkv" codec="ffv1" audio="false" overlay="tag"/>
              <symbols><symbol id="tag">{over}</symbol></symbols><composition>{body}</composition></scene>"##
        );
        let path = dir.join("overlay.scene.xml");
        std::fs::write(&path, xml).unwrap();
        let doc = sr_model::load_file(path, &Default::default()).unwrap();
        let result = deliver(&doc);
        if should_fail {
            assert!(
                matches!(result, Err(sr_deliver::DeliverError::Accessibility(ref m)) if m.contains("flashCheck")),
                "flashing overlay: {result:?}"
            );
        } else {
            assert!(result.unwrap().accessibility.is_empty(), "opaque overlay hides the composition's flashes");
        }
    }
}

#[test]
fn delivery_runs_the_checks() {
    let Some(dir) = fixtures() else { return };
    if gpu().is_none() {
        return;
    }
    // flashing fails with flashCheck="error" and is reported with "warn"
    let err = deliver(&doc(&dir, r#"<accessibility flashCheck="error"/>"#, &flicker())).unwrap_err();
    assert!(matches!(err, sr_deliver::DeliverError::Accessibility(ref m) if m.contains("flashCheck")), "{err}");
    let r = deliver(&doc(&dir, r#"<accessibility flashCheck="warn"/>"#, &flicker())).unwrap();
    assert!(r.accessibility.iter().any(|m| m.contains("flashCheck")), "{:?}", r.accessibility);
    // contrast: white on black passes, dark grey on black fails
    let text = r##"<layer id="t" asset="label" x="4" y="16"/>"##;
    let assets = |c: &str| {
        format!(
            r##"<assets><text id="label" text="AB" width="56" height="32" size="28" color="{c}" font="DejaVu Sans"/></assets>"##
        )
    };
    let with = |c: &str| {
        let xml = format!(
            r##"<scene version="1.1"><project width="64" height="64" fps="30" duration="0.2" background="#000000"/><metadata><accessibility contrastCheck="error" flashCheck="off"/></metadata><output id="o" path="out/c.mp4" codec="h264" preset="ultrafast" audio="false"/>{}<composition>{}</composition></scene>"##,
            assets(c),
            text
        );
        let path = dir.join("contrast.scene.xml");
        std::fs::write(&path, xml).unwrap();
        sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"))
    };
    let ok = deliver(&with("#FFFFFF")).unwrap();
    assert!(ok.accessibility.is_empty(), "{:?}", ok.accessibility);
    let low = deliver(&with("#303030")).unwrap_err();
    assert!(
        matches!(low, sr_deliver::DeliverError::Accessibility(ref m) if m.contains("contrastCheck") && m.contains('t')),
        "{low}"
    );
    // required captions
    let r = deliver(&doc(&dir, r#"<accessibility flashCheck="off" requireCaptions="true"/>"#, ""));
    assert!(matches!(r, Err(sr_deliver::DeliverError::Accessibility(ref m)) if m.contains("requireCaptions")));
}

#[test]
fn flash_check_needs_an_accessibility_element() {
    // as the XSD says: attribute defaults apply to a
    // declared <accessibility>; without one nothing is checked
    let Some(dir) = fixtures() else { return };
    if gpu().is_none() {
        return;
    }
    let r = deliver(&doc(&dir, "", &flicker())).unwrap();
    assert!(r.accessibility.is_empty(), "no <accessibility>: {:?}", r.accessibility);
    let r = deliver(&doc(&dir, "<accessibility/>", &flicker())).unwrap();
    assert!(r.accessibility.iter().any(|m| m.contains("flashCheck")), "declared: {:?}", r.accessibility);
}

#[test]
fn contrast_is_measured_inside_isolated_groups() {
    let Some(dir) = fixtures() else { return };
    if gpu().is_none() {
        return;
    }
    // text drawn into an offscreen (isolated group) is measured against what the viewer sees:
    // the frame rendered with and without the layer
    let with = |c: &str, group: &str| {
        let xml = format!(
            r##"<scene version="1.1"><project width="64" height="64" fps="30" duration="0.2" background="#000000"/><metadata><accessibility contrastCheck="error" flashCheck="off"/></metadata><output id="o" path="out/g.mp4" codec="h264" preset="ultrafast" audio="false"/><assets><text id="label" text="AB" width="56" height="32" size="28" color="{c}" font="DejaVu Sans"/></assets><composition><group id="g" {group}><layer id="t" asset="label" x="4" y="16"/></group></composition></scene>"##
        );
        let path = dir.join("isolated.scene.xml");
        std::fs::write(&path, xml).unwrap();
        sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"))
    };
    let ok = deliver(&with("#FFFFFF", r#"isolate="true""#)).unwrap();
    assert!(ok.accessibility.is_empty(), "{:?}", ok.accessibility);
    let same = deliver(&with("#000000", r#"isolate="true""#)).unwrap_err();
    assert!(matches!(same, sr_deliver::DeliverError::Accessibility(ref m) if m.contains("1.00")), "{same}");
    let low = deliver(&with("#303030", r#"isolate="true""#)).unwrap_err();
    assert!(
        matches!(low, sr_deliver::DeliverError::Accessibility(ref m) if m.contains("contrastCheck") && m.contains(" t ")),
        "{low}"
    );
    // a group fading the text is judged at its delivered, faded contrast
    let faded = deliver(&with("#FFFFFF", r#"isolate="true" opacity="0.12""#)).unwrap_err();
    assert!(matches!(faded, sr_deliver::DeliverError::Accessibility(ref m) if m.contains("contrastCheck")), "{faded}");
}

#[test]
fn identical_text_and_background_fail_only_when_text_has_visible_coverage() {
    if gpu().is_none() {
        return;
    }
    for (attrs, mask, cover, text, fails) in [
        ("", "", "", "AB", true),
        ("opacity=\"0\"", "", "", "AB", false),
        ("x=\"1000\"", "", "", "AB", false),
        ("", r#"<mask type="rect" x="1000" y="0" width="56" height="28"/>"#, "", "AB", false),
        ("", "", r##"<shape id="cover" shape="rect" width="64" height="36" fill="#000000"/>"##, "AB", false),
        ("", "", "", " ", false),
    ] {
        let dir = fixtures().unwrap();
        let xml = format!(
            r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="0.2" background="#000000"/>
          <metadata><accessibility contrastCheck="error" flashCheck="off"/></metadata>
          <output path="out/same.mkv" codec="ffv1" audio="false"/>
          <assets><text id="label" text="{text}" width="56" height="28" size="24" color="#000000" font="DejaVu Sans"/></assets>
          <composition><layer id="same" asset="label" y="4" {attrs}>{mask}</layer>{cover}</composition></scene>"##
        );
        let path = dir.join("same.xml");
        std::fs::write(&path, xml).unwrap();
        let doc = sr_model::load_file(path, &Default::default()).unwrap();
        let result = deliver(&doc);
        if fails {
            assert!(
                matches!(result, Err(sr_deliver::DeliverError::Accessibility(ref m)) if m.contains("contrastCheck") && m.contains("1.00")),
                "{result:?}"
            );
        } else {
            assert!(result.unwrap().accessibility.is_empty(), "attrs={attrs}, text={text:?}");
        }
    }
}

#[test]
fn text_fading_in_is_judged_at_rest() {
    let Some(dir) = fixtures() else { return };
    if gpu().is_none() {
        return;
    }
    // A label fading in passes through every ratio down to 1:1 on its way to rest; the check judges
    // it at its most visible, so white text on black passes while it fades in, in a group or not.
    for group in ["", r#"isolate="true""#] {
        let xml = format!(
            r##"<scene version="1.1"><project width="64" height="64" fps="30" duration="0.4" background="#000000"/><metadata><accessibility contrastCheck="error" flashCheck="off"/></metadata><output id="o" path="out/f.mp4" codec="h264" preset="ultrafast" audio="false"/><assets><text id="label" text="AB" width="56" height="32" size="28" color="#FFFFFF" font="DejaVu Sans"/></assets><composition><group id="g" {group}><layer id="t" asset="label" x="4" y="16"><animate property="opacity"><key time="0" value="0"/><key time="0.3" value="1"/></animate></layer></group></composition></scene>"##
        );
        let path = dir.join("fade.scene.xml");
        std::fs::write(&path, xml).unwrap();
        let doc = sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
        let r = deliver(&doc).unwrap_or_else(|e| panic!("group {group:?}: {e}"));
        assert!(r.accessibility.is_empty(), "group {group:?}: {:?}", r.accessibility);
    }
}

fn caption_requirement(
    workers: u32,
    output_captions: bool,
) -> Result<sr_deliver::pipeline::Report, sr_deliver::DeliverError> {
    let dir = fixtures().unwrap();
    let track = if output_captions {
        r#"<captionTrack id="cc" language="en"><cue start="0" end="1" text="Hello world"/></captionTrack>"#
    } else {
        ""
    };
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="36" fps="2" duration="1" background="#000000"/>
      <metadata><accessibility requireCaptions="true" flashCheck="off" contrastCheck="off"/></metadata>
      <output path="out/captions.mkv" codec="ffv1" audio="false">{track}</output><composition/></scene>"##
    );
    let path = dir.join("captions.xml");
    std::fs::write(&path, xml).unwrap();
    let doc = sr_model::load_file(path, &Default::default()).unwrap();
    let opts = sr_deliver::Options {
        parallel: sr_deliver::Parallel::Count(workers),
        hardware: sr_media::encode::Hardware::Software,
        ..Default::default()
    };
    sr_deliver::deliver(&doc, &doc.scene.outputs[0], gpu().as_ref(), &opts, &mut |_, _| {})
}

#[test]
fn output_caption_tracks_satisfy_the_caption_requirement() {
    if gpu().is_none() {
        return;
    }
    for workers in [1, 2] {
        let report = caption_requirement(workers, true).unwrap();
        assert!(report.accessibility.is_empty());
        assert_eq!(report.segments, expected_segments(workers));
    }
}

#[test]
fn parallel_delivery_rejects_missing_required_captions() {
    if gpu().is_none() {
        return;
    }
    for workers in [1, 2] {
        let result = caption_requirement(workers, false);
        assert!(
            matches!(result, Err(sr_deliver::DeliverError::Accessibility(ref m)) if m.contains("requireCaptions")),
            "workers={workers}: {result:?}"
        );
    }
}

#[test]
fn overlay_contrast_uses_the_delivered_backdrop() {
    if gpu().is_none() {
        return;
    }
    // Transparent overlays must use the composition's background, including when
    // text is inside an isolated group or hidden behind an opaque overlay shape.
    for (color, background, group, covered, fails) in [
        ("#000000", "#000000", false, false, true),
        ("#FFFFFF", "#FFFFFF", true, false, true),
        ("#000000", "#000000", true, true, false),
        ("#303030", "#000000", false, false, true),
        ("#FFFFFF", "#000000", false, false, false),
        ("#CCCCCC", "#FFFFFF", true, false, true),
        ("#000000", "#FFFFFF", true, false, false),
        ("#303030", "#000000", false, true, false),
    ] {
        let dir = fixtures().unwrap();
        let text = r#"<layer id="low" asset="label" x="4" y="8"/>"#;
        let body = if group { format!(r#"<group id="g" isolate="true">{text}</group>"#) } else { text.into() };
        let cover =
            if covered { r##"<shape id="cover" shape="rect" width="64" height="36" fill="#000000"/>"## } else { "" };
        let xml = format!(
            r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="0.2" background="{background}"/>
          <metadata><accessibility contrastCheck="error" flashCheck="off"/></metadata>
          <output path="out/contrast.mkv" codec="ffv1" audio="false" overlay="tag"/>
          <assets><text id="label" text="AB" width="56" height="28" size="24" color="{color}" font="DejaVu Sans"/></assets>
          <symbols><symbol id="tag">{body}{cover}</symbol></symbols><composition/></scene>"##
        );
        let path = dir.join("contrast.xml");
        std::fs::write(&path, xml).unwrap();
        let doc = sr_model::load_file(path, &Default::default()).unwrap();
        let result = deliver(&doc);
        if fails {
            assert!(
                matches!(result, Err(sr_deliver::DeliverError::Accessibility(ref m)) if m.contains("contrastCheck") && m.contains("low")),
                "{color} on {background}: {result:?}"
            );
        } else {
            assert!(result.unwrap().accessibility.is_empty());
        }
    }
}

#[test]
fn caption_contrast_is_checked_over_the_picture() {
    if gpu().is_none() {
        return;
    }
    for (color, composition, fails) in [
        ("#000000", false, true),
        ("#303030", false, true),
        ("#FFFFFF", false, false),
        ("#000000", true, true),
        ("#303030", true, true),
        ("#FFFFFF", true, false),
    ] {
        let dir = fixtures().unwrap();
        let track = r#"<captionTrack id="cc" language="en" mode="burn" style="caption-style" x="32" y="2" width="56"><cue start="0" end="0.2" text="AB"/></captionTrack>"#;
        let (output_captions, captions) =
            if composition { ("", format!("<captions>{track}</captions>")) } else { (track, String::new()) };
        let xml = format!(
            r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="0.2" background="#000000"/>
          <metadata><accessibility contrastCheck="error" flashCheck="off"/></metadata>
          <styles><textStyle id="caption-style" size="18" color="{color}"/></styles>
          <output path="out/caption-contrast.mkv" codec="ffv1" audio="false">{output_captions}</output>
          <composition/>{captions}</scene>"##
        );
        let path = dir.join("caption-contrast.xml");
        std::fs::write(&path, xml).unwrap();
        let doc = sr_model::load_file(path, &Default::default()).unwrap();
        let result = deliver(&doc);
        if fails {
            assert!(
                matches!(result, Err(sr_deliver::DeliverError::Accessibility(ref m)) if m.contains("contrastCheck") && m.contains("captions")),
                "{result:?}"
            );
        } else {
            assert!(result.unwrap().accessibility.is_empty());
        }
    }
}

fn regression_contrast_case(id: &str, cover: &str) -> Result<sr_deliver::pipeline::Report, sr_deliver::DeliverError> {
    let dir = fixtures().unwrap();
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="0.2" background="#000000"/>
      <metadata><accessibility contrastCheck="error" flashCheck="off"/></metadata>
      <output path="out/regression.mkv" codec="ffv1" audio="false"/>
      <assets><text id="label" text="AB" width="56" height="28" size="24" color="#000000" font="DejaVu Sans"/></assets>
      <composition><layer id="{id}" asset="label" y="4"/>{cover}</composition></scene>"##
    );
    let path = dir.join("regression.xml");
    std::fs::write(&path, xml).unwrap();
    let doc = sr_model::load_file(path, &Default::default()).unwrap();
    deliver(&doc)
}

#[test]
fn regression_captions_id_does_not_bypass_contrast() {
    if gpu().is_none() {
        return;
    }
    let result = regression_contrast_case("captions", "");
    assert!(matches!(result, Err(sr_deliver::DeliverError::Accessibility(ref m)) if m.contains("1.00")), "{result:?}");
}

#[test]
fn regression_occluded_text_has_no_contrast_requirement() {
    if gpu().is_none() {
        return;
    }
    let result = regression_contrast_case(
        "label-layer",
        r##"<shape id="cover" shape="rect" width="64" height="36" fill="#303030"/>"##,
    );
    assert!(result.unwrap().accessibility.is_empty());
}

#[test]
fn regression_draft_contrast_probes_use_rendered_coordinates() {
    let Some(gpu) = gpu() else { return };
    let dir = fixtures().unwrap();
    for quality in [sr_model::model::ProjectQuality::Final, sr_model::model::ProjectQuality::Draft] {
        for color in ["#000000", "#FFFFFF"] {
            let xml = format!(
                r##"<scene version="1.2"><project width="640" height="360" fps="10" duration="0.2" background="#000000"/>
              <metadata><accessibility contrastCheck="error" flashCheck="off"/></metadata><output path="out/draft-probe.mkv" codec="ffv1" audio="false"/>
              <assets><text id="txt" text="AB" width="56" height="28" size="24" color="{color}" font="DejaVu Sans"/></assets>
              <composition><layer id="label" asset="txt" x="200" y="40"/><shape id="unrelated" shape="rect" width="1" height="1" fill="#FFFFFF"/></composition></scene>"##
            );
            let path = dir.join("draft-probe.xml");
            std::fs::write(&path, xml).unwrap();
            let doc = sr_model::load_file(path, &Default::default()).unwrap();
            let opts = sr_deliver::Options {
                quality: Some(quality),
                hardware: sr_media::encode::Hardware::Software,
                ..Default::default()
            };
            let result = sr_deliver::deliver(&doc, &doc.scene.outputs[0], Some(&gpu), &opts, &mut |_, _| {});
            if color == "#000000" {
                assert!(
                    matches!(result, Err(sr_deliver::DeliverError::Accessibility(ref m)) if m.contains("1.00")),
                    "{quality:?}: {result:?}"
                );
            } else {
                assert!(result.unwrap().accessibility.is_empty());
            }
        }
    }
}

#[test]
fn regression_contrast_checks_all_times_even_with_a_later_layer() {
    if gpu().is_none() {
        return;
    }
    let dir = fixtures().unwrap();
    for later in [false, true] {
        for low_at_end in [false, true] {
            let (before, after, at) =
                if low_at_end { ("#FFFFFF", "#000000", "0.9") } else { ("#000000", "#FFFFFF", "0.1") };
            let extra = if later {
                r##"<shape id="unrelated" shape="rect" width="1" height="1" fill="#FFFFFF"/>"##
            } else {
                ""
            };
            let xml = format!(
                r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="1" background="#000000"/>
              <metadata><accessibility contrastCheck="error" flashCheck="off"/></metadata><output path="out/temporal-probe.mkv" codec="ffv1" audio="false"/>
              <assets><text id="txt" text="AB" width="56" height="28" size="24" color="#000000" font="DejaVu Sans"/></assets>
              <composition><shape id="backdrop" shape="rect" width="64" height="36" fill="{before}"><animate property="fill">
              <key time="0" value="{before}" interpolation="hold"/><key time="{at}" value="{after}"/></animate></shape>
              <layer id="label" asset="txt" x="4" y="4"/>{extra}</composition></scene>"##
            );
            let path = dir.join("temporal-probe.xml");
            std::fs::write(&path, xml).unwrap();
            let doc = sr_model::load_file(path, &Default::default()).unwrap();
            let result = deliver(&doc);
            assert!(
                matches!(result, Err(sr_deliver::DeliverError::Accessibility(ref m)) if m.contains("1.00")),
                "later={later}, low_at_end={low_at_end}: {result:?}"
            );
        }
    }
}

/// The segments a delivery of `workers` workers makes: one when the GPU debug layers are on, which serialise the workers.
fn expected_segments(workers: u32) -> u32 {
    if sr_gpu::gpu::debug_layers() {
        1
    } else {
        workers
    }
}

/// Delivers a whole document as `workers` time segments at once.
fn deliver_in(
    dir: &std::path::Path,
    name: &str,
    xml: &str,
    workers: u32,
) -> Result<sr_deliver::pipeline::Report, sr_deliver::DeliverError> {
    let path = dir.join(name);
    std::fs::write(&path, xml).unwrap();
    let doc = sr_model::load_file(path, &Default::default()).unwrap();
    let opts = sr_deliver::Options {
        parallel: sr_deliver::Parallel::Count(workers),
        hardware: sr_media::encode::Hardware::Software,
        ..Default::default()
    };
    sr_deliver::deliver(&doc, &doc.scene.outputs[0], gpu().as_ref(), &opts, &mut |_, _| {})
}

#[test]
fn parallel_delivery_counts_flashes_across_its_segments() {
    let Some(dir) = fixtures() else { return };
    if gpu().is_none() {
        return;
    }
    // 270 frames in three bounded segments of 90, reusing the first worker. The frame turns white and black ten times around the join, four
    // times before it and six after: neither side alone reaches the eight transitions in a second that
    // make a flash, so the check must follow the frames across the join.
    let keys: String = (86..=95)
        .map(|k| format!(r#"<key time="{}" value="{}" interpolation="hold"/>"#, (k as f64 - 0.5) / 30.0, k % 2))
        .collect();
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="36" fps="30" duration="9" background="#000000"/>
      <metadata><accessibility flashCheck="warn" contrastCheck="off"/></metadata>
      <output path="out/join-flash.mkv" codec="ffv1" audio="false"/>
      <composition><shape id="flash" shape="rect" width="64" height="36" fill="#FFFFFF"><animate property="opacity">
      <key time="0" value="0" interpolation="hold"/>{keys}</animate></shape></composition></scene>"##
    );
    let mut findings = Vec::new();
    for workers in [1, 2] {
        let r = deliver_in(&dir, "join-flash.xml", &xml, workers).unwrap();
        assert_eq!(r.segments, expected_segments(workers), "the flash check must not force a serial render");
        assert!(r.accessibility.iter().any(|m| m.contains("flashCheck")), "workers={workers}: {:?}", r.accessibility);
        findings.push(r.accessibility);
    }
    assert_eq!(findings[0], findings[1], "the parallel render reports other flashes than the serial one");
}

#[test]
fn parallel_delivery_judges_text_at_its_most_visible_over_the_whole_output() {
    let Some(dir) = fixtures() else { return };
    if gpu().is_none() {
        return;
    }
    // White text on black fades in across the join of two segments: in the first it only reaches a
    // ninth of its opacity, which alone would fail, and it rests in the second. Outside a group the
    // inline probe measures it; inside an isolated one, the renders with and without it.
    for group in ["", r#"isolate="true""#] {
        let xml = format!(
            r##"<scene version="1.2"><project width="64" height="36" fps="30" duration="2" background="#000000"/>
          <metadata><accessibility contrastCheck="error" flashCheck="off"/></metadata>
          <output path="out/join-fade.mkv" codec="ffv1" audio="false"/>
          <assets><text id="label" text="AB" width="56" height="28" size="24" color="#FFFFFF" font="DejaVu Sans"/></assets>
          <composition><group id="g" {group}><layer id="t" asset="label" x="4" y="4"><animate property="opacity">
          <key time="0" value="0"/><key time="0.9" value="0"/><key time="1.5" value="1"/></animate></layer></group></composition></scene>"##
        );
        for workers in [1, 2] {
            let r = deliver_in(&dir, "join-fade.xml", &xml, workers)
                .unwrap_or_else(|e| panic!("group {group:?}, workers={workers}: {e}"));
            assert_eq!(r.segments, expected_segments(workers), "the contrast check must not force a serial render");
            assert!(r.accessibility.is_empty(), "group {group:?}, workers={workers}: {:?}", r.accessibility);
        }
    }
}

#[test]
fn parallel_delivery_reports_the_contrast_findings_of_serial_delivery() {
    let Some(dir) = fixtures() else { return };
    if gpu().is_none() {
        return;
    }
    // Dark grey text over a backdrop that turns from black to dark grey in the second segment: the
    // lowest ratio, and the time it is reported at, lie after the join.
    for group in ["", r#"isolate="true""#] {
        let xml = format!(
            r##"<scene version="1.2"><project width="64" height="36" fps="30" duration="2" background="#000000"/>
          <metadata><accessibility contrastCheck="warn" flashCheck="warn"/></metadata>
          <output path="out/join-contrast.mkv" codec="ffv1" audio="false"/>
          <assets><text id="label" text="AB" width="56" height="28" size="24" color="#606060" font="DejaVu Sans"/></assets>
          <composition><shape id="backdrop" shape="rect" width="64" height="36" fill="#000000"><animate property="fill">
          <key time="0" value="#000000" interpolation="hold"/><key time="1.4" value="#303030"/></animate></shape>
          <group id="g" {group}><layer id="t" asset="label" x="4" y="4"/></group></composition></scene>"##
        );
        let mut findings = Vec::new();
        for workers in [1, 2] {
            let r = deliver_in(&dir, "join-contrast.xml", &xml, workers).unwrap();
            assert_eq!(r.segments, expected_segments(workers));
            assert!(
                r.accessibility.iter().any(|m| m.contains("contrastCheck") && m.contains(" t ")),
                "group {group:?}, workers={workers}: {:?}",
                r.accessibility
            );
            findings.push(r.accessibility);
        }
        assert_eq!(findings[0], findings[1], "group {group:?}: the parallel render reports other findings");
    }
}
