//! `shape="stroke-text"`: the text laid out in a single-line (jhf) font as the shape's outline.

use sr_eval::stroke_font::{layout, parse_jhf, Font};
use sr_eval::{EvalOptions, Evaluator};

/// A record: glyph number, margins and strokes (lists of points) in font units.
fn record(number: usize, left: i32, right: i32, strokes: &[&[(i32, i32)]]) -> String {
    let c = |v: i32| char::from((82 + v) as u8);
    let mut data = format!("{}{}", c(left), c(right));
    for (k, s) in strokes.iter().enumerate() {
        if k > 0 {
            data.push_str(" R");
        }
        for (x, y) in s.iter() {
            data.push(c(*x));
            data.push(c(*y));
        }
    }
    format!("{number:5}{:3}{data}", data.len() / 2)
}

/// Glyphs for U+0020 to U+0042: space (advance 16), "A" (two strokes) and "B" (one), the rest empty.
fn font_text(wrap: bool) -> String {
    let mut out = Vec::new();
    for k in 0..35 {
        out.push(match k {
            0 => record(1, -8, 8, &[]),
            33 => record(34, -6, 6, &[&[(-4, 9), (0, -12), (4, 9)], &[(-2, 2), (2, 2)]]),
            34 => record(35, -5, 5, &[&[(-3, -12), (-3, 9), (3, 0)]]),
            _ => record(k + 1, -3, 3, &[]),
        });
    }
    if wrap {
        // records wrapped at 20 characters, as the distribution wraps them at 72
        out.join("").as_bytes().chunks(20).map(|c| String::from_utf8_lossy(c).to_string()).collect::<Vec<_>>().join("\n") + "\n"
    } else {
        out.join("\n") + "\n"
    }
}

fn numbers(path: &str) -> Vec<f64> {
    path.split_whitespace().map(|t| t.trim_start_matches(['M', 'L']).parse().unwrap()).collect()
}

#[test]
fn a_record_wrapped_over_lines_reads_the_same_as_one_line() {
    let (a, b) = (parse_jhf(&font_text(false)).unwrap(), parse_jhf(&font_text(true)).unwrap());
    assert_eq!(a, b);
    assert_eq!(a.glyphs.len(), 35);
    let g = &a.glyphs[33];
    assert_eq!((g.left, g.right), (-6, 6));
    assert_eq!(g.strokes, vec![vec![(-4, 9), (0, -12), (4, 9)], vec![(-2, 2), (2, 2)]]);
    assert_eq!(a.glyphs[0].strokes.len(), 0, "a space has no strokes");
}

#[test]
fn malformed_files_are_errors_naming_the_record() {
    assert!(parse_jhf("    1  5MW").unwrap_err().contains("record 1"));
    assert!(parse_jhf("  x1  3MWOM").unwrap_err().contains("not a number"));
}

#[test]
fn the_layout_follows_scale_advance_and_baseline() {
    let font: Font = parse_jhf(&font_text(false)).unwrap();
    // font size 21 is a scale of 1: the first baseline is at y = 21, a point (x, y) of a glyph with left margin l lands at
    // (pen + x - l, 21 + y - 9)
    let (data, missing) = layout(&font, "A", 21.0);
    assert!(missing.is_empty());
    assert_eq!(numbers(&data), vec![2.0, 21.0, 6.0, 0.0, 10.0, 21.0, 4.0, 14.0, 8.0, 14.0], "{data}");
    assert!(data.starts_with('M') && data.matches('M').count() == 2, "two strokes, two subpaths: {data}");
    // the pen moves by the glyph's advance (12): "A B" puts B after A (12) and a space (16)
    let (ab, _) = layout(&font, "A B", 21.0);
    let b_start = numbers(&ab)[10..12].to_vec();
    assert_eq!(b_start, vec![12.0 + 16.0 + 2.0, 0.0], "B's first point: pen 28, x - l = 2, y = -12 + 12");
    // scaling: size 42 doubles everything
    let (big, _) = layout(&font, "A", 42.0);
    assert_eq!(numbers(&big), numbers(&data).iter().map(|v| v * 2.0).collect::<Vec<_>>());
    // a line feed returns the pen and moves the baseline down by the em (32 units)
    let (two, _) = layout(&font, "A\nA", 21.0);
    let n = numbers(&two);
    assert_eq!(n[10..12].to_vec(), vec![2.0, 21.0 + 32.0], "{two}");
}

#[test]
fn characters_the_font_lacks_are_skipped_once_each() {
    let font = parse_jhf(&font_text(false)).unwrap();
    let (data, missing) = layout(&font, "A\u{e9}\u{e9}B\u{7f}", 21.0);
    assert_eq!(missing, vec!['\u{e9}', '\u{7f}']);
    // the skipped characters advance nothing: B follows A directly
    let (plain, _) = layout(&font, "AB", 21.0);
    assert_eq!(data, plain);
}

fn doc(tag: &str) -> (sr_model::Document, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("sr-stroke-text-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f.jhf"), font_text(true)).unwrap();
    let xml = r##"<scene version="1.2"><project width="200" height="100" fps="10" duration="2"/>
        <assets><strokeFont id="f" src="f.jhf"/></assets>
        <composition><shape id="w" shape="stroke-text" text="AB" strokeFont="f" fontSize="21" width="100" height="40" stroke="#FFFFFF" strokeWidth="2"/></composition></scene>"##;
    let d = sr_model::load_str(xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir.clone()) })
        .unwrap_or_else(|e| panic!("{e:?}\n{xml}"));
    (d, dir)
}

#[test]
fn the_shape_gets_its_outline_as_the_path_property() {
    let (d, _dir) = doc("path");
    let ev = Evaluator::new(&d, &EvalOptions::default()).unwrap();
    let g = ev.evaluate(0.0);
    assert!(g.problems.is_empty(), "{:?}", g.problems);
    let w = g.nodes.iter().find(|n| &*n.id == "w").unwrap();
    let Some(sr_eval::Value::Str(path)) = w.props.get("path") else { panic!("no path prop") };
    let font = parse_jhf(&font_text(true)).unwrap();
    assert_eq!(&**path, layout(&font, "AB", 21.0).0);
}

#[test]
fn a_font_that_cannot_be_read_is_reported_on_the_frame() {
    let (d, dir) = doc("missing");
    std::fs::remove_file(dir.join("f.jhf")).unwrap();
    let ev = Evaluator::new(&d, &EvalOptions::default()).unwrap();
    let g = ev.evaluate(0.0);
    assert!(g.problems.iter().any(|m| m.contains("f.jhf")), "{:?}", g.problems);
}
