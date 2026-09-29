//! Typography and data graphics in the renderer: text assets (styles,
//! spans, animators, text on a path), charts, audiograms, codes, formulas
//! and burned-in captions, drawn through `sr-text` into vector scenes.

use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use sr_eval::expr::vm::{self, Band, Host, LoopKind, Resolver, Var, V};
use sr_eval::{FrameGraph, FrameNode, Program, Value};
use sr_model::element::{children, Element};
use sr_model::model::{self as m, AssetsChild};
use sr_text::animate::{self, Animator, Combine, OnPath, Order, Preset, Props, Selector, Shape, Unit};
use sr_text::captions::{self, Cue, Page};
use sr_text::glyph::{self, BgMode, Decor, Drawing};
use sr_text::layout::{self, Align, AutoFit, Dir, Layout, Opts, Overflow, Para, Run, VAlign, Wrap, Writing};
use sr_text::style::{parse_variations, Decoration, StrokePos, Transform};
use sr_text::{chart, code, formula, FontLib, Style};
use sr_vector::geom::Xf;
use sr_vector::Paint;

use crate::vector::{is, Attrs};

#[path = "text_basemap.rs"]
mod basemap;
/// Paint resolution over a box (node-local x, y, w, h).
#[path = "text_map.rs"]
mod map;

pub type PaintFn<'a> = dyn FnMut(&Value, [f64; 4]) -> Option<Paint> + 'a;

/// A caption track ready to draw.
pub struct Track {
    pub id: String,
    pub burn: bool,
    pub preset: captions::Preset,
    pub cues: Vec<Cue>,
    pub pages: Vec<Page>,
    pub style: Option<String>,
    pub active_style: Option<String>,
    pub active_color: Option<Value>,
    pub x: sr_model::values::Length,
    pub y: sr_model::values::Length,
    pub width: sr_model::values::Length,
}

/// Caption tracks of a program (by program address), each loaded or failed.
type Tracks = Option<(usize, Arc<Vec<Result<Track, String>>>)>;
/// Chart data from a file: labels and series.
type ChartData = Result<(Vec<String>, Vec<chart::Series>), String>;

/// Fonts, layout and caption caches.
#[derive(Default)]
pub struct TextCache {
    lib: Option<FontLib>,
    font_assets: HashMap<String, Option<(PathBuf, u32)>>,
    layouts: HashMap<u64, Arc<Layout>>,
    exprs: HashMap<String, Option<Arc<vm::Code>>>,
    tracks: Tracks,
    data: HashMap<String, ChartData>,
}

impl TextCache {
    fn lib(&mut self) -> &mut FontLib {
        self.lib.get_or_insert_with(|| FontLib::new(true))
    }

    /// Drops layouts beyond a bound (text that changes every frame).
    fn trim(&mut self) {
        if self.layouts.len() > 4096 {
            self.layouts.clear();
        }
    }
}

/// Everything a drawing needs from the renderer.
pub struct Cx<'a> {
    pub p: &'a Program,
    pub g: &'a FrameGraph,
    pub n: &'a FrameNode,
    pub base: PathBuf,
    pub tol: f64,
    pub paint: &'a mut PaintFn<'a>,
    pub tokens: &'a HashMap<String, [f64; 4]>,
    pub unsupported: &'a mut Vec<String>,
}

fn resolve_path(src: &str, base: &FsPath) -> Option<PathBuf> {
    match sr_model::assets::resolve(src, base) {
        sr_model::assets::Resolved::Local(p) => Some(p),
        _ => None,
    }
}

fn hash_str(s: &str) -> u64 {
    sr_eval::rng::hash_str(s)
}

fn rgba_of(v: &Value, tokens: &HashMap<String, [f64; 4]>) -> Option<[f64; 4]> {
    match v {
        Value::Color(c) => Some(*c),
        Value::Str(s) if s.starts_with("token:") => tokens.get(&s[6..]).copied(),
        _ => None,
    }
}

struct StyleCx<'a, 'b> {
    p: &'a Program,
    base: &'a FsPath,
    paint: &'a mut PaintFn<'b>,
    tokens: &'a HashMap<String, [f64; 4]>,
    fonts: &'a HashMap<String, Option<(PathBuf, u32)>>,
    bx: [f64; 4],
}

