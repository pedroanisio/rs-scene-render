use std::sync::Arc;

use sr_audio::dsp::{db_to_lin, lin_to_db};
use sr_audio::effects::{Effect, Kind};
use sr_audio::*;

const RATE: u32 = 48000;

fn sine(f: f64, amp: f64, secs: f64) -> Vec<f32> {
    (0..(secs * RATE as f64) as usize)
        .map(|i| (amp * (2.0 * std::f64::consts::PI * f * i as f64 / RATE as f64).sin()) as f32)
        .collect()
}

fn mono(v: Vec<f32>) -> Arc<Source> {
    Arc::new(Source { layout: Layout::Mono, planar: vec![v] })
}

fn track(id: &str, src: Arc<Source>, start: f64) -> Node {
    Node {
        id: id.into(),
        kind: NodeKind::Track {
            source: src,
            placement: Placement::Timeline {
                start,
                clip_in: 0.0,
                clip_out: None,
                loops: 0,
                speed: 1.0,
                reverse: false,
                preserve_pitch: true,
                fit: None,
            },
            fade_in: 0.0,
            fade_out: 0.0,
            fade_curve: FadeCurve::Linear,
        },
        volume: Curve::Const(1.0),
        gain: Curve::Const(0.0),
        pan: Curve::Const(0.0),
        mute: false,
        output: None,
        duck: None,
        effects: Vec::new(),
    }
}

fn mix(duration: f64, nodes: Vec<Node>) -> Mix {
    Mix { rate: RATE, layout: Layout::Stereo, duration, control_fps: 30.0, nodes, master: Master::default() }
}

fn peak(x: &[f32]) -> f64 {
    lin_to_db(x.iter().fold(0f32, |m, v| m.max(v.abs())) as f64)
}

fn window(x: &[f32], a: f64, b: f64) -> &[f32] {
    &x[(a * RATE as f64) as usize..(b * RATE as f64) as usize]
}

#[test]
fn placement_clip_loop_and_centre_pan() {
    let src = mono(sine(1000.0, 0.5, 1.0));
    let mut t = track("a", src, 0.5);
    if let NodeKind::Track { placement: Placement::Timeline { clip_in, clip_out, loops, .. }, .. } = &mut t.kind {
        *clip_in = 0.25;
        *clip_out = Some(0.75);
        *loops = 1;
    }
    let m = mix(3.0, vec![t]).render().unwrap();
    let l = &m.master[0];
    assert!(peak(window(l, 0.0, 0.49)) < -120.0);
    // two plays of 0.5 s from 0.5 s, at −3 dB per side (constant-power centre)
    assert!((peak(window(l, 0.55, 1.45)) - (lin_to_db(0.5) - 3.01)).abs() < 0.05);
    assert!(peak(window(l, 1.55, 3.0)) < -120.0);
}

#[test]
fn fades_follow_their_curves() {
    let src = mono(vec![1.0; RATE as usize * 2]);
    for (curve, mid) in [
        (FadeCurve::Linear, 0.5),
        (FadeCurve::EqualPower, std::f64::consts::FRAC_1_SQRT_2),
        (FadeCurve::SCurve, 0.5),
        (FadeCurve::Logarithmic, 5.5f64.log10()),
        (FadeCurve::Exponential, (10f64.sqrt() - 1.0) / 9.0),
    ] {
        let mut t = track("a", src.clone(), 0.0);
        if let NodeKind::Track { fade_in, fade_curve, .. } = &mut t.kind {
            *fade_in = 1.0;
            *fade_curve = curve;
        }
        t.pan = Curve::Const(-1.0);
        let m = mix(2.0, vec![t]).render().unwrap();
        let v = m.master[0][RATE as usize / 2] as f64;
        assert!((v - mid).abs() < 1e-3, "{curve:?}: {v} vs {mid}");
        assert!((m.master[0][RATE as usize + 100] - 1.0).abs() < 1e-6);
    }
}

#[test]
fn automation_buses_and_ducking() {
    let music = mono(sine(200.0, 0.5, 4.0));
    let mut voice_v = vec![0f32; RATE as usize * 4];
    voice_v[RATE as usize..RATE as usize * 2].copy_from_slice(&sine(1000.0, 0.5, 1.0));
    let voice = mono(voice_v);
    let mut m = track("music", music, 0.0);
    m.output = Some("bed".into());
    m.volume = Curve::Frames((0..120).map(|k| if k < 90 { 1.0 } else { 0.5 }).collect());
    let v = track("voice", voice, 0.0);
    let bed = Node {
        id: "bed".into(),
        kind: NodeKind::Bus,
        volume: Curve::Const(1.0),
        gain: Curve::Const(0.0),
        pan: Curve::Const(0.0),
        mute: false,
        output: None,
        duck: Some(Duck { under: vec!["voice".into()], amount: -12.0, threshold: -40.0, attack: 0.05, release: 0.2 }),
        effects: Vec::new(),
    };
    let out = mix(4.0, vec![m, v, bed]).render().unwrap();
    let bed_out = &out.nodes["bed"][0];
    let base = peak(window(bed_out, 0.3, 0.9));
    let ducked = peak(window(bed_out, 1.4, 1.9));
    let recovered = peak(window(bed_out, 2.6, 2.95));
    let half = peak(window(bed_out, 3.2, 3.9));
    assert!((ducked - base + 12.0).abs() < 0.5, "ducked {ducked} vs {base}");
    assert!((recovered - base).abs() < 0.3, "recovered {recovered}");
    assert!((half - base + 6.02).abs() < 0.3, "volume automation {half}");
}

