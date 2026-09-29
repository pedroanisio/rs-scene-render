//! Glyphs to vector scenes: outlines, colour glyphs (COLR v0/v1 as vectors,
//! CBDT/sbix as bitmaps), strokes, shadows, decorations, highlights and
//! backgrounds.

use std::sync::Arc;

use rustybuzz::ttf_parser::{self, colr, GlyphId};
use sr_vector::geom::{p, Rect, Xf};
use sr_vector::scene::{Cmd, FillRule, Gradient, GradientKind, MaskOp, Paint, Scene};
use sr_vector::stroke::{self, Cap, Join, Style as StrokeStyle};
use sr_vector::{shapes, Path, Poly};

use crate::font::FontLib;
use crate::layout::Layout;
use crate::style::{Decoration, StrokePos};

/// A bitmap glyph (PNG) placed in the drawing's space.
#[derive(Debug, Clone, PartialEq)]
pub struct Bitmap {
    pub png: Arc<Vec<u8>>,
    /// Cache key of the image data.
    pub key: u64,
    /// Rectangle x, y, w, h before `xf`.
    pub rect: [f64; 4],
    pub xf: Xf,
    pub opacity: f64,
    /// The part of the image drawn (u0, v0, u1, v1).
    pub uv: [f64; 4],
    /// Drawn before the vector commands instead of after them (map raster tiles).
    pub below: bool,
}

/// Vector commands plus bitmaps, in the text box's space.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drawing {
    pub scene: Scene,
    pub bitmaps: Vec<Bitmap>,
    /// Glyphs with a per-glyph blur, grouped by radius (text-box pixels).
    pub blurred: Vec<(f64, Scene)>,
}

impl Drawing {
    /// Appends another drawing.
    pub fn extend(&mut self, o: Drawing) {
        self.scene.extend(o.scene);
        self.bitmaps.extend(o.bitmaps);
        self.blurred.extend(o.blurred);
    }
    /// Transformed copy.
    pub fn transformed(&self, x: &Xf) -> Drawing {
        Drawing {
            scene: self.scene.transformed(x),
            bitmaps: self.bitmaps.iter().map(|b| Bitmap { xf: x.mul(&b.xf), ..b.clone() }).collect(),
            blurred: self.blurred.iter().map(|(r, s)| (*r * x.max_scale(), s.transformed(x))).collect(),
        }
    }
}

/// Per-glyph effects (text animators, captions).
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphFx {
    /// Applied after placement (in the box's space).
    pub xf: Xf,
    pub opacity: f64,
    /// Replacement fill, mixed in by `fill_mix` (solid colours mix; other paints switch at ½).
    pub fill: Option<Paint>,
    pub fill_mix: f64,
    pub stroke: Option<Paint>,
    pub stroke_mix: f64,
    pub stroke_width: f64,
    /// Replacement glyph (character offsets).
    pub gid: Option<u16>,
    pub hidden: bool,
    /// Blur radius in text-box pixels (drawn separately and blurred by the compositor).
    pub blur: f64,
    /// Variable-font axis targets, reached by `variation_mix` from the style's values.
    pub variation: Vec<([u8; 4], f32)>,
    pub variation_mix: f64,
    /// Highlight box behind the glyph's unit (the highlight animator preset, boxed-word captions).
    pub highlight: Option<Highlight>,
}

/// A box behind a unit of glyphs: the unit's extent on its line and the line box's height,
/// grown by `pad` (half of it vertically) with corner `radius`, covering `fraction` of that
/// width from the unit's start.
#[derive(Debug, Clone, PartialEq)]
pub struct Highlight {
    pub paint: Paint,
    pub fraction: f64,
    /// Glyphs with the same unit on a line share one box.
    pub unit: usize,
    pub pad: f64,
    pub radius: f64,
}

impl Default for GlyphFx {
    fn default() -> GlyphFx {
        GlyphFx {
            xf: Xf::IDENTITY,
            opacity: 1.0,
            fill: None,
            fill_mix: 0.0,
            stroke: None,
            stroke_mix: 0.0,
            stroke_width: 0.0,
            gid: None,
            hidden: false,
            blur: 0.0,
            variation: Vec::new(),
            variation_mix: 0.0,
            highlight: None,
        }
    }
}

