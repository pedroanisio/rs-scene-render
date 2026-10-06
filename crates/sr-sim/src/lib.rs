//! Deterministic simulation for scene-render.
//!
//! * [`physics`]: Rapier 2D rigid bodies, joints and bounds, plus
//!   spring-mass soft bodies (jelly, cloth, rope), stepped at a fixed rate
//!   with a checkpoint every simulated second, so any time is reached by
//!   replaying at most one second of steps.
//! * [`particles`]: emitters with a structure-of-arrays particle store,
//!   checkpointed the same way.
//! * [`fields`]: force fields shared by bodies and particles.
//! * [`flock`], [`fluid`], [`slime`] and [`erosion`]: boids, stable fluids, Physarum networks
//!   and hydraulic erosion, stepped on a [`timeline`] with checkpoints.
//!
//! Everything works in document pixels with y down; physics converts to
//! Rapier's metres with y up through `pixelsPerMeter`. Randomness is a
//! counter-based hash of seeds and indices, so results do not depend on the
//! order in which frames are requested.

#![allow(clippy::manual_is_multiple_of)]

pub mod cratering;
pub mod erosion;
pub mod exchange;
pub mod fields;
pub mod flock;
pub mod fluid;
pub mod gr;
pub mod hydrostatics;
pub mod ocean;
pub mod particles;
pub mod particles3d;
pub mod physics;
pub mod physics3d;
pub mod pyro;
pub mod rng;
pub mod slime;
pub mod soft;
pub mod surface;
pub mod timeline;

pub use fields::{Field, FieldKind};
