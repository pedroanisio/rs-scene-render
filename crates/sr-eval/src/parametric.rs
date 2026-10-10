//! SREP 70: geometry defined by expressions. A `shape="parametric"` takes its outline from a `parametricPath`, the
//! points (x(t), y(t)); an `object3D` with `primitive="parametric"` or `"heightfield"` takes its mesh from a
//! `parametricSurface`, (x, y, z)(u, v), or a `heightfield`, y = −height(x, z).
//!
//! The expressions are those of `expressionType`, with the sampling variables (`t`; `u` and `v`; `x` and `z`),
//! evaluated in 64-bit floats at each frame, so one that reads `time`, `prop` or `param` animates the geometry.
//! A curve becomes the path data of the shape (the `path` property, drawn exactly as `shape="path"` draws it); a
//! surface becomes a mesh ([`sr_3d::prim::parametric_grid`]) on the frame node.

use std::fmt::Write as _;
use std::sync::Arc;

use sr_model::element::{children, AttrValue, Element};

use crate::eval::FrameNode;
use crate::value::Value;

/// What a parametric definition samples, with its attributes (the schema's defaults where absent).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GeomKind {
    /// `parametricPath`: `samples` values of t over [t0, t1] (one step short of t1 when closed).
    Path { t0: f64, t1: f64, samples: usize, closed: bool },
    /// `parametricSurface`: `n[0]` values of u over `u`, `n[1]` of v over `v`.
    Surface { u: [f64; 2], v: [f64; 2], n: [usize; 2], closed: [bool; 2] },
    /// `heightfield`: `nx` values of x over [−width/2, width/2], `nz` of z over [−depth/2, depth/2].
    Height { width: f64, depth: f64, nx: usize, nz: usize },
}

/// A node's parametric geometry: its sampling and its compiled expressions (indices into `Program::exprs`), in
/// the order of [`Def::sources`].
#[derive(Debug, Clone)]
pub struct Geom {
    pub kind: GeomKind,
    pub exprs: Vec<u32>,
}

/// A parametric definition read from a node's element, before its expressions are compiled.
pub(crate) struct Def {
    pub kind: GeomKind,
    /// `(attribute, source)` of each expression: x and y; x, y and z; or height.
    pub sources: Vec<(&'static str, String)>,
    /// The sampling variables the expressions read ([`crate::expr::vm::Var::Sample`] order).
    pub names: &'static [&'static str],
    /// The child element that holds the definition.
    pub child: &'static str,
}

/// A frame's mesh of a parametric surface or heightfield, in the object's local space.
#[derive(Debug)]
pub struct ParamMesh {
    pub mesh: sr_3d::Primitive,
    /// A key of the mesh's content: equal meshes have equal keys, so a renderer can keep one upload.
    pub key: u64,
}

fn num(e: &dyn Element, n: &str, d: f64) -> f64 {
    match e.get_attr(n) {
        Some(AttrValue::Num(v)) => v,
        _ => d,
    }
}

fn flag(e: &dyn Element, n: &str) -> bool {
    match e.get_attr(n) {
        Some(AttrValue::Bool(b)) => b,
        Some(AttrValue::Str(s)) => s == "true" || s == "1",
        _ => false,
    }
}

fn text(e: &dyn Element, n: &str) -> Option<String> {
    match e.get_attr(n)? {
        AttrValue::Str(s) => Some(s),
        other => Some(other.to_string()),
    }
}

fn count(e: &dyn Element, n: &str, d: f64, max: f64) -> usize {
    num(e, n, d).clamp(2.0, max) as usize
}

