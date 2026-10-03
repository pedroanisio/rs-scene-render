//! Deterministic volume-cache timing and missing-file policies.
//!
//! Selection depends on local scene time, never on the previously rendered frame.
//! The caller resolves filenames and uses a bounded decoder. A loader returns
//! `Ok(None)` only for a missing file; malformed or inaccessible files are errors.
//! At most one million frame labels are permitted. Hold searches preceding frames
//! within that range, and never substitutes a future frame at the start.

use std::sync::Arc;

use crate::{Error, Volume};

type ResolvedFrame = (i64, Option<Arc<Volume>>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Hold,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingFrame {
    Error,
    Hold,
    Transparent,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub first: i64,
    pub second: i64,
    pub blend: f64,
}

/// Immutable frames to sample in their own world spaces before temporal blending.
/// None is a transparent frame (zero density and zero other fields).
#[derive(Clone, Debug)]
pub struct FramePair {
    pub first: Option<Arc<Volume>>,
    pub second: Option<Arc<Volume>>,
    pub blend: f64,
}

/// Endpoint fields plus target-minus-source times in seconds for characteristic
/// tracing. Held files retain their actual frame labels. Identical resolved
/// labels, hold interpolation and clamped endpoints freeze with zero elapsed time.
#[derive(Clone, Debug)]
pub struct TimedFramePair {
    pub frames: FramePair,
    pub elapsed: [f64; 2],
}

impl FramePair {
    /// Samples a named scalar field in each cache's asset-world coordinates, then
    /// interpolates the values. Density and kelvin must be blended this way before
    /// evaluating nonlinear transport or blackbody emission. Missing channels in
    /// an existing frame are errors, while a transparent missing frame contributes zero.
    pub fn sample(&self, channel: &str, world: [f64; 3]) -> Result<f64, Error> {
        if !self.blend.is_finite() || !(0.0..=1.0).contains(&self.blend) || world.iter().any(|v| !v.is_finite()) {
            return Err(Error::Invalid("sequence sample requires finite coordinates and blend in [0,1]"));
        }
        let sample = |frame: &Option<Arc<Volume>>| -> Result<f64, Error> {
            match frame {
                None => Ok(0.0),
                Some(frame) => frame
                    .grid(channel)
                    .map(|g| f64::from(g.sample_world(world)))
                    .ok_or(Error::Invalid("selected volume sequence channel is missing")),
            }
        };
        Ok(sample(&self.first)? * (1.0 - self.blend) + sample(&self.second)? * self.blend)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Sequence {
    first: i64,
    last: i64,
    fps: f64,
    interpolation: Interpolation,
    missing: MissingFrame,
}

impl Sequence {
    pub fn new(
        first: i64,
        last: i64,
        fps: f64,
        interpolation: Interpolation,
        missing: MissingFrame,
    ) -> Result<Self, Error> {
        if !fps.is_finite() || fps <= 0.0 {
            return Err(Error::Invalid("sequence fps must be positive and finite"));
        }
        let count = last.checked_sub(first).and_then(|n| n.checked_add(1));
        if !count.is_some_and(|n| (1..=1_000_000).contains(&n)) {
            return Err(Error::Limit("sequence requires 1..1000000 ordered frames"));
        }
        Ok(Self { first, last, fps, interpolation, missing })
    }

    /// Time is local seconds after object start and any authored time remapping.
    /// Both ends clamp. Integral labels stay integers even beyond f64 precision.
    pub fn position(&self, time: f64) -> Result<Position, Error> {
        if !time.is_finite() {
            return Err(Error::Invalid("sequence time must be finite"));
        }
        let offset = (time * self.fps).clamp(0.0, (self.last - self.first) as f64);
        let first = self.first + offset.floor() as i64;
        let blend = offset.fract();
        if self.interpolation == Interpolation::Hold || blend == 0.0 || first == self.last {
            Ok(Position { first, second: first, blend: 0.0 })
        } else {
            Ok(Position { first, second: first + 1, blend })
        }
    }

    /// Loads a pair within an estimated resident-byte budget, sharing identical
    /// frames. The loader must independently bound allocations while decoding.
    /// Missing-frame searches take at most the bounded sequence length plus one
    /// loader call, even if both selected files are absent.
    pub fn load(
        &self,
        time: f64,
        max_bytes: usize,
        loader: impl FnMut(i64) -> Result<Option<Arc<Volume>>, Error>,
    ) -> Result<FramePair, Error> {
        self.load_resolved(time, max_bytes, loader).map(|(frames, _)| frames)
    }

    /// Like `load`, preserving the actual source times needed by motion-aware
    /// sampling. Missing transparent frames keep their nominal time. If a pair
    /// resolves to the same held label it freezes, rather than extrapolating it.
    pub fn load_timed(
        &self,
        time: f64,
        max_bytes: usize,
        loader: impl FnMut(i64) -> Result<Option<Arc<Volume>>, Error>,
    ) -> Result<TimedFramePair, Error> {
        let p = self.position(time)?;
        let (frames, labels) = self.load_resolved(time, max_bytes, loader)?;
        let elapsed = if labels[0] == labels[1] || p.first == p.second {
            [0.; 2]
        } else {
            labels.map(|label| ((p.first - label) as f64 + p.blend) / self.fps)
        };
        if elapsed.iter().any(|v| !v.is_finite()) {
            return Err(Error::Invalid("volume frame interval exceeds finite seconds"));
        }
        Ok(TimedFramePair { frames, elapsed })
    }

    fn load_resolved(
        &self,
        time: f64,
        max_bytes: usize,
        mut loader: impl FnMut(i64) -> Result<Option<Arc<Volume>>, Error>,
    ) -> Result<(FramePair, [i64; 2]), Error> {
        let p = self.position(time)?;
        let first = self.resolve(p.first, &mut loader, None)?;
        let first_bytes = first.1.as_ref().map_or(0, |f| f.bytes());
        if first_bytes > max_bytes {
            return Err(Error::Limit("volume sequence frame bytes"));
        }
        let second = if p.second == p.first {
            first.clone()
        } else {
            self.resolve(p.second, &mut loader, Some((p.first, &first)))?
        };
        let shared = matches!((&first.1, &second.1), (Some(a), Some(b)) if Arc::ptr_eq(a,b));
        let second_bytes = if shared { 0 } else { second.1.as_ref().map_or(0, |f| f.bytes()) };
        if first_bytes.checked_add(second_bytes).is_none_or(|n| n > max_bytes) {
            return Err(Error::Limit("volume sequence frame pair bytes"));
        }
        Ok((FramePair { first: first.1, second: second.1, blend: p.blend }, [first.0, second.0]))
    }

    fn resolve(
        &self,
        mut frame: i64,
        loader: &mut impl FnMut(i64) -> Result<Option<Arc<Volume>>, Error>,
        prior: Option<(i64, &ResolvedFrame)>,
    ) -> Result<ResolvedFrame, Error> {
        loop {
            if let Some((label, result)) = prior {
                if frame == label {
                    return Ok(result.clone());
                }
            }
            if let Some(volume) = loader(frame)? {
                return Ok((frame, Some(volume)));
            }
            match self.missing {
                MissingFrame::Error => return Err(Error::Invalid("missing volume sequence frame")),
                MissingFrame::Transparent => return Ok((frame, None)),
                MissingFrame::Hold if frame > self.first => frame -= 1,
                MissingFrame::Hold => {
                    return Err(Error::Invalid("missing volume sequence has no preceding frame to hold"))
                }
            }
        }
    }
}
