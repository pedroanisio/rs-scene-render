//! Lottie (Bodymovin JSON and dotLottie) import. A document is parsed once
//! into a layer tree and evaluated per frame into a [`Scene`] in
//! composition pixels. Evaluation covers keyframes with Bézier easing,
//! hold keys and spatial tangents; transforms with parenting, skew and
//! split position; precomps with stretch and time remap; solids; shape
//! layers with groups, rectangles, ellipses, polystars, paths, fills,
//! strokes (dashes included), linear and radial gradients, and the
//! modifiers trim, repeater, round corners, merge, pucker-bloat,
//! zig-zag, twist and offset path; masks in every mode; track mattes;
//! slots (`sid`) with overrides; markers and frame segments.
//! Expressions, images, text, effects and layer styles are reported.

use std::collections::HashMap;

use serde_json::Value;

use crate::geom::{p, Xf, P};
use crate::measure::TrimMode;
use crate::modifiers::{self, Item, Modifier};
use crate::path::{Contour, Path, Poly};
use crate::scene::{Cmd, FillRule, Gradient, GradientKind, MaskOp, MatteMode, Paint, Scene};
use crate::shapes;
use crate::stroke::{self, Cap, Join, Style};

/// A keyframe.
#[derive(Debug, Clone)]
struct Key {
    t: f64,
    s: Vec<f64>,
    e: Option<Vec<f64>>,
    hold: bool,
    o: ([f64; 2], [f64; 2]),
    i: ([f64; 2], [f64; 2]),
    to: Option<[f64; 2]>,
    ti: Option<[f64; 2]>,
}

/// An animatable property (numbers, colours, or a flattened Bézier shape).
#[derive(Debug, Clone, Default)]
struct Prop {
    value: Vec<f64>,
    keys: Vec<Key>,
    /// Shape properties: closed flag.
    closed: bool,
}

fn ease_xy(v: &Value, fallback: [f64; 2]) -> ([f64; 2], [f64; 2]) {
    let comp = |x: &Value| -> f64 {
        x.as_f64().or_else(|| x.as_array().and_then(|a| a.first()).and_then(Value::as_f64)).unwrap_or(0.0)
    };
    match v {
        Value::Object(m) => ([comp(&m["x"]), comp(&m["y"])], fallback),
        _ => (fallback, fallback),
    }
}

fn nums(v: &Value) -> Vec<f64> {
    match v {
        Value::Number(n) => vec![n.as_f64().unwrap_or(0.0)],
        Value::Array(a) if a.first().is_some_and(Value::is_object) => shape_vec(&a[0]).0,
        Value::Array(a) => a.iter().filter_map(Value::as_f64).collect(),
        Value::Object(_) => shape_vec(v).0,
        _ => Vec::new(),
    }
}

/// Flattens a Bézier shape `{v, i, o, c}` to [v…, i…, o…] (i and o relative).
fn shape_vec(v: &Value) -> (Vec<f64>, bool) {
    let pts = |k: &str| -> Vec<f64> {
        v[k].as_array().map(|a| a.iter().flat_map(|q| nums(q).into_iter().take(2)).collect()).unwrap_or_default()
    };
    let mut out = pts("v");
    out.extend(pts("i"));
    out.extend(pts("o"));
    (out, v["c"].as_bool().unwrap_or(false))
}

impl Prop {
    fn parse(v: &Value, slots: &HashMap<String, Value>) -> Prop {
        if let Some(sid) = v.get("sid").and_then(Value::as_str) {
            if let Some(s) = slots.get(sid) {
                return Prop::parse(s.get("p").unwrap_or(s), &HashMap::new());
            }
        }
        let k = v.get("k").unwrap_or(v);
        let animated = v.get("a").and_then(Value::as_i64) == Some(1)
            || k.as_array().is_some_and(|a| {
                a.first().is_some_and(|f| f.get("t").is_some() && (f.get("s").is_some() || f.get("h").is_some()))
            });
        if !animated {
            let closed = k
                .get("c")
                .and_then(Value::as_bool)
                .or_else(|| k.as_array().and_then(|a| a.first()).and_then(|f| f.get("c")).and_then(Value::as_bool));
            return Prop { value: nums(k), keys: Vec::new(), closed: closed.unwrap_or(false) };
        }
        let mut keys = Vec::new();
        let mut closed = false;
        for kf in k.as_array().into_iter().flatten() {
            let s = kf.get("s").map(|s| {
                if let Some(obj) = s.as_array().and_then(|a| a.first()).filter(|f| f.is_object()) {
                    let (v, c) = shape_vec(obj);
                    closed = c;
                    v
                } else {
                    nums(s)
                }
            });
            let e = kf.get("e").map(nums);
            let pair = |x: &Value| -> Option<[f64; 2]> {
                let a = nums(x);
                (a.len() >= 2).then(|| [a[0], a[1]])
            };
            keys.push(Key {
                t: kf["t"].as_f64().unwrap_or(0.0),
                s: s.unwrap_or_default(),
                e,
                hold: kf.get("h").and_then(Value::as_i64) == Some(1),
                o: ease_xy(kf.get("o").unwrap_or(&Value::Null), [0.0, 0.0]),
                i: ease_xy(kf.get("i").unwrap_or(&Value::Null), [1.0, 1.0]),
                to: kf.get("to").and_then(pair),
                ti: kf.get("ti").and_then(pair),
            });
        }
        // old-format files end with a key that carries only a time
        let n = keys.len();
        for j in 0..n {
            if keys[j].s.is_empty() && j > 0 {
                keys[j].s = keys[j - 1].e.clone().unwrap_or_else(|| keys[j - 1].s.clone());
            }
        }
        Prop { value: keys.first().map(|k| k.s.clone()).unwrap_or_default(), keys, closed }
    }

