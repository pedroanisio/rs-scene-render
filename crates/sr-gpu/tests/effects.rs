//! Effects, transitions, adjustment layers, motion blur and colour finishing on the GPU.

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
        r#"<effect id="b" type="blur" radius="1"/>"#,
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

/// Standard normal cumulative distribution (Abramowitz–Stegun 7.1.26 through erf).
fn phi(x: f64) -> f64 {
    let z = x.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.3275911 * z);
    let y = 1.0
        - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592)
            * t
            * (-z * z).exp();
    0.5 * (1.0 + if x < 0.0 { -y } else { y })
}

#[test]
fn blur_samples_outside_its_input_are_transparent() {
    // An adjustment layer's blur reads transparency outside the frame, and the
    // layer replaces the backdrop by its effect within its coverage, so a blurred opaque
    // frame loses half its alpha at the frame's edge (transparent background).
    let fx = r##"<effect id="b" type="blur" radius="3"/>"##;
    let body = r#"<layer id="a" asset="white" scaleX="16" scaleY="8"/><adjustment id="adj" effects="b"/>"#;
    let Some(r) = render(&fx_doc(r##"background="#00000000""##, body, fx)) else { return };
    let (edge, inside) = (r.at(0, 16)[3], r.at(32, 16)[3]);
    assert!((inside - 1.0).abs() < 1e-2, "{inside}");
    let want = phi(0.5 / 3.0) as f32;
    assert!((edge - want).abs() < 0.03, "edge alpha {edge}, want {want}");
}

#[test]
fn blur_radius_is_the_standard_deviation() {
    // `radius` is σ; a drop shadow's `radius` is 2σ. Radii are in the node's units,
    // so on a layer scaled 10× these are σ = 3 px. A straight edge at x = 32 blurred with σ covers Φ(d / σ)
    // at signed distance d inside it.
    let fx = r##"<effect id="b" type="blur" radius="0.3"/>
        <effect id="s" type="drop-shadow" radius="0.6" offsetX="0" offsetY="0" color="#000000"/>"##;
    let edge = |e: &str| {
        let body = format!(r#"<layer id="a" asset="white" x="32" y="-4" scaleX="10" scaleY="10" effects="{e}"/>"#);
        render(&fx_doc(r##"background="#00000000""##, &body, fx))
            .inspect(|r| assert!(problems(r).is_empty(), "{:?}", problems(r)))
    };
    let Some(b) = edge("b") else { return };
    for x in [27u32, 29, 31, 33, 35, 37] {
        let want = phi((x as f64 + 0.5 - 32.0) / 3.0) as f32;
        let got = b.at(x, 16)[3];
        assert!((got - want).abs() < 0.05, "blur alpha at x = {x}: {got}, want {want}");
    }
    let s = edge("s").unwrap();
    for x in [27u32, 29, 31] {
        let want = phi((x as f64 + 0.5 - 32.0) / 3.0) as f32;
        let got = s.at(x, 16)[3];
        assert!((got - want).abs() < 0.05, "shadow alpha at x = {x}: {got}, want {want}");
        assert!(s.at(x, 16)[0] < 1e-3, "an absent shadow colour is opaque black: {:?}", s.at(x, 16));
    }
}

#[test]
fn glow_adds_a_bloom_over_the_content() {
    // the pixels above `threshold`, blurred, are added over the content
    let body = r##"<layer id="a" asset="src" x="16" y="8" scaleX="4" scaleY="4" effects="g"/>"##;
    let fx = r##"<effect id="g" type="glow" radius="0.5" threshold="0" intensity="1"/>"##;
    let Some(r) = render(&fx_doc(r##"background="#00000000""##, body, fx)) else { return };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    // the middle of the square is the content plus its own (nearly unblurred) bloom: twice the content
    assert_px(&r, 24, 16, [2.0 * lin8(200), 2.0 * lin8(100), 2.0 * lin8(50), 1.0], 2e-2);
    assert!(r.at(14, 16)[3] > 0.1, "the halo spreads outside: {:?}", r.at(14, 16));
}

#[test]
fn vignette_follows_d9() {
    // Vignette: 1 − amount · smoothstep(r₀, r₀ + softness, r), r over the half diagonal.
    let body = r#"<layer id="a" asset="gray" x="0" y="0" scaleX="16" scaleY="8"/><adjustment id="v" effects="e"/>"#;
    let half = 0.5 * (64f64 * 64.0 + 32.0 * 32.0).sqrt();
    let k = |amount: f64, r0: f64, soft: f64, x: u32, y: u32| {
        let r = ((x as f64 + 0.5 - 32.0).hypot(y as f64 + 0.5 - 16.0)) / half;
        let u = ((r - r0) / soft).clamp(0.0, 1.0);
        (1.0 - amount * u * u * (3.0 - 2.0 * u)) as f32
    };
    let g = lin8(128);
    // the second takes the schema's defaults: amount 1, radius 4, softness 0.1
    for (attrs, amount, r0, soft) in
        [(r#"amount="0.8" radius="10" softness="0.3""#, 0.8, 10.0 / half, 0.3), ("", 1.0, 4.0 / half, 0.1)]
    {
        let fx = format!(r#"<effect id="e" type="vignette" {attrs}/>"#);
        let Some(r) = render(&fx_doc("", body, &fx)) else { return };
        assert!(problems(&r).is_empty(), "{:?}", problems(&r));
        for (x, y) in [(32u32, 16u32), (40, 16), (50, 20), (0, 0), (63, 31)] {
            let v = g * k(amount, r0, soft, x, y);
            assert_px(&r, x, y, [v, v, v, 1.0], 1e-2);
        }
    }
}

#[test]
fn chromatic_aberration_shifts_amount_pixels_at_the_farthest_corner() {
    // Red magnifies by 1 − l and blue by 1 + l about the centre, l = amount / reach
    // (35.8 px here); samples beyond the frame are transparent
    let Some(r) = render(&fx_doc(
        r##"background="#00000000""##,
        r#"<layer id="w" asset="white" scaleX="16" scaleY="8"/><adjustment id="a" effects="c"/>"#,
        r#"<effect id="c" type="chromatic-aberration" amount="4"/>"#,
    )) else {
        return;
    };
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    // the left edge: red reads 3.5 px beyond the frame, blue and green inside
    let e = r.at(0, 16);
    assert!(e[0] < 0.02 && e[1] > 0.98 && e[2] > 0.98, "{e:?}");
    // 4 px in, red reads inside again
    assert!(r.at(4, 16)[0] > 0.98, "{:?}", r.at(4, 16));
    assert_px(&r, 32, 16, [1.0; 4], 1e-3);
}

#[test]
fn chromatic_aberration_edge_modes_preserve_opaque_frame_edges() {
    for mode in [1, 2] {
        let Some(r) = render(&fx_doc(
            r##"background="#00000000""##,
            r#"<layer id="w" asset="white" scaleX="16" scaleY="8"/><adjustment id="a" effects="c"/>"#,
            &format!(
                r#"<effect id="c" type="chromatic-aberration" amount="4"><param name="edgeMode" value="{mode}"/></effect>"#
            ),
        )) else {
            return;
        };
        assert!(problems(&r).is_empty(), "{:?}", problems(&r));
        for (x, y) in [(0, 0), (63, 0), (0, 31), (63, 31), (0, 16), (32, 0)] {
            assert_px(&r, x, y, [1.0; 4], 0.01);
        }
    }
}

#[test]
fn chromatic_aberration_mirror_reflects_instead_of_clamping() {
    let mut edges = Vec::new();
    for mode in [1, 2] {
        let Some(r) = render(&fx_doc(
            r##"background="#00000000""##,
            r##"<shape id="white-bg" shape="rect" width="64" height="32" fill="#FFFFFF"/>
<shape id="black" shape="rect" width="2" height="32" fill="#000000"/>
<adjustment id="a" effects="c"/>"##,
            &format!(
                r#"<effect id="c" type="chromatic-aberration" amount="8"><param name="edgeMode" value="{mode}"/></effect>"#
            ),
        )) else {
            return;
        };
        edges.push(r.at(0, 16)[0]);
    }
    assert!(edges[0] < 0.01 && edges[1] > 0.99, "clamped and reflected red: {edges:?}");
}

#[test]
fn warp_edge_modes_preserve_full_frame_colour() {
    for (kind, attrs) in [
        ("rgb-split", r#"offsetX="128" offsetY="64""#),
        ("wave-warp", r#"amount="16" size="32""#),
        ("lens-distortion", r#"amount="4""#),
    ] {
        for mode in [1, 2] {
            let Some(r) = render(&fx_doc(
                r##"background="#00000000""##,
                r#"<layer id="w" asset="white" scaleX="16" scaleY="8"/><adjustment id="a" effects="e"/>"#,
                &format!(r#"<effect id="e" type="{kind}" {attrs}><param name="edgeMode" value="{mode}"/></effect>"#),
            )) else {
                return;
            };
            assert!(problems(&r).is_empty(), "{kind}: {:?}", problems(&r));
            for (x, y) in [(0, 0), (63, 0), (0, 31), (63, 31), (16, 0)] {
                assert_px(&r, x, y, [1.0; 4], 0.01);
            }
        }
    }
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

/// Transition types without a fixed geometric definition, their <param> children and the motion
/// blur of moving types (180° shutter by default, off with `motionBlur="false"`).
#[test]
fn undefined_transition_types_follow_the_python_renderer() {
    let Some(_) = gpu() else { return };
    let render = |kind: &str, extra: &str, inner: &str| {
        let body = format!(
            r#"<layer id="a" asset="red" x="0" y="0" scaleX="16" scaleY="8" end="2"/>
               <layer id="b" asset="wide" x="0" y="0" scaleX="8" scaleY="8" start="2"/>
               <transition type="{kind}" from="a" to="b" duration="1" curve="linear" {extra}>{inner}</transition>"#
        );
        render_times(&doc_with("", "", &body, ""), &[2.0]).unwrap()
    };
    let (red, blue) = ([1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]);
    // blinds: <param name="count"> slats across the travel, each wiping along it (b where the
    // slat coordinate is below p = ½)
    let r = render("blinds", r#"direction="down" softness="0""#, r#"<param name="count" value="2"/>"#);
    assert!(close(r.at(52, 4), blue, 3e-2) && close(r.at(52, 20), blue, 3e-2), "{:?}", r.at(52, 4));
    assert!(close(r.at(52, 12), red, 3e-2) && close(r.at(52, 28), red, 3e-2));
    // push smears along its motion by default (180° shutter), and not with motionBlur="false"
    let sharp = render("push", r#"motionBlur="false""#, "");
    let smeared = render("push", "", "");
    let diff = (0..64).map(|x| (0..4).map(|c| (sharp.at(x, 10)[c] - smeared.at(x, 10)[c]).abs()).fold(0.0, f32::max));
    assert!(diff.fold(0.0, f32::max) > 0.1, "the blur changes the seams");
    assert_eq!(render("push", r#"motionBlur="false""#, "").px, sharp.px);
    // glitch is deterministic
    let g = render("glitch", "", "");
    assert_eq!(g.px, render("glitch", "", "").px);
}

/// Geometric transitions, in frame pixels, at p = ½ with no softness (b is
/// blue at x ≥ 32, a is red; the frame is 64 × 32 with its centre at (32, 16)).
#[test]
fn transitions_follow_d19_coordinates() {
    let Some(_) = gpu() else { return };
    let (red, blue) = ([1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]);
    let at = |kind: &str, extra: &str, x: u32, y: u32| {
        let r = render_times(&transition_doc(kind, &format!(r#"softness="0" {extra}"#)), &[2.0]).unwrap();
        r.at(x, y)
    };
    // circle-open (and iris): b within half the half-diagonal (35.8 px) of the centre
    for kind in ["circle-open", "iris"] {
        assert!(close(at(kind, "", 46, 16), blue, 3e-2), "{kind} inside");
        assert!(close(at(kind, "", 52, 16), red, 3e-2), "{kind} outside");
    }
    // circle-close: a stays inside the closing circle
    assert!(close(at("circle-close", "", 46, 16), red, 3e-2));
    assert!(close(at("circle-close", "", 52, 16), blue, 3e-2));
    // clock-wipe: clockwise from 12 o'clock, so the right half first
    assert!(close(at("clock-wipe", "", 52, 20), blue, 3e-2));
    // radial-wipe from @angle = 90 (6 o'clock), clockwise: the left half first
    assert!(close(at("radial-wipe", r#"angle="90""#, 52, 20), red, 3e-2));
    // barn-door along the travel: half of the span around the centre line
    assert!(close(at("barn-door", "", 46, 10), blue, 3e-2));
    assert!(close(at("barn-door", "", 52, 10), red, 3e-2));
    // additive-dissolve: both at full strength, clamped at 1
    assert!(close(at("additive-dissolve", "", 52, 10), [1.0, 0.0, 1.0, 1.0], 3e-2));
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

#[test]
fn motion_blurred_shapes_sample_only_the_box_they_sweep() {
    // each blurred node holds its samples until the frame is submitted: sized to the whole frame, a few hundred
    // moving dots needed tens of GB of video memory. Rects and ellipses sample the box they sweep plus their
    // stroke, and keep every pixel a full-frame sample had.
    let mb = r##"background="#00000000" shutterAngle="180" shutterPhase="0" motionBlurSamples="8""##;
    let mv = r#"<animate property="x"><key time="0" value="0"/><key time="4" value="200"/></animate>"#;
    let paint = r##"fill="#FFFFFF" stroke="#FF0000" strokeWidth="4" strokePosition="outside" motionBlur="on""##;
    let rect = format!(r#"<shape id="r" shape="rect" y="10" width="10" height="8" {paint}>{mv}</shape>"#);
    // the same outline as a path, which samples the whole frame
    let path = format!(r#"<shape id="r" shape="path" path="M0 0 H10 V8 H0 Z" y="10" width="10" height="8" {paint}>{mv}</shape>"#);
    let Some(a) = render_sub(&doc_with(mb, "", &rect, ""), 0.1) else { return };
    let b = render_sub(&doc_with(mb, "", &path, ""), 0.1).unwrap();
    assert!(problems(&a).is_empty() && problems(&b).is_empty(), "{:?} {:?}", problems(&a), problems(&b));
    assert!(a.stats.fx_passes > 0, "the rect is blurred");
    let worst = (0..32).flat_map(|y| (0..64).map(move |x| (x, y))).map(|(x, y)| {
        let (p, q) = (a.at(x, y), b.at(x, y));
        (0..4).map(|c| (p[c] - q[c]).abs()).fold(0.0f32, f32::max)
    });
    let worst = worst.fold(0.0f32, f32::max);
    assert!(worst < 0.02, "box-sized samples match full-frame ones: {worst}");
    assert!(a.at(12, 7)[0] > 0.5 && a.at(12, 7)[1] < 0.2, "the outside stroke is kept: {:?}", a.at(12, 7));
    let dots: String = (0..30)
        .map(|k| {
            format!(r##"<shape id="d{k}" shape="ellipse" y="{}" width="2" height="2" fill="#FFFFFF" motionBlur="on"><animate property="x"><key time="0" value="{}"/><key time="4" value="{}"/></animate></shape>"##,
                k % 28, k * 2, k * 2 + 200)
        })
        .collect();
    // a frame large enough that a dot's target (rounded up to 32 px steps) is far smaller than the frame
    let xml = format!(
        r#"<scene version="1.1"><project width="512" height="256" fps="10" duration="4" {mb}/>{ASSETS}<composition>{dots}</composition></scene>"#
    );
    let big = sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) }).unwrap();
    let r = render_sub(&big, 0.1).unwrap();
    assert!(problems(&r).is_empty(), "{:?}", problems(&r));
    let texels = r.renderer.pooled_texels();
    assert!(texels < 30 * 512 * 256 / 8, "blur textures follow the dots, not the frame: {texels} texels");
}

#[test]
fn agx_keeps_white_and_grey_neutral() {
    // AgX's inset/outset matrices are defined for linear Rec.709 and must be given to WGSL column by column:
    // transposed, they turned white pink. Greys stay grey through AgX, in any working space.
    for ws in ["linear-srgb", "acescg"] {
        let body = r##"<shape id="w" shape="rect" x="0" y="0" width="32" height="32" fill="#FFFFFF"/><shape id="g" shape="rect" x="32" y="0" width="32" height="32" fill="#808080"/>"##;
        let cm = format!(r#"<colorManagement workingSpace="{ws}" toneMapping="agx"/>"#);
        let Some(r) = render(&doc_with(&format!(r#"workingColorSpace="{ws}""#), &cm, body, "")) else { return };
        assert!(problems(&r).is_empty(), "{:?}", problems(&r));
        for (x, what) in [(16, "white"), (48, "grey")] {
            let p = r.at(x, 16);
            let spread = p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]);
            assert!(spread < 0.01, "{ws}: {what} through AgX is not neutral: {p:?}");
        }
    }
}
