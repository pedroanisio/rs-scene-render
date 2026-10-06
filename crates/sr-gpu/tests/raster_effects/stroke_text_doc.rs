//! `shape="stroke-text"` draws a string in a single-line font as strokes, written on by `trimEnd` in sequential mode.

use super::common;
use common::*;

fn font() -> String {
    let c = |v: i32| char::from((82 + v) as u8);
    let rec = |n: usize, l: i32, r: i32, strokes: &[&[(i32, i32)]]| {
        let mut d = format!("{}{}", c(l), c(r));
        for (k, s) in strokes.iter().enumerate() {
            if k > 0 {
                d.push_str(" R");
            }
            for (x, y) in s.iter() {
                d.push(c(*x));
                d.push(c(*y));
            }
        }
        format!("{n:5}{:3}{d}", d.len() / 2)
    };
    let mut out = Vec::new();
    for k in 0..50usize {
        let ch = char::from(32 + k as u8);
        out.push(match ch {
            ' ' => rec(k + 1, -8, 8, &[]),
            'H' => rec(k + 1, -5, 5, &[&[(-3, -12), (-3, 9)], &[(3, -12), (3, 9)], &[(-3, -1), (3, -1)]]),
            'I' => rec(k + 1, -2, 2, &[&[(0, -12), (0, 9)]]),
            _ => rec(k + 1, -3, 3, &[]),
        });
    }
    out.join("\n") + "\n"
}

fn doc(attrs: &str) -> sr_model::Document {
    std::fs::write(fixtures().join("stroke-text.jhf"), font()).unwrap();
    let xml = format!(
        r##"<scene version="1.2"><project width="400" height="100" fps="10" duration="2" background="#000000"/>
        <assets><strokeFont id="hand" src="stroke-text.jhf"/></assets>
        <composition><shape id="w" shape="stroke-text" text="HIHI" strokeFont="hand" fontSize="42" width="400" height="100" x="0" y="20" stroke="#FFFFFF" strokeWidth="3" {attrs}/></composition></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) })
        .unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// White ink left and right of x = 28, where the first "HI" ends and the second begins (advances 20 + 8 at scale 2).
fn ink(r: &Rendered) -> (usize, usize) {
    let (mut l, mut rt) = (0, 0);
    for y in 0..r.size[1] {
        for x in 0..r.size[0] {
            if r.at(x, y)[0] > 0.5 {
                if x < 28 {
                    l += 1;
                } else {
                    rt += 1;
                }
            }
        }
    }
    (l, rt)
}

#[test]
fn the_text_is_drawn_as_strokes_only() {
    let Some(r) = render(&doc("")) else { return };
    let (l, rt) = ink(&r);
    assert!(l > 200 && rt > 200, "both halves have ink: {l} {rt}");
    // the fill is not used: an open stroke has no inside (a filled H would cover the gap between its bars)
    assert!(r.at(10, 55)[0] < 0.2, "between the H's legs, below the bar: {:?}", r.at(10, 55));
    assert!(r.at(10, 42)[0] > 0.5, "the bar: {:?}", r.at(10, 42));
}

#[test]
fn sequential_trim_writes_the_text_on_in_order() {
    let Some(half) = render(&doc(r#"trimMode="sequential" trimEnd="0.5""#)) else { return };
    let (l, rt) = ink(&half);
    // the first half of the stroke length is the first "HI": nothing of the second yet
    assert!(l > 200 && rt == 0, "left {l}, right {rt}");
    // every stroke trimmed on its own (the default) puts ink in both halves
    let each = render(&doc(r#"trimEnd="0.5""#)).unwrap();
    let (l2, r2) = ink(&each);
    assert!(l2 > 50 && r2 > 50, "{l2} {r2}");
    // the whole text at trimEnd 1
    let full = render(&doc(r#"trimMode="sequential" trimEnd="1""#)).unwrap();
    assert!(ink(&full).1 > 200);
}
