//! Providers: the programs and services that make generated media and
//! transcriptions.
//!
//! A provider named `name` is, in order:
//! 1. the command in `SR_PROVIDER_<NAME>` (name upper-cased, `-` as `_`), or a
//!    `scene-render-provider-<name>` executable on `PATH`, spoken to with the
//!    JSON protocol ([`crate::protocol`]); external programs override built-ins;
//! 2. a built-in adapter: `whisper` (whisper.cpp), `piper`, `audioforge`, and
//!    the cloud services `openai` and `elevenlabs`, which send the prompt off
//!    the machine and so run only with `--allow-cloud`.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::protocol::{Request, Response};

mod audioforge;
mod cloud;
mod piper;
pub mod tiles;
mod whisper;

/// A provider.
pub trait Provider {
    /// Its name, as documents write it.
    fn name(&self) -> String;
    /// Whether it sends content off the machine.
    fn cloud(&self) -> bool {
        false
    }
    /// Makes the result at `req.output`.
    fn run(&self, req: &Request) -> Result<Response, String>;
}

/// Files that determine a built-in local model's output, using the adapter's own lookup rules.
pub(crate) fn model_files(req: &Request) -> Vec<PathBuf> {
    let model = match req.provider.as_str() {
        "piper" => piper::voice(req).ok(),
        "whisper" => {
            let exe = tool("SR_WHISPER", "whisper-cli").unwrap_or_else(|| PathBuf::from("whisper-cli"));
            whisper::model_file(req, &exe).ok()
        }
        _ => None,
    };
    let Some(model) = model else { return Vec::new() };
    let mut files = vec![model.clone()];
    if req.provider == "piper" {
        let mut config = model.into_os_string();
        config.push(".json");
        let config = PathBuf::from(config);
        if config.is_file() {
            files.push(config);
        }
    }
    files
}

/// An external provider program.
pub struct External {
    name: String,
    command: Vec<String>,
}

impl Provider for External {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn run(&self, req: &Request) -> Result<Response, String> {
        let mut child = Command::new(&self.command[0])
            .args(&self.command[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run provider {} ({}): {e}", self.name, self.command[0]))?;
        let body = serde_json::to_vec(req).expect("serialisable");
        child.stdin.take().expect("piped").write_all(&body).map_err(|e| e.to_string())?;
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let last = stdout.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
        let resp: Option<Response> = serde_json::from_str(last).ok();
        match resp {
            Some(r) if r.ok && out.status.success() => Ok(r),
            Some(r) if !r.ok => Err(r.error.unwrap_or_else(|| "failed".into())),
            _ => Err(format!(
                "provider {} exited with {} without a JSON answer: {}",
                self.name,
                out.status,
                tail(&String::from_utf8_lossy(&out.stderr))
            )),
        }
    }
}

/// The last lines of a program's error output.
pub(crate) fn tail(s: &str) -> String {
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(3)..].join(" | ")
}

/// Finds an executable on `PATH`.
pub(crate) fn which(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.components().count() > 1 {
        return p.is_file().then(|| p.to_path_buf());
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).map(|d| d.join(name)).find(|c| {
            use std::os::unix::fs::PermissionsExt;
            c.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
        })
    })
}

/// A program from an environment variable, else `PATH`.
pub(crate) fn tool(env: &str, default: &str) -> Option<PathBuf> {
    match std::env::var(env) {
        Ok(v) if !v.is_empty() => which(&v).or_else(|| Some(PathBuf::from(v))),
        _ => which(default),
    }
}

