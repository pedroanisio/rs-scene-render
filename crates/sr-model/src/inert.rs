//! Inert attributes (SREP 18, Specification 5): attributes that are valid and have no effect on the picture.
//!
//! Each is reported as `INERT-<rule>` at [`Severity::Info`](crate::Severity::Info): the document means what it says,
//! only less than its author thought, so strict checks never count these findings. The table is closed; a rule is
//! added only by an SREP.
//!
//! Rules I1 to I7 and I16 (SREP 73) are properties of the document alone and are found here, during validation, on
//! the source elements: an attribute is inert only where the author wrote it. Rule I8 needs the nodes' windows on the
//! composition timeline, which the evaluator computes, and is found there (`sr-eval`).
//!
//! A rule never fires on an attribute whose value can change: one an `animate`, `expression` or `link` child
//! drives, or one an `override` or `bind` targets (variants and parameters can then give it an effect).

use std::collections::{HashMap, HashSet};

use roxmltree::{Document, Node};

use crate::diag::{element_path, Diagnostic, Loc};
use crate::parse::ParseValue;
use crate::values::{Color, Paint};
use crate::xsd::structure::ElementTypes;

/// `INERT-I1`: a stroke paint with no stroke width.
pub const I1: &str = "INERT-I1";
/// `INERT-I2`: a stroke width with no stroke paint.
pub const I2: &str = "INERT-I2";
/// `INERT-I3`: stroke styling on an element that draws no stroke.
pub const I3: &str = "INERT-I3";
/// `INERT-I4`: polygon and star parameters on another shape kind.
pub const I4: &str = "INERT-I4";
/// `INERT-I5`: path data on a shape other than `path`.
pub const I5: &str = "INERT-I5";
/// `INERT-I6`: a matte mode or matte visibility without a matte.
pub const I6: &str = "INERT-I6";
/// `INERT-I7`: audio attributes on a layer whose asset has no sound.
pub const I7: &str = "INERT-I7";
/// `INERT-I8`: a node whose window lies wholly outside the composition, so it is never drawn (found by the
/// evaluator).
pub const I8: &str = "INERT-I8";

/// `INERT-I9` (SREP 34): `group/@collapse="true"`, which has no effect.
pub const I9: &str = "INERT-I9";
/// `INERT-I10` (SREP 34): a selective-color effect's `channel` other than `rgb`, which it does not read.
pub const I10: &str = "INERT-I10";
/// `INERT-I11` (SREP 34): a displacement-map, difference-key or shader `source` with opacity 0 for its whole window.
pub const I11: &str = "INERT-I11";
/// `INERT-I12` (SREP 34): a key's `overshoot` or `period` on a curve that does not read it.
pub const I12: &str = "INERT-I12";
/// `INERT-I13` (SREP 34): an effect attribute its type does not read.
pub const I13: &str = "INERT-I13";
/// `INERT-I16` (SREP 73): `orientToVelocity="false"` on a streak flock, which is drawn along the velocity whatever
/// the value.
pub const I16: &str = "INERT-I16";
/// `MASK-MISS` (SREP 34): a rect or ellipse mask that adds or intersects and lies wholly outside its node's box, so
/// the node shows nothing. A warning, not an inert finding: the mask has the strongest effect possible.
pub const MASK_MISS: &str = "MASK-MISS";

/// The closed table of SREP 18 and its amendment SREP 34: `(code, condition)`.
pub const RULES: &[(&str, &str)] = &[
    (I1, "`stroke` is not transparent and `strokeWidth` is 0 or absent, so no stroke is drawn (information)."),
    (I2, "`strokeWidth` > 0 and `stroke` is transparent or absent, so no stroke is drawn (information)."),
    (I3, "`dash`, `dashOffset`, `strokeCap`, `strokeJoin` or `miterLimit` on an element that draws no stroke (I1, I2) (information)."),
    (I4, "`points`, `innerRadius`, `outerRadius`, `innerRoundness` or `outerRoundness` on a shape other than `polygon` or `star` (information)."),
    (I5, "`path` on a shape other than `path` (information)."),
    (I6, "`matteMode` or `matteVisible` without `matte`, on a node (information)."),
    (I7, "`volume`, `mute` or `audioBus` on a layer whose asset has no sound: image, text, vector, code, formula, chart, generator or audiogram (information)."),
    (I8, "A node whose window lies wholly outside [0, project `duration`), so it is never drawn (information)."),
    (I9, "`group` with `collapse=\"true\"`: the attribute has no effect (non-isolated groups already share the frame's camera space; isolated ones draw their children in their own offscreen) (information, SREP 34)."),
    (I10, "An `effect` of type `selective-color` with `channel` other than `rgb`: the effect does not read `channel` (information, SREP 34)."),
    (I11, "An `effect` of type `displacement-map`, `difference-key` or `shader` whose `source` names a node with opacity 0 for the whole of its window: it contributes nothing; `visible=\"false\"` at opacity 1 keeps a map off screen (information, SREP 34)."),
    (I12, "A `key` with `overshoot` on a segment whose curve is not back-*, or `period` on one whose curve is not elastic-* (information, SREP 34)."),
    (I13, "An `effect` carrying an attribute its `type` does not read (`id`, `type`, `enabled` and `mix` are read by every type) (information, SREP 34)."),
    (I16, "A `flock` with `orientToVelocity=\"false\"` whose `shape` is `streak` (the default): a streak is drawn along the velocity whatever the value (information, SREP 73)."),
    (MASK_MISS, "A `mask` of type rect or ellipse, mode add or intersect, not inverted, whose box lies entirely outside the box of the node it masks, in the node's own coordinates: the node shows nothing (warning, SREP 34; `measured` is how far outside, in pixels)."),
];

