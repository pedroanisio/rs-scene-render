//! whisper.cpp: local transcription with word timing (`whisper-cli` on
//! `PATH` or `SR_WHISPER`).
//!
//! `model` is a ggml model name (`base.en`, `small`, `large-v3`), found as
//! `ggml-<model>.bin` in `SR_WHISPER_MODELS` or in the `models/` folder of the
//! whisper.cpp tree the program was built in, or a path to the file. The input
//! is resampled to 16 kHz mono, transcribed with full JSON output, and the
//! tokens are joined into words. For the standard models, token times come from
//! dynamic time warping of the cross-attention (`--dtw`, which whisper.cpp
//! computes only with flash attention off), which places words more accurately
//! than the decoder's own timestamps.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use super::{run, tool, Provider};
use crate::protocol::{Request, Response};

pub struct Whisper;

/// whisper.cpp's `--dtw` preset for a model name.
fn dtw_preset(model: &str) -> Option<String> {
    let m = model.trim_start_matches("ggml-").trim_end_matches(".bin").replace('-', ".");
    const PRESETS: [&str; 12] = [
        "tiny",
        "tiny.en",
        "base",
        "base.en",
        "small",
        "small.en",
        "medium",
        "medium.en",
        "large.v1",
        "large.v2",
        "large.v3",
        "large.v3.turbo",
    ];
    PRESETS.contains(&m.as_str()).then_some(m)
}

pub(crate) fn model_file(req: &Request, exe: &Path) -> Result<PathBuf, String> {
    let m = PathBuf::from(&req.model);
    let direct = if m.is_absolute() { m.clone() } else { PathBuf::from(&req.base_dir).join(&m) };
    if direct.is_file() {
        return Ok(direct);
    }
    let name = format!("ggml-{}.bin", req.model.trim_start_matches("ggml-").trim_end_matches(".bin"));
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("SR_WHISPER_MODELS").map(|d| std::env::split_paths(&d).collect()).unwrap_or_default();
    // whisper.cpp/build/bin/whisper-cli → whisper.cpp/models
    if let Some(tree) = exe.canonicalize().ok().and_then(|p| p.ancestors().nth(3).map(Path::to_path_buf)) {
        dirs.push(tree.join("models"));
    }
    dirs.iter().map(|d| d.join(&name)).find(|p| p.is_file()).ok_or_else(|| {
        format!("whisper model {name} not found (download it with whisper.cpp's models/download-ggml-model.sh and set SR_WHISPER_MODELS)")
    })
}

/// A word of the transcript.
struct Word {
    start: f64,
    end: f64,
    text: String,
}

/// Converts whisper.cpp's full JSON output into the renderer's transcript format.
pub(crate) fn transcript(v: &Value) -> Value {
    let mut segments = Vec::new();
    for seg in v["transcription"].as_array().into_iter().flatten() {
        let ms = |k: &str| seg["offsets"][k].as_f64().unwrap_or(0.0) / 1000.0;
        let (s0, s1) = (ms("from"), ms("to"));
        let mut words: Vec<Word> = Vec::new();
        for tok in seg["tokens"].as_array().into_iter().flatten() {
            let text = tok["text"].as_str().unwrap_or("");
            if text.starts_with("[_") || text.starts_with("<|") || text.is_empty() {
                continue;
            }
            let dtw = tok["t_dtw"].as_f64().filter(|t| *t >= 0.0).map(|t| t / 100.0);
            let at = dtw.unwrap_or_else(|| tok["offsets"]["from"].as_f64().unwrap_or(0.0) / 1000.0).clamp(s0, s1);
            let to = tok["offsets"]["to"].as_f64().map(|t| t / 1000.0).unwrap_or(at).clamp(at, s1);
            match words.last_mut() {
                Some(w) if !text.starts_with(' ') => {
                    w.text.push_str(text);
                    w.end = w.end.max(to);
                }
                _ => words.push(Word { start: at, end: to, text: text.trim_start().to_string() }),
            }
        }
        // a word lasts until the next begins (DTW gives onsets), within the segment
        for i in 0..words.len() {
            let next = words.get(i + 1).map(|n| n.start).unwrap_or(s1);
            words[i].end = next.max(words[i].start);
        }
        let text = seg["text"].as_str().unwrap_or("").trim().to_string();
        if text.is_empty() && words.is_empty() {
            continue;
        }
        // whisper's segments start at the window's edge; captions start with the first word
        let s0 = words.first().map(|w| w.start.max(s0)).unwrap_or(s0);
        segments.push(json!({
            "start": s0,
            "end": s1,
            "text": text,
            "words": words.iter().filter(|w| !w.text.is_empty()).map(|w| json!({"start": w.start, "end": w.end, "text": w.text})).collect::<Vec<_>>(),
        }));
    }
    json!({ "segments": segments })
}

