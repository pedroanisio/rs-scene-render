//! Deterministic simulation for scene-render.
//!
//! * [`physics`]: Rapier 2D rigid bodies, joints and bounds, plus
//!   spring-mass soft bodies (jelly, cloth, rope), stepped at a fixed rate
//!   with a checkpoint every simulated second, so any time is reached by
//!   replaying at most one second of steps.
//! * [`particles`]: emitters with a structure-of-arrays particle store,
//!   checkpointed the same way.
//! * [`fields`]: force fields shared by bodies and particles.
//!
//! Everything works in document pixels with y down; physics converts to
//! Rapier's metres with y up through `pixelsPerMeter`. Randomness is a
//! counter-based hash of seeds and indices, so results do not depend on the
//! order in which frames are requested.

// `is_multiple_of` (Rust 1.87) is newer than the workspace's Rust 1.82.
#![allow(clippy::manual_is_multiple_of)]

pub mod fields;
pub mod particles;
pub mod physics;
pub mod rng;
pub mod soft;

pub use fields::{Field, FieldKind};