/// Elements that draw a stroke from their own `stroke` and `strokeWidth` and carry the shape kinds of
/// [`I4`] and [`I5`].
const SHAPES: [&str; 2] = ["shape", "vector"];

/// Stroke styling that only a drawn stroke reads (I3).
const STROKE_STYLE: [&str; 5] = ["dash", "dashOffset", "strokeCap", "strokeJoin", "miterLimit"];

/// Parameters only polygons and stars read (I4).
const STAR_PARAMS: [&str; 5] = ["points", "innerRadius", "outerRadius", "innerRoundness", "outerRoundness"];

/// Asset kinds that have no sound (I7).
const SILENT_ASSETS: [&str; 8] = ["image", "text", "vector", "code", "formula", "chart", "generator", "audiogram"];

/// Finds I1 to I7 in a parsed document and appends one information diagnostic per inert attribute (I3, I4 and I7
/// once per element, naming every inert attribute of the rule). `types` gives the declarations, and so the
/// defaults, that apply to each element.
pub fn validate(doc: &Document<'_>, types: &ElementTypes, out: &mut Vec<Diagnostic>) {
    let cx = Cx::new(doc, types);
    for n in doc.root_element().descendants().filter(Node::is_element) {
        let name = n.tag_name().name();
        if SHAPES.contains(&name) {
            cx.stroke(n, out);
            cx.shape_kind(n, out);
        }
        if n.has_attribute("matteMode") || n.has_attribute("matteVisible") {
            cx.matte(n, out);
        }
        if name == "layer" {
            cx.audio(n, out);
        }
        if name == "flock" {
            cx.flock_orient(n, out);
        }
    }
}

struct Cx<'a> {
    types: &'a ElementTypes,
    /// `token/@name` → `token/@value`.
    tokens: HashMap<&'a str, &'a str>,
    /// Asset id → asset element name.
    assets: HashMap<&'a str, &'a str>,
    /// `(target id, property)` pairs that an override or bind may change.
    targeted: HashSet<(&'a str, &'a str)>,
}

/// What a paint draws, as far as the document alone says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ink {
    /// Fully transparent, or no paint at all.
    None,
    /// A visible colour or a paint reference.
    Some,
    /// A token that does not resolve to a colour: nothing is claimed.
    Unknown,
}

impl<'a> Cx<'a> {
    fn new<'i: 'a>(doc: &'a Document<'i>, types: &'a ElementTypes) -> Self {
        let root = doc.root_element();
        let mut tokens = HashMap::new();
        let mut assets = HashMap::new();
        let mut targeted = HashSet::new();
        for n in root.descendants().filter(Node::is_element) {
            match n.tag_name().name() {
                "token" => {
                    if let (Some(k), Some(v)) = (n.attribute("name"), n.attribute("value")) {
                        tokens.insert(k, v);
                    }
                }
                "override" | "bind" => {
                    if let (Some(t), Some(p)) = (n.attribute("target"), n.attribute("property")) {
                        targeted.insert((t, p));
                    }
                }
                _ => {}
            }
        }
        if let Some(a) = root.children().find(|c| c.has_tag_name("assets")) {
            for c in a.children().filter(Node::is_element) {
                if let Some(id) = c.attribute("id") {
                    assets.insert(id, c.tag_name().name());
                }
            }
        }
        Cx { types, tokens, assets, targeted }
    }

    /// The value of `attr` on `n`: written, else the default its declaration gives.
    fn value<'n>(&self, n: Node<'n, '_>, attr: &str) -> Option<&'n str> {
        n.attribute(attr).or_else(|| self.types.get(&n.id())?.attr(attr)?.default)
    }

    /// Whether `attr` of `n` can take another value than the one written: animated, linked, computed by an
    /// expression, or the target of an override or bind.
    fn dynamic(&self, n: Node, attr: &str) -> bool {
        let driven = n.children().filter(Node::is_element).any(|c| {
            matches!(c.tag_name().name(), "animate" | "expression" | "link") && c.attribute("property") == Some(attr)
        });
        driven || n.attribute("id").is_some_and(|id| self.targeted.contains(&(id, attr)))
    }

