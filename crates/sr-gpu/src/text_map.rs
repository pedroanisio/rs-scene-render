//! `<map>` assets: geographic data projected and drawn as vectors (geo layers
//! with choropleth colouring and per-feature styles, graticules, great-circle
//! routes drawn on by ground distance, pins with labels), under an animatable
//! camera with smooth fly-to moves.

use std::collections::HashMap;

use sr_geo::data::{Feature, Geometry};
use sr_geo::project::Planar;
use sr_geo::sphere;
use sr_model::element::{children, Element};
use sr_model::model as m;
use sr_text::layout::Align;
use sr_text::{chart, Style};
use sr_vector::geom::p;
use sr_vector::measure::{self, TrimMode};
use sr_vector::scene::{Cmd, FillRule};
use sr_vector::stroke::{self, Cap, Join, Style as StrokeStyle};
use sr_vector::{shapes, Paint, Poly};

use super::{style_for, Cx, Drawing, TextCache};
use crate::vector::{is, Attrs};
use sr_eval::Value;

/// Evaluated attributes of the element with key `key` (its id, or its path under the map).
fn attrs<'a>(g: &'a sr_eval::FrameGraph, e: &'a dyn Element, key: &str) -> Attrs<'a> {
    Attrs { e, props: g.elements.iter().find(|s| *s.key == *key).map(|s| &s.props) }
}

/// The children of `e` with the keys the evaluator gives them (id, else `parent/name[k]`).
fn keyed<'a>(e: &'a dyn Element, parent: &str) -> Vec<(&'a dyn Element, String)> {
    const ANIM: [&str; 4] = ["animate", "expression", "motionPath", "link"];
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut out = Vec::new();
    for c in children(e) {
        let name = c.element_name();
        if ANIM.contains(&name) {
            continue;
        }
        let k = counts.entry(name).or_default();
        let key = c.element_id().map(str::to_string).unwrap_or_else(|| format!("{parent}/{name}[{k}]"));
        *k += 1;
        out.push((c, key));
    }
    out
}

/// Attribution text without HTML markup (tile services often give links).
fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in s.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            c if !inside => out.push(c),
            _ => {}
        }
    }
    out.replace("&copy;", "©").replace("&amp;", "&")
}

/// Parses "lon,lat lon,lat …".
fn lon_lats(s: &str) -> Vec<[f64; 2]> {
    s.split_whitespace()
        .filter_map(|pair| {
            let mut it = pair.split(',').map(|v| v.trim().parse::<f64>());
            match (it.next(), it.next()) {
                (Some(Ok(x)), Some(Ok(y))) => Some([x, y]),
                _ => None,
            }
        })
        .collect()
}

/// `prop=value` and `prop!=value` conditions separated by `;`, all of which must hold.
fn keep(f: &Feature, filter: Option<&str>) -> bool {
    let Some(filter) = filter else { return true };
    filter.split(';').map(str::trim).filter(|c| !c.is_empty()).all(|c| {
        let (prop, want, negate) = match c.split_once("!=") {
            Some((p, v)) => (p.trim(), v.trim(), true),
            None => match c.split_once('=') {
                Some((p, v)) => (p.trim(), v.trim(), false),
                None => return true,
            },
        };
        let equal = match (f.number(prop), want.parse::<f64>()) {
            (Some(a), Ok(b)) => a == b,
            _ => f.text(prop).as_deref() == Some(want),
        };
        equal != negate
    })
}

fn polys_of_rings(rings: &[Vec<[f64; 2]>], closed: bool) -> Vec<Poly> {
    rings
        .iter()
        .filter(|r| r.len() > 1)
        .map(|r| Poly { pts: r.iter().map(|q| p(q[0], q[1])).collect(), closed })
        .collect()
}

pub(super) struct Painter<'d> {
    pub(super) d: &'d mut Drawing,
    tol: f64,
}

