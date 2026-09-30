//! An output's overlay (SREP 13): a symbol drawn over every frame of the output, in output time.
//!
//! The symbol's content is rendered as a composition of its own: the symbol's size (the output's
//! frame when it declares none), the output's duration, and its background (transparent by
//! default), without the document's captions. Each frame is fitted to the output's frame and
//! composited over the placed picture by the output stage, after the layout and before any caption
//! burned in output time.

use std::sync::Arc;

use sr_eval::{EvalOptions, Evaluator};
use sr_gpu::output::{OutputStage, Placement};
use sr_gpu::resources::Tex;
use sr_gpu::{Gpu, Renderer};
use sr_model::model as m;

use crate::DeliverError;

pub struct Overlay {
    ev: Evaluator,
    renderer: Renderer,
    /// The overlay fitted to the output's frame.
    tex: Arc<Tex>,
}

impl Overlay {
    /// The overlay of `output`, or `None` when it names none. `duration` is the output's
    /// duration and `size` its frame size.
    pub fn new(
        doc: &sr_model::Document,
        output: &m::Output,
        duration: f64,
        size: [u32; 2],
        gpu: &Gpu,
    ) -> Result<Option<Overlay>, DeliverError> {
        let Some(id) = &output.overlay else { return Ok(None) };
        let symbol = doc
            .scene
            .symbols
            .as_ref()
            .and_then(|s| s.symbols.iter().find(|s| s.id == *id))
            .ok_or_else(|| DeliverError::Invalid(format!("overlay {id:?}: no such symbol")))?;
        let mut d = doc.clone();
        let s = &mut d.scene;
        s.composition.children = symbol.children.clone();
        s.project.width = symbol.width.unwrap_or(size[0] as u64);
        s.project.height = symbol.height.unwrap_or(size[1] as u64);
        s.project.duration = m::Duration::new(duration).map_err(|e| DeliverError::Invalid(e.to_string()))?;
        s.project.background = symbol.background.clone();
        // captions burn into the picture, not into its overlay
        s.scene360 = None;
        s.captions = None;
        let ev = Evaluator::new(&d, &EvalOptions { variant: output.variant.clone(), ..Default::default() })
            .map_err(DeliverError::Document)?;
        let renderer = Renderer::new(gpu.clone(), ev.program());
        let tex = renderer.texture(size);
        Ok(Some(Overlay { ev, renderer, tex }))
    }

    /// Renders the overlay at output time `t` and fits it to the output's frame; returns the
    /// output-sized texture and what could not be drawn.
    pub fn draw(&mut self, stage: &mut OutputStage, t: f64) -> Result<(&Tex, Vec<String>), DeliverError> {
        let p = self.ev.program();
        let g = self.ev.evaluate(t.clamp(0.0, p.duration));
        let ev = &self.ev;
        let mut sub = |st: f64| ev.evaluate(st.clamp(0.0, p.duration));
        let frame = self.renderer.render_with(&g, p, Some(&mut sub));
        if let Some(e) = frame.stats.errors.first() {
            return Err(DeliverError::Render { time: t, message: format!("overlay: {e}") });
        }
        stage.place(&frame.texture, &self.renderer.working(), &self.tex, Placement::default());
        Ok((&self.tex, frame.stats.unsupported.iter().map(|m| format!("overlay: {m}")).collect()))
    }
}