    fn ink(&self, raw: Option<&str>) -> Ink {
        let Some(raw) = raw else { return Ink::None };
        let mut cur = match Paint::parse_value(raw.trim()) {
            Ok(Paint::Ref(_)) => return Ink::Some,
            Ok(Paint::Color(c)) => c,
            Err(_) => return Ink::Unknown,
        };
        // tokens resolve through at most eight levels, as Document::resolve_color does
        for _ in 0..8 {
            match cur {
                Color::Rgba(c) => return if c.a <= 0.0 { Ink::None } else { Ink::Some },
                Color::Token(name) => {
                    let Some(v) = self.tokens.get(name.as_str()) else { return Ink::Unknown };
                    cur = match Color::parse_value(v.trim()) {
                        Ok(c) => c,
                        Err(_) => return Ink::Unknown,
                    };
                }
            }
        }
        Ink::Unknown
    }

    fn width(&self, n: Node) -> Option<f64> {
        match self.value(n, "strokeWidth") {
            None => Some(0.0),
            Some(w) => crate::xsd::parse_xsd_double(w.trim()),
        }
    }

    /// I1, I2 and I3.
    fn stroke(&self, n: Node, out: &mut Vec<Diagnostic>) {
        if self.dynamic(n, "stroke") || self.dynamic(n, "strokeWidth") {
            return;
        }
        let ink = self.ink(self.value(n, "stroke"));
        let Some(width) = self.width(n) else { return };
        if ink == Ink::Unknown {
            return;
        }
        let no_ink = ink == Ink::None;
        let no_width = width <= 0.0;
        if n.has_attribute("stroke") && !no_ink && no_width {
            let stroke = n.attribute("stroke").unwrap_or_default();
            push(out, I1, n, format!("stroke {stroke} with strokeWidth 0 draws no stroke"));
        }
        if n.has_attribute("strokeWidth") && !no_width && no_ink {
            let w = n.attribute("strokeWidth").unwrap_or_default();
            push(out, I2, n, format!("strokeWidth {w} with no stroke paint draws no stroke"));
        }
        if no_ink || no_width {
            let set: Vec<&str> =
                STROKE_STYLE.iter().copied().filter(|a| n.has_attribute(*a) && !self.dynamic(n, a)).collect();
            if !set.is_empty() {
                push(out, I3, n, format!("{} on an element that draws no stroke", attrs(&set)));
            }
        }
    }

    /// I4 and I5.
    fn shape_kind(&self, n: Node, out: &mut Vec<Diagnostic>) {
        if self.dynamic(n, "shape") {
            return;
        }
        let Some(kind) = self.value(n, "shape") else { return };
        if !matches!(kind, "polygon" | "star") {
            let set: Vec<&str> =
                STAR_PARAMS.iter().copied().filter(|a| n.has_attribute(*a) && !self.dynamic(n, a)).collect();
            if !set.is_empty() {
                push(out, I4, n, format!("{} on shape {kind:?}, which only polygon and star read", attrs(&set)));
            }
        }
        if kind != "path" && n.has_attribute("path") && !self.dynamic(n, "path") {
            push(out, I5, n, format!("@path on shape {kind:?}, which only shape \"path\" reads"));
        }
    }

    /// I6.
    fn matte(&self, n: Node, out: &mut Vec<Diagnostic>) {
        if n.tag_name().name() == "transition" || n.has_attribute("matte") || self.dynamic(n, "matte") {
            return;
        }
        let set: Vec<&str> =
            ["matteMode", "matteVisible"].into_iter().filter(|a| n.has_attribute(*a) && !self.dynamic(n, a)).collect();
        if !set.is_empty() {
            push(out, I6, n, format!("{} without @matte", attrs(&set)));
        }
    }

    /// I16 (SREP 73).
    fn flock_orient(&self, n: Node, out: &mut Vec<Diagnostic>) {
        let upright = matches!(n.attribute("orientToVelocity").map(str::trim), Some("false" | "0"));
        if !upright || self.dynamic(n, "orientToVelocity") || self.dynamic(n, "shape") {
            return;
        }
        if self.value(n, "shape").map(str::trim) == Some("streak") {
            push(out, I16, n, "@orientToVelocity false on a streak flock: a streak is drawn along the velocity".into());
        }
    }

    /// I7.
    fn audio(&self, n: Node, out: &mut Vec<Diagnostic>) {
        if self.dynamic(n, "asset") {
            return;
        }
        let Some(kind) = n.attribute("asset").and_then(|a| self.assets.get(a)) else { return };
        if !SILENT_ASSETS.contains(kind) {
            return;
        }
        let set: Vec<&str> =
            ["volume", "mute", "audioBus"].into_iter().filter(|a| n.has_attribute(*a) && !self.dynamic(n, a)).collect();
        if !set.is_empty() {
            push(out, I7, n, format!("{} on a layer of a {kind} asset, which has no sound", attrs(&set)));
        }
    }
}

fn attrs(names: &[&str]) -> String {
    names.iter().map(|a| format!("@{a}")).collect::<Vec<_>>().join(", ")
}

fn push(out: &mut Vec<Diagnostic>, code: &str, n: Node, message: String) {
    out.push(Diagnostic::info(code, message, Loc::of(n), element_path(n)));
}
