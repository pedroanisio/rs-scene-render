//! # sr-deliver
//!
//! Turns a document into finished files: the scene's audio mix and the
//! analysis table expressions read, the render → convert → encode
//! pipeline for each `<output>`, two-pass and file-size-fitted encodes,
//! posters and thumbnails, and post-render delivery to files, S3, Google
//! Cloud Storage, Azure Blob Storage, HTTP PUT, SFTP and webhooks.

pub mod access;
pub mod audio;
pub mod captions;
pub mod ceiling;
pub mod destinations;
pub mod overlay;
pub mod pipeline;
pub mod segment_audio;
pub mod segments;

pub use pipeline::{adhoc_output, deliver, output_uses_3d, Options, Parallel, RenderAdapter, Report};

/// Delivery errors.
#[derive(Debug, thiserror::Error)]
pub enum DeliverError {
    /// The document or its program is invalid.
    #[error("{}", .0.diagnostics.iter().map(|d| format!("{:?}[{}]: {}", d.severity, d.code, d.message)).collect::<Vec<_>>().join("\n"))]
    Document(sr_model::Report),
    /// Media I/O failed.
    #[error(transparent)]
    Media(#[from] sr_media::MediaError),
    /// The mix is inconsistent.
    #[error(transparent)]
    Mix(#[from] sr_audio::MixError),
    /// No GPU.
    #[error(transparent)]
    Gpu(#[from] sr_gpu::GpuError),
    /// Rendering reported unreadable media.
    #[error("frame at {time:.3} s: {message}")]
    Render {
        /// Composition time.
        time: f64,
        /// What failed.
        message: String,
    },
    /// An accessibility check set to `error` failed.
    #[error("accessibility: {0}")]
    Accessibility(String),
    /// The decoded audio is over the master's true-peak ceiling even after the correction.
    #[error("audio ceiling: {0}")]
    AudioCeiling(String),
    /// Upload or notification failed.
    #[error("destination {uri}: {message}")]
    Destination {
        /// Destination URI.
        uri: String,
        /// What failed.
        message: String,
    },
    /// Invalid settings.
    #[error("{0}")]
    Invalid(String),
    /// I/O.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub(crate) fn pipeline_resolve(dir: &std::path::Path, p: &str) -> std::path::PathBuf {
    let p = p.strip_prefix("file://").unwrap_or(p);
    let pb = std::path::PathBuf::from(p);
    if pb.is_absolute() {
        pb
    } else {
        dir.join(pb)
    }
}
