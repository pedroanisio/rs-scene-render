//! Vector scenes: an ordered list of fills and layer operations, the
//! common output of shapes, SVG and Lottie and the input of the tile
//! encoder.

use crate::geom::{Rect, Xf, P};
use crate::path::{poly_bounds, Path, Poly};

/// Fill rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// How a mask or merged path combines with the coverage before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskOp {
    Add,
    Subtract,
    Intersect,
    Difference,
    Lighten,
    Darken,
}

/// Track matte modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatteMode {
    Alpha,
    AlphaInverted,
    Luma,
    LumaInverted,
}

/// Gradient geometry in gradient space.
#[derive(Debug, Clone, PartialEq)]
pub enum GradientKind {
    Linear {
        a: P,
        b: P,
    },
    Radial {
        c: P,
        r: f64,
        f: P,
        fr: f64,
    },
    /// Sweep around `c`, 0 at `start` degrees clockwise from up (the compositor's conic gradient).
    Conic {
        c: P,
        start: f64,
    },
}

/// A gradient defined by an import (SVG or Lottie).
#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    pub kind: GradientKind,
    /// (offset, straight RGBA).
    pub stops: Vec<(f64, [f64; 4])>,
    /// 0 pad, 1 reflect, 2 repeat.
    pub spread: u32,
    /// Scene space → gradient space.
    pub to_gradient: Xf,
}

/// A paint.
#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    /// Straight RGBA; `srgb` marks imported sRGB colours (converted to the working space by the renderer).
    Solid { rgba: [f64; 4], srgb: bool },
    /// Imported gradient (sRGB stops).
    Gradient(Box<Gradient>),
    /// A document paint (the renderer's paint table index) and scene → paint-local transform.
    External { index: u32, to_local: Xf },
}

/// A scene command.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    /// Fills polygons with a paint.
    Fill { polys: Vec<Poly>, rule: FillRule, paint: Paint, opacity: f64 },
    /// Begins a layer; its mask coverage starts at `mask_init` (0 or 1).
    Push { mask_init: f64 },
    /// Combines a path into the current layer's mask (between its content and its `Pop`).
    Mask { polys: Vec<Poly>, rule: FillRule, op: MaskOp, opacity: f64, invert: bool },
    /// Ends a layer, compositing it over its parent.
    Pop { opacity: f64 },
    /// Begins the matte of the current layer.
    PushMatte,
    /// Ends the matte and its layer: the layer is multiplied by the matte, then composited.
    PopMatte { mode: MatteMode, opacity: f64 },
}

/// An ordered vector scene.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub cmds: Vec<Cmd>,
}

impl Scene {
    /// Fills a path, flattened within `tol`.
    pub fn fill(&mut self, path: &Path, rule: FillRule, paint: Paint, opacity: f64, tol: f64) {
        let polys = path.flatten(tol);
        if !polys.is_empty() && opacity > 0.0 {
            self.cmds.push(Cmd::Fill { polys, rule, paint, opacity });
        }
    }

    /// Transformed copy (polygons, gradient and paint spaces).
    pub fn transformed(&self, x: &Xf) -> Scene {
        let inv = x.inverse().unwrap_or(Xf::IDENTITY);
        let tp = |ps: &[Poly]| -> Vec<Poly> {
            ps.iter().map(|q| Poly { pts: q.pts.iter().map(|&pt| x.apply(pt)).collect(), closed: q.closed }).collect()
        };
        let paint = |pa: &Paint| -> Paint {
            match pa {
                Paint::Solid { .. } => pa.clone(),
                Paint::Gradient(g) => {
                    Paint::Gradient(Box::new(Gradient { to_gradient: g.to_gradient.mul(&inv), ..(**g).clone() }))
                }
                Paint::External { index, to_local } => Paint::External { index: *index, to_local: to_local.mul(&inv) },
            }
        };
        Scene {
            cmds: self
                .cmds
                .iter()
                .map(|c| match c {
                    Cmd::Fill { polys, rule, paint: pa, opacity } => {
                        Cmd::Fill { polys: tp(polys), rule: *rule, paint: paint(pa), opacity: *opacity }
                    }
                    Cmd::Mask { polys, rule, op, opacity, invert } => {
                        Cmd::Mask { polys: tp(polys), rule: *rule, op: *op, opacity: *opacity, invert: *invert }
                    }
                    other => other.clone(),
                })
                .collect(),
        }
    }

    /// Maps every point through `f` (deformers).
    pub fn map_points(&mut self, f: &dyn Fn(P) -> P) {
        for c in &mut self.cmds {
            if let Cmd::Fill { polys, .. } | Cmd::Mask { polys, .. } = c {
                for q in polys {
                    for pt in &mut q.pts {
                        *pt = f(*pt);
                    }
                }
            }
        }
    }

    /// Splits long edges so point maps bend them smoothly.
    pub fn subdivide(&mut self, max: f64) {
        for c in &mut self.cmds {
            if let Cmd::Fill { polys, .. } | Cmd::Mask { polys, .. } = c {
                for q in polys.iter_mut() {
                    *q = q.subdivide(max);
                }
            }
        }
    }

    /// Bounds of everything drawn.
    pub fn bounds(&self) -> Rect {
        let mut r = Rect::EMPTY;
        for c in &self.cmds {
            if let Cmd::Fill { polys, .. } = c {
                r = r.union(&poly_bounds(polys));
            }
        }
        r
    }

    /// Appends another scene.
    pub fn extend(&mut self, o: Scene) {
        self.cmds.extend(o.cmds);
    }
}
