//! The master's true-peak ceiling, held for the stream a delivery actually carries.
//!
//! A lossy encode is not bound by the ceiling the limiter set before it: AAC overshoots on loud, transient-rich mixes
//! (+2.9 dBTP decoded from a mix limited to -1.5 dBTP in the test). So the mix is encoded and decoded once before the
//! real encode; if the decoded true peak is over the ceiling, the mix is turned down by the excess plus a margin and
//! checked again. The second measurement decides: still over is an error.
//!
//! The encoder is the one the real encode uses, with the same flags (`sr_media::encode::compressed_audio_args`), so
//! what is measured is what is delivered. Noise substitution and noise shaping stay off for the reason given there.

use std::path::Path;
use std::process::{Command, Stdio};

use sr_audio::loudness::{self, Planar};
use sr_audio::Layout;
use sr_media::encode::Container;

use crate::DeliverError;

/// How far below the ceiling the corrected mix aims, beyond the measured excess.
pub const MARGIN_DB: f64 = 0.2;
/// How far over the ceiling a decoded measurement may be before it counts as over (estimator noise).
pub const TOLERANCE_DB: f64 = 0.05;

/// How many corrections are tried before the delivery fails.
pub const MAX_PASSES: usize = 3;
/// The most the mix is turned down in all, dB.
pub const MAX_TURN_DOWN_DB: f64 = 6.0;

/// One correction: the gain applied to the mix and the decoded true peak measured after it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Pass {
    /// Gain applied in this pass, dB (negative).
    pub gain_db: f64,
    /// Decoded true peak after it, dBTP.
    pub decoded_true_peak: f64,
}

/// What holding the ceiling measured and did.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Held {
    /// The master's true-peak ceiling, dBTP.
    pub ceiling: f64,
    /// Decoded true peak of the encode of the mix as it was, dBTP.
    pub first: f64,
    /// The corrections, in order; empty when the first encode held the ceiling.
    pub passes: Vec<Pass>,
}

impl Held {
    /// The total gain applied to the mix, dB (0 when none was).
    pub fn total_gain_db(&self) -> f64 {
        self.passes.iter().map(|p| p.gain_db).sum()
    }

    /// The decoded true peak of the stream that is delivered, dBTP.
    pub fn delivered(&self) -> f64 {
        self.passes.last().map_or(self.first, |p| p.decoded_true_peak)
    }
}

/// Whether a delivery encodes this audio with a codec whose decoded peak this module can check: AAC, in a container
/// that keeps it (WebM carries Opus instead, MP3 files MP3).
pub fn applies(audio_codec: &str, container: Option<Container>) -> bool {
    audio_codec == "aac" && !matches!(container, Some(Container::Webm | Container::Mp3 | Container::Wav))
}

fn planar(a: &sr_media::decode::AudioData) -> Planar {
    let n = a.channels.max(1) as usize;
    (0..n).map(|c| a.samples.iter().skip(c).step_by(n).copied().collect()).collect()
}

