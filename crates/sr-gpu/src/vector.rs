//! Vector content of nodes: `shape` nodes, vector and SVG assets, Lottie
//! assets and the deform modifiers of any node, turned into
//! `sr_vector::Scene`s in node-local space.

use std::sync::Arc;

use sr_eval::{FrameGraph, FrameNode, Props, Value};
use sr_model::element::{children, AttrValue, Element};
use sr_vector::arap::{Pin, PinKind, Puppet};
use sr_vector::deform::{Axis, Deformer};
use sr_vector::geom::{p, Xf, P};
use sr_vector::measure::{self, TrimMode};
use sr_vector::modifiers::{self, Item, Modifier};
use sr_vector::scene::{Cmd, FillRule, MaskOp, Paint, Scene};
use sr_vector::stroke::{self, Cap, Join, Style};
use sr_vector::{shapes, Path, Poly};

/// Attribute access: animated value first, then the document attribute.
pub struct Attrs<'a> {
    pub e: &'a dyn Element,
    pub props: Option<&'a Props>,
}

impl Attrs<'_> {
    fn animated(&self, name: &str) -> Option<&Value> {
        self.props.and_then(|p| p.get(name))
    }
    /// A number.
    pub fn num(&self, name: &str, d: f64) -> f64 {
        if let Some(v) = self.animated(name).and_then(Value::as_num) {
            return v;
        }
        match self.e.get_attr(name) {
            Some(AttrValue::Num(v)) => v,
            Some(AttrValue::Length(l)) => l.value,
            Some(AttrValue::Bool(b)) => b as u8 as f64,
            _ => d,
        }
    }
    /// An optional number.
    pub fn opt(&self, name: &str) -> Option<f64> {
        self.animated(name).and_then(Value::as_num).or(match self.e.get_attr(name) {
            Some(AttrValue::Num(v)) => Some(v),
            Some(AttrValue::Length(l)) => Some(l.value),
            _ => None,
        })
    }
    /// A string or enumeration literal.
    pub fn str(&self, name: &str) -> Option<String> {
        // string keys (a shape's path, say) hold between keys, like any other animated value
        if let Some(Value::Str(s)) = self.animated(name) {
            return Some(s.to_string());
        }
        match self.e.get_attr(name) {
            Some(AttrValue::Str(s)) => Some(s),
            Some(other) => Some(other.to_string()),
            None => None,
        }
    }
    /// A number list; keys on it hold or interpolate per element like any other animated value.
    pub fn nums(&self, name: &str) -> Option<Vec<f64>> {
        if let Some(Value::List(l)) = self.animated(name) {
            return Some(l.to_vec());
        }
        match self.e.get_attr(name) {
            Some(AttrValue::Numbers(v)) => Some(v),
            _ => None,
        }
    }
    /// A paint value.
    pub fn paint(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.animated(name) {
            return Some(v.clone());
        }
        match self.e.get_attr(name) {
            Some(AttrValue::Paint(pt)) => Some(crate::value_of_paint_tok(&pt)),
            Some(AttrValue::Color(c)) => Some(crate::value_of_paint_tok(&sr_model::values::Paint::Color(c))),
            _ => None,
        }
    }
}

/// Resolves a paint value over a box in node-local space (the renderer's paint table).
pub type PaintFn<'a> = dyn FnMut(&Value, [f64; 4]) -> Option<Paint> + 'a;

/// Whether an element is `name` (element names of some types report their XSD type name).
pub fn is(e: &dyn Element, name: &str) -> bool {
    let n = e.element_name();
    n == name || n.strip_suffix("Type") == Some(name)
}

pub(crate) fn part<'g>(n: &'g FrameNode, key: &str) -> Option<&'g Props> {
    n.parts.iter().find(|s| &*s.key == key).map(|s| &s.props)
}

fn rule(s: Option<String>, d: FillRule) -> FillRule {
    match s.as_deref() {
        Some("evenodd") => FillRule::EvenOdd,
        Some("nonzero") => FillRule::NonZero,
        _ => d,
    }
}

