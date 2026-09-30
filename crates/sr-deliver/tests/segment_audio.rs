//! The audio of outputs with segments (SREP 13): pitch under `stretch` and `resample`, loudness
//! measured on the output's programme, source selection, and the output's own tracks in output time.

use std::path::{Path, PathBuf};

const RATE: u32 = 48000;

/// A directory with 4 s stereo tones at 440 Hz and 1000 Hz, amplitude 0.25.
fn fixtures(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr-segment-audio-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    for (file, f) in [("a440.wav", 440.0), ("a1000.wav", 1000.0)] {
        let tone: Vec<f32> = (0..4 * RATE)
            .map(|k| (0.25 * (2.0 * std::f64::consts::PI * f * k as f64 / RATE as f64).sin()) as f32)
            .collect();
        sr_audio::wav::write(&d.join(file), &vec![tone.clone(), tone], RATE, 32, sr_audio::Layout::Stereo, false)
            .unwrap();
    }
    d
}

/// Delivers the document's audio-only output and returns its left channel and the report.
fn deliver(dir: &Path, output: &str, mix: &str) -> (Vec<f32>, sr_deliver::pipeline::Report) {
    deliver_with_options(dir, output, mix, &Default::default())
}

fn deliver_with_options(
    dir: &Path,
    output: &str,
    mix: &str,
    opts: &sr_deliver::Options,
) -> (Vec<f32>, sr_deliver::pipeline::Report) {
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="36" fps="25" duration="4" background="#000000"/>
          {output}
          <assets><audio id="a440" src="a440.wav"/><audio id="a1000" src="a1000.wav"/></assets>
          <composition/>
          <audioMix>{mix}</audioMix></scene>"##
    );
    let path = dir.join("scene.xml");
    std::fs::write(&path, xml).unwrap();
    let d = sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let r = sr_deliver::deliver(&d, &d.scene.outputs[0], None, opts, &mut |_, _| {}).unwrap_or_else(|e| panic!("{e}"));
    let a = sr_media::decode_audio(&r.path, 0, RATE).unwrap();
    let ch = a.channels as usize;
    (a.samples.chunks(ch).map(|f| f[0]).collect(), r)
}

/// Frequency from rising zero crossings between `from` and `to` seconds.
fn frequency(x: &[f32], from: f64, to: f64) -> f64 {
    let (a, b) = ((from * RATE as f64) as usize, (to * RATE as f64) as usize);
    let ups: Vec<usize> = (a + 1..b).filter(|&k| x[k - 1] < 0.0 && x[k] >= 0.0).collect();
    (ups.len() - 1) as f64 / ((ups[ups.len() - 1] - ups[0]) as f64 / RATE as f64)
}

fn rms(x: &[f32], from: f64, to: f64) -> f64 {
    let s = &x[(from * RATE as f64) as usize..(to * RATE as f64) as usize];
    (s.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / s.len() as f64).sqrt()
}

#[test]
fn stretch_keeps_pitch_and_resample_follows_the_rate() {
    let d = fixtures("pitch");
    for (mode, want) in [("stretch", 440.0), ("resample", 880.0)] {
        let (x, _) = deliver(
            &d,
            &format!(
                r#"<output path="{mode}.wav" codec="audio-only"><segment from="0" to="2" speed="2" audio="{mode}"/></output>"#
            ),
            r#"<audioTrack id="t" asset="a440"/>"#,
        );
        // 2 s at speed 2 is exactly one second
        assert_eq!(x.len(), RATE as usize, "{mode}");
        let f = frequency(&x, 0.1, 0.9);
        assert!((f - want).abs() < 1.0, "{mode}: {f} Hz");
    }
}

#[test]
fn loudness_is_normalised_on_the_output_programme() {
    let d = fixtures("loud");
    let (_, r) = deliver(
        &d,
        r#"<output path="loud.wav" codec="audio-only"><segment from="0" to="1"/><segment from="2.5" to="3.5" speed="1.5"/></output>"#,
        r#"<audioTrack id="t" asset="a440" volume="0.3"/><master normalize="integrated" loudness="-14"/>"#,
    );
    let l = r.loudness.unwrap();
    assert!((l + 14.0).abs() < 0.5, "{l} LUFS");
}

