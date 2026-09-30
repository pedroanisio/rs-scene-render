//! Accessibility: flash analysis, text contrast and required captions.

use sr_deliver::access::{FlashDetector, COLS, ROWS};

// As in the segment tests, share one device: concurrent device creation can crash
// the software Vulkan driver. Each test still owns its renderers and outputs.
fn gpu() -> Option<sr_gpu::Gpu> {
    static G: std::sync::OnceLock<Option<sr_gpu::Gpu>> = std::sync::OnceLock::new();
    G.get_or_init(|| sr_gpu::Gpu::new().map_err(|e| eprintln!("skipping: {e}")).ok()).clone()
}

fn frame(v: [f64; 3], cells: Option<usize>) -> Vec<[f64; 3]> {
    (0..COLS * ROWS).map(|k| if cells.map(|n| k < n).unwrap_or(true) { v } else { [0.0; 3] }).collect()
}

fn run(fps: f64, secs: f64, hz: f64, on: [f64; 3], cells: Option<usize>) -> FlashDetector {
    let mut d = FlashDetector::default();
    let n = (fps * secs) as usize;
    for k in 0..n {
        let t = k as f64 / fps;
        let lit = ((t * hz * 2.0).floor() as u64) % 2 == 0;
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
