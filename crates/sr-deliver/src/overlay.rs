//! What an output draws over its placed picture, in output time (SREP 13): its overlay symbol, then
//! the captions it burns in output time.
//!
//! Both are rendered as a composition of their own: the symbol's size (the output's frame when it
//! declares none, or when there is no symbol), the output's duration, and the symbol's background
//! (transparent by default). Its caption tracks are the output's, in output time, burned as the
//! output's `burnCaptions` says. Each frame is fitted to the output's frame and composited over the
//! placed picture by the output stage, after the layout.

use std::sync::Arc;

use sr_eval::{EvalOptions, Evaluator};
use sr_gpu::output::{OutputStage, Placement};
use sr_gpu::resources::Tex;
use sr_gpu::{Gpu, Renderer};
use sr_model::model as m;
use sr_model::values::{Color, Paint, Rgba};

use crate::captions::OutputCaptions;
use crate::DeliverError;

pub struct Overlay {
    ev: Evaluator,
    renderer: Renderer,
    /// The layer fitted to the output's frame.
    tex: Arc<Tex>,
}

impl Overlay {
    /// The layer of `output`, or `None` when it has no overlay symbol and burns no caption in output
    /// time. `duration` is the output's duration, `size` its frame size and `captions` its caption
    /// tracks in output time.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        doc: &sr_model::Document,
        output: &m::Output,
        duration: f64,
        size: [u32; 2],
        captions: &OutputCaptions,
        gpu: &Gpu,
        options: &EvalOptions,
        representation: Option<&str>,
    ) -> Result<Option<Overlay>, DeliverError> {
        let symbol = match &output.overlay {
            Some(id) => Some(
                doc.scene
                    .symbols
                    .as_ref()
                    .and_then(|s| s.symbols.iter().find(|s| s.id == *id))
                    .ok_or_else(|| DeliverError::Invalid(format!("overlay {id:?}: no such symbol")))?,
            ),
            None => None,
        };
        let burns = captions.burns(output);
        if symbol.is_none() && !burns {
            return Ok(None);
        }
        let mut d = doc.clone();
        let s = &mut d.scene;
        s.composition.children = symbol.map(|y| y.children.clone()).unwrap_or_default();
        s.project.width = symbol.and_then(|y| y.width).unwrap_or(size[0] as u64);
        s.project.height = symbol.and_then(|y| y.height).unwrap_or(size[1] as u64);
        s.project.duration = m::Duration::new(duration).map_err(|e| DeliverError::Invalid(e.to_string()))?;
        s.project.background = match symbol {
            Some(y) => y.background.clone(),
            None => Paint::Color(Color::Rgba(Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 })),
        };
        s.scene360 = None;
        // the document's captions are in composition time: the layer burns the output's
        s.captions = burns.then(|| m::Captions { loc: Default::default(), caption_tracks: captions.tracks.clone() });
        // Keep caller inputs, but use the symbol/output frame established above rather than
        // reapplying the composition's layout to this independent layer.
        let ev =
            Evaluator::new(&d, &EvalOptions { layout: None, ..options.clone() }).map_err(DeliverError::Document)?;
        let mut renderer = Renderer::new(gpu.clone(), ev.program());
        renderer.representation = representation.map(str::to_owned);
        renderer.burn_captions = output.burn_captions.clone();
        let tex = renderer.texture(size);
        Ok(Some(Overlay { ev, renderer, tex }))
    }

    pub fn warnings(&self) -> &[sr_model::Diagnostic] {
        self.ev.warnings()
    }

    /// Measure each visible text layer (and burned captions) with and without it
    /// in the delivered picture. A transparent overlay has no backdrop of its own;
    /// probing it in isolation would measure against black instead of the picture.
    pub fn contrast(
        &mut self,
        stage: &mut OutputStage,
        t: f64,
        picture: &Tex,
        placement: Placement,
    ) -> Result<Vec<(String, f64, f64)>, DeliverError> {
        let p = self.ev.program();
        let g = self.ev.evaluate(t.clamp(0.0, p.duration));
        let mut targets: Vec<_> = g
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.kind == "layer" && n.text.is_some() && n.draw && n.world_opacity > 0.0)
            .map(|(i, n)| (Some(i), n.id.to_string(), n.world_opacity))
            .collect();
        if p.scene.captions.is_some() {
            targets.push((None, "captions".into(), 1.0));
        }
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let working = self.renderer.working();
        let composite = self.renderer.texture(self.tex.size);
        let ev = &self.ev;
        let mut pixels = |renderer: &mut Renderer,
                          hidden: Option<usize>,
                          captions_off: bool|
         -> Result<Vec<[f32; 4]>, DeliverError> {
            let mut graph = g.clone();
            if let Some(i) = hidden {
                graph.nodes[i].draw = false;
            }
            let mut sub = |st: f64| {
                let mut graph = ev.evaluate(st.clamp(0.0, p.duration));
                if let Some(i) = hidden {
                    // Match by id: temporal samples may have a different set of nodes.
                    for n in &mut graph.nodes {
                        if n.id == g.nodes[i].id {
                            n.draw = false;
                        }
                    }
                }
                graph
            };
            renderer.captions_off = captions_off;
            let frame = renderer.render_with(&graph, p, Some(&mut sub));
            renderer.captions_off = false;
            if let Some(e) = frame.stats.errors.first() {
                return Err(DeliverError::Render { time: t, message: format!("output layer: {e}") });
            }
            stage.place(&frame.texture, &working, &self.tex, Placement::default());
            stage.place_with_overlay(picture, &working, &composite, placement, Some(&self.tex));
            Ok(renderer.read(&composite))
        };
        let after = pixels(&mut self.renderer, None, false)?;
        let mut ratios = Vec::new();
        for (hidden, id, opacity) in targets {
            let before = pixels(&mut self.renderer, hidden, hidden.is_none())?;
            if let Some(ratio) = self.renderer.contrast_of(&before, &after) {
                ratios.push((id, opacity, ratio));
            }
        }
        Ok(ratios)
    }

    /// Renders the layer at output time `t` and fits it to the output's frame; returns the
    /// output-sized texture and what could not be drawn.
    pub fn draw(&mut self, stage: &mut OutputStage, t: f64) -> Result<(&Tex, Vec<String>), DeliverError> {
        let p = self.ev.program();
        let g = self.ev.evaluate(t.clamp(0.0, p.duration));
        let ev = &self.ev;
        let mut sub = |st: f64| ev.evaluate(st.clamp(0.0, p.duration));
        let frame = self.renderer.render_with(&g, p, Some(&mut sub));
        if let Some(e) = frame.stats.errors.first() {
            return Err(DeliverError::Render { time: t, message: format!("output layer: {e}") });
        }
        stage.place(&frame.texture, &self.renderer.working(), &self.tex, Placement::default());
        Ok((&self.tex, frame.stats.unsupported.iter().map(|m| format!("output layer: {m}")).collect()))
    }
}
