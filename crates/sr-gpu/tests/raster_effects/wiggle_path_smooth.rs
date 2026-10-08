//! `shapeModifier type="wiggle-path" mode="smooth"`: the wiggled points joined by a spline instead of a polyline.

use super::common;
use common::*;

fn doc(mode: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="320" height="100" fps="10" duration="1" background="#000000"/><composition>
  <shape id="p" shape="path" path="M 10 50 L 310 50" width="320" height="100" stroke="#FFFFFF" strokeWidth="3">
    <shapeModifier type="wiggle-path" size="8" detail="12" frequency="0" seed="4" {mode}/></shape>
</composition></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: false, base_dir: None })
        .unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

#[test]
fn smooth_mode_draws_a_different_curve_through_the_same_wiggle() {
    let Some(corner) = render(&doc("")) else { return };
    let smooth = render(&doc(r#"mode="smooth""#)).unwrap();
    assert_ne!(corner.px, smooth.px, "the mode changes the picture");
    let ink = |r: &Rendered| r.px.iter().filter(|p| p[0] > 0.5).count();
    // the same wiggle: about the same amount of ink; a spline is not shorter than the polyline it passes through by much
    let (c, s) = (ink(&corner) as f64, ink(&smooth) as f64);
    assert!(c > 500.0 && (s / c - 1.0).abs() < 0.15, "ink {c} vs {s}");
    // deterministic
    assert_eq!(smooth.px, render(&doc(r#"mode="smooth""#)).unwrap().px);
}