/// The decoded true peak of `mix` after one AAC encode at `bitrate`, dBTP.
fn decoded_true_peak(mix: &Planar, rate: u32, layout: Layout, bitrate: u64, tmp: &Path) -> Result<f64, DeliverError> {
    let wav = tmp.join("ceiling-probe.wav");
    let coded = tmp.join("ceiling-probe.m4a");
    sr_audio::wav::write(&wav, mix, rate, 32, layout, false).map_err(|e| DeliverError::AudioCeiling(e.to_string()))?;
    let mut cmd = Command::new(sr_media::ffmpeg());
    cmd.args(["-nostdin", "-v", "error", "-y", "-i"]).arg(&wav).args(["-c:a", "aac"]);
    cmd.args(sr_media::encode::compressed_audio_args("aac", bitrate)).arg(&coded);
    let out = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| DeliverError::AudioCeiling(format!("cannot start ffmpeg: {e}")))?;
    if !out.status.success() {
        return Err(DeliverError::AudioCeiling(format!(
            "the probe encode failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let decoded = sr_media::decode_audio(&coded, 0, rate)?;
    let _ = std::fs::remove_file(&wav);
    let _ = std::fs::remove_file(&coded);
    Ok(loudness::true_peak(&planar(&decoded)))
}

/// Holds `ceiling` (dBTP) for the AAC stream of `mix`: measures the decoded true peak of one encode; while it is over,
/// turns `mix` down by the excess plus [`MARGIN_DB`] and measures again, at most [`MAX_PASSES`] times and
/// [`MAX_TURN_DOWN_DB`] in all. `Err` with every measurement when the last one is still over.
///
/// A codec's overshoot is not a smooth function of level, so one correction can fall short; the passes are the
/// answer to that, not a different target.
pub fn hold(
    mix: &mut Planar,
    rate: u32,
    layout: Layout,
    ceiling: f64,
    bitrate: u64,
    tmp: &Path,
) -> Result<Held, DeliverError> {
    hold_with(mix, ceiling, &mut |m| decoded_true_peak(m, rate, layout, bitrate, tmp))
}

/// [`hold`] with the measurement of the decoded true peak given as a function of the mix.
fn hold_with(
    mix: &mut Planar,
    ceiling: f64,
    measure: &mut dyn FnMut(&Planar) -> Result<f64, DeliverError>,
) -> Result<Held, DeliverError> {
    let first = measure(mix)?;
    let mut held = Held { ceiling, first, passes: Vec::new() };
    let mut measured = first;
    while measured > ceiling + TOLERANCE_DB && held.passes.len() < MAX_PASSES {
        // the excess plus the margin, but never past the cap on the total
        let room = MAX_TURN_DOWN_DB + held.total_gain_db();
        let gain_db = (ceiling - measured - MARGIN_DB).max(-room);
        if gain_db >= -1e-9 {
            break;
        }
        let g = sr_audio::dsp::db_to_lin(gain_db) as f32;
        for ch in mix.iter_mut() {
            for s in ch.iter_mut() {
                *s *= g;
            }
        }
        measured = measure(mix)?;
        held.passes.push(Pass { gain_db, decoded_true_peak: measured });
    }
    if measured > ceiling + TOLERANCE_DB {
        let passes: Vec<String> = held
            .passes
            .iter()
            .enumerate()
            .map(|(i, p)| format!("pass {}: {:.2} dB -> {:.2} dBTP", i + 1, p.gain_db, p.decoded_true_peak))
            .collect();
        return Err(DeliverError::AudioCeiling(format!(
            "the decoded true peak is {first:.2} dBTP over a ceiling of {ceiling:.2}; {} (total {:.2} dB, at most {MAX_TURN_DOWN_DB} dB in {MAX_PASSES} passes), still {measured:.2} dBTP",
            if passes.is_empty() { "no correction was possible".to_string() } else { passes.join("; ") },
            held.total_gain_db()
        )));
    }
    Ok(held)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mix_at(db: f64) -> Planar {
        vec![vec![sr_audio::dsp::db_to_lin(db) as f32; 8]; 2]
    }

    fn peak_db(m: &Planar) -> f64 {
        sr_audio::dsp::lin_to_db(m.iter().flatten().fold(0.0f32, |a, v| a.max(v.abs())) as f64)
    }

    #[test]
    fn the_turn_down_is_capped_and_a_failure_lists_every_pass() {
        // a codec that adds 8 dB to whatever it is given needs 9.7 dB at a ceiling of -1.5 from a mix at 0 dB: the first pass takes
        // the 6 dB the cap allows, and no second pass is possible
        let mut mix = mix_at(0.0);
        let err = hold_with(&mut mix, -1.5, &mut |m| Ok(peak_db(m) + 8.0)).unwrap_err().to_string();
        assert!(err.contains("pass 1: -6.00 dB") && err.contains("total -6.00 dB") && err.contains("still"), "{err}");
        assert!(peak_db(&mix) >= -MAX_TURN_DOWN_DB - 1e-3, "never more than the cap: {}", peak_db(&mix));
    }

    #[test]
    fn corrections_stop_at_the_pass_limit() {
        // a codec whose overshoot shrinks only as the mix does not: every pass leaves 1 dB over
        let mut mix = mix_at(0.0);
        let mut calls = 0;
        let err = hold_with(&mut mix, -3.0, &mut |m| {
            calls += 1;
            Ok(-3.0 + 1.0 + 0.0 * peak_db(m))
        })
        .unwrap_err()
        .to_string();
        assert_eq!(calls, 1 + MAX_PASSES, "{err}");
    }

    #[test]
    fn a_mix_that_holds_the_ceiling_is_left_alone() {
        let mut mix = mix_at(-6.0);
        let held = hold_with(&mut mix, -1.5, &mut |m| Ok(peak_db(m) + 1.0)).unwrap();
        assert!(held.passes.is_empty() && held.total_gain_db() == 0.0);
        assert_eq!(mix, mix_at(-6.0));
    }

    #[test]
    fn each_pass_aims_at_the_ceiling_less_the_margin() {
        // a linear codec (+3 dB): one pass by the excess plus the margin lands 0.2 dB under the ceiling
        let mut mix = mix_at(0.0);
        let held = hold_with(&mut mix, -1.5, &mut |m| Ok(peak_db(m) + 3.0)).unwrap();
        assert_eq!(held.passes.len(), 1);
        assert!((held.passes[0].gain_db - (-1.5 - 3.0 - 0.2)).abs() < 1e-9, "{held:?}");
        assert!((held.delivered() - (-1.7)).abs() < 1e-3, "{held:?}");
    }
}
