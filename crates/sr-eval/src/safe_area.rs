//! `safeArea enforce`: text and nodes tagged `cta` or `logo` must stay inside the safe region
//! (schema/scene-render-1.1.xsd, safeAreaType). `warn` is a finding, `error` fails the run.
//!
//! The check measures each node's layout box through its world transform, at the times it is
//! asked about. The box is what the author placed; text that overflows its own box is not seen.

use sr_model::model::{AssetsChild, SafeAreaEnforce};

use crate::eval::{FrameGraph, FrameNode};
use crate::program::Program;
use crate::sim::text;

/// What `safeArea@enforce` asks of content outside the region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SafeEnforce {
    /// No check: the default when the document names no safe area.
    #[default]
    Off,
    /// Findings are reported, delivery goes on.
    Warn,
    /// A finding fails validation, rendering and encoding.
    Error,
}

impl SafeEnforce {
    /// The level a safe area asks for.
    pub fn of_area(a: &sr_model::model::SafeArea) -> Self {
        Self::of(a.enforce)
    }

    pub(crate) fn of(e: SafeAreaEnforce) -> Self {
        match e {
            SafeAreaEnforce::Off => Self::Off,
            SafeAreaEnforce::Warn => Self::Warn,
            SafeAreaEnforce::Error => Self::Error,
        }
    }
}

/// One node outside the safe region at one time.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// Effective node id.
    pub id: String,
    /// Why the node is checked: `text`, `cta` or `logo`.
    pub kind: &'static str,
    /// The side it crosses furthest: `top`, `right`, `bottom` or `left`.
    pub side: &'static str,
    /// How far past that side, in frame pixels.
    pub overshoot: f64,
}

impl Finding {
    /// The line a report shows for this finding, first seen at `t` seconds.
    pub fn message(&self, t: f64) -> String {
        format!(
            "safeArea: {} ({}) crosses the {} edge of the safe region by {:.0} px at {t:.3} s",
            self.id, self.kind, self.side, self.overshoot
        )
    }
}

/// Why a node is held to the safe region, if it is.
fn checked_as(p: &Program, n: &FrameNode) -> Option<&'static str> {
    if let Some(tags) = text(&*n.elem, "tags") {
        for kind in ["cta", "logo"] {
            if tags.split_whitespace().any(|t| t == kind) {
                return Some(kind);
            }
        }
    }
    let is_text = n.kind == "layer"
        && n.asset.as_deref().and_then(|k| asset_of(p, k)).is_some_and(|a| matches!(a, AssetsChild::Text(_)));
    is_text.then_some("text")
}

fn asset_of<'p>(p: &'p Program, key: &str) -> Option<&'p AssetsChild> {
    let (doc, id) = p.assets.get(key)?;
    let scene = if *doc == 0 { &p.scene } else { &p.includes.get(*doc as usize - 1)?.1 };
    scene.assets.as_ref()?.children.iter().find(|c| c.id() == Some(id.as_str()))
}

/// The insets (top, right, bottom, left, as fractions of the frame) a safe area stands for: its own
/// attributes over its preset's.
pub fn insets_of(a: &sr_model::model::SafeArea) -> [f64; 4] {
    let p = crate::layout::preset_insets(a.preset);
    [
        a.top.map(|v| v.get()).unwrap_or(p[0]),
        a.right.map(|v| v.get()).unwrap_or(p[1]),
        a.bottom.map(|v| v.get()).unwrap_or(p[2]),
        a.left.map(|v| v.get()).unwrap_or(p[3]),
    ]
}

/// How far a box `b` (x0, y0, x1, y1, frame pixels) reaches past the region the insets leave of a
/// frame of `size`: the side crossed furthest, if any crossing is a visible one.
pub fn crossing(id: &str, kind: &'static str, size: [f64; 2], insets: [f64; 4], b: [f64; 4]) -> Option<Finding> {
    let [w, h] = size;
    let [top, right, bottom, left] = insets;
    let region = [left * w, top * h, w - right * w, h - bottom * h];
    let sides = [
        ("left", region[0] - b[0]),
        ("top", region[1] - b[1]),
        ("right", b[2] - region[2]),
        ("bottom", b[3] - region[3]),
    ];
    let worst = sides.into_iter().fold(("", 0.0), |a, s| if s.1 > a.1 { s } else { a });
    // sub-pixel overshoot is rounding, not a placement
    (worst.1 >= 0.5).then(|| Finding { id: id.to_string(), kind, side: worst.0, overshoot: worst.1 })
}

/// The checked nodes of one frame that reach outside the safe region. Empty when the document
/// asks for no enforcement.
pub fn audit(p: &Program, g: &FrameGraph) -> Vec<Finding> {
    if p.safe_enforce == SafeEnforce::Off {
        return Vec::new();
    }
    let mut out = Vec::new();
    for n in &g.nodes {
        if !n.draw || n.world_opacity <= 0.0 {
            continue;
        }
        let (Some(kind), Some([bw, bh])) = (checked_as(p, n), n.size) else { continue };
        let b = n.world.bbox([0.0, 0.0, bw, bh]);
        out.extend(crossing(&n.id, kind, g.size, p.safe_area, b));
    }
    out
}