/// Applies the characterStyle attributes (plus size, color, lineHeight) of an element.
fn apply(st: &mut Style, e: &dyn Element, cx: &mut StyleCx) {
    let a = Attrs { e, props: None };
    if let Some(v) = a.opt("size") {
        st.size = v;
    }
    if let Some(v) = a.paint("color") {
        st.color = (cx.paint)(&v, cx.bx);
    }
    if let Some(v) = a.opt("lineHeight") {
        st.line_height = Some(v);
    }
    if let Some(f) = a.str("font") {
        let mut fams: Vec<String> =
            f.split(',').map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string()).collect();
        fams.extend(st.families.iter().skip(1).cloned());
        st.families = fams;
    }
    if let Some(f) = a.str("fallback") {
        st.families.truncate(1);
        st.families.extend(f.split(',').map(|s| s.trim().trim_matches('"').to_string()));
    }
    if let Some(f) = a.str("fontFile") {
        st.file = resolve_path(&f, cx.base).map(|p| (p, 0));
    }
    if let Some(id) = a.str("fontAsset") {
        if let Some(Some(f)) = cx.fonts.get(&id) {
            st.file = Some(f.clone());
        }
    }
    if let Some(w) = a.opt("weight").or_else(|| a.str("weight").and_then(|s| s.parse().ok())) {
        st.weight = w.clamp(1.0, 1000.0) as u16;
    }
    if let Some(s) = a.str("fontStyle") {
        st.italic = s != "normal";
    }
    if let Some(s) = a.opt("stretch") {
        st.stretch = if s > 4.0 { s / 100.0 } else { s };
    }
    if let Some(v) = a.str("variation") {
        st.variations = parse_variations(&v);
    }
    if let Some(f) = a.str("features") {
        st.features = f.split(',').map(|s| s.trim().trim_matches('"').to_string()).filter(|s| !s.is_empty()).collect();
    }
    let sw = a.opt("strokeWidth");
    let sc = a.paint("strokeColor").and_then(|v| (cx.paint)(&v, cx.bx));
    let sp = a.str("strokePosition").map(|s| match s.as_str() {
        "inside" => StrokePos::Inside,
        "outside" => StrokePos::Outside,
        _ => StrokePos::Center,
    });
    if sc.is_some() || sw.is_some() || sp.is_some() {
        let (c0, w0, p0) = st.stroke.clone().unwrap_or((
            Paint::Solid { rgba: [0.0, 0.0, 0.0, 1.0], srgb: false },
            0.0,
            StrokePos::Center,
        ));
        let w = sw.unwrap_or(if w0 > 0.0 { w0 } else { st.size * 0.06 });
        st.stroke = (w > 0.0).then(|| (sc.unwrap_or(c0), w, sp.unwrap_or(p0)));
    }
    let shc = a.paint("shadowColor").and_then(|v| rgba_of(&v, cx.tokens));
    let (sx, sy, sb) = (a.opt("shadowOffsetX"), a.opt("shadowOffsetY"), a.opt("shadowBlur"));
    if shc.is_some() || sx.is_some() || sy.is_some() || sb.is_some() {
        let (c0, x0, y0, b0) = st.shadow.unwrap_or(([0.0, 0.0, 0.0, 0.6], 0.0, 0.0, 0.0));
        st.shadow = Some((shc.unwrap_or(c0), sx.unwrap_or(x0), sy.unwrap_or(y0), sb.unwrap_or(b0)));
    }
    if let Some(v) = a.opt("baselineShift") {
        st.baseline_shift = v;
    }
    if let Some(v) = a.opt("tracking") {
        st.tracking = v;
    }
    if let Some(t) = a.str("textTransform") {
        st.transform = match t.as_str() {
            "uppercase" => Transform::Upper,
            "lowercase" => Transform::Lower,
            "capitalize" => Transform::Capitalize,
            "small-caps" => Transform::SmallCaps,
            _ => Transform::None,
        };
    }
    if let Some(d) = a.str("decoration") {
        st.decoration = match d.as_str() {
            "underline" => Decoration::Underline,
            "line-through" => Decoration::LineThrough,
            "overline" => Decoration::Overline,
            _ => Decoration::None,
        };
    }
    if let Some(v) = a.paint("highlight") {
        st.highlight = (cx.paint)(&v, cx.bx);
    }
}

fn text_style<'p>(p: &'p Program, id: &str) -> Option<&'p m::TextStyle> {
    p.scene.styles.as_ref()?.children.iter().find_map(|c| match c {
        m::StylesChild::TextStyle(t) if t.id == id => Some(t),
        _ => None,
    })
}

/// Applies a textStyle and its basedOn chain (base first).
fn apply_named(st: &mut Style, id: &str, cx: &mut StyleCx, depth: u32) {
    let Some(ts) = text_style(cx.p, id) else { return };
    if depth < 16 {
        if let Some(b) = &ts.based_on {
            apply_named(st, b, cx, depth + 1);
        }
    }
    apply(st, ts, cx);
}

/// A style for a textStyle id (captions, chart labels) over defaults.
#[allow(clippy::too_many_arguments)]
fn style_for(
    p: &Program,
    id: Option<&str>,
    base: Style,
    fsb: &FsPath,
    paint: &mut PaintFn,
    tokens: &HashMap<String, [f64; 4]>,
    fonts: &HashMap<String, Option<(PathBuf, u32)>>,
    bx: [f64; 4],
) -> Style {
    let mut st = base;
    if let Some(id) = id {
        let mut cx = StyleCx { p, base: fsb, paint, tokens, fonts, bx };
        apply_named(&mut st, id, &mut cx, 0);
    }
    st
}

/// An asset by key (main document or include).
pub fn asset_of<'p>(p: &'p Program, key: &str) -> Option<(&'p AssetsChild, usize)> {
    let (doc, id) = p.assets.get(key)?;
    let scene = if *doc == 0 { &p.scene } else { &p.includes.get(*doc as usize - 1)?.1 };
    let a = scene.assets.as_ref()?.children.iter().find(|c| c.id() == Some(id.as_str()))?;
    Some((a, *doc as usize))
}

/// Loads the documents' font assets once; returns the ones that could not be read as fonts.
fn register_fonts(tc: &mut TextCache, p: &Program) -> Vec<String> {
    if !tc.font_assets.is_empty() {
        return Vec::new();
    }
    tc.font_assets.insert(String::new(), None);
    // fonts are referenced by text styles (fontAsset), never by layers, so they are not in the program's
    // asset table: read them from the documents themselves
    let mut found = Vec::new();
    let docs = std::iter::once(&p.scene).chain(p.includes.iter().map(|i| &i.1));
    for (doc, scene) in docs.enumerate() {
        let base = p.base_dirs.get(doc).cloned().unwrap_or_default();
        for c in scene.assets.iter().flat_map(|a| a.children.iter()) {
            if let (AssetsChild::Font(f), Some(id)) = (c, c.id()) {
                found.push((id.to_string(), resolve_path(&f.src, &base).map(|x| (x, f.collection_index as u32))));
            }
        }
    }
    let lib = tc.lib();
    let mut failed = Vec::new();
    for (k, v) in &found {
        if let Some((path, idx)) = v {
            if lib.file(path, *idx).is_none() {
                failed.push(format!("font asset {k}: {} is not a font this renderer can read", path.display()));
            }
        }
    }
    tc.font_assets.extend(found);
    failed
}

