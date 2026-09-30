//! piper: local neural text-to-speech (`piper` on `PATH` or `SR_PIPER`).
//!
//! `model` names a voice (`en_US-lessac-medium`), found as `<model>.onnx` in
//! `SR_PIPER_VOICES`, `~/.local/share/piper` or `~/.local/share/piper-voices`,
//! or is a path to the `.onnx` file. A numeric `voice` picks a speaker of a
//! multi-speaker model. The text is the `prompt`. piper samples noise, so two
//! runs differ slightly; the resolved cache pins the one the document uses.

use std::path::PathBuf;
use std::process::Command;

use super::{convert, run, tool, Provider};
use crate::protocol::{Request, Response};

pub struct Piper;

pub(crate) fn voice(req: &Request) -> Result<PathBuf, String> {
    let m = PathBuf::from(&req.model);
    let direct = if m.is_absolute() { m.clone() } else { PathBuf::from(&req.base_dir).join(&m) };
    if direct.is_file() {
        return Ok(direct);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::var_os("SR_PIPER_VOICES")
        .map(|d| std::env::split_paths(&d).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain([home.join(".local/share/piper"), home.join(".local/share/piper-voices")])
        .collect();
    dirs.iter().map(|d| d.join(format!("{}.onnx", req.model))).find(|p| p.is_file()).ok_or_else(|| {
        format!("piper voice {:?} not found (set SR_PIPER_VOICES to the folder of its .onnx file)", req.model)
    })
}

impl Provider for Piper {
    fn name(&self) -> String {
        "piper".into()
    }

    fn run(&self, req: &Request) -> Result<Response, String> {
        if req.kind != "speech" {
            return Err(format!("piper makes speech, not {}", req.kind));
        }
        let text = req.prompt.as_deref().filter(|t| !t.trim().is_empty()).ok_or("piper needs the text as @prompt")?;
        let exe = tool("SR_PIPER", "piper").ok_or("piper not found (install piper-tts or set SR_PIPER)")?;
        let model = voice(req)?;
        let text_file = PathBuf::from(&req.workdir).join("piper.txt");
        std::fs::write(&text_file, text).map_err(|e| e.to_string())?;
        let wav = PathBuf::from(&req.workdir).join("piper.wav");
        let mut cmd = Command::new(&exe);
        cmd.arg("-m").arg(&model).arg("-i").arg(&text_file).arg("-f").arg(&wav);
        if let Some(s) = req.voice.as_deref().filter(|v| v.parse::<u32>().is_ok()) {
            cmd.args(["-s", s]);
        }
        run(&mut cmd, "piper")?;
        convert(&wav, &PathBuf::from(&req.output))?;
        Ok(Response {
            ok: true,
            version: Some(format!("piper {}", model.file_name().unwrap_or_default().to_string_lossy())),
            ..Default::default()
        })
    }
}
