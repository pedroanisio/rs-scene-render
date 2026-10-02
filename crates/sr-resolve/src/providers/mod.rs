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
        self.run_with_timeout(req, provider_timeout()?)
    }
}

impl External {
    fn run_with_timeout(&self, req: &Request, timeout: std::time::Duration) -> Result<Response, String> {
        let mut command = Command::new(&self.command[0]);
        command.args(&self.command[1..]);
        let out = execute(&mut command, &self.name, &serde_json::to_vec(req).expect("serialisable"), timeout)?;
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

fn provider_timeout() -> Result<std::time::Duration, String> {
    let seconds = std::env::var("SR_PROVIDER_TIMEOUT")
        .ok()
        .map(|s| s.parse::<u64>().map_err(|_| "SR_PROVIDER_TIMEOUT must be positive seconds".to_string()))
        .transpose()?
        .unwrap_or(1800);
    if seconds == 0 {
        return Err("SR_PROVIDER_TIMEOUT must be positive seconds".into());
    }
    Ok(std::time::Duration::from_secs(seconds))
}

thread_local! {
    static DEADLINE: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

/// One budget for all subprocesses/retries making a target, including conversions.
pub(crate) fn invoke(provider: &dyn Provider, req: &Request) -> Result<Response, String> {
    struct Reset(Option<std::time::Instant>);
    impl Drop for Reset {
        fn drop(&mut self) {
            DEADLINE.set(self.0);
        }
    }
    let deadline = std::time::Instant::now().checked_add(provider_timeout()?).ok_or("provider timeout is too large")?;
    let _reset = Reset(DEADLINE.replace(Some(deadline)));
    provider.run(req)
}

/// Concurrent, bounded pipe draining and request writing under one deadline.
fn execute(
    command: &mut Command,
    what: &str,
    body: &[u8],
    timeout: std::time::Duration,
) -> Result<std::process::Output, String> {
    let timeout =
        DEADLINE.get().map_or(timeout, |d| timeout.min(d.saturating_duration_since(std::time::Instant::now())));
    if timeout.is_zero() {
        return Err(format!("{what} timed out"));
    }
    use std::io::Read;
    use std::sync::mpsc;
    use std::time::Instant;
    // Drain both pipes while writing stdin. Keep only the response/error tail,
    // so a noisy provider cannot fill a pipe or grow our memory indefinitely.
    fn drain(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<std::io::Result<Vec<u8>>> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let mut tail = std::collections::VecDeque::new();
                let mut buf = [0; 8192];
                loop {
                    let n = pipe.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    let excess = (tail.len() + n).saturating_sub(1024 * 1024);
                    tail.drain(..excess);
                    tail.extend(&buf[..n]);
                }
                Ok(tail.into_iter().collect())
            })();
            let _ = tx.send(result);
        });
        rx
    }
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|e| format!("cannot run {what}: {e}"))?;
    let stdout = drain(child.stdout.take().expect("piped"));
    let stderr = drain(child.stderr.take().expect("piped"));
    let mut stdin = child.stdin.take().expect("piped");
    let body = body.to_vec();
    let (tx, written) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(stdin.write_all(&body));
    });
    let start = Instant::now();
    let result = (|| {
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            if start.elapsed() >= timeout {
                return Err(format!("{what} timed out"));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let remaining = || timeout.saturating_sub(start.elapsed());
        let stdout = stdout
            .recv_timeout(remaining())
            .map_err(|_| "provider stdout timed out".to_string())?
            .map_err(|e| e.to_string())?;
        let stderr = stderr
            .recv_timeout(remaining())
            .map_err(|_| "provider stderr timed out".to_string())?
            .map_err(|e| e.to_string())?;
        written
            .recv_timeout(remaining())
            .map_err(|_| "provider stdin timed out".to_string())?
            .map_err(|e| format!("provider request: {e}"))?;
        Ok(std::process::Output { status, stdout, stderr })
    })();
    if result.is_err() {
        #[cfg(unix)]
        {
            let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", child.id())]).status();
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

/// Curl deliberately omits untrusted stderr: it can reflect headers and signed URLs.
/// `--disable` must be the first argument to ignore inherited curl configuration.
pub(crate) fn curl(config: &str) -> Result<u16, String> {
    let exe = std::env::var("SR_CURL").unwrap_or_else(|_| "curl".into());
    let output = execute(
        Command::new(exe).args(["--disable", "--max-filesize", "268435456", "--config", "-"]),
        "curl",
        config.as_bytes(),
        provider_timeout()?,
    )?;
    if !output.status.success() {
        return Err(format!("curl failed ({})", output.status));
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().map_err(|_| "curl gave no HTTP status".into())
}

/// Quote a curl config value without allowing line injection.
pub(crate) fn curl_quote(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\").replace('\"', "\\\"").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
    )
}

/// The last lines of a program's error output.
pub(crate) fn tail(s: &str) -> String {
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(3)..].join(" | ")
}

/// Finds an executable on `PATH`.
pub(crate) fn which(name: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    let extensions = Some(std::env::var_os("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into()));
    #[cfg(not(windows))]
    let extensions: Option<std::ffi::OsString> = None;
    which_in(name, std::env::var_os("PATH").as_deref(), extensions.as_deref())
}

fn which_in(name: &str, paths: Option<&std::ffi::OsStr>, extensions: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let p = Path::new(name);
    let dirs = if p.components().count() > 1 { vec![PathBuf::new()] } else { std::env::split_paths(paths?).collect() };
    for dir in dirs {
        let candidate = dir.join(p);
        if let Some(extensions) = extensions {
            if candidate.is_file() {
                return Some(candidate);
            }
            for ext in extensions.to_string_lossy().split(';').map(str::trim).filter(|e| !e.is_empty()) {
                let mut name = candidate.as_os_str().to_os_string();
                name.push(ext);
                let file = PathBuf::from(name);
                if file.is_file() {
                    return Some(file);
                }
            }
        } else {
            #[cfg(unix)]
            let executable = {
                use std::os::unix::fs::PermissionsExt;
                candidate.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            };
            #[cfg(not(unix))]
            let executable = candidate.is_file();
            if executable {
                return Some(candidate);
            }
        }
    }
    None
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
    let out = execute(cmd, what, &[], provider_timeout()?)?;
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

    #[cfg(unix)]
    #[test]
    fn deadline_covers_blocked_stdin_and_inherited_pipes() {
        for script in [
            "import time;time.sleep(10)",
            "import subprocess,sys;subprocess.Popen([sys.executable,'-c','import time;time.sleep(10)'])",
        ] {
            let start = std::time::Instant::now();
            let result = execute(
                Command::new("python3").args(["-c", script]),
                "blocked provider",
                &vec![b'x'; 262144],
                std::time::Duration::from_millis(100),
            );
            assert!(result.is_err_and(|e| e.contains("timed out")));
            assert!(start.elapsed() < std::time::Duration::from_secs(3));
        }
    }

    #[test]
    fn builtin_execution_honours_deadline_and_bounds_output() {
        const CHILD: &str = "SR_TEST_BUILTIN_EXECUTION";
        if std::env::var_os(CHILD).is_some() {
            let start = std::time::Instant::now();
            let result = run(Command::new("python3").args(["-c", "import time; time.sleep(3)"]), "sleeping tool");
            assert!(result.is_err_and(|e| e.contains("timed out")));
            assert!(start.elapsed() < std::time::Duration::from_secs(2));
            let out = run(Command::new("python3").args(["-c", "print('x'*3000000)"]), "noisy tool").unwrap();
            assert!(out.len() <= 1024 * 1024);
            return;
        }
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "providers::tests::builtin_execution_honours_deadline_and_bounds_output", "--nocapture"])
            .env(CHILD, "1")
            .env("SR_PROVIDER_TIMEOUT", "1")
            .output()
            .unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
    }

    #[test]
    fn test_external_provider_timeout_reaps_the_child() {
        let provider = External {
            name: "hung".into(),
            command: vec!["python3".into(), "-c".into(), "import time;time.sleep(60)".into()],
        };
        let start = std::time::Instant::now();
        let result = provider.run_with_timeout(&Request::default(), std::time::Duration::from_millis(100));
        assert!(result.unwrap_err().contains("timed out"));
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
    }

    #[test]
    fn test_external_provider_drains_output_while_sending_request() {
        let provider = External { name: "chatty".into(), command: vec!["python3".into(), "-c".into(),
            "import sys,json;sys.stderr.write('x'*262144);sys.stderr.flush();sys.stdout.write('y'*262144+'\\n');sys.stdout.flush();json.load(sys.stdin);print('{\"ok\":true}')".into()] };
        let request = Request { prompt: Some("p".repeat(262144)), ..Default::default() };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(provider.run(&request));
        });
        assert!(rx.recv_timeout(std::time::Duration::from_secs(5)).expect("provider deadlocked").unwrap().ok);
    }

    #[test]
    fn executable_lookup_honours_windows_pathext() {
        let dir = std::env::temp_dir().join(format!("sr-provider-which-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("scene-render-provider-test.EXE");
        std::fs::write(&exe, b"fixture").unwrap();
        let paths = std::env::join_paths([&dir]).unwrap();
        let ext = Some(std::ffi::OsStr::new(".COM;.EXE;.CMD"));
        assert_eq!(which_in("scene-render-provider-test", Some(&paths), ext), Some(exe.clone()));
        assert_eq!(which_in(exe.to_str().unwrap(), None, ext), Some(exe.clone()));
        assert_eq!(which_in(dir.join("scene-render-provider-test").to_str().unwrap(), None, ext), Some(exe));
        let cmd = dir.join("scene-render-provider-test.CMD");
        std::fs::write(&cmd, b"fixture").unwrap();
        assert_eq!(
            which_in("scene-render-provider-test", Some(&paths), Some(std::ffi::OsStr::new(";.CMD; .EXE;"))),
            Some(cmd)
        );
        assert_eq!(which_in("missing", Some(&paths), ext), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn executable_lookup_requires_unix_execute_permission() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("sr-provider-permissions-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("provider");
        std::fs::write(&exe, b"fixture").unwrap();
        let paths = std::env::join_paths([&dir]).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(which_in("provider", Some(&paths), None), None);
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(which_in("provider", Some(&paths), None), Some(exe));
        assert_eq!(which_in(dir.to_str().unwrap(), None, None), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

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