impl Painter<'_> {
    pub(super) fn fill(&mut self, polys: Vec<Poly>, paint: &Option<Paint>, opacity: f64) {
        if let (Some(paint), false) = (paint, polys.is_empty()) {
            if opacity > 0.0 {
                self.d.scene.cmds.push(Cmd::Fill { polys, rule: FillRule::NonZero, paint: paint.clone(), opacity });
            }
        }
    }

    pub(super) fn stroke(&mut self, polys: &[Poly], paint: &Option<Paint>, width: f64, opacity: f64) {
        if width <= 0.0 || polys.is_empty() {
            return;
        }
        let style = StrokeStyle { width, cap: Cap::Round, join: Join::Round, miter_limit: 4.0 };
        let outline = stroke::stroke(polys, &style, self.tol);
        self.fill(outline, paint, opacity);
    }

    pub(super) fn dot(
        &mut self,
        c: [f64; 2],
        r: f64,
        fill: &Option<Paint>,
        stroke: &Option<Paint>,
        sw: f64,
        opacity: f64,
    ) {
        if r <= 0.0 {
            return;
        }
        let polys = shapes::ellipse(c[0], c[1], r, r).flatten(self.tol);
        self.fill(polys.clone(), fill, opacity);
        self.stroke(&polys, stroke, sw, opacity);
    }
}

/// A choropleth colour scale.
struct Scale {
    palette: Vec<[f64; 4]>,
    domain: Vec<f64>,
    kind: String,
}

impl Scale {
    fn transform(&self, v: f64) -> Option<f64> {
        match self.kind.as_str() {
            "log" => (v > 0.0).then(|| v.ln()),
            "sqrt" => (v >= 0.0).then(|| v.sqrt()),
            _ => Some(v),
        }
    }