fn opts_of(t: &m::TextAsset) -> (Opts, Decor) {
    let a = Attrs { e: t, props: None };
    let s = |n: &str| a.str(n).unwrap_or_default();
    let o = Opts {
        width: t.width as f64,
        height: t.height as f64,
        align: match s("align").as_str() {
            "center" => Align::Center,
            "end" => Align::End,
            "justify" => Align::Justify,
            _ => Align::Start,
        },
        line_height: t.line_height,
        letter_spacing: t.letter_spacing,
        direction: match s("direction").as_str() {
            "ltr" => Dir::Ltr,
            "rtl" => Dir::Rtl,
            _ => Dir::Auto,
        },
        language: a.str("language"),
        valign: match s("verticalAlign").as_str() {
            "middle" => VAlign::Middle,
            "bottom" => VAlign::Bottom,
            _ => VAlign::Top,
        },
        writing: match s("writingMode").as_str() {
            "vertical-rl" => Writing::VerticalRl,
            "vertical-lr" => Writing::VerticalLr,
            _ => Writing::Horizontal,
        },
        wrap: match s("wrap").as_str() {
            "character" => Wrap::Character,
            "none" => Wrap::None,
            "balance" => Wrap::Balance,
            _ => Wrap::Word,
        },
        hyphenate: t.hyphenate,
        auto_fit: match s("autoFit").as_str() {
            "shrink" => AutoFit::Shrink,
            "grow" => AutoFit::Grow,
            "fit" => AutoFit::Fit,
            _ => AutoFit::None,
        },
        min_size: a.opt("minSize"),
        max_size: a.opt("maxSize"),
        max_lines: t.max_lines.map(|x| x as usize),
        overflow: match s("overflow").as_str() {
            "clip" => Overflow::Clip,
            "ellipsis" => Overflow::Ellipsis,
            _ => Overflow::Visible,
        },
        emoji_color: s("emoji") != "text",
    };
    let d = Decor {
        background: None,
        mode: match s("backgroundMode").as_str() {
            "line" => BgMode::Line,
            "word" => BgMode::Word,
            _ => BgMode::Block,
        },
        padding: a.num("backgroundPadding", 0.0),
        radius: a.num("backgroundRadius", 0.0),
    };
    (o, d)
}

fn layout_key(asset: &str, para: &Para) -> u64 {
    let mut p2 = para.clone();
    for s in &mut p2.styles {
        s.color = None;
        s.highlight = None;
        if let Some(st) = &mut s.stroke {
            st.0 = Paint::Solid { rgba: [0.0; 4], srgb: false };
        }
    }
    hash_str(&format!("{asset}|{p2:?}"))
}

fn cached_layout(tc: &mut TextCache, key: u64, para: &Para) -> Arc<Layout> {
    if let Some(l) = tc.layouts.get(&key) {
        let mut l2 = (**l).clone();
        l2.styles = para.styles.iter().map(|s| Style { size: s.size * l.scale, ..s.clone() }).collect();
        return Arc::new(l2);
    }
    tc.trim();
    let l = Arc::new(layout::layout(tc.lib(), para));
    tc.layouts.insert(key, l.clone());
    l
}

struct UnitHost {
    t: f64,
    fps: f64,
    index: f64,
    total: f64,
    seed: u64,
}

impl Host for UnitHost {
    fn var(&mut self, v: Var) -> V {
        V::Num(match v {
            Var::Time => self.t,
            Var::Frame => libm::floor(self.t * self.fps + 1e-9),
            Var::TextIndex | Var::Index => self.index,
            Var::TextTotal | Var::Count => self.total,
            Var::Seed => self.seed as f64,
            Var::Fps => self.fps,
            Var::Value => 100.0,
            Var::Duration => 0.0,
        })
    }
    fn prop(&mut self, _slot: u32) -> V {
        V::Num(0.0)
    }
    fn value_at_time(&mut self, _t: f64) -> V {
        V::Num(100.0)
    }
    fn param(&mut self, _name: &str) -> V {
        V::Undef
    }
    fn loop_value(&mut self, _out: bool, _kind: LoopKind, _keys: usize) -> V {
        V::Num(100.0)
    }
    fn audio(&mut self, _track: &str, _band: Band) -> f64 {
        0.0
    }
    fn beat(&mut self) -> f64 {
        0.0
    }
    fn random(&mut self, site: u32, component: u32) -> f64 {
        // hash of (seed, frame, call site + property · 2³², the "selector" property)
        let frame = libm::floor(self.t * self.fps + 1e-9) as i64 as u64;
        let index = site as u64 + (self.noise_channel() << 32) + ((component as u64) << 48);
        sr_eval::rng::d24_unit(self.seed, frame, index)
    }
    fn noise_seed(&mut self) -> u64 {
        self.seed
    }
    fn noise_channel(&mut self) -> u64 {
        vm::property_channel("selector")
    }
}

struct NoProps;

impl Resolver for NoProps {
    fn prop(&mut self, path: &str) -> Result<u32, String> {
        Err(format!("prop(\"{path}\") is not available in a text selector"))
    }
    fn marker(&mut self, _id: &str) -> Option<f64> {
        None
    }
}

