//! Font discovery and face access. Faces come from font files named by the
//! document (`fontFile`, `fontAsset`) and from the system font directories
//! through fontdb. Selection follows CSS matching (family list, then
//! weight, style and stretch); characters a face lacks fall back to the
//! style's other families, then to any installed face that has them, with
//! colour emoji faces preferred for emoji when `emoji="color"`.
//!
//! Under `fontPolicy="pinned"` (SREP 21) a library made with [`FontLib::pinned`] knows only the faces the document
//! pins with [`FontLib::pin`], under the family, weight and style their font assets declare: no host font is
//! consulted, families resolve to those faces only, a character falls back only through the style's own family
//! list, and one that no permitted face has is drawn as the primary face's `.notdef`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::style::Style;
use rustybuzz::ttf_parser;

/// The ascent and descent (positive below the baseline) a line of text is seated with, in font units (SREP 20): the
/// `hhea` ascender and descender of the face, or `OS/2` `usWinAscent` and `usWinDescent` when `hhea`'s are both 0.
/// Never the typographic metrics, even when `OS/2` sets USE_TYPO_METRICS: switching would move every line of text.
pub fn line_metrics(f: &ttf_parser::Face) -> (f64, f64) {
    let h = f.tables().hhea;
    if h.ascender != 0 || h.descender != 0 {
        return (h.ascender as f64, -(h.descender as f64));
    }
    match f.tables().os2 {
        Some(o) => (o.windows_ascender() as f64, -(o.windows_descender() as f64)),
        None => (0.0, 0.0),
    }
}

/// A loaded face.
#[derive(Debug, Clone)]
pub struct FaceData {
    pub data: Arc<Vec<u8>>,
    pub index: u32,
    pub family: String,
    /// Has colour glyph tables (COLR, CBDT or sbix).
    pub color: bool,
    /// The face's weight (OS/2 usWeightClass, through fontdb).
    pub weight: u16,
    /// The face is italic or oblique.
    pub italic: bool,
    /// The face has a `wght` variation axis, so it draws any weight in its range.
    pub variable_weight: bool,
}

/// A face pinned by a font asset (SREP 21): the family, weight and style the asset declares.
#[derive(Debug, Clone, PartialEq)]
struct Pin {
    family: String,
    weight: u16,
    italic: bool,
    face: usize,
}

/// A face drawn in place of what a style asked for (SREP 21 `FONT-SUB`).
#[derive(Debug, Clone, PartialEq)]
pub struct Substitution {
    /// What the style asked for: family, weight and style.
    pub requested: String,
    /// The face drawn.
    pub drawn: String,
}

/// Characters that draw nothing of their own, so no face needs a glyph for them: white space, controls, and the
/// default-ignorable joiners, selectors and marks.
pub fn invisible(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(c as u32, 0xAD | 0x34F | 0x180B..=0x180F | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0xFE00..=0xFE0F | 0xFEFF | 0xE0000..=0xE0FFF)
}

/// Generic family names, which name no face of their own.
pub const GENERIC_FAMILIES: [&str; 6] = ["sans-serif", "serif", "monospace", "cursive", "fantasy", "system-ui"];

/// Fonts known to a render: system faces and document files.
pub struct FontLib {
    db: fontdb::Database,
    faces: Vec<FaceData>,
    by_id: HashMap<fontdb::ID, usize>,
    files: HashMap<(PathBuf, u32), Option<usize>>,
    select_cache: Mutex<HashMap<String, Option<usize>>>,
    cover: Mutex<HashMap<(usize, char), bool>>,
    fallback: Mutex<HashMap<(char, bool), Option<usize>>>,
    /// Outline cache: (face, glyph, variation key) → path in font units (y up).
    pub(crate) outlines: Mutex<HashMap<(usize, u16, u64), Arc<sr_vector::Path>>>,
    /// `fontPolicy="pinned"`: the only faces there are, in the order the document pins them.
    pins: Option<Vec<Pin>>,
}

fn has_color(face: &ttf_parser::Face) -> bool {
    let t = face.tables();
    t.colr.is_some() || t.cbdt.is_some() || t.sbix.is_some()
}