/// Cuts the silence before the first sound louder than −50 dBFS (keeping 0.2 s) from a 16-bit
/// mono WAV in place; returns the seconds cut.
fn trim_leading_silence(path: &Path) -> Result<f64, String> {
    let b = std::fs::read(path).map_err(|e| e.to_string())?;
    let data = b.windows(4).position(|w| w == b"data").ok_or("whisper input: no data chunk")?;
    let pcm = &b[data + 8..];
    let first = pcm.chunks_exact(2).position(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs() > 104);
    let cut = first.map(|i| i.saturating_sub(3200)).unwrap_or(0);
    if cut < 1600 {
        return Ok(0.0);
    }
    std::fs::write(path, super::wav_from_pcm16(&pcm[cut * 2..], 16000, 1)).map_err(|e| e.to_string())?;
    Ok(cut as f64 / 16000.0)
}

/// Adds `dt` seconds to every time of a transcript.
fn shift(t: &mut Value, dt: f64) {
    if dt == 0.0 {
        return;
    }
    for s in t["segments"].as_array_mut().into_iter().flatten() {
        for k in ["start", "end"] {
            s[k] = json!(s[k].as_f64().unwrap_or(0.0) + dt);
        }
        for w in s["words"].as_array_mut().into_iter().flatten() {
            for k in ["start", "end"] {
                w[k] = json!(w[k].as_f64().unwrap_or(0.0) + dt);
            }
        }
    }
}

impl Provider for Whisper {
    fn name(&self) -> String {
        "whisper".into()
    }

    fn run(&self, req: &Request) -> Result<Response, String> {
        let input = req.input.as_deref().ok_or("whisper transcribes: the request has no input")?;
        let exe =
            tool("SR_WHISPER", "whisper-cli").ok_or("whisper-cli not found (build whisper.cpp or set SR_WHISPER)")?;
        let model = model_file(req, &exe)?;
        let work = PathBuf::from(&req.workdir);
        let wav16 = work.join("whisper-16k.wav");
        run(
            Command::new(sr_media::ffmpeg())
                .args(["-v", "error", "-nostdin", "-y", "-i", input, "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
                .arg(&wav16),
            "FFmpeg",
        )?;
        // Leading silence misleads whisper's first timestamps: transcribe from just before the
        // first sound and shift every time back.
        let lead = trim_leading_silence(&wav16)?;
        let lang = req.language.as_deref().map(|l| l.split(['-', '_']).next().unwrap_or(l).to_ascii_lowercase());
        let lang = if req.model.ends_with(".en") { Some("en".to_string()) } else { lang };
        let out_base = work.join("whisper");
        let mut cmd = Command::new(&exe);
        cmd.arg("-m").arg(&model).arg("-f").arg(&wav16).arg("-ojf").arg("-of").arg(&out_base).arg("-np");
        cmd.args(["-l", lang.as_deref().unwrap_or("auto")]);
        if let Some(p) = &req.prompt {
            cmd.args(["--prompt", p]);
        }
        let dtw = dtw_preset(&req.model);
        if let Some(d) = &dtw {
            // whisper.cpp computes DTW timestamps only without flash attention
            cmd.args(["--dtw", d, "-nfa"]);
        }
        run(&mut cmd, "whisper-cli")?;
        let raw =
            std::fs::read_to_string(out_base.with_extension("json")).map_err(|e| format!("whisper output: {e}"))?;
        let v: Value = serde_json::from_str(&raw).map_err(|e| format!("whisper output: {e}"))?;
        let mut t = transcript(&v);
        shift(&mut t, lead);
        std::fs::write(&req.output, serde_json::to_vec_pretty(&t).expect("json")).map_err(|e| e.to_string())?;
        let mut notes = Vec::new();
        if dtw.is_none() {
            notes.push(format!("{} has no DTW preset: word times are whisper's token timestamps", req.model));
        }
        Ok(Response {
            ok: true,
            version: Some(format!("whisper.cpp {}", model.file_name().unwrap_or_default().to_string_lossy())),
            notes,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_join_into_timed_words() {
        let v = json!({"transcription": [{
            "offsets": {"from": 1000, "to": 3000}, "text": " Hello, world.",
            "tokens": [
                {"text": "[_BEG_]", "offsets": {"from": 1000, "to": 1000}, "t_dtw": -1},
                {"text": " Hel", "offsets": {"from": 1000, "to": 1300}, "t_dtw": 110},
                {"text": "lo", "offsets": {"from": 1300, "to": 1500}, "t_dtw": 130},
                {"text": ",", "offsets": {"from": 1500, "to": 1600}, "t_dtw": 150},
                {"text": " world", "offsets": {"from": 1700, "to": 2500}, "t_dtw": 180},
                {"text": ".", "offsets": {"from": 2500, "to": 2600}, "t_dtw": 250},
                {"text": "[_TT_150]", "offsets": {"from": 3000, "to": 3000}, "t_dtw": -1}]}]});
        let t = transcript(&v);
        let s = &t["segments"][0];
        assert_eq!(s["text"], "Hello, world.");
        assert_eq!(s["words"][0]["text"], "Hello,");
        assert_eq!(s["words"][0]["start"], 1.1);
        assert_eq!(s["words"][0]["end"], 1.8);
        assert_eq!(s["words"][1]["text"], "world.");
        assert_eq!(s["words"][1]["end"], 3.0);
        assert_eq!(dtw_preset("large-v3").as_deref(), Some("large.v3"));
        assert_eq!(dtw_preset("custom-finetune"), None);
    }
}
