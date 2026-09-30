//! Quality tiers (`project@quality`, or the renderer's override): `draft` renders at half
//! resolution in the document's coordinates with capped motion blur and without grain;
//! `preview` keeps the resolution and caps motion blur; `final` renders as authored.
//! A draft frame must look like the final frame at half the size, so each feature is
//! compared with the final render box-filtered to half size.

mod common;

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
