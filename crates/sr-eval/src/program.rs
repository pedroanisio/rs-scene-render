//! Templating and instantiation.
//!
//! [`build`] runs once per variant and layout. It clones the typed scene,
//! resolves parameters (defaults, variant sets, data rows, command-line
//! values), applies binds and overrides, substitutes `{{placeholders}}`,
//! and expands the composition into a flat tree of instantiated nodes:
//! repeats become one copy per item, instances and includes become
//! containers holding scoped clones of their symbols. It then compiles every
//! animation element into slots, channels, expressions and links, orders
//! them by dependency, and places nodes in time.

#![allow(clippy::type_complexity)]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sr_model::diag::{Diagnostic, Loc};
use sr_model::element::{children, walk, walk_mut, AttrValue, Element};
use sr_model::model::{self as m, Node};
use sr_model::values::{Color, Fps, Length};
use sr_model::xsd::COMPLEX_TYPES;
use sr_model::Document;

use crate::channel::{Channel, ChannelSpec, Lookup};
use crate::curve::{self, Ease, KeyParams};
use crate::data;
use crate::expr::vm::{self, Band, Code, Resolver, V};
use crate::path::MotionPath;
use crate::value::{PropKind, Value};

/// Selections and inputs for one evaluation program.
#[derive(Debug, Clone, Default)]
pub struct EvalOptions {
    /// Variant id to apply.
    pub variant: Option<String>,
    /// Layout id to render (frame size and overrides).
    pub layout: Option<String>,
    /// `--param id=value` values; they win over every other source.
    pub params: Vec<(String, String)>,
    /// Data row for batch rendering: data source id (first source when `None`) and row index.
    pub row: Option<(Option<String>, usize)>,
    /// Audio analysis for `audioAmplitude()`, `beat()` and `audio:` links.
    pub analysis: Analysis,
}

/// Per-track audio analysis, filled by the media pipeline.
#[derive(Debug, Clone, Default)]
pub struct Analysis {
    /// Frames per second of the envelopes.
    pub fps: f64,
    /// Envelopes by audio track id: full, low, mid, high in [0, 1].
    pub tracks: HashMap<String, [Vec<f32>; 4]>,
    /// Detected beat times in seconds, when a beat grid names an audio source.
    pub beats: Option<Vec<f64>>,
}

impl Analysis {
    /// Amplitude of `track` in `band` at time `t` (linear between envelope frames).
    pub fn amplitude(&self, track: &str, band: Band, t: f64) -> f64 {
        let Some(env) = self.tracks.get(track) else { return 0.0 };
        let e = &env[match band {
            Band::Full => 0,
            Band::Low => 1,
            Band::Mid => 2,
            Band::High => 3,
        }];
        if e.is_empty() || self.fps <= 0.0 {
            return 0.0;
        }
        let x = (t * self.fps).max(0.0);
        let i = (libm::floor(x) as usize).min(e.len() - 1);
        let j = (i + 1).min(e.len() - 1);
        let f = (x - i as f64).clamp(0.0, 1.0);
        e[i] as f64 + (e[j] as f64 - e[i] as f64) * f
    }
}

// ------------------------------------------------------------------ program data

/// Maps a container's timeline to its children's timeline.
#[derive(Debug, Clone)]
pub enum Clock {
    /// Children share the container's timeline.
    Same,
    /// `child = origin + (t − origin − offset) · scale` (group timeOffset/timeScale, repeat stagger).
    Affine {
        /// Scaling origin.
        origin: f64,
        /// Offset.
        offset: f64,
        /// Scale.
        scale: f64,
    },
    /// Symbol or include clock.
    Media(Box<MediaClock>),
}

/// Local-to-source time mapping of layers and instances.
#[derive(Debug, Clone)]
pub struct MediaClock {
    /// Node start on its timeline.
    pub start: f64,
    /// Playback rate (speed / timeStretch).
    pub rate: f64,
    /// Source in-point.
    pub clip_in: f64,
    /// Source length after clipIn, when known.
    pub len: Option<f64>,
    /// Extra plays.
    pub loops: u64,
    /// Play backwards.
    pub reverse: bool,
    /// Local time whose source frame is held for the node's whole window.
    pub freeze_at: Option<f64>,
    /// timeRemap channel (local time → source seconds).
    pub remap: Option<Channel>,
}

impl MediaClock {
    /// Source time at timeline time `t`.
    pub fn map(&self, t: f64) -> f64 {
        let local = self.freeze_at.unwrap_or(t - self.start);
        if let Some(r) = &self.remap {
            return r.eval(local).as_num().unwrap_or(0.0);
        }
        let mut s = local * self.rate;
        if let Some(len) = self.len.filter(|l| *l > 0.0) {
            if self.loops > 0 {
                let total = len * (self.loops + 1) as f64;
                s = if s >= total {
                    len
                } else if s < 0.0 {
                    s
                } else {
                    s % len
                };
            }
            if self.reverse {
                s = len - s;
            }
        } else if self.reverse {
            s = -s;
        }
        self.clip_in + s
    }
}

/// Transform properties read every frame.
#[derive(Debug, Clone, Copy)]
pub struct Tf {
    /// x, y, anchorX, anchorY.
    pub pos: [Length; 4],
    /// rotation, scaleX, scaleY, skewX, skewY, opacity, zDepth, rotationX, rotationY.
    pub num: [f64; 9],
}

/// Indices of transform properties in `Tf`.
pub mod tfi {
    pub const X: usize = 0;
    pub const Y: usize = 1;
    pub const AX: usize = 2;
    pub const AY: usize = 3;
    pub const ROT: usize = 0;
    pub const SX: usize = 1;
    pub const SY: usize = 2;
    pub const KX: usize = 3;
    pub const KY: usize = 4;
    pub const OPACITY: usize = 5;
    pub const ZDEPTH: usize = 6;
    pub const RX: usize = 7;
    pub const RY: usize = 8;
}

const TF_NUM: [&str; 9] =
    ["rotation", "scaleX", "scaleY", "skewX", "skewY", "opacity", "zDepth", "rotationX", "rotationY"];
const TF_POS: [&str; 4] = ["x", "y", "anchorX", "anchorY"];
const ALIASES: [(&str, [&str; 2]); 4] = [
    ("position", ["x", "y"]),
    ("scale", ["scaleX", "scaleY"]),
    ("anchor", ["anchorX", "anchorY"]),
    ("skew", ["skewX", "skewY"]),
];

/// Where a transform property's per-frame value comes from.
#[derive(Debug, Clone, Default)]
pub struct TfSlots {
    /// Slots for x, y, anchorX, anchorY (component 0/1 of an alias pair when `pair` is set).
    pub pos: [Option<(u32, Option<u8>)>; 4],
    /// Slots for the numeric transform properties.
    pub num: [Option<(u32, Option<u8>)>; 9],
}

/// A motion path attached to a node.
#[derive(Debug, Clone)]
pub struct Motion {
    /// Path.
    pub path: MotionPath,
    /// Start time (node timeline).
    pub start: f64,
    /// End time; the node end when absent.
    pub end: Option<f64>,
    /// Progress curve.
    pub ease: Ease,
    /// Rotate along the tangent.
    pub auto_orient: bool,
    /// Degrees added to the tangent angle.
    pub orient_offset: f64,
    /// Arc-length parameterisation.
    pub constant_speed: bool,
    /// Slot of an animated `progress`, when present.
    pub progress: Option<u32>,
}

/// Instantiated node kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// A leaf or plain container.
    Plain,
    /// One copy generated by a repeat.
    RepeatCopy {
        /// Copy number.
        index: u32,
        /// Number of copies.
        count: u32,
    },
    /// A transition element (index into `Program::transitions`).
    Transition(u32),
}

/// An instantiated node.
#[derive(Debug, Clone)]
pub struct InstNode {
    /// Effective id (instance and repeat scopes joined with '/').
    pub id: Arc<str>,
    /// Scope its children's ids live in.
    pub scope: Arc<str>,
    /// XML element name (`copy` for repeat copies).
    pub name: &'static str,
    /// The element after overrides, without child nodes.
    pub elem: Arc<Node>,
    /// Kind.
    pub kind: Kind,
    /// Container in the tree.
    pub parent: Option<u32>,
    /// Children in paint order (z, then document order).
    pub children: Vec<u32>,
    /// Document the node comes from (0 = main).
    pub doc: u16,
    /// Start on its timeline.
    pub start: f64,
    /// End on its timeline (exclusive).
    pub end: Option<f64>,
    /// Visible from (transition handles may start it before `start`).
    pub vis_start: f64,
    /// Visible until (transition handles may extend it past `end`).
    pub vis_end: Option<f64>,
    /// Clock for the children.
    pub clock: Clock,
    /// Source clock (layers).
    pub media: Option<MediaClock>,
    /// Static transform.
    pub tf: Tf,
    /// Animated transform sources.
    pub tf_slots: TfSlots,
    /// Stacking order among siblings. `object3D` and `camera` take `z` as a depth in scene units,
    /// so theirs is 0.
    pub z: i32,
    /// Slot of an animated `z`, when the stacking order can change between frames.
    pub z_slot: Option<u32>,
    /// Whether any child has an animated `z`: the evaluator then restacks the children every frame.
    pub restack: bool,
    /// `@threeD`.
    pub three_d: bool,
    /// `@matteVisible`.
    pub matte_visible: bool,
    /// `@visible`.
    pub visible: bool,
    /// Condition expression.
    pub cond: Option<u32>,
    /// `@parent` node.
    pub parent_link: Option<u32>,
    /// `@matte` node.
    pub matte: Option<u32>,
    /// Repeat copy transform: dx, dy, rotation, scale, opacity factor.
    pub copy: Option<[f64; 5]>,
    /// Motion path.
    pub motion: Option<Motion>,
    /// Box for percentage lengths of the children, when the node defines one.
    pub box_size: Option<[Length; 2]>,
    /// Asset key (namespaced for includes).
    pub asset: Option<Arc<str>>,
    /// Resolved text for layers of text assets.
    pub text: Option<Arc<str>>,
    /// Slots owned by the node.
    pub slots: Vec<u32>,
    /// Element targets inside the node (masks, modifiers, …).
    pub parts: Vec<u32>,
    /// Element seed.
    pub seed: u64,
    /// Nearest repeat copy: (index, count, variable, item).
    pub repeat: Option<(u32, u32, Arc<str>, V)>,
    /// Fit, crop and flip of a layer.
    pub fit: Option<crate::layout::FitSpec>,
    /// Intrinsic size of a layer's asset.
    pub asset_size: Option<[f64; 2]>,
    /// Asset element name (`image`, `video`, `generator`, …).
    pub asset_kind: Option<&'static str>,
    /// Shape width and height.
    pub shape_size: Option<[Length; 2]>,
    /// Automatic layout of the children.
    pub layout: Option<crate::layout::LayoutSpec>,
    /// Responsive alignment.
    pub align: Option<crate::layout::AlignSpec>,
    /// Children in document order (layout order).
    pub doc_children: Vec<u32>,
    /// Clip children to the box.
    pub clip: bool,
}

/// Who owns a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Owner {
    /// A node's own attribute.
    Node(u32),
    /// An element target.
    Element(u32),
}

/// One property with its value sources.
#[derive(Debug, Clone)]
pub struct Slot {
    /// Owner.
    pub owner: Owner,
    /// Property name.
    pub prop: Arc<str>,
    /// Kind.
    pub kind: PropKind,
    /// Static value.
    pub base: Value,
    /// Keyframe channels in document order.
    pub channels: Vec<u32>,
    /// Expression.
    pub expr: Option<u32>,
    /// Link.
    pub link: Option<u32>,
    /// Node whose timeline drives the slot; `None` for the composition timeline.
    pub time_node: Option<u32>,
}

/// An element with animated properties that is not a node.
#[derive(Debug, Clone)]
pub struct ElemTarget {
    /// Key: its id, or owner-relative path such as `mask[0]`.
    pub key: Arc<str>,
    /// Element name.
    pub name: &'static str,
    /// Owning node, if inside one.
    pub node: Option<u32>,
    /// The element's own `@id` as nodes see it: scoped by the instances it sits in (`inst/b0`). Parts of a node
    /// (bones, masks, paint stops) carry one when they declare an id, so `prop("b0.rotation")` and links find them.
    pub id: Option<Arc<str>>,
    /// Static attribute kinds and values.
    pub attrs: Vec<(Arc<str>, PropKind, Value)>,
    /// Slots.
    pub slots: Vec<u32>,
}

/// A compiled expression.
#[derive(Debug, Clone)]
pub struct ExprInst {
    /// Bytecode.
    pub code: Code,
    /// Slot it computes, or `None` for node conditions.
    pub slot: Option<u32>,
    /// Owning node.
    pub node: Option<u32>,
    /// Seed for random and noise.
    pub seed: u64,
}

/// Link sources.
#[derive(Debug, Clone)]
pub enum LinkSource {
    /// Another property.
    Prop(u32),
    /// A parameter.
    Param(V),
    /// An audio envelope.
    Audio(String, Band),
    /// Marker progress: 0 before, ramps over the duration, 1 after.
    Marker(f64, f64),
}

/// A compiled link.
#[derive(Debug, Clone)]
pub struct LinkInst {
    /// Source.
    pub source: LinkSource,
    /// Scale.
    pub scale: f64,
    /// Offset.
    pub offset: f64,
    /// Lower clamp.
    pub min: Option<f64>,
    /// Upper clamp.
    pub max: Option<f64>,
    /// Delay in seconds.
    pub delay: f64,
    /// Moving-average window in seconds.
    pub smoothing: f64,
}

/// A transition window between nodes.
#[derive(Debug, Clone)]
pub struct TransitionInst {
    /// Transition node, or `None` for sequence auto-transitions.
    pub node: Option<u32>,
    /// Type literal.
    pub kind: Arc<str>,
    /// Outgoing node.
    pub from: Option<u32>,
    /// Incoming node.
    pub to: Option<u32>,
    /// Window on the siblings' timeline.
    pub window: (f64, f64),
    /// Progress curve.
    pub ease: Ease,
    /// Container whose timeline the window lives on.
    pub container: Option<u32>,
    /// The `luma` matte sibling, which is not drawn itself.
    pub matte: Option<u32>,
}

/// A beat grid.
#[derive(Debug, Clone, Copy)]
pub struct BeatGrid {
    /// Beats per minute.
    pub bpm: f64,
    /// Time of beat 0.
    pub offset: f64,
    /// Beats per bar.
    pub per_bar: u64,
}

