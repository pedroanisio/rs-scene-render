//! Basemaps: tiles drawn under a map's content.
//!
//! Vector tiles (MVT) are drawn layer by layer through a MapLibre style:
//! background, fills (neighbours of one paint filled as one shape, so tile and
//! feature borders leave no seam), lines (width, dashes, caps and joins, casings
//! as their full width), circles, and text labels placed without overlapping,
//! the style's upper layers and lower sort keys first, points at their anchor
//! (with variable anchors and offsets) and road names along their lines,
//! upright. Raster tiles are drawn as images, cut into cells whose corners are
//! projected, so they bend into any projection and stop at a globe's edge.

use std::sync::Arc;

use sr_eval::geo::{self as geo, TileData};
use sr_geo::mvt::{GeomType, Layer as TileLayer};
use sr_geo::project::Projection;
use sr_geo::style::{Ctx, Layer, V};
use sr_geo::tiles::{self, Placer, Tile};
use sr_text::layout::Align;
use sr_text::{chart, Bitmap, FontLib, Style as TextStyle};
use sr_vector::geom::{p, Xf};
use sr_vector::measure;
use sr_vector::scene::{Cmd, FillRule};
use sr_vector::stroke::{self, Cap, Join, Style as StrokeStyle};
use sr_vector::{Paint, Poly};

use super::map::Painter;
use crate::vector::Attrs;

fn solid(c: [f64; 4]) -> Option<Paint> {
    Some(Paint::Solid { rgba: [c[0], c[1], c[2], 1.0], srgb: false })
}

fn closed(parts: &[Vec<[f64; 2]>]) -> Vec<Vec<[f64; 2]>> {
    parts
        .iter()
        .map(|r| {
            let mut r = r.clone();
            if let Some(f) = r.first().copied() {
                r.push(f);
            }
            r
        })
        .collect()
}

fn polys(rings: &[Vec<[f64; 2]>], closed: bool) -> Vec<Poly> {
    rings
        .iter()
        .filter(|r| r.len() > 1)
        .map(|r| Poly { pts: r.iter().map(|q| p(q[0], q[1])).collect(), closed })
        .collect()
}

fn geom_name(k: GeomType) -> &'static str {
    match k {
        GeomType::Point => "Point",
        GeomType::LineString => "LineString",
        GeomType::Polygon => "Polygon",
        GeomType::Unknown => "Unknown",
    }
}

/// Drawing waiting in a run of one style: its key, its polygons, and how to draw them.
type Run = Option<(String, Vec<Poly>, Box<dyn Fn(&mut Painter, Vec<Poly>)>)>;

/// A text label waiting for placement.
struct Label {
    text: String,
    style: TextStyle,
    halo: Option<([f64; 4], f64)>,
    /// Where: a point with its anchor candidates, or along a line.
    at: Anchor,
    /// Offset in ems (x right, y down).
    offset: [f64; 2],
    radial: f64,
    max_width: f64,
    padding: f64,
    /// Placement order: lower first.
    rank: (usize, f64, usize),
}

enum Anchor {
    Point([f64; 2], Vec<String>),
    Line(Vec<[f64; 2]>),
}

/// Everything the basemap drew: vector commands go straight to the painter; raster tiles and
/// labels come back.
pub struct Output {
    pub bitmaps: Vec<Bitmap>,
    pub attribution: Option<String>,
    /// Style layer types the basemap skipped (not drawn by this engine).
    pub skipped: Vec<String>,
}

