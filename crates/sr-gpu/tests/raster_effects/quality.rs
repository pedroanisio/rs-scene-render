//! Quality tiers (`project@quality`, or the renderer's override): `draft` renders at half
//! resolution in the document's coordinates with capped motion blur and without grain;
//! `preview` keeps the resolution and caps motion blur; `final` renders as authored.
//! A draft frame must look like the final frame at half the size, so each feature is
//! compared with the final render box-filtered to half size.

use super::common;

use common::*;
use sr_gpu::{Gpu, Renderer};
use sr_model::model::ProjectQuality;

/// Renders `d` at time `t` with `quality` overriding the document's, with a sub-frame provider.
fn at_quality(gpu: &Gpu, d: &sr_model::Document, t: f64, quality: Option<ProjectQuality>) -> Shot {
    let ev = sr_eval::Evaluator::new(d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = Renderer::new(gpu.clone(), ev.program());
    r.quality = quality;
    // delivery turns probes on when the document's accessibility metadata asks for contrast checks
    r.contrast_probe = true;
    let g = ev.evaluate(t);
    let mut sub = |st: f64| ev.evaluate(st);
    let f = r.render_with(&g, ev.program(), Some(&mut sub));
    Shot { px: r.read(&f.texture), size: f.texture.size, stats: f.stats }
}

/// A frame box-filtered to half its size.
fn half(s: &Shot) -> Vec<[f32; 4]> {
    let (w, h) = (s.size[0] / 2, s.size[1] / 2);
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let mut p = [0.0f32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let q = s.at(2 * x + dx, 2 * y + dy);
                for c in 0..4 {
                    p[c] += q[c] * 0.25;
                }
            }
            out.push(p);
        }
    }
    out
}

