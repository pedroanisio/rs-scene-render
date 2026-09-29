//! The provider protocol: one JSON request on a provider's standard input,
//! one JSON response as the last line of its standard output.
//!
//! A provider writes the requested media to `output` (the extension names
//! the format the document's cache expects) and answers
//! `{"ok": true, "version": "…"}`, or `{"ok": false, "error": "…"}`. A
//! transcriber reads `input` (a mono WAV of the track as it plays, starting
//! at composition time 0) and writes a JSON transcript with times in seconds:
//! `{"segments": [{"start", "end", "text", "words": [{"start", "end", "text"}]}]}`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The protocol version.
pub const PROTOCOL: u32 = 1;

/// What a request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Task {
    /// Make media from a prompt.
    Generate,
    /// Transcribe `input`.
    Transcribe,
}

/// Where the asset plays: lets providers size media to the timeline (`duration=auto`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    /// The project's duration in seconds.
    pub project_duration: f64,
    /// Composition time at which the asset's first audio track plays its start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
}

/// A request to a provider.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub protocol: u32,
    pub task: Option<Task>,
    /// `image`, `video`, `speech`, `music`, `sound-effect` or `captions`.
    pub kind: String,
    /// Element id in the document.
    pub id: String,
    pub provider: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// The scene's audio format (`audioMix`).
    pub sample_rate: u32,
    pub bit_depth: u16,
    pub timeline: Timeline,
    /// Transcription input (mono WAV).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// SHA-256 of the input's bytes (part of the request key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_sha256: Option<String>,
    /// Where to write the result.
    pub output: String,
    /// A scratch directory the provider may use.
    pub workdir: String,
    /// The document's directory (relative model paths resolve against it).
    pub base_dir: String,
}

impl Request {
    /// The request key: SHA-256 of the request without its paths and id, so the same request
    /// in another document or at another cache path finds the same result.
    pub fn key(&self) -> String {
        let mut r = self.clone();
        r.id.clear();
        r.output = std::path::Path::new(&self.output)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy().to_ascii_lowercase()))
            .unwrap_or_default();
        r.workdir.clear();
        r.base_dir.clear();
        r.input = None;
        hex(&Sha256::digest(serde_json::to_vec(&r).expect("serialisable")))
    }
}

/// A provider's answer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Lower-case hex.
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// SHA-256 of a file, hex.
pub fn file_sha256(path: &std::path::Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_ignore_paths_and_ids_but_not_content() {
        let a = Request {
            protocol: PROTOCOL,
            kind: "speech".into(),
            id: "a".into(),
            provider: "piper".into(),
            model: "m".into(),
            prompt: Some("hi".into()),
            output: "/x/a.wav".into(),
            workdir: "/tmp/1".into(),
            ..Default::default()
        };
        let b = Request { id: "b".into(), output: "/y/b.WAV".into(), workdir: "/tmp/2".into(), ..a.clone() };
        assert_eq!(a.key(), b.key());
        assert_ne!(a.key(), Request { prompt: Some("hello".into()), ..a.clone() }.key());
        assert_ne!(a.key(), Request { output: "/x/a.mp3".into(), ..a.clone() }.key());
    }
}