    fn at(&self, f: f64) -> Vec<f64> {
        let ks = &self.keys;
        if ks.is_empty() {
            return self.value.clone();
        }
        if f <= ks[0].t {
            return ks[0].s.clone();
        }
        let last = ks.len() - 1;
        if f >= ks[last].t {
            return if last > 0 && ks[last].s.is_empty() {
                ks[last - 1].e.clone().unwrap_or_default()
            } else {
                ks[last].s.clone()
            };
        }
        let j = ks.partition_point(|k| k.t <= f) - 1;
        let (a, b) = (&ks[j], &ks[j + 1]);
        if a.hold {
            return a.s.clone();
        }
        let end = a.e.clone().unwrap_or_else(|| b.s.clone());
        let u = if b.t > a.t { (f - a.t) / (b.t - a.t) } else { 1.0 };
        let y = bezier_ease(a.o.0, a.i.0, u);
        if let (Some(to), Some(ti)) = (a.to, a.ti) {
            if a.s.len() >= 2 && end.len() >= 2 && (to[0] != 0.0 || to[1] != 0.0 || ti[0] != 0.0 || ti[1] != 0.0) {
                let (p0, p3) = (p(a.s[0], a.s[1]), p(end[0], end[1]));
                let (p1, p2) = (p0 + p(to[0], to[1]), p3 + p(ti[0], ti[1]));
                let q = spatial(p0, p1, p2, p3, y);
                let mut out = a.s.iter().zip(&end).map(|(x, z)| x + (z - x) * y).collect::<Vec<_>>();
                out[0] = q.x;
                out[1] = q.y;
                return out;
            }
        }
        a.s.iter().zip(end.iter()).map(|(x, z)| x + (z - x) * y).collect()
    }

    fn num(&self, f: f64, d: f64) -> f64 {
        self.at(f).first().copied().unwrap_or(d)
    }
}

/// Point at arc-length fraction `u` of a cubic.
fn spatial(a: P, b: P, c: P, d: P, u: f64) -> P {
    const N: usize = 32;
    let pts: Vec<P> = (0..=N).map(|k| crate::path::cubic_point(a, b, c, d, k as f64 / N as f64)).collect();
    crate::measure::at_length(&pts, u * pts.windows(2).map(|w| w[0].dist(w[1])).sum::<f64>()).map(|x| x.0).unwrap_or(d)
}

/// y of a CSS-style cubic Bézier easing at x = `u`.
fn bezier_ease(o: [f64; 2], i: [f64; 2], u: f64) -> f64 {
    let (x1, y1, x2, y2) = (o[0], o[1], i[0], i[1]);
    let bx = |t: f64| 3.0 * (1.0 - t) * (1.0 - t) * t * x1 + 3.0 * (1.0 - t) * t * t * x2 + t * t * t;
    let by = |t: f64| 3.0 * (1.0 - t) * (1.0 - t) * t * y1 + 3.0 * (1.0 - t) * t * t * y2 + t * t * t;
    let (mut lo, mut hi) = (0.0, 1.0);
    let mut t = u;
    for _ in 0..40 {
        let x = bx(t);
        if (x - u).abs() < 1e-9 {
            break;
        }
        if x < u {
            lo = t;
        } else {
            hi = t;
        }
        t = (lo + hi) * 0.5;
    }
    by(t)
}

#[derive(Debug, Clone, Default)]
struct Transform {
    a: Prop,
    p: Prop,
    px: Option<Prop>,
    py: Option<Prop>,
    s: Prop,
    r: Prop,
    o: Prop,
    sk: Prop,
    sa: Prop,
}

impl Transform {
    fn parse(v: &Value, slots: &HashMap<String, Value>) -> Transform {
        let pr = |k: &str| v.get(k).map(|x| Prop::parse(x, slots)).unwrap_or_default();
        let split = v.get("p").and_then(|x| x.get("s")).and_then(Value::as_bool) == Some(true);
        Transform {
            a: pr("a"),
            p: pr("p"),
            px: split.then(|| Prop::parse(&v["p"]["x"], slots)),
            py: split.then(|| Prop::parse(&v["p"]["y"], slots)),
            s: v.get("s")
                .map(|x| Prop::parse(x, slots))
                .unwrap_or(Prop { value: vec![100.0, 100.0], ..Default::default() }),
            r: if v.get("r").is_some() { pr("r") } else { pr("rz") },
            o: v.get("o").map(|x| Prop::parse(x, slots)).unwrap_or(Prop { value: vec![100.0], ..Default::default() }),
            sk: pr("sk"),
            sa: pr("sa"),
        }
    }

    fn matrix(&self, f: f64) -> Xf {
        let a = self.a.at(f);
        let pos = match (&self.px, &self.py) {
            (Some(x), Some(y)) => vec![x.num(f, 0.0), y.num(f, 0.0)],
            _ => self.p.at(f),
        };
        let s = self.s.at(f);
        let g = |v: &[f64], k: usize, d: f64| v.get(k).copied().unwrap_or(d);
        let (sk, sa) = (self.sk.num(f, 0.0), self.sa.num(f, 0.0));
        let skew = if sk != 0.0 { Xf::rotate(sa).mul(&Xf::skew(-sk, 0.0)).mul(&Xf::rotate(-sa)) } else { Xf::IDENTITY };
        Xf::translate(g(&pos, 0, 0.0), g(&pos, 1, 0.0))
            .mul(&Xf::rotate(self.r.num(f, 0.0)))
            .mul(&skew)
            .mul(&Xf::scale(g(&s, 0, 100.0) / 100.0, g(&s, 1, 100.0) / 100.0))
            .mul(&Xf::translate(-g(&a, 0, 0.0), -g(&a, 1, 0.0)))
    }

    fn opacity(&self, f: f64) -> f64 {
        (self.o.num(f, 100.0) / 100.0).clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone)]
enum ShapeItem {
    Group { items: Vec<ShapeItem>, tr: Option<Transform>, hidden: bool },
    Rect { p: Prop, s: Prop, r: Prop },
    Ellipse { p: Prop, s: Prop },
    Star { star: bool, p: Prop, pt: Prop, r: Prop, or: Prop, ir: Prop, os: Prop, is: Prop },
    Path { ks: Prop },
    Fill { c: Prop, o: Prop, rule: FillRule },
    Stroke { c: Prop, o: Prop, w: Prop, cap: Cap, join: Join, ml: f64, dash: Vec<(char, Prop)> },
    GradFill { g: Grad, o: Prop, rule: FillRule },
    GradStroke { g: Grad, o: Prop, w: Prop, cap: Cap, join: Join, ml: f64, dash: Vec<(char, Prop)> },
    Trim { s: Prop, e: Prop, o: Prop, sequential: bool },
    Repeater { c: Prop, o: Prop, below: bool, tr: Transform, so: Prop, eo: Prop },
    RoundCorners { r: Prop },
    Merge { op: MaskOp },
    PuckerBloat { a: Prop },
    ZigZag { s: Prop, r: Prop, smooth: bool },
    Twist { a: Prop },
    Offset { a: Prop, join: Join, ml: Prop },
    Skip,
}

