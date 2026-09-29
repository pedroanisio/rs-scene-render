//! Charts: bar, column, line, area, pie, donut, scatter, counter and
//! progress, drawn on by `progress`, with axes, value labels and number
//! formats.

use sr_vector::geom::{p, Xf};
use sr_vector::measure::{self, TrimMode};
use sr_vector::scene::{Cmd, FillRule, Paint};
use sr_vector::stroke::{self, Cap, Join, Style as StrokeStyle};
use sr_vector::{shapes, Path, Poly};

use crate::font::FontLib;
use crate::glyph::{self, Decor, Drawing};
use crate::layout::{self, Align, Opts, Para, Run};
use crate::style::Style;

/// A data series.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub name: String,
    pub values: Vec<f64>,
    pub color: Option<Paint>,
}

/// Chart kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bar,
    Column,
    Line,
    Area,
    Pie,
    Donut,
    Scatter,
    Counter,
    Progress,
}

impl Kind {
    /// Parses a kind name.
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "bar" => Kind::Bar,
            "column" => Kind::Column,
            "line" => Kind::Line,
            "area" => Kind::Area,
            "pie" => Kind::Pie,
            "donut" => Kind::Donut,
            "scatter" => Kind::Scatter,
            "counter" => Kind::Counter,
            "progress" => Kind::Progress,
            _ => return None,
        })
    }
}

/// A chart to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    pub kind: Kind,
    pub size: [f64; 2],
    pub labels: Vec<String>,
    pub series: Vec<Series>,
    pub progress: f64,
    pub show_axes: bool,
    pub show_values: bool,
    pub format: Option<String>,
    pub text: Style,
}

/// The default palette (sRGB).
pub const PALETTE: [[f64; 3]; 8] = [
    [0.26, 0.52, 0.96],
    [0.96, 0.42, 0.31],
    [0.20, 0.73, 0.49],
    [0.98, 0.76, 0.18],
    [0.62, 0.40, 0.87],
    [0.16, 0.75, 0.85],
    [0.94, 0.38, 0.62],
    [0.55, 0.60, 0.65],
];

fn color_of(s: &Series, i: usize) -> Paint {
    s.color.clone().unwrap_or_else(|| {
        let c = PALETTE[i % PALETTE.len()];
        Paint::Solid { rgba: [c[0], c[1], c[2], 1.0], srgb: true }
    })
}

/// Formats a number with a pattern (`0.0`, `#,##0`, `0%`, `$#,##0.00`, `%.2f`).
pub fn format_number(v: f64, fmt: Option<&str>) -> String {
    let Some(f) = fmt.filter(|f| !f.is_empty()) else {
        return if v.fract().abs() < 1e-9 {
            format!("{}", v.round() as i64)
        } else {
            format!("{:.2}", v).trim_end_matches('0').trim_end_matches('.').to_string()
        };
    };
    if let Some(rest) = f.strip_prefix('%') {
        let dec = rest.trim_start_matches('.').trim_end_matches('f').parse::<usize>().unwrap_or(0);
        return format!("{v:.dec$}");
    }
    let first = f.find(['0', '#']).unwrap_or(f.len());
    let last = f.rfind(['0', '#']).map(|k| k + 1).unwrap_or(first);
    let (prefix, pattern, suffix) = (&f[..first], &f[first..last], &f[last..]);
    let percent = suffix.contains('%');
    let v = if percent { v * 100.0 } else { v };
    let dec = pattern.split_once('.').map(|(_, d)| d.len()).unwrap_or(0);
    let s = format!("{:.dec$}", v.abs());
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    let int = if pattern.contains(',') {
        let b = int.as_bytes();
        let mut out = String::new();
        for (k, c) in b.iter().enumerate() {
            if k > 0 && (b.len() - k) % 3 == 0 {
                out.push(',');
            }
            out.push(*c as char);
        }
        out
    } else {
        int.to_string()
    };
    let sign = if v < 0.0 { "-" } else { "" };
    let body = if frac.is_empty() { int } else { format!("{int}.{frac}") };
    format!("{sign}{prefix}{body}{suffix}")
}