/// Runs a command and returns its standard output, or its error output on failure.
pub(crate) fn run(cmd: &mut Command, what: &str) -> Result<String, String> {
    let out = cmd.stdin(Stdio::null()).output().map_err(|e| format!("cannot run {what}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{what} failed ({}): {}", out.status, tail(&String::from_utf8_lossy(&out.stderr))));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Puts `src` at `dst`: copied when the formats (extensions) agree, else converted by FFmpeg.
pub(crate) fn convert(src: &Path, dst: &Path) -> Result<(), String> {
    let ext = |p: &Path| p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
    if ext(src) == ext(dst) {
        return std::fs::copy(src, dst).map(|_| ()).map_err(|e| format!("{}: {e}", dst.display()));
    }
    run(
        Command::new(sr_media::ffmpeg())
            .args(["-v", "error", "-nostdin", "-y", "-i"])
            .arg(src)
            .args(["-frames:v", "1"])
            .arg(dst),
        "FFmpeg",
    )
    .map(|_| ())
}

/// The provider called `name`.
pub fn find(name: &str) -> Result<Box<dyn Provider>, String> {
    let env = format!("SR_PROVIDER_{}", name.to_ascii_uppercase().replace('-', "_"));
    if let Ok(cmd) = std::env::var(&env) {
        let command: Vec<String> = cmd.split_whitespace().map(str::to_string).collect();
        if !command.is_empty() {
            return Ok(Box::new(External { name: name.into(), command }));
        }
    }
    if let Some(p) = which(&format!("scene-render-provider-{name}")) {
        return Ok(Box::new(External { name: name.into(), command: vec![p.display().to_string()] }));
    }
    Ok(match name {
        "whisper" => Box::new(whisper::Whisper),
        "piper" => Box::new(piper::Piper),
        "audioforge" => Box::new(audioforge::AudioForge),
        "openai" => Box::new(cloud::OpenAi),
        "elevenlabs" => Box::new(cloud::ElevenLabs),
        "tiles" => Box::new(tiles::Tiles),
        other => {
            return Err(format!(
                "unknown provider {other:?}: install a `scene-render-provider-{other}` program, set {env}, \
                 or use whisper, piper, audioforge, openai or elevenlabs"
            ))
        }
    })
}

/// Splits a prompt like a shell: whitespace separates, quotes group (`key='A minor'` → `key=A minor`).
pub(crate) fn shell_split(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote = None;
    let mut any = false;
    for c in s.chars() {
        match (quote, c) {
            (None, '"' | '\'') => {
                quote = Some(c);
                any = true;
            }
            (Some(q), c) if c == q => quote = None,
            (None, c) if c.is_whitespace() => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (_, c) => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out
}

/// Wraps 16-bit little-endian PCM in a WAV header.
pub(crate) fn wav_from_pcm16(pcm: &[u8], rate: u32, channels: u16) -> Vec<u8> {
    let mut v = Vec::with_capacity(44 + pcm.len());
    let block = channels * 2;
    v.extend(b"RIFF");
    v.extend((36 + pcm.len() as u32).to_le_bytes());
    v.extend(b"WAVEfmt ");
    v.extend(16u32.to_le_bytes());
    v.extend(1u16.to_le_bytes());
    v.extend(channels.to_le_bytes());
    v.extend(rate.to_le_bytes());
    v.extend((rate * block as u32).to_le_bytes());
    v.extend(block.to_le_bytes());
    v.extend(16u16.to_le_bytes());
    v.extend(b"data");
    v.extend((pcm.len() as u32).to_le_bytes());
    v.extend(pcm);
    v
}

/// Standard base64 (RFC 4648) decoding, ignoring whitespace.
pub(crate) fn base64(s: &str) -> Result<Vec<u8>, String> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut n) = (0u32, 0);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        acc = (acc << 6) | val(c).ok_or("invalid base64")?;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((acc >> n) as u8);
            acc &= (1 << n) - 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_split_like_a_shell() {
        assert_eq!(shell_split("duration=auto bpm=96 key='A minor'"), ["duration=auto", "bpm=96", "key=A minor"]);
        assert_eq!(shell_split("  a \"\" b"), ["a", "", "b"]);
    }

    #[test]
    fn base64_decodes() {
        assert_eq!(base64("aGVsbG8gd29ybGQ=").unwrap(), b"hello world");
        assert_eq!(base64("AAEC\n/w==").unwrap(), [0, 1, 2, 255]);
        assert!(base64("a*b").is_err());
    }
}
