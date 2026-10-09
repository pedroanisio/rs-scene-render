//! SREP 20: lines are seated on the `hhea` ascender and descender of the primary face, or on OS/2 `usWinAscent`
//! and `usWinDescent` when `hhea`'s are both 0; never on the typographic metrics.

#[path = "support/test_font.rs"]
mod test_font;
use test_font::Metrics;

use sr_text::layout::{self, Opts, Para, Run, VAlign};
use sr_text::{FontLib, Style};

fn metrics(m: Metrics) -> (f64, f64) {
    let data = test_font::build("SREP Test", m);
    let face = rustybuzz::ttf_parser::Face::parse(&data, 0).expect("the test font parses");
    sr_text::font::line_metrics(&face)
}

#[test]
fn hhea_metrics_seat_the_line() {
    assert_eq!(metrics(Metrics::default()), (800.0, 200.0));
}

#[test]
fn the_typographic_flag_does_not_change_them() {
    assert_eq!(metrics(Metrics { use_typo_metrics: true, ..Metrics::default() }), (800.0, 200.0));
}

#[test]
fn an_empty_hhea_falls_back_to_the_windows_metrics() {
    assert_eq!(metrics(Metrics { hhea_ascender: 0, hhea_descender: 0, ..Metrics::default() }), (900.0, 200.0));
    // one of them set: hhea stands
    assert_eq!(metrics(Metrics { hhea_ascender: 0, ..Metrics::default() }), (0.0, 200.0));
}

#[test]
fn the_layout_puts_the_baseline_where_the_srep_computes_it() {
    // size 100, lineHeight 1.5, top: baseline = 0 + (150 − 100) / 2 + 80 = 105 in the box; three lines 150 apart
    let dir = std::env::temp_dir().join(format!("sr-text-srep20-{}", std::process::id()));
    let path = test_font::write(&dir, "srep20.ttf", "SREP Test", Metrics::default());
    let mut lib = FontLib::new(false);
    lib.file(&path, 0).expect("the test font loads");
    let para = Para {
        runs: vec![Run { text: "HH\nHH\nHH".into(), style: 0, role: None }],
        styles: vec![Style {
            families: vec!["SREP Test".into()],
            file: Some((path, 0)),
            size: 100.0,
            ..Default::default()
        }],
        opts: Opts { width: 400.0, height: 450.0, line_height: 1.5, valign: VAlign::Top, ..Default::default() },
    };
    let l = layout::layout(&mut lib, &para);
    let baselines: Vec<f64> = l.lines.iter().map(|b| b.baseline).collect();
    assert_eq!(baselines.len(), 3);
    for (got, want) in baselines.iter().zip([105.0, 255.0, 405.0]) {
        assert!((got - want).abs() < 1e-9, "{baselines:?}");
    }
}
