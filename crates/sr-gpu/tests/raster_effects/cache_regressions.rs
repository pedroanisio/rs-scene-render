//! Regressions for offscreen dependencies, shutter bounds, and document reloads.

use super::common;
use common::*;

#[test]
fn isolated_groups_follow_camera_exposure() {
    let Some(gpu) = gpu() else { return };
    let d = doc(
        "",
        "",
        r#"<camera id="cam" x="32" y="16" z="-80"><animate property="exposure"><key time="0" value="-2"/><key time="1" value="2"/></animate></camera>
      <group id="g" isolate="true"><object3D id="ball" primitive="sphere" x="32" y="16" radius="12"/></group>"#,
    );
    let frames = render_sub_frames(&d, &[0.0, 1.0]).unwrap();
    let fresh = render_times_on(gpu, &d, &[1.0]).unwrap();
    assert!(frames[1].stats.errors.is_empty() && fresh.stats.errors.is_empty());
    assert!(frames[0].px != fresh.px, "exposure must change the image");
    assert!(frames[1].px == fresh.px, "isolated group retained the old camera exposure");
}

#[test]
fn cyclic_frame_mattes_report_an_error_without_aborting() {
    const CHILD: &str = "SR_TEST_CYCLIC_FRAME_MATTES";
    if std::env::var_os(CHILD).is_none() {
        // A regression would abort the renderer process, so keep it outside the test runner.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &exact_test_name(module_path!(), "cyclic_frame_mattes_report_an_error_without_aborting"),
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        // a child that found no test to run also exits successfully
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "the child ran no test: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    }
    let Some(gpu) = gpu() else { return };
    let d = doc(
        "",
        "",
        r##"<shape id="a" shape="rect" width="16" height="16" fill="#FFFFFF"/><shape id="b" shape="rect" width="16" height="16" fill="#FFFFFF"/>"##,
    );
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let mut renderer = sr_gpu::Renderer::new(gpu, ev.program());
    let valid = ev.evaluate(0.0);
    let good = renderer.render(&valid, ev.program());
    let expected = renderer.read(&good.texture);
    renderer.render(&valid, ev.program());
    let mut g = ev.evaluate(0.0);
    g.nodes[0].matte = Some(1);
    g.nodes[1].matte = Some(0);
    let frame = renderer.render(&g, ev.program());
    assert!(frame.stats.errors.iter().any(|e| e.contains("matte") && e.contains("cycle")));
    assert!(renderer.read(&frame.texture).iter().all(|p| *p == [0.0; 4]), "bad frames must clear old pixels");
    let recovered = renderer.render(&valid, ev.program());
    assert!(recovered.stats.errors.is_empty());
    assert!(renderer.read(&recovered.texture) == expected, "the renderer must recover on a valid frame");
}

#[test]
fn isolated_groups_follow_light_constraint_targets() {
    let Some(gpu) = gpu() else { return };
    for constraint in ["parent", "copy-position"] {
        let xml = format!(
            r#"<scene version="1.2"><project width="64" height="32" fps="10" duration="2"/>
          <composition><group id="rig" x="32" y="16"><animate property="x"><key time="0" value="32"/><key time="1" value="232"/></animate></group>
          <group id="g" isolate="true"><object3D id="s" primitive="sphere" radius="12" x="32" y="16"/></group></composition>
          <lights><light id="a" type="point" z="40" intensity="2"><transformConstraint type="{constraint}" target="rig"/></light></lights></scene>"#
        );
        let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let frames = render_sub_frames(&d, &[0.0, 1.0]).unwrap();
        let fresh = render_times_on(gpu.clone(), &d, &[1.0]).unwrap();
        assert!(frames[1].stats.errors.is_empty() && fresh.stats.errors.is_empty());
        assert!(frames[0].px != fresh.px, "{constraint}: control must visibly move the light");
        assert!(frames[1].px == fresh.px, "{constraint}: isolated group retained stale lighting");
    }
}

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