fn animators(
    tc: &mut TextCache,
    cx: &mut Cx,
    lay: &Layout,
    roles: &[Option<String>],
) -> (Vec<Animator>, Option<OnPath>) {
    let n = cx.n;
    let e: &dyn Element = &*n.elem;
    let mut out = Vec::new();
    let mut path = None;
    let (mut ai, mut pi) = (0, 0);
    let bx = [0.0, 0.0, lay.size[0], lay.size[1]];
    // scramble letters: the layer's own seed for "scramble" xor the sum of the animators' @seed
    let node_seed = (Attrs { e, props: None }).opt("seed").map(|v| v as u64);
    let own_seeds = children(e)
        .into_iter()
        .filter(|c| is(*c, "textAnimator"))
        .map(|c| (Attrs { e: c, props: None }).opt("seed").map(|v| v as u64).unwrap_or(0))
        .fold(0u64, u64::wrapping_add);
    let scramble_seed = sr_eval::rng::element_seed(cx.p.seed, &n.id, node_seed, "scramble") ^ own_seeds;
    for c in children(e) {
        if is(c, "textPath") {
            let key = format!("{}/{}[{pi}]", n.id, c.element_name());
            pi += 1;
            let a = Attrs { e: c, props: crate::vector::part(n, &key) };
            let Some(d) = a.str("path") else { continue };
            match sr_vector::Path::parse(&d) {
                Ok(pth) => {
                    let first = lay.lines.first().map(|l| l.rect[2]).unwrap_or(0.0);
                    let so = match c.get_attr("startOffset") {
                        Some(sr_model::element::AttrValue::Length(l))
                            if l.unit == sr_model::values::LengthUnit::Percent =>
                        {
                            let len: f64 = pth
                                .flatten(0.1)
                                .iter()
                                .map(|q| q.pts.windows(2).map(|w| w[0].dist(w[1])).sum::<f64>())
                                .sum();
                            let _ = first;
                            l.value / 100.0 * len
                        }
                        _ => a.num("startOffset", 0.0),
                    };
                    path = Some(OnPath {
                        path: pth,
                        start_offset: so,
                        first_margin: a.num("firstMargin", 0.0),
                        last_margin: a.num("lastMargin", 0.0),
                        reverse: a.num("reverse", 0.0) != 0.0,
                        perpendicular: a.num("perpendicular", 1.0) != 0.0,
                        force_alignment: a.num("forceAlignment", 0.0) != 0.0,
                    });
                }
                Err(err) => cx.unsupported.push(format!("{}: textPath: {err}", n.id)),
            }
            continue;
        }
        if !is(c, "textAnimator") {
            continue;
        }
        let key = format!("{}/{}[{ai}]", n.id, c.element_name());
        ai += 1;
        let a = Attrs { e: c, props: crate::vector::part(n, &key) };
        let s = |name: &str| a.str(name).unwrap_or_default();
        let unit = match s("unit").as_str() {
            "character-no-space" => Unit::CharNoSpace,
            "word" => Unit::Word,
            "line" => Unit::Line,
            "span" => Unit::Span,
            _ => Unit::Char,
        };
        // seeded 64-bit hash draws with per-element seeds: CRC-32 of the project seed,
        // the element's id and @seed, and the purpose
        let own = a.opt("seed").map(|v| v as u64);
        let el_seed = |purpose: &str| sr_eval::rng::element_seed(cx.p.seed, "", own, purpose);
        let seed = el_seed("order");
        let amount = a.num("amount", 100.0);
        let t = n.local_time;
        // a nested <expression property="selector"> makes an expression selector
        let nested = children(c).into_iter().any(|x| {
            is(x, "expression") && x.get_attr("property").map(|v| v.to_string()).as_deref() == Some("selector")
        });
        let kind = if nested { "expression".to_string() } else { s("selector") };
        let selector = match kind.as_str() {
            "wiggly" => Selector::Wiggly { amount, rate: a.num("wiggleRate", 2.0), seed: el_seed("wiggly") & 0xffff },
            "expression" => {
                // the amount expression evaluated per unit with textIndex/textTotal
                let src = children(c)
                    .into_iter()
                    .find(|x| {
                        is(*x, "expression")
                            && matches!(
                                x.get_attr("property").map(|v| v.to_string()).as_deref(),
                                Some("selector" | "amount")
                            )
                    })
                    .map(|x| {
                        // seed: the expression's @seed, else the project's
                        let xs = Attrs { e: x, props: None }.opt("seed").map(|v| v as u64).unwrap_or(cx.p.seed);
                        (x.text().map(str::to_string), xs)
                    });
                let (src, xseed) = match src {
                    Some((s, xs)) => (s, xs),
                    None => (None, cx.p.seed),
                };
                let code = src.and_then(|src| {
                    tc.exprs
                        .entry(src.clone())
                        .or_insert_with(|| vm::compile(&src, &mut NoProps).ok().map(Arc::new))
                        .clone()
                });
                let (_, total) = unit_count(lay, unit);
                match code {
                    Some(code) => {
                        let mut regs = Vec::new();
                        Selector::Values(
                            (0..total)
                                .map(|i| {
                                    vm::run(
                                        &code,
                                        &mut UnitHost {
                                            t,
                                            fps: cx.p.fps.as_f64(),
                                            index: i as f64 + 1.0,
                                            total: total as f64,
                                            seed: xseed,
                                        },
                                        &mut regs,
                                    )
                                    .num()
                                })
                                .collect(),
                        )
                    }
                    None => {
                        cx.unsupported.push(format!(
                            "{}: textAnimator expression selector needs a valid <expression property=\"selector\">",
                            n.id
                        ));
                        Selector::Values(vec![amount; total])
                    }
                }
            }
            _ => Selector::Range {
                percent: s("rangeUnits") != "index",
                start: a.num("start", 0.0),
                end: a.num("end", 100.0),
                offset: a.num("offset", 0.0),
                amount,
                shape: match s("shape").as_str() {
                    "ramp-up" => Shape::RampUp,
                    "ramp-down" => Shape::RampDown,
                    "triangle" => Shape::Triangle,
                    "round" => Shape::Round,
                    "smooth" => Shape::Smooth,
                    _ => Shape::Square,
                },
                smoothness: a.num("smoothness", 1.0),
                ease_high: a.num("easeHigh", 0.0),
                ease_low: a.num("easeLow", 0.0),
                order: match s("order").as_str() {
                    "reverse" => Order::Reverse,
                    "center-out" => Order::CenterOut,
                    "edges-in" => Order::EdgesIn,
                    "random" => Order::Random,
                    _ => Order::Forward,
                },
                seed,
            },
        };
        let mut paint = |name: &str| a.paint(name).and_then(|v| (cx.paint)(&v, bx));
        let mut props = Props {
            variation: None,
            x: a.opt("x"),
            y: a.opt("y"),
            z_depth: a.opt("zDepth"),
            scale: a.opt("scale"),
            scale_x: a.opt("scaleX"),
            scale_y: a.opt("scaleY"),
            rotation: a.opt("rotation"),
            rotation_x: a.opt("rotationX"),
            rotation_y: a.opt("rotationY"),
            skew: a.opt("skew"),
            opacity: a.opt("opacity"),
            fill: paint("fill"),
            stroke: paint("stroke"),
            stroke_width: a.opt("strokeWidth"),
            tracking: a.opt("tracking"),
            line_spacing: a.opt("lineSpacing"),
            blur: a.opt("blur"),
            baseline_shift: a.opt("baselineShift"),
            char_offset: a.opt("characterOffset"),
            anchor_x: a.opt("anchorX"),
            anchor_y: a.opt("anchorY"),
        };
        props.variation = a.str("variation").map(|v| parse_variations(&v)).filter(|v| !v.is_empty());
        let preset = a
            .str("preset")
            .and_then(|p| Preset::parse(&p))
            // presetStart is on the layer's clock (its parent's timeline), by default the layer's start
            .map(|k| {
                (
                    k,
                    a.opt("presetStart").unwrap_or(n.timeline_time - n.local_time),
                    a.opt("presetDuration").unwrap_or(1.0),
                )
            });
        if props.fill.is_none() && preset.is_some_and(|p| p.0 == Preset::Karaoke) {
            // the default karaoke fill, resolved like a document colour so that it mixes with the text's
            props.fill = (cx.paint)(&Value::Color(animate::KARAOKE), bx);
        }
        out.push(Animator {
            unit,
            // the model fills XSD defaults, so the default values read as "not given"
            unit_set: unit != Unit::Char,
            role: a.str("span"),
            selector,
            props,
            combine: match s("combine").as_str() {
                "multiply" => Combine::Multiply,
                "replace" => Combine::Replace,
                _ => Combine::Add,
            },
            preset,
            stagger: a.opt("stagger"),
            overlap: a.opt("overlap").filter(|o| *o != 0.0),
            seed: scramble_seed,
        });
    }
    let _ = roles;
    (out, path)
}