#[derive(Debug, Clone)]
struct Grad {
    radial: bool,
    s: Prop,
    e: Prop,
    h: Prop,
    a: Prop,
    count: usize,
    k: Prop,
}

#[derive(Debug, Clone)]
struct MaskDef {
    op: Option<MaskOp>,
    pt: Prop,
    o: Prop,
    inv: bool,
}

#[derive(Debug, Clone)]
struct Layer {
    ty: i64,
    ind: Option<i64>,
    parent: Option<i64>,
    ip: f64,
    op: f64,
    st: f64,
    sr: f64,
    ks: Transform,
    masks: Vec<MaskDef>,
    matte: Option<MatteMode>,
    matte_parent: Option<i64>,
    is_matte: bool,
    hidden: bool,
    ref_id: Option<String>,
    size: [f64; 2],
    solid: [f64; 4],
    shapes: Vec<ShapeItem>,
    tm: Option<Prop>,
}

/// A parsed Lottie animation.
#[derive(Debug, Clone)]
pub struct Lottie {
    /// Frame rate.
    pub fps: f64,
    /// In and out frames.
    pub ip: f64,
    pub op: f64,
    /// Composition size.
    pub size: [f64; 2],
    /// Markers: name → (start frame, duration in frames).
    pub markers: HashMap<String, (f64, f64)>,
    layers: Vec<Layer>,
    comps: HashMap<String, Vec<Layer>>,
    /// Features that were skipped.
    pub skipped: Vec<String>,
}

fn cap_of(v: &Value) -> Cap {
    match v.as_i64() {
        Some(2) => Cap::Round,
        Some(3) => Cap::Square,
        _ => Cap::Butt,
    }
}
fn join_of(v: &Value) -> Join {
    match v.as_i64() {
        Some(2) => Join::Round,
        Some(3) => Join::Bevel,
        _ => Join::Miter,
    }
}
fn rule_of(v: &Value) -> FillRule {
    if v.as_i64() == Some(2) {
        FillRule::EvenOdd
    } else {
        FillRule::NonZero
    }
}

struct Parser<'a> {
    slots: &'a HashMap<String, Value>,
    skipped: Vec<String>,
}

