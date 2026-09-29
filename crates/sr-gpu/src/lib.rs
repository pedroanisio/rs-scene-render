//! # sr-gpu
//!
//! Renders sr-eval FrameGraphs into linear-light RGBA16F frames with wgpu.

pub mod color;
pub mod fx;
pub mod glsl;
pub mod golden;
pub mod gpu;
pub mod ocio;
pub mod output;
pub mod paint;
pub mod particles;
pub mod pathtrace;
pub mod raster;
pub mod render;
pub mod resources;
pub mod shader;
pub mod text;
pub mod three;
pub mod types;
pub mod vector;
pub mod video;

pub use gpu::{Gpu, GpuError};
pub use render::{Frame, RenderStats, Renderer};

/// The FrameGraph value of a document paint.
pub(crate) fn value_of_paint(p: &sr_model::values::Paint) -> sr_eval::Value {
    use sr_eval::Value;
    use sr_model::values::{Color, Paint};
    match p {
        Paint::Color(Color::Rgba(c)) => Value::Color([c.r as f64, c.g as f64, c.b as f64, c.a as f64]),
        Paint::Color(Color::Token(_)) => Value::Color([0.0; 4]),
        Paint::Ref(r) => Value::PaintRef(r.0.as_str().into()),
    }
}

/// A document paint as a FrameGraph value, with colour tokens left unresolved (transparent).
pub(crate) fn value_of_paint_tok(p: &sr_model::values::Paint) -> sr_eval::Value {
    match p {
        sr_model::values::Paint::Color(sr_model::values::Color::Token(t)) => {
            sr_eval::Value::Str(format!("token:{t}").into())
        }
        other => value_of_paint(other),
    }
}
