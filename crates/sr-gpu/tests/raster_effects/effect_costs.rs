//! What effects cost: targets sized to the content they cover, posterized frames reused until
//! the next step, caches kept when unrelated paints animate, wide strokes computed exactly, and
//! rigid layers rendered once under motion blur. Each test also checks the pixels, since a
//! cheaper result is only acceptable when it is the same result.

use super::common;

use common::*;

/// A document from project attributes, paints, a composition body and effect definitions.
fn scene(project: &str, paints: &str, body: &str, effects: &str) -> sr_model::Document {
    let effects = if effects.is_empty() { String::new() } else { format!("<effects>{effects}</effects>") };
    let xml = format!(
        r#"<scene version="1.1"><project {project}/>{ASSETS}{paints}<composition>{body}</composition>{effects}</scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

const WIDE: &str = r##"width="256" height="144" fps="60" duration="2" background="#00000000""##;
const FRAME: u64 = 256 * 144;

#[test]
fn unchanged_shape_shutter_samples_reuse_the_accumulation() {
    let Some(gpu) = gpu() else { return };
    let d = scene(
        r#"width="96" height="64" fps="30" duration="2" motionBlur="true" motionBlurSamples="4"
            shutterPhase="0" shutterAngle="180" adaptiveMotionBlur="false""#,
        "",
        r##"<group id="g" opacity="0.7"><shape id="moving" shape="ellipse" x="8" y="8"
            width="32" height="24" fill="#EE774488">
            <animate property="x"><key time="0" value="8"/><key time="2" value="60"/></animate>
        </shape></group>
        <shape id="other" shape="rect" x="80" y="40" width="10" height="10" fill="#FFFFFF" motionBlur="off"/>"##,
        "",
    );
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let g = ev.evaluate(0.5);
    let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
    let first = renderer.render_with(&g, ev.program(), Some(&mut |t| ev.evaluate(t)));
    let pixels = renderer.read(&first.texture);
    assert!(first.stats.fx_passes > 0);
    let warm = renderer.render_with(&g, ev.program(), Some(&mut |t| ev.evaluate(t)));
    assert_eq!(renderer.read(&warm.texture), pixels);
    assert_eq!(warm.stats.fx_passes, 0, "identical shutter samples need no new accumulation passes");

    // Changing a different shape invalidates the frame prefix but not this accumulation.
    let mut changed = g.clone();
    changed.nodes.iter_mut().find(|n| &*n.id == "other").unwrap().draw = false;
    let warm = renderer.render_with(&changed, ev.program(), Some(&mut |t| ev.evaluate(t)));
    assert_eq!(warm.stats.fx_passes, 0);
    let mut fresh = sr_gpu::Renderer::new(gpu.clone(), ev.program());
    let expected = fresh.render_with(&changed, ev.program(), Some(&mut |t| ev.evaluate(t)));
    assert_eq!(renderer.read(&warm.texture), fresh.read(&expected.texture));

    // The main frame and the first/last shutter samples are unchanged. An interior
    // sample alone must invalidate the cache, including its inherited opacity.
    for offset in [8.0, 0.0] {
        let mut sample = |t: f64| {
            let mut graph = ev.evaluate(t);
            if t > 0.506 && t < 0.511 {
                let n = graph.nodes.iter_mut().find(|n| &*n.id == "moving").unwrap();
                n.world.0[4] += offset;
                n.world_opacity *= if offset == 0.0 { 1.0 } else { 0.5 };
            }
            graph
        };
        let frame = renderer.render_with(&g, ev.program(), Some(&mut sample));
        assert!(frame.stats.fx_passes > 0);
        let mut fresh = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        let expected = fresh.render_with(&g, ev.program(), Some(&mut sample));
        assert!(renderer.read(&frame.texture) == fresh.read(&expected.texture), "shutter offset {offset}");
    }
}

#[test]
fn motion_cache_tracks_paints_opacity_and_offscreen_samples() {
    let Some(gpu) = gpu() else { return };
    let d = scene(
        r#"width="96" height="64" fps="30" duration="2" motionBlur="true" motionBlurSamples="4"
            shutterPhase="0" shutterAngle="180" adaptiveMotionBlur="false""#,
        GRADIENT,
        r##"<group id="parent" opacity="0.7"><shape id="moving" shape="ellipse"
            x="8" y="8" width="32" height="24" fill="url(#g)"/></group>"##,
        "",
    );
    let ev = sr_eval::Evaluator::new(&d, &Default::default()).unwrap();
    let main = ev.evaluate(0.2);
    let mut renderer = sr_gpu::Renderer::new(gpu.clone(), ev.program());
    let mut previous = None;
    // Main-frame state is fixed. Each change exists only in the shutter samples.
    // Gradient references stay constant while their resolved stops animate.
    for (paint_shift, opacity, offset) in
        [(0.0, 1.0, 0.0), (0.5, 1.0, 0.0), (0.5, 0.5, 0.0), (0.5, 0.5, -20.25), (0.5, 0.5, 42.75), (0.0, 1.0, 0.0)]
    {
        let mut sub = |t| {
            let mut frame = ev.evaluate(t + paint_shift);
            let node = frame.nodes.iter_mut().find(|n| &*n.id == "moving").unwrap();
            node.world_opacity *= opacity;
            node.world.0[4] += offset;
            frame
        };
        let frame = renderer.render_with(&main, ev.program(), Some(&mut sub));
        let actual = renderer.read(&frame.texture);
        let mut fresh = sr_gpu::Renderer::new(gpu.clone(), ev.program());
        let reference = fresh.render_with(&main, ev.program(), Some(&mut sub));
        let expected = fresh.read(&reference.texture);
        assert_eq!(actual, expected, "paint={paint_shift}, opacity={opacity}, offset={offset}");
        assert_eq!(frame.stats.errors, reference.stats.errors);
        assert_eq!(frame.stats.unsupported, reference.stats.unsupported);
        if let Some(before) = previous.replace(expected) {
            assert_ne!(actual, before, "the changed shutter state must affect the oracle");
        }
        let reused = renderer.render_with(&main, ev.program(), Some(&mut sub));
        assert_eq!(renderer.read(&reused.texture), actual);
        assert_eq!(reused.stats.fx_passes, 0, "repeating the complete shutter state reuses its accumulation");
    }
}

#[test]
fn zero_intensity_glow_keeps_combine_pixels_without_building_a_blur() {
    let Some(gpu) = gpu() else { return };
    for working in ["", r#"linearLight="false""#, r#"workingColorSpace="acescg""#] {
        for kind in ["glow", "bloom", "halation"] {
            let make = |intensity: &str| {
                scene(
                    &format!(r##"width="79" height="43" fps="10" duration="2" background="#00000000" {working}"##),
                    "",
                    r##"<group id="g" effects="fx">
                    <shape id="a" shape="ellipse" x="2.3" y="4.7" width="61" height="29" fill="#CF583580"/>
                    <shape id="b" shape="rect" x="21.8" y="13.2" width="45" height="25" fill="#FFFDBB"/>
                </group>"##,
                    &format!(r#"<effect id="fx" type="{kind}" radius="18" threshold="0.2" intensity="{intensity}"/>"#),
                )
            };
            // A nonzero f64 that rounds to zero in the shader's f32 uniform keeps
            // the full extraction/blur/combine path as an independent pixel oracle.
            let reference = render_times_on(gpu.clone(), &make("1e-100"), &[0.0]).unwrap();
            let zero = render_times_on(gpu.clone(), &make("0"), &[0.0]).unwrap();
            assert!(reference.stats.fx_passes > 1);
            assert_eq!(zero.px, reference.px, "{kind}, {working}");
            assert_eq!(zero.stats.fx_passes, 1, "{kind}: only the original combine pass is needed");
            assert!(zero.stats.unsupported.is_empty());
            assert!(zero.stats.errors.is_empty());
        }
    }
}

/// A 32 px white square moving right at 120 px/s from x = 40, with `effects`.
fn mover(effects_attr: &str) -> String {
    format!(
        r#"<layer id="m" asset="white" x="40" y="50" scaleX="8" scaleY="8" effects="{effects_attr}"><animate property="x"><key time="0" value="40"/><key time="1" value="160"/></animate></layer>"#
    )
}

#[test]
fn posterized_layers_keep_their_effect_targets_to_the_layer() {
    let d = scene(
        WIDE,
        "",
        &mover("pt ink"),
        r##"<effect id="pt" type="posterize-time" frequency="30"/><effect id="ink" type="stroke" size="0.5" color="#FF0000"/>"##,
    );
    let Some(r) = render_sub(&d, 0.51) else { return };
    assert!(r.stats.unsupported.is_empty(), "{:?}", r.stats.unsupported);
    // posterized to t = 0.5: the square spans x 100..132; the stroke is 0.5 local units, 4 px at
    // the layer's scale of 8, so it spans 96..100 and 132..136
    assert_px(&r, 116, 66, [1.0, 1.0, 1.0, 1.0], 2e-2);
    assert_px(&r, 98, 66, [1.0, 0.0, 0.0, 1.0], 5e-2);
    assert!(r.at(140, 66)[3] < 0.02, "{:?}", r.at(140, 66));
    assert!(r.stats.effect_pixels > 0);
    assert!(r.stats.effect_pixels < FRAME / 4, "effect targets cover {} of {FRAME} px", r.stats.effect_pixels);
}

#[test]
fn echoed_layers_keep_their_effect_targets_to_the_trail() {
    let d = scene(WIDE, "", &mover("echo"), r#"<effect id="echo" type="echo" samples="3" amount="2"/>"#);
    let Some(r) = render_sub(&d, 0.5) else { return };
    assert!(r.stats.unsupported.is_empty(), "{:?}", r.stats.unsupported);
    // now at x = 100; echoes 2 and 4 frames back at 96 and 92 leave a partial trail left of it
    let a = r.at(94, 66)[3];
    assert!(a > 0.1 && a < 0.9, "trail at 94: {a}");
    assert!(r.stats.effect_pixels < FRAME / 4, "effect targets cover {} of {FRAME} px", r.stats.effect_pixels);
}

#[test]
fn shape_effect_targets_include_the_stroke() {
    // the same stroked, blurred shape drawn alone (its effect target sized to it) and inside a
    // group (the group's target is the whole frame) must be identical, corners included
    for geometry in [r#"shape="ellipse""#, r#"shape="rect""#, r#"shape="star" points="5" innerRadius="0.35""#] {
        let shape = format!(
            r##"<shape id="e" {geometry} x="60" y="40" width="40" height="40" fill="#FF0000" stroke="#FFFFFF" strokeWidth="10"{{fx}}/>"##
        );
        let fx = r#"<effect id="soft" type="blur" radius="2"/>"#;
        let alone = scene(WIDE, "", &shape.replace("{fx}", r#" effects="soft""#), fx);
        let grouped =
            scene(WIDE, "", &format!(r#"<group id="g" effects="soft">{}</group>"#, shape.replace("{fx}", "")), fx);
        let (Some(a), Some(b)) = (render_sub(&alone, 0.0), render_sub(&grouped, 0.0)) else { return };
        let db = psnr(&a.px, &b.px);
        assert!(db >= 60.0, "{geometry}: alone vs grouped: {db:.1} dB");
        // bounded by the shape, its stroke (a star keeps the full miter allowance), the blur and
        // the rounding of target sizes to steps
        assert!(
            a.stats.effect_pixels < FRAME / 2,
            "{geometry}: effect targets cover {} of {FRAME} px",
            a.stats.effect_pixels
        );
    }
}

#[test]
fn posterized_frames_are_reused_until_the_next_step() {
    let d = scene(
        WIDE,
        "",
        &mover("pt ink"),
        r##"<effect id="pt" type="posterize-time" frequency="30"/><effect id="ink" type="stroke" size="0.5" color="#FF0000"/>"##,
    );
    // two output frames in the same 1/30 s step, then one in the next
    let t0 = 0.5 + 0.2 / 60.0;
    let Some(f) = render_sub_frames(&d, &[t0, t0 + 1.0 / 60.0, t0 + 2.0 / 60.0]) else { return };
    assert!(f[0].stats.fx_passes > 0);
    assert_eq!(f[1].stats.fx_passes, 0, "the second frame of a step reuses the first");
    assert!(f[1].stats.cache_hits > f[0].stats.cache_hits);
    assert_eq!(f[1].px, f[0].px);
    assert!(f[2].stats.fx_passes > 0, "the next step renders again");
    assert_ne!(f[2].px, f[1].px);
}

const GRADIENT: &str = r##"<paints><linearGradient id="g" x1="0" y1="0" x2="1" y2="0">
  <stop offset="0" color="#FF0000"><animate property="color"><key time="0" value="#FF0000"/><key time="1" value="#0000FF"/></animate></stop>
  <stop offset="1" color="#FFFFFF"/></linearGradient></paints>"##;

#[test]
fn animated_paints_do_not_invalidate_unrelated_effect_caches() {
    let d = scene(
        WIDE,
        GRADIENT,
        r#"<layer id="b" asset="white" x="10" y="10" scaleX="4" scaleY="4" effects="soft"/>
           <shape id="grad" shape="rect" x="120" y="10" width="40" height="40" fill="url(#g)"/>"#,
        r#"<effect id="soft" type="blur" radius="2"/>"#,
    );
    let Some(f) = render_sub_frames(&d, &[0.2, 0.4]) else { return };
    assert_ne!(f[0].at(125, 30), f[1].at(125, 30), "the gradient animates");
    assert_eq!(f[1].stats.fx_passes, 0, "the blurred layer does not use the gradient");
}

#[test]
fn effects_follow_the_paints_they_use() {
    let d = scene(
        WIDE,
        GRADIENT,
        r#"<shape id="grad" shape="rect" x="120" y="10" width="40" height="40" fill="url(#g)" effects="soft"/>"#,
        r#"<effect id="soft" type="blur" radius="2"/>"#,
    );
    let Some(f) = render_sub_frames(&d, &[0.2, 0.4]) else { return };
    let fresh = render_sub(&d, 0.4).unwrap();
    assert_eq!(f[1].px, fresh.px, "no stale effect result after the paint changed");
    assert!(f[1].stats.fx_passes > 0);
}

#[test]
fn wide_strokes_cover_every_pixel_within_their_width() {
    // a one-pixel dot stroked 20 px wide is a filled disc of radius 20 around it
    let d = scene(
        r##"width="96" height="96" fps="10" duration="1" background="#00000000""##,
        "",
        r##"<shape id="dot" shape="rect" x="47" y="47" width="1" height="1" fill="#FFFFFF" effects="ring"/>"##,
        r##"<effect id="ring" type="stroke" size="20" color="#FF0000"/>"##,
    );
    let Some(r) = render_sub(&d, 0.0) else { return };
    let (mut holes, mut spill) = (Vec::new(), Vec::new());
    for y in 0..96u32 {
        for x in 0..96u32 {
            // distance from the pixel centre to the dot's pixel square [47, 48]²
            let dx = (47.0 - (x as f64 + 0.5)).max(x as f64 + 0.5 - 48.0).max(0.0);
            let dy = (47.0 - (y as f64 + 0.5)).max(y as f64 + 0.5 - 48.0).max(0.0);
            let dist = (dx * dx + dy * dy).sqrt();
            let a = r.at(x, y)[3];
            if dist <= 18.5 && a < 0.95 {
                holes.push((x, y, a));
            }
            if dist >= 21.5 && a > 0.05 {
                spill.push((x, y, a));
            }
        }
    }
    assert!(
        holes.is_empty(),
        "{} uncovered pixels inside the stroke, e.g. {:?}",
        holes.len(),
        &holes[..holes.len().min(5)]
    );
    assert!(
        spill.is_empty(),
        "{} pixels covered beyond the stroke, e.g. {:?}",
        spill.len(),
        &spill[..spill.len().min(5)]
    );
}

#[test]
fn matte_choke_erodes_by_its_amount() {
    let d = scene(
        r##"width="96" height="96" fps="10" duration="1" background="#00000000""##,
        "",
        r##"<shape id="sq" shape="rect" x="28" y="28" width="40" height="40" fill="#FFFFFF" effects="choke"/>"##,
        r#"<effect id="choke" type="matte-choke" amount="12" softness="0"/>"#,
    );
    let Some(r) = render_sub(&d, 0.0) else { return };
    // the square shrinks from 28..68 to 40..56
    for (x, y) in [(42, 48), (48, 42), (53, 53)] {
        assert!(r.at(x, y)[3] > 0.95, "inside at ({x},{y}): {:?}", r.at(x, y));
    }
    for (x, y) in [(37, 48), (48, 37), (59, 48), (30, 30)] {
        assert!(r.at(x, y)[3] < 0.05, "eroded at ({x},{y}): {:?}", r.at(x, y));
    }
}

/// The shutter's sample times: `count` samples across a 180° shutter from the frame time.
fn sample_times(t: f64, fps: f64, count: usize) -> Vec<f64> {
    (0..count).map(|k| t + 0.5 * (k as f64 + 0.5) / count as f64 / fps).collect()
}

/// Motion blur's definition: the average of the frame rendered sharp at every sample time.
fn average_of_sharp_frames(sharp: &sr_model::Document, times: &[f64]) -> Option<Vec<[f32; 4]>> {
    let mut acc: Option<Vec<[f32; 4]>> = None;
    for &t in times {
        let r = render_times(sharp, &[t])?;
        let acc = acc.get_or_insert_with(|| vec![[0.0; 4]; r.px.len()]);
        for (a, p) in acc.iter_mut().zip(&r.px) {
            for c in 0..4 {
                a[c] += p[c] / times.len() as f32;
            }
        }
    }
    acc
}

const BLUR_PROJECT: &str = r##"width="128" height="64" fps="10" duration="4" background="#00000000" shutterAngle="180" shutterPhase="0" motionBlurSamples="12""##;

#[test]
fn rigid_layers_are_rendered_once_under_motion_blur() {
    // a blurred square moving at 96 px/s: 4.8 px of travel across the shutter
    let body = r#"<layer id="m" asset="white" x="8" y="16" scaleX="6" scaleY="6" effects="soft"><animate property="x"><key time="0" value="0"/><key time="1" value="96"/></animate></layer>"#;
    let fx = r#"<effect id="soft" type="blur" radius="2"/>"#;
    let blurred = scene(&format!(r#"{BLUR_PROJECT} motionBlur="true""#), "", body, fx);
    let sharp = scene(&format!(r#"{BLUR_PROJECT} motionBlur="false""#), "", body, fx);
    let Some(r) = render_sub(&blurred, 0.5) else { return };
    let want = average_of_sharp_frames(&sharp, &sample_times(0.5, 10.0, 12)).unwrap();
    let db = psnr(&r.px, &want);
    assert!(db >= 40.0, "motion blur vs the average of sharp frames: {db:.1} dB");
    let one_chain = render_times(&sharp, &[0.5]).unwrap().stats.fx_passes;
    assert!(
        r.stats.fx_passes <= one_chain + 12,
        "{} effect passes; one blur chain is {one_chain}, plus one accumulation per sample",
        r.stats.fx_passes
    );
}

#[test]
fn layers_whose_content_changes_are_still_sampled_one_by_one() {
    // the fill changes during the shutter, so every sample must be drawn as it is
    let body = r##"<shape id="m" shape="rect" x="8" y="16" width="24" height="24" fill="#FF0000">
      <animate property="x"><key time="0" value="0"/><key time="1" value="96"/></animate>
      <animate property="fill"><key time="0" value="#FF0000"/><key time="1" value="#0000FF"/></animate></shape>"##;
    let blurred = scene(&format!(r#"{BLUR_PROJECT} motionBlur="true""#), "", body, "");
    let sharp = scene(&format!(r#"{BLUR_PROJECT} motionBlur="false""#), "", body, "");
    let Some(r) = render_sub(&blurred, 0.5) else { return };
    let want = average_of_sharp_frames(&sharp, &sample_times(0.5, 10.0, 12)).unwrap();
    let db = psnr(&r.px, &want);
    assert!(db >= 40.0, "motion blur vs the average of sharp frames: {db:.1} dB");
}

#[test]
fn transformed_layers_never_reuse_a_stale_effect_result() {
    // a blurred layer that only rotates, and one that only fades: a cached result must follow
    // the transform, and opacity is applied after the effects
    for anim in [
        r#"<animate property="rotation"><key time="0" value="0"/><key time="1" value="90"/></animate>"#,
        r#"<animate property="x"><key time="0" value="40"/><key time="1" value="40.4"/></animate>"#,
    ] {
        let d = scene(
            WIDE,
            "",
            &format!(r#"<layer id="r" asset="wide" x="60" y="40" scaleX="6" scaleY="6" effects="soft">{anim}</layer>"#),
            r#"<effect id="soft" type="blur" radius="0.3"/>"#,
        );
        let Some(f) = render_sub_frames(&d, &[0.2, 0.4]) else { return };
        let fresh = render_sub(&d, 0.4).unwrap();
        assert_eq!(f[1].px, fresh.px, "cached result reused after {anim}");
    }
}

#[test]
fn rigid_motion_blur_matches_sampling_for_entries_and_rotation() {
    // a layer sliding in from beyond the left edge (drawn once, it must not be clipped to the
    // frame), and one that spins: both are still one drawing moved per sample
    let fx = r#"<effect id="soft" type="blur" radius="0.5"/>"#;
    for (at, anim) in [
        (
            r#"x="-10" y="20""#,
            r#"<animate property="x"><key time="0" value="-60"/><key time="1" value="140"/></animate>"#,
        ),
        (
            r#"x="64" y="32" anchorX="4" anchorY="2""#,
            r#"<animate property="rotation"><key time="0" value="0"/><key time="1" value="360"/></animate>"#,
        ),
    ] {
        let body = format!(r#"<layer id="m" asset="wide" {at} scaleX="6" scaleY="6" effects="soft">{anim}</layer>"#);
        let blurred = scene(&format!(r#"{BLUR_PROJECT} motionBlur="true""#), "", &body, fx);
        let sharp = scene(&format!(r#"{BLUR_PROJECT} motionBlur="false""#), "", &body, fx);
        let Some(r) = render_sub(&blurred, 0.3) else { return };
        let want = average_of_sharp_frames(&sharp, &sample_times(0.3, 10.0, 12)).unwrap();
        let covered: f32 = want.iter().map(|p| p[3]).sum();
        assert!(covered > 200.0, "{anim}: the layer is in view ({covered} px)");
        let db = psnr(&r.px, &want);
        eprintln!("{anim}: {db:.1} dB, {} passes", r.stats.fx_passes);
        assert!(db >= 40.0, "{anim}: motion blur vs the average of sharp frames: {db:.1} dB");
    }
}

#[test]
fn regression_generator_secondary_paint_invalidates_effects() {
    let xml = format!(
        r##"<scene version="1.2"><project width="64" height="64" fps="10" duration="1"/>
      <assets><generator id="a" kind="checkerboard" width="64" height="64" scale="16" paint="#FFFFFF" paint2="url(#g)"/></assets>
      {GRADIENT}<composition><layer id="l" asset="a" effects="soft"/></composition>
      <effects><effect id="soft" type="blur" radius="1"/></effects></scene>"##
    );
    let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let Some(f) = render_sub_frames(&d, &[0.0, 0.5]) else { return };
    let fresh = render_sub(&d, 0.5).unwrap();
    assert_ne!(f[0].px, fresh.px, "the secondary paint must actually animate");
    assert!(f[1].px == fresh.px, "cached rendering must follow paint2");
}

#[test]
fn regression_wide_morphology_preserves_partial_alpha() {
    for alpha in ["40", "BF"] {
        for radius in [4, 5, 12] {
            for kind in ["stroke", "matte-choke"] {
                let d = scene(
                    r##"width="96" height="96" fps="10" duration="1" background="#00000000""##,
                    "",
                    &format!(
                        r##"<shape id="sq" shape="rect" x="28" y="28" width="40" height="40" fill="#FFFFFF{alpha}" effects="fx"/>"##
                    ),
                    &format!(
                        r##"<effect id="fx" type="{kind}" size="{radius}" amount="{radius}" softness="0" color="#FF0000"/>"##
                    ),
                );
                let Some(r) = render_sub(&d, 0.0) else { return };
                let expected = u8::from_str_radix(alpha, 16).unwrap() as f32 / 255.0;
                let point = if kind == "stroke" { (26, 48) } else { (48, 48) };
                let got = r.at(point.0, point.1)[3];
                assert!((got - expected).abs() < 0.02, "{kind} radius {radius}, alpha {alpha}: {got} != {expected}");
                if kind == "matte-choke" {
                    assert!(r.at(29, 48)[3] < 0.02, "the edge must still erode");
                }
            }
        }
    }
}

#[test]
fn regression_animated_generator_paint_references_follow_their_gradients() {
    // The reference stays constant in evaluated props while the referenced gradient changes.
    // Check both generator paint slots and an isolated parent effect cache.
    for property in ["paint", "paint2"] {
        for grouped in [false, true] {
            let body = if grouped {
                r#"<group id="outer" effects="soft"><layer id="l" asset="a"/></group>"#
            } else {
                r#"<layer id="l" asset="a" effects="soft"/>"#
            };
            let xml = format!(
                r##"<scene version="1.2"><project width="64" height="64" fps="10" duration="1"/>
              <assets><generator id="a" kind="checkerboard" width="64" height="64" scale="16" paint="#FFFFFF" paint2="#FFFFFF">
              <animate property="{property}"><key time="0" value="url(#g)"/><key time="1" value="url(#g)"/></animate></generator></assets>
              {GRADIENT}<composition>{body}</composition><effects><effect id="soft" type="blur" radius="1"/></effects></scene>"##
            );
            let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap();
            let Some(f) = render_sub_frames(&doc, &[0.0, 0.5]) else { return };
            let fresh = render_sub(&doc, 0.5).unwrap();
            assert!(f[0].px != fresh.px, "gradient must animate");
            assert!(f[1].px == fresh.px, "{property}, grouped={grouped}: stale animated paint dependency");
        }
    }
}

#[test]
fn regression_film_grain_keeps_animating_under_cached_effects() {
    let xml = r##"<scene version="1.2"><project width="64" height="64" fps="10" duration="1"/>
      <assets><generator id="a" kind="film-grain" width="64" height="64" paint="#FFFFFF" paint2="#000000" seed="7"/></assets>
      <composition><layer id="l" asset="a" effects="soft"/></composition><effects><effect id="soft" type="blur" radius="1"/></effects></scene>"##;
    let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
    let Some(f) = render_sub_frames(&doc, &[0.0, 0.5, 0.5]) else { return };
    let fresh = render_sub(&doc, 0.5).unwrap();
    assert!(f[0].px != fresh.px, "grain must change with its frame");
    assert!(f[1].px == fresh.px, "effect cache froze the grain");
    assert!(f[2].px == f[1].px, "grain remains deterministic");
    assert_eq!(f[2].stats.fx_passes, 0, "same-time grain still reuses its effects");
}

#[test]
fn empty_camera_controls_do_not_accumulate_transparent_frames() {
    let Some(gpu) = gpu() else { return };
    for kind in ["group", "camera"] {
        let make = |motion: &str| {
            scene(
                &format!(
                    r##"width="96" height="64" fps="30" duration="2" background="#234567"
                motionBlur="{motion}" motionBlurSamples="4" adaptiveMotionBlur="false""##
                ),
                "",
                &format!(
                    r##"<shape id="mark" shape="ellipse" x="12" y="8" width="45" height="31"
                fill="#ED634580" motionBlur="off"/>
                <{kind} id="control" x="5">
                    <animate property="x"><key time="0" value="5"/><key time="1" value="205"/></animate>
                </{kind}>"##
                ),
                "",
            )
        };
        let draw = |motion: &str| {
            let document = make(motion);
            let evaluator = sr_eval::Evaluator::new(&document, &Default::default()).unwrap();
            let mut renderer = sr_gpu::Renderer::new(gpu.clone(), evaluator.program());
            let frame = renderer.render_with(
                &evaluator.evaluate(0.4),
                evaluator.program(),
                Some(&mut |time| evaluator.evaluate(time)),
            );
            (renderer.read(&frame.texture), frame.stats)
        };
        let (actual, stats) = draw("true");
        let (reference, reference_stats) = draw("false");
        assert_eq!(actual, reference, "{kind}: the controller has no own ink");
        assert_eq!(stats.errors, reference_stats.errors);
        assert_eq!(stats.unsupported, reference_stats.unsupported);
        assert_eq!(stats.fx_passes, reference_stats.fx_passes, "empty camera controls need no accumulation passes");
    }
}