impl Parser<'_> {
    fn prop(&self, v: &Value, k: &str) -> Prop {
        v.get(k).map(|x| Prop::parse(x, self.slots)).unwrap_or_default()
    }

    fn dashes(&self, v: &Value) -> Vec<(char, Prop)> {
        v.get("d")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|d| (d["n"].as_str().and_then(|s| s.chars().next()).unwrap_or('d'), self.prop(d, "v")))
            .collect()
    }

    fn grad(&self, v: &Value) -> Grad {
        Grad {
            radial: v["t"].as_i64() == Some(2),
            s: self.prop(v, "s"),
            e: self.prop(v, "e"),
            h: self.prop(v, "h"),
            a: self.prop(v, "a"),
            count: v["g"]["p"].as_u64().unwrap_or(0) as usize,
            k: v.get("g").map(|g| self.prop(g, "k")).unwrap_or_default(),
        }
    }

    fn shapes(&mut self, items: &Value) -> Vec<ShapeItem> {
        let mut out = Vec::new();
        for it in items.as_array().into_iter().flatten() {
            let ty = it["ty"].as_str().unwrap_or("");
            let hidden = it["hd"].as_bool().unwrap_or(false);
            let s = match ty {
                "gr" => {
                    let children = it["it"].as_array().cloned().unwrap_or_default();
                    let tr = children.iter().find(|c| c["ty"] == "tr").map(|t| Transform::parse(t, self.slots));
                    let rest: Vec<Value> = children.into_iter().filter(|c| c["ty"] != "tr").collect();
                    ShapeItem::Group { items: self.shapes(&Value::Array(rest)), tr, hidden }
                }
                _ if hidden => ShapeItem::Skip,
                "rc" => ShapeItem::Rect { p: self.prop(it, "p"), s: self.prop(it, "s"), r: self.prop(it, "r") },
                "el" => ShapeItem::Ellipse { p: self.prop(it, "p"), s: self.prop(it, "s") },
                "sr" => ShapeItem::Star {
                    star: it["sy"].as_i64() != Some(2),
                    p: self.prop(it, "p"),
                    pt: self.prop(it, "pt"),
                    r: self.prop(it, "r"),
                    or: self.prop(it, "or"),
                    ir: self.prop(it, "ir"),
                    os: self.prop(it, "os"),
                    is: self.prop(it, "is"),
                },
                "sh" => ShapeItem::Path { ks: self.prop(it, "ks") },
                "fl" => ShapeItem::Fill { c: self.prop(it, "c"), o: self.prop(it, "o"), rule: rule_of(&it["r"]) },
                "st" => ShapeItem::Stroke {
                    c: self.prop(it, "c"),
                    o: self.prop(it, "o"),
                    w: self.prop(it, "w"),
                    cap: cap_of(&it["lc"]),
                    join: join_of(&it["lj"]),
                    ml: it["ml"].as_f64().unwrap_or(4.0),
                    dash: self.dashes(it),
                },
                "gf" => ShapeItem::GradFill { g: self.grad(it), o: self.prop(it, "o"), rule: rule_of(&it["r"]) },
                "gs" => ShapeItem::GradStroke {
                    g: self.grad(it),
                    o: self.prop(it, "o"),
                    w: self.prop(it, "w"),
                    cap: cap_of(&it["lc"]),
                    join: join_of(&it["lj"]),
                    ml: it["ml"].as_f64().unwrap_or(4.0),
                    dash: self.dashes(it),
                },
                "tm" => ShapeItem::Trim {
                    s: self.prop(it, "s"),
                    e: self.prop(it, "e"),
                    o: self.prop(it, "o"),
                    sequential: it["m"].as_i64() == Some(2),
                },
                "rp" => ShapeItem::Repeater {
                    c: self.prop(it, "c"),
                    o: self.prop(it, "o"),
                    below: it["m"].as_i64() == Some(2),
                    tr: Transform::parse(&it["tr"], self.slots),
                    so: it["tr"]
                        .get("so")
                        .map(|x| Prop::parse(x, self.slots))
                        .unwrap_or(Prop { value: vec![100.0], ..Default::default() }),
                    eo: it["tr"]
                        .get("eo")
                        .map(|x| Prop::parse(x, self.slots))
                        .unwrap_or(Prop { value: vec![100.0], ..Default::default() }),
                },
                "rd" => ShapeItem::RoundCorners { r: self.prop(it, "r") },
                "mm" => ShapeItem::Merge {
                    op: match it["mm"].as_i64() {
                        Some(3) => MaskOp::Subtract,
                        Some(4) => MaskOp::Intersect,
                        Some(5) => MaskOp::Difference,
                        _ => MaskOp::Add,
                    },
                },
                "pb" => ShapeItem::PuckerBloat { a: self.prop(it, "a") },
                "zz" => ShapeItem::ZigZag {
                    s: self.prop(it, "s"),
                    r: self.prop(it, "r"),
                    smooth: it["pt"].get("k").and_then(Value::as_f64) == Some(2.0),
                },
                "tw" => ShapeItem::Twist { a: self.prop(it, "a") },
                "op" => ShapeItem::Offset { a: self.prop(it, "a"), join: join_of(&it["lj"]), ml: self.prop(it, "ml") },
                "tr" => ShapeItem::Skip,
                other => {
                    self.skipped.push(format!("shape item {other:?}"));
                    ShapeItem::Skip
                }
            };
            out.push(s);
        }
        out
    }

    fn layers(&mut self, v: &Value) -> Vec<Layer> {
        let mut out = Vec::new();
        for l in v.as_array().into_iter().flatten() {
            let ty = l["ty"].as_i64().unwrap_or(-1);
            match ty {
                2 => self.skipped.push("image layer".into()),
                5 => self.skipped.push("text layer (Batch 6)".into()),
                0 | 1 | 3 | 4 => {}
                other => self.skipped.push(format!("layer type {other}")),
            }
            if l.get("ef").is_some_and(|e| e.as_array().is_some_and(|a| !a.is_empty())) {
                self.skipped.push("layer effects".into());
            }
            let masks = l["masksProperties"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|m| MaskDef {
                    op: match m["mode"].as_str().unwrap_or("a") {
                        "a" => Some(MaskOp::Add),
                        "s" => Some(MaskOp::Subtract),
                        "i" => Some(MaskOp::Intersect),
                        "f" => Some(MaskOp::Difference),
                        "l" => Some(MaskOp::Lighten),
                        "d" => Some(MaskOp::Darken),
                        _ => None,
                    },
                    pt: self.prop(m, "pt"),
                    o: m.get("o")
                        .map(|x| Prop::parse(x, self.slots))
                        .unwrap_or(Prop { value: vec![100.0], ..Default::default() }),
                    inv: m["inv"].as_bool().unwrap_or(false),
                })
                .collect();
            let hex = l["sc"].as_str().unwrap_or("#000000").trim_start_matches('#');
            let byte = |k: usize| u8::from_str_radix(hex.get(k..k + 2).unwrap_or("00"), 16).unwrap_or(0) as f64 / 255.0;
            out.push(Layer {
                ty,
                ind: l["ind"].as_i64(),
                parent: l["parent"].as_i64(),
                ip: l["ip"].as_f64().unwrap_or(0.0),
                op: l["op"].as_f64().unwrap_or(f64::INFINITY),
                st: l["st"].as_f64().unwrap_or(0.0),
                sr: l["sr"].as_f64().unwrap_or(1.0).max(1e-6),
                ks: Transform::parse(&l["ks"], self.slots),
                masks,
                matte: match l["tt"].as_i64() {
                    Some(1) => Some(MatteMode::Alpha),
                    Some(2) => Some(MatteMode::AlphaInverted),
                    Some(3) => Some(MatteMode::Luma),
                    Some(4) => Some(MatteMode::LumaInverted),
                    _ => None,
                },
                matte_parent: l["tp"].as_i64(),
                is_matte: l["td"].as_i64() == Some(1),
                hidden: l["hd"].as_bool().unwrap_or(false),
                ref_id: l["refId"].as_str().map(str::to_string),
                size: [
                    l["w"].as_f64().or(l["sw"].as_f64()).unwrap_or(0.0),
                    l["h"].as_f64().or(l["sh"].as_f64()).unwrap_or(0.0),
                ],
                solid: [byte(0), byte(2), byte(4), 1.0],
                shapes: if ty == 4 { self.shapes(&l["shapes"]) } else { Vec::new() },
                tm: l.get("tm").map(|x| Prop::parse(x, self.slots)),
            });
        }
        out
    }
}

/// Parses a slot override value: numbers (`"1, 2"`), a colour (`#RRGGBB[AA]`) or JSON.
fn override_value(s: &str) -> Value {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix('#') {
        let b = |k: usize| u8::from_str_radix(hex.get(k..k + 2).unwrap_or("ff"), 16).unwrap_or(255) as f64 / 255.0;
        let a = if hex.len() >= 8 { b(6) } else { 1.0 };
        return serde_json::json!({"a": 0, "k": [b(0), b(2), b(4), a]});
    }
    let parts: Option<Vec<f64>> = t.split(',').map(|x| x.trim().parse::<f64>().ok()).collect();
    if let Some(v) = parts {
        return if v.len() == 1 { serde_json::json!({"a": 0, "k": v[0]}) } else { serde_json::json!({"a": 0, "k": v}) };
    }
    serde_json::from_str::<Value>(t)
        .map(|v| if v.get("k").is_some() { v } else { serde_json::json!({"a": 0, "k": v}) })
        .unwrap_or(Value::Null)
}

