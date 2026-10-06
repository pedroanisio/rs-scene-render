//! The decoded true peak of a delivered AAC stays under the master's ceiling.

use std::path::Path;

/// A loud, flat-topped stereo mix: summed tones and noise driven hard into a clip, 48 kHz, 16 bit.
fn write_clipped_mix(path: &Path, seconds: usize, drive: f64) {
    let rate = 48_000usize;
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut noise = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) as f64 / u64::MAX as f64) * 2.0 - 1.0
    };
    let tones = [(55.0, 1.0), (110.0, 0.8), (220.0, 0.5), (440.0, 0.4), (880.0, 0.3), (1760.0, 0.25), (3520.0, 0.2)];
    let mut raw = Vec::with_capacity(rate * seconds * 2);
    for i in 0..rate * seconds {
        for c in 0..2 {
            let t = i as f64 / rate as f64;
            let mut s: f64 = tones
                .iter()
                .enumerate()
                .map(|(k, (f, a))| a * (std::f64::consts::TAU * f * (1.0 + 0.003 * c as f64) * t + k as f64).sin())
                .sum();
            s += 0.4 * noise();
            raw.push(s);
        }
    }
    let peak = raw.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let pcm: Vec<u8> = raw
        .iter()
        .flat_map(|v| (((v / peak * drive).clamp(-1.0, 1.0) * 0.84 * 32767.0) as i16).to_le_bytes())
        .collect();
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + pcm.len() as u32).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend((rate as u32).to_le_bytes());
    wav.extend((rate as u32 * 4).to_le_bytes());
    wav.extend(4u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend((pcm.len() as u32).to_le_bytes());
    wav.extend(pcm);
    std::fs::write(path, wav).unwrap();
}

fn decoded_true_peak(path: &Path) -> f64 {
    let a = sr_media::decode_audio(path, 0, 48_000).unwrap();
    let planar: Vec<Vec<f32>> = (0..a.channels as usize)
        .map(|c| a.samples.iter().skip(c).step_by(a.channels as usize).copied().collect())
        .collect();
    sr_audio::loudness::true_peak(&planar)
}

fn scene(dir: &Path, ceiling: f64) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="6" background="#000000"/>
        <output id="m" path="out/ceiling.m4a" codec="audio-only"/>
        <assets><audio id="a" src="mix.wav"/></assets><composition/>
        <audioMix sampleRate="48000"><audioTrack id="t" asset="a"/><master normalize="none" truePeak="{ceiling}" limiter="true"/></audioMix></scene>"##
    );
    let path = dir.join("scene.xml");
    std::fs::write(&path, xml).unwrap();
    sr_model::load_file(&path, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"))
}

#[test]
fn a_delivered_aac_holds_the_masters_true_peak_ceiling() {
    if std::process::Command::new(sr_media::ffmpeg()).arg("-version").output().is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("sr-aac-ceiling-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write_clipped_mix(&dir.join("mix.wav"), 6, 1.2);
    let d = scene(&dir, -1.5);
    let opts = sr_deliver::Options { hardware: sr_media::encode::Hardware::Software, ..Default::default() };
    let report =
        sr_deliver::deliver(&d, &d.scene.outputs[0], None, &opts, &mut |_, _| {}).unwrap_or_else(|e| panic!("{e}"));
    let tp = decoded_true_peak(&dir.join("out/ceiling.m4a"));
    assert!(tp <= -1.5 + 0.05, "decoded true peak {tp:.2} dBTP is over the ceiling of -1.5");
    // the report says what was measured and what was done about it
    let held = report.audio_ceiling.expect("the ceiling was checked");
    assert_eq!(held.ceiling, -1.5);
    assert!(held.first > -1.5 + 0.05, "the mix as it was overshoots: {}", held.first);
    let first_pass = &held.passes[0];
    assert!(first_pass.gain_db < 0.0 && held.delivered() <= -1.5 + 0.05, "{held:?}");
    assert!((first_pass.gain_db - (-1.5 - held.first - 0.2)).abs() < 1e-9, "the excess plus 0.2 dB: {held:?}");
}

#[test]
fn a_mix_that_already_holds_the_ceiling_is_not_touched() {
    if std::process::Command::new(sr_media::ffmpeg()).arg("-version").output().is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("sr-aac-ceiling-quiet-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write_clipped_mix(&dir.join("mix.wav"), 6, 1.2);
    // a ceiling far above the overshoot: the first encode holds it
    let d = scene(&dir, -0.0);
    let opts = sr_deliver::Options { hardware: sr_media::encode::Hardware::Software, ..Default::default() };
    let report =
        sr_deliver::deliver(&d, &d.scene.outputs[0], None, &opts, &mut |_, _| {}).unwrap_or_else(|e| panic!("{e}"));
    let held = report.audio_ceiling.expect("the ceiling was checked");
    assert!(held.first <= 0.05 && held.passes.is_empty(), "{held:?}");
}

fn deliver(dir: &Path, ceiling: f64) -> Result<sr_deliver::pipeline::Report, sr_deliver::DeliverError> {
    let d = scene(dir, ceiling);
    let opts = sr_deliver::Options { hardware: sr_media::encode::Hardware::Software, ..Default::default() };
    sr_deliver::deliver(&d, &d.scene.outputs[0], None, &opts, &mut |_, _| {})
}

#[test]
fn a_hard_driven_mix_is_corrected_in_more_than_one_pass() {
    if std::process::Command::new(sr_media::ffmpeg()).arg("-version").output().is_err() {
        return;
    }
    // driven hard into a clip, the decoded peak sits several dB over the ceiling, and one correction by the excess falls
    // short: the overshoot of the codec is not a smooth function of level
    let dir = std::env::temp_dir().join(format!("sr-aac-ceiling-hard-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write_clipped_mix(&dir.join("mix.wav"), 6, 4.0);
    let report = deliver(&dir, -1.5).unwrap_or_else(|e| panic!("{e}"));
    let held = report.audio_ceiling.expect("the ceiling was checked");
    assert!(held.first > 0.0, "the first encode overshoots by several dB: {}", held.first);
    assert!(held.passes.len() >= 2 && held.passes.len() <= sr_deliver::ceiling::MAX_PASSES, "{held:?}");
    assert!(
        held.delivered() <= -1.5 + 0.05 && held.total_gain_db() >= -sr_deliver::ceiling::MAX_TURN_DOWN_DB,
        "{held:?}"
    );
    let tp = decoded_true_peak(&dir.join("out/ceiling.m4a"));
    assert!(tp <= -1.5 + 0.05, "the delivered file decodes to {tp:.2} dBTP");
}
