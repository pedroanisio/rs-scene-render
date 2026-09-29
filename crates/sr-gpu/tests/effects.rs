//! Batch 7: effects, transitions, adjustment layers, motion blur and colour finishing on the GPU.

mod common;
use common::*;

fn fx_doc(project: &str, body: &str, effects: &str) -> sr_model::Document {
    doc_with(project, "", body, &format!("<effects>{effects}</effects>"))
}

fn alpha_sum(r: &Rendered) -> f32 {
    r.px.iter().map(|p| p[3]).sum()
}

fn problems(r: &Rendered) -> Vec<String> {
    r.stats.unsupported.iter().chain(&r.stats.errors).cloned().collect()
}

#[test]
fn blur_spreads_and_conserves_coverage() {
    let body = r#"<layer id="a" asset="white" x="24" y="8" scaleX="4" scaleY="4"/>"#;
    let Some(sharp) = render(&doc_with(r##"background="#00000000""##, "", body, "")) else { return };
    let blurred = render(&fx_doc(
        r##"background="#00000000""##,
        r#"<layer id="a" asset="white" x="24" y="8" scaleX="4" scaleY="4" effects="b"/>"#,
        r#"<effect id="b" type="blur" radius="2"/>"#,
    ))
    .unwrap();
    assert!(problems(&blurred).is_empty(), "{:?}", problems(&blurred));
    let (s0, s1) = (alpha_sum(&sharp), alpha_sum(&blurred));
    assert!((s1 - s0).abs() / s0 < 0.05, "coverage {s0} → {s1}");
    assert!(blurred.at(22, 16)[3] > 0.05, "spreads outside the box: {:?}", blurred.at(22, 16));
    assert!(blurred.at(24, 8)[3] < 0.6, "corner softens: {:?}", blurred.at(24, 8));
    assert!(blurred.at(32, 16)[3] > 0.85, "centre stays nearly solid: {:?}", blurred.at(32, 16));
}

#[test]
fn glow_and_drop_shadow() {
    let body = r##"<layer id="a" asset="white" x="8" y="8" scaleX="4" scaleY="4" effects="g"/>
        <layer id="b" asset="red" x="40" y="4" scaleX="3" scaleY="3" effects="s"/>"##;
    let fx = r##"<effect id="g" type="glow" radius="1.5" threshold="0.5" intensity="1"/>
        <effect id="s" type="drop-shadow" radius="0" offsetX="2" offsetY="2" color="#000000FF"/>"##;
    let Some(r) = render(&fx_doc(r##"background="#808080""##, body, fx)) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    let bg = lin8(128);
    // glow brightens the background next to the white square
    assert!(r.at(26, 16)[0] > bg + 0.02, "glow {:?}", r.at(26, 16));
    // the shadow sits below-right of the red square, not above-left
    assert_px(&r, 50, 18, [0.0, 0.0, 0.0, 1.0], 2e-2);
    assert_px(&r, 38, 2, [bg, bg, bg, 1.0], 2e-2);
    assert_px(&r, 44, 8, [1.0, 0.0, 0.0, 1.0], 2e-2);
}

#[test]
fn colour_operations_match_reference_formulas() {
    let body = r#"<layer id="e" asset="gray" x="0" y="0" scaleX="2" scaleY="2" effects="exp"/>
        <layer id="s" asset="src" x="8" y="0" scaleX="2" scaleY="2" effects="sat"/>
        <layer id="i" asset="src" x="16" y="0" scaleX="2" scaleY="2" effects="inv"/>
        <layer id="p" asset="gray" x="24" y="0" scaleX="2" scaleY="2" effects="post"/>
        <layer id="t" asset="gray" x="32" y="0" scaleX="2" scaleY="2" effects="thr"/>
        <layer id="c" asset="gray" x="40" y="0" scaleX="2" scaleY="2" effects="cdl"/>
        <layer id="m" asset="gray" x="48" y="0" scaleX="2" scaleY="2" effects="mixed"/>
        <layer id="tm" asset="white" x="56" y="0" scaleX="2" scaleY="2" effects="exp tmo"/>"#;
    let fx = r#"<effect id="exp" type="exposure" exposure="1"/>
        <effect id="sat" type="color-grade" saturation="0"/>
        <effect id="inv" type="invert"/>
        <effect id="post" type="posterize" levels="2"/>
        <effect id="thr" type="threshold" threshold="0.4"/>
        <effect id="cdl" type="cdl" slope="2" offset="0" power="1"/>
        <effect id="mixed" type="invert" mix="0.5"/>
        <effect id="tmo" type="tonemap" tonemapper="reinhard"/>"#;
    let Some(r) = render(&fx_doc("", body, fx)) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    let g = lin8(128);
    assert_px(&r, 4, 4, [2.0 * g, 2.0 * g, 2.0 * g, 1.0], 1e-2);
    let (sr, sg, sb) = (lin8(200), lin8(100), lin8(50));
    let l = 0.2126 * sr + 0.7152 * sg + 0.0722 * sb;
    assert_px(&r, 12, 4, [l, l, l, 1.0], 1e-2);
    let inv = |v: u8| srgb_to_linear(1.0 - v as f32 / 255.0);
    assert_px(&r, 20, 4, [inv(200), inv(100), inv(50), 1.0], 1e-2);
    // 128/255 posterized to 2 levels rounds up to 1
    assert_px(&r, 28, 4, [1.0, 1.0, 1.0, 1.0], 1e-2);
    assert_px(&r, 36, 4, [1.0, 1.0, 1.0, 1.0], 1e-2);
    let cdl = srgb_to_linear((2.0 * 128.0 / 255.0f32).min(10.0));
    assert_px(&r, 44, 4, [cdl, cdl, cdl, 1.0], 2e-2);
    let half = (g + inv(128)) * 0.5;
    assert_px(&r, 52, 4, [half, half, half, 1.0], 1e-2);
    // exposure +1 then Reinhard: 2 / 3
    assert_px(&r, 60, 4, [2.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0, 1.0], 1e-2);
}

#[test]
fn luts_from_files() {
    let dir = fixtures();
    let mut id = String::from("LUT_3D_SIZE 2\n");
    let mut inv = String::from("LUT_3D_SIZE 2\n");
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                id.push_str(&format!("{r} {g} {b}\n"));
                inv.push_str(&format!("{} {} {}\n", 1 - r, 1 - g, 1 - b));
            }
        }
    }
    std::fs::write(dir.join("id.cube"), id).unwrap();
    std::fs::write(dir.join("inv.cube"), inv).unwrap();
    let body = r#"<layer id="a" asset="src" x="0" y="0" scaleX="4" scaleY="4" effects="id"/>
        <layer id="b" asset="src" x="20" y="0" scaleX="4" scaleY="4" effects="inv"/>"#;
    let fx = r#"<effect id="id" type="lut" src="id.cube" space="srgb"/><effect id="inv" type="lut" src="inv.cube" space="srgb"/>"#;
    let Some(r) = render(&fx_doc("", body, fx)) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    assert_px(&r, 8, 8, [lin8(200), lin8(100), lin8(50), 1.0], 1.5e-2);
    let inv = |v: u8| srgb_to_linear(1.0 - v as f32 / 255.0);
    assert_px(&r, 28, 8, [inv(200), inv(100), inv(50), 1.0], 1.5e-2);
}

#[test]
fn keyers_and_warps() {
    let body = r#"<layer id="k" asset="solid" x="0" y="0" scaleX="4" scaleY="4" effects="key"/>
        <layer id="r" asset="red" x="16" y="0" scaleX="4" scaleY="4" effects="key"/>
        <layer id="q" asset="quad" x="32" y="0" scaleX="16" scaleY="16" effects="pix"/>"#;
    let fx = r##"<effect id="key" type="chroma-key" keyColor="#00FF00" tolerance="0.2" softness="0.05"/>
        <effect id="pix" type="pixelate" size="1.001"/>"##;
    let Some(r) = render(&fx_doc(r##"background="#00000000""##, body, fx)) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    assert!(r.at(8, 8)[3] < 0.02, "green keyed out: {:?}", r.at(8, 8));
    assert_px(&r, 24, 8, [1.0, 0.0, 0.0, 1.0], 1e-2);
    // pixelate with the cell equal to one source texel keeps quadrants solid
    // pixelate: each 16 px cell is one colour, and neighbouring cells differ
    assert!(close(r.at(33, 2), r.at(45, 13), 1e-3) && close(r.at(49, 2), r.at(61, 13), 1e-3));
    assert!(!close(r.at(40, 8), r.at(56, 8), 0.1) && !close(r.at(40, 8), r.at(40, 24), 0.1));
}

const TRANSITIONS: [&str; 34] = [
    "cut",
    "crossfade",
    "additive-dissolve",
    "dip-to-color",
    "wipe",
    "slide",
    "push",
    "cover",
    "reveal",
    "zoom-in",
    "zoom-out",
    "spin",
    "whip-pan",
    "circle-open",
    "circle-close",
    "iris",
    "clock-wipe",
    "radial-wipe",
    "barn-door",
    "blinds",
    "luma",
    "blur",
    "glitch",
    "pixelize",
    "flip",
    "cube",
    "page-curl",
    "film-roll",
    "stripe",
    "squash",
    "shuffle",
    "carousel",
    "light-leak",
    "morph",
];

fn transition_doc(kind: &str, extra: &str) -> sr_model::Document {
    let body = format!(
        r#"<layer id="a" asset="red" x="0" y="0" scaleX="16" scaleY="8" end="2"/>
           <layer id="b" asset="wide" x="0" y="0" scaleX="8" scaleY="8" start="2"/>
           <layer id="lm" asset="gray" x="0" y="0" scaleX="16" scaleY="8" visible="false"/>
           <transition type="{kind}" from="a" to="b" duration="1" curve="linear" {extra}/>"#
    );
    doc_with("", "", &body, "")
}

#[test]
fn every_transition_starts_at_the_outgoing_and_ends_at_the_incoming_node() {
    let Some(_) = gpu() else { return };
    for kind in TRANSITIONS {
        let d = transition_doc(kind, if kind == "luma" { r#"matte="lm""# } else { "" });
        let start = render_times(&d, &[1.5005]).unwrap();
        let end = render_times(&d, &[2.4995]).unwrap();
        assert!(problems(&start).is_empty(), "{kind}: {:?}", problems(&start));
        assert!(start.stats.fx_passes >= 1, "{kind}: no transition pass");
        // pixel (10,10): the outgoing layer is red; the incoming one is red on the left half too, so use (52,10) there
        assert_px(&start, 10, 10, [1.0, 0.0, 0.0, 1.0], 3e-2);
        assert!(close(end.at(52, 10), [0.0, 0.0, 1.0, 1.0], 4e-2), "{kind} end: {:?}", end.at(52, 10));
    }
}

#[test]
fn transitions_mid_way() {
    let Some(_) = gpu() else { return };
    let mid = render_times(&transition_doc("crossfade", ""), &[2.0]).unwrap();
    assert_px(&mid, 52, 10, [0.5, 0.0, 0.5, 1.0], 3e-2);
    let wipe = render_times(&transition_doc("wipe", r#"direction="right" softness="0""#), &[2.0]).unwrap();
    // wiping rightwards: the incoming layer covers the left half first
    assert_px(&wipe, 58, 10, [1.0, 0.0, 0.0, 1.0], 3e-2);
    let dip = render_times(&transition_doc("dip-to-color", r##"color="#00FF00""##), &[2.0]).unwrap();
    assert_px(&dip, 52, 10, [0.0, 1.0, 0.0, 1.0], 3e-2);
}

#[test]
fn custom_glsl_effect_and_transition() {
    let dir = fixtures();
    std::fs::write(dir.join("swap.glsl"), "uniform float amount; // = 1.0\nvec4 effect(vec2 uv) { vec4 c = getColor(uv); return vec4(mix(c.rgb, c.bgr, amount), c.a); }\n").unwrap();
    std::fs::write(
        dir.join("fade.glsl"),
        "vec4 transition(vec2 uv) { return mix(getFromColor(uv), getToColor(uv), progress); }\n",
    )
    .unwrap();
    let d = fx_doc(
        "",
        r#"<layer id="a" asset="red" x="0" y="0" scaleX="4" scaleY="4" effects="sh"/>"#,
        r#"<effect id="sh" type="shader" src="swap.glsl"><param name="amount" value="1"/></effect>"#,
    );
    let Some(r) = render(&d) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    assert_px(&r, 8, 8, [0.0, 0.0, 1.0, 1.0], 1e-2);
    let t = render_times(&transition_doc("shader", r#"shader="fade.glsl""#), &[2.0]).unwrap();
    assert!(problems(&t).is_empty(), "{:?}", problems(&t));
    // straight sRGB mix of red and blue at 0.5, back to linear
    let v = srgb_to_linear(0.5);
    assert_px(&t, 52, 10, [v, 0.0, v, 1.0], 2e-2);
}

#[test]
fn adjustment_layers_affect_only_what_is_below() {
    let body = r#"<layer id="a" asset="red" x="0" y="0" scaleX="8" scaleY="8"/>
        <adjustment id="adj" effects="inv" z="1"/>
        <layer id="c" asset="red" x="40" y="0" scaleX="4" scaleY="4" z="2"/>"#;
    let Some(r) = render(&fx_doc("", body, r#"<effect id="inv" type="invert"/>"#)) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    assert_px(&r, 8, 8, [0.0, 1.0, 1.0, 1.0], 1e-2);
    assert_px(&r, 44, 8, [1.0, 0.0, 0.0, 1.0], 1e-2);
    let half = r#"<layer id="a" asset="gray" x="0" y="0" scaleX="16" scaleY="8"/><adjustment id="adj" effects="exp" opacity="0.5"/>"#;
    let r2 = render(&fx_doc("", half, r#"<effect id="exp" type="exposure" exposure="1"/>"#)).unwrap();
    let g = lin8(128);
    assert_px(&r2, 8, 8, [1.5 * g, 1.5 * g, 1.5 * g, 1.0], 1e-2);
}

fn moving(project: &str, fx: &str, effects: &str) -> sr_model::Document {
    let body = format!(
        r#"<layer id="m" asset="white" x="8" y="8" scaleX="4" scaleY="4" {fx}><animate property="x"><key time="0" value="0"/><key time="1" value="40"/></animate></layer>"#
    );
    let project = format!(r##"background="#00000000" {project}"##);
    doc_with(
        &project,
        "",
        &body,
        &if effects.is_empty() { String::new() } else { format!("<effects>{effects}</effects>") },
    )
}

#[test]
fn motion_blur_smears_moving_layers_only_with_a_provider() {
    let d = moving(r#"motionBlur="true" shutterAngle="180" shutterPhase="0" motionBlurSamples="8""#, "", "");
    let Some(plain) = render_times(&d, &[0.5]) else { return };
    assert!(plain.stats.unsupported.iter().any(|m| m.contains("sub-frame provider")), "{:?}", plain.stats.unsupported);
    let blurred = render_sub(&d, 0.5).unwrap();
    assert!(problems(&blurred).is_empty(), "{:?}", problems(&blurred));
    assert!(blurred.stats.subframes >= 8);
    // x = 20 at t = 0.5; the shutter covers t .. t + 0.05 s → 2 px of travel at 40 px/s... widen: 40 px per second over 1/20 s
    let a_plain: f32 = (0..64).map(|x| plain.at(x, 16)[3]).sum();
    let a_blur: f32 = (0..64).map(|x| blurred.at(x, 16)[3]).sum();
    assert!((a_plain - a_blur).abs() < 0.6, "coverage kept: {a_plain} vs {a_blur}");
    let partial = (0..64).filter(|&x| (0.05..0.95).contains(&blurred.at(x, 16)[3])).count();
    let row: Vec<f32> = (0..64).map(|x| blurred.at(x, 16)[3]).collect();
    let prow: Vec<f32> = (0..64).map(|x| plain.at(x, 16)[3]).collect();
    assert!(partial >= 2, "edges smeared: {partial}\n{row:?}\n{prow:?} passes {}", blurred.stats.fx_passes);
    let still = doc_with(
        r##"background="#00000000" motionBlur="true" motionBlurSamples="8""##,
        "",
        r#"<layer id="s" asset="white" x="8" y="8" scaleX="4" scaleY="4"/>"#,
        "",
    );
    let s = render_sub(&still, 0.5).unwrap();
    assert_eq!(s.stats.fx_passes, 0, "adaptive sampling skips still layers");
}

#[test]
fn time_effects_use_other_frames() {
    let d = moving("", r#"effects="pt""#, r#"<effect id="pt" type="posterize-time" frequency="2"/>"#);
    let Some(r) = render_sub(&d, 0.7) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    // posterized to t = 0.5: x = 20, so the square spans 20..36
    assert_px(&r, 21, 16, [1.0, 1.0, 1.0, 1.0], 2e-2);
    assert!(r.at(38, 16)[3] < 0.05);
    let e = moving("", r#"effects="echo""#, r#"<effect id="echo" type="echo" samples="4" amount="2"/>"#);
    let r = render_sub(&e, 0.8).unwrap();
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    // x = 32 now; echoes at 24, 16, 8 (2 frames apart) are partially visible
    assert!(r.at(18, 16)[3] > 0.1 && r.at(18, 16)[3] < 0.9, "{:?}", r.at(18, 16));
    let pm = moving("", r#"effects="pmb""#, r#"<effect id="pmb" type="pixel-motion-blur" amount="1" samples="8"/>"#);
    let r = render_sub(&pm, 0.5).unwrap();
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
}

#[test]
fn colour_finishing_applies_looks_exposure_and_tone_mapping() {
    let body = r#"<layer id="a" asset="gray" x="0" y="0" scaleX="16" scaleY="8"/>"#;
    let cm = r#"<colorManagement looks="lk" exposure="1" toneMapping="reinhard"><look id="lk" slope="1" offset="0" power="1"/></colorManagement>"#;
    let Some(r) = render(&doc_with("", cm, body, "")) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    let g = 2.0 * lin8(128);
    let want = g / (1.0 + g);
    assert_px(&r, 8, 8, [want, want, want, 1.0], 1e-2);
}

#[test]
fn ocio_configs_match_opencolorio() {
    // tools/fixtures/make_ocio_expected.py: OpenColorIO's display code values for sRGB colours
    // through a working space, an exposure, looks and a display/view, for the built-in ACES CG
    // config (ACES 2.0 SDR) and a small config with a LUT file, a log shaper and a CDL look
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ocio");
    let want: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("expected.json")).unwrap()).unwrap();
    let colors = ["#000000", "#FFFFFF", "#808080", "#FF0000", "#00FF00", "#0000FF", "#FFC080", "#204060", "#C8102E"];
    let body: String = colors
        .iter()
        .enumerate()
        .map(|(k, c)| {
            format!(r#"<shape id="p{k}" shape="rect" x="{}" y="0" width="7" height="32" fill="{c}"/>"#, k * 7)
        })
        .collect();
    for (name, case) in want.as_object().unwrap() {
        let s = |k: &str| case[k].as_str().unwrap().to_string();
        let config =
            if s("config").starts_with("ocio://") { s("config") } else { dir.join(s("config")).display().to_string() };
        let working = s("working");
        // a <look> with neither a LUT nor CDL values names a look of the OCIO config
        let (looks, look_el) = if s("looks").is_empty() {
            (String::new(), String::new())
        } else {
            (format!(r#" looks="{}""#, s("looks")), format!(r#"<look id="{}"/>"#, s("looks")))
        };
        for ev in [0.0, 3.0, -2.0] {
            let cm = format!(
                r#"<colorManagement ocioConfig="{config}" workingSpace="{working}" display="{}" view="{}" exposure="{ev}"{looks}>{look_el}</colorManagement>"#,
                s("display"),
                s("view")
            );
            let Some(r) = render(&doc_with("", &cm, &body, "")) else { return };
            if sr_gpu::ocio::tool().is_none() {
                assert!(r.stats.unsupported.iter().any(|m| m.contains("ociobakelut")), "{:?}", r.stats.unsupported);
                return;
            }
            assert!(r.stats.unsupported.is_empty(), "{name}: {:?}", r.stats.unsupported);
            let ws = sr_model::model::ColorSpace::ALL.iter().copied().find(|c| c.as_str() == working).unwrap();
            let m = sr_gpu::color::convert(ws, sr_model::model::ColorSpace::Srgb);
            for (k, c) in colors.iter().enumerate() {
                let px = r.at(k as u32 * 7 + 3, 16);
                let lin: [f64; 3] = std::array::from_fn(|i| (0..3).map(|j| m[i][j] * px[j] as f64).sum());
                let got = lin.map(|v| sr_gpu::color::encode(sr_model::model::Transfer::Srgb, v));
                let e = &case["values"][format!("{c} {ev}")];
                for i in 0..3 {
                    let e = e[i].as_f64().unwrap();
                    // the 129³ table's trilinear error peaks near 0.011 at the gamut boundary
                    // (measured against OCIO's analytic CPU path), plus half-float storage
                    assert!(
                        (got[i] - e).abs() < 0.015,
                        "{name} {c} at {ev} EV: {got:?} vs OpenColorIO {e} (channel {i})"
                    );
                }
            }
        }
    }
}

#[test]
fn every_effect_type_draws() {
    const KINDS: [&str; 80] = [
        "glow",
        "bloom",
        "blur",
        "color-grade",
        "vignette",
        "lens-flare",
        "drop-shadow",
        "lighting",
        "directional-blur",
        "radial-blur",
        "zoom-blur",
        "lens-blur",
        "pixel-motion-blur",
        "tilt-shift",
        "lift-gamma-gain",
        "cdl",
        "lut",
        "curves",
        "levels",
        "white-balance",
        "exposure",
        "hue-saturation",
        "tonemap",
        "tint",
        "tritone",
        "gradient-map",
        "grayscale",
        "sepia",
        "invert",
        "posterize",
        "threshold",
        "color-overlay",
        "gradient-overlay",
        "selective-color",
        "film-grain",
        "noise",
        "chromatic-aberration",
        "sharpen",
        "unsharp-mask",
        "halation",
        "light-leak",
        "light-sweep",
        "glitch",
        "rgb-split",
        "scanlines",
        "vhs",
        "halftone",
        "pixelate",
        "mosaic",
        "emboss",
        "bevel",
        "inner-shadow",
        "inner-glow",
        "long-shadow",
        "stroke",
        "outline",
        "echo",
        "posterize-time",
        "letterbox",
        "mirror",
        "kaleidoscope",
        "tile",
        "displacement-map",
        "turbulent-displace",
        "wave-warp",
        "ripple",
        "twirl",
        "spherize",
        "bulge",
        "lens-distortion",
        "heat-haze",
        "chroma-key",
        "luma-key",
        "difference-key",
        "spill-suppress",
        "matte-choke",
        "fill",
        "fractal-noise",
        "god-rays",
        "shader",
    ];
    let dir = fixtures();
    std::fs::write(dir.join("id.cube"), "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n")
        .unwrap();
    std::fs::write(dir.join("pass.glsl"), "vec4 effect(vec2 uv) { return getColor(uv); }\n").unwrap();
    let Some(_) = gpu() else { return };
    for kind in KINDS {
        let extra = match kind {
            "lut" => r#" src="id.cube""#,
            "shader" => r#" src="pass.glsl""#,
            "curves" => r#" curve="0,0 0.5,0.6 1,1""#,
            "gradient-map" | "gradient-overlay" => r#" paint="url(#grad)""#,
            "displacement-map" | "difference-key" => r#" source="bg""#,
            "lighting" => r#" lights="pl""#,
            _ => "",
        };
        let body = r#"<layer id="bg" asset="gray" x="0" y="0" scaleX="16" scaleY="8"/>
            <layer id="a" asset="quad" x="16" y="4" scaleX="12" scaleY="12" effects="e"><animate property="x"><key time="0" value="16"/><key time="4" value="20"/></animate></layer>"#;
        let post = format!(
            r#"<lights><light id="pl" type="point" x="20" y="10"/></lights><effects><effect id="e" type="{kind}"{extra}/></effects>"#
        );
        let pre = r##"<paints><linearGradient id="grad"><stop offset="0" color="#000000"/><stop offset="1" color="#FF8800"/></linearGradient></paints>"##;
        let xml = format!(
            r#"<scene version="1.1"><project width="64" height="32" fps="10" duration="4"/>{ASSETS}{pre}<composition>{body}</composition>{post}</scene>"#
        );
        let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
        let d = sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{kind}: {e:?}"));
        let r = render_sub(&d, 1.0).unwrap();
        let bad: Vec<&String> = r
            .stats
            .unsupported
            .iter()
            .chain(&r.stats.errors)
            .filter(|m| !m.contains("lighting") || !m.contains("light"))
            .collect();
        assert!(bad.is_empty(), "{kind}: {bad:?}");
        assert!(r.stats.fx_passes >= 1, "{kind}: no passes");
        assert!(r.px.iter().all(|p| p.iter().all(|v| v.is_finite())), "{kind}: non-finite pixels");
    }
}

#[test]
fn texture_pool_stays_bounded_over_a_long_motion_blurred_render() {
    // a sized layer that moves and scales: its motion-blur bounds, and so its temporary texture size, change
    // every frame. Pooled textures of sizes no longer requested must be freed, or a long render runs out of
    // GPU memory.
    let body = r#"<layer id="m" asset="white" x="8" y="8"><animate property="x"><key time="0" value="0"/><key time="4" value="90"/></animate>
        <animate property="scaleX"><key time="0" value="2"/><key time="4" value="9"/></animate>
        <animate property="scaleY"><key time="0" value="2"/><key time="4" value="7"/></animate></layer>"#;
    let d = doc_with(r##"background="#00000000" motionBlur="true" motionBlurSamples="4""##, "", body, "");
    let Some(gpu) = gpu() else { return };
    let ev = sr_eval::Evaluator::new(&d, &sr_eval::EvalOptions::default()).unwrap();
    let mut r = sr_gpu::Renderer::new(gpu, ev.program());
    let mut held = Vec::new();
    for k in 0..40 {
        let g = ev.evaluate(k as f64 * 0.1);
        let mut sub = |st: f64| ev.evaluate(st);
        let _ = r.render_with(&g, ev.program(), Some(&mut sub));
        held.push(r.pooled_textures());
    }
    let max = *held.iter().max().unwrap();
    assert!(max <= 12, "pooled textures stay bounded: {held:?}");
}