#[test]
fn unselected_buses_still_trigger_output_ducking() {
    let d = fixtures("bus-duck");
    let mix = r#"<audioTrack id="voice" asset="a440" role="dialogue" bus="speech"/>
                 <bus id="speech" output="dialogue"/><bus id="dialogue"/>"#;
    let mut reference: Option<f64> = None;
    for key in ["voice", "speech", "dialogue"] {
        let (x, _) = deliver(
            &d,
            &format!(
                r#"<output path="duck.wav" codec="audio-only" audioRoles="effects">
              <segment from="1" to="3"/><audioTrack id="own" asset="a1000" duckUnder="{key}"
                duckAmount="-20" duckThreshold="-40" duckAttack="0.01"/>
              </output>"#
            ),
            mix,
        );
        let level = rms(&x, 0.5, 1.5);
        assert!((level - 0.01768).abs() < 0.001, "{key}: ducked RMS {level}");
        assert!((frequency(&x, 0.5, 1.5) - 1000.0).abs() < 1.0, "excluded voice must not become audible");
        if let Some(previous) = reference {
            assert!((level - previous).abs() < 1e-6, "unity buses preserve the key signal");
        }
        reference = Some(level);
    }
}

#[test]
fn bus_keys_keep_selected_and_output_owned_inputs() {
    let d = fixtures("bus-key-inputs");
    for (mix, own, direct) in [
        (
            r#"<audioTrack id="voice" asset="a440" role="dialogue" bus="speech"/>
             <audioTrack id="bed" asset="a1000" role="effects" bus="speech" volume="0.05"/>
             <bus id="speech"/>"#,
            "",
            "voice",
        ),
        (
            r#"<audioTrack id="voice" asset="a440" role="dialogue" volume="0" bus="speech"/><bus id="speech"/>"#,
            r#"<audioTrack id="narration" asset="a440" bus="speech"/>"#,
            "narration",
        ),
    ] {
        let render = |key: &str| {
            deliver(
                &d,
                &format!(
                    r#"<output path="key.wav" codec="audio-only" audioRoles="effects">
              <segment from="1" to="3"/>{own}<audioTrack id="music" asset="a1000" duckUnder="{key}"
                duckAmount="-20" duckThreshold="-40" duckAttack="0.01"/>
              </output>"#
                ),
                mix,
            )
            .0
        };
        let reference = render(direct);
        let via_bus = render("speech");
        assert!(
            reference.iter().zip(&via_bus).all(|(a, b)| (a - b).abs() < 1e-6),
            "bus key must retain its inputs without leaking or duplicating them in the master: {direct}"
        );
    }
}

#[test]
fn bus_keys_feed_effect_sidechains_and_apply_bus_gain() {
    let d = fixtures("bus-key-fx");
    let render = |key: &str, gain: i32| {
        deliver(
            &d,
            &format!(
                r#"<output path="key.wav" codec="audio-only" audioRoles="effects">
          <segment from="0" to="2"/><audioTrack id="music" asset="a1000">
            <audioEffect type="compressor" threshold="-30" ratio="10" attack="0.01" sidechain="{key}"/>
          </audioTrack></output>"#
            ),
            &format!(
                r#"<audioTrack id="voice" asset="a440" role="dialogue" bus="speech"/><bus id="speech" gain="{gain}"/>"#
            ),
        )
        .0
    };
    let reference = render("voice", 0);
    let via_bus = render("speech", 0);
    assert!(reference.iter().zip(&via_bus).all(|(a, b)| (a - b).abs() < 1e-6), "effect must hear the bus key");
    let quiet_key = render("speech", -60);
    assert!(rms(&quiet_key, 0.5, 1.5) > 2.0 * rms(&via_bus, 0.5, 1.5), "bus gain must affect the key");
}