/// Base outline of a primitive in its `w`×`h` box.
pub fn primitive(a: &Attrs, kind: &str, w: f64, h: f64) -> Result<Path, String> {
    let c = p(w * 0.5, h * 0.5);
    // polygon and star vertices lie on the ellipse inscribed in the box, or on
    // a circle of `outerRadius` units; a star's inner vertices on a circle of `innerRadius` units, or
    // on the outer figure scaled by 0.5
    let outer = a.opt("outerRadius").map(|r| p(r, r)).unwrap_or(c);
    Ok(match kind {
        "rect" | "rounded-rect" => {
            let r = a.num("radius", 0.0);
            let radii =
                a.nums("cornerRadii").filter(|v| v.len() == 4).map(|v| [v[0], v[1], v[2], v[3]]).unwrap_or([r; 4]);
            shapes::rect(0.0, 0.0, w, h, radii)
        }
        "ellipse" => shapes::ellipse(c.x, c.y, w * 0.5, h * 0.5),
        "polygon" => {
            shapes::polygon_on(c, a.num("points", 5.0).max(3.0) as u32, outer, a.num("outerRoundness", 0.0), 0.0)
        }
        "star" => shapes::star_on(
            c,
            a.num("points", 5.0).max(2.0) as u32,
            outer,
            a.opt("innerRadius").map(|r| p(r, r)).unwrap_or(outer * 0.5),
            a.num("outerRoundness", 0.0),
            a.num("innerRoundness", 0.0),
            0.0,
        ),
        "line" => shapes::line(w, h),
        "path" => match a.str("path") {
            Some(d) => Path::parse(&d).map_err(|e| e.to_string())?,
            None => return Err("shape=\"path\" needs @path".into()),
        },
        other => return Err(format!("shape {other:?} has no outline here")),
    })
}

/// How far (node units) a shape's ink can reach outside its box, when it stays near the box at all: a
/// rect or ellipse without modifiers or deformers, whose only overhang is its stroke (a 90° miter at most)
pub fn box_overhang(n: &FrameNode) -> Option<f64> {
    let e: &dyn Element = &*n.elem;
    let a = Attrs { e, props: Some(&n.props) };
    if !matches!(a.str("shape").as_deref().unwrap_or("rect"), "rect" | "rounded-rect" | "ellipse") {
        return None;
    }
    if children(e).into_iter().any(|c| is(c, "shapeModifier") || is(c, "deform")) {
        return None;
    }
    let sw = if a.paint("stroke").is_some() { a.num("strokeWidth", 0.0).max(0.0) } else { 0.0 };
    let side = match a.str("strokePosition").as_deref() {
        Some("inside") => 0.0,
        Some("outside") => 1.0,
        _ => 0.5,
    };
    Some(sw * side * std::f64::consts::SQRT_2)
}

fn modifier_of(a: &Attrs) -> Option<Modifier> {
    let mode = a.str("mode");
    Some(match a.str("type")?.as_str() {
        "repeater" => Modifier::Repeater {
            copies: a.num("copies", 3.0),
            offset: a.num("offset", 0.0),
            offset_x: a.num("offsetX", 0.0),
            offset_y: a.num("offsetY", 0.0),
            rotation: a.num("rotation", 0.0),
            scale: a.num("scale", 1.0),
            start_opacity: a.num("startOpacity", 1.0),
            end_opacity: a.num("endOpacity", 1.0),
            below: a.str("composite").as_deref() == Some("below"),
        },
        "offset-path" => Modifier::OffsetPath {
            amount: a.num("amount", 0.0),
            join: match mode.as_deref() {
                Some("round") => Join::Round,
                Some("bevel") => Join::Bevel,
                _ => Join::Miter,
            },
            miter_limit: 4.0,
        },
        "pucker-bloat" => Modifier::PuckerBloat { amount: a.num("amount", 0.0) },
        "zig-zag" => Modifier::ZigZag {
            size: a.num("size", 10.0),
            ridges: a.num("ridges", 5.0) as u32,
            smooth: mode.as_deref() == Some("smooth"),
        },
        "twist" => Modifier::Twist { amount: a.num("amount", 0.0) },
        "round-corners" => Modifier::RoundCorners { radius: a.num("amount", 0.0) },
        "wiggle-path" => Modifier::WigglePath {
            size: a.num("size", 10.0),
            detail: a.num("detail", 10.0),
            frequency: a.num("frequency", 2.0),
            seed: a.num("seed", 0.0) as u64,
        },
        "merge" => Modifier::Merge {
            op: match mode.as_deref() {
                Some("subtract") => MaskOp::Subtract,
                Some("intersect") => MaskOp::Intersect,
                Some("exclude") => MaskOp::Difference,
                _ => MaskOp::Add,
            },
        },
        "trim" => Modifier::Trim {
            amount: a.num("amount", 0.0),
            offset: a.num("offset", 0.0),
            mode: if mode.as_deref() == Some("simultaneous") { TrimMode::Simultaneous } else { TrimMode::Sequential },
        },
        _ => return None,
    })
}