/// Everything evaluation needs, built once.
#[derive(Debug)]
pub struct Program {
    /// Allocation identity retained by downstream caches across moves and document reloads.
    identity: Arc<()>,
    /// Imported animation frames live only as long as their compiled scene.
    pub(crate) mesh_sequence_cache: std::sync::Mutex<crate::mesh_sequence::Cache>,
    /// Models read for joint sockets, and the joints each object is asked for.
    pub(crate) joint_models: crate::joints::Cache,
    pub(crate) joint_sockets: std::sync::OnceLock<crate::joints::Sockets>,
    /// The templated main scene.
    pub scene: m::Scene,
    /// Included documents: namespace and templated scene.
    pub includes: Vec<(Arc<str>, m::Scene)>,
    /// Frame rate.
    pub fps: Fps,
    /// Duration in seconds.
    pub duration: f64,
    /// Frame size (layout size when a layout that reflows is selected).
    pub size: [f64; 2],
    /// A selected layout that crops or fits the composition instead of reflowing it.
    pub reframe: Option<Reframe>,
    /// Project seed.
    pub seed: u64,
    /// Nodes; roots are the composition children.
    pub nodes: Vec<InstNode>,
    /// Root nodes in document order.
    pub roots: Vec<u32>,
    /// Root nodes in document order.
    pub doc_roots: Vec<u32>,
    /// Whether any root has an animated `z`.
    pub restack_roots: bool,
    /// Slots.
    pub slots: Vec<Slot>,
    /// Slot evaluation order (dependencies first).
    pub order: Vec<u32>,
    /// Whether another slot's expression or link reads the slot: one nobody reads is not evaluated while
    /// its node is off screen (see eval::Frame::slots).
    pub slot_read: Vec<bool>,
    /// Channels.
    pub channels: Vec<Channel>,
    /// Expressions.
    pub exprs: Vec<ExprInst>,
    /// Links.
    pub links: Vec<LinkInst>,
    /// Element targets.
    pub elements: Vec<ElemTarget>,
    /// Transitions.
    pub transitions: Vec<TransitionInst>,
    /// Marker times (including generated `beat.N`/`bar.N` on demand).
    pub markers: HashMap<String, f64>,
    /// Beat grid.
    pub beat: Option<BeatGrid>,
    /// Parameter values.
    pub params: HashMap<String, V>,
    /// Audio analysis.
    pub analysis: Analysis,
    /// Asset key → (document index, asset id).
    pub assets: HashMap<Arc<str>, (u16, String)>,
    /// Safe-area insets (top, right, bottom, left) as fractions of the frame.
    pub safe_area: [f64; 4],
    /// What `safeArea@enforce` asks of content outside that region.
    pub safe_enforce: crate::safe_area::SafeEnforce,
    /// Id of the safe area `safe_area` and `safe_enforce` come from.
    pub safe_area_id: Option<String>,
    /// Base directory of each document (0 = main, then includes).
    pub base_dirs: Vec<PathBuf>,
    /// Warnings.
    pub warnings: Vec<Diagnostic>,
    /// Tracking data by id.
    pub tracks: HashMap<String, Arc<crate::rig::TrackEntry>>,
    /// Skin weights of skeletons with `@weights`, by skeleton id.
    pub skins: HashMap<Arc<str>, Arc<crate::eval::SkinWeights>>,
}

/// The element names the 3D pass draws. The renderer's 3D pass and the choice of GPU adapter both read this one list,
/// so a kind added here is drawn and is also asked for an adapter that can run the pass.
pub const THREE_D_DRAWN: &[&str] = &["object3D", "particles3D", "ocean"];

/// True when the 3D pass draws elements named `name` (see [`THREE_D_DRAWN`]).
pub fn draws_in_3d(name: &str) -> bool {
    THREE_D_DRAWN.contains(&name)
}

impl Program {
    /// Identity of this compiled document. Caches can retain a clone and compare with
    /// `Arc::ptr_eq`; retaining it prevents reuse of the identity after the program is dropped.
    pub fn identity(&self) -> &Arc<()> {
        &self.identity
    }

    /// True when the document has an element the 3D pass draws (`object3D`, `particles3D`, `ocean`),
    /// so the adapter must be one that can run that pass.
    pub fn uses_3d(&self) -> bool {
        self.nodes.iter().any(|n| draws_in_3d(n.name))
    }
}

// ------------------------------------------------------------------ helpers

fn attr_num(e: &dyn Element, n: &str) -> Option<f64> {
    match e.get_attr(n)? {
        AttrValue::Num(x) => Some(x),
        _ => None,
    }
}

fn attr_str(e: &dyn Element, n: &str) -> Option<String> {
    match e.get_attr(n)? {
        AttrValue::Str(s) => Some(s),
        AttrValue::Tokens(t) => Some(t.join(" ")),
        other => Some(other.to_string()),
    }
}

fn attr_bool(e: &dyn Element, n: &str) -> Option<bool> {
    match e.get_attr(n)? {
        AttrValue::Bool(b) => Some(b),
        _ => None,
    }
}

fn attr_len(e: &dyn Element, n: &str) -> Option<Length> {
    match e.get_attr(n)? {
        AttrValue::Length(l) => Some(l),
        AttrValue::Num(x) => Some(Length::px(x)),
        _ => None,
    }
}

/// Most nodes a composition may expand into (repeat copies, instances and their contents).
pub const MAX_NODES: usize = 1 << 19;

fn err(code: &str, msg: impl Into<String>, loc: Loc, path: impl Into<String>) -> Diagnostic {
    Diagnostic::error(code, msg, loc, path)
}

fn tokens_of(scene: &m::Scene) -> HashMap<String, [f64; 4]> {
    let mut raw: HashMap<String, String> = HashMap::new();
    if let Some(s) = &scene.styles {
        for c in &s.children {
            if let m::StylesChild::Token(t) = c {
                raw.entry(t.name.clone()).or_insert_with(|| t.value.clone());
            }
        }
    }
    let mut out = HashMap::new();
    for name in raw.keys() {
        let mut cur = name.clone();
        for _ in 0..8 {
            let Some(v) = raw.get(&cur) else { break };
            match <Color as sr_model::parse::ParseValue>::parse_value(v.trim()) {
                Ok(Color::Rgba(c)) => {
                    out.insert(name.clone(), [c.r as f64, c.g as f64, c.b as f64, c.a as f64]);
                    break;
                }
                Ok(Color::Token(t)) => cur = t,
                Err(_) => break,
            }
        }
    }
    out
}

fn markers_of(scene: &m::Scene) -> (HashMap<String, f64>, Option<BeatGrid>) {
    let mut mk = HashMap::new();
    let mut grid = None;
    if let Some(ms) = &scene.markers {
        for c in &ms.children {
            match c {
                m::MarkersChild::Marker(x) => {
                    if let Some(id) = &x.id {
                        mk.insert(id.clone(), x.time);
                    }
                }
                m::MarkersChild::BeatGrid(b) => {
                    grid.get_or_insert(BeatGrid { bpm: b.bpm.get(), offset: b.offset, per_bar: b.beats_per_bar });
                }
            }
        }
    }
    (mk, grid)
}

/// Marker time including generated `beat.N` and `bar.N` ids of the beat grid.
pub fn marker_time(markers: &HashMap<String, f64>, grid: Option<BeatGrid>, id: &str) -> Option<f64> {
    if let Some(t) = markers.get(id) {
        return Some(*t);
    }
    let g = grid?;
    let (kind, n) = id.split_once('.')?;
    let n: f64 = n.parse::<u64>().ok()? as f64;
    let beat = 60.0 / g.bpm;
    match kind {
        "beat" => Some(g.offset + n * beat),
        "bar" => Some(g.offset + n * beat * g.per_bar as f64),
        _ => None,
    }
}

/// Substitutes `{{name}}` and `{{name.field}}` using `lookup`; unknown names are kept.
pub fn substitute(s: &str, lookup: &dyn Fn(&str) -> Option<String>, unknown: &mut dyn FnMut(&str)) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find("}}") {
            Some(j) => {
                let name = after[..j].trim();
                match lookup(name) {
                    Some(v) => out.push_str(&v),
                    None => {
                        unknown(name);
                        out.push_str(&rest[i..i + 2 + j + 2]);
                    }
                }
                rest = &after[j + 2..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn lookup_value(params: &HashMap<String, V>, name: &str) -> Option<V> {
    let (root, field) = match name.split_once('.') {
        Some((r, f)) => (r, Some(f)),
        None => (name, None),
    };
    let v = params.get(root)?;
    match field {
        None => Some(v.clone()),
        Some(f) => match v {
            V::Obj(o) => o.get(f).cloned(),
            _ => None,
        },
    }
}

// ------------------------------------------------------------------ parameters

fn load_data(ds: &m::DataSource, base: &Path) -> Result<Vec<V>, String> {
    let text = match &ds.src {
        Some(src) => {
            let p = match sr_model::assets::resolve(src, base) {
                sr_model::assets::Resolved::Local(p) => p,
                sr_model::assets::Resolved::Remote(s) => {
                    return Err(format!("{s} data sources are not fetched; use a local file"))
                }
            };
            std::fs::read_to_string(&p).map_err(|e| format!("cannot read {}: {e}", p.display()))?
        }
        None => ds.value.clone(),
    };
    data::parse(ds.format, &text)
}

struct ParamDef<'a> {
    p: &'a m::Param,
}

fn convert_param(def: &ParamDef, raw: &str, fps: Fps, asset_ids: &HashSet<String>) -> Result<V, String> {
    let p = def.p;
    use m::ParamKind as K;
    let v = match p.r#type {
        K::String | K::Enum | K::Color | K::Asset => V::Str(raw.into()),
        K::Number => {
            let n = sr_model::xsd::parse_xsd_double(raw.trim())
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("{raw:?} is not a number"))?;
            V::Num(n)
        }
        K::Boolean => match raw.trim() {
            "true" | "1" => V::Bool(true),
            "false" | "0" => V::Bool(false),
            _ => return Err(format!("{raw:?} is not a boolean")),
        },
        K::Time => {
            let t = raw.trim();
            if let Ok(tc) = <sr_model::values::Timecode as sr_model::parse::ParseValue>::parse_value(t) {
                V::Num(tc.to_seconds(fps).map_err(|e| e.to_string())?)
            } else {
                V::Num(
                    sr_model::xsd::parse_xsd_double(t)
                        .filter(|n| n.is_finite())
                        .ok_or_else(|| format!("{raw:?} is not a time in seconds or HH:MM:SS:FF"))?,
                )
            }
        }
        K::List => data::parse_list(raw),
    };
    match p.r#type {
        K::Number | K::Time => {
            let n = v.num();
            if let Some(lo) = p.min.filter(|lo| n < *lo) {
                return Err(format!("{n} is below the minimum {lo}"));
            }
            if let Some(hi) = p.max.filter(|hi| n > *hi) {
                return Err(format!("{n} is above the maximum {hi}"));
            }
        }
        K::Color => {
            <Color as sr_model::parse::ParseValue>::parse_value(raw.trim()).map_err(|e| e.to_string())?;
        }
        K::Asset => {
            if !asset_ids.contains(raw.trim()) {
                return Err(format!("{raw:?} is not an asset id"));
            }
        }
        K::Enum => {
            let opts: Vec<&str> =
                p.options.as_deref().unwrap_or("").split(',').map(str::trim).filter(|o| !o.is_empty()).collect();
            if !opts.is_empty() && !opts.contains(&raw) {
                return Err(format!("{raw:?} is not one of: {}", opts.join(", ")));
            }
        }
        _ => {}
    }
    if matches!(p.r#type, K::String | K::Enum) {
        if let Some(max) = p.max_length {
            let n = raw.chars().count() as u64;
            if n > max {
                return Err(format!("{n} characters exceed maxLength {max}"));
            }
        }
        if let Some(pat) = &p.pattern {
            let re = regex::Regex::new(&format!("^(?:{pat})$"))
                .map_err(|e| format!("pattern {pat:?} does not compile: {e}"))?;
            if !re.is_match(raw) {
                return Err(format!("{raw:?} does not match the pattern {pat:?}"));
            }
        }
    }
    Ok(v)
}

fn param_string(v: &V) -> String {
    match v {
        V::Arr(a) => a.iter().map(V::to_js_string).collect::<Vec<_>>().join(","),
        other => other.to_js_string(),
    }
}

fn apply_override(
    scene: &mut m::Scene,
    target: &str,
    property: &str,
    value: &str,
    loc: Loc,
    what: &str,
    diags: &mut Vec<Diagnostic>,
) {
    let mut found = false;
    let mut error = None;
    walk_mut(scene, &mut |e| {
        if !found && e.element_id() == Some(target) {
            found = true;
            if let Err(x) = e.set_attr(property, value) {
                error = Some(x.to_string());
            }
        }
    });
    if !found {
        diags.push(err("E05", format!("{what}: no element has id {target:?}"), loc, target));
    } else if let Some(e) = error {
        diags.push(err("E06", format!("{what} of {target:?}: {e}"), loc, target));
    }
}

/// Resolved parameters and the templated scene.
struct Templated {
    scene: m::Scene,
    params: HashMap<String, V>,
    size: [f64; 2],
    reframe: Option<Reframe>,
}

/// How a layout that does not reflow fits the composition, rendered at the project size, into its frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reframe {
    /// `crop`, `fit` or `fit-blur`.
    pub mode: m::Reframe,
    /// `focusX`, `focusY`: where the composition is anchored in the frame (0‥1).
    pub focus: [f64; 2],
    /// The layout's frame size.
    pub size: [f64; 2],
}

