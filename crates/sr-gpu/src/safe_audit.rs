//! `safeArea enforce` over a whole render: nodes from the evaluator, burned captions from their layout.
//!
//! A caption track is held to the safe area it names with `@safeArea`, else the project's; the
//! level (`warn` or `error`) is that area's own `@enforce`.

use std::collections::BTreeMap;

use sr_eval::safe_area::{Finding, SafeEnforce};
use sr_eval::{Evaluator, Program, Value};
use sr_model::model::SafeArea;

use crate::text::{caption_scene, TextCache};

/// A finding with the time it was first seen at and the level that applies to it.
#[derive(Debug, Clone, PartialEq)]
pub struct Timed {
    /// First time the node or caption was outside the region, in seconds.
    pub t: f64,
    /// The worst crossing seen over the audited times.
    pub finding: Finding,
    /// `warn` or `error`, from the safe area that applies.
    pub level: SafeEnforce,
    /// The safe area the node or caption was held to.
    pub area: Option<String>,
}

impl Timed {
    /// The finding as a diagnostic (`SA01`) located at its safe area: a warning for `warn`, an error for `error`.
    pub fn diagnostic(&self, p: &Program) -> sr_model::Diagnostic {
        let (loc, path) = self
            .area
            .as_deref()
            .and_then(|id| p.scene.safe_areas.as_ref()?.safe_areas.iter().position(|a| a.id == id))
            .map(|i| {
                (
                    p.scene.safe_areas.as_ref().unwrap().safe_areas[i].loc,
                    format!("/scene/safeAreas/safeArea[{}]", i + 1),
                )
            })
            .unwrap_or_default();
        let message = self.finding.message(self.t);
        if self.level == SafeEnforce::Error {
            sr_model::Diagnostic::error("SA01", message, loc, path)
        } else {
            sr_model::Diagnostic::warning("SA01", message, loc, path)
        }
    }
}

/// `SA01` diagnostics for rendering `times`: empty when the document asks for no enforcement.
pub fn diagnostics(ev: &Evaluator, times: &[f64]) -> Vec<sr_model::Diagnostic> {
    let p = ev.program();
    audit(ev, times).iter().map(|t| t.diagnostic(p)).collect()
}

/// The insets and enforcement of the safe area named `id`.
fn area_of(p: &Program, id: &str) -> Option<([f64; 4], SafeEnforce)> {
    let a: &SafeArea = p.scene.safe_areas.as_ref()?.safe_areas.iter().find(|a| a.id == id)?;
    Some((sr_eval::safe_area::insets_of(a), SafeEnforce::of_area(a)))
}

/// Every checked node and burned caption outside the safe region at `times`, one entry per
/// (id, kind, side): the first time it happens, and its worst overshoot.
pub fn audit(ev: &Evaluator, times: &[f64]) -> Vec<Timed> {
    audit_forced(ev, times).0
}

/// What an audit found: the findings, the nodes that carry `safeAreaForce`, and how the frames were chosen.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// Nodes and captions outside the region.
    pub timed: Vec<Timed>,
    /// Ids of nodes carrying `safeAreaForce` (instances and symbols included), in first-seen order.
    pub forced: Vec<String>,
    /// `sampled N of M frames`, when the audit did not look at every frame.
    pub note: Option<String>,
}

impl Outcome {
    /// `SA01` diagnostics for the findings.
    pub fn diagnostics(&self, p: &Program) -> Vec<sr_model::Diagnostic> {
        self.timed.iter().map(|t| t.diagnostic(p)).collect()
    }
}

/// Audits the frames of `from..to` seconds that matter: every frame of a short range, else the window
/// edges and key times of the checked nodes, the caption page edges, and a coarse grid (see
/// [`sr_eval::safe_area::sample_times`]). Placement is evaluated without simulations, which do not move text,
/// captions or tagged nodes.
pub fn check(ev: &Evaluator, from: f64, to: f64) -> Outcome {
    let p = ev.program();
    let mut s = sr_eval::safe_area::sample_times(p, from, to);
    if s.times.len() as u64 != s.frames {
        // caption pages change what is drawn without any node moving: their edges are frames to look at
        let mut tc = TextCache::default();
        let fps = p.fps.as_f64();
        let mut extra = Vec::new();
        for tr in crate::text::tracks(&mut tc, p).iter().flatten().filter(|t| t.burn) {
            for pg in &tr.pages {
                for t in [pg.start, pg.end - 1.0 / fps] {
                    if t >= from && t < to {
                        extra.push(p.fps.frame_time((t * fps).round().max(0.0) as u64));
                    }
                }
            }
        }
        s.times.extend(extra);
        s.times.sort_by(|a, b| a.total_cmp(b));
        s.times.dedup();
    }
    let (timed, forced) = audit_forced(ev, &s.times);
    Outcome { timed, forced, note: s.note() }
}

fn audit_forced(ev: &Evaluator, times: &[f64]) -> (Vec<Timed>, Vec<String>) {
    let p = ev.program();
    let mut forced: Vec<String> = Vec::new();
    let mut tc = TextCache::default();
    let mut found: BTreeMap<(String, &'static str, &'static str), Timed> = BTreeMap::new();
    let mut note = |t: f64, f: Finding, level: SafeEnforce, area: Option<String>| {
        let key = (f.id.clone(), f.kind, f.side);
        match found.get_mut(&key) {
            Some(old) => {
                if f.overshoot > old.finding.overshoot {
                    old.finding = f;
                }
            }
            None => {
                found.insert(key, Timed { t, finding: f, level, area });
            }
        }
    };
    for &t in times {
        let g = ev.evaluate_layout(t);
        for id in sr_eval::safe_area::forced(p, &g) {
            if !forced.contains(&id) {
                forced.push(id);
            }
        }
        for f in sr_eval::safe_area::audit(p, &g) {
            note(t, f, p.safe_enforce, p.safe_area_id.clone());
        }
        let mut errors = Vec::new();
        let tokens = Default::default();
        // each burned track on its own, so a track's box is its own
        for tr in p.scene.captions.as_ref().map(|c| c.caption_tracks.as_slice()).unwrap_or(&[]) {
            // a sidecar file is not drawn into the picture
            if tr.mode.as_str() == "sidecar" {
                continue;
            }
            let (insets, level, area) = match tr.safe_area.as_deref().and_then(|id| area_of(p, id).map(|a| (a, id))) {
                Some(((i, l), id)) => (i, l, Some(id.to_string())),
                None => (p.safe_area, p.safe_enforce, p.safe_area_id.clone()),
            };
            if level == SafeEnforce::Off {
                continue;
            }
            let mut paint =
                |_: &Value, _: [f64; 4]| Some(sr_vector::scene::Paint::Solid { rgba: [1.0; 4], srgb: true });
            let Some((scene, _)) = caption_scene(&mut tc, p, &g, &mut paint, &tokens, Some(&tr.id), &mut errors) else {
                continue;
            };
            let r = scene.bounds().0;
            if !r[0].is_finite() {
                continue;
            }
            if let Some(f) = sr_eval::safe_area::crossing(&tr.id, "caption", g.size, insets, r) {
                note(t, f, level, area);
            }
        }
    }
    (found.into_values().collect(), forced)
}
