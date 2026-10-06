//! PDF pages for `pdf` assets (SREP 17): one page rendered to the cache image, and the boxes of phrases on it.
//!
//! The renderer never reads a PDF; this module is the resolve step that does, once, and its results are pinned in
//! the document (the cache's SHA-256, its size and the regions' boxes). It is built on hayro (Apache-2.0 OR MIT), a
//! pure-Rust PDF interpreter and rasteriser, whose substitutes for the 14 standard fonts come from PDFium
//! (BSD-3-Clause).
//!
//! * **Page box:** the CropBox clipped to the MediaBox, turned clockwise by `/Rotate` (ISO 32000-2 §7.7.3.3 and
//!   §14.11.2). The image is ⌈W · dpi / 72⌉ × ⌈H · dpi / 72⌉ pixels for the turned box W × H in points.
//! * **Picture:** the page's content over `background`; annotations (their normal appearance) only when asked;
//!   optional content in its default state.
//! * **Characters:** in the order the interpreter draws them. A character's box is its advance from its origin
//!   across, and from 0.2 em below the baseline to 0.8 em above it, mapped to cache pixels. A line break is
//!   assumed where a character's baseline leaves the previous one's by more than half an em, and a word space where
//!   the gap to the previous character on the line exceeds 0.15 em.
//! * **Phrases:** both the page text and the phrase are normalised (NFKC per character; soft hyphens removed; a
//!   hyphen at a line end removed with the line break; every run of white space one space) and the
//!   `occurrence`-th case-sensitive match, left to right without overlap, gives the union of its characters' boxes,
//!   rounded to 0.1 px.

use std::fmt;

use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::{
    interpret_page, BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps, InterpreterCache,
    InterpreterSettings, SoftMask,
};
use hayro::hayro_syntax::page::Page;
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::{Affine, BezPath, Point, Rect};
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources};
use hayro::{RenderCache, RenderSettings};
use unicode_normalization::UnicodeNormalization;

/// Why a page or a phrase could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfError {
    /// The file is not a PDF hayro can read (or it is encrypted).
    Unreadable(String),
    /// The page number is outside the document.
    NoPage {
        /// Requested page, from 1.
        page: u32,
        /// Pages in the document.
        count: usize,
    },
    /// The page image would exceed the largest raster the renderer makes.
    TooLarge {
        /// Width in pixels.
        width: u64,
        /// Height in pixels.
        height: u64,
    },
    /// The phrase does not occur that often on the page.
    NoMatch {
        /// The phrase as written.
        text: String,
        /// The occurrence asked for.
        occurrence: u32,
        /// How many matches the page has.
        found: usize,
    },
}

impl fmt::Display for PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PdfError::Unreadable(e) => write!(f, "not a readable PDF: {e}"),
            PdfError::NoPage { page, count } => write!(f, "page {page} does not exist; the document has {count}"),
            PdfError::TooLarge { width, height } => {
                write!(f, "the page image would be {width} x {height} pixels, more than {MAX_SIDE} on a side")
            }
            PdfError::NoMatch { text, occurrence, found } => {
                write!(f, "{text:?}: occurrence {occurrence} asked, the page has {found}")
            }
        }
    }
}

impl std::error::Error for PdfError {}

/// The longest side of a page image (the rasteriser's 16-bit limit).
pub const MAX_SIDE: u64 = u16::MAX as u64;

/// A rendered page: straight-alpha sRGB RGBA8, row by row.
#[derive(Debug, Clone, PartialEq)]
pub struct PageImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixels.
    pub rgba: Vec<u8>,
}

/// One character of the page text and its box in cache pixels (x0, y0, x1, y1), when it was drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct PageChar {
    /// The character's text (one or more code points from the font's Unicode mapping).
    pub text: String,
    /// Box in cache pixels.
    pub rect: [f64; 4],
    /// Baseline origin in cache pixels.
    origin: [f64; 2],
    /// Em size in cache pixels.
    em: f64,
}

fn open(bytes: &[u8]) -> Result<Pdf, PdfError> {
    Pdf::new(bytes.to_vec()).map_err(|e| PdfError::Unreadable(format!("{e:?}")))
}

fn page_of<'a>(pdf: &'a Pdf, page: u32) -> Result<&'a Page<'a>, PdfError> {
    let pages = pdf.pages();
    let count = pages.len();
    if page == 0 || page as usize > count {
        return Err(PdfError::NoPage { page, count });
    }
    Ok(&pages[page as usize - 1])
}

/// The image size for `page` at `dpi`, and the transform from PDF user space to its pixels.
fn geometry(page: &Page, dpi: f64) -> Result<(u16, u16, Affine), PdfError> {
    let (w, h) = page.render_dimensions();
    let s = dpi / 72.0;
    let (pw, ph) = ((w as f64 * s).ceil().max(1.0) as u64, (h as f64 * s).ceil().max(1.0) as u64);
    if pw > MAX_SIDE || ph > MAX_SIDE {
        return Err(PdfError::TooLarge { width: pw, height: ph });
    }
    let t = Affine::scale(s) * Affine::new(page.initial_transform(true).as_coeffs());
    Ok((pw as u16, ph as u16, t))
}

