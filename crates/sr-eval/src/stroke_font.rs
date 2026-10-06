//! Stroke fonts: single-line fonts whose glyphs are pen strokes (the Hershey `jhf` format), laid out as the outline of a
//! `shape="stroke-text"` shape (SREP stroke-text).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use sr_model::element::Element;

use crate::{FrameNode, Program, Value};

/// The top of the capitals above the baseline, in font units (the Hershey fonts: y = -12 to y = 9).
const CAP_HEIGHT: f64 = 21.0;
/// The baseline, in font units.
const BASELINE: f64 = 9.0;
/// The em of the font in units (the line height).
const EM: f64 = 32.0;

/// One glyph: margins and strokes (runs of points between pen lifts).
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    /// Left margin.
    pub left: i32,
    /// Right margin.
    pub right: i32,
    /// Pen strokes of at least two points.
    pub strokes: Vec<Vec<(i32, i32)>>,
}

/// A stroke font: glyphs in file order, the n-th being U+0020 + n.
#[derive(Debug, Clone, PartialEq)]
pub struct Font {
    /// Glyphs in file order.
    pub glyphs: Vec<Glyph>,
}

/// Fonts read for stroke text, by file; they live as long as their compiled scene.
pub(crate) type Cache = Mutex<HashMap<PathBuf, Arc<Result<Arc<Font>, String>>>>;

/// Reads a `jhf` file. A record is the glyph number (5 characters), the count `n` of coordinate pairs including the margins
/// (3 characters) and `2n` characters of data, which may be wrapped over lines: every coordinate is a character whose code
/// minus that of `R` is the value, the first pair is the margins and the pair ` R` lifts the pen.
pub fn parse_jhf(text: &str) -> Result<Font, String> {
    let s: Vec<char> = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
    let mut i = 0;
    let mut glyphs = Vec::new();
    while i < s.len() {
        // a trailing blank line
        if s[i..].iter().all(|c| c.is_whitespace()) {
            break;
        }
        let record = glyphs.len() + 1;
        let field = |from: usize, len: usize| -> Result<usize, String> {
            let t: String = s
                .get(from..from + len)
                .ok_or_else(|| format!("record {record}: the file ends inside its header"))?
                .iter()
                .collect();
            t.trim().parse::<usize>().map_err(|_| format!("record {record}: {t:?} is not a number"))
        };
        let n = field(i + 5, 3)?;
        if n == 0 {
            return Err(format!("record {record}: a glyph has at least the margin pair"));
        }
        let data: &[char] = s
            .get(i + 8..i + 8 + 2 * n)
            .ok_or_else(|| format!("record {record}: the file ends inside its {n} pairs"))?;
        i += 8 + 2 * n;
        let v = |c: char| c as i32 - 'R' as i32;
        let (left, right) = (v(data[0]), v(data[1]));
        let mut strokes = Vec::new();
        let mut run: Vec<(i32, i32)> = Vec::new();
        for pair in data[2..].chunks(2) {
            if pair == [' ', 'R'] {
                if run.len() >= 2 {
                    strokes.push(std::mem::take(&mut run));
                } else {
                    run.clear();
                }
            } else {
                run.push((v(pair[0]), v(pair[1])));
            }
        }
        if run.len() >= 2 {
            strokes.push(run);
        }
        glyphs.push(Glyph { left, right, strokes });
    }
    Ok(Font { glyphs })
}

