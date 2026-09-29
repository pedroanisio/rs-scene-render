//! Paint descriptors: solid colours, linear, radial, conic and mesh
//! gradients, and patterns, with animated stop values from the FrameGraph.

use std::collections::HashMap;

use sr_eval::{FrameGraph, Value};
use sr_model::model::{self as m, PaintsChild};

use crate::color::Working;
use crate::types::{PaintDesc, Stop};

/// Paint tables built for one frame.
#[derive(Default)]
pub struct PaintTable {
    /// Descriptors.
    pub paints: Vec<PaintDesc>,
    /// Stops and mesh colours.
    pub stops: Vec<Stop>,
}

/// A resolved paint request.
pub enum PaintRef<'a> {
    /// A colour value from the FrameGraph or document (document literal).
    Color([f64; 4]),
    /// A paint element with its animated props.
    Paint(&'a PaintsChild),
}

/// Affine `[a, b, c, d, e, f]` mapping local coordinates to paint space. `rotation` (degrees,
/// clockwise) turns the gradient about the centre of the painted box in local pixels, whatever the
/// units (CONVENTIONS 5.18, as the Python renderer does), so a non-square box rotates without shear.
fn object_xform(box_rect: [f64; 4], rotation: f64, object: bool) -> [f64; 6] {
    let [x0, y0, w, h] = box_rect;
    let pivot = [x0 + w * 0.5, y0 + h * 0.5];
    let r = (-rotation).to_radians();
    let (c, s) = (libm::cos(r), libm::sin(r));
    // g = M·p + k − M·pivot, M = S⁻¹·R(−rotation) with S the box size (object units) or 1 (user units),
    // and k the pivot in paint space
    let (sx, sy, k) = if object {
        (1.0 / w.max(1e-9), 1.0 / h.max(1e-9), [0.5, 0.5])
    } else {
        (1.0, 1.0, pivot)
    };
    let (a, b, cc, d) = (c * sx, s * sy, -s * sx, c * sy);
    [a, b, cc, d, k[0] - (a * pivot[0] + cc * pivot[1]), k[1] - (b * pivot[0] + d * pivot[1])]
}

fn prop<'g>(g: &'g FrameGraph, key: &str, name: &str) -> Option<&'g Value> {
    g.elements.iter().find(|e| &*e.key == key).and_then(|e| e.props.get(name))
}

fn value_color(v: &Value) -> Option<[f64; 4]> {
    match v {
        Value::Color(c) => Some(*c),
        _ => None,
    }
}

fn num(v: Option<&Value>, d: f64) -> f64 {
    v.and_then(Value::as_num).unwrap_or(d)
}

fn literal(c: &sr_model::values::Color, tokens: &dyn Fn(&str) -> Option<[f64; 4]>) -> [f64; 4] {
    match c {
        sr_model::values::Color::Rgba(r) => [r.r as f64, r.g as f64, r.b as f64, r.a as f64],
        sr_model::values::Color::Token(t) => tokens(t).unwrap_or([0.0, 0.0, 0.0, 0.0]),
    }
}

/// Stops of a linear, radial or conic gradient as (offset, straight stored working RGBA).
pub fn gradient_stops(
    w: &Working,
    p: &PaintsChild,
    g: &FrameGraph,
    tokens: &dyn Fn(&str) -> Option<[f64; 4]>,
) -> Option<Vec<(f64, [f64; 4])>> {
    let key = p.id()?.to_string();
    let children = match p {
        PaintsChild::LinearGradient(l) => &l.children,
        PaintsChild::RadialGradient(r) => &r.children,
        PaintsChild::ConicGradient(c) => &c.children,
        _ => return None,
    };
    let mut out: Vec<(f64, [f64; 4])> = Vec::new();
    let mut k = 0;
    for c in children {
        if let m::GradientBaseChild::Stop(s) = c {
            let sk = format!("{key}/stop[{k}]");
            k += 1;
            let color = prop(g, &sk, "color").and_then(value_color).unwrap_or_else(|| literal(&s.color, tokens));
            let opacity = num(prop(g, &sk, "opacity"), s.opacity.get());
            let offset = num(prop(g, &sk, "offset"), s.offset.get());
            let off = out.last().map(|l| l.0.max(offset)).unwrap_or(offset);
            out.push((off, w.from_literal([color[0], color[1], color[2], color[3] * opacity])));
        }
    }
    Some(out)
}