impl FontLib {
    /// A library with the system fonts (`system` = false starts empty, for tests with explicit files).
    pub fn new(system: bool) -> FontLib {
        let mut db = fontdb::Database::new();
        if system {
            db.load_system_fonts();
        }
        // generic families: the first installed of common choices, so documents that say "sans-serif" render the same on similar systems
        let pick = |db: &fontdb::Database, names: &[&str]| {
            names
                .iter()
                .find(|n| db.faces().any(|f| f.families.iter().any(|(fam, _)| fam == *n)))
                .map(|s| s.to_string())
        };
        if let Some(n) = pick(&db, &["DejaVu Sans", "Liberation Sans", "Noto Sans", "Arial", "Helvetica"]) {
            db.set_sans_serif_family(n);
        }
        if let Some(n) = pick(&db, &["DejaVu Serif", "Liberation Serif", "Noto Serif", "Times New Roman"]) {
            db.set_serif_family(n);
        }
        if let Some(n) = pick(&db, &["DejaVu Sans Mono", "Liberation Mono", "Noto Sans Mono", "Courier New"]) {
            db.set_monospace_family(n);
        }
        FontLib {
            db,
            faces: Vec::new(),
            by_id: HashMap::new(),
            files: HashMap::new(),
            select_cache: Mutex::new(HashMap::new()),
            cover: Mutex::new(HashMap::new()),
            fallback: Mutex::new(HashMap::new()),
            outlines: Mutex::new(HashMap::new()),
            pins: None,
        }
    }

    /// A library for `fontPolicy="pinned"` (SREP 21): no host fonts, fontconfig included; only the faces
    /// [`FontLib::pin`] adds.
    pub fn pinned() -> FontLib {
        FontLib { pins: Some(Vec::new()), ..FontLib::new(false) }
    }

    /// Whether this library is pinned.
    pub fn is_pinned(&self) -> bool {
        self.pins.is_some()
    }

    /// Adds the face of a font asset (file `path`, collection `index`) under the family, weight and style the asset
    /// declares. Returns the face, or `None` when the file is not a font this library can read.
    pub fn pin(&mut self, path: &Path, index: u32, family: &str, weight: u16, italic: bool) -> Option<usize> {
        let face = self.file(path, index)?;
        if let Some(pins) = &mut self.pins {
            pins.push(Pin { family: family.to_lowercase(), weight, italic, face });
        }
        Some(face)
    }

    /// The pinned face of `family` nearest the style's weight and slant, when a font asset declares that family.
    fn pinned_family(&self, family: &str, st: &Style) -> Option<usize> {
        let fam = family.to_lowercase();
        self.pins
            .as_ref()?
            .iter()
            .filter(|p| p.family == fam)
            .min_by_key(|p| (p.italic != st.italic, (p.weight as i32 - st.weight as i32).abs()))
            .map(|p| p.face)
    }

    /// Faces loaded so far.
    pub fn face(&self, i: usize) -> &FaceData {
        &self.faces[i]
    }

    fn load_id(&mut self, id: fontdb::ID) -> Option<usize> {
        if let Some(&i) = self.by_id.get(&id) {
            return Some(i);
        }
        let family = self.db.face(id).and_then(|f| f.families.first().map(|x| x.0.clone())).unwrap_or_default();
        let (weight, italic) =
            self.db.face(id).map(|f| (f.weight.0, f.style != fontdb::Style::Normal)).unwrap_or((400, false));
        let (data, index) = self.db.with_face_data(id, |d, i| (d.to_vec(), i))?;
        let (color, variable_weight) = ttf_parser::Face::parse(&data, index)
            .map(|f| {
                (has_color(&f), f.variation_axes().into_iter().any(|a| a.tag == ttf_parser::Tag::from_bytes(b"wght")))
            })
            .ok()?;
        self.faces.push(FaceData { data: Arc::new(data), index, family, color, weight, italic, variable_weight });
        self.by_id.insert(id, self.faces.len() - 1);
        Some(self.faces.len() - 1)
    }

    /// Loads a font file (collection `index`); returns the face.
    pub fn file(&mut self, path: &Path, index: u32) -> Option<usize> {
        let key = (path.to_path_buf(), index);
        if let Some(r) = self.files.get(&key) {
            return *r;
        }
        let ids = self.db.load_font_source(fontdb::Source::File(path.to_path_buf()));
        let id = ids
            .iter()
            .copied()
            .find(|id| self.db.face(*id).is_some_and(|f| f.index == index))
            .or_else(|| ids.first().copied());
        let r = id.and_then(|id| self.load_id(id));
        self.files.insert(key, r);
        r
    }

