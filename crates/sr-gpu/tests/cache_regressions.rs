//! Regressions for offscreen dependencies, shutter bounds, and document reloads.

mod common;
use common::*;

#[test]
fn matte_cache_follows_animated_paints() {
    let d = doc(
        "",
        r##"<paints><linearGradient id="p" x1="0" y1="0" x2="1" y2="0"><stop offset="0" color="#000000"><animate property="color"><key time="0" value="#000000"/><key time="1" value="#FFFFFF"/></animate></stop><stop offset="1" color="#FFFFFF"/></linearGradient></paints>"##,
        r##"<shape id="m" shape="rect" width="50" height="30" fill="url(#p)"/><layer id="s" asset="white" scaleX="12" scaleY="7" matte="m" matteMode="luma"/>"##,
    );
    let Some(fresh) = render_times(&d, &[1.0]) else { return };
    let after = render_times(&d, &[0.0, 1.0]).unwrap();
    assert!(fresh.stats.errors.is_empty() && after.stats.errors.is_empty());
    assert_eq!(after.at(10, 10), fresh.at(10, 10));
}

#[test]
fn isolated_groups_ignore_unrelated_light_animation() {
    let d = doc_with(
        "",
        "",
        r##"<group id="g" isolate="true"><shape id="s" shape="rect" width="50" height="30" fill="#FFFFFF"/></group>"##,
        r#"<lights><light id="a" type="ambient" intensity="0.1"><animate property="intensity"><key time="0" value="0.1"/><key time="1" value="1"/></animate></light></lights>"#,
    );
    let Some(frames) = render_sub_frames(&d, &[0.0, 1.0]) else { return };
    assert!(frames[1].stats.errors.is_empty());
    assert_eq!(frames[0].px, frames[1].px);
    assert!(frames[1].stats.cache_hits > 0, "unrelated lights must not invalidate the 2D offscreen");
}

#[test]
fn isolated_groups_follow_animated_materials() {
    let Some(gpu) = gpu() else { return };
    let d = doc(
        "",
        r##"<materials><material id="m" unlit="true" baseColor="#FF0000"><animate property="baseColor"><key time="0" value="#FF0000"/><key time="1" value="#0000FF"/></animate></material></materials>"##,
        r#"<group id="g" isolate="true"><object3D id="s" primitive="sphere" radius="12" x="32" y="16" material="m"/></group>"#,
    );
    let fresh = render_times_on(gpu.clone(), &d, &[1.0]).unwrap();
    let after = render_times_on(gpu, &d, &[0.0, 1.0]).unwrap();
    assert!(fresh.stats.errors.is_empty() && after.stats.errors.is_empty());
    assert_eq!(after.at(32, 16), fresh.at(32, 16));
}

#[test]
fn isolated_groups_follow_animated_paints() {
    if gpu().is_none() {
        return;
    }
    let d = doc(
        "",
        r##"<paints><linearGradient id="p" x1="0" y1="0" x2="1" y2="0"><stop offset="0" color="#FF0000"><animate property="color"><key time="0" value="#FF0000"/><key time="1" value="#0000FF"/></animate></stop><stop offset="1" color="#FFFFFF"/></linearGradient></paints>"##,
        r##"<group id="g" isolate="true"><shape id="s" shape="rect" width="50" height="30" fill="url(#p)"/></group>"##,
    );
    let fresh = render_times(&d, &[1.0]).expect("GPU");
    let after = render_times(&d, &[0.0, 1.0]).expect("GPU");
    assert!(fresh.stats.errors.is_empty() && after.stats.errors.is_empty());
    assert_eq!(after.at(10, 10), fresh.at(10, 10));
}

#[test]
fn isolated_groups_follow_animated_lights() {
    if gpu().is_none() {
        return;
    }
    let d = doc_with(
        "",
        "",
        r#"<group id="g" isolate="true"><object3D id="s" primitive="sphere" radius="12" x="32" y="16"/></group>"#,
        r#"<lights><light id="a" type="ambient" intensity="0.1"><animate property="intensity"><key time="0" value="0.1"/><key time="1" value="1"/></animate></light></lights>"#,
    );
    let fresh = render_times(&d, &[1.0]).expect("GPU");
    let after = render_times(&d, &[0.0, 1.0]).expect("GPU");
    assert!(fresh.stats.errors.is_empty() && after.stats.errors.is_empty());
    assert_eq!(after.at(32, 16), fresh.at(32, 16));
}

#[test]
fn motion_blur_bounds_follow_shape_growth() {
    if gpu().is_none() {
        return;
    }
    let body = r##"<shape id="s" shape="rect" x="10" y="5" width="5" height="20" fill="#FFFFFF"><animate property="x"><key time="0" value="10"/><key time="1" value="20"/></animate><animate property="width"><key time="0.5" value="5"/><key time="0.6" value="45"/></animate></shape>"##;
    let project = r#"shutterAngle="360" shutterPhase="0" motionBlurSamples="12" adaptiveMotionBlur="false""#;
    let blurred = doc(&format!("{project} motionBlur=\"true\""), "", body);
    let sharp = doc(&format!("{project} motionBlur=\"false\""), "", body);
    let times: Vec<_> = (0..12).map(|k| 0.5 + 0.1 * (k as f64 + 0.5) / 12.0).collect();
    let frames = render_sub_frames(&sharp, &times).expect("GPU");
    let mut expected = vec![[0.0f32; 4]; frames[0].px.len()];
    for frame in frames {
        for (a, b) in expected.iter_mut().zip(frame.px) {
            for c in 0..4 {
                a[c] += b[c] / 12.0;
            }
        }
    }
    // A single-copy repeater preserves the image and selects the full-frame fallback.
    let unbounded = doc(
        &format!("{project} motionBlur=\"true\""),
        "",
        &body.replace("</shape>", "<shapeModifier type=\"repeater\" copies=\"1\"/></shape>"),
    );
    let control = render_sub_frames(&unbounded, &[0.5]).expect("GPU").remove(0);
    assert!(psnr(&control.px, &expected) > 40.0, "unbounded control differs");
    let actual = render_sub_frames(&blurred, &[0.5]).expect("GPU").remove(0);
    let score = psnr(&actual.px, &expected);
    assert!(
        score > 40.0,
        "blur differs from sampled frames: {score} dB, pixel50 {:?} expected {:?}",
        actual.at(50, 10),
        expected[10 * 64 + 50]
    );
}

#[test]
fn caption_cache_survives_program_address_reuse() {
    if gpu().is_none() {
        return;
    }
    let source = r##"<captions><captionTrack id="cc" language="en" mode="burn" x="32" y="2" width="56"><cue start="0" end="2" text="ABC"/></captionTrack></captions>"##;
    let first = doc_with("", "", "", source);
    let second = doc_with("", "", "", &source.replace("ABC", "XYZ"));
    // Keep the storage address fixed while replacing the document, as watch does.
    let mut ev = Box::new(sr_eval::Evaluator::new(&first, &Default::default()).unwrap());
    let mut renderer = sr_gpu::Renderer::new(gpu().expect("GPU"), ev.program());
    let frame = renderer.render(&ev.evaluate(0.0), ev.program());
    let old = renderer.read(&frame.texture);
    *ev = sr_eval::Evaluator::new(&second, &Default::default()).unwrap();
    let frame = renderer.render(&ev.evaluate(0.0), ev.program());
    let actual = renderer.read(&frame.texture);
    let expected = render_times(&second, &[0.0]).expect("GPU");
    assert!(old != expected.px, "text differs");
    assert!(actual == expected.px, "reloaded captions remain stale: unchanged = {}", actual == old);
}