/// Applies `counter` and `scramble` presets to the paragraph's text before layout: every
/// number (in the span with the animator's role, if any) counts up from 0, cubic-out over
/// the preset's duration, and scrambled characters show their random letters.
fn substitutions(cx: &Cx, para: &mut Para, anims: &[Animator]) {
    let t = cx.n.local_time;
    for c in children(&*cx.n.elem) {
        if !is(c, "textAnimator") || c.get_attr("preset").map(|v| v.to_string()).as_deref() != Some("counter") {
            continue;
        }
        let a = Attrs { e: c, props: None };
        let (start, dur) = (a.num("presetStart", 0.0), a.opt("presetDuration").unwrap_or(1.0));
        let Some(k) = animate::counter_progress(start, dur, a.num("amount", 100.0), t) else {
            continue;
        };
        let role = a.str("span");
        for r in &mut para.runs {
            if role.is_none() || r.role == role {
                r.text = animate::counter_text(&r.text, k);
            }
        }
    }
    for a in anims {
        let Some((Preset::Scramble, start, dur)) = a.preset else { continue };
        let mut chars: Vec<char> = Vec::new();
        let mut run_of: Vec<usize> = Vec::new();
        for (k, r) in para.runs.iter().enumerate() {
            chars.extend(r.text.chars());
            run_of.resize(chars.len(), k);
        }
        let pick = |i: usize| a.role.is_none() || para.runs[run_of[i]].role == a.role;
        let out = animate::scramble_text(&chars, &pick, a, start, dur, t);
        let mut it = out.into_iter();
        for r in &mut para.runs {
            let n = r.text.chars().count();
            r.text = it.by_ref().take(n).collect();
        }
    }
}

fn unit_count(lay: &Layout, unit: Unit) -> ((), usize) {
    let n = match unit {
        Unit::Char => lay.chars.len(),
        Unit::CharNoSpace => lay.chars.iter().filter(|c| !c.is_whitespace()).count(),
        Unit::Word => lay.word_count,
        Unit::Line => lay.lines.len(),
        Unit::Span => lay.char_span.iter().max().map(|m| m + 1).unwrap_or(0),
    };
    ((), n)
}

/// The paragraph of a text asset (with the layer's resolved text).
fn para_of(tc: &mut TextCache, cx: &mut Cx, t: &m::TextAsset) -> (Para, Decor, Vec<Option<String>>) {
    let (opts, mut decor) = opts_of(t);
    let bx = [0.0, 0.0, t.width as f64, t.height as f64];
    let fonts = std::mem::take(&mut tc.font_assets);
    let mut scx = StyleCx { p: cx.p, base: &cx.base, paint: &mut *cx.paint, tokens: cx.tokens, fonts: &fonts, bx };
    let mut base = Style::default();
    if let Some(id) = &t.style {
        apply_named(&mut base, id, &mut scx, 0);
    }
    apply(&mut base, t, &mut scx);
    if let Some(v) = (Attrs { e: t, props: None }).paint("background") {
        decor.background = (scx.paint)(&v, bx);
    }
    let mut styles = vec![base.clone()];
    let mut runs = Vec::new();
    let mut roles = Vec::new();
    let text = cx.n.text.as_deref().map(str::to_string).or_else(|| t.text.clone());
    if t.spans.is_empty() || text.as_deref().is_some_and(|s| !s.is_empty()) {
        runs.push(Run { text: text.unwrap_or_default(), style: 0, role: None });
        roles.push(None);
    } else {
        for sp in &t.spans {
            let mut st = base.clone();
            if let Some(id) = &sp.style {
                apply_named(&mut st, id, &mut scx, 0);
            }
            apply(&mut st, sp, &mut scx);
            styles.push(st);
            runs.push(Run { text: sp.value.clone(), style: styles.len() - 1, role: sp.role.clone() });
            roles.push(sp.role.clone());
        }
    }
    tc.font_assets = fonts;
    (Para { runs, styles, opts }, decor, roles)
}

