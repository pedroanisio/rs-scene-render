//! Cloud providers. They send the prompt to a remote service, so the resolver
//! runs them only with `--allow-cloud`, and they need their API key in the
//! environment. Requests go through `curl` (`SR_CURL`), with the key passed in
//! curl's configuration on standard input, never on a command line.
//!
//! * `openai`: speech (`POST /audio/speech`; `model` such as `gpt-4o-mini-tts`
//!   or `tts-1`, `voice` default `alloy`) and images (`POST /images/generations`;
//!   `model` such as `gpt-image-1`, size from `width`×`height`). Key
//!   `OPENAI_API_KEY`, base URL `OPENAI_BASE_URL`.
//! * `elevenlabs`: speech (`POST /text-to-speech/<voice>`, `voice` is the voice
//!   id, `model` the model id, `seed` passed through). Key `ELEVENLABS_API_KEY`,
//!   base URL `ELEVENLABS_BASE_URL`.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{base64, convert, wav_from_pcm16, Provider};
use crate::protocol::{Request, Response};

fn key(var: &str) -> Result<String, String> {
    std::env::var(var).ok().filter(|k| !k.is_empty()).ok_or_else(|| format!("{var} is not set"))
}

fn base(var: &str, default: &str) -> String {
    std::env::var(var)
        .ok()
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| default.into())
        .trim_end_matches('/')
        .to_string()
}

/// POSTs `body` (JSON) to `url` with `headers`, writing the response to `out`; returns the
/// HTTP status.
fn post(url: &str, headers: &[(&str, String)], body: &Value, work: &Path, out: &Path) -> Result<u16, String> {
    let req_file = work.join("request.json");
    std::fs::write(&req_file, serde_json::to_vec(body).expect("json")).map_err(|e| e.to_string())?;
    let q = super::curl_quote;
    let mut cfg = String::new();
    cfg += &format!("url = {}\nrequest = \"POST\"\nsilent\nshow-error\n", q(url));
    cfg += "header = \"Content-Type: application/json\"\n";
    for (k, v) in headers {
        cfg += &format!("header = {}\n", q(&format!("{k}: {v}")));
    }
    cfg += &format!(
        "data-binary = {}\noutput = {}\nwrite-out = \"%{{http_code}}\"\n",
        q(&format!("@{}", req_file.display())),
        q(&out.display().to_string())
    );
    super::curl(&cfg)
}

/// The service's error message from a failed response body.
fn failure(service: &str, status: u16, body: &Path) -> String {
    // Remote bodies may reflect credentials or private prompts. Report the
    // status without reading or logging untrusted response content.
    let _ = body;
    format!("{service} answered HTTP {status}")
}

pub struct OpenAi;

impl Provider for OpenAi {
    fn name(&self) -> String {
        "openai".into()
    }
    fn cloud(&self) -> bool {
        true
    }

    fn run(&self, req: &Request) -> Result<Response, String> {
        let k = key("OPENAI_API_KEY")?;
        let url = base("OPENAI_BASE_URL", "https://api.openai.com/v1");
        let auth = [("Authorization", format!("Bearer {k}"))];
        let work = PathBuf::from(&req.workdir);
        let prompt = req.prompt.clone().filter(|p| !p.trim().is_empty()).ok_or("openai needs @prompt")?;
        let body_file = work.join("openai.out");
        match req.kind.as_str() {
            "speech" => {
                let body = json!({"model": req.model, "input": prompt, "voice": req.voice.as_deref().unwrap_or("alloy"), "response_format": "wav"});
                let status = post(&format!("{url}/audio/speech"), &auth, &body, &work, &body_file)?;
                if status != 200 {
                    return Err(failure("OpenAI", status, &body_file));
                }
                let wav = work.join("openai.wav");
                std::fs::rename(&body_file, &wav).map_err(|e| e.to_string())?;
                convert(&wav, &PathBuf::from(&req.output))?;
            }
            "image" => {
                let size = match (req.width, req.height) {
                    (Some(w), Some(h)) => format!("{w}x{h}"),
                    _ => "1024x1024".into(),
                };
                let mut body = json!({"model": req.model, "prompt": prompt, "n": 1, "size": size});
                if req.model.starts_with("dall-e") {
                    body["response_format"] = json!("b64_json");
                }
                let status = post(&format!("{url}/images/generations"), &auth, &body, &work, &body_file)?;
                if status != 200 {
                    return Err(failure("OpenAI", status, &body_file));
                }
                let v: Value = serde_json::from_slice(&std::fs::read(&body_file).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("OpenAI answer: {e}"))?;
                let b64 = v["data"][0]["b64_json"].as_str().ok_or("OpenAI answer has no data[0].b64_json")?;
                let png = work.join("openai.png");
                std::fs::write(&png, base64(b64)?).map_err(|e| e.to_string())?;
                convert(&png, &PathBuf::from(&req.output))?;
            }
            other => return Err(format!("openai makes speech and images, not {other}")),
        }
        Ok(Response { ok: true, version: Some(format!("openai {}", req.model)), ..Default::default() })
    }
}