fn nice_max(v: f64) -> f64 {
    if v <= 0.0 {
        return 1.0;
    }
    let e = libm::pow(10.0, libm::floor(libm::log10(v)));
    for m in [1.0, 2.0, 2.5, 5.0, 10.0] {
        if m * e >= v {
            return m * e;
        }
    }
    10.0 * e
}

#[allow(clippy::too_many_arguments)]
fn text(lib: &mut FontLib, s: &str, st: &Style, x: f64, y: f64, align: Align, width: f64, tol: f64) -> Drawing {
    let para = Para {
        runs: vec![Run { text: s.to_string(), style: 0, role: None }],
        styles: vec![st.clone()],
        opts: Opts { width, align, ..Default::default() },
    };
    let lay = layout::layout(lib, &para);
    glyph::draw(lib, &lay, None, &Decor::default(), tol).transformed(&Xf::translate(x, y))
}

/// Draws a chart in its `size` box.
pub fn draw(lib: &mut FontLib, c: &Chart, tol: f64) -> Drawing {
    let mut d = Drawing::default();
    let [w, h] = c.size;
    let prog = c.progress.clamp(0.0, 1.0);
    let fs = c.text.size;
    let axis = c.text.color.clone().unwrap_or(Paint::Solid { rgba: [1.0; 4], srgb: false });
    let n = c.labels.len().max(c.series.iter().map(|s| s.values.len()).max().unwrap_or(0));
    let max = c.series.iter().flat_map(|s| s.values.iter().copied()).fold(0.0f64, f64::max);
    let top = nice_max(max);
    let fill = |d: &mut Drawing, path: &Path, paint: Paint| d.scene.fill(path, FillRule::NonZero, paint, 1.0, tol);
    match c.kind {
        Kind::Counter => {
            // one value counts from 0, two values from the first to the second
            let vals = c.series.first().map(|s| s.values.as_slice()).unwrap_or(&[]);
            let v = match vals {
                [a, b, ..] => a + (b - a) * prog,
                [a] => a * prog,
                [] => 0.0,
            };
            let st = Style { size: (h * 0.6).min(w / 5.0).max(fs), ..c.text.clone() };
            d.extend(text(
                lib,
                &format_number(v, c.format.as_deref()),
                &st,
                0.0,
                (h - st.size * 1.2) * 0.5,
                Align::Center,
                w,
                tol,
            ));
            return d;
        }
        Kind::Progress => {
            let v = c.series.first().and_then(|s| s.values.first()).copied().unwrap_or(0.0);
            let frac = if v > 1.0 { v / 100.0 } else { v }.clamp(0.0, 1.0) * prog;
            let bh = (h * 0.35).max(4.0);
            let y = (h - bh) * 0.5;
            fill(
                &mut d,
                &shapes::rect(0.0, y, w, bh, [bh * 0.5; 4]),
                Paint::Solid { rgba: [1.0, 1.0, 1.0, 0.2], srgb: true },
            );
            if frac > 0.0 {
                let color = c
                    .series
                    .first()
                    .map(|s| color_of(s, 0))
                    .unwrap_or_else(|| color_of(&Series { name: String::new(), values: vec![], color: None }, 0));
                fill(&mut d, &shapes::rect(0.0, y, w * frac, bh, [bh * 0.5; 4]), color);
            }
            if c.show_values {
                let label = format_number(
                    if v > 1.0 { v * prog } else { frac },
                    c.format.as_deref().or(if v > 1.0 { None } else { Some("0%") }),
                );
                d.extend(text(lib, &label, &c.text, 0.0, y - fs * 1.4, Align::End, w, tol));
            }
            return d;
        }
        Kind::Pie | Kind::Donut => {
            let vals = c.series.first().map(|s| s.values.clone()).unwrap_or_default();
            let total: f64 = vals.iter().filter(|v| **v > 0.0).sum::<f64>().max(1e-12);
            let r = w.min(h) * 0.45;
            let ctr = p(w * 0.5, h * 0.5);
            let mut a0 = -90.0f64;
            let sweep_total = 360.0 * prog;
            for (i, v) in vals.iter().enumerate() {
                let sw = (v.max(0.0) / total * 360.0).min(sweep_total - (a0 + 90.0)).max(0.0);
                if sw <= 0.0 {
                    break;
                }
                let mut path = Path::default();
                path.move_to(ctr);
                let steps = ((sw / 5.0).ceil() as usize).max(2);
                for k in 0..=steps {
                    let a = (a0 + sw * k as f64 / steps as f64).to_radians();
                    path.line_to(ctr + p(libm::cos(a), libm::sin(a)) * r);
                }
                path.close();
                let color =
                    c.series.first().and_then(|s| s.color.clone()).filter(|_| vals.len() == 1).unwrap_or_else(|| {
                        let col = PALETTE[i % PALETTE.len()];
                        Paint::Solid { rgba: [col[0], col[1], col[2], 1.0], srgb: true }
                    });
                fill(&mut d, &path, color);
                if c.show_values {
                    let am = (a0 + sw * 0.5).to_radians();
                    let lp =
                        ctr + p(libm::cos(am), libm::sin(am)) * (r * if c.kind == Kind::Donut { 0.8 } else { 0.65 });
                    d.extend(text(
                        lib,
                        &format_number(*v, c.format.as_deref()),
                        &c.text,
                        lp.x - 100.0,
                        lp.y - fs * 0.6,
                        Align::Center,
                        200.0,
                        tol,
                    ));
                }
                a0 += sw;
            }
            if c.kind == Kind::Donut {
                // punch the hole with the rest of the scene intact
                let hole: Vec<Poly> = shapes::ellipse(ctr.x, ctr.y, r * 0.6, r * 0.6).flatten(tol);
                let body = std::mem::take(&mut d.scene.cmds);
                d.scene.cmds.push(Cmd::Push { mask_init: 1.0 });
                d.scene.cmds.extend(body);
                d.scene.cmds.push(Cmd::Mask {
                    polys: hole,
                    rule: FillRule::NonZero,
                    op: sr_vector::MaskOp::Subtract,
                    opacity: 1.0,
                    invert: false,
                });
                d.scene.cmds.push(Cmd::Pop { opacity: 1.0 });
            }
            return d;
        }
        _ => {}
    }
    // cartesian charts
    let left = if c.show_axes || c.kind == Kind::Bar { fs * 3.5 } else { 0.0 };
    let bottom = if c.show_axes || !c.labels.is_empty() { fs * 1.8 } else { 0.0 };
    let (px, py, pw, ph) = (left, fs * 0.8, (w - left).max(1.0), (h - bottom - fs * 0.8).max(1.0));
    let ns = c.series.len().max(1);
    let axis_line = |d: &mut Drawing, a: sr_vector::P, b: sr_vector::P| {
        let s = stroke::stroke(
            &[Poly { pts: vec![a, b], closed: false }],
            &StrokeStyle { width: 1.0, cap: Cap::Butt, join: Join::Miter, miter_limit: 4.0 },
            tol,
        );
        d.scene.cmds.push(Cmd::Fill { polys: s, rule: FillRule::NonZero, paint: axis.clone(), opacity: 0.6 });
    };
    if c.show_axes {
        axis_line(&mut d, p(px, py), p(px, py + ph));
        axis_line(&mut d, p(px, py + ph), p(px + pw, py + ph));
        if c.kind != Kind::Bar {
            for k in 0..=4 {
                let v = top * k as f64 / 4.0;
                let y = py + ph - ph * k as f64 / 4.0;
                d.extend(text(
                    lib,
                    &format_number(v, c.format.as_deref()),
                    &c.text,
                    0.0,
                    y - fs * 0.6,
                    Align::End,
                    left - fs * 0.4,
                    tol,
                ));
            }
        }
    }
    let slot = |i: usize| -> (f64, f64) {
        let along = if c.kind == Kind::Bar { ph } else { pw };
        let sw = along / n.max(1) as f64;
        (sw * i as f64, sw)
    };
    for (li, lab) in c.labels.iter().enumerate() {
        let (o, sw) = slot(li);
        if c.kind == Kind::Bar {
            d.extend(text(lib, lab, &c.text, 0.0, py + o + sw * 0.5 - fs * 0.6, Align::End, left - fs * 0.4, tol));
        } else {
            d.extend(text(lib, lab, &c.text, px + o, py + ph + fs * 0.3, Align::Center, sw, tol));
        }
    }
    for (si, s) in c.series.iter().enumerate() {
        let color = color_of(s, si);
        match c.kind {
            Kind::Column | Kind::Bar => {
                for (i, v) in s.values.iter().enumerate() {
                    let (o, sw) = slot(i);
                    let bw = sw * 0.7 / ns as f64;
                    let off = o + sw * 0.15 + bw * si as f64;
                    let len = (v.max(0.0) / top) * prog;
                    let r = if c.kind == Kind::Column {
                        [px + off, py + ph * (1.0 - len), bw, ph * len]
                    } else {
                        [px, py + off, pw * len, bw]
                    };
                    if r[2] > 0.0 && r[3] > 0.0 {
                        fill(&mut d, &shapes::rect(r[0], r[1], r[2], r[3], [0.0; 4]), color.clone());
                    }
                    if c.show_values && prog > 0.0 {
                        let label = format_number(v * prog, c.format.as_deref());
                        if c.kind == Kind::Column {
                            d.extend(text(
                                lib,
                                &label,
                                &c.text,
                                r[0] - bw,
                                r[1] - fs * 1.3,
                                Align::Center,
                                bw * 3.0,
                                tol,
                            ));
                        } else {
                            d.extend(text(
                                lib,
                                &label,
                                &c.text,
                                r[0] + r[2] + fs * 0.3,
                                r[1] + bw * 0.5 - fs * 0.6,
                                Align::Start,
                                f64::INFINITY,
                                tol,
                            ));
                        }
                    }
                }
            }
            Kind::Line | Kind::Area => {
                let pts: Vec<sr_vector::P> = s
                    .values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let (o, sw) = slot(i);
                        p(px + o + sw * 0.5, py + ph * (1.0 - v.max(0.0) / top))
                    })
                    .collect();
                if pts.len() < 2 {
                    continue;
                }
                let line = Poly { pts: pts.clone(), closed: false };
                let shown = measure::trim(&[line], 0.0, prog, 0.0, TrimMode::Simultaneous);
                if c.kind == Kind::Area {
                    for q in &shown {
                        let mut ap = q.pts.clone();
                        ap.push(p(q.pts.last().unwrap().x, py + ph));
                        ap.push(p(q.pts[0].x, py + ph));
                        let mut col = color.clone();
                        if let Paint::Solid { rgba, .. } = &mut col {
                            rgba[3] *= 0.45;
                        }
                        d.scene.cmds.push(Cmd::Fill {
                            polys: vec![Poly { pts: ap, closed: true }],
                            rule: FillRule::NonZero,
                            paint: col,
                            opacity: 1.0,
                        });
                    }
                }
                let st = stroke::stroke(
                    &shown,
                    &StrokeStyle { width: (fs * 0.18).max(2.0), cap: Cap::Round, join: Join::Round, miter_limit: 4.0 },
                    tol,
                );
                d.scene.cmds.push(Cmd::Fill { polys: st, rule: FillRule::NonZero, paint: color.clone(), opacity: 1.0 });
                if c.show_values {
                    let upto = ((pts.len() as f64 - 1.0) * prog + 1e-9).floor() as usize;
                    for (i, q) in pts.iter().enumerate().take(upto + 1) {
                        d.extend(text(
                            lib,
                            &format_number(s.values[i], c.format.as_deref()),
                            &c.text,
                            q.x - 60.0,
                            q.y - fs * 1.5,
                            Align::Center,
                            120.0,
                            tol,
                        ));
                    }
                }
            }
            Kind::Scatter => {
                // values alternate x, y
                let pairs: Vec<(f64, f64)> =
                    s.values.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0], c[1])).collect();
                let xmax = nice_max(pairs.iter().map(|q| q.0).fold(0.0, f64::max));
                let ymax = nice_max(pairs.iter().map(|q| q.1).fold(0.0, f64::max));
                let shown = ((pairs.len() as f64) * prog).ceil() as usize;
                for &(x, y) in pairs.iter().take(shown) {
                    let q = p(px + pw * x / xmax, py + ph * (1.0 - y / ymax));
                    fill(&mut d, &shapes::ellipse(q.x, q.y, fs * 0.25, fs * 0.25), color.clone());
                }
            }
            _ => {}
        }
    }
    d
}