impl PaintTable {
    /// Adds a solid colour (document literal, straight) and returns its index.
    pub fn solid(&mut self, w: &Working, c: [f64; 4]) -> u32 {
        let stored = w.from_literal(c);
        self.stops.push(Stop { color: stored.map(|x| x as f32), offset: 0.0, midpoint: 0.5, pad: [0.0; 2] });
        self.paints.push(PaintDesc {
            kind: 0,
            stop_off: (self.stops.len() - 1) as u32,
            stop_count: 1,
            xform0: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        });
        (self.paints.len() - 1) as u32
    }

    /// Adds a gradient paint over `box_rect` (x, y, w, h in local space).
    pub fn gradient(
        &mut self,
        w: &Working,
        p: &PaintsChild,
        g: &FrameGraph,
        box_rect: [f64; 4],
        tokens: &dyn Fn(&str) -> Option<[f64; 4]>,
    ) -> Option<u32> {
        let key = p.id()?.to_string();
        let a = |name: &str, d: f64| num(prop(g, &key, name), d);
        let stops_of = |children: &[m::GradientBaseChild], stops: &mut Vec<Stop>| -> (u32, u32) {
            let off = stops.len() as u32;
            let mut k = 0;
            for c in children {
                if let m::GradientBaseChild::Stop(s) = c {
                    let sk = format!("{key}/stop[{k}]");
                    k += 1;
                    let color =
                        prop(g, &sk, "color").and_then(value_color).unwrap_or_else(|| literal(&s.color, tokens));
                    let opacity = num(prop(g, &sk, "opacity"), s.opacity.get());
                    let offset = num(prop(g, &sk, "offset"), s.offset.get());
                    let stored = w.from_literal([color[0], color[1], color[2], color[3] * opacity]);
                    stops.push(Stop {
                        color: stored.map(|x| x as f32),
                        offset: offset as f32,
                        midpoint: num(prop(g, &sk, "midpoint"), s.midpoint.get()) as f32,
                        pad: [0.0; 2],
                    });
                }
            }
            // non-decreasing offsets, as the Schematron rule requires of the document
            let slice = &mut stops[off as usize..];
            for i in 1..slice.len() {
                if slice[i].offset < slice[i - 1].offset {
                    slice[i].offset = slice[i - 1].offset;
                }
            }
            (off, stops.len() as u32 - off)
        };
        let space = |s: m::GradientCommonInterpolationSpace| match s {
            m::GradientCommonInterpolationSpace::Linear => 0,
            m::GradientCommonInterpolationSpace::Srgb => 1,
            m::GradientCommonInterpolationSpace::Oklab => 2,
            m::GradientCommonInterpolationSpace::Oklch => 3,
        };
        let spread = |s: m::GradientCommonSpread| match s {
            m::GradientCommonSpread::Pad => 0,
            m::GradientCommonSpread::Reflect => 1,
            m::GradientCommonSpread::Repeat => 2,
        };
        let f32x4 = |v: [f64; 4]| v.map(|x| x as f32);
        let desc = match p {
            PaintsChild::LinearGradient(l) => {
                let (off, n) = stops_of(&l.children, &mut self.stops);
                let x = object_xform(box_rect, a("rotation", l.rotation), l.units == m::GradientCommonUnits::Object);
                PaintDesc {
                    xform0: f32x4([x[0], x[1], x[2], x[3]]),
                    xform1: f32x4([x[4], x[5], 0.0, 0.0]),
                    p0: f32x4([a("x1", l.x1), a("y1", l.y1), a("x2", l.x2), a("y2", l.y2)]),
                    kind: 1,
                    spread: spread(l.spread),
                    space: space(l.interpolation_space),
                    stop_off: off,
                    stop_count: n,
                    dither: l.dither as u32,
                    ..Default::default()
                }
            }
            PaintsChild::RadialGradient(r) => {
                let (off, n) = stops_of(&r.children, &mut self.stops);
                let x = object_xform(box_rect, a("rotation", r.rotation), r.units == m::GradientCommonUnits::Object);
                let (cx, cy) = (a("cx", r.cx), a("cy", r.cy));
                PaintDesc {
                    xform0: f32x4([x[0], x[1], x[2], x[3]]),
                    xform1: f32x4([x[4], x[5], 0.0, 0.0]),
                    p0: f32x4([cx, cy, a("r", r.r.get()), a("aspect", r.aspect.get())]),
                    p1: f32x4([
                        r.fx.map(|v| a("fx", v)).unwrap_or(cx),
                        r.fy.map(|v| a("fy", v)).unwrap_or(cy),
                        a("fr", r.fr.get()),
                        0.0,
                    ]),
                    kind: 2,
                    spread: spread(r.spread),
                    space: space(r.interpolation_space),
                    stop_off: off,
                    stop_count: n,
                    dither: r.dither as u32,
                    ..Default::default()
                }
            }
            PaintsChild::ConicGradient(c) => {
                let (off, n) = stops_of(&c.children, &mut self.stops);
                let x = object_xform(box_rect, a("rotation", c.rotation), c.units == m::GradientCommonUnits::Object);
                PaintDesc {
                    xform0: f32x4([x[0], x[1], x[2], x[3]]),
                    xform1: f32x4([x[4], x[5], 0.0, 0.0]),
                    p0: f32x4([a("cx", c.cx), a("cy", c.cy), a("angle", c.angle), 0.0]),
                    kind: 3,
                    spread: spread(c.spread),
                    space: space(c.interpolation_space),
                    stop_off: off,
                    stop_count: n,
                    dither: c.dither as u32,
                    ..Default::default()
                }
            }
            PaintsChild::MeshGradient(mg) => {
                let (rows, cols) = (mg.rows.get() as usize, mg.cols.get() as usize);
                let off = self.stops.len() as u32;
                let mut grid = vec![[0.0f64; 4]; rows * cols];
                let mut known = vec![false; rows * cols];
                let mut k = 0;
                for c in &mg.children {
                    if let m::MeshGradientChild::Point(pt) = c {
                        let pk = format!("{key}/point[{k}]");
                        k += 1;
                        let (r, cc) = (pt.row as usize, pt.col as usize);
                        if r < rows && cc < cols {
                            grid[r * cols + cc] = prop(g, &pk, "color")
                                .and_then(value_color)
                                .unwrap_or_else(|| literal(&pt.color, tokens));
                            known[r * cols + cc] = true;
                        }
                    }
                }
                // a missing point takes the colour of the nearest defined one (as the Python renderer does)
                let defined: Vec<usize> = (0..rows * cols).filter(|&i| known[i]).collect();
                for i in (0..rows * cols).filter(|&i| !known[i]) {
                    let d2 = |j: usize| {
                        let (dr, dc) = ((i / cols) as i64 - (j / cols) as i64, (i % cols) as i64 - (j % cols) as i64);
                        dr * dr + dc * dc
                    };
                    if let Some(&j) = defined.iter().min_by_key(|&&j| d2(j)) {
                        grid[i] = grid[j];
                    }
                }
                for col in grid {
                    self.stops.push(Stop { color: w.from_literal(col).map(|x| x as f32), ..Default::default() });
                }
                let x = object_xform(box_rect, 0.0, true);
                PaintDesc {
                    xform0: f32x4([x[0], x[1], x[2], x[3]]),
                    xform1: f32x4([x[4], x[5], 0.0, 0.0]),
                    p0: [rows as f32, cols as f32, 0.0, 0.0],
                    kind: 4,
                    space: match mg.interpolation_space {
                        m::MeshGradientInterpolationSpace::Linear => 0,
                        m::MeshGradientInterpolationSpace::Srgb => 1,
                        m::MeshGradientInterpolationSpace::Oklab => 2,
                    },
                    stop_off: off,
                    stop_count: (rows * cols) as u32,
                    ..Default::default()
                }
            }
            PaintsChild::Pattern(_) => return None,
        };
        self.paints.push(desc);
        Some((self.paints.len() - 1) as u32)
    }

