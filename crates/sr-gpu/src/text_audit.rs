//! Text that does not fit its box (SREP 18 `TXT-FIT`) or loses characters to `maxLines` or `overflow`
//! (`TXT-CUT`), measured on the layout each text layer is drawn with, at the size drawn (after `autoFit`).
//!
//! The audit evaluates the frames it is given, at most [`MAX_FRAMES`] of them; a longer range is sampled with the
//! safe-area audit's frame choice ([`sr_eval::safe_area::sample_times`]: window edges, key times and a grid), and
//! [`Outcome::note`] says so. Findings never change pixels.
//!
//! Overflow is told apart by what it costs: text that reaches past its box but is drawn whole, inside the frame,
//! loses nothing; text clipped to its box (`overflow="clip"`), or whose block leaves the frame, is cut. The delivery
//! reports the first as information and the second as a warning, so that only lost content fails `--strict`.

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
    /// The frames where overflowing text is lost: clipped to its box (`overflow="clip"`), measured as the overflow in
    /// px of the box; or with its block reaching out of the frame, measured in px of the frame.
    pub lost: Option<Span>,
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

/// How far the laid-out lines of text node `n` reach out of the frame `[0, frame]`, in frame px: 0 when the block
/// stays inside. The lines are placed as the renderer places the text: through the layer's content placement (its
/// fit into the layer box) and the node's world transform.
pub fn outside_frame(lay: &sr_text::Layout, n: &sr_eval::FrameNode, box_size: [f64; 2], frame: [f64; 2]) -> f64 {
    let [aw, ah] = box_size;
    // asset box -> layer local, as the renderer's content placement does
    let local = |p: [f64; 2]| -> [f64; 2] {
        let Some(c) = n.content else { return p };
        let [x0, y0, x1, y1] = c.dest;
        let [u0, v0, u1, v1] = c.uv;
        let du = (u1 - u0).abs().max(1e-9) * (u1 - u0).signum();
        let dv = (v1 - v0).abs().max(1e-9) * (v1 - v0).signum();
        let (sx, sy) = ((x1 - x0) / (du * aw.max(1e-9)), (y1 - y0) / (dv * ah.max(1e-9)));
        [x0 - u0 * aw * sx + p[0] * sx, y0 - v0 * ah * sy + p[1] * sy]
    };
    let mut out = 0.0f64;
    for l in &lay.lines {
        let [x, y, w, h] = l.rect;
        if w <= 0.0 && h <= 0.0 {
            continue;
        }
        for c in [[x, y], [x + w, y], [x, y + h], [x + w, y + h]] {
            let q = n.world.apply(local(c));
            out = out.max(-q[0]).max(q[0] - frame[0]).max(-q[1]).max(q[1] - frame[1]);
        }
    }
    if out > 1e-6 {
        out
    } else {
        0.0
    }
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
                (next, TextFit { id: n.id.to_string(), loc: n.elem.loc(), overflow: None, lost: None, dropped: None })
            });
            if lay.overflow > 0.0 {
                Span::note(&mut f.overflow, lay.overflow, t);
                let clipped = if lay.clip { lay.overflow } else { 0.0 };
                let outside = outside_frame(&lay, n, lay.size, g.size);
                if clipped > 0.0 || outside > 0.0 {
                    Span::note(&mut f.lost, clipped.max(outside), t);
                }
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