impl Lottie {
    /// Parses Lottie JSON or a dotLottie archive (`animation` picks an id inside it).
    pub fn parse(data: &[u8], animation: Option<&str>, overrides: &[(String, String)]) -> Result<Lottie, String> {
        let json: Vec<u8> = if data.starts_with(b"PK") {
            let files = crate::zip::entries(data)?;
            let want = |n: &str| match animation {
                Some(id) => n == format!("animations/{id}.json") || n == format!("a/{id}.json"),
                None => (n.starts_with("animations/") || n.starts_with("a/")) && n.ends_with(".json"),
            };
            files.into_iter().find(|(n, _)| want(n)).map(|x| x.1).ok_or_else(|| match animation {
                Some(id) => format!("dotLottie has no animation {id:?}"),
                None => "dotLottie has no animations".into(),
            })?
        } else {
            data.to_vec()
        };
        let v: Value = serde_json::from_slice(&json).map_err(|e| format!("Lottie JSON: {e}"))?;
        if v.get("layers").is_none() {
            return Err("not a Lottie animation (no layers)".into());
        }
        let mut slots: HashMap<String, Value> =
            v["slots"].as_object().map(|m| m.iter().map(|(k, x)| (k.clone(), x.clone())).collect()).unwrap_or_default();
        for (id, val) in overrides {
            slots.insert(id.clone(), serde_json::json!({ "p": override_value(val) }));
        }
        let mut pr = Parser { slots: &slots, skipped: Vec::new() };
        let layers = pr.layers(&v["layers"]);
        let mut comps = HashMap::new();
        for a in v["assets"].as_array().into_iter().flatten() {
            if let (Some(id), Some(ls)) = (a["id"].as_str(), a.get("layers")) {
                comps.insert(id.to_string(), pr.layers(ls));
            }
        }
        let markers = v["markers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some((
                    m["cm"].as_str()?.to_string(),
                    (m["tm"].as_f64().unwrap_or(0.0), m["dr"].as_f64().unwrap_or(0.0)),
                ))
            })
            .collect();
        let mut skipped = pr.skipped;
        let expr = json.windows(4).any(|w| w == b"\"x\":") && String::from_utf8_lossy(&json).contains("\"x\":\"");
        if expr {
            skipped.push("expressions (keyframed values used)".into());
        }
        skipped.sort();
        skipped.dedup();
        Ok(Lottie {
            fps: v["fr"].as_f64().unwrap_or(30.0).max(1e-3),
            ip: v["ip"].as_f64().unwrap_or(0.0),
            op: v["op"].as_f64().unwrap_or(0.0),
            size: [v["w"].as_f64().unwrap_or(0.0), v["h"].as_f64().unwrap_or(0.0)],
            markers,
            layers,
            comps,
            skipped,
        })
    }

    /// Frame range of a segment: a marker name or `"start,end"` frames.
    pub fn segment(&self, s: &str) -> Option<(f64, f64)> {
        if let Some(&(t, d)) = self.markers.get(s) {
            return Some((t, t + d));
        }
        let (a, b) = s.split_once(',')?;
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    }

    /// Frame shown at `seconds` into the animation (held at the last frame).
    pub fn frame_at(&self, seconds: f64, segment: Option<(f64, f64)>) -> f64 {
        let (a, b) = segment.unwrap_or((self.ip, self.op));
        (a + seconds.max(0.0) * self.fps).min((b - 1e-6).max(a))
    }

    /// Evaluates frame `f` into a scene in composition pixels, flattening within `tol`.
    pub fn render(&self, f: f64, tol: f64) -> Scene {
        let mut out = Scene::default();
        self.comp(&self.layers, f, &Xf::IDENTITY, tol, &mut out, 0);
        out
    }

    fn comp(&self, layers: &[Layer], f: f64, base: &Xf, tol: f64, out: &mut Scene, depth: usize) {
        if depth > 16 {
            return;
        }
        let by_ind: HashMap<i64, usize> =
            layers.iter().enumerate().filter_map(|(i, l)| l.ind.map(|x| (x, i))).collect();
        let world = |i: usize| -> Xf {
            let mut m = layers[i].ks.matrix(f);
            let mut cur = layers[i].parent;
            let mut guard = 0;
            while let Some(pi) = cur.and_then(|x| by_ind.get(&x).copied()) {
                m = layers[pi].ks.matrix(f).mul(&m);
                cur = layers[pi].parent;
                guard += 1;
                if guard > layers.len() {
                    break;
                }
            }
            base.mul(&m)
        };
        // bottom layer first
        for i in (0..layers.len()).rev() {
            let l = &layers[i];
            if l.is_matte || l.hidden || f < l.ip || f >= l.op {
                continue;
            }
            let matte = l.matte.and_then(|mode| {
                let src = match l.matte_parent {
                    Some(tp) => by_ind.get(&tp).copied(),
                    None => i.checked_sub(1),
                }?;
                Some((src, mode))
            });
            let x = world(i);
            let opacity = l.ks.opacity(f);
            let masked = l.masks.iter().any(|m| m.op.is_some());
            let clip_comp = l.ty == 0 && l.size[0] > 0.0 && l.size[1] > 0.0;
            let isolate = masked || matte.is_some() || opacity < 1.0 || clip_comp;
            if isolate {
                let first = l.masks.iter().find_map(|m| m.op);
                let starts_empty = matches!(first, Some(MaskOp::Add) | Some(MaskOp::Lighten)) || (clip_comp && !masked);
                out.cmds.push(Cmd::Push { mask_init: if starts_empty { 0.0 } else { 1.0 } });
            }
            self.layer_content(l, f, &x, tol, out, depth);
            if !isolate {
                continue;
            }
            for m in &l.masks {
                if let Some(op) = m.op {
                    let polys = bezier_path(&m.pt, f).transform(&x).flatten(tol);
                    out.cmds.push(Cmd::Mask {
                        polys,
                        rule: FillRule::NonZero,
                        op,
                        opacity: (m.o.num(f, 100.0) / 100.0).clamp(0.0, 1.0),
                        invert: m.inv,
                    });
                }
            }
            // precomps clip to their size
            if clip_comp && !masked {
                let r = shapes::rect(0.0, 0.0, l.size[0], l.size[1], [0.0; 4]).transform(&x);
                out.cmds.push(Cmd::Mask {
                    polys: r.flatten(tol),
                    rule: FillRule::NonZero,
                    op: MaskOp::Add,
                    opacity: 1.0,
                    invert: false,
                });
            }
            match matte {
                Some((src, mode)) => {
                    out.cmds.push(Cmd::PushMatte);
                    let ml = &layers[src];
                    if f >= ml.ip && f < ml.op {
                        let mx = world(src);
                        let mo = ml.ks.opacity(f);
                        let mut tmp = Scene::default();
                        self.layer_content(ml, f, &mx, tol, &mut tmp, depth);
                        for c in tmp.cmds {
                            out.cmds.push(match c {
                                Cmd::Fill { polys, rule, paint, opacity } => {
                                    Cmd::Fill { polys, rule, paint, opacity: opacity * mo }
                                }
                                other => other,
                            });
                        }
                    }
                    out.cmds.push(Cmd::PopMatte { mode, opacity });
                }
                None => out.cmds.push(Cmd::Pop { opacity }),
            }
        }
    }

    fn layer_content(&self, l: &Layer, f: f64, x: &Xf, tol: f64, out: &mut Scene, depth: usize) {
        match l.ty {
            0 => {
                let Some(layers) = l.ref_id.as_ref().and_then(|r| self.comps.get(r)) else { return };
                let local = match &l.tm {
                    Some(tm) => tm.num(f, 0.0) * self.fps,
                    None => (f - l.st) / l.sr,
                };
                self.comp(layers, local, x, tol, out, depth + 1);
            }
            1 => {
                let r = shapes::rect(0.0, 0.0, l.size[0], l.size[1], [0.0; 4]).transform(x);
                out.fill(&r, FillRule::NonZero, Paint::Solid { rgba: l.solid, srgb: true }, 1.0, tol);
            }
            4 => {
                let draws = eval_items(&l.shapes, f, tol);
                for d in draws.into_iter().rev() {
                    emit_draw(d, x, tol, out);
                }
            }
            _ => {}
        }
    }
}

