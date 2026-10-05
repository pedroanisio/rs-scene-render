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

fn flag(e: &dyn sr_model::element::Element, name: &str) -> bool {
    use sr_model::element::AttrValue;
    match e.get_attr(name) {
        Some(AttrValue::Bool(b)) => b,
        Some(AttrValue::Str(s)) => s == "true",
        _ => false,
    }
}

/// Whether the symbol named `id` carries `safeAreaForce`.
fn symbol_forced(p: &Program, id: &str) -> bool {
    let find = |scene: &sr_model::model::Scene| {
        scene.symbols.as_ref().and_then(|s| s.symbols.iter().find(|s| s.id == id)).map(|s| flag(s, "safeAreaForce"))
    };
    find(&p.scene).or_else(|| p.includes.iter().find_map(|(_, scene)| find(scene))).unwrap_or(false)
}

/// Whether node `n` itself carries `safeAreaForce`, directly or through the symbol it instantiates.
fn carries_force(p: &Program, n: &FrameNode) -> bool {
    flag(&*n.elem, "safeAreaForce")
        || (n.kind == "instance" && text(&*n.elem, "symbol").is_some_and(|id| symbol_forced(p, &id)))
}

/// Ids of the nodes of a frame that carry `safeAreaForce`, whether or not they are outside the region.
pub fn forced(p: &Program, g: &FrameGraph) -> Vec<String> {
    g.nodes.iter().filter(|n| carries_force(p, n)).map(|n| n.id.to_string()).collect()
}

/// The checked nodes of one frame that reach outside the safe region. Empty when the document
/// asks for no enforcement.
pub fn audit(p: &Program, g: &FrameGraph) -> Vec<Finding> {
    if p.safe_enforce == SafeEnforce::Off {
        return Vec::new();
    }
    let mut out = Vec::new();
    // a forced node exempts everything it contains: containers come before their children
    let mut exempt = vec![false; g.nodes.len()];
    for (i, n) in g.nodes.iter().enumerate() {
        exempt[i] = carries_force(p, n) || n.parent.is_some_and(|k| exempt[k as usize]);
    }
    for (i, n) in g.nodes.iter().enumerate() {
        if exempt[i] || !n.draw || n.world_opacity <= 0.0 {
            continue;
        }
        let (Some(kind), Some([bw, bh])) = (checked_as(p, n), n.size) else { continue };
        let b = n.world.bbox([0.0, 0.0, bw, bh]);
        out.extend(crossing(&n.id, kind, g.size, p.safe_area, b));
    }
    out
}

/// Most frames one audit evaluates.
const MAX_SAMPLES: usize = 240;

/// The frames to audit in `from..to` seconds, and how many frames the range has.
#[derive(Debug, Clone, PartialEq)]
pub struct Sampling {
    /// Frame times to evaluate, ascending.
    pub times: Vec<f64>,
    /// Frames in the range.
    pub frames: u64,
}

impl Sampling {
    /// `sampled N of M frames` when not every frame is audited.
    pub fn note(&self) -> Option<String> {
        (self.times.len() as u64 != self.frames)
            .then(|| format!("sampled {} of {} frames", self.times.len(), self.frames))
    }
}

/// Whether a node is held to the region (without asking where it is).
fn is_candidate(p: &Program, n: &crate::program::InstNode) -> bool {
    if let Some(tags) = text(&*n.elem, "tags") {
        if tags.split_whitespace().any(|t| t == "cta" || t == "logo") {
            return true;
        }
    }
    n.name == "layer"
        && n.asset.as_deref().and_then(|k| asset_of(p, k)).is_some_and(|a| matches!(a, AssetsChild::Text(_)))
}

/// The frames of `from..to` at which a checked node can change place: the window edges and key times of
/// every checked node and of the containers it sits in, plus a coarse grid. Keys inside an instance's own
/// clock are mapped by the instance start (speed 1), and the grid covers the rest; the audit is not exhaustive,
/// and says so through [`Sampling::note`]. When the range has no more than a few hundred frames, every frame.
pub fn sample_times(p: &Program, from: f64, to: f64) -> Sampling {
    let fps = p.fps.as_f64();
    let first = (from * fps).ceil().max(0.0) as u64;
    let end = ((to * fps).ceil().max(0.0) as u64).min(p.fps.frame_count(p.duration));
    let frames = end.saturating_sub(first);
    if frames as usize <= MAX_SAMPLES {
        return Sampling { times: (first..end).map(|k| p.fps.frame_time(k)).collect(), frames };
    }
    // nodes whose position matters: the candidates and every container above them
    let mut relevant = vec![false; p.nodes.len()];
    for (i, n) in p.nodes.iter().enumerate() {
        if is_candidate(p, n) {
            let mut k = Some(i as u32);
            while let Some(j) = k {
                if relevant[j as usize] {
                    break;
                }
                relevant[j as usize] = true;
                k = p.nodes[j as usize].parent;
            }
        }
    }
    // composition time of a node's own timeline origin: instance and clock containers shift it by their start
    let origin = |i: usize| -> f64 {
        let mut o = 0.0;
        let mut k = p.nodes[i].parent;
        while let Some(j) = k {
            let a = &p.nodes[j as usize];
            if !matches!(a.clock, crate::program::Clock::Same) {
                o += a.start;
            }
            k = a.parent;
        }
        o
    };
    let mut keys: Vec<f64> = vec![from, to - 1.0 / fps];
    for (i, n) in p.nodes.iter().enumerate().filter(|(i, _)| relevant[*i]) {
        let o = origin(i);
        for t in [Some(n.start), n.end, Some(n.vis_start), n.vis_end].into_iter().flatten() {
            keys.push(o + t);
            keys.push(o + t - 1.0 / fps);
        }
    }
    for s in &p.slots {
        if let crate::program::Owner::Node(i) = s.owner {
            if relevant[i as usize] {
                let o = origin(i as usize);
                for &c in &s.channels {
                    keys.extend(p.channels[c as usize].key_times().map(|t| o + t));
                }
            }
        }
    }
    let to_frames = |t: f64| {
        let k = t * fps;
        [k.floor(), k.ceil()].map(|k| k.clamp(first as f64, end as f64 - 1.0) as u64)
    };
    let mut picked: std::collections::BTreeSet<u64> =
        keys.iter().filter(|t| t.is_finite()).flat_map(|&t| to_frames(t)).collect();
    let mut grid =
        (first..end).step_by(((1.0f64).max((to - from) / 64.0) * fps).round().max(1.0) as usize).collect::<Vec<_>>();
    grid.push(end - 1);
    picked.extend(grid);
    let mut all: Vec<u64> = picked.into_iter().collect();
    if all.len() > MAX_SAMPLES {
        // thin evenly, keeping both ends
        let step = all.len() as f64 / MAX_SAMPLES as f64;
        let thinned: Vec<u64> =
            (0..MAX_SAMPLES).map(|i| all[((i as f64 * step) as usize).min(all.len() - 1)]).collect();
        all = thinned;
        all.dedup();
    }
    Sampling { times: all.into_iter().map(|k| p.fps.frame_time(k)).collect(), frames }
}
