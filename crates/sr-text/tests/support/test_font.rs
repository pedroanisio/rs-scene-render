//! A pinned TrueType test font, built in memory (SREP 20, Conformance): unitsPerEm 1000, `hhea` ascender 800 and
//! descender −200, every glyph of U+0021 to U+007E a rectangle from x 50 to 550 and from the baseline up to 700
//! units, advance 600; the space has the advance and no outline. Glyph 0, `.notdef`, is a hollow box (outer 50–550
//! × 0–700, inner 100–500 × 50–650), so a character the font lacks is measurable (SREP 21).
//!
//! The `OS/2` table carries other vertical metrics on purpose (typographic 700/−300, Windows 900/200), which seat a
//! line elsewhere than `hhea` does, so a test can tell which metrics an engine uses.

#![allow(dead_code)]

/// Vertical metrics of the font, in font units.
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    /// `hhea.ascender`.
    pub hhea_ascender: i16,
    /// `hhea.descender` (negative below the baseline).
    pub hhea_descender: i16,
    /// `OS/2.sTypoAscender`.
    pub typo_ascender: i16,
    /// `OS/2.sTypoDescender`.
    pub typo_descender: i16,
    /// `OS/2.usWinAscent`.
    pub win_ascent: u16,
    /// `OS/2.usWinDescent` (positive below the baseline).
    pub win_descent: u16,
    /// `OS/2.fsSelection` bit 7, USE_TYPO_METRICS.
    pub use_typo_metrics: bool,
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics {
            hhea_ascender: 800,
            hhea_descender: -200,
            typo_ascender: 700,
            typo_descender: -300,
            win_ascent: 900,
            win_descent: 200,
            use_typo_metrics: false,
        }
    }
}

const UPEM: u16 = 1000;
const ADVANCE: u16 = 600;
const FIRST: u16 = 0x20;
const LAST: u16 = 0x7E;

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn be32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

/// A simple glyph from closed contours of on-curve points.
fn glyph(contours: &[&[(i16, i16)]]) -> Vec<u8> {
    let pts: Vec<(i16, i16)> = contours.iter().flat_map(|c| c.iter().copied()).collect();
    let mut v = Vec::new();
    be16(&mut v, contours.len() as u16);
    let (xs, ys): (Vec<i16>, Vec<i16>) = pts.iter().copied().unzip();
    for b in
        [*xs.iter().min().unwrap(), *ys.iter().min().unwrap(), *xs.iter().max().unwrap(), *ys.iter().max().unwrap()]
    {
        be16(&mut v, b as u16);
    }
    let mut end = 0u16;
    for c in contours {
        end += c.len() as u16;
        be16(&mut v, end - 1);
    }
    be16(&mut v, 0); // no instructions
    v.extend(std::iter::repeat_n(0x01u8, pts.len())); // on curve, 16-bit deltas
    let (mut px, mut py) = (0i16, 0i16);
    for &(x, _) in &pts {
        be16(&mut v, (x - px) as u16);
        px = x;
    }
    for &(_, y) in &pts {
        be16(&mut v, (y - py) as u16);
        py = y;
    }
    while v.len() % 4 != 0 {
        v.push(0);
    }
    v
}

fn name_table(family: &str) -> Vec<u8> {
    let names: [(u16, String); 4] =
        [(1, family.into()), (2, "Regular".into()), (4, format!("{family} Regular")), (6, family.replace(' ', ""))];
    let mut strings = Vec::new();
    let mut records = Vec::new();
    for (id, s) in &names {
        let utf16: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        for x in [3u16, 1, 0x409, *id, utf16.len() as u16, strings.len() as u16] {
            be16(&mut records, x);
        }
        strings.extend(utf16);
    }
    let mut v = Vec::new();
    be16(&mut v, 0);
    be16(&mut v, names.len() as u16);
    be16(&mut v, 6 + records.len() as u16);
    v.extend(records);
    v.extend(strings);
    v
}

