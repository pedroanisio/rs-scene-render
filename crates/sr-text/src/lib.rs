//! # sr-text
//!
//! Typography and data graphics. Text is shaped with rustybuzz, ordered
//! with the Unicode bidirectional algorithm, broken into lines at UAX #14
//! opportunities (with Knuth–Liang hyphenation for English) and laid out
//! into a box with alignment, autoFit, line limits and vertical writing.
//! Glyphs become `sr_vector` paths, so text stays sharp under any
//! transform and renders through the same rasteriser as shapes; colour
//! glyphs come from COLR v0/v1 (vector) or CBDT/sbix (bitmaps). Text
//! animators, text on a path, captions, charts, audiograms, machine-readable
//! codes and formulas all produce the same [`Drawing`].

pub mod animate;
pub mod audiogram;
pub mod captions;
pub mod chart;
pub mod code;
pub mod extrusion;
pub mod font;
pub mod formula;
pub mod glyph;
pub mod layout;
pub mod style;

pub use font::FontLib;
pub use glyph::{Bitmap, Drawing};
pub use layout::{Layout, Opts, Para};
pub use style::Style;
