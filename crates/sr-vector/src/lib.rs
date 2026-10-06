//! # sr-vector
//!
//! The geometry kernel every path-based feature shares: shapes, SVG and
//! Lottie imports, path masks, strokes and deformers all become [`Path`]s,
//! are flattened to polylines in f64, and reach the GPU through one tile
//! encoder ([`tile`]). Rigging (bones, constraints, IK), puppet
//! deformation (as-rigid-as-possible) and tracking data live here too,
//! because they are geometry evaluated on the CPU.

pub mod arap;
pub mod d24;
pub mod deform;
pub mod geom;
pub mod lottie;
pub mod markers;
pub mod measure;
pub mod modifiers;
pub mod path;
pub mod rig;
pub mod scene;
pub mod shapes;
pub mod stroke;
pub mod svg;
pub mod tile;
pub mod track;
pub mod zip;

static THREADS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Caps the worker threads of the tile encoder (0 restores the default: the machine's
/// parallelism, at most 16). Results do not depend on the count.
pub fn set_threads(n: usize) {
    THREADS.store(n, std::sync::atomic::Ordering::Relaxed);
}

/// Worker threads the tile encoder uses.
pub fn threads() -> usize {
    match THREADS.load(std::sync::atomic::Ordering::Relaxed) {
        0 => std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(16),
        n => n,
    }
}

pub use geom::{Xf, P};
pub use path::{Contour, Path, Poly, Seg};
pub use scene::{Cmd, FillRule, Gradient, GradientKind, MaskOp, MatteMode, Paint, Scene};