/// Background boxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BgMode {
    #[default]
    Block,
    Line,
    Word,
}

/// Paragraph decorations drawn behind the text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Decor {
    pub background: Option<Paint>,
    pub mode: BgMode,
    pub padding: f64,
    pub radius: f64,
}

struct Outline(Path);

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(p(x as f64, y as f64));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(p(x as f64, y as f64));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.segs.push(sr_vector::Seg::Quad(p(x1 as f64, y1 as f64), p(x as f64, y as f64)));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(p(x1 as f64, y1 as f64), p(x2 as f64, y2 as f64), p(x as f64, y as f64));
    }
    fn close(&mut self) {
        self.0.close();
    }
}

fn var_key(v: &[([u8; 4], f32)]) -> u64 {
    v.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, (t, x)| {
        (h ^ (u32::from_be_bytes(*t) as u64 ^ x.to_bits() as u64)).wrapping_mul(0x100_0000_01b3)
    })
}

impl FontLib {
    /// Glyph outline in font units (y up), cached.
    pub fn outline(&self, face: usize, gid: u16, vars: &[([u8; 4], f32)]) -> Arc<Path> {
        let key = (face, gid, var_key(vars));
        if let Some(pth) = self.outlines.lock().unwrap().get(&key) {
            return pth.clone();
        }
        let pth = self
            .with_face(face, vars, |f| {
                let mut o = Outline(Path::default());
                f.outline_glyph(GlyphId(gid), &mut o);
                o.0
            })
            .unwrap_or_default();
        let a = Arc::new(pth);
        self.outlines.lock().unwrap().insert(key, a.clone());
        a
    }

    /// Units per em of a face.
    /// Default value of a variation axis of a face.
    pub fn axis_default(&self, face: usize, tag: [u8; 4]) -> Option<f32> {
        self.with_face(face, &[], |f| {
            f.variation_axes().into_iter().find(|a| a.tag.to_bytes() == tag).map(|a| a.def_value)
        })
        .flatten()
    }

    pub fn upem(&self, face: usize) -> f64 {
        self.with_face(face, &[], |f| f.units_per_em() as f64).unwrap_or(1000.0)
    }
}

fn mix(a: &Paint, b: &Paint, t: f64) -> Paint {
    match (a, b) {
        (Paint::Solid { rgba: x, srgb: s1 }, Paint::Solid { rgba: y, srgb: s2 }) if s1 == s2 => {
            Paint::Solid { rgba: [0, 1, 2, 3].map(|k| x[k] + (y[k] - x[k]) * t), srgb: *s1 }
        }
        _ => {
            if t >= 0.5 {
                b.clone()
            } else {
                a.clone()
            }
        }
    }
}

struct Colr<'s> {
    out: &'s mut Scene,
    lib: &'s FontLib,
    face: usize,
    xf: Vec<Xf>,
    path: Option<Vec<Poly>>,
    clips: Vec<Vec<Poly>>,
    tol: f64,
    opacity: f64,
}

impl Colr<'_> {
    fn top(&self) -> Xf {
        *self.xf.last().unwrap()
    }
    fn stops(it: impl Iterator<Item = colr::ColorStop>) -> Vec<(f64, [f64; 4])> {
        it.map(|s| {
            (
                s.stop_offset as f64,
                [
                    s.color.red as f64 / 255.0,
                    s.color.green as f64 / 255.0,
                    s.color.blue as f64 / 255.0,
                    s.color.alpha as f64 / 255.0,
                ],
            )
        })
        .collect()
    }
    fn spread(e: colr::GradientExtend) -> u32 {
        match e {
            colr::GradientExtend::Pad => 0,
            colr::GradientExtend::Reflect => 1,
            colr::GradientExtend::Repeat => 2,
        }
    }
}