/// The parametric definition of `e`, a `shape` or `object3D` element, when it has one (PAR1 and PAR2 make sure
/// a parametric shape or primitive has exactly one).
pub(crate) fn definition(e: &dyn Element) -> Option<Def> {
    let want = match (e.element_name(), text(e, "shape").as_deref(), text(e, "primitive").as_deref()) {
        ("shape", Some("parametric"), _) => "parametricPath",
        ("object3D", _, Some("parametric")) => "parametricSurface",
        ("object3D", _, Some("heightfield")) => "heightfield",
        _ => return None,
    };
    let c = children(e).into_iter().find(|c| c.element_name() == want)?;
    let src = |a: &'static str| (a, text(c, a).unwrap_or_default());
    Some(match want {
        "parametricPath" => Def {
            kind: GeomKind::Path {
                t0: num(c, "t0", 0.0),
                t1: num(c, "t1", 1.0),
                samples: count(c, "samples", 256.0, 1_000_000.0),
                closed: flag(c, "closed"),
            },
            sources: vec![src("x"), src("y")],
            names: &["t"],
            child: want,
        },
        "parametricSurface" => Def {
            kind: GeomKind::Surface {
                u: [num(c, "u0", 0.0), num(c, "u1", 1.0)],
                v: [num(c, "v0", 0.0), num(c, "v1", 1.0)],
                n: [count(c, "uSamples", 64.0, 4096.0), count(c, "vSamples", 64.0, 4096.0)],
                closed: [flag(c, "closedU"), flag(c, "closedV")],
            },
            sources: vec![src("x"), src("y"), src("z")],
            names: &["u", "v"],
            child: want,
        },
        _ => Def {
            kind: GeomKind::Height {
                width: num(c, "width", 1.0),
                depth: num(c, "depth", 1.0),
                nx: count(c, "xSamples", 64.0, 4096.0),
                nz: count(c, "zSamples", 64.0, 4096.0),
            },
            sources: vec![src("height")],
            names: &["x", "z"],
            child: want,
        },
    })
}

/// The `k`-th of `n` samples over [a, b]: the last is b on an open range; a closed one stops a step short of b.
pub fn sample_at(a: f64, b: f64, n: usize, closed: bool, k: usize) -> f64 {
    let steps = if closed { n } else { n - 1 };
    a + k as f64 * (b - a) / steps as f64
}

/// The outline of a sampled curve as path data: `M p0 L p1 …`, closed by `Z` when `closed` and unbroken. A
/// non-finite point is dropped and breaks the path: the next finite point starts a new subpath. Also returns the
/// bounding box of the finite points `[x0, y0, x1, y1]`, `None` when there are none.
pub fn path_data(points: &[[f64; 2]], closed: bool) -> (String, Option<[f64; 4]>) {
    let mut d = String::new();
    let mut bbox: Option<[f64; 4]> = None;
    let mut open = false;
    let mut broken = false;
    for &[x, y] in points {
        if !(x.is_finite() && y.is_finite()) {
            broken |= bbox.is_some();
            open = false;
            continue;
        }
        if !d.is_empty() {
            d.push(' ');
        }
        let _ = write!(d, "{} {x} {y}", if open { 'L' } else { 'M' });
        open = true;
        bbox = Some(match bbox {
            None => [x, y, x, y],
            Some([a, b, c, e]) => [a.min(x), b.min(y), c.max(x), e.max(y)],
        });
    }
    // a break after the last finite point also leaves the curve open
    broken |= points.last().is_some_and(|p| !(p[0].is_finite() && p[1].is_finite()));
    if closed && !broken && bbox.is_some() {
        d.push_str(" Z");
    }
    (d, bbox)
}