/// Draws the basemap child `ba` of a map.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    cx: &super::Cx,
    lib: &mut FontLib,
    pt: &mut Painter,
    proj: &Projection,
    size: [f64; 2],
    ba: &Attrs,
    tol: f64,
) -> Result<Output, String> {
    let id = ba.str("tiles").unwrap_or_default();
    let (asset, path) = geo::tiles_asset(cx.p, &id)?;
    let arch = geo::archive(&path)?;
    let opacity = ba.num("opacity", 1.0);
    let detail = ba.num("detail", 0.0);
    let attribution = asset.attribution.clone().or_else(|| arch.metadata["attribution"].as_str().map(str::to_string));
    let mut out = Output { bitmaps: Vec::new(), attribution, skipped: Vec::new() };
    if opacity <= 0.0 {
        return Ok(out);
    }
    if arch.header.tile_type.raster() {
        let z = geo::basemap_zoom(proj, asset.tile_size.unwrap_or(256) as f64, detail, true);
        raster(&arch, &path, proj, z, opacity, size, &mut out.bitmaps)?;
        return Ok(out);
    }
    let style = geo::style(cx.p, ba.str("mapStyle").as_deref())?;
    let zoom = tiles::map_zoom(proj) + detail;
    let z = geo::basemap_zoom(proj, asset.tile_size.unwrap_or(512) as f64, detail, false).min(22);
    let mut loaded: Vec<(Tile, [f64; 3], Arc<TileData>)> = Vec::new();
    for t in tiles::visible(proj, z) {
        let (src, window) = tiles::source(t, arch.header.max_zoom);
        if let Some(d) = geo::tile(&arch, src.z, src.x, src.y)? {
            loaded.push((src, window, d));
        }
    }
    let empty = serde_json::Map::new();
    let mut labels: Vec<Label> = Vec::new();
    let mut seq = 0usize;
    for (li, layer) in style.layers.iter().enumerate() {
        if !layer.visible_at(zoom) {
            continue;
        }
        match layer.kind.as_str() {
            "background" => {
                let cx0 = Ctx { zoom, properties: &empty, geometry: "Polygon", id: None };
                let c = layer.paint("background-color", &cx0).color().unwrap_or([0.0; 4]);
                let o = layer.paint("background-opacity", &cx0).num().unwrap_or(1.0) * c[3] * opacity;
                let world = proj.project(&sr_geo::data::Geometry::Sphere);
                let rings: Vec<Poly> = world.polygons.iter().flat_map(|r| polys(r, true)).collect();
                pt.fill(rings, &solid(c), o);
            }
            "fill" | "line" | "circle" | "symbol" => {
                // runs of equal style drawn together
                let mut run: Run = None;
                for (src, window, data) in &loaded {
                    let TileData::Vector(tl) = &**data else { continue };
                    let Some(l) = tl.iter().find(|l| Some(&l.name) == layer.source_layer.as_ref()) else { continue };
                    let placer = Placer::new(proj, *src, l.extent, *window);
                    features(layer, l, zoom, &placer, pt, &mut run, &mut labels, li, &mut seq, opacity, tol);
                }
                if let Some((_, ps, f)) = run.take() {
                    f(pt, ps);
                }
            }
            other => {
                if !out.skipped.iter().any(|k| k == other) {
                    out.skipped.push(other.to_string());
                }
            }
        }
    }
    if ba.str("labels").as_deref() != Some("false") {
        place_labels(lib, pt, labels, size, opacity, tol);
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn features(
    layer: &Layer,
    l: &TileLayer,
    zoom: f64,
    placer: &Placer,
    pt: &mut Painter,
    run: &mut Run,
    labels: &mut Vec<Label>,
    li: usize,
    seq: &mut usize,
    opacity: f64,
    tol: f64,
) {
    for f in &l.features {
        let cx = Ctx { zoom, properties: &f.properties, geometry: geom_name(f.kind), id: f.id };
        if !layer.accepts(&cx) {
            continue;
        }
        match layer.kind.as_str() {
            "fill" if f.kind == GeomType::Polygon => {
                let c = layer.paint("fill-color", &cx).color().unwrap_or([0.0; 4]);
                let o = layer.paint("fill-opacity", &cx).num().unwrap_or(1.0) * c[3] * opacity;
                if o <= 0.0 {
                    continue;
                }
                let rings = placer.polygons(&f.polygons());
                let key = format!("fill {c:?} {o}");
                let ps = polys(&rings, true);
                push(pt, run, key, ps, move |pt: &mut Painter, ps| pt.fill(ps, &solid(c), o));
            }
            "line" if f.kind != GeomType::Point => {
                let c = layer.paint("line-color", &cx).color().unwrap_or([0.0; 4]);
                let o = layer.paint("line-opacity", &cx).num().unwrap_or(1.0) * c[3] * opacity;
                let w = layer.paint("line-width", &cx).num().unwrap_or(1.0);
                let gap = layer.paint("line-gap-width", &cx).num().unwrap_or(0.0);
                let width = if gap > 0.0 { gap + 2.0 * w } else { w };
                if o <= 0.0 || width <= 0.0 {
                    continue;
                }
                let dash: Vec<f64> = match layer.paint("line-dasharray", &cx) {
                    V::Arr(a) => a.iter().filter_map(V::num).map(|d| d * w.max(1.0)).collect(),
                    _ => Vec::new(),
                };
                let cap = match layer.layout("line-cap", &cx).text().as_str() {
                    "round" => Cap::Round,
                    "square" => Cap::Square,
                    _ => Cap::Butt,
                };
                let join = match layer.layout("line-join", &cx).text().as_str() {
                    "round" => Join::Round,
                    "bevel" => Join::Bevel,
                    _ => Join::Miter,
                };
                let parts = if f.kind == GeomType::Polygon { closed(&f.parts) } else { f.parts.clone() };
                let lines = placer.lines(&parts);
                let key = format!("line {c:?} {o} {width} {dash:?} {cap:?} {join:?}");
                let ps = polys(&lines, false);
                push(pt, run, key, ps, move |pt: &mut Painter, ps: Vec<Poly>| {
                    let ps = if dash.len() >= 2 && dash.iter().any(|d| *d > 0.0) {
                        measure::dash(&ps, &dash, 0.0)
                    } else {
                        ps
                    };
                    let style = StrokeStyle { width, cap, join, miter_limit: 2.0 };
                    let outline = stroke::stroke(&ps, &style, tol);
                    pt.fill(outline, &solid(c), o);
                });
            }
            "circle" => {
                let c = layer.paint("circle-color", &cx).color().unwrap_or([0.0; 4]);
                let o = layer.paint("circle-opacity", &cx).num().unwrap_or(1.0) * c[3] * opacity;
                let r = layer.paint("circle-radius", &cx).num().unwrap_or(5.0);
                for q in placer.points(&f.parts) {
                    pt.dot(q, r, &solid(c), &None, 0.0, o);
                }
            }
            "symbol" => {
                let text = layer.layout("text-field", &cx).text();
                let text = match layer.layout("text-transform", &cx).text().as_str() {
                    "uppercase" => text.to_uppercase(),
                    "lowercase" => text.to_lowercase(),
                    _ => text,
                };
                if text.trim().is_empty() {
                    continue;
                }
                let size = layer.layout("text-size", &cx).num().unwrap_or(16.0);
                let color = layer.paint("text-color", &cx).color().unwrap_or([0.0, 0.0, 0.0, 1.0]);
                let halo_w = layer.paint("text-halo-width", &cx).num().unwrap_or(0.0);
                let halo_c = layer.paint("text-halo-color", &cx).color().unwrap_or([0.0; 4]);
                let font = layer.layout.get("text-font").map(|v| v.to_string()).unwrap_or_default();
                let weight = if font.contains("Bold") {
                    700
                } else if font.contains("Medium") || font.contains("Semibold") {
                    500
                } else {
                    400
                };
                let mut style = TextStyle { size, weight, color: solid(color), ..Default::default() };
                style.italic = font.contains("Italic");
                let em2 = |v: V| match v {
                    V::Arr(a) if a.len() >= 2 => [a[0].num().unwrap_or(0.0), a[1].num().unwrap_or(0.0)],
                    _ => [0.0, 0.0],
                };
                let offset = em2(layer.layout("text-offset", &cx));
                let radial = layer.layout("text-radial-offset", &cx).num().unwrap_or(0.0);
                let anchors = match layer.layout("text-variable-anchor", &cx) {
                    V::Arr(a) if !a.is_empty() => a.iter().map(V::text).collect(),
                    _ => vec![layer.layout("text-anchor", &cx).text()],
                };
                let sort = layer.layout("symbol-sort-key", &cx).num().unwrap_or(0.0);
                let max_width = layer.layout("text-max-width", &cx).num().unwrap_or(10.0);
                let padding = layer.layout("text-padding", &cx).num().unwrap_or(2.0);
                let halo = (halo_w > 0.0 && halo_c[3] > 0.0).then_some((halo_c, halo_w));
                let line_placed = layer.layout("symbol-placement", &cx).text() == "line";
                let mut add = |at: Anchor| {
                    *seq += 1;
                    labels.push(Label {
                        text: text.clone(),
                        style: style.clone(),
                        halo,
                        at,
                        offset,
                        radial,
                        max_width,
                        padding,
                        rank: (usize::MAX - li, sort, *seq),
                    });
                };
                match f.kind {
                    GeomType::LineString if line_placed => {
                        for l in placer.lines(&f.parts) {
                            add(Anchor::Line(l));
                        }
                    }
                    GeomType::Point => {
                        for q in placer.points(&f.parts) {
                            add(Anchor::Point(q, anchors.clone()));
                        }
                    }
                    GeomType::Polygon => {
                        // the centre of the largest ring
                        let rings = placer.polygons(&f.polygons());
                        let best = rings.iter().max_by(|a, b| {
                            sr_geo::mvt::signed_area(a).abs().total_cmp(&sr_geo::mvt::signed_area(b).abs())
                        });
                        if let Some(r) = best {
                            let n = r.len() as f64;
                            let c = r.iter().fold([0.0, 0.0], |s, q| [s[0] + q[0] / n, s[1] + q[1] / n]);
                            add(Anchor::Point(c, anchors.clone()));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// Adds polygons to the current run, flushing it when the style changes.
fn push(pt: &mut Painter, run: &mut Run, key: String, ps: Vec<Poly>, draw: impl Fn(&mut Painter, Vec<Poly>) + 'static) {
    if ps.is_empty() {
        return;
    }
    match run {
        Some((k, acc, _)) if *k == key => acc.extend(ps),
        _ => {
            if let Some((_, acc, f)) = run.take() {
                f(pt, acc);
            }
            *run = Some((key, ps, Box::new(draw)));
        }
    }
}

fn overlaps(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// Places labels without overlap and draws them.
fn place_labels(lib: &mut FontLib, pt: &mut Painter, mut labels: Vec<Label>, size: [f64; 2], opacity: f64, tol: f64) {
    labels.sort_by(|a, b| a.rank.0.cmp(&b.rank.0).then(a.rank.1.total_cmp(&b.rank.1)).then(a.rank.2.cmp(&b.rank.2)));
    let mut placed: Vec<[f64; 4]> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for l in labels {
        let st = &l.style;
        let em = st.size;
        let width = (l.max_width * em).max(em);
        let d = chart::text(lib, &l.text, st, -width / 2.0, 0.0, Align::Center, width, tol);
        let polys: Vec<Poly> = d
            .scene
            .cmds
            .iter()
            .flat_map(|c| match c {
                Cmd::Fill { polys, .. } => polys.clone(),
                _ => Vec::new(),
            })
            .collect();
        let b = sr_vector::path::poly_bounds(&polys).0;
        if !matches!(b[2].partial_cmp(&b[0]), Some(std::cmp::Ordering::Greater)) {
            continue;
        }
        let (tw, th) = (b[2] - b[0], b[3] - b[1]);
        // candidate transforms (text box → map), with their boxes
        let mut candidates: Vec<(Xf, [f64; 4])> = Vec::new();
        match &l.at {
            Anchor::Point(q, anchors) => {
                for a in anchors {
                    let (ax, ay) = match a.as_str() {
                        "left" => (0.0, 0.5),
                        "right" => (1.0, 0.5),
                        "top" => (0.5, 0.0),
                        "bottom" => (0.5, 1.0),
                        "top-left" => (0.0, 0.0),
                        "top-right" => (1.0, 0.0),
                        "bottom-left" => (0.0, 1.0),
                        "bottom-right" => (1.0, 1.0),
                        _ => (0.5, 0.5),
                    };
                    // a radial offset pushes the text away from the point, toward its anchor side
                    let r = l.radial * em;
                    let dir = [(0.5 - ax) * 2.0, (0.5 - ay) * 2.0];
                    let ox = l.offset[0] * em + dir[0] * r;
                    let oy = l.offset[1] * em + dir[1] * r;
                    let x0 = q[0] + ox - ax * tw;
                    let y0 = q[1] + oy - ay * th;
                    let xf = Xf::translate(x0 - b[0], y0 - b[1]);
                    candidates.push((xf, [x0, y0, x0 + tw, y0 + th]));
                }
            }
            Anchor::Line(pts) => {
                let len: f64 = pts.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
                if len < tw * 1.1 {
                    continue;
                }
                let mut left = len / 2.0;
                let mut at = None;
                for w in pts.windows(2) {
                    let s = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
                    if left <= s && s > 0.0 {
                        let t = left / s;
                        at = Some((
                            [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t],
                            (w[1][1] - w[0][1]).atan2(w[1][0] - w[0][0]),
                        ));
                        break;
                    }
                    left -= s;
                }
                let Some((c, mut ang)) = at else { continue };
                // keep text upright
                if ang > std::f64::consts::FRAC_PI_2 {
                    ang -= std::f64::consts::PI;
                } else if ang < -std::f64::consts::FRAC_PI_2 {
                    ang += std::f64::consts::PI;
                }
                let xf = Xf::translate(c[0], c[1])
                    .mul(&Xf::rotate(ang.to_degrees()))
                    .mul(&Xf::translate(-(b[0] + b[2]) / 2.0, -(b[1] + b[3]) / 2.0));
                let (cs, sn) = (ang.cos().abs(), ang.sin().abs());
                let (hw, hh) = ((tw * cs + th * sn) / 2.0, (tw * sn + th * cs) / 2.0);
                candidates.push((xf, [c[0] - hw, c[1] - hh, c[0] + hw, c[1] + hh]));
            }
        }
        let pad = l.padding;
        let chosen = candidates.into_iter().find(|(_, bx)| {
            let bx = [bx[0] - pad, bx[1] - pad, bx[2] + pad, bx[3] + pad];
            bx[2] > 0.0 && bx[3] > 0.0 && bx[0] < size[0] && bx[1] < size[1] && !placed.iter().any(|p| overlaps(p, &bx))
        });
        let Some((xf, bx)) = chosen else { continue };
        // the same name at nearly the same place (a label repeated by neighbouring tiles)
        let key = (l.text.clone(), (bx[0] / 8.0).round() as i64, (bx[1] / 8.0).round() as i64);
        if !seen.insert(key) {
            continue;
        }
        placed.push([bx[0] - pad, bx[1] - pad, bx[2] + pad, bx[3] + pad]);
        let moved: Vec<Poly> = polys
            .iter()
            .map(|q| Poly { pts: q.pts.iter().map(|&v| xf.apply(v)).collect(), closed: q.closed })
            .collect();
        if let Some((hc, hw)) = l.halo {
            let halo = stroke::stroke(
                &moved,
                &StrokeStyle { width: 2.0 * hw, cap: Cap::Round, join: Join::Round, miter_limit: 2.0 },
                tol,
            );
            pt.fill(halo, &solid(hc), hc[3] * opacity);
        }
        let color = match &st.color {
            Some(Paint::Solid { rgba, .. }) => *rgba,
            _ => [0.0, 0.0, 0.0, 1.0],
        };
        pt.d.scene.cmds.push(Cmd::Fill {
            polys: moved,
            rule: FillRule::NonZero,
            paint: Paint::Solid { rgba: color, srgb: false },
            opacity,
        });
    }
}

/// A part of a raster tile drawn as one affine image.
struct Cell<'a> {
    t: Tile,
    proj: &'a Projection,
    frame_diag: f64,
    size: [f64; 2],
    bytes: &'a Arc<Vec<u8>>,
    key: u64,
    /// The tile's square inside the archive tile (x, y, size in its tile units).
    window: [f64; 3],
    opacity: f64,
}

impl Cell<'_> {
    /// Emits the part (u0, v0)‥(u1, v1) of the tile (0‥1 units): cropped to the frame exactly
    /// when it lies square to it, else split while it crosses the frame's edge.
    fn emit(&self, u0: f64, v0: f64, u1: f64, v1: f64, depth: u32, out: &mut Vec<Bitmap>) {
        let at = |u: f64, v: f64| {
            let [lon, lat] = tiles::lonlat(self.t.z, self.t.x as f64 + u, self.t.y as f64 + v);
            // corners must be on the visible side; the unclipped point places them
            self.proj.visible(lon, lat).then(|| self.proj.point_unclipped(lon, lat))
        };
        let (Some(p0), Some(p1), Some(p2), Some(p3)) = (at(u0, v0), at(u1, v0), at(u0, v1), at(u1, v1)) else { return };
        let far = |a: [f64; 2], b: [f64; 2]| (b[0] - a[0]).hypot(b[1] - a[1]) > self.frame_diag;
        // a cell torn across the view's antimeridian
        if far(p0, p1) || far(p0, p2) || far(p0, p3) {
            return;
        }
        let xs = [p0[0], p1[0], p2[0], p3[0]];
        let ys = [p0[1], p1[1], p2[1], p3[1]];
        let (x0, x1) =
            (xs.iter().cloned().fold(f64::INFINITY, f64::min), xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
        let (y0, y1) =
            (ys.iter().cloned().fold(f64::INFINITY, f64::min), ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
        if x1 <= 0.0 || y1 <= 0.0 || x0 >= self.size[0] || y0 >= self.size[1] {
            return;
        }
        let [ox, oy, s] = self.window;
        let square = (p1[1] - p0[1]).abs() < 1e-9 && (p2[0] - p0[0]).abs() < 1e-9 && p1[0] > p0[0] && p2[1] > p0[1];
        let inside = x0 >= 0.0 && y0 >= 0.0 && x1 <= self.size[0] && y1 <= self.size[1];
        let (mut a, mut b, mut c, mut d) = (u0, v0, u1, v1);
        let (mut q0, mut qx, mut qy) = (p0, [p1[0] - p0[0], p1[1] - p0[1]], [p2[0] - p0[0], p2[1] - p0[1]]);
        if square && !inside {
            // crop to the frame, moving the texture window with it
            let (cx0, cx1) = (x0.max(0.0), x1.min(self.size[0]));
            let (cy0, cy1) = (y0.max(0.0), y1.min(self.size[1]));
            let (du, dv) = ((u1 - u0) / (x1 - x0), (v1 - v0) / (y1 - y0));
            a = u0 + (cx0 - x0) * du;
            c = u0 + (cx1 - x0) * du;
            b = v0 + (cy0 - y0) * dv;
            d = v0 + (cy1 - y0) * dv;
            q0 = [cx0, cy0];
            qx = [cx1 - cx0, 0.0];
            qy = [0.0, cy1 - cy0];
        } else if !inside && depth > 0 {
            let (um, vm) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
            self.emit(u0, v0, um, vm, depth - 1, out);
            self.emit(um, v0, u1, vm, depth - 1, out);
            self.emit(u0, vm, um, v1, depth - 1, out);
            self.emit(um, vm, u1, v1, depth - 1, out);
            return;
        }
        out.push(Bitmap {
            png: self.bytes.clone(),
            key: self.key,
            rect: [0.0, 0.0, 1.0, 1.0],
            xf: Xf([qx[0], qx[1], qy[0], qy[1], q0[0], q0[1]]),
            opacity: self.opacity,
            uv: [ox + a * s, oy + b * s, ox + c * s, oy + d * s],
            below: true,
        });
    }
}

/// Raster tiles as warped image cells beneath the map's vectors.
fn raster(
    arch: &Arc<sr_geo::pmtiles::Archive>,
    path: &std::path::Path,
    proj: &Projection,
    z: u8,
    opacity: f64,
    size: [f64; 2],
    out: &mut Vec<Bitmap>,
) -> Result<(), String> {
    let exact = proj.raw() == sr_geo::project::Raw::Mercator && proj.rotate()[1] == 0.0 && proj.rotate()[2] == 0.0;
    let n = if exact { 1 } else { 8 };
    let frame = proj.extent_rect().map(|r| (r.x1 - r.x0).hypot(r.y1 - r.y0)).unwrap_or(1e9);
    for t in tiles::visible(proj, z) {
        // the finest archive tile that holds this one
        let mut src = tiles::source(t, arch.header.max_zoom);
        let mut data = geo::tile(arch, src.0.z, src.0.x, src.0.y)?;
        while data.is_none() && src.0.z > arch.header.min_zoom {
            let up = Tile { z: src.0.z - 1, x: src.0.x / 2, y: src.0.y / 2 };
            let w = src.1;
            src = (up, [(w[0] + (src.0.x % 2) as f64) / 2.0, (w[1] + (src.0.y % 2) as f64) / 2.0, w[2] / 2.0]);
            data = geo::tile(arch, up.z, up.x, up.y)?;
        }
        let Some(d) = data else { continue };
        let TileData::Raster(bytes) = &*d else { continue };
        let key = sr_eval::rng::hash_str(&format!("{}|{}/{}/{}", path.display(), src.0.z, src.0.x, src.0.y));
        let (ox, oy, s) = (src.1[0], src.1[1], src.1[2]);
        let cell = Cell { t, proj, frame_diag: frame, size, bytes, key, window: [ox, oy, s], opacity };
        for i in 0..n {
            for j in 0..n {
                let f = n as f64;
                cell.emit(i as f64 / f, j as f64 / f, (i + 1) as f64 / f, (j + 1) as f64 / f, 3, out);
            }
        }
    }
    Ok(())
}