/// Draws a text or data-graphics asset in its own box (0, 0, width, height).
pub fn asset_drawing(tc: &mut TextCache, cx: &mut Cx, key: &str, a: &AssetsChild) -> Option<Result<Drawing, String>> {
    let failed = register_fonts(tc, cx.p);
    cx.unsupported.extend(failed);
    let tol = cx.tol;
    Some(match a {
        AssetsChild::Text(t) => {
            let (mut para, decor, roles) = para_of(tc, cx, t);
            let (anims, on_path) = {
                // the animators as far as they do not depend on the layout
                let pre = cached_layout(tc, layout_key(key, &para), &para);
                animators(tc, cx, &pre, &roles)
            };
            substitutions(cx, &mut para, &anims);
            let lk = layout_key(key, &para);
            let lay = cached_layout(tc, lk, &para);
            let lib = tc.lib();
            let (mut fx, clip_lines) = if anims.is_empty() {
                (Vec::new(), Vec::new())
            } else {
                animate::apply(lib, &lay, &roles, &anims, cx.n.timeline_time)
            };
            if let Some(op) = &on_path {
                let pf = animate::on_path(&lay, op);
                if fx.is_empty() {
                    fx = vec![glyph::GlyphFx::default(); lay.glyphs.len()];
                }
                for (f, x) in fx.iter_mut().zip(pf) {
                    f.xf = x.mul(&f.xf);
                }
            }
            let mut d = glyph::draw(lib, &lay, (!fx.is_empty()).then_some(&fx[..]), &decor, tol);
            // mask-reveal clips each line to its box
            if clip_lines.iter().any(|c| *c) {
                let mut s = sr_vector::Scene::default();
                s.cmds.push(sr_vector::Cmd::Push { mask_init: 0.0 });
                s.extend(std::mem::take(&mut d.scene));
                let polys = lay
                    .lines
                    .iter()
                    .flat_map(|l| {
                        sr_vector::shapes::rect(l.rect[0] - 4.0, l.rect[1], l.rect[2] + 8.0, l.rect[3], [0.0; 4])
                            .flatten(tol)
                    })
                    .collect();
                s.cmds.push(sr_vector::Cmd::Mask {
                    polys,
                    rule: sr_vector::FillRule::NonZero,
                    op: sr_vector::MaskOp::Add,
                    opacity: 1.0,
                    invert: false,
                });
                s.cmds.push(sr_vector::Cmd::Pop { opacity: 1.0 });
                d.scene = s;
            }
            Ok(d)
        }
        AssetsChild::Chart(c) => chart_drawing(tc, cx, key, c),
        AssetsChild::Map(mp) => map::map_drawing(tc, cx, mp),
        AssetsChild::Audiogram(au) => {
            let at = Attrs { e: au, props: None };
            let color = at
                .paint("color")
                .and_then(|v| (cx.paint)(&v, [0.0, 0.0, au.width as f64, au.height as f64]))
                .unwrap_or(Paint::Solid { rgba: [1.0; 4], srgb: false });
            let an = &cx.p.analysis;
            let bars = au.bars.max(2) as usize;
            let dt = 1.0 / an.fps.max(1.0);
            let t = cx.g.time;
            if !an.tracks.contains_key(&au.source) {
                cx.unsupported.push(format!(
                    "{}: audiogram source {} has no analysis (render through an output with audio)",
                    cx.n.id, au.source
                ));
            }
            let history: Vec<f64> =
                (0..bars * 2).rev().map(|k| an.amplitude(&au.source, Band::Full, t - k as f64 * dt)).collect();
            let bands = [Band::Low, Band::Mid, Band::High].map(|b| an.amplitude(&au.source, b, t));
            let kind = sr_text::audiogram::Kind::parse(&at.str("style").unwrap_or_default());
            Ok(sr_text::audiogram::draw(
                kind,
                bars,
                [au.width as f64, au.height as f64],
                color,
                &history,
                bands,
                au.smoothing.get(),
                tol,
            ))
        }
        AssetsChild::Code(cd) => {
            let at = Attrs { e: cd, props: None };
            let bx = [0.0, 0.0, cd.width as f64, cd.height as f64];
            let fg = at
                .paint("foreground")
                .and_then(|v| (cx.paint)(&v, bx))
                .unwrap_or(Paint::Solid { rgba: [0.0, 0.0, 0.0, 1.0], srgb: false });
            let bg = at
                .paint("background")
                .and_then(|v| (cx.paint)(&v, bx))
                .unwrap_or(Paint::Solid { rgba: [0.0; 4], srgb: false });
            code::draw(
                &at.str("kind").unwrap_or_default(),
                &cd.data,
                [cd.width as f64, cd.height as f64],
                fg,
                bg,
                &at.str("errorCorrection").unwrap_or_else(|| "M".into()),
                cd.quiet_zone as u32,
                tol,
            )
        }
        AssetsChild::Formula(f) => {
            let at = Attrs { e: f, props: None };
            let color = at
                .paint("color")
                .and_then(|v| (cx.paint)(&v, [0.0, 0.0, f.width as f64, f.height as f64]))
                .unwrap_or(Paint::Solid { rgba: [1.0; 4], srgb: false });
            formula::draw(tc.lib(), &f.tex, f.size.get(), [f.width as f64, f.height as f64], color, tol)
        }
        _ => return None,
    })
}