#[test]
fn routing_errors_are_reported() {
    let mut a = track("a", mono(vec![0.0; 10]), 0.0);
    a.output = Some("nowhere".into());
    assert_eq!(mix(0.1, vec![a]).render().unwrap_err(), MixError::Unknown("a".into(), "nowhere".into()));
    let mk = |id: &str, out: &str| Node {
        id: id.into(),
        kind: NodeKind::Bus,
        volume: Curve::Const(1.0),
        gain: Curve::Const(0.0),
        pan: Curve::Const(0.0),
        mute: false,
        output: Some(out.into()),
        duck: None,
        effects: vec![],
    };
    assert!(matches!(mix(0.1, vec![mk("x", "y"), mk("y", "x")]).render(), Err(MixError::Cycle(_))));
}

#[test]
fn integrated_normalisation_and_true_peak_ceiling() {
    let src = mono(sine(440.0, 0.9, 6.0));
    let mut m = mix(6.0, vec![track("a", src, 0.0)]);
    m.master = Master { normalize: Normalize::Integrated, loudness: -16.0, true_peak: -1.0, ..Master::default() };
    let out = m.render().unwrap();
    assert!((out.loudness + 16.0).abs() < 0.3, "{}", out.loudness);
    assert!(out.true_peak <= -0.95, "{}", out.true_peak);
    // a quiet source is raised to the target
    let quiet = mono(sine(440.0, 0.01, 6.0));
    let mut m = mix(6.0, vec![track("a", quiet, 0.0)]);
    m.master = Master { normalize: Normalize::Integrated, loudness: -23.0, ..Master::default() };
    assert!((m.render().unwrap().loudness + 23.0).abs() < 0.2);
}

#[test]
fn dynamic_normalisation_evens_out_sections() {
    let mut v = sine(440.0, 0.03, 8.0);
    v.extend(sine(440.0, 0.5, 8.0));
    let mut m = mix(16.0, vec![track("a", mono(v), 0.0)]);
    m.master = Master { normalize: Normalize::Dynamic, loudness: -20.0, ..Master::default() };
    let out = m.render().unwrap();
    let a = peak(window(&out.master[0], 5.0, 7.0));
    let b = peak(window(&out.master[0], 13.0, 15.0));
    // the input sections differ by 24.4 dB; three-second gain riding brings them within 4 dB
    assert!((a - b).abs() < 4.0, "sections {a} and {b} dB");
}

#[test]
fn time_stretch_keeps_pitch_and_varispeed_does_not() {
    let src = mono(sine(440.0, 0.5, 2.0));
    for (preserve, freq) in [(true, 440.0), (false, 880.0)] {
        let mut t = track("a", src.clone(), 0.0);
        if let NodeKind::Track { placement: Placement::Timeline { speed, preserve_pitch, .. }, .. } = &mut t.kind {
            *speed = 2.0;
            *preserve_pitch = preserve;
        }
        t.pan = Curve::Const(-1.0);
        let out = mix(2.0, vec![t]).render().unwrap();
        let l = &out.master[0];
        let body = window(l, 0.2, 0.8);
        let crossings = body.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() as f64 / 0.6 / 2.0;
        assert!((crossings - freq).abs() < 8.0, "preserve {preserve}: {crossings} Hz");
        assert!(peak(window(l, 1.05, 2.0)) < -60.0, "plays for 1 s");
    }
}

#[test]
fn mapped_video_audio_follows_source_time() {
    // a layer that starts at 1 s and plays its source at 0.5×
    let src = mono((0..RATE as usize * 2).map(|i| i as f32 / (RATE * 2) as f32).collect());
    let times: Vec<Option<f64>> = (0..90)
        .map(|k| {
            let t = k as f64 / 30.0;
            (t >= 1.0).then_some((t - 1.0) * 0.5)
        })
        .collect();
    let mut t = track("layer", src, 0.0);
    if let NodeKind::Track { placement, .. } = &mut t.kind {
        *placement = Placement::Mapped(times);
    }
    t.pan = Curve::Const(-1.0);
    let out = mix(3.0, vec![t]).render().unwrap();
    let at = |secs: f64| out.master[0][(secs * RATE as f64) as usize];
    assert_eq!(at(0.5), 0.0);
    // at 2 s the source is at 0.5 s, i.e. value 0.25
    assert!((at(2.0) - 0.25).abs() < 1e-3, "{}", at(2.0));
}

