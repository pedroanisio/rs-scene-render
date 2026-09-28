use sr_text::glyph::{self, Decor};
use sr_text::layout::{self, Align, AutoFit, Dir, Opts, Overflow, Para, Run, Wrap, Writing};
use sr_text::{FontLib, Style};
use std::sync::{Mutex, OnceLock};

fn lib() -> &'static Mutex<FontLib> {
    static L: OnceLock<Mutex<FontLib>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(FontLib::new(true)))
}

fn para(text: &str, size: f64, opts: Opts) -> Para {
    Para {
        runs: vec![Run { text: text.into(), style: 0, role: None }],
        styles: vec![Style { families: vec!["DejaVu Sans".into()], size, ..Default::default() }],
        opts,
    }
}

fn width(l: &sr_text::Layout, line: usize) -> f64 {
    l.lines[line].rect[2]
}

#[test]
fn shapes_and_wraps() {
    let mut lib = lib().lock().unwrap_or_else(|e| e.into_inner());
    let one = layout::layout(&mut lib, &para("Hello world", 32.0, Opts::default()));
    assert_eq!(one.lines.len(), 1);
    assert_eq!(one.glyphs.len(), 11);
    let w = width(&one, 0);
    assert!(w > 150.0 && w < 220.0, "{w}");
    // kerning and ligatures come from the font: advances are not all equal
    let wrapped = layout::layout(
        &mut lib,
        &para("Hello world again and again", 32.0, Opts { width: 200.0, ..Default::default() }),
    );
    assert!(wrapped.lines.len() >= 2);
    assert!(wrapped.lines.iter().all(|l| l.rect[2] <= 200.0 + 1e-6));
    // centre alignment
    let c =
        layout::layout(&mut lib, &para("Hi", 32.0, Opts { width: 300.0, align: Align::Center, ..Default::default() }));
    let l = &c.lines[0].rect;
    assert!((l[0] + l[2] * 0.5 - 150.0).abs() < 1e-6);
    // mandatory breaks
    assert_eq!(layout::layout(&mut lib, &para("a\nb\nc", 20.0, Opts::default())).lines.len(), 3);
    // no wrap keeps one line even when too wide
    assert_eq!(
        layout::layout(
            &mut lib,
            &para("Hello world again", 32.0, Opts { width: 50.0, wrap: Wrap::None, ..Default::default() })
        )
        .lines
        .len(),
        1
    );
}

#[test]
fn balance_hyphenation_ellipsis_autofit() {
    let mut lib = lib().lock().unwrap_or_else(|e| e.into_inner());
    let t = "The quick brown fox jumps over the lazy dog";
    let greedy = layout::layout(&mut lib, &para(t, 24.0, Opts { width: 360.0, ..Default::default() }));
    let bal =
        layout::layout(&mut lib, &para(t, 24.0, Opts { width: 360.0, wrap: Wrap::Balance, ..Default::default() }));
    assert_eq!(greedy.lines.len(), bal.lines.len());
    let spread = |l: &sr_text::Layout| {
        l.lines.iter().map(|x| x.rect[2]).fold(0.0f64, f64::max)
            - l.lines.iter().map(|x| x.rect[2]).fold(f64::INFINITY, f64::min)
    };
    assert!(spread(&bal) <= spread(&greedy) + 1e-6);
    // hyphenation splits a long word with a hyphen glyph
    let h = layout::layout(
        &mut lib,
        &para("internationalization", 32.0, Opts { width: 200.0, hyphenate: true, ..Default::default() }),
    );
    assert!(h.lines.len() >= 2);
    // ellipsis on the last kept line
    let e = layout::layout(
        &mut lib,
        &para(t, 24.0, Opts { width: 200.0, max_lines: Some(1), overflow: Overflow::Ellipsis, ..Default::default() }),
    );
    assert_eq!(e.lines.len(), 1);
    assert!(e.truncated && e.lines[0].rect[2] <= 200.0 + 1e-6);
    // autoFit shrink fits the box
    let f = layout::layout(
        &mut lib,
        &para(t, 64.0, Opts { width: 300.0, height: 100.0, auto_fit: AutoFit::Shrink, ..Default::default() }),
    );
    assert!(f.scale < 1.0);
    let bottom = f.lines.last().map(|l| l.rect[1] + l.rect[3]).unwrap();
    assert!(bottom <= 100.0 + 1e-6, "{bottom}");
    // grow enlarges to fill
    let g = layout::layout(
        &mut lib,
        &para(
            "Hi",
            10.0,
            Opts { width: 300.0, height: 200.0, auto_fit: AutoFit::Grow, max_size: Some(80.0), ..Default::default() },
        ),
    );
    assert!(g.scale > 7.9 * 0.99 && g.scale <= 8.0 + 1e-9, "{}", g.scale);
}

#[test]
fn bidi_vertical_and_fallback() {
    let mut lib = lib().lock().unwrap_or_else(|e| e.into_inner());
    // Hebrew is drawn right to left: the first logical character sits rightmost
    let rtl = layout::layout(&mut lib, &para("שלום", 32.0, Opts { width: 400.0, ..Default::default() }));
    let first = rtl.glyphs.iter().find(|g| g.ch == 0).unwrap().x;
    let last = rtl.glyphs.iter().find(|g| g.ch == 3).unwrap().x;
    assert!(first > last);
    // RTL paragraphs start at the right edge
    assert!((rtl.lines[0].rect[0] + rtl.lines[0].rect[2] - 400.0).abs() < 1e-6);
    let mixed =
        layout::layout(&mut lib, &para("abc שלום def", 20.0, Opts { direction: Dir::Ltr, ..Default::default() }));
    assert_eq!(mixed.glyphs.iter().filter(|g| !mixed.chars[g.ch].is_whitespace()).count(), 10);
    // vertical-rl stacks glyphs down a column at the right
    let v = layout::layout(
        &mut lib,
        &para("縦書き", 32.0, Opts { width: 100.0, height: 400.0, writing: Writing::VerticalRl, ..Default::default() }),
    );
    assert!(v.glyphs.windows(2).all(|w| w[1].y > w[0].y));
    assert!(v.glyphs[0].x > 50.0);
    // emoji fall back to a colour face
    let e = layout::layout(&mut lib, &para("ok 😀", 32.0, Opts::default()));
    let emoji = e.glyphs.iter().find(|g| e.chars[g.ch] == '😀').unwrap();
    assert!(lib.face(emoji.face).color);
    let d = glyph::draw(&lib, &e, None, &Decor::default(), 0.1);
    assert_eq!(d.bitmaps.len(), 1, "CBDT emoji become a bitmap");
}

#[test]
fn draws_with_coverage() {
    let mut lib = lib().lock().unwrap_or_else(|e| e.into_inner());
    let l = layout::layout(&mut lib, &para("H", 100.0, Opts::default()));
    let d = glyph::draw(&lib, &l, None, &Decor::default(), 0.05);
    let e = sr_vector::tile::encode(&d.scene, [100, 140]);
    let px = sr_vector::tile::render_cpu(&e, &|_, _, _| [1.0; 4]);
    let a: f32 = px.iter().map(|c| c[3]).sum();
    // DejaVu Sans "H" at 100 px covers roughly 17 % of its em square
    assert!(a > 1000.0 && a < 3000.0, "{a}");
}