fn scene(project: &str, body: &str, extra: &str, effects: &str) -> sr_model::Document {
    let effects = if effects.is_empty() { String::new() } else { format!("<effects>{effects}</effects>") };
    let xml = format!(
        r#"<scene version="1.1"><project width="128" height="96" fps="10" duration="2" {project}/>{ASSETS}{extra}<composition>{body}</composition>{effects}</scene>"#
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

#[test]
fn draft_frames_are_the_final_frames_at_half_size() {
    let Some(gpu) = gpu() else { return };
    let cases: &[(&str, &str, &str, &str)] = &[
        (
            "layers and shapes",
            r##"<layer id="img" asset="wide" x="20" y="14" scaleX="9" scaleY="9" rotation="18"/>
               <shape id="e" shape="ellipse" x="60" y="40" width="50" height="36" fill="url(#g)" stroke="#FFFFFF" strokeWidth="4"/>"##,
            r##"<paints><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" color="#FF6A1A"/><stop offset="1" color="#1A6AFF"/></linearGradient></paints>"##,
            "",
        ),
        (
            "effects",
            r#"<layer id="img" asset="wide" x="20" y="20" scaleX="9" scaleY="9" effects="soft shadow ink"/>"#,
            "",
            r##"<effect id="soft" type="blur" radius="0.2"/><effect id="shadow" type="drop-shadow" offsetX="0.5" offsetY="0.5" radius="0.4"/><effect id="ink" type="stroke" size="0.3" color="#00FF00"/>"##,
        ),
        (
            "masks and blending",
            r##"<shape id="base" shape="rect" x="0" y="0" width="128" height="96" fill="#304050"/>
               <layer id="img" asset="wide" x="10" y="10" scaleX="12" scaleY="16" blend="screen">
                 <mask type="ellipse" x="0" y="0" width="8" height="4" feather="0.5"/></layer>"##,
            "",
            "",
        ),
        (
            "particles",
            r##"<particleEmitter id="p" x="64" y="48" emitterShape="point" rate="0" lifetime="3" speed="40" spread="360" size="5" color="#FFFFFF" shape="disc" seed="4"><burst time="0" count="40"/></particleEmitter>"##,
            "",
            "",
        ),
        (
            "text",
            r#"<layer id="t" asset="words" x="8" y="20"/>"#,
            r##"<assets><text id="words" text="Draft" width="110" height="60" size="44" color="#FFFFFF" font="DejaVu Sans"/></assets>"##,
            "",
        ),
        (
            "transitions",
            r##"<sequence id="tseq"><layer id="a" asset="red" end="1" x="16" y="12" scaleX="24" scaleY="18"/>
                 <transition id="tr" type="push" from="a" to="b" duration="1"><param name="direction" value="0"/></transition>
                 <layer id="b" asset="wide" start="1" x="16" y="12" scaleX="12" scaleY="18"/></sequence>"##,
            "",
            "",
        ),
        (
            "adjustment layers",
            r#"<layer id="img" asset="wide" x="0" y="0" scaleX="16" scaleY="24"/><adjustment id="finish" effects="grade vig"/>"#,
            "",
            r#"<effect id="grade" type="color-grade" saturation="1.3" contrast="1.2"/><effect id="vig" type="vignette" amount="0.5" radius="40"/>"#,
        ),
        (
            "3D",
            r#"<object3D id="box" primitive="box" width="40" height="40" depth="40" x="64" y="48" rotationY="30" rotationX="20"/>"#,
            "",
            "",
        ),
    ];
    let mut failures = Vec::new();
    for (name, body, extra, effects) in cases {
        // the text case declares its own assets
        let d = if extra.starts_with("<assets>") {
            let xml = format!(
                r##"<scene version="1.1"><project width="128" height="96" fps="10" duration="2" background="#101820"/>{extra}<composition>{body}</composition></scene>"##
            );
            sr_model::load_str(&xml, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"))
        } else {
            scene(r##"background="#101820""##, body, extra, effects)
        };
        let full = at_quality(&gpu, &d, 0.5, None);
        let draft = at_quality(&gpu, &d, 0.5, Some(ProjectQuality::Draft));
        if draft.size != [64, 48] {
            failures.push(format!("{name}: draft frame is {:?}", draft.size));
            continue;
        }
        // a feature drawn at the wrong scale or place scores below 15 dB; resampling alone (thin
        // strokes of soft edges sampled at half the radius) stays near 30
        let db = psnr(&draft.px, &half(&full));
        if db < 28.0 {
            failures.push(format!("{name}: {db:.1} dB against the final frame at half size"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_document_quality_applies_unless_the_renderer_overrides_it() {
    let Some(gpu) = gpu() else { return };
    let d = scene(r#"quality="draft""#, r#"<layer id="img" asset="wide" scaleX="8" scaleY="8"/>"#, "", "");
    assert_eq!(at_quality(&gpu, &d, 0.0, None).size, [64, 48]);
    assert_eq!(at_quality(&gpu, &d, 0.0, Some(ProjectQuality::Final)).size, [128, 96]);
    let p = scene(r#"quality="preview""#, r#"<layer id="img" asset="wide" scaleX="8" scaleY="8"/>"#, "", "");
    assert_eq!(at_quality(&gpu, &p, 0.0, None).size, [128, 96]);
}

#[test]
fn drafts_cap_motion_blur_and_skip_grain() {
    let Some(gpu) = gpu() else { return };
    let moving = r##"<shape id="m" shape="rect" x="10" y="20" width="30" height="30" fill="#FFFFFF">
        <animate property="x"><key time="0" value="0"/><key time="2" value="90"/></animate>
        <animate property="fill"><key time="0" value="#FFFFFF"/><key time="2" value="#FF0000"/></animate></shape>"##;
    let d = scene(r#"motionBlur="true" motionBlurSamples="16" shutterAngle="180""#, moving, "", "");
    let fin = at_quality(&gpu, &d, 0.5, Some(ProjectQuality::Final));
    let pre = at_quality(&gpu, &d, 0.5, Some(ProjectQuality::Preview));
    let dra = at_quality(&gpu, &d, 0.5, Some(ProjectQuality::Draft));
    assert!(fin.stats.subframes >= 16, "final: {} sub-frames", fin.stats.subframes);
    assert!(pre.stats.subframes <= 4 && pre.stats.subframes > 1, "preview: {} sub-frames", pre.stats.subframes);
    assert!(dra.stats.subframes <= 2 && dra.stats.subframes > 0, "draft: {} sub-frames", dra.stats.subframes);
    assert_eq!(pre.size, [128, 96]);

    let grainy = scene(
        "",
        r#"<layer id="img" asset="wide" scaleX="16" scaleY="24" effects="grain"/>"#,
        "",
        r#"<effect id="grain" type="film-grain" amount="0.2" seed="3"/>"#,
    );
    assert!(at_quality(&gpu, &grainy, 0.0, None).stats.fx_passes > 0);
    let draft = at_quality(&gpu, &grainy, 0.0, Some(ProjectQuality::Draft));
    assert_eq!(draft.stats.fx_passes, 0, "grain is left out of drafts");
}

#[test]
fn drafts_still_measure_text_contrast() {
    // accessibility probes measure text against what is behind it; a draft must measure too,
    // and find about the same ratio
    let Some(gpu) = gpu() else { return };
    let xml = r##"<scene version="1.1"><project width="128" height="96" fps="10" duration="2" background="#303030"/>
        <metadata><accessibility contrastCheck="warn" flashCheck="off"/></metadata>
        <assets><text id="words" text="Grey" width="110" height="60" size="44" color="#707070" font="DejaVu Sans"/></assets>
        <composition><layer id="t" asset="words" x="8" y="20"/></composition></scene>"##;
    let d = sr_model::load_str(xml, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let full = at_quality(&gpu, &d, 0.0, Some(ProjectQuality::Final));
    let draft = at_quality(&gpu, &d, 0.0, Some(ProjectQuality::Draft));
    assert_eq!(full.stats.contrast.len(), 1, "{:?}", full.stats);
    assert_eq!(draft.stats.contrast.len(), 1, "a draft measures contrast too: {:?}", draft.stats.contrast_unprobed);
    let (a, b) = (full.stats.contrast[0].1, draft.stats.contrast[0].1);
    assert!((a - b).abs() / a < 0.1, "contrast {a:.2} at full size, {b:.2} in the draft");
}

#[test]
fn masks_of_unsized_nodes_resolve_against_the_document_at_every_tier() {
    // a group, a group with effects and an adjustment, none with a box of its own: their masks'
    // percentages are of the frame in document units, not of the draft's half-size target
    let Some(gpu) = gpu() else { return };
    let picture = r#"<layer id="img" asset="wide" x="0" y="0" scaleX="16" scaleY="24"/>"#;
    let mask = r#"<mask type="ellipse" x="0" y="0" width="100%" height="100%"/>"#;
    let fx = r#"<effect id="soft" type="blur" radius="1"/><effect id="inv" type="invert"/>"#;
    for (name, body) in [
        ("group", format!(r#"<group id="grp">{mask}{picture}</group>"#)),
        ("group with effects", format!(r#"<group id="grp" effects="soft">{mask}{picture}</group>"#)),
        ("adjustment", format!(r#"{picture}<adjustment id="adj" effects="inv">{mask}</adjustment>"#)),
    ] {
        let d = scene(r##"background="#101820""##, &body, "", fx);
        let full = at_quality(&gpu, &d, 0.5, None);
        let draft = at_quality(&gpu, &d, 0.5, Some(ProjectQuality::Draft));
        // inside the ellipse near its right and bottom edges
        for (x, y) in [(110, 48), (64, 86)] {
            let (a, b) = (full.at(x, y), draft.at(x / 2, y / 2));
            assert!(close(a, b, 0.05), "{name}: ({x},{y}) is {a:?}, the draft has {b:?}");
        }
        let db = psnr(&draft.px, &half(&full));
        assert!(db >= 28.0, "{name}: {db:.1} dB against the final frame at half size");
    }
}

#[test]
fn point_lights_fall_off_over_their_range_in_document_units() {
    // a light of range 12 at (64, 48) over a grey bar, wide or tall: lit within 12 px of it in
    // every direction, at every tier, whatever the shape of the effect's target
    let Some(gpu) = gpu() else { return };
    for (bar, light, unlit, far) in [
        (
            r#"x="4" y="24" width="120" height="48""#,
            r#"x="60" y="24""#,
            (112, 48),
            [(79, 48), (49, 48), (64, 63), (64, 33)],
        ),
        (
            r#"x="40" y="4" width="48" height="88""#,
            r#"x="24" y="44""#,
            (64, 84),
            [(79, 48), (49, 48), (64, 63), (64, 33)],
        ),
    ] {
        let xml = format!(
            r##"<scene version="1.1"><project width="128" height="96" fps="10" duration="2" background="#000000"/>
            <composition><shape id="bar" shape="rect" {bar} fill="#808080" effects="lit"/></composition>
            <lights><light id="pl" type="point" {light} range="12" intensity="2"/></lights>
            <effects><effect id="lit" type="lighting" lights="pl" intensity="1"/></effects></scene>"##
        );
        let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
        for quality in [None, Some(ProjectQuality::Draft)] {
            let s = at_quality(&gpu, &d, 0.0, quality);
            assert!(s.stats.unsupported.is_empty(), "{:?}", s.stats.unsupported);
            let k = 128 / s.size[0];
            let lum = |(x, y): (u32, u32)| s.at(x / k, y / k)[1];
            let base = lum(unlit);
            let (right, below) = (lum((70, 48)), lum((64, 54)));
            assert!(right > base * 1.2, "{bar} {quality:?}: 6 px right of the light {right}, unlit {base}");
            assert!((right - below).abs() < 0.1 * right, "{bar} {quality:?}: right {right}, below {below}");
            for p in far {
                let v = lum(p);
                assert!((v - base).abs() < 0.02 * base, "{bar} {quality:?}: 15 px away at {p:?} {v}, unlit {base}");
            }
        }
    }
}

#[test]
fn drafts_burn_captions_where_the_final_frame_has_them() {
    // burned captions were built in document coordinates and drawn into the half-size draft target
    // unscaled, so they landed off the frame and vanished
    let Some(gpu) = gpu() else { return };
    let xml = r##"<scene version="1.1"><project width="128" height="96" fps="10" duration="2" background="#101820"/>
<styles><textStyle id="cap" size="14" color="#FFFFFF"/></styles>
<composition/>
<captions><captionTrack id="cc" language="en" mode="burn" preset="classic" style="cap" x="50%" y="55%" width="90%"><cue start="0" end="2" text="Draft caption"/></captionTrack></captions></scene>"##;
    let d = sr_model::load_str(xml, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let full = at_quality(&gpu, &d, 0.5, Some(ProjectQuality::Final));
    let draft = at_quality(&gpu, &d, 0.5, Some(ProjectQuality::Draft));
    assert_eq!(draft.size, [64, 48]);
    let lit = |s: &Shot| {
        (0..s.size[1]).flat_map(|y| (0..s.size[0]).map(move |x| (x, y))).filter(|&(x, y)| s.at(x, y)[0] > 0.5).count()
    };
    assert!(lit(&full) > 20, "the final frame has the caption: {}", lit(&full));
    assert!(lit(&draft) > 5, "the draft frame lost its caption: {} lit pixels", lit(&draft));
    let db = psnr(&draft.px, &half(&full));
    assert!(db >= 28.0, "captions at the wrong place or scale: {db:.1} dB against the final frame at half size");
}
