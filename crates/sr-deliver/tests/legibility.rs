//! SREP 19: legibility checks for video text and captions, measured as the SREP defines them. The conformance cases
//! `srep-0019-speed`, `-short` and `-size` on the measuring functions; the delivery reports them through SREP 18.

use sr_deliver::legibility::{
    caption_findings, characters, resolve, text_findings, Settings, Shown, ShownTrack, DEFAULT_MIN_DISPLAY_TIME,
};
use sr_model::values::{Length, LengthUnit};
use sr_model::Severity;

fn settings(reading_speed: Option<f64>, min_text_size: Option<Length>) -> Settings {
    Settings { severity: Severity::Warning, reading_speed, min_display_time: DEFAULT_MIN_DISPLAY_TIME, min_text_size }
}

fn track(reading_speed: Option<f64>, shown: Vec<(f64, f64, &str)>) -> ShownTrack {
    ShownTrack {
        id: "cc".into(),
        reading_speed,
        shown: shown.into_iter().map(|(start, end, t)| Shown { start, end, text: t.into() }).collect(),
    }
}

#[test]
fn characters_are_nfc_code_points_without_line_breaks() {
    assert_eq!(characters("abc def"), 7, "spaces count");
    assert_eq!(characters("ab\ncd\r\nef"), 6, "line breaks do not");
    // e + combining acute composes to one code point under NFC
    assert_eq!(characters("e\u{301}"), 1);
    assert_eq!(characters("\u{e9}"), 1);
}

#[test]
fn lengths_resolve_against_the_output_frame() {
    let size = [1920.0, 1080.0];
    let l = |value, unit| Length { value, unit };
    assert!((resolve(&l(3.0, LengthUnit::Vh), size) - 32.4).abs() < 1e-9);
    assert!((resolve(&l(1.0, LengthUnit::Vw), size) - 19.2).abs() < 1e-9);
    assert!((resolve(&l(2.0, LengthUnit::Vmin), size) - 21.6).abs() < 1e-9);
    assert!((resolve(&l(2.0, LengthUnit::Vmax), size) - 38.4).abs() < 1e-9);
    assert_eq!(resolve(&l(24.0, LengthUnit::Px), size), 24.0);
}

#[test]
fn srep_0019_speed() {
    // a caption track with readingSpeed 20 and one cue of 30 characters over 1 s
    let cue = "abcdefghij klmnopqrs tuvwxyz12";
    assert_eq!(characters(cue), 30);
    let f = caption_findings(&settings(None, None), &[track(Some(20.0), vec![(2.0, 3.0, cue)])], [0.0, 10.0]);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!((f[0].code.as_str(), f[0].severity), ("LEG-SPEED", Severity::Warning));
    assert!((f[0].measured.unwrap() - 30.0).abs() <= 0.1);
    assert_eq!((f[0].limit, f[0].unit.as_deref()), (Some(20.0), Some("cps")));
    assert_eq!(f[0].time, Some([2.0, 3.0]));
    // the track's speed overrides the accessibility element's; without either there is no speed check
    assert!(caption_findings(&settings(Some(10.0), None), &[track(Some(40.0), vec![(2.0, 3.0, cue)])], [0.0, 10.0])
        .is_empty());
    assert_eq!(
        caption_findings(&settings(Some(10.0), None), &[track(None, vec![(2.0, 3.0, cue)])], [0.0, 10.0]).len(),
        1
    );
    assert!(caption_findings(&settings(None, None), &[track(None, vec![(2.0, 3.0, cue)])], [0.0, 10.0]).is_empty());
}

#[test]
fn srep_0019_short() {
    let f = caption_findings(&settings(None, None), &[track(None, vec![(1.0, 1.5, "Hi")])], [0.0, 10.0]);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].code, "LEG-SHORT");
    assert!((f[0].measured.unwrap() - 0.5).abs() < 1e-9);
    assert_eq!(f[0].limit, Some(DEFAULT_MIN_DISPLAY_TIME));
    // the part of a cue the output shows is what counts
    let cut = caption_findings(&settings(None, None), &[track(None, vec![(0.0, 2.0, "Hi")])], [1.5, 10.0]);
    assert_eq!(cut.len(), 1);
    assert_eq!(cut[0].time, Some([1.5, 2.0]));
    assert!(caption_findings(&settings(None, None), &[track(None, vec![(0.0, 2.0, "Hi")])], [0.0, 10.0]).is_empty());
}

