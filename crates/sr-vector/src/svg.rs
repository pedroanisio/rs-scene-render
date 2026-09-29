//! SVG import through `usvg`: the normalised tree becomes a [`Scene`] in
//! SVG user units. Groups with opacity below one, clip paths or masks
//! become layers; clip paths become intersecting masks and `<mask>`
//! becomes a luminance or alpha matte. Colours are sRGB. Text is
//! not imported; images, patterns and filters are reported and skipped.

use crate::geom::{p, Xf};
use crate::path::{Path, Poly};
use crate::scene::{Cmd, FillRule, Gradient, GradientKind, MaskOp, MatteMode, Paint, Scene};
use crate::stroke::{self, Cap, Join, Style};

/// An imported SVG.
#[derive(Debug, Clone)]
pub struct Svg {
    /// Scene in SVG user units (the viewBox mapped to `size`).
    pub scene: Scene,
    /// Intrinsic size.
    pub size: [f64; 2],
    /// Features that were skipped.
    pub skipped: Vec<String>,
}

fn xf(t: usvg::Transform) -> Xf {
    Xf([t.sx as f64, t.ky as f64, t.kx as f64, t.sy as f64, t.tx as f64, t.ty as f64])
}

fn path_of(d: &usvg::tiny_skia_path::Path, x: &Xf) -> Path {
    let mut out = Path::default();
    let pt = |q: usvg::tiny_skia_path::Point| x.apply(p(q.x as f64, q.y as f64));
    for s in d.segments() {
        use usvg::tiny_skia_path::PathSegment as S;
        match s {
            S::MoveTo(a) => out.move_to(pt(a)),
            S::LineTo(a) => out.line_to(pt(a)),
            S::QuadTo(a, b) => out.segs.push(crate::path::Seg::Quad(pt(a), pt(b))),
            S::CubicTo(a, b, c) => out.cubic_to(pt(a), pt(b), pt(c)),
            S::Close => out.close(),
        }
    }
    out
}

struct Ctx {
    tol: f64,
    /// Extra transform for content defined relative to a referencing element (mask content).
    extra: Xf,
    skipped: Vec<String>,
}

impl Ctx {
    fn paint(&mut self, pa: &usvg::Paint, opacity: f64, x: &Xf) -> Option<(Paint, f64)> {
        let stops = |g: &usvg::BaseGradient| -> Vec<(f64, [f64; 4])> {
            g.stops()
                .iter()
                .map(|s| {
                    let c = s.color();
                    (
                        s.offset().get() as f64,
                        [c.red as f64 / 255.0, c.green as f64 / 255.0, c.blue as f64 / 255.0, s.opacity().get() as f64],
                    )
                })
                .collect()
        };
        let spread = |g: &usvg::BaseGradient| match g.spread_method() {
            usvg::SpreadMethod::Pad => 0,
            usvg::SpreadMethod::Reflect => 1,
            usvg::SpreadMethod::Repeat => 2,
        };
        match pa {
            usvg::Paint::Color(c) => Some((
                Paint::Solid {
                    rgba: [c.red as f64 / 255.0, c.green as f64 / 255.0, c.blue as f64 / 255.0, 1.0],
                    srgb: true,
                },
                opacity,
            )),
            usvg::Paint::LinearGradient(g) => {
                let to_g = x.mul(&xf(g.transform())).inverse()?;
                Some((
                    Paint::Gradient(Box::new(Gradient {
                        kind: GradientKind::Linear {
                            a: p(g.x1() as f64, g.y1() as f64),
                            b: p(g.x2() as f64, g.y2() as f64),
                        },
                        stops: stops(g),
                        spread: spread(g),
                        to_gradient: to_g,
                    })),
                    opacity,
                ))
            }
            usvg::Paint::RadialGradient(g) => {
                let to_g = x.mul(&xf(g.transform())).inverse()?;
                Some((
                    Paint::Gradient(Box::new(Gradient {
                        kind: GradientKind::Radial {
                            c: p(g.cx() as f64, g.cy() as f64),
                            r: g.r().get() as f64,
                            f: p(g.fx() as f64, g.fy() as f64),
                            fr: g.fr().get() as f64,
                        },
                        stops: stops(g),
                        spread: spread(g),
                        to_gradient: to_g,
                    })),
                    opacity,
                ))
            }
            usvg::Paint::Pattern(pt) => {
                self.skipped.push(format!("pattern paint {:?} (drawn with its first colour)", pt.id()));
                None
            }
        }
    }

    fn polys(&self, path: &Path) -> Vec<Poly> {
        path.flatten(self.tol)
    }

