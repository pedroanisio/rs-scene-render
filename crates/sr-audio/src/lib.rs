//! # sr-audio
//!
//! Offline audio for scene-render: a mixing graph of tracks, video-layer
//! audio and buses with automation, five fade shapes, ducking and the 16
//! effects; ITU-R BS.1770-4 loudness and true peak with integrated or
//! dynamic normalisation and a look-ahead true-peak limiter; channel
//! layouts from mono to 7.1.4 and third-order ambisonics; the analysis
//! table (band envelopes and beats) that expressions read; WAV output with
//! TPDF dither.
//!
//! Everything is deterministic: the same inputs give the same samples.

// DSP loops index several buffers by sample position
#![allow(clippy::needless_range_loop)]

pub mod analysis;
pub mod dsp;
pub mod effects;
pub mod layout;
pub mod loudness;
pub mod mix;
pub mod wav;

pub use layout::Layout;
pub use loudness::Planar;
pub use mix::{Curve, Duck, FadeCurve, Master, Mix, MixError, Mixed, Node, NodeKind, Normalize, Placement, Source};