fn template(
    doc: &Document,
    opts: &EvalOptions,
    diags: &mut Vec<Diagnostic>,
    warnings: &mut Vec<Diagnostic>,
) -> Templated {
    let mut scene = doc.scene.clone();
    let fps = scene.project.fps;
    let mut raw: HashMap<String, Option<String>> = HashMap::new();
    let mut defs: Vec<m::Param> = Vec::new();
    let mut variants: HashMap<String, m::Variant> = HashMap::new();
    let mut sources: Vec<m::DataSource> = Vec::new();
    let mut binds: Vec<m::Bind> = Vec::new();
    if let Some(p) = &scene.parameters {
        for c in &p.children {
            match c {
                m::ParametersChild::Param(x) => {
                    raw.insert(x.id.clone(), x.default.clone());
                    defs.push(x.clone());
                }
                m::ParametersChild::Variant(v) => {
                    variants.insert(v.id.clone(), v.clone());
                }
                m::ParametersChild::Data(d) => sources.push(d.clone()),
                m::ParametersChild::Bind(b) => binds.push(b.clone()),
            }
        }
    }
    let variant = match &opts.variant {
        Some(id) => match variants.get(id) {
            Some(v) => Some(v.clone()),
            None => {
                let known: Vec<&str> = variants.keys().map(String::as_str).collect();
                diags.push(err(
                    "E10",
                    format!(
                        "no variant {id:?}; variants: {}",
                        if known.is_empty() { "none".into() } else { known.join(", ") }
                    ),
                    Loc::default(),
                    "/scene/parameters",
                ));
                None
            }
        },
        None => None,
    };
    if let Some(v) = &variant {
        for c in &v.children {
            if let m::VariantChild::Set(s) = c {
                raw.insert(s.param.clone(), Some(s.value.clone()));
            }
        }
    }
    if let Some((data_id, row)) = &opts.row {
        let ds = match data_id {
            Some(id) => sources.iter().find(|d| &d.id == id),
            None => sources.first(),
        };
        match ds {
            None => diags.push(err(
                "E09",
                format!("no data source {}", data_id.as_deref().unwrap_or("(none declared)")),
                Loc::default(),
                "/scene/parameters",
            )),
            Some(ds) => match load_data(ds, doc.base_dir()) {
                Err(e) => diags.push(err("E09", format!("data source {:?}: {e}", ds.id), ds.loc, ds.id.as_str())),
                Ok(rows) => match rows.get(*row) {
                    None => diags.push(err(
                        "E09",
                        format!("data source {:?} has {} rows; row {row} does not exist", ds.id, rows.len()),
                        ds.loc,
                        ds.id.as_str(),
                    )),
                    Some(V::Obj(o)) => {
                        for (k, v) in o.iter() {
                            if raw.contains_key(k) {
                                raw.insert(k.clone(), Some(param_string(v)));
                            }
                        }
                    }
                    Some(_) => diags.push(err(
                        "E09",
                        format!("row {row} of {:?} is not a record", ds.id),
                        ds.loc,
                        ds.id.as_str(),
                    )),
                },
            },
        }
    }
    for (k, v) in &opts.params {
        if raw.contains_key(k) {
            raw.insert(k.clone(), Some(v.clone()));
        } else {
            let hint = crate::suggest(k, raw.keys().map(String::as_str))
                .map(|s| format!("; did you mean {s:?}?"))
                .unwrap_or_default();
            diags.push(err(
                "E07",
                format!("--param {k}: the document declares no parameter {k:?}{hint}"),
                Loc::default(),
                "/scene/parameters",
            ));
        }
    }
    let asset_ids: HashSet<String> =
        scene.assets.iter().flat_map(|a| a.children.iter()).filter_map(|c| c.id().map(str::to_string)).collect();
    let mut params = HashMap::new();
    for d in &defs {
        match raw.get(&d.id).cloned().flatten() {
            Some(r) => match convert_param(&ParamDef { p: d }, &r, fps, &asset_ids) {
                Ok(v) => {
                    params.insert(d.id.clone(), v);
                }
                Err(e) => diags.push(err("E07", format!("param {:?}: {e}", d.id), d.loc, d.id.as_str())),
            },
            None => {
                if d.required {
                    diags.push(
                        err("E08", format!("param {:?} is required and has no value", d.id), d.loc, d.id.as_str())
                            .with_help(format!("pass --param {}=<value>, or set it in a variant or data row", d.id)),
                    );
                }
                params
                    .insert(d.id.clone(), if d.r#type == m::ParamKind::String { V::Str("".into()) } else { V::Undef });
            }
        }
    }
    // data sources are also visible to repeat/@over through params
    for ds in &sources {
        match load_data(ds, doc.base_dir()) {
            Ok(rows) => {
                params.entry(ds.id.clone()).or_insert(V::Arr(rows.into()));
            }
            Err(e) => diags.push(err("E09", format!("data source {:?}: {e}", ds.id), ds.loc, ds.id.as_str())),
        }
    }
    for b in &binds {
        let Some(v) = params.get(&b.param) else { continue };
        let mut s = param_string(v);
        if let Some(map) = &b.map {
            for pair in map.split(';') {
                if let Some((from, to)) = pair.split_once('=') {
                    if from.trim() == s {
                        s = to.trim().to_string();
                        break;
                    }
                }
            }
        }
        apply_override(&mut scene, &b.target, &b.property, &s, b.loc, "bind", diags);
    }
    if let Some(v) = &variant {
        for c in &v.children {
            if let m::VariantChild::Override(o) = c {
                apply_override(&mut scene, &o.target, &o.property, &o.value, o.loc, "variant override", diags);
            }
        }
    }
    let mut size = [scene.project.width as f64, scene.project.height as f64];
    let mut reframe = None;
    if let Some(l) = &opts.layout {
        match scene.layouts.as_ref().and_then(|ls| ls.layouts.iter().find(|x| &x.id == l)).cloned() {
            Some(layout) => {
                let frame = [layout.width as f64, layout.height as f64];
                if layout.reframe == m::Reframe::Reflow {
                    size = frame;
                } else {
                    // the composition renders at the project size; delivery fits it into the frame
                    reframe = Some(Reframe {
                        mode: layout.reframe,
                        focus: [layout.focus_x.get(), layout.focus_y.get()],
                        size: frame,
                    });
                }
                for o in &layout.overrides {
                    apply_override(&mut scene, &o.target, &o.property, &o.value, o.loc, "layout override", diags);
                }
            }
            None => diags.push(err("E10", format!("no layout {l:?}"), Loc::default(), "/scene/layouts")),
        }
    }
    // {{placeholder}} substitution in text-bearing attributes
    let mut repeat_vars: HashSet<String> = ["index".to_string(), "count".to_string()].into();
    walk(&scene, &mut |e| {
        if e.element_name() == "repeat" {
            if let Some(v) = attr_str(e, "var") {
                repeat_vars.insert(v);
            }
        }
    });
    let lookup = |name: &str| lookup_value(&params, name).map(|v| param_string(&v));
    walk_mut(&mut scene, &mut |e| {
        let loc = e.loc();
        ignored_attribute(e, &mut *warnings);
        masks_that_miss(e, &mut *warnings);
        let mut unknown = |name: &str| {
            let root = name.split('.').next().unwrap_or(name);
            if !repeat_vars.contains(root) {
                warnings.push(Diagnostic::warning(
                    "E16",
                    format!("{{{{{name}}}}} names no parameter; the text is left unchanged"),
                    loc,
                    name,
                ));
            }
        };
        for a in ["text", "data", "tex", "value", "label", "prompt"] {
            if let Some(AttrValue::Str(s)) = e.get_attr(a) {
                if s.contains("{{")
                    && matches!(
                        e.element_name(),
                        "text"
                            | "textAssetType"
                            | "object3D"
                            | "object3DType"
                            | "code"
                            | "codeAssetType"
                            | "formula"
                            | "formulaAssetType"
                            | "meta"
                            | "metaType"
                            | "word"
                            | "wordType"
                    )
                {
                    let n = substitute(&s, &lookup, &mut unknown);
                    let _ = e.set_attr(a, &n);
                }
            }
        }
        if let Some(t) = e.text() {
            if t.contains("{{") && matches!(e.element_name(), "span" | "spanType") {
                let n = substitute(t, &lookup, &mut unknown);
                e.set_text(n);
            }
        }
    });
    Templated { scene, params, size, reframe }
}

// ------------------------------------------------------------------ builder

struct DocCtx {
    scene: Arc<m::Scene>,
    ns: Arc<str>,
    base: PathBuf,
    tokens: HashMap<String, [f64; 4]>,
    markers: HashMap<String, f64>,
    grid: Option<BeatGrid>,
    duration: f64,
    size: [f64; 2],
}

#[derive(Clone)]
struct Ctx {
    scope: Arc<str>,
    doc: u16,
    repeat: Option<(u32, u32, Arc<str>, V)>,
    stack: Vec<String>,
}

struct Builder {
    docs: Vec<DocCtx>,
    nodes: Vec<InstNode>,
    slots: Vec<Slot>,
    slot_ix: HashMap<(Owner, Arc<str>), u32>,
    channels: Vec<Channel>,
    exprs: Vec<ExprInst>,
    links: Vec<LinkInst>,
    elements: Vec<ElemTarget>,
    transitions: Vec<TransitionInst>,
    ids: HashMap<Arc<str>, u32>,
    diags: Vec<Diagnostic>,
    warnings: Vec<Diagnostic>,
    /// (slot or None for a condition, owning node, source, seed, location, scope, document)
    pending_expr: Vec<(Option<u32>, Option<u32>, String, u64, Loc, Arc<str>, u16)>,
    pending_link: Vec<(u32, m::Link, Option<u32>, Arc<str>, u16)>,
    project_seed: u64,
    params: HashMap<String, V>,
    assets: HashMap<Arc<str>, (u16, String)>,
    analysis_tracks: HashSet<String>,
}

fn strip(n: &Node) -> Node {
    let mut n = n.clone();
    match &mut n {
        Node::Group(g) => g.children.retain(|c| !matches!(c, m::GroupChild::Node(_))),
        Node::Sequence(g) => g.children.retain(|c| !matches!(c, m::GroupChild::Node(_))),
        Node::Repeat(r) => r.children.retain(|c| !matches!(c, m::RepeatChild::Node(_))),
        _ => {}
    }
    n
}

fn node_children(n: &Node) -> Vec<Node> {
    n.child_nodes().cloned().collect()
}

const ANIM: [&str; 4] = ["animate", "expression", "motionPath", "link"];

fn scoped(scope: &str, id: &str) -> Arc<str> {
    if scope.is_empty() {
        id.into()
    } else {
        format!("{scope}/{id}").into()
    }
}

impl Builder {
    fn doc(&self, i: u16) -> &DocCtx {
        &self.docs[i as usize]
    }

    fn marker(&self, doc: u16, id: &str) -> Option<f64> {
        let d = self.doc(doc);
        marker_time(&d.markers, d.grid, id)
    }

    /// Resolves an id from `scope` outward.
    fn resolve(&self, scope: &str, id: &str) -> Option<u32> {
        let mut s = scope.to_string();
        loop {
            let key = if s.is_empty() { id.to_string() } else { format!("{s}/{id}") };
            if let Some(&n) = self.ids.get(key.as_str()) {
                return Some(n);
            }
            if s.is_empty() {
                return None;
            }
            s = match s.rfind('/') {
                Some(i) => s[..i].to_string(),
                None => String::new(),
            };
        }
    }

    /// The node that owns the crater `id`, found from `scope` outward like any id.
    fn resolve_crater(&self, scope: &str, id: &str) -> Option<u32> {
        let mut s = scope.to_string();
        loop {
            let found = self.nodes.iter().position(|n| {
                n.scope.as_ref() == s.as_str()
                    && children(&*n.elem)
                        .into_iter()
                        .any(|c| c.element_name() == "crater" && attr_str(c, "id").as_deref() == Some(id))
            });
            if let Some(k) = found {
                return Some(k as u32);
            }
            if s.is_empty() {
                return None;
            }
            s = match s.rfind('/') {
                Some(i) => s[..i].to_string(),
                None => String::new(),
            };
        }
    }

    fn media_len(&self, doc: u16, asset: &str) -> Option<f64> {
        let a = self.doc(doc).scene.assets.as_ref()?.children.iter().find(|c| c.id() == Some(asset))?;
        match a {
            m::AssetsChild::Video(v) => Some(v.duration.get()),
            m::AssetsChild::ImageSequence(s) => {
                let frames = (s.last.saturating_sub(s.first) as f64 / s.step as f64).floor() + 1.0;
                Some(frames.max(0.0) / s.fps.as_f64())
            }
            _ => None,
        }
    }

    /// Whether `more` nodes still fit; reports E18 (once) when they do not.
    fn room(&mut self, more: u64, loc: Loc, path: &str) -> bool {
        if more <= (MAX_NODES.saturating_sub(self.nodes.len())) as u64 {
            return true;
        }
        if !self.diags.iter().any(|d| d.code == "E18") {
            let msg = format!("{path:?} expands the composition past {MAX_NODES} nodes");
            self.diags.push(err("E18", msg, loc, path));
        }
        false
    }

    fn instantiate(&mut self, list: &[Node], parent: Option<u32>, ctx: &Ctx) -> Vec<u32> {
        let mut out = Vec::new();
        let mut tcount = 0;
        for n in list {
            let name = n.element_name();
            let id: Arc<str> = match n.id() {
                Some(id) => scoped(&ctx.scope, id),
                None => {
                    tcount += 1;
                    let p = parent.map(|p| self.nodes[p as usize].id.to_string()).unwrap_or_default();
                    format!("{p}~{name}{tcount}").into()
                }
            };
            // object3D/@instances: one node per copy, each evaluated with its own index and count
            // copy 0 keeps the id, copy k is `id[k]`.
            let copies = if name == "object3D" { attr_num(n, "instances").unwrap_or(1.0).max(1.0) as u32 } else { 1 };
            if !self.room(copies as u64, Loc::default(), &id) {
                break;
            }
            if copies > 1 {
                let mut one = n.clone();
                let _ = one.set_attr("instances", "1");
                for k in 0..copies {
                    let (var, item) = match &ctx.repeat {
                        Some((_, _, v, it)) => (v.clone(), it.clone()),
                        None => ("index".into(), V::Num(k as f64)),
                    };
                    let cctx = Ctx { repeat: Some((k, copies, var, item)), ..ctx.clone() };
                    let got = self.instantiate(std::slice::from_ref(&one), parent, &cctx);
                    let cid: Arc<str> = if k == 0 { id.clone() } else { format!("{id}[{k}]").into() };
                    for &c in &got {
                        let node = &mut self.nodes[c as usize];
                        node.id = cid.clone();
                        if attr_num(n, "seed").is_none() {
                            node.seed = crate::rng::hash_str(&cid);
                        }
                        if n.id().is_some() && k > 0 {
                            self.ids.insert(cid.clone(), c);
                        }
                    }
                    out.extend(got);
                }
                if n.id().is_some() {
                    self.ids.insert(id.clone(), out[out.len() - copies as usize]);
                }
                continue;
            }
            let idx = self.nodes.len() as u32;
            let e: &dyn Element = n;
            let tf = Tf {
                pos: TF_POS.map(|p| attr_len(e, p).unwrap_or(Length::px(0.0))),
                num: [
                    attr_num(e, "rotation").unwrap_or(0.0),
                    attr_num(e, "scaleX").unwrap_or(1.0),
                    attr_num(e, "scaleY").unwrap_or(1.0),
                    attr_num(e, "skewX").unwrap_or(0.0),
                    attr_num(e, "skewY").unwrap_or(0.0),
                    attr_num(e, "opacity").unwrap_or(1.0),
                    attr_num(e, "zDepth").unwrap_or(0.0),
                    attr_num(e, "rotationX").unwrap_or(0.0),
                    attr_num(e, "rotationY").unwrap_or(0.0),
                ],
            };
            let mut start = attr_num(e, "start").unwrap_or(0.0);
            if let Some(mk) = attr_str(e, "startMarker") {
                start += self.marker(ctx.doc, &mk).unwrap_or(0.0);
            }
            let mut end = attr_num(e, "end");
            if let Some(mk) = attr_str(e, "endMarker") {
                end = Some(self.marker(ctx.doc, &mk).unwrap_or(0.0) + end.unwrap_or(0.0));
            }
            let seed = attr_num(e, "seed").map(|s| s as u64).unwrap_or_else(|| crate::rng::hash_str(&id));
            let asset_ref = match name {
                "layer" => attr_str(e, "asset"),
                // Shared assets retain their namespace when objects occur in included documents.
                "object3D" if attr_str(e, "primitive").as_deref() == Some("volume") => attr_str(e, "volume"),
                "object3D" | "particles3D" => attr_str(e, "mesh"),
                "ocean" => attr_str(e, "bathymetry"),
                _ => None,
            };
            let asset = asset_ref.map(|a| {
                let ns = &self.doc(ctx.doc).ns;
                let key: Arc<str> = if ns.is_empty() { a.as_str().into() } else { format!("{ns}/{a}").into() };
                self.assets.insert(key.clone(), (ctx.doc, a));
                key
            });
            if matches!(name, "particleEmitter" | "particles3D") {
                // sprites and emission masks are image assets too
                for r in [attr_str(e, "sprite"), attr_str(e, "emitterAsset"), attr_str(e, "emitterMesh")]
                    .into_iter()
                    .flatten()
                {
                    let ns = &self.doc(ctx.doc).ns;
                    let key: Arc<str> = if ns.is_empty() { r.as_str().into() } else { format!("{ns}/{r}").into() };
                    self.assets.insert(key, (ctx.doc, r));
                }
            }
            if name == "object3D" {
                if let Some(r) = attr_str(e, "terrain") {
                    let ns = &self.doc(ctx.doc).ns;
                    let key: Arc<str> = if ns.is_empty() { r.as_str().into() } else { format!("{ns}/{r}").into() };
                    self.assets.insert(key, (ctx.doc, r));
                }
                for pyro in sr_model::element::children(e).into_iter().filter(|c| c.element_name() == "pyro") {
                    for source in sr_model::element::children(pyro) {
                        if let Some(r) = attr_str(source, "mesh") {
                            let ns = &self.doc(ctx.doc).ns;
                            let key: Arc<str> =
                                if ns.is_empty() { r.as_str().into() } else { format!("{ns}/{r}").into() };
                            self.assets.insert(key, (ctx.doc, r));
                        }
                    }
                }
            }
            let node = InstNode {
                id: id.clone(),
                scope: ctx.scope.clone(),
                name,
                elem: Arc::new(strip(n)),
                kind: Kind::Plain,
                parent,
                children: Vec::new(),
                doc: ctx.doc,
                start,
                end,
                vis_start: f64::NAN,
                vis_end: None,
                clock: Clock::Same,
                media: None,
                tf,
                tf_slots: TfSlots::default(),
                z: if matches!(name, "object3D" | "particles3D" | "ocean" | "camera") {
                    0
                } else {
                    attr_num(e, "z").unwrap_or(0.0) as i32
                },
                z_slot: None,
                restack: false,
                three_d: attr_bool(e, "threeD").unwrap_or(false),
                matte_visible: attr_bool(e, "matteVisible").unwrap_or(false),
                visible: attr_bool(e, "visible").unwrap_or(true)
                    && (name != "camera" || attr_bool(e, "active").unwrap_or(true)),
                cond: None,
                parent_link: None,
                matte: None,
                copy: None,
                motion: None,
                box_size: None,
                asset,
                text: None,
                slots: Vec::new(),
                parts: Vec::new(),
                seed,
                repeat: ctx.repeat.clone(),
                fit: None,
                asset_size: None,
                asset_kind: None,
                shape_size: None,
                layout: None,
                align: None,
                doc_children: Vec::new(),
                clip: attr_bool(e, "clip").unwrap_or(false),
            };
            self.nodes.push(node);
            if matches!(name, "flock" | "fluid" | "slime" | "erosion") {
                // simulations draw in their width × height box
                let (w, h) = (attr_num(e, "width").unwrap_or(1.0), attr_num(e, "height").unwrap_or(1.0));
                self.nodes[idx as usize].box_size = Some([Length::px(w), Length::px(h)]);
            }
            self.statics(idx, n, ctx.doc);
            if n.id().is_some() {
                self.ids.insert(id.clone(), idx);
            }
            out.push(idx);
            match n {
                Node::Group(g) => {
                    self.group_clock(idx, g.time_offset, g.time_scale.get());
                    self.nodes[idx as usize].box_size = g.width.zip(g.height).map(|(w, h)| [w, h]);
                    let kids = node_children(n);
                    let c = self.instantiate(&kids, Some(idx), ctx);
                    self.nodes[idx as usize].children = c;
                }
                Node::Sequence(s) => {
                    self.group_clock(idx, s.time_offset, s.time_scale.get());
                    self.nodes[idx as usize].box_size = s.width.zip(s.height).map(|(w, h)| [w, h]);
                    let kids = node_children(n);
                    let c = self.instantiate(&kids, Some(idx), ctx);
                    self.nodes[idx as usize].children = c;
                    self.place_sequence(idx, s);
                }
                Node::Repeat(r) => self.repeat(idx, r, n, ctx),
                Node::Instance(i) => self.instance(idx, i, ctx),
                Node::Include(i) => self.include(idx, i, ctx),
                Node::Layer(l) => self.layer_clock(idx, l, ctx),
                _ => {}
            }
        }
        out
    }

    /// Static layout, fit and size data of a node.
    fn statics(&mut self, idx: u32, n: &Node, doc: u16) {
        use crate::layout::{AlignSpec, FitSpec, LayoutSpec};
        let e: &dyn Element = n;
        let align = {
            let x =
                attr_str(e, "alignX").and_then(|v| <m::AlignX as sr_model::parse::ParseValue>::parse_value(&v).ok());
            let y =
                attr_str(e, "alignY").and_then(|v| <m::AlignY as sr_model::parse::ParseValue>::parse_value(&v).ok());
            (x.is_some() || y.is_some()).then(|| AlignSpec {
                x,
                y,
                to: attr_str(e, "alignTo")
                    .and_then(|v| <m::AlignTo as sr_model::parse::ParseValue>::parse_value(&v).ok())
                    .unwrap_or(m::AlignTo::Parent),
                margin: attr_len(e, "margin").unwrap_or(Length::px(0.0)),
            })
        };
        let layout = match n {
            Node::Group(g) if g.layout != m::GroupLayout::None => Some(LayoutSpec {
                kind: g.layout,
                gap: g.gap,
                padding: g.padding,
                justify: g.justify,
                align_items: g.align_items,
                columns: g.grid_columns as u32,
            }),
            Node::Sequence(g) if g.layout != m::GroupLayout::None => Some(LayoutSpec {
                kind: g.layout,
                gap: g.gap,
                padding: g.padding,
                justify: g.justify,
                align_items: g.align_items,
                columns: g.grid_columns as u32,
            }),
            _ => None,
        };
        let shape_size = match n {
            Node::Shape(s) => Some([s.width, s.height]),
            _ => None,
        };
        let (mut asset_size, mut asset_kind, mut fit) = (None, None, None);
        if let Node::Layer(l) = n {
            if let Some(a) = self
                .doc(doc)
                .scene
                .assets
                .as_ref()
                .and_then(|a| a.children.iter().find(|c| c.id() == Some(l.asset.as_str())))
            {
                asset_kind = Some(a.element_name());
                let wh = |w: Option<f64>, h: Option<f64>| w.zip(h).map(|(w, h)| [w, h]);
                asset_size = match a {
                    m::AssetsChild::Image(x) => wh(Some(x.width as f64), Some(x.height as f64)),
                    m::AssetsChild::Video(x) => wh(Some(x.width as f64), Some(x.height as f64)),
                    m::AssetsChild::ImageSequence(x) => wh(Some(x.width as f64), Some(x.height as f64)),
                    m::AssetsChild::Text(x) => wh(Some(x.width as f64), Some(x.height as f64)),
                    m::AssetsChild::Vector(x) => wh(Some(x.width as f64), Some(x.height as f64)),
                    m::AssetsChild::Lottie(x) => wh(Some(x.width as f64), Some(x.height as f64)),
                    m::AssetsChild::Generator(x) => wh(Some(x.width as f64), Some(x.height as f64)),
                    m::AssetsChild::Chart(x) => wh(Some(x.width as f64), Some(x.height as f64)),
                    m::AssetsChild::Map(x) => wh(Some(x.width as f64), Some(x.height as f64)),
                    m::AssetsChild::Audiogram(x) => wh(Some(x.width as f64), Some(x.height as f64)),
                    m::AssetsChild::Code(x) => wh(Some(x.width as f64), Some(x.height as f64)),
                    m::AssetsChild::Formula(x) => wh(Some(x.width as f64), Some(x.height as f64)),
                    m::AssetsChild::Generated(x) => wh(x.width.map(|v| v as f64), x.height.map(|v| v as f64)),
                    _ => None,
                };
            }
            fit = Some(FitSpec {
                fit: l.fit,
                box_w: l.box_width,
                box_h: l.box_height,
                focus: [l.focus_x.get(), l.focus_y.get()],
                crop: [l.crop_left.get(), l.crop_top.get(), l.crop_right.get(), l.crop_bottom.get()],
                flip: [l.flip_x, l.flip_y],
            });
        }
        let node = &mut self.nodes[idx as usize];
        node.align = align;
        node.layout = layout;
        node.shape_size = shape_size;
        node.asset_size = asset_size;
        node.asset_kind = asset_kind;
        node.fit = fit;
    }

    fn group_clock(&mut self, idx: u32, offset: f64, scale: f64) {
        let node = &mut self.nodes[idx as usize];
        if offset != 0.0 || scale != 1.0 {
            node.clock = Clock::Affine { origin: node.start, offset, scale };
        }
    }

    fn layer_clock(&mut self, idx: u32, l: &m::Layer, ctx: &Ctx) {
        let len = self
            .media_len(ctx.doc, &l.asset)
            .map(|full| (l.clip_out.unwrap_or(full) - l.clip_in).max(0.0))
            .or(l.clip_out.map(|o| (o - l.clip_in).max(0.0)));
        let rate = l.speed / l.time_stretch.get();
        let mut remap = None;
        for c in &l.children {
            if let m::LayerChild::TimeRemap(tr) = c {
                let spec = ChannelSpec {
                    keys: &tr.keys,
                    default: tr.default_interpolation,
                    before: m::Extrapolation::Hold,
                    after: m::Extrapolation::Hold,
                    additive: false,
                    time_base: m::TimeBase::Local,
                    kind: PropKind::Number(Default::default()),
                };
                match Channel::compile(&spec, &DocLookup { b: self, doc: ctx.doc }) {
                    Ok(ch) => remap = Some(ch),
                    Err(e) => {
                        self.diags.push(err("E04", format!("timeRemap of {:?}: {e}", l.id), tr.loc, l.id.as_str()))
                    }
                }
            }
        }
        let start = self.nodes[idx as usize].start;
        // a negative speed plays backwards
        let clock = MediaClock {
            start,
            rate: rate.abs(),
            clip_in: l.clip_in,
            len,
            loops: l.r#loop,
            reverse: l.reverse != (rate < 0.0),
            freeze_at: l.freeze_at,
            remap,
        };
        let node = &mut self.nodes[idx as usize];
        // a clip ends when its media runs out, unless it is remapped or frozen on one frame
        if node.end.is_none() && clock.remap.is_none() && clock.freeze_at.is_none() {
            if let Some(len) = clock.len.filter(|_| rate != 0.0) {
                node.end = Some(start + len * (clock.loops + 1) as f64 / rate.abs());
            }
        }
        node.media = Some(clock);
        // layer text with per-copy placeholders
        let asset_text = self.doc(ctx.doc).scene.assets.as_ref().and_then(|a| {
            a.children.iter().find_map(|c| match c {
                m::AssetsChild::Text(t) if t.id == l.asset => t.text.clone(),
                _ => None,
            })
        });
        if let Some(t) = asset_text {
            let text = if t.contains("{{") {
                let rp = ctx.repeat.clone();
                let params = &self.params;
                let look = |name: &str| -> Option<String> {
                    if let Some((i, c, var, item)) = &rp {
                        let (root, field) = name.split_once('.').map(|(a, b)| (a, Some(b))).unwrap_or((name, None));
                        match root {
                            "index" => return Some(i.to_string()),
                            "count" => return Some(c.to_string()),
                            r if r == &**var => {
                                return match (field, item) {
                                    (None, v) => Some(param_string(v)),
                                    (Some(f), V::Obj(o)) => o.get(f).map(param_string),
                                    _ => None,
                                };
                            }
                            _ => {}
                        }
                    }
                    lookup_value(params, name).map(|v| param_string(&v))
                };
                substitute(&t, &look, &mut |_| {})
            } else {
                t
            };
            self.nodes[idx as usize].text = Some(text.into());
        }
    }

    fn repeat(&mut self, idx: u32, r: &m::Repeat, n: &Node, ctx: &Ctx) {
        let items: Vec<V> = match (&r.count, &r.over) {
            (Some(c), _) if !self.room(*c, r.loc, r.id.as_str()) => Vec::new(),
            (Some(c), _) => {
                let at = |k: u64| k.checked_mul(r.step).and_then(|x| x.checked_add(r.from));
                (0..*c)
                    .map(|k| V::Num(at(k).map(|v| v as f64).unwrap_or(r.from as f64 + k as f64 * r.step as f64)))
                    .collect()
            }
            (None, Some(over)) => match self.params.get(over) {
                Some(V::Arr(a)) => a.iter().skip(r.from as usize).step_by(r.step.max(1) as usize).cloned().collect(),
                _ => {
                    self.diags.push(err(
                        "E10",
                        format!("repeat {:?}: @over {over:?} is not a list parameter or data source", r.id),
                        r.loc,
                        r.id.as_str(),
                    ));
                    Vec::new()
                }
            },
            _ => Vec::new(),
        };
        let kids = node_children(n);
        let count = items.len() as u32;
        let mut copies = Vec::new();
        let rid = self.nodes[idx as usize].id.clone();
        let var: Arc<str> = r.var.as_str().into();
        if !self.room(items.len() as u64, r.loc, r.id.as_str()) {
            return;
        }
        for (k, item) in items.into_iter().enumerate() {
            let k = k as u32;
            let cid: Arc<str> = format!("{rid}[{k}]").into();
            let cidx = self.nodes.len() as u32;
            let mut copy = self.nodes[idx as usize].clone();
            copy.id = cid.clone();
            copy.name = "copy";
            copy.kind = Kind::RepeatCopy { index: k, count };
            copy.parent = Some(idx);
            copy.children = Vec::new();
            copy.start = 0.0;
            copy.end = None;
            copy.tf = Tf { pos: [Length::px(0.0); 4], num: [0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0] };
            copy.tf_slots = TfSlots::default();
            copy.z = 0;
            copy.visible = true;
            copy.box_size = None;
            copy.slots = Vec::new();
            copy.parts = Vec::new();
            let kf = k as f64;
            copy.copy = Some([
                kf * r.offset_x,
                kf * r.offset_y,
                kf * r.rotation_step,
                libm::pow(r.scale_step.get(), kf),
                (1.0 - kf * r.opacity_step.get()).max(0.0),
            ]);
            copy.clock = if r.time_step != 0.0 {
                Clock::Affine { origin: 0.0, offset: kf * r.time_step, scale: 1.0 }
            } else {
                Clock::Same
            };
            copy.repeat = Some((k, count, var.clone(), item.clone()));
            copy.seed = crate::rng::hash_str(&cid);
            copy.cond = None;
            copy.align = None;
            copy.layout = None;
            copy.fit = None;
            copy.asset_size = None;
            copy.asset_kind = None;
            copy.shape_size = None;
            copy.clip = false;
            self.nodes.push(copy);
            self.ids.insert(cid.clone(), cidx);
            let cctx = Ctx { scope: cid.clone(), repeat: Some((k, count, var.clone(), item)), ..ctx.clone() };
            let c = self.instantiate(&kids, Some(cidx), &cctx);
            self.nodes[cidx as usize].children = c;
            copies.push(cidx);
        }
        self.nodes[idx as usize].children = copies;
    }

    fn apply_scoped_overrides(&mut self, nodes: &mut [Node], overrides: &[m::Override], owner: &str) {
        for o in overrides {
            let mut found = false;
            let mut error = None;
            for n in nodes.iter_mut() {
                walk_mut(n, &mut |e| {
                    if !found && e.element_id() == Some(o.target.as_str()) {
                        found = true;
                        if let Err(x) = e.set_attr(&o.property, &o.value) {
                            error = Some(x.to_string());
                        }
                    }
                });
            }
            if !found {
                self.diags.push(err(
                    "E05",
                    format!("override in {owner:?}: no element {:?} inside it", o.target),
                    o.loc,
                    owner,
                ));
            } else if let Some(e) = error {
                self.diags.push(err("E06", format!("override of {:?} in {owner:?}: {e}", o.target), o.loc, owner));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn symbol_clock(
        &mut self,
        idx: u32,
        len: f64,
        speed: f64,
        clip_in: f64,
        loops: u64,
        reverse: bool,
        remap: Option<Channel>,
    ) {
        let start = self.nodes[idx as usize].start;
        let len = (len - clip_in).max(0.0);
        let (rate, reverse) = (speed.abs(), reverse != (speed < 0.0));
        let clock = MediaClock { start, rate, clip_in, len: Some(len), loops, reverse, freeze_at: None, remap };
        let node = &mut self.nodes[idx as usize];
        if node.end.is_none() && clock.remap.is_none() && speed != 0.0 {
            node.end = Some(start + len * (loops + 1) as f64 / speed.abs());
        }
        node.clock = Clock::Media(Box::new(clock));
    }

    fn instance(&mut self, idx: u32, inst: &m::Instance, ctx: &Ctx) {
        let doc = self.doc(ctx.doc).scene.clone();
        let Some(sym) = doc.symbols.as_ref().and_then(|s| s.symbols.iter().find(|x| x.id == inst.symbol)) else {
            return;
        };
        let key = format!("{}#{}", ctx.doc, sym.id);
        if ctx.stack.contains(&key) {
            self.diags.push(err(
                "E13",
                format!("instance {:?} instantiates symbol {:?} inside itself", inst.id, sym.id),
                inst.loc,
                &*self.nodes[idx as usize].id,
            ));
            return;
        }
        let mut kids = sym.children.clone();
        let overrides: Vec<m::Override> = inst
            .children
            .iter()
            .filter_map(|c| if let m::InstanceChild::Override(o) = c { Some(o.clone()) } else { None })
            .collect();
        let id = self.nodes[idx as usize].id.clone();
        self.apply_scoped_overrides(&mut kids, &overrides, &id);
        let mut remap = None;
        for c in &inst.children {
            if let m::InstanceChild::TimeRemap(tr) = c {
                let spec = ChannelSpec {
                    keys: &tr.keys,
                    default: tr.default_interpolation,
                    before: m::Extrapolation::Hold,
                    after: m::Extrapolation::Hold,
                    additive: false,
                    time_base: m::TimeBase::Local,
                    kind: PropKind::Number(Default::default()),
                };
                match Channel::compile(&spec, &DocLookup { b: self, doc: ctx.doc }) {
                    Ok(ch) => remap = Some(ch),
                    Err(e) => self.diags.push(err(
                        "E04",
                        format!("timeRemap of {:?}: {e}", inst.id),
                        tr.loc,
                        inst.id.as_str(),
                    )),
                }
            }
        }
        let dur = sym.duration.map(|d| d.get()).unwrap_or(self.doc(ctx.doc).duration);
        self.symbol_clock(idx, dur, inst.speed, inst.clip_in, inst.r#loop, inst.reverse, remap);
        let size = self.doc(ctx.doc).size;
        self.nodes[idx as usize].box_size = Some([
            Length::px(sym.width.map(|w| w as f64).unwrap_or(size[0])),
            Length::px(sym.height.map(|h| h as f64).unwrap_or(size[1])),
        ]);
        let mut stack = ctx.stack.clone();
        stack.push(key);
        let cctx = Ctx { scope: id, stack, ..ctx.clone() };
        let c = self.instantiate(&kids, Some(idx), &cctx);
        self.nodes[idx as usize].children = c;
    }

    fn include(&mut self, idx: u32, inc: &m::Include, ctx: &Ctx) {
        let base = self.doc(ctx.doc).base.clone();
        let path = match sr_model::assets::resolve(&inc.src, &base) {
            sr_model::assets::Resolved::Local(p) => p,
            sr_model::assets::Resolved::Remote(s) => {
                self.diags.push(err(
                    "E12",
                    format!("include {:?}: {s} URIs are not fetched", inc.id),
                    inc.loc,
                    inc.id.as_str(),
                ));
                return;
            }
        };
        let key = format!("file:{}", path.display());
        if ctx.stack.contains(&key) || ctx.stack.len() > 16 {
            self.diags.push(err(
                "E13",
                format!("include {:?} includes {} inside itself", inc.id, path.display()),
                inc.loc,
                inc.id.as_str(),
            ));
            return;
        }
        let doc = match sr_model::load_file(&path, &sr_model::LoadOptions::without_assets()) {
            Ok(d) => d,
            Err(e) => {
                let first = e
                    .report()
                    .and_then(|r| r.diagnostics.iter().find(|d| d.is_error()))
                    .map(|d| format!(": {d}"))
                    .unwrap_or_else(|| format!(": {e}"));
                self.diags.push(err(
                    "E12",
                    format!("include {:?}: {} is not a valid document{first}", inc.id, path.display()),
                    inc.loc,
                    inc.id.as_str(),
                ));
                return;
            }
        };
        let mut sub_d = Vec::new();
        let mut sub_w = Vec::new();
        let t = template(&doc, &EvalOptions::default(), &mut sub_d, &mut sub_w);
        let ns = self.nodes[idx as usize].id.clone();
        let (mut kids, dur, size) = match &inc.symbol {
            Some(s) => match t.scene.symbols.as_ref().and_then(|x| x.symbols.iter().find(|y| &y.id == s)) {
                Some(sym) => (
                    sym.children.clone(),
                    sym.duration.map(|d| d.get()).unwrap_or(t.scene.project.duration.get()),
                    [
                        sym.width.map(|w| w as f64).unwrap_or(t.size[0]),
                        sym.height.map(|h| h as f64).unwrap_or(t.size[1]),
                    ],
                ),
                None => {
                    self.diags.push(err(
                        "E12",
                        format!("include {:?}: {} has no symbol {s:?}", inc.id, path.display()),
                        inc.loc,
                        inc.id.as_str(),
                    ));
                    return;
                }
            },
            None => (t.scene.composition.children.clone(), t.scene.project.duration.get(), t.size),
        };
        let overrides: Vec<m::Override> = inc
            .children
            .iter()
            .filter_map(|c| if let m::IncludeChild::Override(o) = c { Some(o.clone()) } else { None })
            .collect();
        self.apply_scoped_overrides(&mut kids, &overrides, &ns);
        let (markers, grid) = markers_of(&t.scene);
        let doc_ix = self.docs.len() as u16;
        self.docs.push(DocCtx {
            tokens: tokens_of(&t.scene),
            scene: Arc::new(t.scene),
            ns: ns.clone(),
            base: path.parent().map(Path::to_path_buf).unwrap_or_default(),
            markers,
            grid,
            duration: dur,
            size,
        });
        self.symbol_clock(idx, dur, 1.0, 0.0, 0, false, None);
        self.nodes[idx as usize].box_size = Some([Length::px(size[0]), Length::px(size[1])]);
        let mut stack = ctx.stack.clone();
        stack.push(key);
        let cctx = Ctx { scope: ns, doc: doc_ix, stack, repeat: ctx.repeat.clone() };
        let c = self.instantiate(&kids, Some(idx), &cctx);
        self.nodes[idx as usize].children = c;
    }

    fn intrinsic(&self, n: u32) -> Option<f64> {
        let node = &self.nodes[n as usize];
        node.end.map(|e| e - node.start)
    }

    fn place_sequence(&mut self, idx: u32, s: &m::Sequence) {
        let kids = self.nodes[idx as usize].children.clone();
        let mut cursor = self.nodes[idx as usize].start;
        let mut placed: Vec<u32> = Vec::new();
        for &k in &kids {
            if self.nodes[k as usize].name == "transition" {
                continue;
            }
            let own = self.nodes[k as usize].start;
            let d = self.intrinsic(k);
            let has_marker = attr_str(&*self.nodes[k as usize].elem, "startMarker").is_some();
            if !has_marker {
                let new_start = cursor + own;
                let delta = new_start - own;
                self.shift(k, delta);
            }
            let node = &self.nodes[k as usize];
            cursor = match d {
                Some(d) => node.start + d + s.time_gap,
                None => f64::INFINITY,
            };
            placed.push(k);
        }
        if let Some(kind) = &s.transition {
            // auto-transitions at junctions without an explicit transition
            let explicit: HashSet<(Option<String>, Option<String>)> = kids
                .iter()
                .filter(|k| self.nodes[**k as usize].name == "transition")
                .map(|k| {
                    let e = &*self.nodes[*k as usize].elem;
                    (attr_str(e, "from"), attr_str(e, "to"))
                })
                .collect();
            for w in placed.windows(2) {
                let (a, b) = (w[0], w[1]);
                let ida = self.nodes[a as usize].elem.id().map(str::to_string);
                let idb = self.nodes[b as usize].elem.id().map(str::to_string);
                if explicit.contains(&(ida, idb)) {
                    continue;
                }
                // the cut is from's end
                let cut = self.nodes[a as usize].end.unwrap_or(self.nodes[b as usize].start);
                let d = s.transition_duration.get();
                self.transitions.push(TransitionInst {
                    node: None,
                    kind: kind.as_str().into(),
                    from: Some(a),
                    to: Some(b),
                    window: (cut - d / 2.0, cut + d / 2.0),
                    ease: curve::EASE_IN_OUT,
                    container: Some(idx),
                    matte: None,
                });
            }
        }
    }

    /// Moves a node in time, keeping its media clock aligned, and with it what shares its timeline:
    /// the contents of a group move with it, those of an instance run on its own clock.
    fn shift(&mut self, k: u32, delta: f64) {
        if delta == 0.0 {
            return;
        }
        let mut todo = vec![k];
        while let Some(k) = todo.pop() {
            let n = &mut self.nodes[k as usize];
            n.start += delta;
            if let Some(e) = &mut n.end {
                *e += delta;
            }
            if let Some(mc) = &mut n.media {
                mc.start += delta;
            }
            match &mut n.clock {
                Clock::Media(mc) => {
                    mc.start += delta;
                    continue;
                }
                Clock::Affine { origin, .. } => *origin += delta,
                Clock::Same => {}
            }
            // contents placed by a marker stay on it, as a sequence's own children do
            let nodes = &self.nodes;
            let moving = nodes[k as usize].children.iter().copied();
            todo.extend(moving.filter(|c| attr_str(&*nodes[*c as usize].elem, "startMarker").is_none()));
            for t in self.transitions.iter_mut().filter(|t| t.container == Some(k)) {
                t.window = (t.window.0 + delta, t.window.1 + delta);
            }
        }
    }

    // ---------------------------------------------------------- slots

    fn slot(&mut self, owner: Owner, prop: &str) -> Result<u32, String> {
        let key = (owner, Arc::<str>::from(prop));
        if let Some(&s) = self.slot_ix.get(&key) {
            return Ok(s);
        }
        let (kind, base, time_node) = match owner {
            Owner::Node(n) => {
                let node = &self.nodes[n as usize];
                let e: &dyn Element = &*node.elem;
                let tokens = &self.doc(node.doc).tokens;
                let tok = |t: &str| tokens.get(t).copied();
                if let Some((_, [a, b])) = ALIASES.iter().find(|(al, _)| *al == prop) {
                    if !e.declares(a) {
                        return Err(format!("<{}> has no property '{prop}'", node.name));
                    }
                    let va = e.get_attr(a).map(|x| PropKind::Length { positive: false }.from_attr(&x, &tok));
                    let vb = e.get_attr(b).map(|x| PropKind::Length { positive: false }.from_attr(&x, &tok));
                    let l = |v: Option<Value>, d: f64| match v {
                        Some(Value::Len(l)) => l,
                        Some(Value::Num(x)) => Length::px(x),
                        _ => Length::px(d),
                    };
                    let d = if prop == "scale" { 1.0 } else { 0.0 };
                    (PropKind::Pair, Value::Pair([l(va, d), l(vb, d)]), node.parent)
                } else {
                    let decl = COMPLEX_TYPES[e.xsd_type()].attr(prop).ok_or_else(|| {
                        let hint = crate::suggest(
                            prop,
                            COMPLEX_TYPES[e.xsd_type()]
                                .attrs
                                .iter()
                                .map(|a| a.name)
                                .chain(["position", "scale", "anchor", "skew"]),
                        )
                        .map(|s| format!("; did you mean '{s}'?"))
                        .unwrap_or_default();
                        format!("<{}> has no property '{prop}'{hint}", node.name)
                    })?;
                    let kind = PropKind::of_simple_type(decl.ty);
                    let base = e.get_attr(prop).map(|a| kind.from_attr(&a, &tok)).unwrap_or(kind.neutral());
                    (kind, base, node.parent)
                }
            }
            Owner::Element(t) => {
                let el = &self.elements[t as usize];
                let (kind, base) =
                    el.attrs.iter().find(|(n, _, _)| n.as_ref() == prop).map(|(_, k, v)| (*k, v.clone())).ok_or_else(
                        || {
                            let hint = crate::suggest(prop, el.attrs.iter().map(|a| a.0.as_ref()))
                                .map(|s| format!("; did you mean '{s}'?"))
                                .unwrap_or_default();
                            format!("<{}> has no property '{prop}'{hint}", el.name)
                        },
                    )?;
                (kind, base, el.node)
            }
        };
        // node slots live on the node's own timeline, which is its container's child timeline
        let time_node = match owner {
            Owner::Node(n) => self.nodes[n as usize].parent.map(|_| n).or(Some(n)),
            Owner::Element(_) => time_node,
        };
        let i = self.slots.len() as u32;
        self.slots.push(Slot {
            owner,
            prop: key.1.clone(),
            kind,
            base,
            channels: Vec::new(),
            expr: None,
            link: None,
            time_node,
        });
        self.slot_ix.insert(key, i);
        match owner {
            Owner::Node(n) => self.nodes[n as usize].slots.push(i),
            Owner::Element(t) => self.elements[t as usize].slots.push(i),
        }
        Ok(i)
    }
}

/// Element-target attribute snapshot: (name, (kind, value)).
fn snapshot(e: &dyn Element, tokens: &HashMap<String, [f64; 4]>) -> Vec<(Arc<str>, PropKind, Value)> {
    let tok = |t: &str| tokens.get(t).copied();
    COMPLEX_TYPES[e.xsd_type()]
        .attrs
        .iter()
        .map(|a| {
            let kind = PropKind::of_simple_type(a.ty);
            let v = e.get_attr(a.name).map(|x| kind.from_attr(&x, &tok)).unwrap_or(kind.neutral());
            (Arc::from(a.name), kind, v)
        })
        .collect()
}

/// Numeric `<param name value>` children of shader effects and transitions, as animatable properties:
/// `<animate property="NAME">` then drives the uniform of the same name.
fn shader_params(e: &dyn Element, known: &[(Arc<str>, PropKind, Value)]) -> Vec<(Arc<str>, PropKind, Value)> {
    const NUM: PropKind = PropKind::Number(crate::value::Range { lo: None, hi: None, integer: false });
    sr_model::element::children(e)
        .into_iter()
        .filter(|c| c.element_name() == "param")
        .filter_map(|c| {
            let name = c.get_attr("name")?.to_string();
            if known.iter().any(|(n, _, _)| n.as_ref() == name) {
                return None;
            }
            let v: f64 = c.get_attr("value")?.to_string().trim().parse().ok()?;
            Some((Arc::from(name), NUM, Value::Num(v)))
        })
        .collect()
}

struct DocLookup<'b> {
    b: &'b Builder,
    doc: u16,
}

impl Lookup for DocLookup<'_> {
    fn token(&self, name: &str) -> Option<[f64; 4]> {
        self.b.doc(self.doc).tokens.get(name).copied()
    }
    fn marker(&self, id: &str) -> Option<f64> {
        self.b.marker(self.doc, id)
    }
}

impl Builder {
    fn add_element(&mut self, key: Arc<str>, e: &dyn Element, node: Option<u32>, doc: u16) -> u32 {
        self.add_element_with_id(key, e, node, doc, None)
    }

    fn add_element_with_id(
        &mut self,
        key: Arc<str>,
        e: &dyn Element,
        node: Option<u32>,
        doc: u16,
        id: Option<Arc<str>>,
    ) -> u32 {
        let i = self.elements.len() as u32;
        let mut attrs = snapshot(e, &self.doc(doc).tokens);
        // effects and transitions report their schema type name
        if matches!(e.element_name(), "effect" | "effectType" | "transition" | "transitionType") {
            let params = shader_params(e, &attrs);
            attrs.extend(params);
        }
        self.elements.push(ElemTarget { key, name: e.element_name(), node, id, attrs, slots: Vec::new() });
        i
    }

    /// Compiles the animation children of `e` onto `owner`.
    fn animate(&mut self, owner: Owner, e: &dyn Element, node: Option<u32>, doc: u16, scope: &Arc<str>, who: &str) {
        for c in children(e) {
            match c.element_name() {
                "animate" => {
                    let a = c.as_any().downcast_ref::<m::Animate>().expect("animate");
                    let slot = match self.slot(owner, &a.property) {
                        Ok(s) => s,
                        Err(msg) => {
                            self.diags.push(err("E02", format!("animate on {who:?}: {msg}"), a.loc, who));
                            continue;
                        }
                    };
                    ignored_key_parameters(&a.keys, a.default_interpolation, who, &mut self.warnings);
                    let spec = ChannelSpec {
                        keys: &a.keys,
                        default: a.default_interpolation,
                        before: a.extrapolate_before,
                        after: a.extrapolate_after,
                        additive: a.additive,
                        time_base: a.time_base,
                        kind: self.slots[slot as usize].kind,
                    };
                    match Channel::compile(&spec, &DocLookup { b: self, doc }) {
                        Ok(ch) => {
                            self.channels.push(ch);
                            let ci = (self.channels.len() - 1) as u32;
                            self.slots[slot as usize].channels.push(ci);
                        }
                        Err(msg) => self.diags.push(err(
                            "E04",
                            format!("animate {:?} on {who:?}: {msg}", a.property),
                            a.loc,
                            who,
                        )),
                    }
                }
                "expression" => {
                    let x = c.as_any().downcast_ref::<m::Expression>().expect("expression");
                    if !x.enabled {
                        continue;
                    }
                    match self.slot(owner, &x.property) {
                        Ok(slot) => {
                            // the expression's @seed, else the project's
                            let seed = x.seed.unwrap_or(self.project_seed);
                            self.pending_expr.push((
                                Some(slot),
                                node,
                                x.value.clone(),
                                seed,
                                x.loc,
                                scope.clone(),
                                doc,
                            ));
                        }
                        Err(msg) => self.diags.push(err("E02", format!("expression on {who:?}: {msg}"), x.loc, who)),
                    }
                }
                "link" => {
                    let l = c.as_any().downcast_ref::<m::Link>().expect("link");
                    match self.slot(owner, &l.property) {
                        Ok(slot) => self.pending_link.push((slot, l.clone(), node, scope.clone(), doc)),
                        Err(msg) => self.diags.push(err("E02", format!("link on {who:?}: {msg}"), l.loc, who)),
                    }
                }
                "motionPath" => {
                    let mp = c.as_any().downcast_ref::<m::MotionPath>().expect("motionPath");
                    let Owner::Node(n) = owner else {
                        self.diags.push(err(
                            "E02",
                            format!("motionPath on {who:?}: only nodes follow motion paths"),
                            mp.loc,
                            who,
                        ));
                        continue;
                    };
                    let path = match MotionPath::parse(&mp.path) {
                        Ok(p) => p,
                        Err(e) => {
                            self.diags.push(err(
                                "E15",
                                format!("motionPath on {who:?}: {} at offset {}", e.message, e.offset),
                                mp.loc,
                                who,
                            ));
                            continue;
                        }
                    };
                    let mut progress = None;
                    for a in &mp.animates {
                        if a.property != "progress" {
                            self.diags.push(err(
                                "E02",
                                format!("motionPath on {who:?}: only 'progress' can be animated, not {:?}", a.property),
                                a.loc,
                                who,
                            ));
                            continue;
                        }
                        let spec = ChannelSpec {
                            keys: &a.keys,
                            default: a.default_interpolation,
                            before: a.extrapolate_before,
                            after: a.extrapolate_after,
                            additive: a.additive,
                            time_base: a.time_base,
                            kind: PropKind::Number(crate::value::Range {
                                lo: Some((0.0, false)),
                                hi: Some((1.0, false)),
                                integer: false,
                            }),
                        };
                        match Channel::compile(&spec, &DocLookup { b: self, doc }) {
                            Ok(ch) => {
                                let key: Arc<str> = format!("{who}/motionPath").into();
                                let el = self.elements.len() as u32;
                                self.elements.push(ElemTarget {
                                    key,
                                    name: "motionPath",
                                    node: Some(n),
                                    id: None,
                                    attrs: vec![(Arc::from("progress"), spec.kind, Value::Num(0.0))],
                                    slots: Vec::new(),
                                });
                                let s = self.slot(Owner::Element(el), "progress").expect("declared");
                                self.channels.push(ch);
                                self.slots[s as usize].channels.push((self.channels.len() - 1) as u32);
                                progress = Some(s);
                            }
                            Err(msg) => self.diags.push(err(
                                "E04",
                                format!("motionPath progress on {who:?}: {msg}"),
                                a.loc,
                                who,
                            )),
                        }
                    }
                    self.nodes[n as usize].motion = Some(Motion {
                        path,
                        start: mp.start,
                        end: mp.end,
                        ease: curve::resolve(mp.interpolation, &KeyParams::default()),
                        auto_orient: mp.auto_orient,
                        orient_offset: mp.orient_offset,
                        constant_speed: mp.constant_speed,
                        progress,
                    });
                }
                _ => {}
            }
        }
    }

    /// Element targets for animated non-node descendants of a node.
    fn parts(&mut self, n: u32) {
        let elem = self.nodes[n as usize].elem.clone();
        let (doc, scope, id) =
            (self.nodes[n as usize].doc, self.nodes[n as usize].scope.clone(), self.nodes[n as usize].id.clone());
        let mut stack: Vec<(&dyn Element, String)> = vec![(&*elem as &dyn Element, id.to_string())];
        while let Some((e, path)) = stack.pop() {
            let mut counts: HashMap<&str, usize> = HashMap::new();
            for c in children(e) {
                let name = c.element_name();
                if ANIM.contains(&name) || matches!(name, "timeRemap" | "override") || Node::ELEMENTS.contains(&name) {
                    continue;
                }
                let k = counts.entry(name).or_default();
                let key = format!("{path}/{name}[{k}]");
                *k += 1;
                if children(c).iter().any(|g| ANIM.contains(&g.element_name())) {
                    let id = c.element_id().map(|i| -> Arc<str> {
                        if scope.is_empty() {
                            i.into()
                        } else {
                            format!("{scope}/{i}").into()
                        }
                    });
                    let t = self.add_element_with_id(key.as_str().into(), c, Some(n), doc, id);
                    self.nodes[n as usize].parts.push(t);
                    self.animate(Owner::Element(t), c, Some(n), doc, &scope, &key);
                }
                stack.push((c, key));
            }
        }
    }

    /// Element targets outside the composition (paints, effects, lights, …).
    fn globals(&mut self) {
        let scene = self.docs[0].scene.clone();
        let mut stack: Vec<(&dyn Element, String)> = Vec::new();
        for c in children(&*scene) {
            if !matches!(c.element_name(), "compositionType" | "symbolsType" | "parametersType") {
                stack.push((c, c.element_name().to_string()));
            }
        }
        let root: Arc<str> = "".into();
        while let Some((e, path)) = stack.pop() {
            let key = e.element_id().map(str::to_string).unwrap_or(path);
            if children(e).iter().any(|g| ANIM.contains(&g.element_name())) {
                let t = self.add_element(key.as_str().into(), e, None, 0);
                self.animate(Owner::Element(t), e, None, 0, &root, &key);
            }
            let mut counts: HashMap<&str, usize> = HashMap::new();
            for c in children(e) {
                let name = c.element_name();
                if ANIM.contains(&name) {
                    continue;
                }
                let k = counts.entry(name).or_default();
                stack.push((c, format!("{key}/{name}[{k}]")));
                *k += 1;
            }
        }
    }

    fn resolve_links(&mut self) {
        for n in 0..self.nodes.len() {
            let node = &self.nodes[n];
            let e: &dyn Element = &*node.elem;
            let scope = node.scope.clone();
            let (parent_attr, matte_attr, id) =
                (attr_str(e, "parent"), attr_str(e, "matte").filter(|_| node.name != "transition"), node.id.clone());
            if let Some(p) = parent_attr {
                match self.resolve(&scope, &p) {
                    Some(t) => self.nodes[n].parent_link = Some(t),
                    None => self.diags.push(err(
                        "E11",
                        format!("{id:?}: @parent {p:?} is not a node of the composition"),
                        e.loc(),
                        &*id,
                    )),
                }
            }
            if let Some(mt) = matte_attr {
                self.nodes[n].matte = self.resolve(&scope, &mt);
            }
            // The 3D renderer and simulation read these links from the element
            // snapshot, rather than the 2D parent_link. Resolve them once here
            // using the compiler's lexical scope, including targets omitted by
            // a frame condition; a hidden local target must not bind globally.
            let parent = self.nodes[n].parent_link.map(|i| self.nodes[i as usize].id.clone());
            let constraints: HashMap<String, Arc<str>> = sr_model::element::children(&*self.nodes[n].elem)
                .into_iter()
                .filter(|e| e.element_name() == "transformConstraint")
                .filter_map(|e| attr_str(e, "target"))
                .filter_map(|id| self.resolve(&scope, &id).map(|i| (id, self.nodes[i as usize].id.clone())))
                .collect();
            let mut lists: Vec<_> = sr_model::element::children(&*self.nodes[n].elem)
                .into_iter()
                .filter(|e| e.element_name() == "pyro")
                .filter_map(|e| attr_str(e, "colliders").map(|s| (s, e.loc())))
                .collect();
            if matches!(self.nodes[n].name, "particles3D" | "ocean") {
                if let Some(list) = attr_str(&*self.nodes[n].elem, "colliders") {
                    lists.push((list, self.nodes[n].elem.loc()));
                }
            }
            let mut colliders = HashMap::new();
            for (list, loc) in lists {
                let mut resolved = Vec::new();
                for id in list.split_whitespace() {
                    match self.resolve(&scope, id) {
                        Some(i) => resolved.push(self.nodes[i as usize].id.to_string()),
                        None => self.diags.push(err(
                            "E11",
                            format!("collider {id:?} is not instantiated in this scope"),
                            loc,
                            &*self.nodes[n].id,
                        )),
                    }
                }
                colliders.insert(list, resolved.join(" "));
            }
            // The emitters whose particles fall into an ocean are named in the same lexical scope.
            let splash: Option<(String, String)> = (self.nodes[n].name == "ocean")
                .then(|| attr_str(&*self.nodes[n].elem, "splash"))
                .flatten()
                .map(|list| {
                    let mut resolved = Vec::new();
                    for id in list.split_whitespace() {
                        match self.resolve(&scope, id) {
                            Some(i) => resolved.push(self.nodes[i as usize].id.to_string()),
                            None => self.diags.push(err(
                                "E11",
                                format!("splash emitter {id:?} is not instantiated in this scope"),
                                self.nodes[n].elem.loc(),
                                &*self.nodes[n].id,
                            )),
                        }
                    }
                    (list, resolved.join(" "))
                });
            // The smoke that drags the particles of an emitter is named in the same lexical scope.
            let gas: Option<(String, String)> = (self.nodes[n].name == "particles3D")
                .then(|| attr_str(&*self.nodes[n].elem, "gas"))
                .flatten()
                .and_then(|id| match self.resolve(&scope, &id) {
                    Some(i) => Some((id, self.nodes[i as usize].id.to_string())),
                    None => {
                        self.diags.push(err(
                            "E11",
                            format!("gas {id:?} is not instantiated in this scope"),
                            self.nodes[n].elem.loc(),
                            &*self.nodes[n].id,
                        ));
                        None
                    }
                });
            // What a crater causes (smoke, ejecta) names the crater by its id, in the same lexical
            // scope; the reference becomes the effective id of the object that owns the crater.
            let mut cause_owners: HashMap<String, Arc<str>> = HashMap::new();
            let kids = sr_model::element::children(&*self.nodes[n].elem);
            for id in kids
                .iter()
                .copied()
                .filter(|e| e.element_name() == "pyro")
                .flat_map(|pyro| sr_model::element::children(pyro))
                .filter(|e| matches!(e.element_name(), "pyroSource" | "pyroImpulse"))
                .chain(kids.iter().copied().filter(|e| e.element_name() == "burst"))
                .filter_map(|e| attr_str(e, "crater"))
            {
                match self.resolve_crater(&scope, &id) {
                    Some(owner) => {
                        cause_owners.insert(id, self.nodes[owner as usize].id.clone());
                    }
                    None => self.diags.push(err(
                        "E11",
                        format!("crater {id:?} is not instantiated in this scope"),
                        self.nodes[n].elem.loc(),
                        &*self.nodes[n].id,
                    )),
                }
            }
            // A crater that grows from an impact names its source body, in the same lexical
            // scope: inside a symbol instance it is that instance's body.
            let mut sources: HashMap<String, Arc<str>> = HashMap::new();
            for id in sr_model::element::children(&*self.nodes[n].elem)
                .into_iter()
                .filter(|e| e.element_name() == "crater")
                .filter_map(|e| attr_str(e, "source"))
            {
                match self.resolve(&scope, &id) {
                    Some(i) => {
                        sources.insert(id, self.nodes[i as usize].id.clone());
                    }
                    None => self.diags.push(err(
                        "E11",
                        format!("crater source {id:?} is not instantiated in this scope"),
                        self.nodes[n].elem.loc(),
                        &*self.nodes[n].id,
                    )),
                }
            }
            // A water impulse that comes from a body names it in the same lexical scope.
            let mut entries: HashMap<String, Arc<str>> = HashMap::new();
            for id in sr_model::element::children(&*self.nodes[n].elem)
                .into_iter()
                .filter(|e| e.element_name() == "waterImpulse")
                .filter_map(|e| attr_str(e, "source"))
            {
                match self.resolve(&scope, &id) {
                    Some(i) => {
                        entries.insert(id, self.nodes[i as usize].id.clone());
                    }
                    None => self.diags.push(err(
                        "E11",
                        format!("water impulse source {id:?} is not instantiated in this scope"),
                        self.nodes[n].elem.loc(),
                        &*self.nodes[n].id,
                    )),
                }
            }
            if parent.is_some()
                || !entries.is_empty()
                || !constraints.is_empty()
                || !colliders.is_empty()
                || !sources.is_empty()
                || !cause_owners.is_empty()
                || gas.is_some()
                || splash.is_some()
            {
                let elem = Arc::make_mut(&mut self.nodes[n].elem);
                if let Some((_, effective)) = &splash {
                    elem.set_attr("splash", effective).expect("resolved ocean splash emitters");
                }
                if let Some((_, effective)) = &gas {
                    elem.set_attr("gas", effective).expect("resolved particle gas");
                }
                if let Some(list) = attr_str(elem, "colliders").and_then(|list| colliders.get(&list)) {
                    elem.set_attr("colliders", list).expect("resolved particle colliders");
                }
                if let Some(parent) = parent {
                    elem.set_attr("parent", &parent).expect("resolved string parent attribute");
                }
                elem.visit_mut(&mut |child| {
                    if child.element_name() == "transformConstraint" {
                        if let Some(target) = attr_str(child, "target").and_then(|id| constraints.get(&id)) {
                            child.set_attr("target", target).expect("resolved string constraint target");
                        }
                    }
                    if child.element_name() == "pyro" {
                        if let Some(list) = attr_str(child, "colliders").and_then(|list| colliders.get(&list)) {
                            child.set_attr("colliders", list).expect("resolved collider token list");
                        }
                        child.visit_mut(&mut |input| {
                            if let Some(owner) = attr_str(input, "crater").and_then(|id| cause_owners.get(&id)) {
                                input.set_attr("crater", owner).expect("resolved crater owner");
                            }
                        });
                    }
                    if child.element_name() == "waterImpulse" {
                        if let Some(body) = attr_str(child, "source").and_then(|id| entries.get(&id)) {
                            child.set_attr("source", body).expect("resolved water impulse source");
                        }
                    }
                    if child.element_name() == "burst" {
                        if let Some(owner) = attr_str(child, "crater").and_then(|id| cause_owners.get(&id)) {
                            child.set_attr("crater", owner).expect("resolved crater owner");
                        }
                    }
                    if child.element_name() == "crater" {
                        if let Some(source) = attr_str(child, "source").and_then(|id| sources.get(&id)) {
                            child.set_attr("source", source).expect("resolved crater source");
                        }
                    }
                });
            }
        }
        // Overrides and symbol expansion can introduce matte cycles absent from the source XML.
        let children: Vec<Vec<usize>> =
            self.nodes.iter().map(|n| n.children.iter().map(|&c| c as usize).collect()).collect();
        let mattes: Vec<_> = self.nodes.iter().map(|n| n.matte.map(|m| m as usize)).collect();
        for i in sr_model::rules::cyclic_mattes(&children, &mattes) {
            let n = &self.nodes[i];
            self.diags.push(err(
                "P05",
                format!("{:?}: matte dependencies form a cycle through mattes or contained children", n.id),
                n.elem.loc(),
                &*n.id,
            ));
        }
        // Transform dependencies include the container when @parent is absent.
        for start in 0..self.nodes.len() {
            let mut seen = HashSet::new();
            let mut cur = start as u32;
            while let Some(p) = self.nodes[cur as usize].parent_link.or(self.nodes[cur as usize].parent) {
                if !seen.insert(cur) || p as usize == start {
                    let id = self.nodes[start].id.clone();
                    let loc = self.nodes[start].elem.loc();
                    if !self.diags.iter().any(|d| d.code == "E11" && d.path == *id) {
                        self.diags.push(err("E11", format!("{id:?}: @parent forms a cycle"), loc, &*id));
                    }
                    self.nodes[start].parent_link = None;
                    break;
                }
                cur = p;
            }
        }
    }

    fn transitions(&mut self) {
        for n in 0..self.nodes.len() {
            if self.nodes[n].name != "transition" {
                continue;
            }
            let Node::Transition(t) = &*self.nodes[n].elem.clone() else { continue };
            let scope = self.nodes[n].scope.clone();
            let from = t.from.as_ref().and_then(|f| self.resolve(&scope, f));
            let to = t.to.as_ref().and_then(|x| self.resolve(&scope, x));
            let matte = t.matte.as_ref().and_then(|x| self.resolve(&scope, x));
            // the cut is from's end, or to's start when there is no from (or from never ends)
            let cut = match (from.and_then(|a| self.nodes[a as usize].end), from, to) {
                (Some(end), _, _) => end,
                (None, _, Some(b)) => self.nodes[b as usize].start,
                (None, Some(_), None) => f64::INFINITY,
                (None, None, None) => continue,
            };
            let d = t.duration.get();
            let window = match t.alignment {
                m::TransitionAlignment::Center => (cut - d / 2.0, cut + d / 2.0),
                m::TransitionAlignment::End => (cut - d, cut),
                m::TransitionAlignment::Start => (cut, cut + d),
            };
            let ti = self.transitions.len() as u32;
            self.transitions.push(TransitionInst {
                node: Some(n as u32),
                kind: t.r#type.as_str().into(),
                from,
                to,
                window,
                ease: curve::resolve(t.curve, &KeyParams::default()),
                container: self.nodes[n].parent,
                matte,
            });
            self.nodes[n].kind = Kind::Transition(ti);
            self.nodes[n].start = window.0;
            self.nodes[n].end = Some(window.1);
        }
    }

    fn compile_pending(&mut self) {
        for (slot, node, src, seed, loc, scope, doc) in std::mem::take(&mut self.pending_expr) {
            let who = match (slot, node) {
                (Some(s), _) => self.slot_name(s),
                (None, Some(n)) => format!("{}@condition", self.nodes[n as usize].id),
                _ => "expression".into(),
            };
            let mut res = ExprResolver { b: self, scope, doc };
            match vm::compile(&src, &mut res) {
                Ok(code) => {
                    self.exprs.push(ExprInst { code, slot, node, seed });
                    let i = (self.exprs.len() - 1) as u32;
                    match slot {
                        Some(s) => self.slots[s as usize].expr = Some(i),
                        None => {
                            if let Some(n) = node {
                                self.nodes[n as usize].cond = Some(i);
                            }
                        }
                    }
                }
                Err(e) => {
                    let excerpt: String = src.chars().skip(e.offset.saturating_sub(10)).take(30).collect();
                    self.diags.push(err(
                        "E01",
                        format!("{who}: {} (near {:?})", e.message, excerpt.trim()),
                        loc,
                        who.as_str(),
                    ));
                }
            }
        }
        for (slot, l, _node, scope, doc) in std::mem::take(&mut self.pending_link) {
            let who = self.slot_name(slot);
            let source = if let Some(p) = l.source.strip_prefix("param:") {
                match self.params.get(p) {
                    Some(v) => Some(LinkSource::Param(v.clone())),
                    None => {
                        self.diags.push(err("E14", format!("link on {who}: no parameter {p:?}"), l.loc, who.as_str()));
                        None
                    }
                }
            } else if let Some(a) = l.source.strip_prefix("audio:") {
                let (track, band) = a.split_once(':').unwrap_or((a, "full"));
                match Band::parse(band) {
                    Some(b) => {
                        self.analysis_tracks.insert(track.to_string());
                        Some(LinkSource::Audio(track.to_string(), b))
                    }
                    None => {
                        self.diags.push(err(
                            "E14",
                            format!("link on {who}: unknown band {band:?}; use low, mid, high or full"),
                            l.loc,
                            who.as_str(),
                        ));
                        None
                    }
                }
            } else if let Some(mk) = l.source.strip_prefix("marker:") {
                let dur = self.doc(doc).scene.markers.iter().flat_map(|m| m.children.iter()).find_map(|c| match c {
                    m::MarkersChild::Marker(x) if x.id.as_deref() == Some(mk) => Some(x.duration.get()),
                    _ => None,
                });
                match self.marker(doc, mk) {
                    Some(t) => Some(LinkSource::Marker(t, dur.unwrap_or(0.0))),
                    None => {
                        self.diags.push(err("E14", format!("link on {who}: no marker {mk:?}"), l.loc, who.as_str()));
                        None
                    }
                }
            } else {
                let mut res = ExprResolver { b: self, scope, doc };
                match res.prop(&l.source) {
                    Ok(s) => Some(LinkSource::Prop(s)),
                    Err(e) => {
                        self.diags.push(err(
                            "E14",
                            format!("link on {who}: {e}; a source is nodeId.property, param:name, audio:track[:band] or marker:id"),
                            l.loc,
                            who.as_str(),
                        ));
                        None
                    }
                }
            };
            if let Some(source) = source {
                self.links.push(LinkInst {
                    source,
                    scale: l.scale,
                    offset: l.offset,
                    min: l.min,
                    max: l.max,
                    delay: l.delay,
                    smoothing: l.smoothing.get(),
                });
                self.slots[slot as usize].link = Some((self.links.len() - 1) as u32);
            }
        }
    }

    fn slot_name(&self, s: u32) -> String {
        let slot = &self.slots[s as usize];
        let owner = match slot.owner {
            Owner::Node(n) => self.nodes[n as usize].id.to_string(),
            Owner::Element(e) => self.elements[e as usize].key.to_string(),
        };
        format!("{owner}.{}", slot.prop)
    }

    /// For each slot, whether another slot's expression or link reads it.
    fn slot_read(&self) -> Vec<bool> {
        let mut read = vec![false; self.slots.len()];
        for (i, s) in self.slots.iter().enumerate() {
            if let Some(x) = s.expr {
                for &d in &self.exprs[x as usize].code.deps {
                    if d as usize != i {
                        read[d as usize] = true;
                    }
                }
            }
            if let Some(l) = s.link {
                if let LinkSource::Prop(p) = self.links[l as usize].source {
                    read[p as usize] = true;
                }
            }
        }
        read
    }

    /// Topological order of slots; reports cycles.
    fn order(&mut self) -> Vec<u32> {
        let n = self.slots.len();
        let mut deps: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (i, s) in self.slots.iter().enumerate() {
            if let Some(x) = s.expr {
                deps[i].extend(self.exprs[x as usize].code.deps.iter().copied());
            }
            if let Some(l) = s.link {
                if let LinkSource::Prop(p) = self.links[l as usize].source {
                    deps[i].push(p);
                }
            }
            if let Owner::Node(nd) = s.owner {
                // a motion progress slot feeds the node transform, not other slots
                let _ = nd;
            }
        }
        let mut indeg = vec![0usize; n];
        let mut users: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (i, d) in deps.iter().enumerate() {
            for &p in d {
                if p as usize != i {
                    indeg[i] += 1;
                    users[p as usize].push(i as u32);
                } else {
                    indeg[i] += 1;
                }
            }
        }
        let mut ready: Vec<u32> = (0..n as u32).filter(|&i| indeg[i as usize] == 0).collect();
        ready.reverse();
        let mut out = Vec::with_capacity(n);
        while let Some(i) = ready.pop() {
            out.push(i);
            for &u in &users[i as usize] {
                indeg[u as usize] -= 1;
                if indeg[u as usize] == 0 {
                    ready.push(u);
                }
            }
        }
        if out.len() < n {
            let stuck: Vec<u32> = (0..n as u32).filter(|&i| indeg[i as usize] > 0).collect();
            // walk one cycle for the message
            let mut cycle = vec![stuck[0]];
            let mut cur = stuck[0];
            for _ in 0..n {
                let Some(&next) = deps[cur as usize].iter().find(|d| indeg[**d as usize] > 0) else { break };
                if let Some(pos) = cycle.iter().position(|c| *c == next) {
                    cycle.drain(..pos);
                    cycle.push(next);
                    break;
                }
                cycle.push(next);
                cur = next;
            }
            let names: Vec<String> = cycle.iter().map(|s| self.slot_name(*s)).collect();
            let s0 = stuck[0];
            let loc = match self.slots[s0 as usize].owner {
                Owner::Node(nd) => self.nodes[nd as usize].elem.loc(),
                Owner::Element(_) => Loc::default(),
            };
            self.diags.push(err(
                "E03",
                format!("properties depend on each other in a cycle: {}", names.join(" → ")),
                loc,
                names[0].as_str(),
            ));
            out.extend(stuck);
        }
        out
    }

    fn fill_tf_slots(&mut self) {
        for i in 0..self.nodes.len() {
            let mut t = TfSlots::default();
            for &s in &self.nodes[i].slots {
                let p = &*self.slots[s as usize].prop;
                if let Some(k) = TF_POS.iter().position(|x| *x == p) {
                    t.pos[k] = Some((s, None));
                }
                if let Some(k) = TF_NUM.iter().position(|x| *x == p) {
                    t.num[k] = Some((s, None));
                }
            }
            for &s in &self.nodes[i].slots {
                match &*self.slots[s as usize].prop {
                    "position" => {
                        t.pos[tfi::X] = Some((s, Some(0)));
                        t.pos[tfi::Y] = Some((s, Some(1)));
                    }
                    "anchor" => {
                        t.pos[tfi::AX] = Some((s, Some(0)));
                        t.pos[tfi::AY] = Some((s, Some(1)));
                    }
                    "scale" => {
                        t.num[tfi::SX] = Some((s, Some(0)));
                        t.num[tfi::SY] = Some((s, Some(1)));
                    }
                    "skew" => {
                        t.num[tfi::KX] = Some((s, Some(0)));
                        t.num[tfi::KY] = Some((s, Some(1)));
                    }
                    _ => {}
                }
            }
            self.nodes[i].tf_slots = t;
        }
    }
}

struct ExprResolver<'b> {
    b: &'b mut Builder,
    scope: Arc<str>,
    doc: u16,
}

impl Resolver for ExprResolver<'_> {
    fn prop(&mut self, path: &str) -> Result<u32, String> {
        let (id, prop) = path.rsplit_once('.').ok_or_else(|| format!("prop({path:?}) needs \"id.property\""))?;
        if let Some(n) = self.b.resolve(&self.scope, id) {
            return self.b.slot(Owner::Node(n), prop);
        }
        if let Some(t) = self.b.elements.iter().position(|e| &*e.key == id) {
            return self.b.slot(Owner::Element(t as u32), prop);
        }
        // an animated part of a node that declares this id (a bone, a mask), from the scope outward
        let mut scope = self.scope.to_string();
        loop {
            let want = if scope.is_empty() { id.to_string() } else { format!("{scope}/{id}") };
            if let Some(t) = self.b.elements.iter().position(|e| e.id.as_deref() == Some(want.as_str())) {
                return self.b.slot(Owner::Element(t as u32), prop);
            }
            match scope.rfind('/') {
                Some(i) => scope.truncate(i),
                None if scope.is_empty() => break,
                None => scope.clear(),
            }
        }
        // a static element outside the composition
        let scene = self.b.docs[self.doc as usize].scene.clone();
        let mut found = None;
        walk(&*scene, &mut |e| {
            if found.is_none() && e.element_id() == Some(id) {
                found = Some(e);
            }
        });
        match found {
            Some(e) => {
                let t = self.b.add_element(id.into(), e, None, self.doc);
                self.b.slot(Owner::Element(t), prop)
            }
            None => {
                let hint = crate::suggest(id, self.b.ids.keys().map(|k| &**k))
                    .map(|s| format!("; did you mean {s:?}?"))
                    .unwrap_or_default();
                Err(format!("no element has id {id:?}{hint}"))
            }
        }
    }

    fn marker(&mut self, id: &str) -> Option<f64> {
        self.b.marker(self.doc, id)
    }
}

/// Builds the evaluation program for a validated document.
pub fn build(doc: &Document, opts: &EvalOptions) -> Result<Program, sr_model::Report> {
    let mut diags = Vec::new();
    let mut warnings = Vec::new();
    let t = template(doc, opts, &mut diags, &mut warnings);
    let duration = t.scene.project.duration.get();
    let (markers, grid) = markers_of(&t.scene);
    let scene = Arc::new(t.scene);
    zero_opacity_sources(&scene, &mut warnings);
    let mut b = Builder {
        docs: vec![DocCtx {
            tokens: tokens_of(&scene),
            scene: scene.clone(),
            ns: "".into(),
            base: doc.base_dir().to_path_buf(),
            markers: markers.clone(),
            grid,
            duration,
            size: t.size,
        }],
        nodes: Vec::new(),
        slots: Vec::new(),
        slot_ix: HashMap::new(),
        channels: Vec::new(),
        exprs: Vec::new(),
        links: Vec::new(),
        elements: Vec::new(),
        transitions: Vec::new(),
        ids: HashMap::new(),
        diags,
        warnings,
        pending_expr: Vec::new(),
        pending_link: Vec::new(),
        project_seed: scene.project.seed,
        params: t.params,
        assets: HashMap::new(),
        analysis_tracks: HashSet::new(),
    };
    let root = Ctx { scope: "".into(), doc: 0, repeat: None, stack: Vec::new() };
    let roots = b.instantiate(&scene.composition.children, None, &root);
    b.transitions();
    b.resolve_links();
    // the animated parts of every node exist before anything reads them: a link or expression may name a bone
    // or another part that sits later in the document
    for n in 0..b.nodes.len() as u32 {
        if b.nodes[n as usize].name != "copy" {
            b.parts(n);
        }
    }
    for n in 0..b.nodes.len() as u32 {
        let node = &b.nodes[n as usize];
        if node.name == "copy" {
            continue;
        }
        let (elem, doc_ix, scope, id) = (node.elem.clone(), node.doc, node.scope.clone(), node.id.clone());
        b.animate(Owner::Node(n), &*elem, Some(n), doc_ix, &scope, &id);
        if let Some(c) = attr_str(&*elem, "condition") {
            let seed = b.project_seed;
            b.pending_expr.push((None, Some(n), c, seed, elem.loc(), scope.clone(), doc_ix));
        }
    }
    b.globals();
    b.compile_pending();
    let order = b.order();
    let mut slot_read = b.slot_read();
    b.fill_tf_slots();
    // transition handles: extend visibility of the nodes on both sides
    let windows: Vec<(Option<u32>, Option<u32>, (f64, f64))> =
        b.transitions.iter().map(|t| (t.from, t.to, t.window)).collect();
    let mut handles: HashMap<u32, (f64, f64)> = HashMap::new();
    for (from, to, (w0, w1)) in windows {
        if let Some(f) = from {
            handles
                .entry(f)
                .and_modify(|h| h.1 = if h.1.is_nan() { w1 } else { h.1.max(w1) })
                .or_insert((f64::NAN, w1));
        }
        if let Some(tn) = to {
            handles
                .entry(tn)
                .and_modify(|h| h.0 = if h.0.is_nan() { w0 } else { h.0.min(w0) })
                .or_insert((w0, f64::NAN));
        }
    }
    let mut vis = vec![(f64::NAN, f64::NAN); b.nodes.len()];
    for (k, (s, e)) in handles {
        vis[k as usize] = (s, e);
    }
    let tracks = crate::rig::load_tracks(&scene, &b.docs[0].base, &mut b.diags);
    let skin_dirs: Vec<std::path::PathBuf> = b.docs.iter().map(|d| d.base.clone()).collect();
    let skins = crate::rig::load_skins(&b.nodes, &skin_dirs, &mut b.diags);
    let errors: Vec<Diagnostic> = b.diags.iter().filter(|d| d.is_error()).cloned().collect();
    if !errors.is_empty() {
        let mut r = sr_model::Report { diagnostics: b.diags };
        r.diagnostics.extend(b.warnings);
        return Err(r);
    }
    let mut warnings = b.warnings;
    warnings.extend(b.diags);
    for (i, node) in b.nodes.iter_mut().enumerate() {
        node.vis_start = node.start;
        node.vis_end = node.end;
        let (s, e) = vis[i];
        if !s.is_nan() {
            node.vis_start = s.min(node.start);
        }
        if !e.is_nan() && e.is_finite() {
            node.vis_end = Some(node.end.map_or(e, |x| x.max(e)));
        }
    }
    let zs: Vec<i32> = b.nodes.iter().map(|n| n.z).collect();
    // an animated `z` (keys, expression or link) restacks its siblings every frame
    for i in 0..b.nodes.len() {
        if matches!(b.nodes[i].name, "object3D" | "particles3D" | "ocean" | "camera") {
            continue;
        }
        b.nodes[i].z_slot = b.nodes[i].slots.iter().copied().find(|&s| {
            let slot = &b.slots[s as usize];
            &*slot.prop == "z"
                && slot.owner == Owner::Node(i as u32)
                && (!slot.channels.is_empty() || slot.expr.is_some() || slot.link.is_some())
        });
    }
    // stacking reads every sibling's z before any is drawn, off-screen ones included (eval Frame::stacked)
    for n in &b.nodes {
        if let Some(s) = n.z_slot {
            slot_read[s as usize] = true;
        }
    }
    let dynamic: Vec<bool> = b.nodes.iter().map(|n| n.z_slot.is_some()).collect();
    for node in b.nodes.iter_mut() {
        node.doc_children = node.children.clone();
        node.children.sort_by_key(|&k| zs[k as usize]);
        node.restack = node.children.iter().any(|&k| dynamic[k as usize]);
    }
    let doc_roots = roots.clone();
    let restack_roots = roots.iter().any(|&k| dynamic[k as usize]);
    let mut roots = roots;
    roots.sort_by_key(|&k| zs[k as usize]);
    let includes = b.docs.iter().skip(1).map(|d| (d.ns.clone(), (*d.scene).clone())).collect();
    let resolved_safe_area = {
        let sid = opts
            .layout
            .as_ref()
            .and_then(|l| {
                scene
                    .layouts
                    .as_ref()
                    .and_then(|ls| ls.layouts.iter().find(|x| &x.id == l))
                    .and_then(|x| x.safe_area.clone())
            })
            .or_else(|| scene.project.safe_area.clone());
        sid.and_then(|id| scene.safe_areas.as_ref().and_then(|s| s.safe_areas.iter().find(|a| a.id == id)).cloned())
    };
    let safe_area = resolved_safe_area.as_ref().map(crate::safe_area::insets_of).unwrap_or([0.0; 4]);
    let safe_enforce =
        resolved_safe_area.as_ref().map(|a| crate::safe_area::SafeEnforce::of(a.enforce)).unwrap_or_default();
    let base_dirs = b.docs.iter().map(|d| d.base.clone()).collect();
    Ok(Program {
        identity: Arc::new(()),
        mesh_sequence_cache: Default::default(),
        joint_models: Default::default(),
        joint_sockets: Default::default(),
        base_dirs,
        safe_area,
        safe_enforce,
        safe_area_id: resolved_safe_area.as_ref().map(|a| a.id.clone()),
        scene: (*scene).clone(),
        includes,
        fps: scene.project.fps,
        duration,
        size: t.size,
        reframe: t.reframe,
        seed: b.project_seed,
        nodes: b.nodes,
        roots,
        doc_roots,
        restack_roots,
        slots: b.slots,
        order,
        slot_read,
        channels: b.channels,
        exprs: b.exprs,
        links: b.links,
        elements: b.elements,
        transitions: b.transitions,
        markers,
        beat: grid,
        params: b.params,
        analysis: opts.analysis.clone(),
        assets: b.assets,
        warnings,
        tracks,
        skins,
    })
}

/// Whether `value` is the attribute's declared default (an attribute set to its default is not worth a warning).
fn is_default(value: &AttrValue, default: Option<&str>) -> bool {
    let Some(d) = default else { return false };
    match value {
        AttrValue::Num(x) => d.parse::<f64>().is_ok_and(|v| v == *x),
        AttrValue::Bool(b) => d == if *b { "true" } else { "false" },
        other => other.to_string() == d,
    }
}

/// `effectType` is one attribute bag shared by every effect type, so the schema accepts any of its attributes on any type;
/// each type reads a subset (`sr_model::effect_attrs`). An attribute set to a value other than its default on a type that
/// does not read it does nothing, silently (E19, a warning): a vignette given `intensity` keeps the default `radius`.
fn unread_effect_attributes(e: &dyn Element, warnings: &mut Vec<Diagnostic>) {
    let Some(ty) = e.get_attr("type").map(|t| t.to_string()) else { return };
    let Some(reads) = sr_model::effect_attrs::declared(&ty) else { return };
    for decl in sr_model::xsd::COMPLEX_TYPES[e.xsd_type()].attrs {
        if sr_model::effect_attrs::ALWAYS.contains(&decl.name) || reads.contains(&decl.name) {
            continue;
        }
        let Some(value) = e.get_attr(decl.name) else { continue };
        if is_default(&value, decl.default) {
            continue;
        }
        let id = e.element_id().unwrap_or("");
        let list = if reads.is_empty() {
            "no attributes".to_string()
        } else {
            reads.iter().map(|r| format!("@{r}")).collect::<Vec<_>>().join(", ")
        };
        warnings.push(Diagnostic::warning(
            "E19",
            format!(
                "effect {id:?} of type {ty:?}: @{} is accepted but not read by this type, which reads {list}",
                decl.name
            ),
            e.loc(),
            id,
        ));
    }
}

/// Attributes the schema accepts that this build does not read: reported (E19, a warning) so that nothing is
/// accepted silently and then ignored.
fn ignored_attribute(e: &dyn Element, warnings: &mut Vec<Diagnostic>) {
    if matches!(e.element_name(), "effect" | "effectType") {
        unread_effect_attributes(e, warnings);
    }
    let mut note = |what: &str, why: &str| {
        warnings.push(Diagnostic::warning(
            "E19",
            format!("{what} is accepted but has {why}"),
            e.loc(),
            e.element_id().unwrap_or(""),
        ));
    };
    match e.element_name() {
        "group" if matches!(e.get_attr("collapse"), Some(AttrValue::Bool(true))) => note(
            "<group> @collapse",
            "no effect in this build (non-isolated groups already share the frame's camera space)",
        ),
        "effect" | "effectType"
            if e.get_attr("type").map(|t| t.to_string()).as_deref() == Some("selective-color")
                && e.get_attr("channel").map(|c| c.to_string()).is_some_and(|c| c != "rgb") =>
        {
            note(
                "selective-color @channel",
                "no effect: the effect reads hue, tolerance, saturation, brightness, color and amount",
            )
        }
        _ => {}
    }
}

/// A key's `overshoot` is read only by the `back-*` curves and its `period` only by the `elastic-*` ones: set on a
/// segment that leaves with another curve, the attribute does nothing (E19, a warning).
fn ignored_key_parameters(keys: &[m::Key], default: m::Curve, who: &str, warnings: &mut Vec<Diagnostic>) {
    use m::Curve::*;
    for (i, k) in keys.iter().enumerate() {
        let curve = k.interpolation.unwrap_or(default);
        let mut note = |attr: &str, family: &str| {
            warnings.push(Diagnostic::warning(
                "E19",
                format!(
                    "key {} of an animation on {who:?}: @{attr} is accepted but has no effect on the {curve:?} curve, which does not read it (only {family} curves do)",
                    i + 1
                ),
                k.loc,
                who,
            ))
        };
        if k.overshoot.is_some() && !matches!(curve, BackIn | BackOut | BackInOut) {
            note("overshoot", "back-*");
        }
        if k.period.is_some() && !matches!(curve, ElasticIn | ElasticOut | ElasticInOut) {
            note("period", "elastic-*");
        }
    }
}

/// Masks use the node's own coordinates: a rectangle or ellipse that adds to or intersects the node's shape and lies
/// entirely outside the node's box leaves nothing of it, which looks like the node vanishing (E20, a warning).
fn masks_that_miss(e: &dyn Element, warnings: &mut Vec<Diagnostic>) {
    let num = |e: &dyn Element, name: &str| match e.get_attr(name) {
        Some(AttrValue::Num(v)) => Some(v),
        Some(AttrValue::Length(l)) if l.unit == sr_model::values::LengthUnit::Px => Some(l.value),
        _ => None,
    };
    let (Some(w), Some(h)) = (num(e, "width"), num(e, "height")) else { return };
    for m in children(e).into_iter().filter(|c| c.element_name() == "mask") {
        let rect_like = matches!(m.get_attr("type").map(|t| t.to_string()).as_deref(), Some("rect" | "ellipse"));
        let subtracts = matches!(m.get_attr("mode").map(|t| t.to_string()).as_deref(), Some("subtract" | "difference"));
        let inverted = matches!(m.get_attr("invert"), Some(AttrValue::Bool(true)));
        let (Some(mw), Some(mh)) = (num(m, "width"), num(m, "height")) else { continue };
        if !rect_like || subtracts || inverted {
            continue;
        }
        let (x, y) = (num(m, "x").unwrap_or(0.0), num(m, "y").unwrap_or(0.0));
        if x >= w || y >= h || x + mw <= 0.0 || y + mh <= 0.0 {
            warnings.push(Diagnostic::warning(
                "E20",
                format!("a mask at ({x}, {y}) of {mw} x {mh} lies outside the node's {w} x {h} box: masks use the node's own coordinates"),
                m.loc(),
                e.element_id().unwrap_or(""),
            ));
        }
    }
}

/// A node named as the `source` of a displacement-map, difference-key or shader effect is drawn with its own opacity:
/// at 0 it contributes nothing and the effect does nothing, silently (E19, a warning). `visible="false"` at opacity 1
/// keeps a map off screen.
fn zero_opacity_sources(scene: &sr_model::model::Scene, warnings: &mut Vec<Diagnostic>) {
    let mut sources: Vec<(String, Loc)> = Vec::new();
    walk(scene, &mut |e| {
        if matches!(e.element_name(), "effect" | "effectType")
            && matches!(
                e.get_attr("type").map(|t| t.to_string()).as_deref(),
                Some("displacement-map" | "difference-key" | "shader")
            )
        {
            if let Some(AttrValue::Str(id)) = e.get_attr("source") {
                sources.push((id, e.loc()));
            }
        }
    });
    for (id, loc) in sources {
        let mut zero = false;
        walk(scene, &mut |e| {
            if e.element_id() == Some(id.as_str())
                && matches!(e.get_attr("opacity"), Some(AttrValue::Num(v)) if v <= 0.0)
            {
                zero = true;
            }
        });
        if zero {
            warnings.push(Diagnostic::warning(
                "E19",
                format!("effect source {id:?} has opacity 0, so it is accepted but contributes nothing; hide a map with visible=\"false\" and leave its opacity at 1"),
                loc,
                id.as_str(),
            ));
        }
    }
}