pub struct ElevenLabs;

impl Provider for ElevenLabs {
    fn name(&self) -> String {
        "elevenlabs".into()
    }
    fn cloud(&self) -> bool {
        true
    }

    fn run(&self, req: &Request) -> Result<Response, String> {
        if req.kind != "speech" {
            return Err(format!("elevenlabs makes speech, not {}", req.kind));
        }
        let k = key("ELEVENLABS_API_KEY")?;
        let url = base("ELEVENLABS_BASE_URL", "https://api.elevenlabs.io/v1");
        let voice = req.voice.as_deref().ok_or("elevenlabs needs @voice (a voice id)")?;
        let prompt = req.prompt.clone().filter(|p| !p.trim().is_empty()).ok_or("elevenlabs needs @prompt")?;
        let work = PathBuf::from(&req.workdir);
        let mut body = json!({"text": prompt, "model_id": req.model});
        if let Some(s) = req.seed {
            body["seed"] = json!(s % 4_294_967_296);
        }
        if let Some(l) = &req.language {
            body["language_code"] = json!(l.split(['-', '_']).next().unwrap_or(l));
        }
        let raw = work.join("elevenlabs.pcm");
        let status = post(
            &format!("{url}/text-to-speech/{voice}?output_format=pcm_44100"),
            &[("xi-api-key", k)],
            &body,
            &work,
            &raw,
        )?;
        if status != 200 {
            return Err(failure("ElevenLabs", status, &raw));
        }
        let wav = work.join("elevenlabs.wav");
        std::fs::write(&wav, wav_from_pcm16(&std::fs::read(&raw).map_err(|e| e.to_string())?, 44100, 1))
            .map_err(|e| e.to_string())?;
        convert(&wav, &PathBuf::from(&req.output))?;
        Ok(Response { ok: true, version: Some(format!("elevenlabs {}", req.model)), ..Default::default() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_errors_do_not_echo_credentials_or_unbounded_bodies() {
        let work = tempfile::tempdir().unwrap();
        let body = work.path().join("body");
        std::fs::write(
            &body,
            serde_json::to_vec(&json!({"error":{"message":format!("Bearer dummy-secret {}", "x".repeat(100000))}}))
                .unwrap(),
        )
        .unwrap();
        let error = failure("OpenAI", 400, &body);
        assert!(!error.contains("dummy-secret"), "{error}");
        assert!(error.len() < 1024);
    }

    #[cfg(unix)]
    #[test]
    fn curl_disables_user_config_and_keeps_error_stream_private() {
        use std::os::unix::fs::PermissionsExt;
        let work = tempfile::tempdir().unwrap();
        let curl = work.path().join("curl");
        std::fs::write(&curl, "#!/usr/bin/env python3\nimport sys,pathlib\nconfig=sys.stdin.read()\npathlib.Path(__file__).with_suffix('.args').write_text(' '.join(sys.argv[1:]))\nsys.stderr.write(config)\nsys.exit(1)\n").unwrap();
        std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("SR_CURL", &curl);
        let result = post(
            "https://example.invalid/?token=dummy-url-secret",
            &[("Authorization", "Bearer dummy-api-secret".into())],
            &json!({}),
            work.path(),
            &work.path().join("out"),
        );
        std::env::remove_var("SR_CURL");
        let error = result.unwrap_err();
        assert!(!error.contains("dummy-"), "{error}");
        assert!(std::fs::read_to_string(curl.with_extension("args")).unwrap().starts_with("--disable "));
    }
}