    /// The primary face of a style. Pinned: the style's file (a font asset), else the first of its families a font
    /// asset declares, else the document's first pinned face.
    pub fn select(&mut self, st: &Style) -> Option<usize> {
        if let Some((p, i)) = &st.file {
            if let Some(f) = self.file(p, *i) {
                return Some(f);
            }
        }
        if let Some(pins) = &self.pins {
            let first = pins.first().map(|p| p.face);
            return st.families.iter().find_map(|f| self.pinned_family(f, st)).or(first);
        }
        let key = format!("{:?}|{}|{}|{}", st.families, st.weight, st.italic, st.stretch);
        if let Some(r) = self.select_cache.lock().unwrap().get(&key) {
            return *r;
        }
        let mut found = None;
        for fam in st.families.iter().map(String::as_str).chain(["sans-serif"]) {
            let family = match fam.to_ascii_lowercase().as_str() {
                "sans-serif" | "system-ui" => fontdb::Family::SansSerif,
                "serif" => fontdb::Family::Serif,
                "monospace" => fontdb::Family::Monospace,
                "cursive" => fontdb::Family::Cursive,
                "fantasy" => fontdb::Family::Fantasy,
                _ => fontdb::Family::Name(fam),
            };
            let q = fontdb::Query {
                families: &[family],
                weight: fontdb::Weight(st.weight),
                stretch: stretch_of(st.stretch),
                style: if st.italic { fontdb::Style::Italic } else { fontdb::Style::Normal },
            };
            if let Some(id) = self.db.query(&q) {
                found = self.load_id(id);
                if found.is_some() {
                    break;
                }
            }
        }
        if found.is_none() {
            // anything at all
            let first = self.db.faces().next().map(|f| f.id);
            found = first.and_then(|id| self.load_id(id));
        }
        self.select_cache.lock().unwrap().insert(key, found);
        found
    }

    /// The face `face` drawn for style `st`, when it is not what the style asked for: another family (a named one),
    /// another weight (unless the face varies its weight) or another slant. A face from the file the style names is
    /// what it asked for.
    pub fn substitution(&self, st: &Style, face: usize) -> Option<Substitution> {
        // a style that names its file (fontAsset, fontFile) asked for that face
        if st.file.as_ref().is_some_and(|(p, i)| self.files.get(&(p.clone(), *i)) == Some(&Some(face))) {
            return None;
        }
        let f = &self.faces[face];
        let asked = st.families.first().map(String::as_str).unwrap_or("sans-serif");
        let generic = GENERIC_FAMILIES.iter().any(|g| g.eq_ignore_ascii_case(asked));
        let pinned_as = self.pins.as_ref().and_then(|p| p.iter().find(|p| p.face == face)).map(|p| p.family.clone());
        let family_ok = generic
            || f.family.eq_ignore_ascii_case(asked)
            || pinned_as.as_deref().is_some_and(|p| p.eq_ignore_ascii_case(asked));
        let weight = self.pins.as_ref().and_then(|p| p.iter().find(|p| p.face == face)).map_or(f.weight, |p| p.weight);
        let weight_ok = f.variable_weight || weight == st.weight;
        let slant_ok = f.italic == st.italic;
        if family_ok && weight_ok && slant_ok {
            return None;
        }
        let slant = |i: bool| if i { "italic" } else { "normal" };
        let drawn_family = pinned_as.unwrap_or_else(|| f.family.clone());
        Some(Substitution {
            requested: format!("{asked} {} {}", st.weight, slant(st.italic)),
            drawn: format!("{drawn_family} {weight} {}", slant(f.italic)),
        })
    }

    /// Whether face `f` maps `ch`.
    pub fn covers(&self, f: usize, ch: char) -> bool {
        if let Some(&c) = self.cover.lock().unwrap().get(&(f, ch)) {
            return c;
        }
        let fd = &self.faces[f];
        let c = ttf_parser::Face::parse(&fd.data, fd.index).map(|face| face.glyph_index(ch).is_some()).unwrap_or(false);
        self.cover.lock().unwrap().insert((f, ch), c);
        c
    }

