//! What the layout measures for SREP 18's `TXT-FIT` and `TXT-CUT`: how far the lines reach past the box at the size
//! drawn, and how many characters `maxLines` and `overflow` dropped.

use sr_text::layout::{self, AutoFit, Opts, Overflow, Para, Run, Wrap};
use sr_text::{FontLib, Style};
use std::sync::{Mutex, OnceLock};

fn lib() -> &'static Mutex<FontLib> {
    static L: OnceLock<Mutex<FontLib>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(FontLib::new(true)))
}

fn lay(text: &str, size: f64, opts: Opts) -> sr_text::Layout {
    let para = Para {
        runs: vec![Run { text: text.into(), style: 0, role: None }],
        styles: vec![Style { families: vec!["DejaVu Sans".into()], size, ..Default::default() }],
        opts,
    };
    let mut lib = lib().lock().unwrap_or_else(|e| e.into_inner());
    layout::layout(&mut lib, &para)
}

#[test]
fn text_that_fits_measures_nothing() {
    let l = lay("Hi", 20.0, Opts { width: 200.0, height: 100.0, ..Default::default() });
    assert_eq!((l.overflow, l.dropped), (0.0, 0));
    // an unbounded box cannot be crossed
    let u = lay("Hello world", 40.0, Opts::default());
    assert_eq!((u.overflow, u.dropped), (0.0, 0));
}

#[test]
fn a_block_taller_than_its_box_overflows_by_the_difference() {
    // three lines of 20 px at lineHeight 1.5 make a 90 px block in a 50 px box, top-aligned
    let l = lay("a\nb\nc", 20.0, Opts { width: 200.0, height: 50.0, line_height: 1.5, ..Default::default() });
    assert_eq!(l.lines.len(), 3);
    assert!((l.overflow - 40.0).abs() < 1e-9, "{}", l.overflow);
    assert_eq!(l.dropped, 0);
}

#[test]
fn a_line_wider_than_its_box_overflows_sideways() {
    let l = lay("Hello world again", 32.0, Opts { width: 50.0, height: 500.0, wrap: Wrap::None, ..Default::default() });
    let w = l.lines[0].rect[2];
    assert!((l.overflow - (w - 50.0)).abs() < 1e-9, "{} vs {w}", l.overflow);
}

#[test]
fn max_lines_drops_the_characters_of_the_lines_it_cuts() {
    // "ab\ncd\nef" with maxLines 1 keeps "ab": four characters of "cd" and "ef" are dropped, line breaks not counted
    let l = lay("ab\ncd\nef", 20.0, Opts { width: 200.0, height: 200.0, max_lines: Some(1), ..Default::default() });
    assert_eq!(l.lines.len(), 1);
    assert!(l.truncated);
    assert_eq!(l.dropped, 4);
}

#[test]
fn clip_drops_the_lines_below_the_box() {
    let l = lay(
        "a\nb\nc",
        20.0,
        Opts { width: 200.0, height: 50.0, line_height: 1.5, overflow: Overflow::Clip, ..Default::default() },
    );
    // 30 px lines: the second ends at 60, past the 50 px box, and is kept to be clipped; the third is dropped
    assert_eq!(l.lines.len(), 2);
    assert_eq!(l.dropped, 1);
    assert!((l.overflow - 10.0).abs() < 1e-9, "{}", l.overflow);
}

#[test]
fn an_ellipsis_counts_the_characters_it_replaces() {
    let t = "The quick brown fox jumps over the lazy dog";
    let l = lay(t, 24.0, Opts { width: 200.0, max_lines: Some(1), overflow: Overflow::Ellipsis, ..Default::default() });
    assert_eq!(l.lines.len(), 1);
    // every character after the last one drawn before the ellipsis (the last glyph) is dropped; a space where the
    // line was cut belongs to the kept line and is not counted
    let n = l.glyphs.len();
    let last = l.glyphs[..n - 1].iter().map(|g| g.ch).max().expect("glyphs before the ellipsis");
    let after = t.chars().count() - (last + 1);
    assert!(l.dropped == after || l.dropped + 1 == after, "dropped {}, after the last drawn {after}", l.dropped);
    assert!(l.dropped > 0);
}

#[test]
fn autofit_shrink_leaves_nothing_to_report() {
    let l = lay(
        "The quick brown fox jumps over the lazy dog",
        64.0,
        Opts { width: 300.0, height: 100.0, auto_fit: AutoFit::Shrink, ..Default::default() },
    );
    assert_eq!((l.overflow, l.dropped), (0.0, 0));
}