/// Renders page `page` (from 1) at `dpi` over `background` (straight sRGB RGBA8), with annotations when asked.
pub fn render_page(
    bytes: &[u8],
    page: u32,
    dpi: f64,
    background: [u8; 4],
    annotations: bool,
) -> Result<PageImage, PdfError> {
    let pdf = open(bytes)?;
    let page = page_of(&pdf, page)?;
    let (w, h, transform) = geometry(page, dpi)?;
    let settings = InterpreterSettings { render_annotations: annotations, ..InterpreterSettings::default() };
    let cache = RenderCache::new();
    let mut ctx = RenderContext::new(w, h);
    hayro::render_into(page, &cache, &settings, &RenderSettings::default(), &mut ctx, transform);
    ctx.flush();
    let mut pixmap = Pixmap::new(w, h);
    ctx.render_with(&mut pixmap, &mut Resources::default(), RasterizerSettings::default());
    // the pixmap holds premultiplied RGBA8
    let rgba = over(pixmap.data_as_u8_slice(), background);
    Ok(PageImage { width: w as u32, height: h as u32, rgba })
}

/// `page` (premultiplied RGBA8) composited over `bg` (straight RGBA8), returned straight.
fn over(page: &[u8], bg: [u8; 4]) -> Vec<u8> {
    let ba = bg[3] as f64 / 255.0;
    let mut out = Vec::with_capacity(page.len());
    for p in page.as_chunks::<4>().0 {
        let a = p[3] as f64 / 255.0;
        let alpha = a + ba * (1.0 - a);
        for c in 0..3 {
            let premul = p[c] as f64 / 255.0 + bg[c] as f64 / 255.0 * ba * (1.0 - a);
            let straight = if alpha > 0.0 { premul / alpha } else { 0.0 };
            out.push((straight * 255.0).round().clamp(0.0, 255.0) as u8);
        }
        out.push((alpha * 255.0).round().clamp(0.0, 255.0) as u8);
    }
    out
}

/// Collects the drawn characters of a page through hayro's device interface.
struct Chars {
    out: Vec<PageChar>,
}

impl Device<'_> for Chars {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'_>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn draw_image(&mut self, _: Image<'_, '_>, _: ImageDrawProps<'_>) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, '_>, props: DrawProps<'_>, _: &DrawMode) {
        use hayro::hayro_interpret::font::Glyph;
        for g in run.glyphs() {
            let Some(u) = g.as_unicode() else { continue };
            let text = match u {
                hayro::hayro_interpret::hayro_cmap::BfString::Char(c) => c.to_string(),
                hayro::hayro_interpret::hayro_cmap::BfString::String(s) => s,
            };
            // glyph space is 1000 units per em
            let t = props.transform * g.transform();
            let advance = match &**g {
                Glyph::Outline(o) => o.advance_width().map(f64::from).unwrap_or(0.0),
                Glyph::Type3(_) => 0.0,
            };
            let corners = [
                Point::new(0.0, -200.0),
                Point::new(advance, -200.0),
                Point::new(advance, 800.0),
                Point::new(0.0, 800.0),
            ]
            .map(|p| t * p);
            let mut r = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
            for p in corners {
                r = [r[0].min(p.x), r[1].min(p.y), r[2].max(p.x), r[3].max(p.y)];
            }
            let o = t * Point::ZERO;
            let up = t * Point::new(0.0, 1000.0);
            let em = (up - o).hypot();
            self.out.push(PageChar { text, rect: r, origin: [o.x, o.y], em });
        }
    }
}

/// The characters of page `page` (from 1) with their boxes in the pixels of its image at `dpi`.
pub fn page_chars(bytes: &[u8], page: u32, dpi: f64) -> Result<Vec<PageChar>, PdfError> {
    let pdf = open(bytes)?;
    let page = page_of(&pdf, page)?;
    let (w, h, transform) = geometry(page, dpi)?;
    let cache = InterpreterCache::new();
    let settings = InterpreterSettings::default();
    let mut ctx = Context::new(transform, Rect::new(0.0, 0.0, w as f64, h as f64), &cache, page.xref(), settings);
    let mut dev = Chars { out: Vec::new() };
    interpret_page(page, &mut ctx, &mut dev);
    Ok(dev.out)
}

/// One normalised character and the page character it comes from (None for inserted white space).
type Mapped = Vec<(char, Option<usize>)>;

/// The page text as the extractor reports it, with line breaks and word spaces inferred from positions.
fn page_text(chars: &[PageChar]) -> Mapped {
    let mut out: Mapped = Vec::new();
    let mut prev: Option<&PageChar> = None;
    for (i, c) in chars.iter().enumerate() {
        if let Some(p) = prev {
            let em = p.em.max(c.em).max(1e-9);
            if (c.origin[1] - p.origin[1]).abs() > 0.5 * em {
                out.push(('\n', None));
            } else if c.rect[0] - p.rect[2] > 0.15 * em {
                out.push((' ', None));
            }
        }
        out.extend(c.text.chars().map(|ch| (ch, Some(i))));
        prev = Some(c);
    }
    out
}

