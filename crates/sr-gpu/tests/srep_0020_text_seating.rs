//! SREP 20: where lines of text sit in a text box. CSS 2.1 half-leading over the `hhea` ascender and descender of
//! the primary face (OS/2 `usWinAscent`/`usWinDescent` when `hhea`'s are both 0), lines at a pitch of
//! `lineHeight` × `size`, the block placed by `verticalAlign` and each line by `align`.
//!
//! The SREP's conformance cases, with its pinned test font (unitsPerEm 1000, ascender 800, descender −200, every
//! glyph a rectangle 50–550 × 0–700, advance 600): text `HH`, size 100 px, `lineHeight="1.5"`, in a box at (100, 30).

mod common;
use common::*;

#[path = "../../sr-text/tests/support/test_font.rs"]
mod test_font;
use test_font::Metrics;

fn doc(font: &str, text: &str, box_h: u32, attrs: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="640" height="520" fps="10" duration="1" background="#000000"/>
  <assets>
    <font id="tf" family="SREP Test" src="{font}"/>
    <text id="t" text="{text}" width="400" height="{box_h}" size="100" lineHeight="1.5" color="#FFFFFF" fontAsset="tf" {attrs}/>
  </assets>
  <composition><layer id="l" asset="t" x="100" y="30"/></composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// Writes the test font as `name` among the fixtures. Tests run at once: each writes its own file, so none reads a
/// file another is rewriting.
fn font(name: &str, m: Metrics) -> String {
    test_font::write(&fixtures(), name, "SREP Test", m);
    name.to_string()
}

fn inked(r: &Rendered, x: u32, y: u32) -> bool {
    r.at(x, y)[0] > 0.5
}

/// First and last pixel row with ink.
fn ink_rows(r: &Rendered) -> (u32, u32) {
    let rows: Vec<u32> = (0..r.size[1]).filter(|&y| (0..r.size[0]).any(|x| inked(r, x, y))).collect();
    (*rows.first().expect("ink"), *rows.last().expect("ink"))
}

/// Maximal runs of pixel rows with ink, as (first, last).
fn row_runs(r: &Rendered) -> Vec<(u32, u32)> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for y in 0..r.size[1] {
        if (0..r.size[0]).any(|x| inked(r, x, y)) {
            match runs.last_mut() {
                Some(run) if run.1 + 1 == y => run.1 = y,
                _ => runs.push((y, y)),
            }
        }
    }
    runs
}

/// Maximal runs of pixel columns with ink.
fn column_runs(r: &Rendered) -> Vec<(u32, u32)> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for x in 0..r.size[0] {
        if (0..r.size[1]).any(|y| inked(r, x, y)) {
            match runs.last_mut() {
                Some(run) if run.1 + 1 == x => run.1 = x,
                _ => runs.push((x, x)),
            }
        }
    }
    runs
}

#[test]
fn srep_0020_text_top_middle_bottom() {
    let f = font("srep20-valign.ttf", Metrics::default());
    // ink rows 65–135, 140–210 and 215–285: pixel rows 65..=134, 140..=209, 215..=284
    for (valign, rows) in [("top", (65, 134)), ("middle", (140, 209)), ("bottom", (215, 284))] {
        let Some(r) = render(&doc(&f, "HH", 300, &format!(r#"verticalAlign="{valign}""#))) else { return };
        assert_eq!(ink_rows(&r), rows, "verticalAlign={valign}");
    }
}

#[test]
fn srep_0020_text_lines() {
    let f = font("srep20-lines.ttf", Metrics::default());
    // baselines at y = 135, 285 and 435; each line inks 70 px above its baseline
    let Some(r) = render(&doc(&f, "HH&#10;HH&#10;HH", 450, r#"verticalAlign="top""#)) else { return };
    assert_eq!(row_runs(&r), [(65, 134), (215, 284), (365, 434)]);
}

#[test]
fn srep_0020_text_align() {
    let f = font("srep20-align.ttf", Metrics::default());
    // ink columns 105–215, 245–355 and 385–495: two glyphs of 60 px advance, each inking 5–55 px of its advance
    for (align, x0) in [("start", 105), ("center", 245), ("end", 385)] {
        let Some(r) = render(&doc(&f, "HH", 300, &format!(r#"align="{align}""#))) else { return };
        assert_eq!(column_runs(&r), [(x0, x0 + 49), (x0 + 60, x0 + 109)], "align={align}");
    }
}

#[test]
fn hhea_metrics_win_over_typographic_ones_even_when_the_font_asks_for_them() {
    // USE_TYPO_METRICS would seat the line on 700/−300 (baseline 125); SREP 20 takes hhea (baseline 135)
    let f = font("srep20-typo.ttf", Metrics { use_typo_metrics: true, ..Metrics::default() });
    let Some(r) = render(&doc(&f, "HH", 300, r#"verticalAlign="top""#)) else { return };
    assert_eq!(ink_rows(&r), (65, 134));
}

#[test]
fn windows_metrics_replace_an_empty_hhea() {
    // hhea 0/0: usWinAscent 900, usWinDescent 200 give A = 0.9, D = 0.2, a 110 px content area and 20 px of
    // half-leading over the 150 px pitch: baseline 30 + 20 + 90 = 140 (not the typographic metrics' 125)
    let m = Metrics { hhea_ascender: 0, hhea_descender: 0, ..Metrics::default() };
    let f = font("srep20-win.ttf", m);
    let Some(r) = render(&doc(&f, "HH", 300, r#"verticalAlign="top""#)) else { return };
    assert_eq!(ink_rows(&r), (70, 139));
}