/// The path data of `text` set in `font` at cap height `font_size`: every stroke of every character in order, each stroke an
/// open subpath, with the characters the font lacks (listed once each) skipped. The first baseline is at `font_size`, the pen
/// starts at 0, a line feed returns it and moves the baseline down by the font's em.
pub fn layout(font: &Font, text: &str, font_size: f64) -> (String, Vec<char>) {
    let s = font_size / CAP_HEIGHT;
    let (mut data, mut missing) = (String::new(), Vec::new());
    let (mut pen, mut baseline) = (0.0, font_size);
    for ch in text.chars() {
        if ch == '\n' {
            pen = 0.0;
            baseline += EM * s;
            continue;
        }
        let glyph = (ch as usize).checked_sub(0x20).and_then(|k| font.glyphs.get(k)).filter(|_| !ch.is_control());
        let Some(g) = glyph else {
            if !missing.contains(&ch) {
                missing.push(ch);
            }
            continue;
        };
        for stroke in &g.strokes {
            for (k, (x, y)) in stroke.iter().enumerate() {
                let (px, py) = (pen + (*x - g.left) as f64 * s, baseline + (*y as f64 - BASELINE) * s);
                let _ = write!(data, "{}{} {} ", if k == 0 { "M" } else { "L" }, round(px), round(py));
            }
        }
        pen += (g.right - g.left) as f64 * s;
    }
    (data.trim_end().to_string(), missing)
}

fn round(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

fn load(p: &Program, id: &str) -> Result<Arc<Font>, String> {
    // the key of an asset of an included document carries the document's namespace in front of the id
    let (doc, key) = p
        .assets
        .get(id)
        .or_else(|| p.assets.iter().find(|(k, _)| k.rsplit('/').next() == Some(id)).map(|(_, v)| v))
        .ok_or_else(|| format!("stroke font asset {id} not found"))?;
    let scene = if *doc == 0 { &p.scene } else { &p.includes.get(*doc as usize - 1).ok_or("include missing")?.1 };
    let src = scene
        .assets
        .as_ref()
        .and_then(|a| {
            a.children.iter().find_map(|c| match c {
                sr_model::model::AssetsChild::StrokeFont(f) if f.id == *key => Some(f.src.clone()),
                _ => None,
            })
        })
        .ok_or_else(|| format!("asset {id} is not a stroke font"))?;
    let base = p.base_dirs.get(*doc as usize).cloned().unwrap_or_default();
    let path = match sr_model::assets::resolve(&src, &base) {
        sr_model::assets::Resolved::Local(path) => path,
        sr_model::assets::Resolved::Remote(u) => {
            return Err(format!("remote stroke font {u} is not fetched while rendering"))
        }
    };
    let entry = p
        .stroke_fonts
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(path.clone())
        .or_insert_with(|| {
            Arc::new(
                std::fs::read_to_string(&path)
                    .map_err(|e| format!("{}: {e}", path.display()))
                    .and_then(|t| parse_jhf(&t).map(Arc::new).map_err(|e| format!("{}: {e}", path.display()))),
            )
        })
        .clone();
    (*entry).clone()
}

/// Gives a `stroke-text` shape its outline as the `path` property, and reports what could not be laid out.
pub(crate) fn attach(p: &Program, n: &mut FrameNode, problems: &mut Vec<String>) {
    let e: &dyn Element = &*n.elem;
    if crate::sim::text(e, "shape").as_deref() != Some("stroke-text") {
        return;
    }
    let (Some(text), Some(id)) = (crate::sim::text(e, "text"), crate::sim::text(e, "strokeFont")) else { return };
    let size = n.props.get("fontSize").and_then(Value::as_num).unwrap_or_else(|| crate::sim::num(e, "fontSize", 48.0));
    match load(p, &id) {
        Ok(font) => {
            let (data, missing) = layout(&font, &text, size.max(1e-6));
            if !missing.is_empty() {
                let list: String = missing.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>().join(", ");
                let msg = format!("{}: the stroke font has no glyph for {list}; skipped", n.id);
                if !problems.contains(&msg) {
                    problems.push(msg);
                }
            }
            n.props.0.push(("path".into(), Value::Str(data.into())));
        }
        Err(e) => {
            let msg = format!("{}: {e}", n.id);
            if !problems.contains(&msg) {
                problems.push(msg);
            }
        }
    }
}