    /// Pattern tiling: a descriptor whose transform maps local coordinates to
    /// repeat-sampled texture coordinates.
    pub fn pattern(&mut self, pt: &m::PatternPaint, g: &FrameGraph, asset_size: [f64; 2]) -> u32 {
        let key = pt.id.as_str();
        let a = |name: &str, d: f64| num(prop(g, key, name), d);
        let tw = pt.tile_width.map(|v| a("tileWidth", v.get())).unwrap_or(asset_size[0]) * a("scale", pt.scale.get());
        let th = pt.tile_height.map(|v| a("tileHeight", v.get())).unwrap_or(asset_size[1]) * a("scale", pt.scale.get());
        let r = (-a("rotation", pt.rotation)).to_radians();
        let (c, s) = (libm::cos(r), libm::sin(r));
        let (ox, oy) = (a("offsetX", pt.offset_x), a("offsetY", pt.offset_y));
        // uv = S(1/tw,1/th)·R(−rot)·(p − offset)
        let x = [c / tw, s / th, -s / tw, c / th, (-c * ox + s * oy) / tw, (-s * ox - c * oy) / th];
        self.paints.push(PaintDesc {
            xform0: [x[0] as f32, x[1] as f32, x[2] as f32, x[3] as f32],
            xform1: [x[4] as f32, x[5] as f32, 0.0, 0.0],
            kind: 5,
            ..Default::default()
        });
        (self.paints.len() - 1) as u32
    }
}

