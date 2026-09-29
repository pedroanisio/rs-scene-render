//! `scene-render-provider-example`: a reference provider for the resolve
//! protocol, and the provider the tests use.
//!
//! Put it on `PATH` (or name it in `SR_PROVIDER_EXAMPLE`) and write
//! `provider="example"`. It reads one JSON request on standard input and
//! answers with one JSON line:
//!
//! * speech, music and sound effects: a WAV sine tone at the scene's sample
//!   rate, `duration` seconds long (for speech, 0.3 s per word of the prompt),
//!   its pitch set by `seed`;
//! * images: a PNG of `width` × `height` in a colour derived from the prompt;
//! * transcription: one word per stretch of sound in the input, timed where
//!   the sound is, with the words of `prompt` (or `word1`, `word2`, …).
//!
//! When `SR_EXAMPLE_LOG` is set, each request appends a line to
//! `.example-calls` in the document's folder.

use std::io::{Read, Write};

use serde_json::json;
use sr_resolve::protocol::{Request, Response, Task};

fn answer(r: Result<(), String>) -> ! {
    let resp = match r {
        Ok(()) => Response { ok: true, version: Some("example 1".into()), ..Default::default() },
        Err(e) => Response { ok: false, error: Some(e), ..Default::default() },
    };
    println!("{}", serde_json::to_string(&resp).expect("json"));
    std::process::exit(if resp.ok { 0 } else { 1 })
}

fn wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let pcm: Vec<u8> = samples.iter().flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
    let mut v = Vec::new();
    v.extend(b"RIFF");
    v.extend((36 + pcm.len() as u32).to_le_bytes());
    v.extend(b"WAVEfmt ");
    v.extend(16u32.to_le_bytes());
    v.extend(1u16.to_le_bytes());
    v.extend(1u16.to_le_bytes());
    v.extend(rate.to_le_bytes());
    v.extend((rate * 2).to_le_bytes());
    v.extend(2u16.to_le_bytes());
    v.extend(16u16.to_le_bytes());
    v.extend(b"data");
    v.extend((pcm.len() as u32).to_le_bytes());
    v.extend(pcm);
    v
}

/// Mono samples of a PCM WAV (16, 24 or 32-bit integer, or 32-bit float), and its rate.
fn read_wav(b: &[u8]) -> Result<(Vec<f32>, u32), String> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("input is not a WAV file".into());
    }
    let (mut fmt, mut data) = (None, None);
    let mut i = 12;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let n = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
        let body = &b[i + 8..(i + 8 + n).min(b.len())];
        match id {
            b"fmt " => fmt = Some(body),
            b"data" => data = Some(body),
            _ => {}
        }
        i += 8 + n + (n & 1);
    }
    let (fmt, data) = (fmt.ok_or("no fmt chunk")?, data.ok_or("no data chunk")?);
    let u16_at = |o: usize| u16::from_le_bytes([fmt[o], fmt[o + 1]]);
    let (format, channels, rate, bits) =
        (u16_at(0), u16_at(2) as usize, u32::from_le_bytes(fmt[4..8].try_into().unwrap()), u16_at(14));
    let width = bits as usize / 8;
    let frame = width * channels;
    let sample = |s: &[u8]| -> f32 {
        match (format, bits) {
            (3, 32) => f32::from_le_bytes(s.try_into().unwrap()),
            (_, 16) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
            (_, 24) => ((i32::from_le_bytes([0, s[0], s[1], s[2]])) >> 8) as f32 / 8_388_608.0,
            (_, 32) => i32::from_le_bytes(s.try_into().unwrap()) as f32 / 2_147_483_648.0,
            _ => 0.0,
        }
    };
    Ok((data.chunks_exact(frame).map(|f| sample(&f[..width])).collect(), rate))
}

fn run(req: &Request) -> Result<(), String> {
    if std::env::var_os("SR_EXAMPLE_LOG").is_some() {
        let log = std::path::Path::new(&req.base_dir).join(".example-calls");
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(log).map_err(|e| e.to_string())?;
        writeln!(f, "{} {}", req.id, req.kind).map_err(|e| e.to_string())?;
    }
    let out = std::path::Path::new(&req.output);
    let ext = out.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match (req.task, req.kind.as_str()) {
        (Some(Task::Transcribe), _) => {
            let (x, rate) =
                read_wav(&std::fs::read(req.input.as_deref().ok_or("no input")?).map_err(|e| e.to_string())?)?;
            let words: Vec<String> =
                req.prompt.as_deref().unwrap_or("").split_whitespace().map(str::to_string).collect();
            // stretches louder than −40 dBFS, joined across gaps under 50 ms
            let hop = (rate / 100).max(1) as usize;
            let loud: Vec<bool> = x.chunks(hop).map(|c| c.iter().fold(0f32, |m, v| m.max(v.abs())) > 0.01).collect();
            let mut runs: Vec<(usize, usize)> = Vec::new();
            for (k, l) in loud.iter().enumerate() {
                if !*l {
                    continue;
                }
                match runs.last_mut() {
                    Some(r) if k - r.1 <= 5 => r.1 = k + 1,
                    _ => runs.push((k, k + 1)),
                }
            }
            let t = |k: usize| (k * hop) as f64 / rate as f64;
            let ws: Vec<serde_json::Value> = runs
                .iter()
                .enumerate()
                .map(|(i, &(a, b))| {
                    let text = words.get(i).cloned().unwrap_or_else(|| format!("word{}", i + 1));
                    json!({"start": t(a), "end": t(b), "text": text})
                })
                .collect();
            let text: Vec<&str> = ws.iter().filter_map(|w| w["text"].as_str()).collect();
            let seg = json!({"segments": [{"start": runs.first().map(|r| t(r.0)).unwrap_or(0.0),
                                            "end": runs.last().map(|r| t(r.1)).unwrap_or(0.0),
                                            "text": text.join(" "), "words": ws}]});
            std::fs::write(out, serde_json::to_vec(&seg).expect("json")).map_err(|e| e.to_string())
        }
        (_, "speech" | "music" | "sound-effect") => {
            if ext != "wav" {
                return Err(format!("the example provider writes WAV, not .{ext}"));
            }
            let words = req.prompt.as_deref().map(|p| p.split_whitespace().count()).unwrap_or(0);
            let secs = req.duration.unwrap_or(if req.kind == "speech" { 0.3 * words.max(1) as f64 } else { 1.0 });
            let rate = req.sample_rate.max(8000);
            let f = 220.0 + (req.seed.unwrap_or(0) % 8) as f32 * 55.0;
            let n = (secs * rate as f64).round() as usize;
            let x: Vec<f32> =
                (0..n).map(|i| 0.5 * (std::f32::consts::TAU * f * i as f32 / rate as f32).sin()).collect();
            std::fs::write(out, wav(&x, rate)).map_err(|e| e.to_string())
        }
        (_, "image") => {
            let h = req
                .prompt
                .as_deref()
                .unwrap_or("")
                .bytes()
                .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
            let (w, hgt) = (req.width.unwrap_or(64) as u32, req.height.unwrap_or(64) as u32);
            let img = image::RgbImage::from_pixel(w, hgt, image::Rgb([(h >> 16) as u8, (h >> 8) as u8, h as u8]));
            img.save(out).map_err(|e| e.to_string())
        }
        (_, other) => Err(format!("the example provider does not make {other}")),
    }
}

fn main() {
    let mut body = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut body) {
        answer(Err(e.to_string()));
    }
    match serde_json::from_str::<Request>(&body) {
        Ok(req) => answer(run(&req)),
        Err(e) => answer(Err(format!("bad request: {e}"))),
    }
}
