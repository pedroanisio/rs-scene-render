//! Motion blur is the average of the frame drawn sharp at every sample time, whatever the
//! moving node is: one that blends with what is below it, an adjustment layer, or a turning
//! layer whose effects have a direction on screen.

mod common;

use common::*;

const PROJECT: &str =
    r##"width="128" height="64" fps="10" duration="4" shutterAngle="180" shutterPhase="0" motionBlurSamples="12""##;

fn scene(blur: bool, body: &str, post: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.1"><project {PROJECT} motionBlur="{blur}"/>{ASSETS}<composition>{body}</composition>{post}</scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// The average of the sharp frames at the shutter's 12 sample times from `t`.
fn average(sharp: &sr_model::Document, t: f64) -> Option<Vec<[f32; 4]>> {
    let mut acc: Option<Vec<[f32; 4]>> = None;
    for k in 0..12 {
        let r = render_times(sharp, &[t + 0.5 * (k as f64 + 0.5) / 12.0 / 10.0])?;
        let acc = acc.get_or_insert_with(|| vec![[0.0; 4]; r.px.len()]);
        for (a, p) in acc.iter_mut().zip(&r.px) {
            for c in 0..4 {
                a[c] += p[c] / 12.0;
            }
        }
    }
    acc
}

/// PSNR of the blurred render of `body` at `t` against the average of its sharp frames.
fn against_average(body: &str, post: &str, t: f64) -> Option<(f64, Rendered)> {
    let r = render_sub(&scene(true, body, post), t)?;
    let want = average(&scene(false, body, post), t)?;
    Some((psnr(&r.px, &want), r))
}

const MOVE: &str = r#"<animate property="x"><key time="0" value="0"/><key time="1" value="96"/></animate>"#;

#[test]
fn moving_layers_keep_their_blend_mode() {
    // a grey square crossing a red and blue backdrop: multiplied, screened or added, it is
    // nothing like the square drawn over it
    let backdrop = r#"<layer id="bg" asset="wide" x="0" y="0" scaleX="16" scaleY="16"/>"#;
    for blend in ["multiply", "screen", "add", "normal"] {
        for effects in ["", r#" effects="soft""#] {
            let body = format!(
                r#"{backdrop}<layer id="m" asset="gray" x="8" y="16" scaleX="6" scaleY="6" blend="{blend}"{effects}>{MOVE}</layer>"#
            );
            let fx = r#"<effects><effect id="soft" type="blur" radius="1"/></effects>"#;
            let Some((db, r)) = against_average(&body, fx, 0.5) else { return };
            assert!(r.stats.unsupported.is_empty(), "{:?}", r.stats.unsupported);
            assert!(db >= 40.0, "{blend}{effects}: motion blur vs the average of sharp frames: {db:.1} dB");
        }
    }
}

#[test]
fn moving_groups_keep_their_blend_mode() {
    let body = format!(
        r#"<layer id="bg" asset="wide" x="0" y="0" scaleX="16" scaleY="16"/>
        <group id="grp" x="8" y="16" blend="multiply">{MOVE}<layer id="a" asset="gray" scaleX="6" scaleY="6"/><layer id="b" asset="white" x="12" y="12" scaleX="4" scaleY="4" blend="difference"/></group>"#
    );
    let Some((db, _)) = against_average(&body, "", 0.5) else { return };
    assert!(db >= 40.0, "motion blur vs the average of sharp frames: {db:.1} dB");
}

#[test]
fn moving_adjustment_layers_still_adjust() {
    // an inverting adjustment over the left part of a red frame, sliding right
    let body = format!(
        r#"<layer id="bg" asset="red" x="0" y="0" scaleX="32" scaleY="16"/>
        <adjustment id="adj" effects="inv"><mask type="rect" x="0" y="0" width="40" height="64"/>{MOVE}</adjustment>"#
    );
    let fx = r#"<effects><effect id="inv" type="invert"/></effects>"#;
    let Some(r) = render_sub(&scene(true, &body, fx), 0.5) else { return };
    // at 0.5 s the mask spans x 48..88
    assert_px(&r, 20, 32, [1.0, 0.0, 0.0, 1.0], 2e-2);
    assert_px(&r, 68, 32, [0.0, 1.0, 1.0, 1.0], 2e-2);
    assert_px(&r, 110, 32, [1.0, 0.0, 0.0, 1.0], 2e-2);
    // it adjusts where it is at the frame time, and says that it was not blurred
    let sharp = render_sub(&scene(false, &body, fx), 0.5).unwrap();
    assert!(psnr(&r.px, &sharp.px) >= 50.0, "{:.1} dB from the frame without motion blur", psnr(&r.px, &sharp.px));
    assert!(
        r.stats.unsupported.iter().any(|m| m.contains("adj") && m.contains("motion blur")),
        "{:?}",
        r.stats.unsupported
    );
    assert!(sharp.stats.unsupported.is_empty(), "{:?}", sharp.stats.unsupported);
}

#[test]
fn turning_layers_keep_the_direction_of_their_effects() {
    // a layer that spins 18° across the shutter: what its effects do along a direction must be
    // what each sample shows, not one drawing turned
    let mut failures = Vec::new();
    let turn = r#"<animate property="rotation"><key time="0" value="0"/><key time="1" value="360"/></animate>"#;
    for effect in [
        r##"<effect id="fx" type="drop-shadow" offsetX="10" offsetY="0" radius="1" color="#00FF00"/>"##,
        r##"<effect id="fx" type="long-shadow" size="14" angle="0" color="#00FF00"/>"##,
        r#"<effect id="fx" type="directional-blur" radius="6" angle="0"/>"#,
        r#"<effect id="fx" type="bevel" size="3" angle="20" intensity="1"/>"#,
        r#"<effect id="fx" type="emboss" angle="20" relief="2"/>"#,
        r#"<effect id="fx" type="scanlines"/>"#,
    ] {
        let body = format!(
            r#"<layer id="m" asset="wide" x="64" y="32" anchorX="4" anchorY="2" scaleX="5" scaleY="5" effects="fx">{turn}</layer>"#
        );
        let Some((db, r)) = against_average(&body, &format!("<effects>{effect}</effects>"), 0.3) else { return };
        assert!(r.stats.unsupported.is_empty(), "{:?}", r.stats.unsupported);
        eprintln!("{effect}: {db:.1} dB, {} passes", r.stats.fx_passes);
        // one drawing turned comes within 37 to 47 dB where the effect does not turn with it
        if db < 50.0 {
            failures.push(format!("{effect}: {db:.1} dB against the average of sharp frames"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