/// SREP 17 §2.5 normalisation, keeping where each character came from.
fn normalise(text: Mapped) -> Mapped {
    // NFKC, character by character so each result keeps its source
    let mut nfkc: Mapped = Vec::new();
    for (c, src) in text {
        nfkc.extend(c.to_string().nfkc().map(|d| (d, src)));
    }
    // soft hyphens go
    nfkc.retain(|(c, _)| *c != '\u{00AD}');
    // a hyphen at a line end goes with the line break (and the white space around it)
    let mut joined: Mapped = Vec::new();
    let mut i = 0;
    while i < nfkc.len() {
        if nfkc[i].0 == '-' {
            let mut j = i + 1;
            while j < nfkc.len() && nfkc[j].0 != '\n' && nfkc[j].0.is_whitespace() {
                j += 1;
            }
            if j < nfkc.len() && nfkc[j].0 == '\n' {
                j += 1;
                while j < nfkc.len() && nfkc[j].0.is_whitespace() {
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        joined.push(nfkc[i]);
        i += 1;
    }
    // every run of white space is one space
    let mut out: Mapped = Vec::new();
    for (c, src) in joined {
        if c.is_whitespace() {
            if out.last().is_some_and(|(p, _)| *p == ' ') {
                continue;
            }
            out.push((' ', src));
        } else {
            out.push((c, src));
        }
    }
    out
}

/// Normalises a phrase as the page text is normalised.
pub fn normalise_phrase(text: &str) -> String {
    normalise(text.chars().map(|c| (c, None)).collect()).into_iter().map(|(c, _)| c).collect()
}

/// The box (x, y, width, height), rounded to 0.1 px, of the `occurrence`-th match (from 1) of `phrase` among
/// `chars`.
pub fn find(chars: &[PageChar], phrase: &str, occurrence: u32) -> Result<[f64; 4], PdfError> {
    let page = normalise(page_text(chars));
    let hay: Vec<char> = page.iter().map(|(c, _)| *c).collect();
    let needle: Vec<char> = normalise_phrase(phrase).chars().collect();
    let mut found = Vec::new();
    if !needle.is_empty() {
        let mut i = 0;
        while i + needle.len() <= hay.len() {
            if hay[i..i + needle.len()] == needle[..] {
                found.push(i);
                i += needle.len();
            } else {
                i += 1;
            }
        }
    }
    let Some(&start) = found.get(occurrence.saturating_sub(1) as usize).filter(|_| occurrence > 0) else {
        return Err(PdfError::NoMatch { text: phrase.to_string(), occurrence, found: found.len() });
    };
    let mut r = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for (_, src) in &page[start..start + needle.len()] {
        if let Some(k) = src {
            let b = chars[*k].rect;
            r = [r[0].min(b[0]), r[1].min(b[1]), r[2].max(b[2]), r[3].max(b[3])];
        }
    }
    if !r[0].is_finite() {
        return Err(PdfError::NoMatch { text: phrase.to_string(), occurrence, found: 0 });
    }
    let tenth = |v: f64| (v * 10.0).round() / 10.0;
    Ok([tenth(r[0]), tenth(r[1]), tenth(r[2] - r[0]), tenth(r[3] - r[1])])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapped(s: &str) -> Mapped {
        s.chars().enumerate().map(|(i, c)| (c, Some(i))).collect()
    }

    fn text(m: &Mapped) -> String {
        m.iter().map(|(c, _)| *c).collect()
    }

    #[test]
    fn normalisation() {
        assert_eq!(text(&normalise(mapped("ﬁne  print\n\tand\u{00AD}more"))), "fine print andmore");
        assert_eq!(text(&normalise(mapped("exam-\nple, two-part"))), "example, two-part");
        assert_eq!(text(&normalise(mapped("exam- \n  ple"))), "example");
        assert_eq!(normalise_phrase("  Ⅳ  ﬂow "), " IV flow ");
        // a ligature keeps its source for both letters
        let n = normalise(mapped("aﬁb"));
        assert_eq!(n.iter().map(|(_, s)| *s).collect::<Vec<_>>(), vec![Some(0), Some(1), Some(1), Some(2)]);
    }

    #[test]
    fn over_background() {
        // transparent page: the background; opaque page: the page
        assert_eq!(over(&[0, 0, 0, 0], [255, 255, 255, 255]), vec![255, 255, 255, 255]);
        assert_eq!(over(&[10, 20, 30, 255], [255, 255, 255, 255]), vec![10, 20, 30, 255]);
        // half-covered black over white is mid grey
        assert_eq!(over(&[0, 0, 0, 128], [255, 255, 255, 255]), vec![127, 127, 127, 255]);
        // over a transparent background the page keeps its alpha, straight
        assert_eq!(over(&[64, 0, 0, 128], [0, 0, 0, 0]), vec![128, 0, 0, 128]);
    }
}
