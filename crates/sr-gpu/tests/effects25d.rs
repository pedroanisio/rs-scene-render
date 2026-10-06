//! Effects on a node placed in 2.5D: the chain runs on the node in its own space and the result is placed through the
//! camera (README: "the result composites with the node's opacity, blend, masks and matte"; a threeD node is placed in
//! camera space), so a blur grows and turns with the projection and a no-op chain changes nothing.

mod common;
use common::*;

/// The lab's repro at a fifth of its size: the same rectangle twice, one with a blur, seen by a 40 degree camera from
/// 296.7 units with the rectangles 130 units nearer than the z = 0 plane (a projection scale of 1.78).
fn scene(effect: &str, extra: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="216" height="384" fps="10" duration="1" background="#000000"/>
<composition><camera id="cam" fov="40" x="108" y="192" z="-296.7"/>
<shape id="plain" shape="rect" x="60" y="140" width="40" height="80" fill="#FFFFFF" threeD="true" zDepth="-130" {extra}/>
<shape id="fx" shape="rect" x="116" y="140" width="40" height="80" fill="#FFFFFF" threeD="true" zDepth="-130" effects="e" {extra}/></composition>
<effects>{effect}</effects></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"))
}

/// Bounding box of the pixels brighter than one half within columns `x0..x1`.
fn bbox(r: &Rendered, x0: u32, x1: u32) -> Option<[u32; 4]> {
    let lit: Vec<(u32, u32)> =
        (0..384u32).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| r.at(x, y)[0] > 0.5).collect();
    Some([
        lit.iter().map(|p| p.0).min()?,
        lit.iter().map(|p| p.1).min()?,
        lit.iter().map(|p| p.0).max()?,
        lit.iter().map(|p| p.1).max()?,
    ])
}

const BLUR: &str = r#"<effect id="e" type="blur" radius="2"/>"#;

#[test]
fn a_node_with_effects_is_placed_in_2_5d_like_one_without() {
    let Some(r) = render(&scene(BLUR, "")) else { return };
    assert!(r.stats.unsupported.is_empty() && r.stats.errors.is_empty(), "{:?}", r.stats.unsupported);
    let (plain, fx) = (bbox(&r, 0, 108).unwrap(), bbox(&r, 108, 216).unwrap());
    let (pw, ph) = (plain[2] - plain[0], plain[3] - plain[1]);
    let (fw, fh) = (fx[2] - fx[0], fx[3] - fx[1]);
    // the projected rectangle is about 71 x 142 px; the blurred one has to be the same size, not the layer's 40 x 80
    assert!(pw > 60 && ph > 120, "the plain rectangle is projected: {plain:?}");
    assert!(
        (fw as i32 - pw as i32).abs() <= 6 && (fh as i32 - ph as i32).abs() <= 6,
        "blurred {fx:?} against plain {plain:?}"
    );
    // it really blurs: its edge is neither 0 nor 1
    let edge = r.at(fx[0], (fx[1] + fx[3]) / 2)[0];
    assert!(edge > 0.05 && edge < 0.95, "the edge is soft: {edge}");
    // and sits where the plain one sits, mirrored about the camera axis (x = 108): both are 50 px from it
    let (pc, fc) = ((plain[0] + plain[2]) as i32 / 2, (fx[0] + fx[2]) as i32 / 2);
    assert!(((108 - pc) - (fc - 108)).abs() <= 6, "centres {pc} and {fc}");
}

#[test]
fn a_rotated_node_with_effects_keeps_its_perspective() {
    let height_at = |r: &Rendered, x: u32| (0..384u32).filter(|&y| r.at(x, y)[0] > 0.5).count();
    let Some(r) = render(&scene(BLUR, r#"rotationY="50""#)) else { return };
    let (plain, fx) = (bbox(&r, 0, 108).unwrap(), bbox(&r, 108, 216).unwrap());
    // a positive rotationY turns the right edge away: it projects shorter than the left one, with and without the blur
    let (l0, r0) = (height_at(&r, plain[0] + 3), height_at(&r, plain[2] - 3));
    let (l1, r1) = (height_at(&r, fx[0] + 3), height_at(&r, fx[2] - 3));
    assert!(r0 + 4 < l0, "the plain node turns: left {l0}, right {r0}");
    assert!(r1 + 4 < l1, "the blurred node turns too: left {l1}, right {r1}");
    // and by the same amount: the two edges are as tall as the plain node's, the blur adding a few pixels at most
    assert!(
        (l1 as i32 - l0 as i32).abs() <= 8 && (r1 as i32 - r0 as i32).abs() <= 8,
        "edges {l1}/{r1} against {l0}/{r0}"
    );
}

#[test]
fn a_moving_camera_does_not_reuse_a_stale_effected_frame() {
    // the effected texture is cached by the node's content; the camera and zDepth that place it must be in the hash
    let xml = r##"<scene version="1.2"><project width="216" height="384" fps="10" duration="1" background="#000000"/>
<composition><camera id="cam" fov="40" x="108" y="192" z="-296.7"><animate property="x"><key time="0" value="108"/><key time="1" value="60"/></animate></camera>
<shape id="fx" shape="rect" x="116" y="140" width="40" height="80" fill="#FFFFFF" threeD="true" zDepth="-130" effects="e"/></composition>
<effects><effect id="e" type="blur" radius="2"/></effects></scene>"##;
    let d = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
    let Some(warm) = render_times(&d, &[0.0, 0.5]) else { return };
    let cold = render_times(&d, &[0.5]).unwrap();
    let worst = warm
        .px
        .iter()
        .zip(&cold.px)
        .map(|(a, b)| (0..4).map(|c| (a[c] - b[c]).abs()).fold(0.0f32, f32::max))
        .fold(0.0f32, f32::max);
    assert!(worst < 2e-2, "the frame after a camera move differs from a cold render by {worst}");
    // and the camera did move the node
    let first = render_times(&d, &[0.0]).unwrap();
    assert_ne!(first.px, cold.px);
}
