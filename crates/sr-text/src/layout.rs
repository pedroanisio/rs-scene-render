//! Paragraph layout: itemisation by style, face and bidi level, shaping,
//! line breaking, bidi reordering, alignment and box fitting.

use std::sync::OnceLock;

use unicode_bidi::{BidiInfo, Level};
use unicode_linebreak::BreakOpportunity;

use crate::font::FontLib;
use crate::style::{Style, Transform};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VAlign {
    #[default]
    Top,
    Middle,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dir {
    #[default]
    Auto,
    Ltr,
    Rtl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Writing {
    #[default]
    Horizontal,
    VerticalRl,
    VerticalLr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Wrap {
    #[default]
    Word,
    Character,
    None,
    Balance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutoFit {
    #[default]
    None,
    Shrink,
    Grow,
    Fit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overflow {
    #[default]
    Visible,
    Clip,
    Ellipsis,
}

/// Paragraph options (the text asset's box and flow attributes).
#[derive(Debug, Clone, PartialEq)]
pub struct Opts {
    /// Box size; infinite for unbounded.
    pub width: f64,
    pub height: f64,
    pub align: Align,
    pub line_height: f64,
    /// Extra space after every character, px.
    pub letter_spacing: f64,
    pub direction: Dir,
    pub language: Option<String>,
    pub valign: VAlign,
    pub writing: Writing,
    pub wrap: Wrap,
    pub hyphenate: bool,
    pub auto_fit: AutoFit,
    pub min_size: Option<f64>,
    pub max_size: Option<f64>,
    pub max_lines: Option<usize>,
    pub overflow: Overflow,
    pub emoji_color: bool,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts {
            width: f64::INFINITY,
            height: f64::INFINITY,
            align: Align::Start,
            line_height: 1.2,
            letter_spacing: 0.0,
            direction: Dir::Auto,
            language: None,
            valign: VAlign::Top,
            writing: Writing::Horizontal,
            wrap: Wrap::Word,
            hyphenate: false,
            auto_fit: AutoFit::None,
            min_size: None,
            max_size: None,
            max_lines: None,
            overflow: Overflow::Visible,
            emoji_color: true,
        }
    }
}

/// A styled run of text (a span) with its optional role.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub text: String,
    pub style: usize,
    pub role: Option<String>,
}

/// A paragraph to lay out: runs, their styles and options.
#[derive(Debug, Clone, PartialEq)]
pub struct Para {
    pub runs: Vec<Run>,
    pub styles: Vec<Style>,
    pub opts: Opts,
}

/// A positioned glyph.
#[derive(Debug, Clone, PartialEq)]
pub struct PGlyph {
    pub face: usize,
    pub gid: u16,
    /// Pen position on the baseline (vertical text: the glyph origin), px.
    pub x: f64,
    pub y: f64,
    /// Font size in px (after autoFit).
    pub size: f64,
    pub style: usize,
    /// Logical character index of the glyph's cluster.
    pub ch: usize,
    pub word: usize,
    pub line: usize,
    pub span: usize,
    pub advance: f64,
    pub vertical: bool,
}

/// A laid-out line (column, in vertical text).
#[derive(Debug, Clone, PartialEq)]
pub struct LineBox {
    /// Box of the line's content: x, y, width, height.
    pub rect: [f64; 4],
    pub baseline: f64,
    pub glyphs: std::ops::Range<usize>,
}

/// The result of laying out a paragraph.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    pub glyphs: Vec<PGlyph>,
    pub lines: Vec<LineBox>,
    /// Word boxes (x, y, w, h), one per word occurrence on a line, with the word index.
    pub words: Vec<([f64; 4], usize)>,
    /// Logical characters after text transforms.
    pub chars: Vec<char>,
    /// Word index of each character (spaces take the following word's index).
    pub char_word: Vec<usize>,
    /// Span (run) of each character.
    pub char_span: Vec<usize>,
    pub word_count: usize,
    /// Size scale applied by autoFit.
    pub scale: f64,
    /// The box.
    pub size: [f64; 2],
    /// Lines were dropped (maxLines or overflow).
    pub truncated: bool,
    /// Content must be clipped to the box.
    pub clip: bool,
    /// Styles at the laid-out size.
    pub styles: Vec<Style>,
    pub vertical: bool,
    /// Share of a width change that moves a line's start: 0 for start (and justified)
    /// alignment, ½ for centre, 1 for end (mirrored in right-to-left paragraphs).
    pub align_shift: f64,
}

/// A glyph on a line: face, glyph, advance, x and y offsets, character, style, vertical.
type LineGlyph = (usize, u16, f64, f64, f64, usize, usize, bool);

struct Shaped {
    face: usize,
    gid: u16,
    adv: f64,
    dx: f64,
    dy: f64,
    ch: usize,
    style: usize,
    vertical: bool,
}

fn transform_run(t: Transform, s: &str, prev: Option<char>) -> String {
    match t {
        Transform::Upper => s.to_uppercase(),
        Transform::Lower => s.to_lowercase(),
        Transform::Capitalize => {
            let mut out = String::with_capacity(s.len());
            let mut start = prev.is_none_or(|c| c.is_whitespace());
            for c in s.chars() {
                if start && c.is_alphabetic() {
                    out.extend(c.to_uppercase());
                } else {
                    out.push(c);
                }
                start = c.is_whitespace();
            }
            out
        }
        _ => s.to_string(),
    }
}

fn hyphenator() -> Option<&'static hyphenation::Standard> {
    static D: OnceLock<Option<hyphenation::Standard>> = OnceLock::new();
    D.get_or_init(|| {
        use hyphenation::Load;
        hyphenation::Standard::from_embedded(hyphenation::Language::EnglishUS).ok()
    })
    .as_ref()
}

/// Lays out a paragraph, applying autoFit.
pub fn layout(lib: &mut FontLib, para: &Para) -> Layout {
    let o = &para.opts;
    let base = para.styles.iter().map(|s| s.size).fold(0.0f64, f64::max).max(1e-6);
    let lo = o.min_size.map(|m| m / base).unwrap_or(0.05);
    let hi = o.max_size.map(|m| m / base).unwrap_or(8.0);
    let (a, b) = match o.auto_fit {
        AutoFit::None => return layout_at(lib, para, 1.0).0,
        AutoFit::Shrink => {
            let (l, over) = layout_at(lib, para, 1.0);
            if !over {
                return l;
            }
            (lo.min(1.0), 1.0)
        }
        AutoFit::Grow => (1.0, hi.max(1.0)),
        AutoFit::Fit => (lo, hi),
    };
    // largest scale in [a, b] that fits
    let (mut lo_s, mut hi_s) = (a, b);
    if !layout_at(lib, para, lo_s).1 && !layout_at(lib, para, hi_s).1 {
        return layout_at(lib, para, hi_s).0;
    }
    for _ in 0..16 {
        let mid = (lo_s + hi_s) * 0.5;
        if layout_at(lib, para, mid).1 {
            hi_s = mid;
        } else {
            lo_s = mid;
        }
    }
    layout_at(lib, para, lo_s).0
}

/// Lays out at a size scale; returns the layout and whether it overflows the box.
pub fn layout_at(lib: &mut FontLib, para: &Para, k: f64) -> (Layout, bool) {
    let o = &para.opts;
    let vertical = o.writing != Writing::Horizontal;
    let styles: Vec<Style> = para.styles.iter().map(|s| Style { size: s.size * k, ..s.clone() }).collect();
    // logical text
    let mut chars: Vec<char> = Vec::new();
    let mut ch_style = Vec::new();
    let mut ch_span = Vec::new();
    for (ri, r) in para.runs.iter().enumerate() {
        let st = &styles[r.style.min(styles.len() - 1)];
        let t = transform_run(st.transform, &r.text, chars.last().copied());
        for c in t.chars() {
            chars.push(c);
            ch_style.push(r.style.min(styles.len() - 1));
            ch_span.push(ri);
        }
    }
    let text: String = chars.iter().collect();
    let byte_of: Vec<usize> = text.char_indices().map(|(b, _)| b).chain(std::iter::once(text.len())).collect();
    let char_at = |b: usize| byte_of.partition_point(|&x| x < b);
    let n = chars.len();
    let mut out = Layout {
        chars: chars.clone(),
        char_span: ch_span.clone(),
        scale: k,
        size: [o.width, o.height],
        styles: styles.clone(),
        vertical,
        ..Default::default()
    };
    // words
    let mut char_word = vec![0usize; n];
    let mut w = 0usize;
    let mut in_word = false;
    for (i, c) in chars.iter().enumerate() {
        if c.is_whitespace() {
            if in_word {
                w += 1;
            }
            in_word = false;
        } else {
            in_word = true;
        }
        char_word[i] = w;
    }
    out.word_count = if n == 0 { 0 } else { w + in_word as usize };
    out.char_word = char_word.clone();
    if n == 0 {
        return (out, false);
    }
    // faces
    let primary: Vec<Option<usize>> = styles.iter().map(|s| lib.select(s)).collect();
    let Some(any_face) = primary.iter().flatten().next().copied() else { return (out, false) };
    let ch_face: Vec<usize> = (0..n)
        .map(|i| {
            let p = primary[ch_style[i]].unwrap_or(any_face);
            lib.face_for(p, &styles[ch_style[i]], chars[i], o.emoji_color)
        })
        .collect();
    // bidi
    let default = match o.direction {
        Dir::Ltr => Some(Level::ltr()),
        Dir::Rtl => Some(Level::rtl()),
        Dir::Auto => None,
    };
    let bidi = BidiInfo::new(&text, default);
    let rtl = bidi.paragraphs.first().is_some_and(|p| p.level.is_rtl());
    out.align_shift = match (o.align, rtl) {
        (Align::Center, _) => 0.5,
        (Align::End, false) | (Align::Start | Align::Justify, true) => 1.0,
        _ => 0.0,
    };
    let level = |i: usize| bidi.levels.get(byte_of[i]).copied().unwrap_or(Level::ltr());
    // items and shaping
    let mut shaped: Vec<Vec<Shaped>> = Vec::new();
    let mut items: Vec<(usize, usize, bool)> = Vec::new(); // char range, rtl
    let mut i = 0;
    while i < n {
        let mut j = i + 1;
        while j < n
            && ch_style[j] == ch_style[i]
            && ch_face[j] == ch_face[i]
            && level(j) == level(i)
            && chars[j] != '\n'
            && chars[i] != '\n'
        {
            j += 1;
        }
        let st = &styles[ch_style[i]];
        let rtl = level(i).is_rtl();
        let slice = &text[byte_of[i]..byte_of[j]];
        let face = ch_face[i];
        let mut feats: Vec<rustybuzz::Feature> = st.features.iter().filter_map(|f| f.parse().ok()).collect();
        if st.transform == Transform::SmallCaps {
            if let Ok(f) = "smcp".parse() {
                feats.push(f);
            }
        }
        let spacing = o.letter_spacing + st.tracking / 1000.0 * st.size;
        let glyphs = lib
            .with_face(face, &st.variations, |f| {
                let mut buf = rustybuzz::UnicodeBuffer::new();
                buf.push_str(slice);
                buf.set_direction(if vertical {
                    rustybuzz::Direction::TopToBottom
                } else if rtl {
                    rustybuzz::Direction::RightToLeft
                } else {
                    rustybuzz::Direction::LeftToRight
                });
                if let Some(l) = o.language.as_deref().and_then(|l| l.parse::<rustybuzz::Language>().ok()) {
                    buf.set_language(l);
                }
                buf.guess_segment_properties();
                let res = rustybuzz::shape(f, &feats, buf);
                let sc = st.size / f.units_per_em() as f64;
                let infos = res.glyph_infos();
                let pos = res.glyph_positions();
                let mut v: Vec<Shaped> = infos
                    .iter()
                    .zip(pos)
                    .map(|(gi, gp)| {
                        let id = rustybuzz::ttf_parser::GlyphId(gi.glyph_id as u16);
                        // vertical: upright glyphs centred on the column, top at the pen
                        let (adv, dx, dy) = if vertical {
                            let v = f
                                .glyph_ver_advance(id)
                                .filter(|v| *v > 0)
                                .map(f64::from)
                                .unwrap_or((f.ascender() as f64 - f.descender() as f64).max(1.0));
                            let h = f.glyph_hor_advance(id).map(f64::from).unwrap_or(0.0);
                            (v * sc, -h * 0.5 * sc, f.ascender() as f64 * sc)
                        } else {
                            (gp.x_advance as f64 * sc, gp.x_offset as f64 * sc, -gp.y_offset as f64 * sc)
                        };
                        (gi, adv, dx, dy)
                    })
                    .map(|(gi, adv, dx, dy)| Shaped {
                        face,
                        gid: gi.glyph_id as u16,
                        adv,
                        dx,
                        dy,
                        ch: i + slice[..gi.cluster as usize].chars().count(),
                        style: ch_style[i],
                        vertical,
                    })
                    .collect();
                // letter spacing on the last glyph of each cluster
                let len = v.len();
                for g in 0..len {
                    let last = g + 1 == len || v[g + 1].ch != v[g].ch;
                    if last && spacing != 0.0 {
                        v[g].adv += spacing;
                    }
                }
                v
            })
            .unwrap_or_default();
        shaped.push(glyphs);
        items.push((i, j, rtl));
        i = j;
    }
    // advance per character (ligatures credit their first character)
    let mut adv = vec![0.0f64; n];
    for it in &shaped {
        for g in it {
            adv[g.ch] += g.adv;
        }
    }
    // break opportunities
    let mut allowed = vec![false; n + 1];
    let mut mandatory = vec![false; n + 1];
    for (b, op) in unicode_linebreak::linebreaks(&text) {
        let c = char_at(b);
        match op {
            BreakOpportunity::Mandatory => mandatory[c] = true,
            BreakOpportunity::Allowed => allowed[c] = true,
        }
    }
    if o.wrap == Wrap::Character {
        for (c, a) in allowed.iter_mut().enumerate().skip(1) {
            *a = c < n;
        }
    }
    if o.wrap == Wrap::None {
        allowed.iter_mut().for_each(|a| *a = false);
    }
    let limit = if o.wrap == Wrap::None {
        f64::INFINITY
    } else if vertical {
        o.height
    } else {
        o.width
    };
    let width_of = |a: usize, b: usize| -> f64 {
        let mut e = b;
        while e > a && (chars[e - 1].is_whitespace()) {
            e -= 1;
        }
        adv[a..e].iter().sum()
    };
    let hyphen_w = styles.iter().map(|s| s.size * 0.33).fold(0.0, f64::max);
    let break_lines = |lim: f64| -> Vec<(usize, usize, bool, bool)> {
        let mut lines = Vec::new();
        let mut a = 0;
        while a < n {
            let mut best: Option<usize> = None;
            let mut end = None;
            let mut hyph = false;
            let mut c = a + 1;
            while c <= n {
                if allowed[c] || mandatory[c] || c == n {
                    if width_of(a, c) <= lim {
                        best = Some(c);
                        if mandatory[c] {
                            end = Some((c, true));
                            break;
                        }
                    } else {
                        // the segment ending here overflows
                        let from = best.unwrap_or(a);
                        if o.hyphenate {
                            if let Some(h) = hyphen_point(&chars, from, c, |x| width_of(a, x) + hyphen_w <= lim) {
                                end = Some((h, false));
                                hyph = true;
                                break;
                            }
                        }
                        if let Some(bb) = best {
                            end = Some((bb, false));
                        } else {
                            // emergency: as many characters as fit, at least one
                            let mut e = a + 1;
                            while e < c && width_of(a, e + 1) <= lim {
                                e += 1;
                            }
                            end = Some((e, false));
                        }
                        break;
                    }
                }
                c += 1;
            }
            let (e, m) = end.unwrap_or((best.unwrap_or(n), false));
            let e = e.max(a + 1);
            lines.push((a, e, hyph, m || e == n));
            a = e;
        }
        lines
    };
    let mut lines = break_lines(limit);
    if o.wrap == Wrap::Balance && limit.is_finite() && lines.len() > 1 {
        let count = lines.len();
        let (mut lo_w, mut hi_w) = (limit * 0.25, limit);
        for _ in 0..14 {
            let mid = (lo_w + hi_w) * 0.5;
            if break_lines(mid).len() > count {
                lo_w = mid;
            } else {
                hi_w = mid;
            }
        }
        lines = break_lines(hi_w);
    }
    // line metrics
    let metrics = |face: usize, size: f64| -> (f64, f64) {
        lib.with_face(face, &[], |f| {
            let sc = size / f.units_per_em() as f64;
            (f.ascender() as f64 * sc, -(f.descender() as f64) * sc)
        })
        .unwrap_or((size * 0.8, size * 0.2))
    };
    let across = if vertical { o.width } else { o.height };
    let mut overflow = false;
    let mut truncated = false;
    if let Some(ml) = o.max_lines {
        if lines.len() > ml {
            lines.truncate(ml);
            truncated = true;
        }
    }
    // line heights and the lines that fit across
    let line_h: Vec<(f64, f64, f64)> = lines
        .iter()
        .map(|&(a, b, _, _)| {
            let (mut asc, mut desc, mut lh) = (0.0f64, 0.0f64, 0.0f64);
            for c in a..b.max(a + 1).min(n) {
                let st = &styles[ch_style[c]];
                let (ga, gd) = metrics(ch_face[c], st.size);
                asc = asc.max(ga);
                desc = desc.max(gd);
                lh = lh.max(st.line_height.unwrap_or(o.line_height) * st.size);
            }
            (asc, desc, lh)
        })
        .collect();
    let total: f64 = line_h.iter().map(|x| x.2).sum();
    if total > across + 1e-6 {
        overflow = true;
        if o.overflow != Overflow::Visible {
            let mut acc = 0.0;
            let mut keep = 0;
            for (k2, lh) in line_h.iter().enumerate() {
                acc += lh.2;
                if acc > across + 1e-6 && o.overflow == Overflow::Ellipsis {
                    break;
                }
                keep = k2 + 1;
                if acc > across + 1e-6 {
                    break;
                }
            }
            if keep < lines.len() {
                truncated = true;
                lines.truncate(keep.max(1));
            }
        }
    }
    if lines.iter().any(|&(a, b, _, _)| width_of(a, b) > limit + 1e-6) {
        overflow = true;
    }
    overflow |= truncated;
    out.truncated = truncated;
    out.clip = o.overflow == Overflow::Clip;
    // visual glyph order per line and positions
    let total: f64 = line_h[..lines.len()].iter().map(|x| x.2).sum();
    let mut v = if across.is_finite() {
        match o.valign {
            VAlign::Top => 0.0,
            VAlign::Middle => (across - total) * 0.5,
            VAlign::Bottom => across - total,
        }
    } else {
        0.0
    };
    let nlines = lines.len();
    for (li, &(a, b, hyph, hard)) in lines.iter().enumerate() {
        let (asc, desc, lh) = line_h[li];
        // glyphs of the line in visual order
        let para_i = bidi.paragraphs.iter().position(|p| p.range.contains(&byte_of[a])).unwrap_or(0);
        let para = &bidi.paragraphs[para_i];
        let mut e = b;
        while e > a && chars[e - 1].is_whitespace() {
            e -= 1;
        }
        let range = byte_of[a]..byte_of[e.max(a)];
        let mut order: Vec<&Shaped> = Vec::new();
        if range.start < range.end {
            let (levels, runs) = bidi.visual_runs(para, range.clone());
            for run in runs {
                let rtl = levels.get(run.start).is_some_and(|l| l.is_rtl());
                let (c0, c1) = (char_at(run.start), char_at(run.end));
                let mut its: Vec<usize> =
                    items.iter().enumerate().filter(|(_, it)| it.0 < c1 && it.1 > c0).map(|(k2, _)| k2).collect();
                if rtl {
                    its.reverse();
                }
                for k2 in its {
                    order.extend(shaped[k2].iter().filter(|g| g.ch >= c0 && g.ch < c1 && chars[g.ch] != '\n'));
                }
            }
        }
        let mut glyphs: Vec<LineGlyph> =
            order.iter().map(|g| (g.face, g.gid, g.adv, g.dx, g.dy, g.ch, g.style, g.vertical)).collect();
        // ellipsis or hyphen
        let last_style = glyphs.last().map(|g| g.6).unwrap_or(ch_style[a]);
        let extra = if truncated && li + 1 == nlines && o.overflow == Overflow::Ellipsis {
            Some('\u{2026}')
        } else if hyph {
            Some('-')
        } else {
            None
        };
        if let Some(xc) = extra {
            let st = &styles[last_style];
            let face = primary[last_style].unwrap_or(any_face);
            let face = lib.face_for(face, st, xc, false);
            let g = lib
                .with_face(face, &st.variations, |f| {
                    let sc = st.size / f.units_per_em() as f64;
                    let gid = f.glyph_index(xc).map(|g| g.0).unwrap_or(0);
                    let id = rustybuzz::ttf_parser::GlyphId(gid);
                    let a2: f64 = if vertical {
                        f.glyph_ver_advance(id).map(f64::from).unwrap_or(f.units_per_em() as f64)
                    } else {
                        f.glyph_hor_advance(id).map(f64::from).unwrap_or(0.0)
                    };
                    (gid, a2 * sc)
                })
                .unwrap_or((0, 0.0));
            if xc == '\u{2026}' && limit.is_finite() {
                while !glyphs.is_empty() && glyphs.iter().map(|x| x.2).sum::<f64>() + g.1 > limit {
                    glyphs.pop();
                }
            }
            let ch = glyphs.last().map(|x| x.5).unwrap_or(a);
            glyphs.push((face, g.0, g.1, 0.0, 0.0, ch, last_style, vertical));
        }
        let lw: f64 = glyphs.iter().map(|g| g.2).sum();
        let rtl_para = para.level.is_rtl();
        let start_side = |al: Align| -> f64 {
            // alignment is within the box even when lines do not wrap
            let bw = if vertical { o.height } else { o.width };
            let free = if limit.is_finite() {
                limit - lw
            } else if bw.is_finite() {
                bw - lw
            } else {
                0.0
            };
            match (al, rtl_para) {
                (Align::Center, _) => free * 0.5,
                (Align::End, false) | (Align::Start, true) => free,
                (Align::Justify, true) if hard => free,
                _ => 0.0,
            }
        };
        let mut u = start_side(o.align);
        let spaces = glyphs.iter().filter(|g| chars[g.5] == ' ').count();
        let gap = if o.align == Align::Justify && !hard && spaces > 0 && limit.is_finite() {
            (limit - lw) / spaces as f64
        } else {
            0.0
        };
        let first = out.glyphs.len();
        let line_top = v;
        let baseline = v + (lh - (asc + desc)) * 0.5 + asc;
        let (mut umin, mut umax) = (f64::INFINITY, f64::NEG_INFINITY);
        for g in glyphs {
            let st = &styles[g.6];
            let (x, y) = if vertical {
                // columns: rl from the right, lr from the left
                let col_c = match o.writing {
                    Writing::VerticalRl => o.width - (v + lh * 0.5),
                    _ => v + lh * 0.5,
                };
                (col_c + g.3, u + g.4)
            } else {
                (u + g.3, baseline + g.4 - st.baseline_shift)
            };
            umin = umin.min(u);
            umax = umax.max(u + g.2);
            out.glyphs.push(PGlyph {
                face: g.0,
                gid: g.1,
                x,
                y,
                size: st.size,
                style: g.6,
                ch: g.5,
                word: char_word[g.5],
                line: li,
                span: ch_span[g.5],
                advance: g.2,
                vertical: g.7,
            });
            u += g.2 + if chars[g.5] == ' ' { gap } else { 0.0 };
        }
        if umin > umax {
            umin = u;
            umax = u;
        }
        let rect = if vertical {
            let x0 = match o.writing {
                Writing::VerticalRl => o.width - v - lh,
                _ => v,
            };
            [x0, umin, lh, umax - umin]
        } else {
            [umin, line_top, umax - umin, lh]
        };
        out.lines.push(LineBox { rect, baseline, glyphs: first..out.glyphs.len() });
        v += lh;
    }
    // word boxes per line
    for lb in &out.lines {
        let mut k2 = lb.glyphs.start;
        while k2 < lb.glyphs.end {
            let wi = out.glyphs[k2].word;
            if chars[out.glyphs[k2].ch].is_whitespace() {
                k2 += 1;
                continue;
            }
            let mut j = k2;
            let (mut lo_u, mut hi_u) = (f64::INFINITY, f64::NEG_INFINITY);
            while j < lb.glyphs.end && out.glyphs[j].word == wi && !chars[out.glyphs[j].ch].is_whitespace() {
                let g = &out.glyphs[j];
                let (p0, p1) = if vertical { (g.y, g.y + g.advance) } else { (g.x, g.x + g.advance) };
                lo_u = lo_u.min(p0.min(p1));
                hi_u = hi_u.max(p0.max(p1));
                j += 1;
            }
            let r = if vertical {
                [lb.rect[0], lo_u, lb.rect[2], hi_u - lo_u]
            } else {
                [lo_u, lb.rect[1], hi_u - lo_u, lb.rect[3]]
            };
            out.words.push((r, wi));
            k2 = j.max(k2 + 1);
        }
    }
    (out, overflow)
}

/// The latest hyphenation point in the word overlapping `[from, to)` that satisfies `fits`.
fn hyphen_point(chars: &[char], from: usize, to: usize, fits: impl Fn(usize) -> bool) -> Option<usize> {
    let d = hyphenator()?;
    let mut s = from;
    while s < to && !chars[s].is_alphabetic() {
        s += 1;
    }
    let mut e = s;
    while e < chars.len() && chars[e].is_alphabetic() {
        e += 1;
    }
    if e - s < 5 {
        return None;
    }
    let word: String = chars[s..e].iter().collect();
    use hyphenation::Hyphenator;
    let lower = word.to_lowercase();
    let h = d.hyphenate(&lower);
    let bytes: Vec<usize> = word.char_indices().map(|(b, _)| b).collect();
    h.breaks.iter().rev().map(|&b| s + bytes.partition_point(|&x| x < b)).find(|&c| c > s && fits(c))
}