    fn node(&mut self, n: &usvg::Node, out: &mut Scene) {
        match n {
            usvg::Node::Group(g) => self.group(g, out),
            usvg::Node::Path(pa) => {
                if !pa.is_visible() {
                    return;
                }
                let x = self.extra.mul(&xf(pa.abs_transform()));
                let path = path_of(pa.data(), &x);
                let draw_fill = |s: &mut Self, out: &mut Scene| {
                    if let Some(f) = pa.fill() {
                        if let Some((paint, op)) = s.paint(f.paint(), f.opacity().get() as f64, &x) {
                            let rule =
                                if f.rule() == usvg::FillRule::EvenOdd { FillRule::EvenOdd } else { FillRule::NonZero };
                            out.fill(&path, rule, paint, op, s.tol);
                        }
                    }
                };
                let draw_stroke = |s: &mut Self, out: &mut Scene| {
                    if let Some(st) = pa.stroke() {
                        if let Some((paint, op)) = s.paint(st.paint(), st.opacity().get() as f64, &x) {
                            // stroke in user space, then transformed: non-uniform scale deforms the pen like SVG
                            let local = path_of(pa.data(), &Xf::IDENTITY);
                            let scale = x.max_scale().max(1e-9);
                            let mut polys = local.flatten(s.tol / scale);
                            if let Some(d) = st.dasharray() {
                                let pat: Vec<f64> = d.iter().map(|v| *v as f64).collect();
                                polys = crate::measure::dash(&polys, &pat, st.dashoffset() as f64);
                            }
                            let style = Style {
                                width: st.width().get() as f64,
                                cap: match st.linecap() {
                                    usvg::LineCap::Butt => Cap::Butt,
                                    usvg::LineCap::Round => Cap::Round,
                                    usvg::LineCap::Square => Cap::Square,
                                },
                                join: match st.linejoin() {
                                    usvg::LineJoin::Round => Join::Round,
                                    usvg::LineJoin::Bevel => Join::Bevel,
                                    _ => Join::Miter,
                                },
                                miter_limit: st.miterlimit().get() as f64,
                            };
                            let outline: Vec<Poly> = stroke::stroke(&polys, &style, s.tol / scale)
                                .into_iter()
                                .map(|q| Poly { pts: q.pts.iter().map(|&pt| x.apply(pt)).collect(), closed: true })
                                .collect();
                            if !outline.is_empty() && op > 0.0 {
                                out.cmds.push(Cmd::Fill {
                                    polys: outline,
                                    rule: FillRule::NonZero,
                                    paint,
                                    opacity: op,
                                });
                            }
                        }
                    }
                };
                if pa.paint_order() == usvg::PaintOrder::StrokeAndFill {
                    draw_stroke(self, out);
                    draw_fill(self, out);
                } else {
                    draw_fill(self, out);
                    draw_stroke(self, out);
                }
            }
            usvg::Node::Image(_) => self.skipped.push("embedded image".into()),
            usvg::Node::Text(_) => self.skipped.push("text".into()),
        }
    }

    fn clip_polys(&mut self, cp: &usvg::ClipPath, owner: &Xf, out: &mut Vec<(Vec<Poly>, FillRule)>) {
        fn walk(c: &mut Ctx, g: &usvg::Group, extra: &Xf, out: &mut Vec<(Vec<Poly>, FillRule)>) {
            for n in g.children() {
                match n {
                    usvg::Node::Path(pa) => {
                        let x = extra.mul(&xf(pa.abs_transform()));
                        let rule = if pa.fill().is_some_and(|f| f.rule() == usvg::FillRule::EvenOdd) {
                            FillRule::EvenOdd
                        } else {
                            FillRule::NonZero
                        };
                        out.push((c.polys(&path_of(pa.data(), &x)), rule));
                    }
                    usvg::Node::Group(gg) => walk(c, gg, extra, out),
                    _ => {}
                }
            }
        }
        // clip content lives in the referencing element's user space
        let base = owner.mul(&xf(cp.transform()));
        walk(self, cp.root(), &base, out);
        if cp.clip_path().is_some() {
            self.skipped.push("nested clip-path (outer clip only)".into());
        }
    }

    fn group(&mut self, g: &usvg::Group, out: &mut Scene) {
        if !g.filters().is_empty() {
            self.skipped.push(format!("filter on group {:?}", g.id()));
        }
        if g.blend_mode() != usvg::BlendMode::Normal {
            self.skipped.push(format!("mix-blend-mode on group {:?} (drawn normal)", g.id()));
        }
        let opacity = g.opacity().get() as f64;
        let layered = opacity < 1.0 || g.clip_path().is_some() || g.mask().is_some();
        if !layered {
            for c in g.children() {
                self.node(c, out);
            }
            return;
        }
        out.cmds.push(Cmd::Push { mask_init: if g.clip_path().is_some() { 0.0 } else { 1.0 } });
        for c in g.children() {
            self.node(c, out);
        }
        if let Some(cp) = g.clip_path() {
            let mut polys = Vec::new();
            self.clip_polys(cp, &xf(g.abs_transform()), &mut polys);
            for (ps, rule) in polys {
                out.cmds.push(Cmd::Mask { polys: ps, rule, op: MaskOp::Add, opacity: 1.0, invert: false });
            }
        }
        match g.mask() {
            Some(m) => {
                out.cmds.push(Cmd::PushMatte);
                let saved = self.extra;
                self.extra = saved.mul(&xf(g.abs_transform()));
                self.group(m.root(), out);
                self.extra = saved;
                let mode = if m.kind() == usvg::MaskType::Alpha { MatteMode::Alpha } else { MatteMode::Luma };
                out.cmds.push(Cmd::PopMatte { mode, opacity });
            }
            None => out.cmds.push(Cmd::Pop { opacity }),
        }
    }
}

/// Parses SVG bytes into a scene; `tol` is the flattening tolerance in user units.
pub fn load(data: &[u8], tol: f64) -> Result<Svg, String> {
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(data, &opt).map_err(|e| e.to_string())?;
    let mut ctx = Ctx { tol: tol.max(1e-4), extra: Xf::IDENTITY, skipped: Vec::new() };
    let mut scene = Scene::default();
    ctx.group(tree.root(), &mut scene);
    ctx.skipped.sort();
    ctx.skipped.dedup();
    Ok(Svg { scene, size: [tree.size().width() as f64, tree.size().height() as f64], skipped: ctx.skipped })
}
