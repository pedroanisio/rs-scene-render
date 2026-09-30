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
    let r = sr_deliver::deliver(&d, &d.scene.outputs[0], None, &Default::default(), &mut |_, _| {})
        .unwrap_or_else(|e| panic!("{e}"));
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