/// The sampling variables of every sample of `kind`, in the order the geometry reads them: t for a curve; (u, v)
/// for a surface and (x, z) for a heightfield, grid index `i + j * nu` with i along the first direction.
pub fn samples(kind: &GeomKind) -> Vec<[f64; 2]> {
    match *kind {
        GeomKind::Path { t0, t1, samples, closed } => {
            (0..samples).map(|k| [sample_at(t0, t1, samples, closed, k), 0.0]).collect()
        }
        GeomKind::Surface { u, v, n, closed } => {
            let mut out = Vec::with_capacity(n[0] * n[1]);
            for j in 0..n[1] {
                for i in 0..n[0] {
                    out.push([sample_at(u[0], u[1], n[0], closed[0], i), sample_at(v[0], v[1], n[1], closed[1], j)]);
                }
            }
            out
        }
        // u = z (zSamples), v = x (xSamples); the expression reads (x, z)
        GeomKind::Height { width, depth, nx, nz } => {
            let mut out = Vec::with_capacity(nx * nz);
            for j in 0..nx {
                for i in 0..nz {
                    let z = sample_at(-depth / 2.0, depth / 2.0, nz, false, i);
                    let x = sample_at(-width / 2.0, width / 2.0, nx, false, j);
                    out.push([x, z]);
                }
            }
            out
        }
    }
}

/// The mesh of a surface or heightfield from its samples (`samples`) and the values of its expressions there
/// (`values[k][s]`: expression k at sample s).
pub fn mesh(kind: &GeomKind, samples: &[[f64; 2]], values: &[Vec<f64>]) -> Option<ParamMesh> {
    /// Vertices, texture coordinates, grid size along u and v, and the closed directions.
    type Grid = (Vec<[f64; 3]>, Vec<[f32; 2]>, usize, usize, [bool; 2]);
    let (points, uvs, nu, nv, closed): Grid = match *kind {
        GeomKind::Path { .. } => return None,
        GeomKind::Surface { u, v, n, closed } => {
            let span = |r: [f64; 2], x: f64| if r[1] != r[0] { (x - r[0]) / (r[1] - r[0]) } else { 0.0 };
            (
                (0..samples.len()).map(|s| [values[0][s], values[1][s], values[2][s]]).collect(),
                samples.iter().map(|q| [span(u, q[0]) as f32, span(v, q[1]) as f32]).collect(),
                n[0],
                n[1],
                closed,
            )
        }
        GeomKind::Height { width, depth, nx, nz } => (
            samples.iter().zip(&values[0]).map(|(q, h)| [q[0], -h, q[1]]).collect(),
            samples
                .iter()
                .map(|q| [((q[1] + depth / 2.0) / depth) as f32, ((q[0] + width / 2.0) / width) as f32])
                .collect(),
            nz,
            nx,
            [false, false],
        ),
    };
    let mesh = sr_3d::prim::parametric_grid(&points, &uvs, nu, nv, closed);
    let mut words: Vec<u64> = vec![nu as u64, nv as u64, closed[0] as u64, closed[1] as u64];
    words.extend(points.iter().flat_map(|p| p.map(f64::to_bits)));
    words.extend(uvs.iter().flat_map(|q| q.map(|c| c.to_bits() as u64)));
    Some(ParamMesh { mesh, key: crate::rng::hash(&words) })
}

/// Gives node `n`'s frame node its parametric geometry at this frame: a curve's outline as the `path` property, a
/// surface's mesh as [`FrameNode::param_mesh`].
pub(crate) fn attach(f: &mut crate::eval::Frame, n: u32, g: &Geom, out: &mut FrameNode) {
    let samples = samples(&g.kind);
    let mut values = Vec::with_capacity(g.exprs.len());
    for &x in &g.exprs {
        let mut v = Vec::new();
        f.sampled(n, x, &samples, &mut v);
        values.push(v);
    }
    match g.kind {
        GeomKind::Path { closed, .. } => {
            let points: Vec<[f64; 2]> = values[0].iter().zip(&values[1]).map(|(x, y)| [*x, *y]).collect();
            let (d, bbox) = path_data(&points, closed);
            set(out, "path", Value::Str(d.into()));
            out.param_box = bbox;
        }
        _ => out.param_mesh = mesh(&g.kind, &samples, &values).map(Arc::new),
    }
}

fn set(out: &mut FrameNode, k: &str, v: Value) {
    match out.props.0.iter_mut().find(|(name, _)| &**name == k) {
        Some(slot) => slot.1 = v,
        None => out.props.0.push((Arc::from(k), v)),
    }
}