fn evaluator(xml: &str) -> sr_eval::Evaluator {
    let d = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
    sr_eval::Evaluator::new(&d, &Default::default()).unwrap()
}

fn frames(fps: f64, n: usize) -> Vec<f64> {
    (0..n).map(|k| k as f64 / fps).collect()
}

#[test]
fn srep_0019_size() {
    // a 1080-pixel-high output, minTextSize 3vh, a text layer of size 24 at scale 1: measured 24, limit 32.4
    let ev = evaluator(
        r##"<scene version="1.2"><project width="1920" height="1080" fps="10" duration="2"/>
  <assets><text id="t" text="Small print" width="600" height="100" size="24" font="DejaVu Sans"/></assets>
  <composition><layer id="small" asset="t"/></composition></scene>"##,
    );
    let s = settings(None, Some(Length { value: 3.0, unit: LengthUnit::Vh }));
    let f = text_findings(&ev, &s, &frames(10.0, 20), 10.0, [1920.0, 1080.0], [1920.0, 1080.0]);
    let size: Vec<_> = f.iter().filter(|f| f.code == "LEG-SIZE").collect();
    assert_eq!(size.len(), 1, "one finding per layer: {f:?}");
    assert!((size[0].measured.unwrap() - 24.0).abs() < 1e-9, "{:?}", size[0]);
    assert!((size[0].limit.unwrap() - 32.4).abs() < 1e-9);
    assert_eq!(size[0].unit.as_deref(), Some("px"));
    assert_eq!(size[0].time, Some([0.0, 2.0]));
    // the layer's scale and the output's size count
    let ev2 = evaluator(
        r##"<scene version="1.2"><project width="1920" height="1080" fps="10" duration="2"/>
  <assets><text id="t" text="Small print" width="600" height="100" size="24" font="DejaVu Sans"/></assets>
  <composition><layer id="big" asset="t" scaleX="2" scaleY="2"/></composition></scene>"##,
    );
    assert!(text_findings(&ev2, &s, &frames(10.0, 20), 10.0, [1920.0, 1080.0], [1920.0, 1080.0])
        .iter()
        .all(|f| f.code != "LEG-SIZE"));
    let half = text_findings(&ev2, &s, &frames(10.0, 20), 10.0, [1920.0, 1080.0], [960.0, 540.0]);
    let h: Vec<_> = half.iter().filter(|f| f.code == "LEG-SIZE").collect();
    assert_eq!(h.len(), 1);
    assert!((h[0].measured.unwrap() - 24.0).abs() < 1e-9 && (h[0].limit.unwrap() - 16.2).abs() < 1e-9);
}

#[test]
fn text_layers_are_measured_over_their_visible_frames() {
    // 11 characters drawn for 0.5 s: 22 cps; shown for less than 5/6 s
    let ev = evaluator(
        r##"<scene version="1.2"><project width="640" height="360" fps="10" duration="2"/>
  <assets><text id="t" text="Hello world" width="600" height="100" size="40" font="DejaVu Sans"/></assets>
  <composition><layer id="flash" asset="t" start="0.5" end="1"/><layer id="hidden" asset="t" opacity="0"/></composition></scene>"##,
    );
    let f = text_findings(&ev, &settings(Some(17.0), None), &frames(10.0, 20), 10.0, [640.0, 360.0], [640.0, 360.0]);
    let codes: Vec<(&str, &str)> = f.iter().map(|f| (f.code.as_str(), f.message.as_str())).collect();
    let speed = f.iter().find(|x| x.code == "LEG-SPEED").unwrap_or_else(|| panic!("{codes:?}"));
    assert!((speed.measured.unwrap() - 22.0).abs() < 1e-9, "{speed:?}");
    assert_eq!(speed.time, Some([0.5, 1.0]));
    let short = f.iter().find(|x| x.code == "LEG-SHORT").unwrap();
    assert!((short.measured.unwrap() - 0.5).abs() < 1e-9);
    // a layer drawn at opacity 0 is never visible, so it is not measured
    assert!(f.iter().all(|x| !x.message.contains("hidden")), "{codes:?}");
}