fn load_data(path: &FsPath) -> ChartData {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) {
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let labels = v["labels"]
            .as_array()
            .map(|a| a.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect())
            .unwrap_or_default();
        let series = v["series"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|s| chart::Series {
                        name: s["name"].as_str().unwrap_or("").to_string(),
                        values: s["values"]
                            .as_array()
                            .map(|v| v.iter().filter_map(|x| x.as_f64()).collect())
                            .unwrap_or_default(),
                        color: None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        return Ok((labels, series));
    }
    // CSV: header row of series names; first column labels
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header: Vec<String> = lines
        .next()
        .ok_or_else(|| format!("{}: empty", path.display()))?
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .collect();
    let mut labels = Vec::new();
    let mut series: Vec<chart::Series> =
        header.iter().skip(1).map(|n| chart::Series { name: n.clone(), values: Vec::new(), color: None }).collect();
    for l in lines {
        let cells: Vec<&str> = l.split(',').map(|s| s.trim().trim_matches('"')).collect();
        labels.push(cells.first().copied().unwrap_or("").to_string());
        for (k, s) in series.iter_mut().enumerate() {
            s.values.push(cells.get(k + 1).and_then(|c| c.parse().ok()).unwrap_or(0.0));
        }
    }
    Ok((labels, series))
}

fn chart_drawing(tc: &mut TextCache, cx: &mut Cx, key: &str, c: &m::ChartAsset) -> Result<Drawing, String> {
    let at = Attrs { e: c, props: cx.g.elements.iter().find(|e| *e.key == *c.id).map(|e| &e.props) };
    let (w, h) = (c.width as f64, c.height as f64);
    let bx = [0.0, 0.0, w, h];
    let mut labels: Vec<String> =
        c.labels.as_deref().map(|l| l.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default();
    let mut series: Vec<chart::Series> = Vec::new();
    for ch in children(c) {
        if is(ch, "series") {
            let sa = Attrs { e: ch, props: None };
            series.push(chart::Series {
                name: sa.str("name").unwrap_or_default(),
                values: sa.nums("values").unwrap_or_default(),
                color: sa.paint("color").and_then(|v| (cx.paint)(&v, bx)),
            });
        }
    }
    if let Some(src) = &c.src {
        let path = resolve_path(src, &cx.base).ok_or_else(|| format!("{src}: only local files are supported"))?;
        let r = tc.data.entry(key.to_string()).or_insert_with(|| load_data(&path)).clone()?;
        if labels.is_empty() {
            labels = r.0;
        }
        if series.is_empty() {
            series = r.1;
        }
    }
    let fonts = std::mem::take(&mut tc.font_assets);
    let base = Style {
        size: (h / 18.0).clamp(10.0, 28.0),
        color: Some(Paint::Solid { rgba: [1.0; 4], srgb: false }),
        ..Default::default()
    };
    let text = style_for(cx.p, c.text_style.as_deref(), base, &cx.base, &mut *cx.paint, cx.tokens, &fonts, bx);
    tc.font_assets = fonts;
    let kind = chart::Kind::parse(&at.str("kind").unwrap_or_default()).unwrap_or(chart::Kind::Column);
    let spec = chart::Chart {
        kind,
        size: [w, h],
        labels,
        series,
        progress: at.num("progress", 1.0),
        show_axes: c.show_axes,
        show_values: c.show_values,
        format: c.format.clone(),
        text,
    };
    Ok(chart::draw(tc.lib(), &spec, cx.tol))
}

/// Loads the caption tracks of a program (once).
pub fn tracks(tc: &mut TextCache, p: &Program) -> Arc<Vec<Result<Track, String>>> {
    let pid = p as *const Program as usize;
    if let Some((id, t)) = &tc.tracks {
        if *id == pid {
            return t.clone();
        }
    }
    let base = p.base_dirs.first().cloned().unwrap_or_default();
    let mut out = Vec::new();
    for tr in p.scene.captions.as_ref().map(|c| c.caption_tracks.as_slice()).unwrap_or(&[]) {
        out.push(load_track(tr, &base));
    }
    let arc = Arc::new(out);
    tc.tracks = Some((pid, arc.clone()));
    arc
}

/// Reads a track's cues (inline, file or transcription cache).
pub fn track_cues(tr: &m::CaptionTrack, base: &FsPath) -> Result<Vec<Cue>, String> {
    let at = Attrs { e: tr, props: None };
    let mut cues: Vec<Cue> = tr
        .cues
        .iter()
        .map(|c| Cue {
            start: c.start,
            end: c.end,
            text: c
                .text
                .clone()
                .unwrap_or_else(|| c.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")),
            speaker: c.speaker.clone(),
            style: c.style.clone(),
            position: c.position.clone(),
            words: c
                .words
                .iter()
                .map(|w| captions::Word { start: w.start, end: w.end, text: w.text.clone(), emphasis: w.emphasis })
                .collect(),
        })
        .collect();
    if let Some(src) = &tr.src {
        let path =
            resolve_path(src, base).ok_or_else(|| format!("{}: {src}: only local files are supported", tr.id))?;
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {}: {e}", tr.id, path.display()))?;
        let fmt = at
            .str("format")
            .unwrap_or_else(|| path.extension().and_then(|e| e.to_str()).unwrap_or("srt").to_ascii_lowercase());
        cues.extend(captions::parse(&text, &fmt).map_err(|e| format!("{}: {e}", tr.id))?);
    }
    if tr.transcribe.is_some() {
        let Some(cache) = &tr.cache else {
            return Err(format!("{}: transcribe needs @cache (transcriptions are read from a verified cache so renders stay deterministic)", tr.id));
        };
        let path =
            resolve_path(cache, base).ok_or_else(|| format!("{}: {cache}: only local files are supported", tr.id))?;
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {}: {e}", tr.id, path.display()))?;
        cues.extend(captions::from_transcript(&text).map_err(|e| format!("{}: {e}", tr.id))?);
    }
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    if tr.profanity_filter {
        for c in &mut cues {
            c.text = captions::profanity(&c.text);
            for w in &mut c.words {
                w.text = captions::profanity(&w.text);
            }
        }
    }
    Ok(cues)
}

fn load_track(tr: &m::CaptionTrack, base: &FsPath) -> Result<Track, String> {
    let at = Attrs { e: tr, props: None };
    let cues = track_cues(tr, base)?;
    let pages = captions::paginate(
        &cues,
        tr.max_words_per_line.map(|x| x as usize),
        tr.max_chars_per_line as usize,
        tr.max_lines as usize,
        at.str("preset").as_deref() == Some("one-word"),
    );
    let mode = at.str("mode").unwrap_or_default();
    Ok(Track {
        id: tr.id.clone(),
        burn: mode != "sidecar",
        preset: captions::Preset::parse(&at.str("preset").unwrap_or_default()),
        cues,
        pages,
        style: tr.style.clone(),
        active_style: tr.active_style.clone(),
        active_color: at.paint("activeColor"),
        x: tr.x,
        y: tr.y,
        width: tr.width,
    })
}

fn len_px(l: &sr_model::values::Length, axis: f64, frame: [f64; 2]) -> f64 {
    use sr_model::values::LengthUnit::*;
    match l.unit {
        Percent => l.value / 100.0 * axis,
        Vw => l.value / 100.0 * frame[0],
        Vh => l.value / 100.0 * frame[1],
        _ => l.value,
    }
}

/// Burned-in captions at frame time, in frame space. `only` restricts to one track.
pub fn caption_scene(
    tc: &mut TextCache,
    p: &Program,
    g: &FrameGraph,
    paint: &mut PaintFn,
    tokens: &HashMap<String, [f64; 4]>,
    only: Option<&str>,
    errors: &mut Vec<String>,
) -> Option<(sr_vector::Scene, u64)> {
    let tracks = tracks(tc, p);
    let frame = g.size;
    let t = g.time;
    let base = p.base_dirs.first().cloned().unwrap_or_default();
    let mut scene = sr_vector::Scene::default();
    let mut hsh = 0u64;
    for tr in tracks.iter() {
        let tr = match tr {
            Ok(t) => t,
            Err(e) => {
                errors.push(e.clone());
                continue;
            }
        };
        let burn = match only {
            Some(id) => tr.id == id,
            None => tr.burn,
        };
        if !burn {
            continue;
        }
        let Some(page) = tr.pages.iter().find(|pg| t >= pg.start && t < pg.end) else { continue };
        let width = len_px(&tr.width, frame[0], frame);
        let (cxp, cyp) = (len_px(&tr.x, frame[0], frame), len_px(&tr.y, frame[1], frame));
        let bx = [0.0, 0.0, width, frame[1]];
        // default style (4.4 % of the frame height,
        // white DejaVu Sans at its regular weight, line height 1.2)
        let size = (frame[1] * 0.044 * 100.0).round() / 100.0;
        let default = Style {
            size,
            families: vec!["DejaVu Sans".into()],
            // resolved like document colours, so that karaoke cross-fades mix like with like
            color: paint(&Value::Color([1.0; 4]), bx),
            ..Default::default()
        };
        let fonts = std::mem::take(&mut tc.font_assets);
        let cue_style = tr.cues.get(page.cue).and_then(|c| c.style.clone());
        let mut st =
            style_for(p, cue_style.as_deref().or(tr.style.as_deref()), default, &base, paint, tokens, &fonts, bx);
        let size = st.size;
        if tr.preset == captions::Preset::Classic && st.shadow.is_none() && st.stroke.is_none() {
            // classic adds a soft drop shadow to a style with neither shadow nor stroke
            st.shadow = Some(([0.0, 0.0, 0.0, 0.8], 0.0, size * 0.04, size * 0.12));
        }
        let active_st =
            tr.active_style.as_deref().map(|id| style_for(p, Some(id), st.clone(), &base, paint, tokens, &fonts, bx));
        tc.font_assets = fonts;
        let active = active_st
            .as_ref()
            .and_then(|s| s.color.clone())
            .or_else(|| tr.active_color.as_ref().and_then(|v| paint(v, bx)))
            .or_else(|| paint(&Value::Color([1.0, 212.0 / 255.0, 0.0, 1.0]), bx))
            .unwrap_or(Paint::Solid { rgba: [1.0, 212.0 / 255.0, 0.0, 1.0], srgb: true });
        let boxed = tr.preset == captions::Preset::BoxedLine;
        let para = Para {
            runs: vec![Run { text: page.text(), style: 0, role: None }],
            styles: vec![st],
            // lines break where the pagination put them, never at the width
            opts: Opts {
                width,
                align: Align::Center,
                wrap: Wrap::None,
                emoji_color: false,
                line_height: 1.2,
                ..Default::default()
            },
        };
        let lay = cached_layout(tc, layout_key(&format!("caption:{}", tr.id), &para), &para);
        let decor = Decor {
            background: boxed.then_some(Paint::Solid { rgba: [0.0, 0.0, 0.0, 0xB3 as f64 / 255.0], srgb: true }),
            mode: BgMode::Line,
            padding: size * 0.25,
            radius: size * 0.2,
        };
        let fx = captions::effects(tr.preset, &lay, page, t, &active);
        if fx.iter().all(|f| f.opacity <= 0.0) {
            continue;
        }
        let d = glyph::draw(tc.lib(), &lay, Some(&fx), &decor, 0.1);
        // (x, y) is the centre of the caption block's top edge
        scene.extend(d.scene.transformed(&Xf::translate(cxp - width * 0.5, cyp)));
        hsh = sr_eval::rng::hash(&[hsh, hash_str(&tr.id), (t * 1000.0) as u64, page.start.to_bits()]);
    }
    (!scene.cmds.is_empty()).then_some((scene, hsh))
}

/// Glyph outlines of `text` set in `family` at `size` px (y down, baseline at 0),
/// flattened to polygons for 3D extrusion. Multi-line text stacks at the layout's line height.
pub fn outline_polygons(
    tc: &mut TextCache,
    p: &Program,
    text: &str,
    family: Option<&str>,
    size: f64,
    tol: f64,
) -> Vec<Vec<[f64; 2]>> {
    // a font that fails to load is reported when the document's text is drawn
    let _ = register_fonts(tc, p);
    let mut st = Style { size, ..Default::default() };
    if let Some(f) = family.filter(|f| !f.is_empty()) {
        st.families = f.split(',').map(|s| s.trim().trim_matches(|c| c == '"' || c == '\'').to_string()).collect();
    }
    let para = Para {
        runs: vec![Run { text: text.to_string(), style: 0, role: None }],
        styles: vec![st],
        opts: Opts::default(),
    };
    let lib = tc.lib();
    let lay = layout::layout(lib, &para);
    let mut out = Vec::new();
    for g in &lay.glyphs {
        let vars = &lay.styles.get(g.style).map(|s| s.variations.clone()).unwrap_or_default();
        let o = lib.outline(g.face, g.gid, vars);
        let s = g.size / lib.upem(g.face).max(1.0);
        let xf = Xf([s, 0.0, 0.0, -s, g.x, g.y]);
        for poly in o.transform(&xf).flatten(tol) {
            if poly.pts.len() >= 3 {
                out.push(poly.pts.iter().map(|q| [q.x, q.y]).collect());
            }
        }
    }
    out
}
