//! SREP 23: a `rect` with a nonzero `radius` or `cornerRadii` is drawn with the outline D27 gives `rounded-rect`;
//! a `rect` whose radii are all 0 is drawn exactly.
//!
//! The SREP's conformance cases: a 640 × 360 frame on black, a red (`#FF0000FF`) `rect` of 200 × 100 at (220, 130).

mod common;
use common::*;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

fn rect(attrs: &str) -> Option<Rendered> {
    let xml = format!(
        r##"<scene version="1.2"><project width="640" height="360" fps="10" duration="1" background="#000000"/>
  <composition><shape id="r" shape="rect" x="220" y="130" width="200" height="100" fill="#FF0000FF" {attrs}/></composition></scene>"##
    );
    let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
    render(&d)
}

/// Every pixel of x0..=x1 × y0..=y1 is `want`.
#[track_caller]
fn region(r: &Rendered, (x0, x1): (u32, u32), (y0, y1): (u32, u32), want: [f32; 4]) {
    for y in y0..=y1 {
        for x in x0..=x1 {
            assert_px(r, x, y, want, 0.02);
        }
    }
}

#[test]
fn srep_0023_rect_radius() {
    let Some(r) = rect(r#"radius="30""#) else { return };
    // inside the box, outside the corner arc
    region(&r, (221, 224), (131, 134), BLACK);
    // the top edge
    region(&r, (318, 322), (131, 135), RED);
    // every corner is round
    region(&r, (415, 418), (131, 134), BLACK);
    region(&r, (221, 224), (225, 228), BLACK);
    region(&r, (415, 418), (225, 228), BLACK);
}

#[test]
fn srep_0023_rect_corner_radii() {
    let Some(r) = rect(r#"cornerRadii="30 0 0 0""#) else { return };
    // the top-left corner is round
    region(&r, (221, 224), (131, 134), BLACK);
    // the top-right corner is square
    region(&r, (415, 418), (131, 134), RED);
    // and so are the bottom ones (bottom-right, bottom-left)
    region(&r, (415, 418), (225, 228), RED);
    region(&r, (221, 224), (225, 228), RED);
}

#[test]
fn srep_0023_rect_exact() {
    let Some(r) = rect("") else { return };
    region(&r, (221, 224), (131, 134), RED);
    // zero radii are the exact rect too
    let Some(z) = rect(r#"radius="0" cornerRadii="0 0 0 0""#) else { return };
    region(&z, (221, 224), (131, 134), RED);
    region(&z, (415, 418), (225, 228), RED);
}

#[test]
fn a_rect_with_radii_draws_as_the_rounded_rect() {
    let Some(a) = rect(r#"radius="30""#) else { return };
    let xml = r##"<scene version="1.2"><project width="640" height="360" fps="10" duration="1" background="#000000"/>
  <composition><shape id="r" shape="rounded-rect" x="220" y="130" width="200" height="100" fill="#FF0000FF" radius="30"/></composition></scene>"##;
    let d = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let b = render(&d).unwrap();
    assert_eq!(a.px, b.px, "rect with radius 30 and rounded-rect with radius 30 draw the same pixels");
}