#[test]
fn surround_and_ambisonic_mixes() {
    let src = mono(sine(500.0, 0.5, 0.5));
    let mut m = mix(0.5, vec![track("a", src.clone(), 0.0)]);
    m.layout = Layout::Surround714;
    let out = m.render().unwrap();
    assert_eq!(out.master.len(), 12);
    assert!(peak(&out.master[2]) > -6.1 && peak(&out.master[0]) < -100.0, "centred mono goes to C");
    let mut m = mix(0.5, vec![track("a", src, 0.0)]);
    m.layout = Layout::Ambisonic3;
    let out = m.render().unwrap();
    assert_eq!(out.master.len(), 16);
    assert!((peak(&out.master[0]) - peak(&out.master[3])).abs() < 0.01, "front source: W = X");
}

#[test]
fn analysis_envelopes_and_beats() {
    // clicks at 120 BPM starting 0.1 s
    let mut v = vec![0f32; RATE as usize * 8];
    let mut t = 0.1;
    while t < 8.0 {
        let s = (t * RATE as f64) as usize;
        for k in 0..400 {
            v[s + k] = ((k as f32 * 0.9).sin()) * (1.0 - k as f32 / 400.0);
        }
        t += 0.5;
    }
    let buf = vec![v];
    let (bpm, beats) = analysis::beats(&buf, RATE as f64, None, 8.0);
    assert!((bpm - 120.0).abs() < 1.0, "{bpm}");
    assert!((beats[0] - 0.1).abs() < 0.03, "{:?}", &beats[..3]);
    let (_, fixed) = analysis::beats(&buf, RATE as f64, Some(120.0), 8.0);
    assert_eq!(fixed.len(), 16);
    let tone = vec![sine(100.0, 0.5, 1.0)];
    let env = analysis::envelopes(&tone, RATE as f64, 10.0, 10);
    assert!(env[1][5] > env[3][5] + 0.3, "low band carries a 100 Hz tone: {:?} vs {:?}", env[1][5], env[3][5]);
    assert!((env[0][5] - ((lin_to_db(0.5 / 2f64.sqrt()) + 60.0) / 60.0) as f32).abs() < 0.01);
}

#[test]
fn effects_run_in_the_graph_with_sidechains() {
    let src = mono(sine(1000.0, db_to_lin(-6.0), 2.0));
    let key = mono(sine(200.0, 0.9, 2.0));
    let mut a = track("a", src, 0.0);
    a.effects.push(Effect { sidechain: Some("key".into()), knee: 0.0, ..Effect::new(Kind::Compressor) });
    let mut k = track("key", key, 0.0);
    k.mute = false;
    k.volume = Curve::Const(1.0);
    let out = mix(2.0, vec![a, k]).render().unwrap();
    // the key (−0.9 dBFS·−3 dB pan) drives gain reduction on "a"
    assert!(peak(window(&out.nodes["a"][0], 1.0, 2.0)) < -15.0);
}

#[test]
fn wav_files_are_valid() {
    let dir = std::env::temp_dir().join(format!("sr-audio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let buf = vec![sine(440.0, 0.5, 0.1); 6];
    for bits in [16u16, 24, 32] {
        let p = dir.join(format!("t{bits}.wav"));
        wav::write(&p, &buf, RATE, bits, Layout::Surround51, true).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 8 + 4 + 8 + 40 + 8 + 4800 * 6 * bits as usize / 8);
    }
    // TPDF dither keeps silence within one LSB and is deterministic
    let a = wav::pcm_bytes(&vec![vec![0.0; 1000]], 16, true, 1);
    assert_eq!(a, wav::pcm_bytes(&vec![vec![0.0; 1000]], 16, true, 1));
    assert!(a.chunks(2).all(|c| i16::from_le_bytes([c[0], c[1]]).abs() <= 1));
}

#[test]
fn fitting_an_empty_segment_is_silent() {
    // clipIn past the end, clipOut before clipIn, and an empty file
    for (samples, a, b) in [(RATE as usize, 2.0, None), (RATE as usize, 0.5, Some(0.25)), (0, 0.0, None)] {
        let mut t = track("a", mono(vec![0.5; samples]), 0.0);
        if let NodeKind::Track { placement: Placement::Timeline { clip_in, clip_out, fit, .. }, .. } = &mut t.kind {
            (*clip_in, *clip_out, *fit) = (a, b, Some((120.0, 2.0)));
        }
        let m = mix(2.0, vec![t]).render().unwrap();
        assert!(m.master.iter().all(|c| c.iter().all(|v| *v == 0.0)));
    }
}