impl<'a> colr::Painter<'a> for Colr<'_> {
    fn outline_glyph(&mut self, g: GlyphId) {
        let o = self.lib.outline(self.face, g.0, &[]);
        self.path = Some(o.transform(&self.top()).flatten(self.tol));
    }
    fn paint(&mut self, paint: colr::Paint<'a>) {
        let inv = self.top().inverse().unwrap_or(Xf::IDENTITY);
        let pa = match paint {
            colr::Paint::Solid(c) => Paint::Solid {
                rgba: [c.red as f64 / 255.0, c.green as f64 / 255.0, c.blue as f64 / 255.0, c.alpha as f64 / 255.0],
                srgb: true,
            },
            colr::Paint::LinearGradient(g) => {
                let (p0, p1, p2) =
                    (p(g.x0 as f64, g.y0 as f64), p(g.x1 as f64, g.y1 as f64), p(g.x2 as f64, g.y2 as f64));
                // the gradient runs along p0→p1 projected onto the normal of p0→p2
                let n = (p2 - p0).perp().norm();
                let d = p1 - p0;
                let p1b = if n.len() > 0.0 { p0 + n * d.dot(n) } else { p1 };
                Paint::Gradient(Box::new(Gradient {
                    kind: GradientKind::Linear { a: p0, b: p1b },
                    stops: Self::stops(g.stops(0, &[])),
                    spread: Self::spread(g.extend),
                    to_gradient: inv,
                }))
            }
            colr::Paint::RadialGradient(g) => Paint::Gradient(Box::new(Gradient {
                kind: GradientKind::Radial {
                    c: p(g.x1 as f64, g.y1 as f64),
                    r: g.r1 as f64,
                    f: p(g.x0 as f64, g.y0 as f64),
                    fr: g.r0 as f64,
                },
                stops: Self::stops(g.stops(0, &[])),
                spread: Self::spread(g.extend),
                to_gradient: inv,
            })),
            colr::Paint::SweepGradient(g) => {
                let (s, e) = (g.start_angle as f64, g.end_angle as f64);
                let stops = Self::stops(g.stops(0, &[]))
                    .into_iter()
                    .map(|(o, c)| (((s + o * (e - s)) / 360.0).rem_euclid(1.0), c))
                    .collect::<Vec<_>>();
                // font space is y-up: counter-clockwise from +x becomes clockwise from +x on screen, 90° after "up"
                let mut stops = stops;
                stops.sort_by(|a, b| a.0.total_cmp(&b.0));
                Paint::Gradient(Box::new(Gradient {
                    kind: GradientKind::Conic { c: p(g.center_x as f64, g.center_y as f64), start: 90.0 },
                    stops,
                    spread: 0,
                    to_gradient: inv,
                }))
            }
        };
        // fill the current clip (or the last outline)
        let area = self.clips.last().cloned().or_else(|| self.path.clone()).unwrap_or_default();
        let b = sr_vector::path::poly_bounds(&area);
        if b.is_empty() {
            return;
        }
        let r = shapes::rect(b.0[0], b.0[1], b.0[2] - b.0[0], b.0[3] - b.0[1], [0.0; 4]).flatten(self.tol);
        self.out.cmds.push(Cmd::Fill { polys: r, rule: FillRule::NonZero, paint: pa, opacity: self.opacity });
    }
    fn push_clip(&mut self) {
        let polys = self.path.clone().unwrap_or_default();
        self.out.cmds.push(Cmd::Push { mask_init: 0.0 });
        self.clips.push(polys);
    }
    fn push_clip_box(&mut self, b: colr::ClipBox) {
        let r = shapes::rect(
            b.x_min as f64,
            b.y_min as f64,
            (b.x_max - b.x_min) as f64,
            (b.y_max - b.y_min) as f64,
            [0.0; 4],
        )
        .transform(&self.top())
        .flatten(self.tol);
        self.out.cmds.push(Cmd::Push { mask_init: 0.0 });
        self.clips.push(r);
    }
    fn pop_clip(&mut self) {
        if let Some(polys) = self.clips.pop() {
            self.out.cmds.push(Cmd::Mask {
                polys,
                rule: FillRule::NonZero,
                op: MaskOp::Add,
                opacity: 1.0,
                invert: false,
            });
            self.out.cmds.push(Cmd::Pop { opacity: 1.0 });
        }
    }
    fn push_layer(&mut self, _mode: colr::CompositeMode) {
        self.out.cmds.push(Cmd::Push { mask_init: 1.0 });
    }
    fn pop_layer(&mut self) {
        self.out.cmds.push(Cmd::Pop { opacity: 1.0 });
    }
    fn push_transform(&mut self, t: ttf_parser::Transform) {
        let x = self.top().mul(&Xf([t.a as f64, t.b as f64, t.c as f64, t.d as f64, t.e as f64, t.f as f64]));
        self.xf.push(x);
    }
    fn pop_transform(&mut self) {
        if self.xf.len() > 1 {
            self.xf.pop();
        }
    }
}