fn bezier_path(pr: &Prop, f: f64) -> Path {
    let v = pr.at(f);
    let n = v.len() / 6;
    if n == 0 {
        return Path::default();
    }
    let pt = |k: usize| p(v[2 * k], v[2 * k + 1]);
    let mut c = Contour { closed: pr.closed, ..Default::default() };
    for k in 0..n {
        let vv = pt(k);
        c.v.push(vv);
        c.i.push(vv + pt(n + k));
        c.o.push(vv + pt(2 * n + k));
    }
    Path::from_contours(&[c])
}

/// A styled draw in group space: geometry items, paint and stroke.
struct Draw {
    items: Vec<Item>,
    /// Shapes this style paints (resolved when its group finishes), or `None` once resolved.
    tags: Option<Vec<u32>>,
    xf: Xf,
    paint: Paint,
    opacity: f64,
    rule: FillRule,
    stroke: Option<(Style, Vec<f64>, f64)>,
    /// Gradient geometry in group space (transformed with the draw).
    grad: Option<Gradient>,
}

fn emit_draw(d: Draw, layer: &Xf, tol: f64, out: &mut Scene) {
    let x = layer.mul(&d.xf);
    let paint = match (&d.paint, d.grad) {
        (_, Some(mut g)) => {
            g.to_gradient = g.to_gradient.mul(&x.inverse().unwrap_or(Xf::IDENTITY));
            Paint::Gradient(Box::new(g))
        }
        (p, None) => p.clone(),
    };
    let scale = x.max_scale().max(1e-9);
    for it in d.items {
        let op = d.opacity * it.opacity;
        if op <= 0.0 {
            continue;
        }
        match &d.stroke {
            Some((style, dash, dash_off)) => {
                let mut polys: Vec<Poly> = it.parts.iter().flat_map(|(pa, _)| pa.flatten(tol / scale)).collect();
                if !dash.is_empty() {
                    polys = crate::measure::dash(&polys, dash, *dash_off);
                }
                let outline: Vec<Poly> = stroke::stroke(&polys, style, tol / scale)
                    .into_iter()
                    .map(|q| Poly { pts: q.pts.iter().map(|&pt| x.apply(pt)).collect(), closed: true })
                    .collect();
                if !outline.is_empty() {
                    out.cmds.push(Cmd::Fill {
                        polys: outline,
                        rule: FillRule::NonZero,
                        paint: paint.clone(),
                        opacity: op,
                    });
                }
            }
            None if it.is_compound() => {
                let mut bounds = crate::geom::Rect::EMPTY;
                let parts: Vec<(Vec<Poly>, MaskOp)> =
                    it.parts.iter().map(|(pa, o)| (pa.transform(&x).flatten(tol), *o)).collect();
                for (ps, _) in &parts {
                    bounds = bounds.union(&crate::path::poly_bounds(ps));
                }
                if bounds.is_empty() {
                    continue;
                }
                out.cmds.push(Cmd::Push { mask_init: 0.0 });
                let b = bounds.0;
                out.fill(
                    &shapes::rect(b[0], b[1], b[2] - b[0], b[3] - b[1], [0.0; 4]),
                    FillRule::NonZero,
                    paint.clone(),
                    op,
                    tol,
                );
                for (ps, o) in parts {
                    out.cmds.push(Cmd::Mask { polys: ps, rule: d.rule, op: o, opacity: 1.0, invert: false });
                }
                out.cmds.push(Cmd::Pop { opacity: 1.0 });
            }
            None => {
                let mut path = Path::default();
                for (pa, _) in &it.parts {
                    path.extend(pa);
                }
                out.fill(&path.transform(&x), d.rule, paint.clone(), op, tol);
            }
        }
    }
}

fn color(pr: &Prop, f: f64) -> [f64; 4] {
    let c = pr.at(f);
    let g = |k: usize, d: f64| c.get(k).copied().unwrap_or(d);
    let scale = if c.iter().take(3).any(|v| *v > 1.0) { 255.0 } else { 1.0 };
    [g(0, 0.0) / scale, g(1, 0.0) / scale, g(2, 0.0) / scale, g(3, scale) / scale]
}

fn dash_of(ds: &[(char, Prop)], f: f64) -> (Vec<f64>, f64) {
    let mut pat = Vec::new();
    let mut off = 0.0;
    for (n, pr) in ds {
        match n {
            'o' => off = pr.num(f, 0.0),
            _ => pat.push(pr.num(f, 0.0)),
        }
    }
    (pat, off)
}