/// The font's bytes, with family name `family`.
pub fn build(family: &str, m: Metrics) -> Vec<u8> {
    let rect: &[(i16, i16)] = &[(50, 0), (50, 700), (550, 700), (550, 0)];
    let hole: &[(i16, i16)] = &[(100, 50), (500, 50), (500, 650), (100, 650)];
    // glyph 0 .notdef, then U+0020 (empty) to U+007E
    let mut glyphs = vec![glyph(&[rect, hole])];
    for c in FIRST..=LAST {
        glyphs.push(if c == 0x20 { Vec::new() } else { glyph(&[rect]) });
    }
    let n = glyphs.len() as u16;
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for g in &glyphs {
        be32(&mut loca, glyf.len() as u32);
        glyf.extend(g);
    }
    be32(&mut loca, glyf.len() as u32);

    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0); // checkSumAdjustment
    be32(&mut head, 0x5F0F_3CF5);
    be16(&mut head, 0x000B);
    be16(&mut head, UPEM);
    head.extend([0u8; 16]); // created, modified
    for b in [50i16, 0, 550, 700] {
        be16(&mut head, b as u16);
    }
    be16(&mut head, 0); // macStyle
    be16(&mut head, 8); // lowestRecPPEM
    be16(&mut head, 2); // fontDirectionHint
    be16(&mut head, 1); // indexToLocFormat: long
    be16(&mut head, 0);

    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    be16(&mut hhea, m.hhea_ascender as u16);
    be16(&mut hhea, m.hhea_descender as u16);
    be16(&mut hhea, 0); // lineGap
    be16(&mut hhea, ADVANCE);
    be16(&mut hhea, 50); // minLeftSideBearing
    be16(&mut hhea, 50); // minRightSideBearing
    be16(&mut hhea, 550); // xMaxExtent
    be16(&mut hhea, 1); // caretSlopeRise
    be16(&mut hhea, 0);
    be16(&mut hhea, 0);
    hhea.extend([0u8; 8]);
    be16(&mut hhea, 0); // metricDataFormat
    be16(&mut hhea, n);

    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0001_0000);
    be16(&mut maxp, n);
    for x in [8u16, 2, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0] {
        be16(&mut maxp, x);
    }

    let mut hmtx = Vec::new();
    for _ in 0..n {
        be16(&mut hmtx, ADVANCE);
        be16(&mut hmtx, 50);
    }

    // cmap: format 4, U+0020..U+007E to glyphs 1..95
    let mut sub = Vec::new();
    let seg = [(FIRST, LAST, 1u16.wrapping_sub(FIRST)), (0xFFFF, 0xFFFF, 1)];
    be16(&mut sub, 4);
    be16(&mut sub, (16 + 8 * seg.len()) as u16);
    be16(&mut sub, 0);
    be16(&mut sub, 2 * seg.len() as u16);
    be16(&mut sub, 4); // searchRange
    be16(&mut sub, 1); // entrySelector
    be16(&mut sub, 0); // rangeShift
    seg.iter().for_each(|s| be16(&mut sub, s.1));
    be16(&mut sub, 0);
    seg.iter().for_each(|s| be16(&mut sub, s.0));
    seg.iter().for_each(|s| be16(&mut sub, s.2));
    seg.iter().for_each(|_| be16(&mut sub, 0));
    let mut cmap = Vec::new();
    for x in [0u16, 1, 3, 1] {
        be16(&mut cmap, x);
    }
    be32(&mut cmap, 12);
    cmap.extend(sub);

    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    be32(&mut post, 0);
    be16(&mut post, (-100i16) as u16);
    be16(&mut post, 50);
    post.extend([0u8; 20]);

    let mut os2 = Vec::new();
    be16(&mut os2, 4); // version
    be16(&mut os2, ADVANCE); // xAvgCharWidth
    be16(&mut os2, 400); // usWeightClass
    be16(&mut os2, 5); // usWidthClass
    be16(&mut os2, 0); // fsType
    for _ in 0..10 {
        be16(&mut os2, 0); // sub/superscript sizes and offsets, strikeout
    }
    be16(&mut os2, 0); // sFamilyClass
    os2.extend([0u8; 10]); // panose
    os2.extend([0u8; 16]); // ulUnicodeRange
    os2.extend(*b"TEST");
    be16(&mut os2, 0x40 | if m.use_typo_metrics { 0x80 } else { 0 }); // REGULAR, USE_TYPO_METRICS
    be16(&mut os2, FIRST);
    be16(&mut os2, LAST);
    be16(&mut os2, m.typo_ascender as u16);
    be16(&mut os2, m.typo_descender as u16);
    be16(&mut os2, 0); // sTypoLineGap
    be16(&mut os2, m.win_ascent);
    be16(&mut os2, m.win_descent);
    os2.extend([0u8; 8]); // ulCodePageRange
    be16(&mut os2, 700); // sxHeight
    be16(&mut os2, 700); // sCapHeight
    be16(&mut os2, 0); // usDefaultChar
    be16(&mut os2, 0x20); // usBreakChar
    be16(&mut os2, 1); // usMaxContext

    let tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"OS/2", os2),
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"name", name_table(family)),
        (b"post", post),
    ];
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    be16(&mut out, tables.len() as u16);
    be16(&mut out, 128); // searchRange for 10 tables: 8 × 16
    be16(&mut out, 3);
    be16(&mut out, (tables.len() as u16) * 16 - 128);
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in &tables {
        let sum = data
            .chunks(4)
            .map(|c| {
                let mut w = [0u8; 4];
                w[..c.len()].copy_from_slice(c);
                u32::from_be_bytes(w)
            })
            .fold(0u32, u32::wrapping_add);
        out.extend_from_slice(*tag);
        be32(&mut out, sum);
        be32(&mut out, offset as u32);
        be32(&mut out, data.len() as u32);
        let mut padded = data.clone();
        while padded.len() % 4 != 0 {
            padded.push(0);
        }
        offset += padded.len();
        body.extend(padded);
    }
    out.extend(body);
    out
}

/// Writes the font into `dir` as `name` and returns its path.
pub fn write(dir: &std::path::Path, name: &str, family: &str, m: Metrics) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, build(family, m)).unwrap();
    p
}