/// Style-token colours of a scene, for paint literals.
pub fn token_table(scene: &m::Scene) -> HashMap<String, [f64; 4]> {
    let mut raw = HashMap::new();
    if let Some(s) = &scene.styles {
        for c in &s.children {
            if let m::StylesChild::Token(t) = c {
                raw.insert(t.name.clone(), t.value.clone());
            }
        }
    }
    let mut out = HashMap::new();
    for k in raw.keys() {
        let mut cur = k.clone();
        for _ in 0..8 {
            let Some(v) = raw.get(&cur) else { break };
            match <sr_model::values::Color as sr_model::parse::ParseValue>::parse_value(v.trim()) {
                Ok(sr_model::values::Color::Rgba(c)) => {
                    out.insert(k.clone(), [c.r as f64, c.g as f64, c.b as f64, c.a as f64]);
                    break;
                }
                Ok(sr_model::values::Color::Token(t)) => cur = t,
                Err(_) => break,
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_units_map_the_box_to_the_unit_square() {
        let x = object_xform([10.0, 20.0, 100.0, 50.0], 0.0, true);
        let ap = |p: [f64; 2]| [x[0] * p[0] + x[2] * p[1] + x[4], x[1] * p[0] + x[3] * p[1] + x[5]];
        let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12;
        assert!(near(ap([10.0, 20.0]), [0.0, 0.0]));
        assert!(near(ap([110.0, 70.0]), [1.0, 1.0]));
        let r = object_xform([0.0, 0.0, 1.0, 1.0], 90.0, true);
        let ap = |p: [f64; 2]| [r[0] * p[0] + r[2] * p[1] + r[4], r[1] * p[0] + r[3] * p[1] + r[5]];
        let c = ap([0.5, 0.5]);
        assert!((c[0] - 0.5).abs() < 1e-12 && (c[1] - 0.5).abs() < 1e-12, "rotation keeps the centre");
    }

    #[test]
    fn rotation_turns_about_the_box_centre_in_pixels() {
        // CONVENTIONS 5.18: a 200 × 100 box turned 90° clockwise: the pixel 50 px below the centre shows
        // the point 50 px right of it in the unturned gradient, i.e. x = 0.75 of the box (not 1.0, as
        // turning the unit square would give)
        let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12;
        let x = object_xform([0.0, 0.0, 200.0, 100.0], 90.0, true);
        let ap = |p: [f64; 2]| [x[0] * p[0] + x[2] * p[1] + x[4], x[1] * p[0] + x[3] * p[1] + x[5]];
        assert!(near(ap([100.0, 50.0]), [0.5, 0.5]));
        assert!(near(ap([100.0, 100.0]), [0.75, 0.5]), "{:?}", ap([100.0, 100.0]));
        // user units pivot on the box centre too
        let u = object_xform([0.0, 0.0, 200.0, 100.0], 90.0, false);
        let ap = |p: [f64; 2]| [u[0] * p[0] + u[2] * p[1] + u[4], u[1] * p[0] + u[3] * p[1] + u[5]];
        assert!(near(ap([100.0, 50.0]), [100.0, 50.0]));
        assert!(near(ap([100.0, 100.0]), [150.0, 50.0]), "{:?}", ap([100.0, 100.0]));
    }
}