fn gradient(g: &Grad, f: f64) -> Gradient {
    let k = g.k.at(f);
    let n = g.count;
    let mut stops: Vec<(f64, [f64; 4])> = (0..n)
        .filter(|i| k.len() >= 4 * (i + 1))
        .map(|i| (k[4 * i], [k[4 * i + 1], k[4 * i + 2], k[4 * i + 3], 1.0]))
        .collect();
    // opacity stops follow the colour stops as (offset, alpha) pairs
    let alphas: Vec<(f64, f64)> =
        k[(4 * n).min(k.len())..].chunks(2).filter(|c| c.len() == 2).map(|c| (c[0], c[1])).collect();
    if !alphas.is_empty() {
        for s in &mut stops {
            let a = match alphas.iter().position(|a| a.0 >= s.0) {
                Some(0) => alphas[0].1,
                Some(j) => {
                    let (a0, a1) = (alphas[j - 1], alphas[j]);
                    a0.1 + (a1.1 - a0.1) * (s.0 - a0.0) / (a1.0 - a0.0).max(1e-9)
                }
                None => alphas.last().unwrap().1,
            };
            s.1[3] = a;
        }
    }
    let s = g.s.at(f);
    let e = g.e.at(f);
    let (a, b) = (
        p(s.first().copied().unwrap_or(0.0), s.get(1).copied().unwrap_or(0.0)),
        p(e.first().copied().unwrap_or(0.0), e.get(1).copied().unwrap_or(0.0)),
    );
    let kind = if g.radial {
        let r = a.dist(b);
        let h = (g.h.num(f, 0.0) / 100.0).clamp(-0.99, 0.99);
        let ang = g.a.num(f, 0.0).to_radians() + (b - a).angle();
        let focal = a + p(libm::cos(ang), libm::sin(ang)) * (r * h);
        GradientKind::Radial { c: a, r, f: focal, fr: 0.0 }
    } else {
        GradientKind::Linear { a, b }
    };
    Gradient { kind, stops, spread: 0, to_gradient: Xf::IDENTITY }
}

/// Evaluates shape items into draws, top-most first.
fn eval_items(items: &[ShapeItem], f: f64, tol: f64) -> Vec<Draw> {
    let mut geo: Vec<Item> = Vec::new();
    // draws recorded per item position (top first)
    let mut draws: Vec<Draw> = Vec::new();
    let mut next_tag = 0u32;
    let tags_now = |geo: &[Item]| -> Option<Vec<u32>> {
        let mut t: Vec<u32> = geo.iter().map(|i| i.tag).collect();
        t.sort_unstable();
        t.dedup();
        Some(t)
    };
    for it in items {
        let ctx = modifiers::Ctx { center: p(0.0, 0.0), time: f, tol };
        match it {
            ShapeItem::Group { items: sub, tr, hidden } => {
                if *hidden {
                    continue;
                }
                let (m, op) = tr.as_ref().map(|t| (t.matrix(f), t.opacity(f))).unwrap_or((Xf::IDENTITY, 1.0));
                let sub_draws = eval_items(sub, f, tol);
                // the group's shapes also feed styles further down this group
                let sub_geo = collect_geo(sub, f, tol);
                for g in sub_geo {
                    geo.push(Item {
                        parts: g.parts.iter().map(|(pa, o)| (pa.transform(&m), *o)).collect(),
                        opacity: g.opacity * op,
                        tag: next_tag,
                    });
                }
                next_tag += 1;
                for mut d in sub_draws {
                    d.xf = m.mul(&d.xf);
                    d.opacity *= op;
                    draws.push(d);
                }
            }
            ShapeItem::Rect { .. } | ShapeItem::Ellipse { .. } | ShapeItem::Star { .. } | ShapeItem::Path { .. } => {
                geo.push(Item { tag: next_tag, ..Item::new(primitive(it, f)) });
                next_tag += 1;
            }
            ShapeItem::Fill { c, o, rule } => {
                let col = color(c, f);
                draws.push(Draw {
                    items: Vec::new(),
                    tags: tags_now(&geo),
                    xf: Xf::IDENTITY,
                    paint: Paint::Solid { rgba: [col[0], col[1], col[2], 1.0], srgb: true },
                    opacity: (o.num(f, 100.0) / 100.0).clamp(0.0, 1.0),
                    rule: *rule,
                    stroke: None,
                    grad: None,
                });
            }
            ShapeItem::GradFill { g, o, rule } => draws.push(Draw {
                items: Vec::new(),
                tags: tags_now(&geo),
                xf: Xf::IDENTITY,
                paint: Paint::Solid { rgba: [0.0; 4], srgb: true },
                opacity: (o.num(f, 100.0) / 100.0).clamp(0.0, 1.0),
                rule: *rule,
                stroke: None,
                grad: Some(gradient(g, f)),
            }),
            ShapeItem::Stroke { o, w, cap, join, ml, dash, .. }
            | ShapeItem::GradStroke { o, w, cap, join, ml, dash, .. } => {
                let (paint, grad) = match it {
                    ShapeItem::Stroke { c, .. } => {
                        let col = color(c, f);
                        (Paint::Solid { rgba: [col[0], col[1], col[2], 1.0], srgb: true }, None)
                    }
                    ShapeItem::GradStroke { g, .. } => {
                        (Paint::Solid { rgba: [0.0; 4], srgb: true }, Some(gradient(g, f)))
                    }
                    _ => unreachable!(),
                };
                let (pat, off) = dash_of(dash, f);
                draws.push(Draw {
                    items: Vec::new(),
                    tags: tags_now(&geo),
                    xf: Xf::IDENTITY,
                    paint,
                    opacity: (o.num(f, 100.0) / 100.0).clamp(0.0, 1.0),
                    rule: FillRule::NonZero,
                    stroke: Some((Style { width: w.num(f, 1.0), cap: *cap, join: *join, miter_limit: *ml }, pat, off)),
                    grad,
                });
            }
            ShapeItem::Trim { s, e, o, sequential } => {
                let (s, e, o) = (s.num(f, 0.0) / 100.0, e.num(f, 100.0) / 100.0, o.num(f, 0.0) / 360.0);
                let mode = if *sequential { TrimMode::Sequential } else { TrimMode::Simultaneous };
                trim_items(&mut geo, s, e, o, mode, tol);
            }
            ShapeItem::Repeater { c, o, below, tr, so, eo } => {
                let s = tr.s.at(f);
                let pos = tr.p.at(f);
                let a = tr.a.at(f);
                let g = |v: &[f64], k: usize, d: f64| v.get(k).copied().unwrap_or(d);
                let center = p(g(&a, 0, 0.0), g(&a, 1, 0.0));
                let m = Modifier::Repeater {
                    copies: c.num(f, 1.0),
                    offset: o.num(f, 0.0),
                    offset_x: g(&pos, 0, 0.0),
                    offset_y: g(&pos, 1, 0.0),
                    rotation: tr.r.num(f, 0.0),
                    scale: g(&s, 0, 100.0) / 100.0,
                    start_opacity: so.num(f, 100.0) / 100.0,
                    end_opacity: eo.num(f, 100.0) / 100.0,
                    below: *below,
                };
                // the styles above see the copies through their shape tags
                modifiers::apply(&mut geo, &m, &modifiers::Ctx { center, ..ctx });
            }
            ShapeItem::RoundCorners { r } => {
                modifiers::apply(&mut geo, &Modifier::RoundCorners { radius: r.num(f, 0.0) }, &ctx)
            }
            ShapeItem::Merge { op } => modifiers::apply(&mut geo, &Modifier::Merge { op: *op }, &ctx),
            ShapeItem::PuckerBloat { a } => {
                for g in &mut geo {
                    let c = centroid(g);
                    let mut one = vec![g.clone()];
                    modifiers::apply(
                        &mut one,
                        &Modifier::PuckerBloat { amount: a.num(f, 0.0) },
                        &modifiers::Ctx { center: c, ..ctx },
                    );
                    *g = one.remove(0);
                }
            }
            ShapeItem::ZigZag { s, r, smooth } => modifiers::apply(
                &mut geo,
                &Modifier::ZigZag { size: s.num(f, 0.0), ridges: r.num(f, 1.0).max(0.0) as u32, smooth: *smooth },
                &ctx,
            ),
            ShapeItem::Twist { a } => modifiers::apply(&mut geo, &Modifier::Twist { amount: a.num(f, 0.0) }, &ctx),
            ShapeItem::Offset { a, join, ml } => modifiers::apply(
                &mut geo,
                &Modifier::OffsetPath { amount: a.num(f, 0.0), join: *join, miter_limit: ml.num(f, 4.0) },
                &ctx,
            ),
            ShapeItem::Skip => {}
        }
    }
    // styles paint their shapes as they are after every operator of the group
    for d in &mut draws {
        if let Some(tags) = d.tags.take() {
            d.items = geo.iter().filter(|g| tags.binary_search(&g.tag).is_ok()).cloned().collect();
        }
    }
    // draws were recorded top-first in item order: the first style is drawn last
    draws
}

