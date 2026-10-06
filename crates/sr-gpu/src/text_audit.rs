//! Text that does not fit its box (SREP 18 `TXT-FIT`) or loses characters to `maxLines` or `overflow`
//! (`TXT-CUT`), measured on the layout each text layer is drawn with, at the size drawn (after `autoFit`).
//!
//! The audit evaluates the frames it is given, at most [`MAX_FRAMES`] of them; a longer range is sampled with the
//! safe-area audit's frame choice ([`sr_eval::safe_area::sample_times`]: window edges, key times and a grid), and
//! [`Outcome::note`] says so. Findings never change pixels.

use std::collections::{BTreeMap, HashMap};

use sr_eval::{Evaluator, Value};
use sr_model::element::Element as _;
use sr_model::model::AssetsChild;
use sr_model::Loc;

use crate::text::{asset_of, text_layout, Cx, TextCache};

/// Frames audited one by one; longer ranges are sampled.
pub const MAX_FRAMES: usize = 240;

/// One measure of one text layer over the frames where it held.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    /// The largest value seen: overflow in px, or characters dropped.
    pub worst: f64,
    /// First and last composition time it was seen at, in seconds.
    pub time: [f64; 2],
}

impl Span {
    fn note(slot: &mut Option<Span>, value: f64, t: f64) {
        match slot {
            Some(s) => {
                s.worst = s.worst.max(value);
                s.time[0] = s.time[0].min(t);
                s.time[1] = s.time[1].max(t);
            }
            None => *slot = Some(Span { worst: value, time: [t, t] }),
        }
    }
}

/// What the audit found for one text layer.
#[derive(Debug, Clone, PartialEq)]
pub struct TextFit {
    /// Effective node id.
    pub id: String,
    /// Source position of the layer's element (in the document that defines it).
    pub loc: Loc,
    /// Lines reaching past the box, in px of the text box at the size drawn (`TXT-FIT`).
    pub overflow: Option<Span>,
    /// Characters dropped (`TXT-CUT`).
    pub dropped: Option<Span>,
}

/// The findings, by layer in first-seen order, and how the frames were chosen.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// Layers whose text did not fit or lost characters at some audited frame.
    pub found: Vec<TextFit>,
    /// `sampled N of M frames`, when the audit did not look at every frame.
    pub note: Option<String>,
}

/// Audits the text layers drawn at `times` (composition seconds, ascending).
pub fn check(ev: &Evaluator, times: &[f64]) -> Outcome {
    let p = ev.program();
    let (times, note) = if times.len() <= MAX_FRAMES {
        (times.to_vec(), None)
    } else {
        let fps = p.fps.as_f64();
        let (from, to) = (times[0], times[times.len() - 1] + 1.0 / fps);
        let s = sr_eval::safe_area::sample_times(p, from, to);
        let note = s.note();
        (s.times, note)
    };
    let mut tc = TextCache::default();
    let tokens = HashMap::new();
    let mut found: BTreeMap<String, (usize, TextFit)> = BTreeMap::new();
    for &t in &times {
        let g = ev.evaluate_layout(t);
        for n in &g.nodes {
            if !n.draw || n.world_opacity <= 0.0 {
                continue;
            }
            let Some(key) = n.asset.as_deref() else { continue };
            let Some((AssetsChild::Text(asset), doc)) = asset_of(p, key) else { continue };
            let mut paint = |_: &Value, _: [f64; 4]| -> Option<sr_vector::Paint> { None };
            let mut unsupported = Vec::new();
            let mut cx = Cx {
                p,
                g: &g,
                n,
                base: p.base_dirs.get(doc).cloned().unwrap_or_default(),
                tol: 0.25,
                paint: &mut paint,
                tokens: &tokens,
                unsupported: &mut unsupported,
            };
            let lay = text_layout(&mut tc, &mut cx, key, asset);
            if lay.overflow <= 0.0 && lay.dropped == 0 {
                continue;
            }
            let next = found.len();
            let (_, f) = found.entry(n.id.to_string()).or_insert_with(|| {
                (next, TextFit { id: n.id.to_string(), loc: n.elem.loc(), overflow: None, dropped: None })
            });
            if lay.overflow > 0.0 {
                Span::note(&mut f.overflow, lay.overflow, t);
            }
            if lay.dropped > 0 {
                Span::note(&mut f.dropped, lay.dropped as f64, t);
            }
        }
    }
    let mut found: Vec<(usize, TextFit)> = found.into_values().collect();
    found.sort_by_key(|(k, _)| *k);
    Outcome { found: found.into_iter().map(|(_, f)| f).collect(), note }
}
