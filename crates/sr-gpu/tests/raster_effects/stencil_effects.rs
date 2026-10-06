//! A stencil blend cuts the backdrop away outside the layer, with effects on the layer or without.

use super::common;
use common::*;

fn stencil(effects: bool) -> sr_model::Document {
    let (attr, post) =
        if effects { (r#" effects="fx""#, r#"<effects><effect id="fx" type="invert"/></effects>"#) } else { ("", "") };
    doc_with(
        r##"background="#00000000""##,
        "",
        &format!(
            r##"<shape id="w" shape="rect" x="0" y="0" width="64" height="32" fill="#FFFFFF"/>
            <layer id="s" asset="red" x="8" y="8" scaleX="2" scaleY="2" blend="stencil-alpha"{attr}/>"##
        ),
        post,
    )
}

#[test]
fn stencil_alpha_cuts_the_backdrop_outside_the_layer() {
    let Some(r) = render(&stencil(false)) else { return };
    assert_px(&r, 10, 10, [1.0; 4], 1e-3);
    assert_px(&r, 40, 20, [0.0; 4], 1e-3);
    assert_px(&r, 4, 4, [0.0; 4], 1e-3);
}

#[test]
fn stencil_alpha_cuts_the_backdrop_outside_a_layer_with_effects() {
    let Some(r) = render(&stencil(true)) else { return };
    // inside the layer the backdrop stays (the stencil uses the layer's alpha, not its colours)
    assert_px(&r, 10, 10, [1.0; 4], 1e-3);
    // outside it the backdrop is cut, far away and in the margin next to the layer's box
    assert_px(&r, 40, 20, [0.0; 4], 1e-3);
    assert_px(&r, 4, 4, [0.0; 4], 1e-3);
    assert_px(&r, 7, 10, [0.0; 4], 1e-3);
}

#[test]
fn stencil_alpha_inside_a_group_with_effects_cuts_that_groups_backdrop() {
    // the fill and the stencil layer sit in a group that has an effect: the group is drawn on its own, so the stencil
    // cuts the fill (and only the fill) outside the layer, and the group's result shows nothing else there
    let d = doc_with(
        r##"background="#00000000""##,
        "",
        r##"<shape id="floor" shape="rect" x="0" y="0" width="64" height="32" fill="#0000FF"/>
            <group id="g" effects="fx" x="0" y="0" width="64" height="32">
              <shape id="w" shape="rect" x="0" y="0" width="64" height="32" fill="#FFFFFF"/>
              <layer id="s" asset="red" x="8" y="8" scaleX="2" scaleY="2" blend="stencil-alpha"/>
            </group>"##,
        r#"<effects><effect id="fx" type="invert"/></effects>"#,
    );
    let Some(r) = render(&d) else { return };
    // inside the layer: the inverted white fill is black; outside it the group is empty, so the blue floor shows
    assert_px(&r, 10, 10, [0.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 40, 20, [0.0, 0.0, 1.0, 1.0], 1e-3);
    assert_px(&r, 7, 10, [0.0, 0.0, 1.0, 1.0], 1e-3);
}