    fn color(&self, v: f64) -> Option<[f64; 4]> {
        let n = self.palette.len();
        if n == 0 {
            return None;
        }
        let d: Vec<f64> = self.domain.iter().map(|&x| self.transform(x)).collect::<Option<_>>()?;
        let v = self.transform(v)?;
        let lerp = |a: [f64; 4], b: [f64; 4], t: f64| [0, 1, 2, 3].map(|k| a[k] + (b[k] - a[k]) * t);
        if d.len() == n && n > 2 {
            // Piecewise: one domain stop per colour.
            let i = d.windows(2).position(|w| v <= w[1]).unwrap_or(n - 2);
            let t = ((v - d[i]) / (d[i + 1] - d[i])).clamp(0.0, 1.0);
            return Some(lerp(self.palette[i], self.palette[i + 1], if t.is_finite() { t } else { 0.0 }));
        }
        let (lo, hi) = (d[0], *d.last().unwrap_or(&d[0]));
        let t = if hi > lo { ((v - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
        if self.kind == "quantize" {
            return Some(self.palette[((t * n as f64) as usize).min(n - 1)]);
        }
        if n == 1 {
            return Some(self.palette[0]);
        }
        let x = t * (n - 1) as f64;
        let i = (x.floor() as usize).min(n - 2);
        Some(lerp(self.palette[i], self.palette[i + 1], x - i as f64))
    }
}

/// Graticule lines every `step` degrees (d3's `geoGraticule` layout: minor meridians stop at
/// ±80°, those on multiples of 90° reach the poles; parallels up to ±80°).
fn graticule(step: f64) -> Geometry {
    let step = step.max(0.1);
    let mut lines = Vec::new();
    let n = (360.0 / step).round() as i64;
    for i in 0..n {
        let lon = -180.0 + i as f64 * step;
        let major = (lon / 90.0).fract().abs() < 1e-9;
        let lim = if major { 90.0 } else { 80.0 };
        let k = ((2.0 * lim) / 2.5f64).ceil() as usize;
        lines.push((0..=k).map(|j| [lon, -lim + 2.0 * lim * j as f64 / k as f64]).collect());
    }
    let mut lat = -80.0f64.div_euclid(step) * step;
    while lat <= 80.0 + 1e-9 {
        if lat >= -80.0 - 1e-9 {
            lines.push((0..=144).map(|j| [-180.0 + j as f64 * 2.5, lat]).collect());
        }
        lat += step;
    }
    Geometry::Lines(lines)
}

/// The first `fraction` of a great-circle polyline, by ground distance.
fn ground_trim(pts: &[[f64; 2]], fraction: f64) -> Vec<[f64; 2]> {
    let r: Vec<[f64; 2]> = pts.iter().map(|q| [q[0] * sphere::RAD, q[1] * sphere::RAD]).collect();
    let lens: Vec<f64> = r.windows(2).map(|w| sphere::distance(w[0], w[1])).collect();
    let total: f64 = lens.iter().sum();
    let mut left = total * fraction.clamp(0.0, 1.0);
    let mut out = vec![pts[0]];
    for (i, &l) in lens.iter().enumerate() {
        if left >= l {
            out.push(pts[i + 1]);
            left -= l;
            continue;
        }
        if l > 0.0 && left > 0.0 {
            let q = sphere::interpolate(r[i], r[i + 1], left / l);
            out.push([q[0] * sphere::DEG, q[1] * sphere::DEG]);
        }
        break;
    }
    out
}

/// Area-weighted centroid of the largest ring of a projected feature.
fn label_anchor(pl: &Planar) -> Option<([f64; 2], f64)> {
    let mut best: Option<([f64; 2], f64)> = None;
    for ring in pl.polygons.iter().filter_map(|poly| poly.first()) {
        let (mut a, mut cx, mut cy) = (0.0, 0.0, 0.0);
        for (i, q) in ring.iter().enumerate() {
            let r = ring[(i + 1) % ring.len()];
            let c = q[0] * r[1] - r[0] * q[1];
            a += c;
            cx += (q[0] + r[0]) * c;
            cy += (q[1] + r[1]) * c;
        }
        if a.abs() > best.map_or(0.0, |b| b.1) {
            best = Some(([cx / (3.0 * a), cy / (3.0 * a)], a.abs() / 2.0));
        }
    }
    best
}

/// Draws a map asset in its width × height box.
pub fn map_drawing(tc: &mut TextCache, cx: &mut Cx, mp: &m::MapAsset) -> Result<Drawing, String> {
    map_drawing_as(tc, cx, mp, None)
}

/// Draws a map asset, in its own frame and camera, or in `frame` (a map setup and view that
/// replace them: the whole world for a globe's texture).
pub fn map_drawing_as(
    tc: &mut TextCache,
    cx: &mut Cx,
    mp: &m::MapAsset,
    frame: Option<(sr_geo::view::Map, sr_geo::view::View)>,
) -> Result<Drawing, String> {
    let key = mp.id.as_str();
    let g = cx.g;
    let (w, h) = match &frame {
        Some((m, _)) => (m.size[0], m.size[1]),
        None => (mp.width as f64, mp.height as f64),
    };
    let bx = [0.0, 0.0, w, h];
    let at = attrs(g, mp, key);
    let proj = match &frame {
        Some((m, v)) => m.projection(v),
        None => {
            let cam = sr_eval::geo::camera(cx.p, mp)?;
            let view = sr_eval::geo::view(
                &cam,
                mp,
                &|name| at.opt(name).filter(|_| at.props.is_some_and(|p| p.get(name).is_some())),
                cx.g.time,
            );
            cam.map.projection(&view)
        }
    };

    let mut d = Drawing::default();
    let tol = cx.tol;
    let fonts = std::mem::take(&mut tc.font_assets);
    let label_style = |cx: &mut Cx, id: Option<&str>| {
        let base = Style {
            size: (h / 40.0).clamp(10.0, 24.0),
            color: Some(Paint::Solid { rgba: [1.0; 4], srgb: false }),
            ..Default::default()
        };
        style_for(cx.p, id, base, &cx.base, &mut *cx.paint, cx.tokens, &fonts, bx)
    };
    let mut labels: Vec<(String, Style, [f64; 2], Align)> = Vec::new();
    let paint = |cx: &mut Cx, v: Option<Value>| v.and_then(|v| (cx.paint)(&v, bx));

    let mut raster: Vec<sr_text::Bitmap> = Vec::new();
    let mut credits: Vec<String> = Vec::new();
    {
        let mut pt = Painter { d: &mut d, tol };
        let sphere_outline = proj.project(&Geometry::Sphere);
        let bg = paint(cx, at.paint("background"));
        pt.fill(sphere_outline.polygons.iter().flat_map(|p| polys_of_rings(p, true)).collect(), &bg, 1.0);

        for (c, ckey) in keyed(mp, key) {
            let ca = attrs(g, c, &ckey);
            let opacity = ca.num("opacity", 1.0);
            if is(c, "basemap") {
                let lib = tc.lib();
                let out = super::basemap::draw(cx, lib, &mut pt, &proj, [w, h], &ca, tol)?;
                for k in &out.skipped {
                    let msg = format!("{}: basemap style layers of type {k} are not drawn", mp.id);
                    if !cx.unsupported.contains(&msg) {
                        cx.unsupported.push(msg);
                    }
                }
                raster.extend(out.bitmaps);
                if ca.str("attribution").as_deref() != Some("false") {
                    credits.extend(out.attribution.filter(|a| !credits.contains(a)));
                }
            } else if is(c, "geoLayer") {
                let geo = ca.str("geo").unwrap_or_default();
                let fs = sr_eval::geo::features(cx.p, &geo)?;
                let fill = paint(cx, ca.paint("fill"));
                let stroke_paint = paint(cx, ca.paint("stroke"));
                let sw = ca.num("strokeWidth", 0.5);
                let filter = ca.str("filter");
                let key_by = ca.str("keyBy").unwrap_or_else(|| "id".into());
                let progress = ca.num("progress", 1.0);
                let point_r = ca.num("pointRadius", 3.0);
                // choropleth
                let fill_by = ca.str("fillBy");
                let no_data = paint(cx, ca.paint("noData"));
                let scale = fill_by.as_ref().map(|prop| {
                    let palette: Vec<[f64; 4]> = ca
                        .str("palette")
                        .unwrap_or_else(|| "#F7FBFF #08306B".into())
                        .split_whitespace()
                        .filter_map(|s| {
                            match <sr_model::values::Color as sr_model::parse::ParseValue>::parse_value(s) {
                                Ok(sr_model::values::Color::Rgba(c)) => {
                                    Some([c.r as f64, c.g as f64, c.b as f64, c.a as f64])
                                }
                                Ok(sr_model::values::Color::Token(t)) => cx.tokens.get(&t).copied(),
                                Err(_) => None,
                            }
                        })
                        .collect();
                    let values: Vec<f64> =
                        fs.iter().filter(|f| keep(f, filter.as_deref())).filter_map(|f| f.number(prop)).collect();
                    let domain = ca.nums("domain").filter(|d| !d.is_empty()).unwrap_or_else(|| {
                        let lo = values.iter().cloned().fold(f64::INFINITY, f64::min);
                        let hi = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                        if lo.is_finite() {
                            vec![lo, hi]
                        } else {
                            vec![0.0, 1.0]
                        }
                    });
                    Scale { palette, domain, kind: ca.str("scale").unwrap_or_else(|| "linear".into()) }
                });
                // per-feature overrides
                // fill, stroke, stroke width, opacity
                type Override = (Option<Paint>, Option<Paint>, Option<f64>, f64);
                let mut styles: HashMap<String, Override> = HashMap::new();
                for (s, skey) in keyed(c, &ckey) {
                    if is(s, "featureStyle") {
                        let sa = attrs(g, s, &skey);
                        let k = sa.str("key").unwrap_or_default();
                        let v = (
                            paint(cx, sa.paint("fill")),
                            paint(cx, sa.paint("stroke")),
                            sa.opt("strokeWidth"),
                            sa.num("opacity", 1.0),
                        );
                        styles.insert(k, v);
                    }
                }
                let label_prop = ca.str("label");
                let lstyle = label_prop.as_ref().map(|_| label_style(cx, ca.str("textStyle").as_deref()));
                let mut run: Option<(Option<Paint>, f64, Vec<Poly>)> = None;
                let mut strokes: Vec<(Vec<Poly>, Option<Paint>, f64, f64)> = Vec::new();
                for f in fs.iter().filter(|f| keep(f, filter.as_deref())) {
                    let over = f.text(&key_by).and_then(|k| styles.get(&k));
                    let mut fpaint = match (&scale, &fill_by) {
                        (Some(sc), Some(prop)) => match f.number(prop).and_then(|v| sc.color(v)) {
                            Some(rgba) => Some(Paint::Solid { rgba, srgb: false }),
                            None => no_data.clone().or(fill.clone()),
                        },
                        _ => fill.clone(),
                    };
                    let mut spaint = stroke_paint.clone();
                    let mut fw = sw;
                    let mut fo = opacity;
                    if let Some((of, os, ow, oo)) = over {
                        if of.is_some() {
                            fpaint = of.clone();
                        }
                        if os.is_some() {
                            spaint = os.clone();
                        }
                        if let Some(ow) = ow {
                            fw = *ow;
                        }
                        fo *= oo;
                    }
                    let pl = proj.project(&f.geometry);
                    // Neighbours with the same paint fill as one shape, so the borders they share
                    // leave no anti-aliasing seam; strokes follow once the run of fills ends.
                    let rings: Vec<Poly> = pl.polygons.iter().flat_map(|poly| polys_of_rings(poly, true)).collect();
                    if run.as_ref().is_some_and(|(p, o, _)| *p != fpaint || *o != fo) {
                        let (p, o, polys) = run.take().expect("checked");
                        pt.fill(polys, &p, o);
                        for (polys, sp, w, o) in strokes.drain(..) {
                            pt.stroke(&polys, &sp, w, o);
                        }
                    }
                    run.get_or_insert_with(|| (fpaint.clone(), fo, Vec::new())).2.extend(rings.iter().cloned());
                    if spaint.is_some() && fw > 0.0 && !rings.is_empty() {
                        // `progress` traces each ring from its first vertex by the same fraction, closing edge
                        // included, like a line; the fill stays whole
                        let outline = if progress < 1.0 {
                            measure::trim(&rings, 0.0, progress, 0.0, TrimMode::Simultaneous)
                        } else {
                            rings
                        };
                        strokes.push((outline, spaint.clone(), fw, fo));
                    }
                    if !pl.lines.is_empty() {
                        let mut lines = polys_of_rings(&pl.lines, false);
                        if progress < 1.0 {
                            lines = measure::trim(&lines, 0.0, progress, 0.0, TrimMode::Simultaneous);
                        }
                        let lp = if spaint.is_some() { &spaint } else { &fpaint };
                        pt.stroke(&lines, lp, fw.max(0.5), fo);
                    }
                    for q in &pl.points {
                        pt.dot(*q, point_r, &fpaint, &spaint, fw, fo);
                    }
                    if let (Some(prop), Some(st)) = (&label_prop, &lstyle) {
                        if let Some(text) = f.text(prop) {
                            let anchor = label_anchor(&pl).map(|a| a.0).or_else(|| pl.points.first().copied());
                            if let Some(a) = anchor {
                                labels.push((text, st.clone(), [a[0], a[1] - st.size * 0.6], Align::Center));
                            }
                        }
                    }
                }
                if let Some((p, o, polys)) = run.take() {
                    pt.fill(polys, &p, o);
                }
                for (polys, sp, w, o) in strokes.drain(..) {
                    pt.stroke(&polys, &sp, w, o);
                }
            } else if is(c, "graticule") {
                let pl = proj.project(&graticule(ca.num("step", 10.0)));
                let sp = paint(cx, ca.paint("stroke"));
                pt.stroke(&polys_of_rings(&pl.lines, false), &sp, ca.num("strokeWidth", 0.5), opacity);
            } else if is(c, "route") {
                let mut paths: Vec<Vec<[f64; 2]>> = Vec::new();
                if let Some(s) = ca.str("points") {
                    paths.push(lon_lats(&s));
                }
                if let Some(geo) = ca.str("geo") {
                    let filter = ca.str("filter");
                    for f in sr_eval::geo::features(cx.p, &geo)?.iter().filter(|f| keep(f, filter.as_deref())) {
                        if let Geometry::Lines(ls) = &f.geometry {
                            paths.extend(ls.iter().cloned());
                        }
                    }
                }
                let progress = ca.num("progress", 1.0);
                let sp = paint(cx, ca.paint("stroke"));
                let sw = ca.num("strokeWidth", 3.0);
                let dash = ca.nums("dash").filter(|d| d.iter().any(|v| *v > 0.0));
                let head_r = ca.num("headRadius", 0.0);
                let head_paint = paint(cx, ca.paint("headFill")).or(sp.clone());
                for path in paths.iter().filter(|p| p.len() > 1) {
                    let part = ground_trim(path, progress);
                    if part.len() > 1 {
                        let pl = proj.project(&Geometry::Lines(vec![part.clone()]));
                        let mut lines = polys_of_rings(&pl.lines, false);
                        if let Some(dash) = &dash {
                            lines = measure::dash(&lines, dash, 0.0);
                        }
                        pt.stroke(&lines, &sp, sw, opacity);
                    }
                    if head_r > 0.0 && progress > 0.0 {
                        let tip = part.last().copied().unwrap_or(path[0]);
                        if let Some(q) = proj.point(tip[0], tip[1]) {
                            pt.dot(q, head_r, &head_paint, &None, 0.0, opacity);
                        }
                    }
                }
            } else if is(c, "pin") {
                let (lon, lat) = (ca.num("lon", 0.0), ca.num("lat", 0.0));
                let Some(q) = proj.point(lon, lat) else { continue };
                let fp = paint(cx, ca.paint("fill"));
                let sp = paint(cx, ca.paint("stroke"));
                pt.dot(q, ca.num("radius", 6.0), &fp, &sp, ca.num("strokeWidth", 2.0), opacity);
                if let Some(text) = ca.str("label") {
                    let st = label_style(cx, ca.str("textStyle").as_deref());
                    let pos = [q[0] + ca.num("labelDx", 10.0), q[1] + ca.num("labelDy", 0.0) - st.size * 0.6];
                    let align = if ca.num("labelDx", 10.0) < 0.0 { Align::End } else { Align::Start };
                    labels.push((text, st, pos, align));
                }
            }
        }
        let op = paint(cx, at.paint("outline"));
        let ring: Vec<Poly> = sphere_outline.polygons.iter().flat_map(|p| polys_of_rings(p, true)).collect();
        pt.stroke(&ring, &op, at.num("outlineWidth", 1.0), 1.0);
    }
    tc.font_assets = fonts;
    let lib = tc.lib();
    // raster tiles are drawn beneath all of the map's vector content, its background included
    let opaque_bg = at.paint("background").is_some_and(|v| match (cx.paint)(&v, bx) {
        Some(Paint::Solid { rgba, .. }) => rgba[3] > 0.0,
        Some(_) => true,
        None => false,
    });
    if !raster.is_empty() && opaque_bg {
        let msg = format!("{}: the raster basemap is drawn beneath the map's background", mp.id);
        if !cx.unsupported.contains(&msg) {
            cx.unsupported.push(msg);
        }
    }
    d.bitmaps.extend(raster);

    // the data's credit, small in the bottom-right corner
    if !credits.is_empty() {
        let text = strip_tags(&credits.join(" · "));
        let st = Style {
            size: (h / 60.0).clamp(9.0, 14.0),
            color: Some(Paint::Solid { rgba: [0.1, 0.1, 0.1, 1.0], srgb: false }),
            ..Default::default()
        };
        let width = w * 0.8;
        let t = chart::text(lib, &text, &st, w - width - 6.0, h - st.size * 1.6, Align::End, width, tol);
        let bg: Vec<Poly> = t
            .scene
            .cmds
            .iter()
            .flat_map(|c| match c {
                Cmd::Fill { polys, .. } => polys.clone(),
                _ => Vec::new(),
            })
            .collect();
        let b = sr_vector::path::poly_bounds(&bg).0;
        if b[2] > b[0] {
            let pad = st.size * 0.3;
            let plate =
                shapes::rect(b[0] - pad, b[1] - pad, b[2] - b[0] + 2.0 * pad, b[3] - b[1] + 2.0 * pad, [0.0; 4])
                    .flatten(tol);
            d.scene.cmds.push(Cmd::Fill {
                polys: plate,
                rule: FillRule::NonZero,
                paint: Paint::Solid { rgba: [1.0; 4], srgb: false },
                opacity: 0.7,
            });
        }
        d.extend(t);
    }
    for (text, st, pos, align) in labels {
        let width = 1000.0;
        let x = match align {
            Align::Center => pos[0] - width / 2.0,
            Align::End => pos[0] - width,
            _ => pos[0],
        };
        d.extend(chart::text(lib, &text, &st, x, pos[1], align, width, tol));
    }
    // a map stops at its frame (projection keeps a margin for strokes; pins and labels reach over)
    let content = std::mem::take(&mut d.scene);
    d.scene.cmds.push(Cmd::Push { mask_init: 0.0 });
    d.scene.extend(content);
    d.scene.cmds.push(Cmd::Mask {
        polys: shapes::rect(0.0, 0.0, w, h, [0.0; 4]).flatten(tol),
        rule: FillRule::NonZero,
        op: sr_vector::scene::MaskOp::Add,
        opacity: 1.0,
        invert: false,
    });
    d.scene.cmds.push(Cmd::Pop { opacity: 1.0 });
    Ok(d)
}