fn hash_bytes(b: &[u8]) -> u64 {
    b.iter()
        .step_by(7)
        .fold(0xcbf2_9ce4_8422_2325u64 ^ b.len() as u64, |h, &x| (h ^ x as u64).wrapping_mul(0x100_0000_01b3))
}

/// Draws a laid-out paragraph. `fx` holds per-glyph effects (same length as the glyphs).
pub fn draw(lib: &FontLib, lay: &Layout, fx: Option<&[GlyphFx]>, decor: &Decor, tol: f64) -> Drawing {
    // blurred glyphs draw apart, one scene per radius bucket (steps of √2)
    let Some(f) = fx.filter(|f| f.iter().any(|e| e.blur >= 0.5 && !e.hidden)) else {
        return draw_glyphs(lib, lay, fx, decor, tol);
    };
    let bucket = |b: f64| (b.log2() * 2.0).round() as i32;
    let mut main: Vec<GlyphFx> = f.to_vec();
    let mut buckets: Vec<i32> = Vec::new();
    for e in main.iter_mut() {
        if e.blur >= 0.5 && !e.hidden {
            let k = bucket(e.blur);
            if !buckets.contains(&k) {
                buckets.push(k);
            }
            e.hidden = true;
        }
    }
    let mut d = draw_glyphs(lib, lay, Some(&main), decor, tol);
    for k in buckets {
        let only: Vec<GlyphFx> = f
            .iter()
            .map(|e| {
                let mut e = e.clone();
                e.hidden = e.hidden || e.blur < 0.5 || bucket(e.blur) != k;
                e
            })
            .collect();
        let part = draw_glyphs(lib, lay, Some(&only), &Decor::default(), tol);
        d.blurred.push((libm::exp2(k as f64 / 2.0), part.scene));
        d.bitmaps.extend(part.bitmaps);
    }
    d
}

