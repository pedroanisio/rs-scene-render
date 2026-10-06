//! Stroke markers (SREP 15): `markerStart` and `markerEnd` at the ends of the drawn part of an open outline.

mod common;
use common::*;

fn frame(shape: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.1"><project width="640" height="360" fps="10" duration="2" background="#000000"/><composition>{shape}</composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: false, base_dir: None };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn path(attrs: &str, d: &str, stroke: &str) -> sr_model::Document {
    frame(&format!(
        r#"<shape id="p" shape="path" path="{d}" x="0" y="0" width="640" height="360" stroke="{stroke}" strokeWidth="6" {attrs}/>"#
    ))
}

/// The bounding box (x0, y0, x1, y1) of the pixels with green above ½.
fn green_box(r: &Rendered) -> [u32; 4] {
    let mut b = [u32::MAX, u32::MAX, 0, 0];
    for y in 0..r.size[1] {
        for x in 0..r.size[0] {
            if r.at(x, y)[1] > 0.5 {
                b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
            }
        }
    }
    b
}

#[track_caller]
fn near(got: [u32; 4], want: [i32; 4]) {
    for k in 0..4 {
        assert!((got[k] as i32 - want[k]).abs() <= 2, "box {got:?}, want {want:?} (±2)");
    }
}

#[test]
fn an_arrow_at_the_end_adds_a_head_of_four_stroke_widths() {
    let Some(r) = render_times(&path(r#"markerEnd="arrow""#, "M180 180 L460 180", "#00FF00"), &[0.0]) else { return };
    // the head is 24 px long and 24 px wide, its tip on the end of the path; the stroke stops where the head starts
    near(green_box(&r), [180, 168, 460, 192]);
    assert!(r.at(455, 180)[1] > 0.99, "inside the head");
    assert!(r.at(455, 170)[1] < 0.5, "the head narrows to its tip");
}

#[test]
fn without_markers_the_stroke_is_as_before() {
    let Some(a) = render_times(&path("", "M180 180 L460 180", "#00FF00"), &[0.0]) else { return };
    let Some(b) = render_times(&path(r#"markerStart="none" markerEnd="none""#, "M180 180 L460 180", "#00FF00"), &[0.0])
    else {
        return;
    };
    assert_eq!(a.px, b.px);
    near(green_box(&a), [180, 177, 460, 183]);
}

#[test]
fn the_marker_rides_the_trimmed_tip() {
    let Some(r) = render_times(&path(r#"markerEnd="arrow" trimEnd="0.5""#, "M180 180 L460 180", "#00FF00"), &[0.0])
    else {
        return;
    };
    near(green_box(&r), [180, 168, 320, 192]);
    assert!(r.at(330, 180)[1] < 0.1, "nothing right of the tip");
}

#[test]
fn the_start_marker_points_backwards() {
    let Some(r) =
        render_times(&path(r#"markerStart="arrow" markerEnd="arrow""#, "M100 180 L500 180", "#00FF00"), &[0.0])
    else {
        return;
    };
    near(green_box(&r), [100, 168, 500, 192]);
    assert!(r.at(98, 180)[1] < 0.1, "no green left of the start");
}

#[test]
fn stroke_and_marker_are_one_coverage() {
    // a translucent stroke is not darker where the head meets it
    let Some(r) = render_times(&path(r#"markerEnd="arrow""#, "M180 180 L460 180", "#00FF0080"), &[0.0]) else { return };
    let (stroke, head, join) = (r.at(300, 180)[1], r.at(445, 180)[1], r.at(437, 180)[1]);
    assert!(stroke > 0.1 && stroke < 0.9, "translucent: {stroke}");
    assert!((head - stroke).abs() < 0.01 && (join - stroke).abs() < 0.01, "{stroke} {head} {join}");
}

#[test]
fn the_marker_set_has_these_shapes() {
    let at = |kind: &str| {
        render_times(&path(&format!(r#"markerEnd="{kind}""#, kind = kind), "M180 180 L460 180", "#00FF00"), &[0.0])
    };
    // circle, square and diamond of side 24 sit behind the end; the square fills its corner, the circle and diamond do not
    let (Some(circle), Some(square), Some(diamond)) = (at("circle"), at("square"), at("diamond")) else { return };
    near(green_box(&circle), [180, 168, 460, 192]);
    near(green_box(&square), [180, 168, 460, 192]);
    near(green_box(&diamond), [180, 168, 460, 192]);
    assert!(square.at(458, 170)[1] > 0.9, "square corner");
    assert!(circle.at(458, 170)[1] < 0.5, "circle corner");
    assert!(diamond.at(458, 170)[1] < 0.5, "diamond corner");
    // the bar is a stroke across the end, as wide as the stroke; the open arrow is a stroked chevron
    let (Some(bar), Some(open)) = (at("bar"), at("open-arrow")) else { return };
    near(green_box(&bar), [180, 168, 463, 192]);
    near(green_box(&open), [180, 165, 467, 195]);
}

#[test]
fn a_zero_width_stroke_draws_no_markers() {
    let doc = frame(
        r##"<shape id="p" shape="path" path="M180 180 L460 180" x="0" y="0" width="640" height="360" stroke="#00FF00" strokeWidth="0" markerEnd="arrow"/>"##,
    );
    let Some(r) = render_times(&doc, &[0.0]) else { return };
    assert_eq!(green_box(&r), [u32::MAX, u32::MAX, 0, 0]);
}

#[test]
fn a_closed_outline_has_no_ends_to_mark() {
    let closed = "M100 100 L200 100 L200 200 Z";
    let Some(a) = render_times(&path("", closed, "#00FF00"), &[0.0]) else { return };
    let Some(b) = render_times(&path(r#"markerStart="arrow" markerEnd="arrow""#, closed, "#00FF00"), &[0.0]) else {
        return;
    };
    assert_eq!(a.px, b.px);
    // trimmed, the outline is open and its tip carries the marker
    let Some(t) = render_times(&path(r#"markerEnd="arrow" trimEnd="0.5""#, closed, "#00FF00"), &[0.0]) else { return };
    let Some(u) = render_times(&path(r#"trimEnd="0.5""#, closed, "#00FF00"), &[0.0]) else { return };
    assert_ne!(t.px, u.px, "a trimmed closed outline is open");
}

/// SREP 15: "`markerStart` applies at the first point of the drawn interval in path order, and `markerEnd` at the last,
/// each only when the subpath holding that point is open."
#[test]
fn in_a_path_with_open_and_closed_subpaths_a_marker_needs_the_open_one() {
    let closed = "M100 100 L200 100 L200 200 Z";
    let open = "M300 100 L400 100 L400 200";
    let pixels = |attrs: &str, d: &str| render_times(&path(attrs, d, "#00FF00"), &[0.0]).map(|r| r.px);
    for (d, what, start_marks, end_marks) in [
        (format!("{closed} {open}"), "closed first, open last", false, true),
        (format!("{open} {closed}"), "open first, closed last", true, false),
        (format!("{closed} M300 100 L400 100 Z"), "every subpath closed", false, false),
    ] {
        let Some(plain) = pixels("", &d) else { return };
        let start = pixels(r#"markerStart="arrow""#, &d).unwrap();
        let end = pixels(r#"markerEnd="arrow""#, &d).unwrap();
        assert_eq!(start != plain, start_marks, "{what}: markerStart");
        assert_eq!(end != plain, end_marks, "{what}: markerEnd");
    }
}