#[test]
fn outputs_select_their_sources() {
    let d = fixtures("select");
    let mix = r#"<audioTrack id="music" asset="a440" role="music" bus="fx"/>
                 <audioTrack id="voice" asset="a1000" role="dialogue"/>
                 <bus id="fx"/>"#;
    for (attr, want) in
        [(r#"audioRoles="dialogue""#, 1000.0), (r#"audioTracks="music""#, 440.0), (r#"audioBuses="fx""#, 440.0)]
    {
        let (x, _) = deliver(
            &d,
            &format!(r#"<output path="sel.wav" codec="audio-only" {attr}><segment from="0" to="1"/></output>"#),
            mix,
        );
        let f = frequency(&x, 0.1, 0.9);
        assert!((f - want).abs() < 1.0, "{attr}: {f} Hz");
    }
}

#[test]
fn nonsegmented_audio_honours_command_line_range_overrides() {
    let d = fixtures("select-range");
    let opts = sr_deliver::Options { start: Some(0.0), end: Some(3.0), ..Default::default() };
    let (x, _) = deliver_with_options(
        &d,
        r#"<output path="range.wav" codec="audio-only" start="1" end="2" audioTracks="music"/>"#,
        r#"<audioTrack id="music" asset="a440"/>"#,
        &opts,
    );
    assert_eq!(x.len(), 3 * RATE as usize);
    for t in [0.1, 1.1, 2.1] {
        assert!((frequency(&x, t, t + 0.5) - 440.0).abs() < 1.0);
    }
}

#[test]
fn own_tracks_honour_command_line_range_overrides() {
    let d = fixtures("own-range");
    for (start, end) in [(0.0, 3.0), (1.5, 3.5)] {
        let opts = sr_deliver::Options { start: Some(start), end: Some(end), ..Default::default() };
        let (x, _) = deliver_with_options(
            &d,
            r#"<output path="range.wav" codec="audio-only" start="1" end="2">
                 <audioTrack id="own" asset="a1000" volume="0"/>
               </output>"#,
            r#"<audioTrack id="music" asset="a440"/>"#,
            &opts,
        );
        assert_eq!(x.len(), ((end - start) * RATE as f64) as usize);
        for t in [0.1, end - start - 0.6] {
            assert!(rms(&x, t, t + 0.5) > 0.1, "composition audio throughout overridden range");
            assert!((frequency(&x, t, t + 0.5) - 440.0).abs() < 1.0);
        }
    }
}

fn check_own_automation(property: &str, before: f64, after: f64, rises: bool) {
    let d = fixtures(&format!("own-{property}"));
    // The same automation is in output time with and without a composition time map.
    for (attrs, segments) in [(r#"start="1" end="3""#, ""), ("", r#"<segment from="2" to="4"/>"#)] {
        let (x, _) = deliver(
            &d,
            &format!(
                r#"<output path="auto.wav" codec="audio-only" {attrs}>{segments}
                  <audioTrack id="own" asset="a1000"><animate property="{property}">
                    <key time="0" value="{before}" interpolation="hold"/><key time="1" value="{after}"/>
                  </animate></audioTrack></output>"#
            ),
            r#"<audioTrack id="music" asset="a440" volume="0"/>"#,
        );
        let (early, late) = (rms(&x, 0.1, 0.8), rms(&x, 1.1, 1.8));
        let (quiet, loud) = if rises { (early, late) } else { (late, early) };
        assert!(quiet < 0.001 && loud > 0.1, "{property}: early RMS {early}, late RMS {late}");
    }
}

#[test]
fn own_tracks_animate_volume_in_output_time() {
    check_own_automation("volume", 0.0, 1.0, true);
}

#[test]
fn own_tracks_animate_gain_in_output_time() {
    check_own_automation("gain", -60.0, 0.0, true);
}

#[test]
fn own_tracks_animate_pan_in_output_time() {
    check_own_automation("pan", -1.0, 1.0, false);
}

#[test]
fn outputs_without_segments_keep_all_sources() {
    let d = fixtures("select-plain");
    let mix = r#"<audioTrack id="music" asset="a440" role="music" bus="fx"/>
                 <audioTrack id="voice" asset="a1000" role="dialogue"/>
                 <bus id="fx"/>"#;
    let (all, _) = deliver(&d, r#"<output path="all.wav" codec="audio-only" start="1" end="2"/>"#, mix);
    for attr in [r#"audioRoles="dialogue""#, r#"audioTracks="music""#, r#"audioBuses="fx""#, r#"audioRoles="effects""#]
    {
        let (x, _) =
            deliver(&d, &format!(r#"<output path="sel.wav" codec="audio-only" start="1" end="2" {attr}/>"#), mix);
        assert_eq!(x.len(), RATE as usize);
        assert!(
            x.iter().zip(&all).all(|(a, b)| (a - b).abs() < 1e-6),
            "{attr} must not filter an output without segments"
        );
    }
}

#[test]
fn an_outputs_own_tracks_play_in_output_time() {
    let d = fixtures("own");
    // nothing of the composition is selected; the output's own track starts at output time 0.5, while
    // the segment plays composition time 2‥3
    let (x, _) = deliver(
        &d,
        r#"<output path="own.wav" codec="audio-only" audioRoles="effects">
             <segment from="2" to="3"/>
             <audioTrack id="sting" asset="a1000" start="0.5"/>
           </output>"#,
        r#"<audioTrack id="music" asset="a440" role="music"/>"#,
    );
    assert!(rms(&x, 0.0, 0.49) < 1e-6);
    let f = frequency(&x, 0.6, 0.95);
    assert!((f - 1000.0).abs() < 2.0, "{f} Hz");
}

#[test]
fn joins_crossfade_without_a_click() {
    let d = fixtures("join");
    // the same tone cut out of phase: the join is a short equal-power crossfade, so no sample jumps by
    // more than a tone sample ever does
    let (x, _) = deliver(
        &d,
        r#"<output path="join.wav" codec="audio-only"><segment from="0" to="1"/><segment from="2.0011" to="3"/></output>"#,
        r#"<audioTrack id="t" asset="a440"/>"#,
    );
    let step = 0.25 * 2.0 * std::f64::consts::PI * 440.0 / RATE as f64;
    let worst = x.windows(2).map(|w| (w[1] - w[0]).abs() as f64).fold(0.0, f64::max);
    assert!(worst < step * 1.5, "largest step {worst}, a tone step is at most {step}");
}

#[test]
fn own_tracks_play_without_segments_from_the_outputs_start() {
    let d = fixtures("plain");
    // no scene audio at all; the output starts at composition 1 s and its track at output 0.5 s
    let xml = r##"<scene version="1.2"><project width="64" height="36" fps="25" duration="4" background="#000000"/>
          <output path="plain.wav" codec="audio-only" start="1"><audioTrack id="sting" asset="a1000" start="0.5"/></output>
          <assets><audio id="a1000" src="a1000.wav"/></assets>
          <composition/></scene>"##;
    let path = d.join("plain.xml");
    std::fs::write(&path, xml).unwrap();
    let doc = sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let r = sr_deliver::deliver(&doc, &doc.scene.outputs[0], None, &Default::default(), &mut |_, _| {}).unwrap();
    let a = sr_media::decode_audio(&r.path, 0, RATE).unwrap();
    let x: Vec<f32> = a.samples.chunks(a.channels as usize).map(|f| f[0]).collect();
    assert_eq!(x.len(), 3 * RATE as usize);
    assert!(rms(&x, 0.0, 0.49) < 1e-6);
    assert!((frequency(&x, 0.6, 2.9) - 1000.0).abs() < 1.0);
}

#[test]
fn selection_acts_only_with_segments() {
    let d = fixtures("noselect");
    // without segments, audioRoles has no effect: the composition's music plays under the output's own track
    let xml = r##"<scene version="1.2"><project width="64" height="36" fps="25" duration="4" background="#000000"/>
          <output path="all.wav" codec="audio-only" audioRoles="effects"><audioTrack id="sting" asset="a1000" start="2"/></output>
          <assets><audio id="a440" src="a440.wav"/><audio id="a1000" src="a1000.wav"/></assets>
          <composition/><audioMix><audioTrack id="music" asset="a440" role="music"/></audioMix></scene>"##;
    let path = d.join("all.xml");
    std::fs::write(&path, xml).unwrap();
    let doc = sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let r = sr_deliver::deliver(&doc, &doc.scene.outputs[0], None, &Default::default(), &mut |_, _| {}).unwrap();
    let a = sr_media::decode_audio(&r.path, 0, RATE).unwrap();
    let x: Vec<f32> = a.samples.chunks(a.channels as usize).map(|f| f[0]).collect();
    assert!((frequency(&x, 0.1, 1.9) - 440.0).abs() < 1.0);
}

#[test]
fn stretching_keeps_loudness() {
    let d = fixtures("stretchloud");
    let loud = |speed: &str| {
        let (_, r) = deliver(
            &d,
            &format!(
                r#"<output path="l{speed}.wav" codec="audio-only"><segment from="0" to="3" speed="{speed}"/></output>"#
            ),
            r#"<audioTrack id="t" asset="a440"/>"#,
        );
        r.loudness.unwrap()
    };
    let (a, b) = (loud("1"), loud("2"));
    assert!((a - b).abs() < 0.5, "{a} LUFS at speed 1, {b} LUFS stretched to half");
}