fn centroid(it: &Item) -> P {
    let b = crate::path::poly_bounds(&it.parts.iter().flat_map(|(pa, _)| pa.flatten(1.0)).collect::<Vec<_>>());
    if b.is_empty() {
        p(0.0, 0.0)
    } else {
        p((b.0[0] + b.0[2]) * 0.5, (b.0[1] + b.0[3]) * 0.5)
    }
}

fn trim_items(geo: &mut [Item], s: f64, e: f64, o: f64, mode: TrimMode, tol: f64) {
    // express trim as the shared modifier: start is emulated by an offset shift
    let len = (e - s).clamp(0.0, 1.0);
    let m = Modifier::Trim { amount: (1.0 - len) * 100.0, offset: (o + s) * 360.0, mode };
    let mut v = geo.to_vec();
    modifiers::apply(&mut v, &m, &modifiers::Ctx { center: p(0.0, 0.0), time: 0.0, tol });
    geo.clone_from_slice(&v);
}

/// Geometry of a group's shapes (after its modifiers), for styles in enclosing groups.
fn collect_geo(items: &[ShapeItem], f: f64, tol: f64) -> Vec<Item> {
    let mut geo = Vec::new();
    for it in items {
        match it {
            ShapeItem::Group { items: sub, tr, hidden } if !hidden => {
                let m = tr.as_ref().map(|t| t.matrix(f)).unwrap_or(Xf::IDENTITY);
                for g in collect_geo(sub, f, tol) {
                    geo.push(Item {
                        parts: g.parts.iter().map(|(pa, o)| (pa.transform(&m), *o)).collect(),
                        opacity: g.opacity,
                        tag: 0,
                    });
                }
            }
            ShapeItem::Rect { .. } | ShapeItem::Ellipse { .. } | ShapeItem::Star { .. } | ShapeItem::Path { .. } => {
                geo.push(Item::new(primitive(it, f)))
            }
            ShapeItem::Trim { s, e, o, sequential } => {
                let mode = if *sequential { TrimMode::Sequential } else { TrimMode::Simultaneous };
                trim_items(&mut geo, s.num(f, 0.0) / 100.0, e.num(f, 100.0) / 100.0, o.num(f, 0.0) / 360.0, mode, tol);
            }
            ShapeItem::RoundCorners { r } => modifiers::apply(
                &mut geo,
                &Modifier::RoundCorners { radius: r.num(f, 0.0) },
                &modifiers::Ctx { center: p(0.0, 0.0), time: f, tol },
            ),
            _ => {}
        }
    }
    geo
}

fn primitive(it: &ShapeItem, f: f64) -> Path {
    let g = |v: &[f64], k: usize, d: f64| v.get(k).copied().unwrap_or(d);
    match it {
        ShapeItem::Rect { p: pp, s, r } => {
            let (c, sz) = (pp.at(f), s.at(f));
            let (w, h) = (g(&sz, 0, 0.0), g(&sz, 1, 0.0));
            let rr = r.num(f, 0.0).min(w.min(h) * 0.5);
            shapes::rect(g(&c, 0, 0.0) - w * 0.5, g(&c, 1, 0.0) - h * 0.5, w, h, [rr; 4])
        }
        ShapeItem::Ellipse { p: pp, s } => {
            let (c, sz) = (pp.at(f), s.at(f));
            shapes::ellipse_top(g(&c, 0, 0.0), g(&c, 1, 0.0), g(&sz, 0, 0.0) * 0.5, g(&sz, 1, 0.0) * 0.5)
        }
        ShapeItem::Star { star, p: pp, pt, r, or, ir, os, is } => {
            let c = pp.at(f);
            let center = p(g(&c, 0, 0.0), g(&c, 1, 0.0));
            let n = pt.num(f, 5.0).max(1.0) as u32;
            if *star {
                shapes::star(
                    center,
                    n,
                    or.num(f, 0.0),
                    ir.num(f, 0.0),
                    os.num(f, 0.0) / 100.0,
                    is.num(f, 0.0) / 100.0,
                    r.num(f, 0.0),
                )
            } else {
                shapes::polygon(center, n, or.num(f, 0.0), os.num(f, 0.0) / 100.0, r.num(f, 0.0))
            }
        }
        ShapeItem::Path { ks } => bezier_path(ks, f),
        _ => Path::default(),
    }
}