/// Builds a shape node's scene in node-local space; `tol` is the local flattening tolerance.
pub fn shape_scene(n: &FrameNode, paint: &mut PaintFn, tol: f64) -> Result<Scene, String> {
    let e: &dyn Element = &*n.elem;
    let a = Attrs { e, props: Some(&n.props) };
    let [w, h] = n.size.unwrap_or([a.num("width", 0.0), a.num("height", 0.0)]);
    let kind = a.str("shape").unwrap_or_else(|| "rect".into());
    let base = primitive(&a, &kind, w, h)?;
    let mut items = vec![Item::new(base)];
    let ctx = modifiers::Ctx { center: p(w * 0.5, h * 0.5), time: n.local_time, tol };
    let mut k = 0;
    for c in children(e) {
        if !is(c, "shapeModifier") {
            continue;
        }
        let key = format!("{}/{}[{k}]", n.id, c.element_name());
        k += 1;
        if let Some(m) = modifier_of(&Attrs { e: c, props: part(n, &key) }) {
            modifiers::apply(&mut items, &m, &ctx);
        }
    }
    // trim paths on the final outline
    let (ts, te, to) = (a.num("trimStart", 0.0), a.num("trimEnd", 1.0), a.num("trimOffset", 0.0));
    let trim_mode =
        if a.str("trimMode").as_deref() == Some("sequential") { TrimMode::Sequential } else { TrimMode::Simultaneous };
    let trimmed = ts > 0.0 || te < 1.0;
    let rule_ = rule(a.str("fillRule"), FillRule::NonZero);
    let box_rect = [0.0, 0.0, w, h];
    let fill = a.paint("fill").and_then(|v| paint(&v, box_rect));
    let stroke_paint = a.paint("stroke").and_then(|v| paint(&v, box_rect));
    let sw = a.num("strokeWidth", 0.0);
    let style = Style {
        width: sw,
        cap: match a.str("strokeCap").as_deref() {
            Some("round") => Cap::Round,
            Some("square") => Cap::Square,
            _ => Cap::Butt,
        },
        join: match a.str("strokeJoin").as_deref() {
            Some("round") => Join::Round,
            Some("bevel") => Join::Bevel,
            _ => Join::Miter,
        },
        miter_limit: a.num("miterLimit", 4.0),
    };
    let dash = a.nums("dash").unwrap_or_default();
    let dash_off = a.num("dashOffset", 0.0);
    let position = a.str("strokePosition").unwrap_or_else(|| "center".into());
    let stroke_first = a.str("paintOrder").as_deref() == Some("stroke-fill");
    let mut scene = Scene::default();
    // per item: fill and stroke geometry
    let mut per_item: Vec<(Item, Vec<Poly>, Vec<Poly>)> = Vec::new();
    let all_polys: Vec<Vec<Poly>> =
        items.iter().map(|it| it.parts.iter().flat_map(|(pa, _)| pa.flatten(tol)).collect()).collect();
    let trimmed_all = if trimmed && trim_mode == TrimMode::Sequential {
        let flat: Vec<Poly> = all_polys.iter().flatten().cloned().collect();
        Some(measure::trim(&flat, ts, te, to / 360.0, TrimMode::Sequential))
    } else {
        None
    };
    for (idx, (it, polys)) in items.into_iter().zip(all_polys).enumerate() {
        let outline = if !trimmed {
            polys.clone()
        } else if let Some(all) = &trimmed_all {
            if idx == 0 {
                all.clone()
            } else {
                Vec::new()
            }
        } else {
            measure::trim(&polys, ts, te, to / 360.0, TrimMode::Simultaneous)
        };
        let fill_polys = if trimmed { outline.clone() } else { polys };
        per_item.push((it, fill_polys, outline));
    }
    let draw_fill = |scene: &mut Scene, it: &Item, fp: &[Poly]| {
        let Some(fp_paint) = &fill else { return };
        if it.is_compound() && !trimmed {
            let b = sr_vector::path::poly_bounds(fp).0;
            if b[0] > b[2] {
                return;
            }
            scene.cmds.push(Cmd::Push { mask_init: 0.0 });
            scene.fill(
                &shapes::rect(b[0], b[1], b[2] - b[0], b[3] - b[1], [0.0; 4]),
                FillRule::NonZero,
                fp_paint.clone(),
                it.opacity,
                tol,
            );
            for (pa, op) in &it.parts {
                scene.cmds.push(Cmd::Mask {
                    polys: pa.flatten(tol),
                    rule: rule_,
                    op: *op,
                    opacity: 1.0,
                    invert: false,
                });
            }
            scene.cmds.push(Cmd::Pop { opacity: 1.0 });
        } else if !fp.is_empty() {
            scene.cmds.push(Cmd::Fill {
                polys: fp.to_vec(),
                rule: rule_,
                paint: fp_paint.clone(),
                opacity: it.opacity,
            });
        }
    };
    let draw_stroke = |scene: &mut Scene, it: &Item, fp: &[Poly], outline: &[Poly]| {
        let Some(sp) = &stroke_paint else { return };
        if sw <= 0.0 || outline.is_empty() {
            return;
        }
        let dashed =
            if dash.iter().any(|v| *v > 0.0) { measure::dash(outline, &dash, dash_off) } else { outline.to_vec() };
        let clip = position != "center";
        let st = Style { width: if clip { sw * 2.0 } else { sw }, ..style };
        let polys = stroke::stroke(&dashed, &st, tol);
        if polys.is_empty() {
            return;
        }
        if clip {
            scene.cmds.push(Cmd::Push { mask_init: 0.0 });
            scene.cmds.push(Cmd::Fill { polys, rule: FillRule::NonZero, paint: sp.clone(), opacity: it.opacity });
            scene.cmds.push(Cmd::Mask {
                polys: fp.to_vec(),
                rule: rule_,
                op: MaskOp::Add,
                opacity: 1.0,
                invert: position == "outside",
            });
            scene.cmds.push(Cmd::Pop { opacity: 1.0 });
        } else {
            scene.cmds.push(Cmd::Fill { polys, rule: FillRule::NonZero, paint: sp.clone(), opacity: it.opacity });
        }
    };
    for (it, fp, outline) in &per_item {
        if stroke_first {
            draw_stroke(&mut scene, it, fp, outline);
            draw_fill(&mut scene, it, fp);
        } else {
            draw_fill(&mut scene, it, fp);
            draw_stroke(&mut scene, it, fp, outline);
        }
    }
    Ok(scene)
}

