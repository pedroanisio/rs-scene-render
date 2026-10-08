//! `encode` says what the AAC ceiling check measured and did.

use std::path::Path;
use std::process::Command;

/// A loud, flat-topped stereo mix: summed tones and noise driven hard into a clip, 48 kHz, 16 bit.
fn write_clipped_mix(path: &Path, seconds: usize) {
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
    let pcm: Vec<u8> =
        raw.iter().flat_map(|v| (((v / peak * 1.2).clamp(-1.0, 1.0) * 0.84 * 32767.0) as i16).to_le_bytes()).collect();
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

#[test]
fn encode_reports_the_decoded_aac_true_peak_and_the_gain_it_took() {
    if Command::new(sr_media_ffmpeg()).arg("-version").output().is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("sr-cli-aac-ceiling-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write_clipped_mix(&dir.join("mix.wav"), 6);
    std::fs::write(
        dir.join("scene.xml"),
        r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="6" background="#000000"/>
        <assets><audio id="a" src="mix.wav"/></assets><composition/>
        <audioMix sampleRate="48000"><audioTrack id="t" asset="a"/><master normalize="none" truePeak="-1.5" limiter="true"/></audioMix></scene>"##,
    )
    .unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_scene-render"))
        .args(["encode", "scene.xml", "-o", "out.m4a", "--codec", "audio-only", "--no-upload"])
        .current_dir(&dir)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&o.stdout);
    assert_eq!(o.status.code(), Some(0), "{text}{}", String::from_utf8_lossy(&o.stderr));
    assert!(text.contains("audio ceiling: the AAC decodes to") && text.contains("mixed down"), "{text}");
}

fn sr_media_ffmpeg() -> String {
    std::env::var("SR_FFMPEG").unwrap_or_else(|_| "ffmpeg".into())
}
