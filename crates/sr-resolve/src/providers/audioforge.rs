//! AudioForge: music beds, stings and sound effects from its cue library
//! (`audioforge` on `PATH` or `SR_AUDIOFORGE`).
//!
//! `model` is the cue (`bed/documentary-airy`), `prompt` its parameters
//! (`duration=auto bpm=96 key='A minor'`) and `seed` the performance seed.
//! `duration=auto` sizes the cue to the project: its duration less the start
//! of the audio track that plays it. Sample rate and bit depth follow the
//! scene's `audioMix`.

use std::path::PathBuf;
use std::process::Command;

use super::{convert, run, shell_split, tool, Provider};
use crate::protocol::{Request, Response};

pub struct AudioForge;

/// The cue parameters of a request.
pub(crate) fn params(req: &Request) -> Vec<String> {
    let mut ps: Vec<String> = req.prompt.as_deref().map(shell_split).unwrap_or_default();
    let has = |ps: &[String], k: &str| ps.iter().any(|p| p.split_once('=').is_some_and(|(a, _)| a.trim() == k));
    if !has(&ps, "seed") {
        if let Some(s) = req.seed {
            ps.push(format!("seed={s}"));
        }
    }
    if !has(&ps, "duration") {
        if let Some(d) = req.duration {
            ps.push(format!("duration={d}"));
        }
    }
    for p in &mut ps {
        if p.replace(' ', "") == "duration=auto" {
            let start = req.timeline.start.unwrap_or(0.0).max(0.0);
            let d = (req.timeline.project_duration - start).max(0.05);
            *p = format!("duration={}", (d * 1e4).round() / 1e4);
        }
    }
    ps
}

impl Provider for AudioForge {
    fn name(&self) -> String {
        "audioforge".into()
    }

    fn run(&self, req: &Request) -> Result<Response, String> {
        if !matches!(req.kind.as_str(), "music" | "sound-effect") {
            return Err(format!("audioforge makes music and sound effects, not {}", req.kind));
        }
        let exe =
            tool("SR_AUDIOFORGE", "audioforge").ok_or("audioforge not found (install it or set SR_AUDIOFORGE)")?;
        let wav = PathBuf::from(&req.workdir).join("audioforge.wav");
        let bits = if req.bit_depth == 16 { "16" } else { "24" };
        let mut cmd = Command::new(&exe);
        cmd.args(["cue", "render", &req.model]).args(params(req)).arg("-o").arg(&wav).args([
            "--sample-rate",
            &req.sample_rate.to_string(),
            "--bit-depth",
            bits,
        ]);
        run(&mut cmd, "audioforge")?;
        convert(&wav, &PathBuf::from(&req.output))?;
        let version = Command::new(&exe)
            .arg("--version")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "audioforge".into());
        Ok(Response { ok: true, version: Some(version), ..Default::default() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Timeline;

    #[test]
    fn auto_duration_fills_the_project_after_the_track_starts() {
        let req = Request {
            prompt: Some("duration=auto bpm=96 key='A minor'".into()),
            seed: Some(7),
            timeline: Timeline { project_duration: 30.0, start: Some(4.5) },
            ..Default::default()
        };
        assert_eq!(params(&req), ["duration=25.5", "bpm=96", "key=A minor", "seed=7"]);
    }
}