/// Vector asset (primitive or SVG outline attributes) scene in its `w`×`h` space.
pub fn vector_asset_scene(a: &Attrs, paint: &mut PaintFn, tol: f64) -> Result<Scene, String> {
    let (w, h) = (a.num("width", 1.0), a.num("height", 1.0));
    let kind = a.str("shape").unwrap_or_else(|| "rect".into());
    let path = primitive(a, &kind, w, h)?;
    let box_rect = [0.0, 0.0, w, h];
    let mut s = Scene::default();
    if let Some(f) = a.paint("fill").and_then(|v| paint(&v, box_rect)) {
        s.fill(&path, rule(a.str("fillRule"), FillRule::EvenOdd), f, 1.0, tol);
    }
    let sw = a.num("strokeWidth", 0.0);
    if let (Some(sp), true) = (a.paint("stroke").and_then(|v| paint(&v, box_rect)), sw > 0.0) {
        let style = Style {
            width: sw,
            cap: match a.str("strokeCap").as_deref() {
                Some("round") => Cap::Round,
                Some("square") => Cap::Square,
                _ => Cap::Butt,
            },
            join: match a.str("strokeJoin").as_deref() {
                Some("round") => Join::Round,
                Some("bevel") => Join::Bevel,
                _ => Join::Miter,
            },
            miter_limit: a.num("miterLimit", 4.0),
        };
        let mut polys = path.flatten(tol);
        if let Some(d) = a.nums("dash") {
            polys = measure::dash(&polys, &d, a.num("dashOffset", 0.0));
        }
        let out = stroke::stroke(&polys, &style, tol);
        if !out.is_empty() {
            s.cmds.push(Cmd::Fill { polys: out, rule: FillRule::NonZero, paint: sp, opacity: 1.0 });
        }
    }
    Ok(s)
}