fn draw_glyphs(lib: &FontLib, lay: &Layout, fx: Option<&[GlyphFx]>, decor: &Decor, tol: f64) -> Drawing {
    let mut d = Drawing::default();
    let s = &mut d.scene;
    let rrect = |r: [f64; 4], pad: f64, rad: f64| {
        shapes::rect(r[0] - pad, r[1] - pad, r[2] + 2.0 * pad, r[3] + 2.0 * pad, [rad; 4])
    };
    if lay.clip {
        s.cmds.push(Cmd::Push { mask_init: 0.0 });
    }
    // backgrounds
    if let Some(bg) = &decor.background {
        let rects: Vec<[f64; 4]> = match decor.mode {
            BgMode::Block => {
                let mut r = Rect::EMPTY;
                for l in &lay.lines {
                    r.add(p(l.rect[0], l.rect[1]));
                    r.add(p(l.rect[0] + l.rect[2], l.rect[1] + l.rect[3]));
                }
                if r.is_empty() {
                    Vec::new()
                } else {
                    vec![[r.0[0], r.0[1], r.0[2] - r.0[0], r.0[3] - r.0[1]]]
                }
            }
            BgMode::Line => lay.lines.iter().map(|l| l.rect).collect(),
            BgMode::Word => lay.words.iter().map(|w| w.0).collect(),
        };
        for r in rects {
            s.fill(&rrect(r, decor.padding, decor.radius), FillRule::NonZero, bg.clone(), 1.0, tol);
        }
    }
    let fxs = |i: usize| -> std::borrow::Cow<GlyphFx> {
        match fx.and_then(|f| f.get(i)) {
            Some(x) => std::borrow::Cow::Borrowed(x),
            None => std::borrow::Cow::Owned(GlyphFx::default()),
        }
    };
    // highlights, per line and style run
    for l in &lay.lines {
        let mut k = l.glyphs.start;
        while k < l.glyphs.end {
            let st = &lay.styles[lay.glyphs[k].style];
            let Some(hl) = &st.highlight else {
                k += 1;
                continue;
            };
            let mut j = k;
            while j < l.glyphs.end && lay.glyphs[j].style == lay.glyphs[k].style {
                j += 1;
            }
            let (x0, x1) = (lay.glyphs[k].x, lay.glyphs[j - 1].x + lay.glyphs[j - 1].advance);
            s.fill(
                &shapes::rect(x0.min(x1), l.rect[1], (x1 - x0).abs(), l.rect[3], [0.0; 4]),
                FillRule::NonZero,
                hl.clone(),
                1.0,
                tol,
            );
            k = j;
        }
    }
    // animator highlight boxes: each unit's extent on its line, wiped in from its start
    if let Some(fx) = fx {
        for l in &lay.lines {
            let mut k = l.glyphs.start;
            while k < l.glyphs.end {
                let Some(h) = fx.get(k).and_then(|f| f.highlight.as_ref()) else {
                    k += 1;
                    continue;
                };
                let mut j = k;
                while j < l.glyphs.end && fx.get(j).and_then(|f| f.highlight.as_ref()).is_some_and(|o| o.unit == h.unit)
                {
                    j += 1;
                }
                let (a, b) = (&lay.glyphs[k], &lay.glyphs[j - 1]);
                let (x0, x1) = (a.x.min(b.x), (a.x + a.advance).max(b.x + b.advance));
                let w = (x1 - x0 + 2.0 * h.pad) * h.fraction.min(1.0);
                // right to left: from the right
                let hx = if a.x > b.x { x1 + h.pad - w } else { x0 - h.pad };
                let r = [hx, l.rect[1] - h.pad * 0.5, w, l.rect[3] + h.pad];
                let path = rrect(r, 0.0, h.radius).transform(&fx[k].xf);
                s.fill(&path, FillRule::NonZero, h.paint.clone(), 1.0, tol);
                k = j;
            }
        }
    }
    // glyph placement
    struct G {
        path: Option<Path>,
        colr: bool,
        style: usize,
        opacity: f64,
        fill: Option<Paint>,
        stroke: Option<(Paint, f64, StrokePos)>,
    }
    let mut gs: Vec<G> = Vec::with_capacity(lay.glyphs.len());
    for (i, g) in lay.glyphs.iter().enumerate() {
        let e = fxs(i);
        let st = &lay.styles[g.style];
        if e.hidden || e.opacity <= 0.0 || lay.chars.get(g.ch).is_some_and(|c| c.is_whitespace()) {
            gs.push(G { path: None, colr: false, style: g.style, opacity: 0.0, fill: None, stroke: None });
            continue;
        }
        let gid = e.gid.unwrap_or(g.gid);
        let upem = lib.upem(g.face);
        let k = g.size / upem;
        let xf = e.xf.mul(&Xf([k, 0.0, 0.0, -k, g.x, g.y]));
        let fd = lib.face(g.face);
        let mut colr = false;
        if fd.color {
            let handled = lib
                .with_face(g.face, &[], |f| {
                    if f.is_color_glyph(GlyphId(gid)) {
                        let mut c = Colr {
                            out: &mut *s,
                            lib,
                            face: g.face,
                            xf: vec![xf],
                            path: None,
                            clips: Vec::new(),
                            tol,
                            opacity: e.opacity,
                        };
                        let fg = ttf_parser::RgbaColor::new(255, 255, 255, 255);
                        f.paint_color_glyph(GlyphId(gid), 0, fg, &mut c).is_some()
                    } else if let Some(img) = f.glyph_raster_image(GlyphId(gid), (g.size.round() as u16).max(1)) {
                        if img.format == ttf_parser::RasterImageFormat::PNG {
                            let sc = g.size / img.pixels_per_em.max(1) as f64;
                            d.bitmaps.push(Bitmap {
                                png: Arc::new(img.data.to_vec()),
                                key: hash_bytes(img.data),
                                rect: [
                                    g.x + img.x as f64 * sc,
                                    g.y - (img.y as f64 + img.height as f64) * sc,
                                    img.width as f64 * sc,
                                    img.height as f64 * sc,
                                ],
                                xf: e.xf,
                                opacity: e.opacity,
                                uv: [0.0, 0.0, 1.0, 1.0],
                                below: false,
                            });
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                })
                .unwrap_or(false);
            colr = handled;
        }
        let vars: std::borrow::Cow<[([u8; 4], f32)]> = if e.variation.is_empty() || e.variation_mix <= 0.0 {
            std::borrow::Cow::Borrowed(&st.variations)
        } else {
            let mut v = st.variations.clone();
            for (tag, target) in &e.variation {
                let base = v
                    .iter()
                    .find(|x| x.0 == *tag)
                    .map(|x| x.1)
                    .or_else(|| lib.axis_default(g.face, *tag))
                    .unwrap_or(*target);
                let val = base + (target - base) * e.variation_mix as f32;
                match v.iter_mut().find(|x| x.0 == *tag) {
                    Some(x) => x.1 = val,
                    None => v.push((*tag, val)),
                }
            }
            std::borrow::Cow::Owned(v)
        };
        let path = (!colr).then(|| lib.outline(g.face, gid, &vars).transform(&xf));
        let fill = match (&st.color, &e.fill) {
            (Some(a), Some(b)) => Some(mix(a, b, e.fill_mix)),
            (None, Some(b)) if e.fill_mix >= 0.5 => Some(b.clone()),
            (a, _) => a.clone(),
        };
        let stroke = match (&st.stroke, &e.stroke) {
            (Some((a, w, pos)), Some(b)) => Some((mix(a, b, e.stroke_mix), w + e.stroke_width, *pos)),
            (Some((a, w, pos)), None) => Some((a.clone(), w + e.stroke_width, *pos)),
            (None, Some(b)) if e.stroke_width > 0.0 || e.stroke_mix > 0.0 => {
                Some((b.clone(), e.stroke_width.max(1.0), StrokePos::Center))
            }
            _ => None,
        };
        gs.push(G { path, colr, style: g.style, opacity: e.opacity, fill, stroke });
    }
    // shadows
    for g in &gs {
        let (Some(pth), Some((c, dx, dy, blur))) = (&g.path, &lay.styles[g.style].shadow) else { continue };
        let base = pth.flatten(tol);
        let offsets: Vec<(f64, f64, f64)> = if *blur > 0.5 {
            let r = blur * 0.5;
            let mut v = vec![(0.0, 0.0, 0.4)];
            v.extend((0..8).map(|k| {
                let a = k as f64 * std::f64::consts::FRAC_PI_4;
                (r * libm::cos(a), r * libm::sin(a), 0.075)
            }));
            v
        } else {
            vec![(0.0, 0.0, 1.0)]
        };
        for (ox, oy, w) in offsets {
            let polys = base
                .iter()
                .map(|q| Poly { pts: q.pts.iter().map(|&pt| pt + p(dx + ox, dy + oy)).collect(), closed: true })
                .collect();
            s.cmds.push(Cmd::Fill {
                polys,
                rule: FillRule::NonZero,
                paint: Paint::Solid { rgba: *c, srgb: false },
                opacity: g.opacity * w,
            });
        }
    }
    let stroke_of = |pth: &Path, w: f64| -> Vec<Poly> {
        let polys = pth.flatten(tol);
        stroke::stroke(&polys, &StrokeStyle { width: w, cap: Cap::Butt, join: Join::Round, miter_limit: 4.0 }, tol)
    };
    // outside strokes sit behind the fill
    for g in &gs {
        if let (Some(pth), Some((sp, w, StrokePos::Outside))) = (&g.path, &g.stroke) {
            s.cmds.push(Cmd::Fill {
                polys: stroke_of(pth, w * 2.0),
                rule: FillRule::NonZero,
                paint: sp.clone(),
                opacity: g.opacity,
            });
        }
    }
    // fills, merged across consecutive glyphs with the same paint and opacity
    let mut k = 0;
    while k < gs.len() {
        let (Some(_), Some(fp)) = (&gs[k].path, &gs[k].fill) else {
            k += 1;
            continue;
        };
        let mut polys: Vec<Poly> = Vec::new();
        let mut j = k;
        while j < gs.len() && gs[j].path.is_some() && gs[j].fill.as_ref() == Some(fp) && gs[j].opacity == gs[k].opacity
        {
            polys.extend(gs[j].path.as_ref().unwrap().flatten(tol));
            j += 1;
        }
        s.cmds.push(Cmd::Fill { polys, rule: FillRule::NonZero, paint: fp.clone(), opacity: gs[k].opacity });
        k = j.max(k + 1);
    }
    // centre and inside strokes over the fill
    for g in &gs {
        if let (Some(pth), Some((sp, w, pos))) = (&g.path, &g.stroke) {
            match pos {
                StrokePos::Center => s.cmds.push(Cmd::Fill {
                    polys: stroke_of(pth, *w),
                    rule: FillRule::NonZero,
                    paint: sp.clone(),
                    opacity: g.opacity,
                }),
                StrokePos::Inside => {
                    s.cmds.push(Cmd::Push { mask_init: 0.0 });
                    s.cmds.push(Cmd::Fill {
                        polys: stroke_of(pth, w * 2.0),
                        rule: FillRule::NonZero,
                        paint: sp.clone(),
                        opacity: g.opacity,
                    });
                    s.cmds.push(Cmd::Mask {
                        polys: pth.flatten(tol),
                        rule: FillRule::NonZero,
                        op: MaskOp::Add,
                        opacity: 1.0,
                        invert: false,
                    });
                    s.cmds.push(Cmd::Pop { opacity: 1.0 });
                }
                StrokePos::Outside => {}
            }
        }
    }
    // decorations
    for l in &lay.lines {
        let mut k = l.glyphs.start;
        while k < l.glyphs.end {
            let g0 = &lay.glyphs[k];
            let st = &lay.styles[g0.style];
            if st.decoration == Decoration::None || gs[k].colr {
                k += 1;
                continue;
            }
            let mut j = k;
            while j < l.glyphs.end && lay.glyphs[j].style == g0.style {
                j += 1;
            }
            let (x0, x1) = (lay.glyphs[k].x, lay.glyphs[j - 1].x + lay.glyphs[j - 1].advance);
            let (pos, th) = lib
                .with_face(g0.face, &[], |f| {
                    let sc = g0.size / f.units_per_em() as f64;
                    let m = match st.decoration {
                        Decoration::Underline => f.underline_metrics().map(|m| (m.position as f64, m.thickness as f64)),
                        Decoration::LineThrough => {
                            f.strikeout_metrics().map(|m| (m.position as f64, m.thickness as f64))
                        }
                        _ => Some((
                            f.ascender() as f64,
                            f.underline_metrics().map(|m| m.thickness as f64).unwrap_or(50.0),
                        )),
                    };
                    let (pp, t) = m.unwrap_or((-(f.units_per_em() as f64) * 0.1, f.units_per_em() as f64 * 0.05));
                    (pp * sc, t * sc)
                })
                .unwrap_or((-g0.size * 0.1, g0.size * 0.05));
            let y = l.baseline - pos - th * 0.5;
            if let Some(c) = &gs[k].fill {
                let r = shapes::rect(x0.min(x1), y, (x1 - x0).abs(), th.max(0.5), [0.0; 4]).transform(&fxs(k).xf);
                s.fill(&r, FillRule::NonZero, c.clone(), gs[k].opacity, tol);
            }
            k = j;
        }
    }
    if lay.clip {
        let [w, h] = lay.size;
        let r =
            shapes::rect(0.0, 0.0, if w.is_finite() { w } else { 1e6 }, if h.is_finite() { h } else { 1e6 }, [0.0; 4])
                .flatten(tol);
        s.cmds.push(Cmd::Mask { polys: r, rule: FillRule::NonZero, op: MaskOp::Add, opacity: 1.0, invert: false });
        s.cmds.push(Cmd::Pop { opacity: 1.0 });
    }
    d
}
