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

pub use geom::{Xf, P};
pub use path::{Contour, Path, Poly, Seg};
pub use scene::{Cmd, FillRule, Gradient, GradientKind, MaskOp, MatteMode, Paint, Scene};