    /// The face that draws `ch` for a style whose primary face is `primary`.
    pub fn face_for(&mut self, primary: usize, st: &Style, ch: char, color_emoji: bool) -> usize {
        let emoji = color_emoji && is_emoji(ch);
        if !emoji && (ch.is_whitespace() || ch.is_control() || self.covers(primary, ch)) {
            return primary;
        }
        if emoji && self.faces[primary].color && self.covers(primary, ch) {
            return primary;
        }
        if self.pins.is_some() {
            // only the style's own fallback families, then the primary face's .notdef
            return st
                .families
                .iter()
                .skip(1)
                .filter_map(|f| self.pinned_family(f, st))
                .find(|&f| self.covers(f, ch))
                .unwrap_or(primary);
        }
        // other families of the style
        for fam in st.families.iter().skip(1) {
            let alt = Style { families: vec![fam.clone()], file: None, ..st.clone() };
            if let Some(f) = self.select(&alt) {
                if self.covers(f, ch) && (!emoji || self.faces[f].color) {
                    return f;
                }
            }
        }
        if let Some(r) = self.fallback.lock().unwrap().get(&(ch, emoji)) {
            return r.unwrap_or(primary);
        }
        // any installed face; colour faces first for emoji
        let ids: Vec<fontdb::ID> = self.db.faces().map(|f| f.id).collect();
        let mut best = None;
        for pass in 0..2 {
            for &id in &ids {
                let Some(f) = self.load_id_lazy(id, ch) else { continue };
                if pass == 0 && emoji && !self.faces[f].color {
                    continue;
                }
                best = Some(f);
                break;
            }
            if best.is_some() || !emoji {
                break;
            }
        }
        self.fallback.lock().unwrap().insert((ch, emoji), best);
        best.unwrap_or(primary)
    }

    /// Loads a face only when it maps `ch` (checked on the database's memory map first).
    fn load_id_lazy(&mut self, id: fontdb::ID, ch: char) -> Option<usize> {
        if let Some(&i) = self.by_id.get(&id) {
            return self.covers(i, ch).then_some(i);
        }
        let has = self.db.with_face_data(id, |d, i| {
            ttf_parser::Face::parse(d, i).map(|f| f.glyph_index(ch).is_some()).unwrap_or(false)
        })?;
        if !has {
            return None;
        }
        self.load_id(id)
    }

    /// Runs `f` with a parsed face (variations applied).
    pub fn with_face<R>(
        &self,
        i: usize,
        variations: &[([u8; 4], f32)],
        f: impl FnOnce(&rustybuzz::Face) -> R,
    ) -> Option<R> {
        let fd = &self.faces[i];
        let mut face = rustybuzz::Face::from_slice(&fd.data, fd.index)?;
        if !variations.is_empty() {
            let v: Vec<rustybuzz::Variation> = variations
                .iter()
                .map(|(t, x)| rustybuzz::Variation { tag: ttf_parser::Tag::from_bytes(t), value: *x })
                .collect();
            face.set_variations(&v);
        }
        Some(f(&face))
    }

    /// Number of faces loaded.
    pub fn loaded(&self) -> usize {
        self.faces.len()
    }

    /// Families of every installed face (sorted, unique).
    pub fn families(&self) -> Vec<String> {
        let mut v: Vec<String> = self.db.faces().filter_map(|f| f.families.first().map(|x| x.0.clone())).collect();
        v.sort();
        v.dedup();
        v
    }

    /// The first installed face with an OpenType MATH table (formula fallback).
    pub fn math_face(&mut self) -> Option<usize> {
        let mut ids: Vec<(String, fontdb::ID)> =
            self.db.faces().map(|f| (f.families.first().map(|x| x.0.clone()).unwrap_or_default(), f.id)).collect();
        ids.sort();
        for (_, id) in ids {
            let has = self
                .db
                .with_face_data(id, |d, i| {
                    ttf_parser::Face::parse(d, i).map(|f| f.tables().math.is_some()).unwrap_or(false)
                })
                .unwrap_or(false);
            if has {
                return self.load_id(id);
            }
        }
        None
    }
}

fn stretch_of(s: f64) -> fontdb::Stretch {
    use fontdb::Stretch::*;
    match s {
        x if x < 0.5625 => UltraCondensed,
        x if x < 0.6875 => ExtraCondensed,
        x if x < 0.8125 => Condensed,
        x if x < 0.9375 => SemiCondensed,
        x if x < 1.0625 => Normal,
        x if x < 1.1875 => SemiExpanded,
        x if x < 1.375 => Expanded,
        x if x < 1.75 => ExtraExpanded,
        _ => UltraExpanded,
    }
}

/// Characters drawn with emoji presentation by default.
pub fn is_emoji(ch: char) -> bool {
    matches!(ch as u32, 0x1F300..=0x1FAFF | 0x1F000..=0x1F2FF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0xFE0F | 0x200D)
}
