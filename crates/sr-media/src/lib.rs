//! # sr-media
//!
//! Media I/O through the `ffmpeg` and `ffprobe` executables:
//!
//! * [`probe`] reads stream properties (size, rate, pixel format, colour
//!   tags, rotation, audio layout).
//! * [`VideoDecoder`] delivers frame-accurate planar frames by index, with a
//!   background look-ahead ring so sequential access never waits on seeks.
//! * [`decode_audio`] returns interleaved `f32` samples at a chosen rate.
//! * [`encode`] builds and runs encoder command lines for every codec and
//!   container of the schema, with hardware encoder selection.
//!
//! FFmpeg runs as a child process, so a crashing decoder cannot take the
//! renderer with it, and any FFmpeg 4–7 build works, including builds with
//! NVENC, VideoToolbox, VAAPI, QSV or AMF.

pub mod decode;
pub mod encode;
pub mod probe;
pub mod spherical;

pub use decode::{decode_audio, AudioData, PixelLayout, Plane, VideoDecoder, VideoFrame};
pub use probe::{probe, MediaInfo};

/// Errors from media operations.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    /// `ffmpeg` or `ffprobe` could not be started.
    #[error("cannot run {tool}: {source}; install FFmpeg or set SR_FFMPEG / SR_FFPROBE")]
    Spawn {
        /// Executable.
        tool: String,
        /// Cause.
        source: std::io::Error,
    },
    /// The tool failed.
    #[error("{tool} failed on {path}: {message}")]
    Failed {
        /// Executable.
        tool: String,
        /// Input or output path.
        path: String,
        /// Last lines of its error output.
        message: String,
    },
    /// The media has no stream of the requested kind.
    #[error("{0} has no {1} stream")]
    NoStream(String, &'static str),
    /// Invalid combination of settings.
    #[error("{0}")]
    Invalid(String),
    /// I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The `ffmpeg` executable (`SR_FFMPEG` overrides).
pub fn ffmpeg() -> String {
    std::env::var("SR_FFMPEG").unwrap_or_else(|_| "ffmpeg".into())
}

/// The `ffprobe` executable (`SR_FFPROBE` overrides).
pub fn ffprobe() -> String {
    std::env::var("SR_FFPROBE").unwrap_or_else(|_| "ffprobe".into())
}

/// Last `n` non-empty lines of a tool's error output.
pub(crate) fn tail(stderr: &[u8], n: usize) -> String {
    let s = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}