/// Deformers of a node for this frame, over its `w`×`h` box.
pub fn deformers(n: &FrameNode, g: &FrameGraph, size: [f64; 2]) -> Vec<Deformer> {
    let e: &dyn Element = &*n.elem;
    let mut out = Vec::new();
    let [w, h] = size;
    let mut di = 0;
    for d in children(e) {
        if !is(d, "deform") {
            continue;
        }
        let dkey = format!("{}/{}[{di}]", n.id, d.element_name());
        di += 1;
        let mut mi = 0;
        for m in children(d) {
            if !is(m, "modifier") {
                continue;
            }
            let mkey = format!("{dkey}/{}[{mi}]", m.element_name());
            mi += 1;
            let a = Attrs { e: m, props: part(n, &mkey) };
            let center = p(a.opt("centerX").unwrap_or(w * 0.5), a.opt("centerY").unwrap_or(h * 0.5));
            let radius = a.opt("radius").unwrap_or(w.min(h) * 0.5);
            let axis = if a.str("axis").as_deref() == Some("x") { Axis::X } else { Axis::Y };
            let amount = a.num("amount", 0.0);
            let (frequency, phase) = (a.num("frequency", 1.0), a.num("phase", 0.0));
            let kids = children(m);
            let sub = |name: &str| -> Vec<(Attrs, usize)> {
                let mut k = 0;
                kids.iter()
                    .filter(|c| is(**c, name))
                    .map(|c| {
                        let key = format!("{mkey}/{}[{k}]", c.element_name());
                        k += 1;
                        (Attrs { e: *c, props: part(n, &key) }, k - 1)
                    })
                    .collect()
            };
            let def = match a.str("type").as_deref().unwrap_or("") {
                "bend" => Deformer::Bend {
                    amount,
                    axis: if axis == Axis::Y { Axis::X } else { Axis::Y },
                    center,
                    extent: if axis == Axis::Y { w } else { h },
                },
                "twist" => Deformer::Twist { amount, center, radius },
                "wave" => Deformer::Wave { amount, frequency, phase, axis, size: p(w, h) },
                "squash" => Deformer::Squash { amount, axis, center },
                "stretch" => Deformer::Squash { amount: -amount, axis, center },
                "mesh-warp" => {
                    let (rows, cols) = (a.num("rows", 4.0).max(2.0) as usize, a.num("cols", 4.0).max(2.0) as usize);
                    let mut offsets = vec![p(0.0, 0.0); rows * cols];
                    for (pt, _) in sub("point") {
                        let (r, c) = (pt.num("row", 0.0) as usize, pt.num("col", 0.0) as usize);
                        if r < rows && c < cols {
                            offsets[r * cols + c] = p(pt.num("x", 0.0), pt.num("y", 0.0));
                        }
                    }
                    Deformer::MeshWarp { rows, cols, size: p(w, h), offsets }
                }
                "puppet" => {
                    let pins: Vec<Pin> = sub("pin")
                        .into_iter()
                        .map(|(pa, _)| Pin {
                            kind: match pa.str("kind").as_deref() {
                                Some("bend") => PinKind::Bend,
                                Some("starch") => PinKind::Starch,
                                _ => PinKind::Position,
                            },
                            rest: p(pa.num("restX", 0.0), pa.num("restY", 0.0)),
                            offset: p(pa.num("x", 0.0), pa.num("y", 0.0)),
                            rotation: pa.num("rotation", 0.0),
                            amount: pa.num("amount", 1.0),
                        })
                        .collect();
                    let cells = 16.0;
                    let (cols, rows) = if w >= h {
                        (cells, (cells * h / w.max(1e-9)).ceil().max(2.0))
                    } else {
                        ((cells * w / h.max(1e-9)).ceil().max(2.0), cells)
                    };
                    Deformer::Puppet(Box::new(Puppet::solve([0.0, 0.0, w, h], cols as usize, rows as usize, &pins)))
                }
                "skin" => {
                    let Some(sk) =
                        a.str("skeleton").and_then(|id| g.nodes.iter().find(|x| x.kind == "skeleton" && *x.id == *id))
                    else {
                        continue;
                    };
                    let rest: Vec<Xf> = sk.bones.iter().map(|b| Xf(b.rest.0)).collect();
                    let now: Vec<Xf> = sk.bones.iter().map(|b| Xf(b.world.0)).collect();
                    let lens: Vec<f64> = sk.bones.iter().map(|b| b.length).collect();
                    let skw = Xf(sk.world.0);
                    let samples = sk
                        .skin
                        .as_ref()
                        .map(|s| s.0.iter().map(|(q, ws)| (skw.apply(p(q[0], q[1])), ws.clone())).collect())
                        .unwrap_or_default();
                    Deformer::Skin(Box::new(sr_vector::rig::Skin::new(&rest, &now, &lens, Xf(n.world.0), samples)))
                }
                "bulge" => Deformer::Bulge { amount, center, radius },
                "pinch" => Deformer::Bulge { amount: -amount, center, radius },
                "spherize" => Deformer::Spherize { amount, center, radius },
                "ripple" => {
                    Deformer::Ripple { amount, frequency, phase, center, radius: a.opt("radius").unwrap_or(0.0) }
                }
                "turbulence" => Deformer::Turbulence { amount, frequency, phase, seed: a.num("seed", 0.0) as u64 },
                "corner-pin" => {
                    let c = a
                        .nums("corners")
                        .filter(|v| v.len() == 8)
                        .unwrap_or_else(|| vec![0.0, 0.0, w, 0.0, w, h, 0.0, h]);
                    match sr_vector::deform::corner_pin(
                        w,
                        h,
                        [p(c[0], c[1]), p(c[2], c[3]), p(c[4], c[5]), p(c[6], c[7])],
                    ) {
                        Some(hm) => Deformer::CornerPin { h: hm },
                        None => continue,
                    }
                }
                _ => continue,
            };
            out.push(def);
        }
    }
    // a soft body's simulated lattice
    if let Some(sw) = &n.soft {
        out.push(Deformer::MeshWarp {
            rows: sw.rows,
            cols: sw.cols,
            size: p(sw.size[0], sw.size[1]),
            offsets: sw.offsets.iter().map(|o| p(o[0], o[1])).collect(),
        });
    }
    out
}

/// Applies deformers to a local-space scene.
pub fn deform_scene(s: &mut Scene, ds: &[Deformer], tol: f64) {
    if ds.is_empty() {
        return;
    }
    // subdivide so straight edges bend: a few pixels on screen
    s.subdivide((tol * 20.0).max(0.5));
    s.map_points(&|q: P| sr_vector::deform::apply_all(ds, q));
}

/// Parsed Lottie animations, shared between frames.
pub type LottieCache = std::collections::HashMap<String, Result<Arc<sr_vector::lottie::Lottie>, String>>;
