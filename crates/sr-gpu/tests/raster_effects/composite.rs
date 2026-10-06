use super::common;
use common::*;

#[test]
fn background_and_image_placement() {
    let d = doc(r##"background="#0000FF""##, "", r#"<layer id="a" asset="red" x="8" y="4" scaleX="2" scaleY="2"/>"#);
    let Some(r) = render(&d) else { return };
    assert_px(&r, 0, 0, [0.0, 0.0, 1.0, 1.0], 1e-3);
    assert_px(&r, 10, 6, [1.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 15, 11, [1.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 17, 6, [0.0, 0.0, 1.0, 1.0], 1e-3);
    assert_eq!(r.stats.draws, 2);
}

#[test]
fn solid_background_fast_path_matches_a_constant_gradient_exactly() {
    for color in ["#334455", "#33445580"] {
        let body = r#"<layer id="a" asset="half" x="8" y="4" scaleX="2" scaleY="2" opacity="0.7"/>"#;
        let solid = doc(&format!(r#"background="{color}""#), "", body);
        let gradient = doc(
            r#"background="url(#flat)""#,
            &format!(
                r#"<paints><linearGradient id="flat" x1="2" x2="3" dither="false"><stop offset="0" color="{color}"/><stop offset="1" color="{color}"/></linearGradient></paints>"#
            ),
            body,
        );
        let (Some(solid), Some(general)) = (render(&solid), render(&gradient)) else { return };
        assert_eq!(solid.px, general.px, "background {color}");
    }
}

#[test]
fn opacity_rotation_and_anchor() {
    let d = doc(
        r##"background="#0000FF""##,
        "",
        r#"<layer id="a" asset="red" x="4" y="4" opacity="0.5"/>
           <layer id="q" asset="quad" x="40" y="16" anchorX="1" anchorY="1" scaleX="8" scaleY="8" rotation="90"/>"#,
    );
    let Some(r) = render(&d) else { return };
    assert_px(&r, 5, 5, [0.5, 0.0, 0.5, 1.0], 2e-3);
    // quad.png: red TL, green TR, blue BL, white BR; rotated 90° clockwise → blue TL, red TR, white BL, green BR
    assert_px(&r, 34, 10, [0.0, 0.0, 1.0, 1.0], 2e-3);
    assert_px(&r, 45, 10, [1.0, 0.0, 0.0, 1.0], 2e-3);
    assert_px(&r, 34, 21, [1.0, 1.0, 1.0, 1.0], 2e-3);
    assert_px(&r, 45, 21, [0.0, 1.0, 0.0, 1.0], 2e-3);
}

/// CPU reference for B(Cb, Cs) with opaque source and backdrop (W3C
/// Compositing and Blending Level 1 plus the After Effects modes).
fn reference(mode: &str, b: [f32; 3], s: [f32; 3]) -> [f32; 4] {
    // W3C luminance for the non-separable modes and darker/lighter colour; Rec. 709 for the stencils
    let lum = |c: [f32; 3]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
    let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let clip = |c: [f32; 3]| {
        let l = lum(c);
        let n = c.iter().cloned().fold(f32::MAX, f32::min);
        let x = c.iter().cloned().fold(f32::MIN, f32::max);
        let mut c = c;
        if n < 0.0 {
            c = c.map(|v| l + (v - l) * l / (l - n));
        }
        if x > 1.0 {
            c = c.map(|v| l + (v - l) * (1.0 - l) / (x - l));
        }
        c
    };
    let set_lum = |c: [f32; 3], l: f32| {
        let d = l - lum(c);
        clip(c.map(|v| v + d))
    };
    let sat = |c: [f32; 3]| c.iter().cloned().fold(f32::MIN, f32::max) - c.iter().cloned().fold(f32::MAX, f32::min);
    let set_sat = |c: [f32; 3], s: f32| {
        let mx = c.iter().cloned().fold(f32::MIN, f32::max);
        let mn = c.iter().cloned().fold(f32::MAX, f32::min);
        if mx > mn {
            c.map(|v| (v - mn) * s / (mx - mn))
        } else {
            [0.0; 3]
        }
    };
    let sep = |f: &dyn Fn(f32, f32) -> f32| [f(b[0], s[0]), f(b[1], s[1]), f(b[2], s[2]), 1.0];
    let overlay = |b: f32, s: f32| if b <= 0.5 { 2.0 * b * s } else { 1.0 - 2.0 * (1.0 - b) * (1.0 - s) };
    let dodge = |b: f32, s: f32| {
        if b == 0.0 {
            0.0
        } else if s >= 1.0 {
            1.0
        } else {
            (b / (1.0 - s)).min(1.0)
        }
    };
    let burn = |b: f32, s: f32| {
        if b >= 1.0 {
            1.0
        } else if s <= 0.0 {
            0.0
        } else {
            1.0 - ((1.0 - b) / s).min(1.0)
        }
    };
    let rgb1 = |c: [f32; 3]| [c[0], c[1], c[2], 1.0];
    match mode {
        "normal" | "dissolve" | "alpha-add" => rgb1(s),
        "add" | "linear-dodge" => sep(&|b, s| b + s),
        "plus-lighter" => sep(&|b, s| (b + s).min(1.0)),
        "multiply" => sep(&|b, s| b * s),
        "screen" => sep(&|b, s| b + s - b * s),
        "overlay" => sep(&overlay),
        "difference" => sep(&|b, s| (b - s).abs()),
        "exclusion" => sep(&|b, s| b + s - 2.0 * b * s),
        "subtract" => sep(&|b, s| (b - s).max(0.0)),
        "divide" => sep(&|b, s| if s <= 0.0 { (b > 0.0) as u8 as f32 } else { (b / s).min(1.0) }),
        "darken" => sep(&f32::min),
        "lighten" => sep(&f32::max),
        "darker-color" => rgb1(if lum(s) < lum(b) { s } else { b }),
        "lighter-color" => rgb1(if lum(s) > lum(b) { s } else { b }),
        "color-dodge" => sep(&dodge),
        "color-burn" => sep(&burn),
        "linear-burn" => sep(&|b, s| (b + s - 1.0).max(0.0)),
        "soft-light" => sep(&|b, s| {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }),
        "hard-light" => sep(&|b, s| overlay(s, b)),
        "linear-light" => sep(&|b, s| (b + 2.0 * s - 1.0).clamp(0.0, 1.0)),
        "vivid-light" => sep(&|b, s| if s <= 0.5 { burn(b, 2.0 * s) } else { dodge(b, 2.0 * (s - 0.5)) }),
        "pin-light" => sep(&|b, s| if s <= 0.5 { b.min(2.0 * s) } else { b.max(2.0 * s - 1.0) }),
        "hard-mix" => sep(&|b, s| if b + s >= 1.0 { 1.0 } else { 0.0 }),
        "hue" => rgb1(set_lum(set_sat(s, sat(b)), lum(b))),
        "saturation" => rgb1(set_lum(set_sat(b, sat(s)), lum(b))),
        "color" => rgb1(set_lum(s, lum(b))),
        "luminosity" => rgb1(set_lum(b, lum(s))),
        "stencil-alpha" | "behind" => rgb1(b),
        "stencil-luma" => rgb1(b.map(|v| v * luma(s)))
            .map(|v| v)
            .into_iter()
            .enumerate()
            .map(|(i, v)| if i == 3 { luma(s) } else { v })
            .collect::<Vec<_>>()
            .try_into()
            .unwrap(),
        "silhouette-alpha" => [0.0; 4],
        "silhouette-luma" => {
            let k = 1.0 - luma(s);
            [b[0] * k, b[1] * k, b[2] * k, k]
        }
        other => panic!("no reference for {other}"),
    }
}

#[test]
fn all_blend_modes_match_the_reference_formulas() {
    let all: Vec<&str> = sr_model::model::Blend::ALL.iter().map(|b| b.as_str()).collect();
    assert_eq!(all.len(), 35);
    // the stencil modes clear the whole frame outside their layer, so they get their own frames
    let modes: Vec<&str> = all.iter().copied().filter(|m| !m.starts_with("stencil")).collect();
    // one 4×4 layer per mode, in a row, over a coloured backdrop layer (the project background goes
    // beneath everything last, so the modes do not see it)
    let body: String = std::iter::once(BACKDROP.replace("HEIGHT", "12"))
        .chain(modes.iter().enumerate().map(|(i, m)| {
            format!(r#"<layer id="l{i}" asset="src" x="{}" y="{}" blend="{m}"/>"#, (i % 16) * 4, (i / 16) * 4)
        }))
        .collect();
    let d = doc(r##"width="64" height="12" background="#00000000""##, "", &body);
    let Some(r) = render(&d) else { return };
    let b = [lin8(0x40), lin8(0x80), lin8(0xC0)];
    let s = [lin8(200), lin8(100), lin8(50)];
    let mut failures = Vec::new();
    for (i, m) in modes.iter().enumerate() {
        let got = r.at((i as u32 % 16) * 4 + 1, (i as u32 / 16) * 4 + 1);
        let want = reference(m, b, s);
        let tol: Vec<f32> = want.iter().map(|w| 3e-3 * w.abs().max(1.0)).collect();
        if !(0..4).all(|k| (got[k] - want[k]).abs() <= tol[k]) {
            failures.push(format!("{m}: got {got:?}, want {want:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // every mode reads a copy of the backdrop except add and linear-dodge, which the blender does
    assert!(r.stats.backdrop_copies >= 29, "{:?}", r.stats);
    for m in ["stencil-alpha", "stencil-luma"] {
        let body = format!(r#"{}<layer id="s" asset="src" blend="{m}"/>"#, BACKDROP.replace("HEIGHT", "32"));
        let d = doc(r##"background="#00000000""##, "", &body);
        let r = render(&d).unwrap();
        assert_px(&r, 1, 1, reference(m, b, s), 3e-3);
        assert_px(&r, 20, 20, [0.0; 4], 1e-4);
    }
}

/// A 64-wide backdrop rectangle of #4080C0 (replace HEIGHT).
const BACKDROP: &str = r##"<shape id="bd" shape="rect" x="0" y="0" width="64" height="HEIGHT" fill="#4080C0"/>"##;

#[test]
fn stencil_alpha_cuts_the_backdrop_outside_the_layer() {
    let d = doc(
        r##"background="#00000000""##,
        "",
        r##"<shape id="w" shape="rect" x="0" y="0" width="64" height="32" fill="#FFFFFF"/>
            <layer id="s" asset="red" x="8" y="8" scaleX="2" scaleY="2" blend="stencil-alpha"/>"##,
    );
    let Some(r) = render(&d) else { return };
    assert_px(&r, 10, 10, [1.0; 4], 1e-3);
    assert_px(&r, 40, 20, [0.0; 4], 1e-3);
}

#[test]
fn the_project_background_goes_beneath_everything_last() {
    // The layers composite on transparency; behind, subtract and stencils do not see
    // the background, which fills whatever the layers leave uncovered.
    let d = doc(
        r##"background="#0000FF""##,
        "",
        r#"<layer id="w" asset="white" x="0" y="0" scaleX="8" scaleY="8"/>
           <layer id="s" asset="red" x="8" y="8" scaleX="2" scaleY="2" blend="stencil-alpha"/>
           <layer id="b" asset="red" x="40" y="8" scaleX="2" scaleY="2" blend="behind"/>
           <layer id="u" asset="white" x="56" y="0" scaleX="2" scaleY="2" blend="subtract"/>"#,
    );
    let Some(r) = render(&d) else { return };
    // the stencil keeps the white layer inside itself and cuts it outside, down to the background
    assert_px(&r, 10, 10, [1.0; 4], 1e-3);
    assert_px(&r, 4, 4, [0.0, 0.0, 1.0, 1.0], 1e-3);
    // behind over transparency shows the layer, not the background
    assert_px(&r, 42, 10, [1.0, 0.0, 0.0, 1.0], 1e-3);
    // subtract over transparency leaves white; the blue background does not darken
    assert_px(&r, 58, 2, [1.0; 4], 1e-3);
    assert_px(&r, 30, 30, [0.0, 0.0, 1.0, 1.0], 1e-3);
}

#[test]
fn non_separable_modes_use_w3c_luminance() {
    // Lum = 0.3 R + 0.59 G + 0.11 B. `color` of pure green over grey l: SetLum((0, 1, 0), l) = (0, l / 0.59, 0).
    let d = doc(
        "",
        "",
        r#"<layer id="g" asset="gray" scaleX="16" scaleY="8"/><layer id="c" asset="solid" x="8" y="8" blend="color"/>"#,
    );
    let Some(r) = render(&d) else { return };
    let l = lin8(128);
    assert_px(&r, 9, 9, [0.0, l / 0.59, 0.0, 1.0], 3e-3);
}

#[test]
fn divide_takes_clamped_inputs() {
    // divide is cb / cs on inputs in [0, 1], and 1 where cs is 0 and cb is not.
    let d = doc(
        "",
        "",
        r#"<layer id="g" asset="gray" scaleX="16" scaleY="8"/>
           <layer id="v" asset="solid" x="8" y="8" blend="divide"/>
           <layer id="h" asset="half" x="24" y="8" blend="divide"/>"#,
    );
    let Some(r) = render(&d) else { return };
    let l = lin8(128);
    assert_px(&r, 9, 9, [1.0, l, 1.0, 1.0], 3e-3);
    // half-covered white: (1 − αs)·Cb + αs·Cb / 1
    assert_px(&r, 25, 9, [l, l, l, 1.0], 3e-3);
}

#[test]
fn partial_alpha_uses_the_general_compositing_formula() {
    // half.png: white at alpha 128/255 over an opaque backdrop layer with multiply:
    // co = (1 − αs)·Cb + αs·B(Cb, Cs)
    let body = format!(
        r#"{}<layer id="a" asset="half" blend="multiply"/><layer id="b" asset="half" x="8" blend="screen"/>"#,
        BACKDROP.replace("HEIGHT", "32")
    );
    let d = doc("", "", &body);
    let Some(r) = render(&d) else { return };
    let a = 128.0 / 255.0;
    let b = [lin8(0x40), lin8(0x80), lin8(0xC0)];
    let m: Vec<f32> = b.iter().map(|&cb| (1.0 - a) * cb + a * cb * 1.0).collect();
    assert_px(&r, 1, 1, [m[0], m[1], m[2], 1.0], 3e-3);
    let sc: Vec<f32> = b.iter().map(|&cb| (1.0 - a) * cb + a * (cb + 1.0 - cb)).collect();
    assert_px(&r, 9, 1, [sc[0], sc[1], sc[2], 1.0], 3e-3);
}

/// The dissolve hash of a frame pixel and the project seed.
fn dissolve_hash(x: u32, y: u32, seed: u64) -> f32 {
    let mut h = seed ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^= h >> 33;
    (h >> 40) as f32 / 16777216.0
}

#[test]
fn dissolve_keeps_a_random_fraction_of_pixels() {
    // a seed above 2^32 exercises both words of the GPU's 64-bit arithmetic
    let seed = 12_345_678_901_234u64;
    let d = doc(
        &format!(r##"width="64" height="64" background="#000000" seed="{seed}""##),
        "",
        r#"<layer id="a" asset="white" scaleX="16" scaleY="16" opacity="0.5" blend="dissolve"/>"#,
    );
    let Some(r) = render(&d) else { return };
    let on = r.px.iter().filter(|p| p[0] > 0.99).count();
    let off = r.px.iter().filter(|p| p[0] < 0.01).count();
    assert_eq!(on + off, 64 * 64, "dissolve is binary");
    assert!((on as f64 / 4096.0 - 0.5).abs() < 0.05, "{on}");
    // exactly the pixels whose hash is below the layer's alpha
    for y in 0..64 {
        for x in 0..64 {
            assert_eq!(r.at(x, y)[0] > 0.5, dissolve_hash(x, y, seed) < 0.5, "pixel ({x}, {y})");
        }
    }
}

#[test]
fn masks_combine_modes_invert_and_feather() {
    // mask coordinates are in the layer's local space: the 4×4 image box, drawn at 4× scale
    let d = doc(
        r##"background="#000000""##,
        "",
        r#"<layer id="a" asset="white" scaleX="4" scaleY="4">
             <mask type="rect" x="0" y="0" width="2" height="4"/>
           </layer>
           <layer id="b" asset="white" x="20" scaleX="4" scaleY="4">
             <mask type="rect" x="0" y="0" width="2" height="4" invert="true"/>
           </layer>
           <layer id="c" asset="white" x="40" scaleX="4" scaleY="4">
             <mask type="rect" x="0" y="0" width="4" height="4" feather="0.5"/>
           </layer>
           <layer id="d" asset="white" x="0" y="16" scaleX="4" scaleY="4">
             <mask type="rect" x="0" y="0" width="4" height="4"/>
             <mask type="ellipse" x="0" y="0" width="4" height="4" mode="subtract"/>
           </layer>
           <layer id="e" asset="white" x="20" y="16" scaleX="4" scaleY="4">
             <mask type="rect" x="0" y="0" width="1" height="4" mode="add"/>
             <mask type="rect" x="3" y="0" width="1" height="4" mode="add"/>
           </layer>"#,
    );
    let Some(r) = render(&d) else { return };
    assert_px(&r, 4, 4, [1.0; 4], 1e-3);
    assert_px(&r, 10, 4, [0.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 24, 4, [0.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 30, 4, [1.0; 4], 1e-3);
    // feather is a Gaussian of σ = 0.5 local units (2 pixels); the first pixel's centre lies
    // 0.125 units inside the edge
    let edge = r.at(40, 8)[0];
    let centre = r.at(48, 8)[0];
    assert!((edge - 0.5987).abs() < 0.02, "feathered edge {edge}");
    assert!(centre > 0.99, "centre {centre}");
    // rect minus ellipse: corners stay, centre removed
    assert_px(&r, 0, 16, [1.0; 4], 2e-2);
    assert_px(&r, 8, 24, [0.0, 0.0, 0.0, 1.0], 1e-3);
    // two added strips
    assert_px(&r, 22, 20, [1.0; 4], 1e-3);
    assert_px(&r, 28, 20, [0.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 34, 20, [1.0; 4], 1e-3);
}

#[test]
fn masks_add_as_a_plus_m_minus_am() {
    // two half-opaque rectangles added overlap to 0.5 + 0.5 − 0.25
    let d = doc(
        r##"background="#000000""##,
        "",
        r#"<layer id="a" asset="white" scaleX="4" scaleY="4">
             <mask type="rect" x="0" y="0" width="3" height="4" mode="add" opacity="0.5"/>
             <mask type="rect" x="1" y="0" width="3" height="4" mode="add" opacity="0.5"/>
           </layer>"#,
    );
    let Some(r) = render(&d) else { return };
    assert_px(&r, 2, 8, [0.5, 0.5, 0.5, 1.0], 3e-3);
    assert_px(&r, 8, 8, [0.75, 0.75, 0.75, 1.0], 3e-3);
}

#[test]
fn polygons_and_stars_lie_on_the_inscribed_ellipse() {
    // a square (4 points) in a 2:1 box is a diamond touching the middle of every side,
    // as a mask and as a shape
    let d = doc(
        r##"background="#000000""##,
        "",
        r##"<layer id="a" asset="wide" scaleX="2" scaleY="2">
             <mask type="polygon" x="0" y="0" width="8" height="4" points="4"/>
           </layer>
           <shape id="s" shape="polygon" x="32" y="0" width="16" height="8" points="4" fill="#FF0000"/>"##,
    );
    let Some(r) = render(&d) else { return };
    let lit = |x: u32, y: u32| r.at(x, y)[0].max(r.at(x, y)[2]);
    for x0 in [0u32, 32] {
        assert!(lit(x0 + 13, 4) > 0.98, "right vertex at {}: {:?}", x0 + 13, r.at(x0 + 13, 4));
        assert!(lit(x0 + 2, 4) > 0.98, "left vertex at {}: {:?}", x0 + 2, r.at(x0 + 2, 4));
        assert!(lit(x0 + 1, 1) < 0.02, "outside the diamond at {}: {:?}", x0 + 1, r.at(x0 + 1, 1));
    }
}

#[test]
fn polygon_star_and_path_masks() {
    let d = doc(
        r##"background="#000000""##,
        "",
        r#"<layer id="a" asset="white" scaleX="4" scaleY="4">
             <mask type="path" path="M0 0 L4 0 L0 4 Z"/>
           </layer>
           <layer id="b" asset="white" x="20" scaleX="4" scaleY="4">
             <mask type="star" x="0" y="0" width="4" height="4" points="5" innerRadius="0.75"/>
           </layer>
           <layer id="c" asset="white" x="40" scaleX="4" scaleY="4">
             <mask type="path" path="M0 0 H4 V4 H0 Z M1 1 H3 V3 H1 Z" fillRule="evenodd"/>
           </layer>"#,
    );
    let Some(r) = render(&d) else { return };
    assert_px(&r, 2, 2, [1.0; 4], 1e-3);
    assert_px(&r, 13, 13, [0.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 28, 8, [1.0; 4], 1e-3);
    assert_px(&r, 21, 15, [0.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 41, 1, [1.0; 4], 1e-3);
    assert_px(&r, 48, 8, [0.0, 0.0, 0.0, 1.0], 1e-3);
}

#[test]
fn alpha_and_luma_mattes() {
    let d = doc(
        r##"background="#000000""##,
        "",
        r#"<layer id="m" asset="red" x="0" y="0" scaleX="2" scaleY="4"/>
           <layer id="a" asset="white" scaleX="4" scaleY="4" matte="m"/>
           <layer id="lm" asset="gray" x="20" scaleX="4" scaleY="4"/>
           <layer id="b" asset="white" x="20" scaleX="4" scaleY="4" matte="lm" matteMode="luma"/>
           <layer id="ci" asset="white" x="40" scaleX="2" scaleY="4"/>
           <layer id="c" asset="white" x="40" scaleX="4" scaleY="4" matte="ci" matteMode="alpha-inverted"/>"#,
    );
    let Some(r) = render(&d) else { return };
    assert_px(&r, 2, 2, [1.0; 4], 1e-3);
    assert_px(&r, 12, 2, [0.0, 0.0, 0.0, 1.0], 1e-3);
    let g = lin8(128);
    assert_px(&r, 22, 2, [g, g, g, 1.0], 3e-3);
    assert_px(&r, 42, 2, [0.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 52, 2, [1.0; 4], 1e-3);
}

#[test]
fn isolated_groups_composite_as_one_layer() {
    let body = |iso: bool| {
        format!(
            r#"<group id="g" opacity="0.5" isolate="{iso}" width="64" height="32">
                 <layer id="a" asset="white" scaleX="4" scaleY="4"/>
                 <layer id="b" asset="white" x="8" scaleX="4" scaleY="4"/>
               </group>"#
        )
    };
    let Some(iso) = render(&doc(r##"background="#000000""##, "", &body(true))) else { return };
    let flat = render(&doc(r##"background="#000000""##, "", &body(false))).unwrap();
    // overlap region (8..16): isolated = 0.5, pass-through = 0.75
    assert_px(&iso, 12, 4, [0.5, 0.5, 0.5, 1.0], 3e-3);
    assert_px(&flat, 12, 4, [0.75, 0.75, 0.75, 1.0], 3e-3);
    assert_px(&iso, 4, 4, [0.5, 0.5, 0.5, 1.0], 3e-3);
    // a multiply group blends its composite, not each child
    let d = doc(
        r##"background="#FFFFFF""##,
        "",
        r#"<group id="g" blend="multiply" width="64" height="32"><layer id="a" asset="gray" scaleX="4" scaleY="4"/></group>"#,
    );
    let r = render(&d).unwrap();
    let g = lin8(128);
    assert_px(&r, 4, 4, [g, g, g, 1.0], 3e-3);
}

#[test]
fn masks_apply_to_groups_without_a_box() {
    let d = doc(
        r##"background="#000000""##,
        "",
        r#"<group id="g" x="8" y="0"><mask type="rect" x="0" y="0" width="8" height="32"/>
             <layer id="a" asset="white" scaleX="8" scaleY="8"/></group>"#,
    );
    let Some(r) = render(&d) else { return };
    assert_px(&r, 12, 4, [1.0; 4], 1e-3);
    assert_px(&r, 20, 4, [0.0, 0.0, 0.0, 1.0], 1e-3);
}

#[test]
fn clipped_instance_and_moved_group_reuse_the_offscreen() {
    let d = doc(
        r##"background="#000000""##,
        r#"<symbols><symbol id="s" width="8" height="8"><layer id="big" asset="white" scaleX="4" scaleY="4"/></symbol></symbols>"#,
        r#"<instance id="i" symbol="s" x="4" y="4">
             <animate property="x"><key time="0" value="4"/><key time="2" value="44"/></animate>
           </instance>"#,
    );
    let Some(a) = render_times(&d, &[0.0]) else { return };
    // the symbol box clips the 16×16 child to 8×8
    assert_px(&a, 6, 6, [1.0; 4], 1e-3);
    assert_px(&a, 14, 6, [0.0, 0.0, 0.0, 1.0], 1e-3);
    let b = render_times(&d, &[0.0, 1.0]).unwrap();
    assert_px(&b, 26, 6, [1.0; 4], 1e-3);
    assert_px(&b, 6, 6, [0.0, 0.0, 0.0, 1.0], 1e-3);
    assert!(b.stats.cache_hits >= 1, "moving an instance reuses its offscreen: {:?}", b.stats);
}

#[test]
fn unchanged_frames_restore_the_root_prefix() {
    let d = doc(
        r##"background="#202020""##,
        "",
        r#"<layer id="bg1" asset="white" scaleX="16" scaleY="8" opacity="0.2"/>
           <layer id="bg2" asset="red" x="16" scaleX="4" scaleY="4" blend="screen"/>
           <layer id="mover" asset="red" x="0" y="20"><animate property="x"><key time="0" value="0"/><key time="2" value="60"/></animate></layer>"#,
    );
    let Some(r) = render_times(&d, &[0.0, 0.1, 0.2]) else { return };
    // the two static layers are restored; the mover draws, then the background beneath everything
    assert_eq!(r.stats.prefix_restored, 2, "{:?}", r.stats);
    assert_eq!(r.stats.draws, 2);
    let fresh = render_times(&d, &[0.2]).unwrap();
    assert_eq!(fresh.px, r.px, "restored frames are identical to fresh renders");
}

#[test]
fn gradients_interpolate_in_the_requested_space() {
    let paints = |space: &str| {
        format!(
            r##"<paints><linearGradient id="g" interpolationSpace="{space}" dither="false"><stop offset="0" color="#000000"/><stop offset="1" color="#FFFFFF"/></linearGradient></paints>"##
        )
    };
    let Some(lin) = render(&doc(r#"background="url(#g)""#, &paints("linear"), "")) else { return };
    let ok = render(&doc(r#"background="url(#g)""#, &paints("oklab"), "")).unwrap();
    let srgb = render(&doc(r#"background="url(#g)""#, &paints("srgb"), "")).unwrap();
    // pixel 31.5/64 ≈ t 0.4922
    let t = 31.5 / 64.0;
    assert!((lin.at(31, 16)[0] - t).abs() < 3e-3, "{:?}", lin.at(31, 16));
    assert!((srgb.at(31, 16)[0] - srgb_to_linear(t)).abs() < 3e-3, "{:?}", srgb.at(31, 16));
    // oklab: L is linear in t, and Y = L³ for greys
    assert!((ok.at(31, 16)[0] - t.powi(3)).abs() < 3e-3, "{:?}", ok.at(31, 16));
    // radial and conic
    let d = doc(
        r#"background="url(#r)""#,
        r##"<paints><radialGradient id="r" dither="false"><stop offset="0" color="#FFFFFF"/><stop offset="1" color="#000000"/></radialGradient></paints>"##,
        "",
    );
    let r = render(&d).unwrap();
    assert!(r.at(32, 16)[0] > 0.9 && r.at(0, 0)[0] < 1e-3);
    let d = doc(
        r#"background="url(#c)""#,
        r##"<paints><conicGradient id="c" dither="false"><stop offset="0" color="#000000"/><stop offset="1" color="#FFFFFF"/></conicGradient></paints>"##,
        "",
    );
    let c = render(&d).unwrap();
    // right of centre is a quarter turn clockwise from the top; left is three quarters
    assert!(
        (c.at(60, 16)[0] - 0.25).abs() < 0.03 && (c.at(4, 16)[0] - 0.75).abs() < 0.03,
        "{:?} {:?}",
        c.at(60, 16),
        c.at(4, 16)
    );
}

#[test]
fn mesh_gradient_corners() {
    let d = doc(
        r#"background="url(#m)""#,
        r##"<paints><meshGradient id="m" interpolationSpace="linear">
              <point row="0" col="0" color="#FF0000"/><point row="0" col="1" color="#00FF00"/>
              <point row="1" col="0" color="#0000FF"/><point row="1" col="1" color="#FFFFFF"/>
            </meshGradient></paints>"##,
        "",
    );
    let Some(r) = render(&d) else { return };
    assert!(r.at(0, 0)[0] > 0.95 && r.at(63, 0)[1] > 0.95 && r.at(0, 31)[2] > 0.95);
    let c = r.at(32, 16);
    assert!((c[0] - 0.5).abs() < 0.05 && (c[1] - 0.5).abs() < 0.05, "{c:?}");
    // positions and colours follow one Catmull-Rom surface, so a 2 × 2 grid is linear
    // in position (not smoothstepped)
    let q = r.at(16, 0);
    let f = 16.5 / 64.0;
    assert!((q[0] - (1.0 - f)).abs() < 0.02 && (q[1] - f).abs() < 0.02, "{q:?}");
}

#[test]
fn generator_patterns_and_gradient_geometry_follow_the_python_renderer() {
    // patterns centre on the asset (a 12-wide checkerboard of 4 px squares starts
    // mid-square), stripes repeat every 2 × scale, radial aspect stretches x, rotation pivots on the
    // box centre in pixels
    let assets = r##"<paints>
        <radialGradient id="rg" aspect="2"><stop offset="0" color="#FFFFFF"/><stop offset="1" color="#000000"/></radialGradient>
        <linearGradient id="lg" rotation="90"><stop offset="0" color="#000000"/><stop offset="1" color="#FFFFFF"/></linearGradient>
      </paints>"##;
    let xml = format!(
        r##"<scene version="1.1"><project width="64" height="48" fps="10" duration="1" background="#000000"/>
        <assets>
          <generator id="ck" kind="checkerboard" width="12" height="4" scale="4" paint="#FFFFFF" paint2="#000000"/>
          <generator id="st" kind="stripes" width="32" height="4" scale="4" paint="#FFFFFF" paint2="#000000"/>
        </assets>{assets}<composition>
          <layer id="c" asset="ck"/><layer id="s" asset="st" y="8"/>
          <shape id="r" shape="rect" x="0" y="16" width="32" height="16" fill="url(#rg)"/>
          <shape id="l" shape="rect" x="32" y="16" width="32" height="16" fill="url(#lg)"/>
        </composition></scene>"##
    );
    let d = sr_model::load_str(&xml, &sr_model::LoadOptions::default()).unwrap();
    let Some(r) = render(&d) else { return };
    // checker: squares from the centre (x = 6): x 6..10 paint, 2..6 paint2, 0..2 paint
    assert_px(&r, 0, 2, [1.0; 4], 1e-3);
    assert_px(&r, 3, 2, [0.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 7, 2, [1.0; 4], 1e-3);
    // stripes from the centre (x = 16): paint for 16..20, paint2 for 20..24, paint again from 24
    assert_px(&r, 17, 10, [1.0; 4], 1e-3);
    assert_px(&r, 21, 10, [0.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 25, 10, [1.0; 4], 1e-3);
    // radial, aspect 2 in a 32 × 16 box: the ellipse reaches x = ±32 (the box's full width) and y = ±8,
    // so 8 px right of the centre is a quarter of the way out, 4 px down half of it
    let right = r.at(24, 23)[0];
    let down = r.at(15, 28)[0];
    assert!(right > down + 0.1, "stretched along x: right {right}, down {down}");
    // linear rotated 90° clockwise about the box centre in pixels: dark at the top, light at the bottom,
    // spanning the middle half of the gradient (the box is 16 px tall, the gradient 32 px long)
    let (top, bottom) = (r.at(48, 16)[0], r.at(48, 31)[0]);
    assert!((top - 0.266).abs() < 0.03 && (bottom - 0.734).abs() < 0.03, "{top} {bottom}");
}

#[test]
fn generators_render_deterministically() {
    let d = doc(
        r##"background="#000000""##,
        "",
        r#"<layer id="c" asset="checker"/><layer id="n" asset="noise" x="20"/><layer id="s" asset="solid" x="40"/>"#,
    );
    let Some(a) = render(&d) else { return };
    assert_px(&a, 1, 1, [1.0; 4], 1e-3);
    assert_px(&a, 5, 1, [0.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&a, 5, 5, [1.0; 4], 1e-3);
    assert_px(&a, 41, 1, [0.0, 1.0, 0.0, 1.0], 1e-3);
    let b = render(&d).unwrap();
    let noise_a: Vec<_> = (20..36).map(|x| a.at(x, 8)).collect();
    let noise_b: Vec<_> = (20..36).map(|x| b.at(x, 8)).collect();
    assert_eq!(noise_a, noise_b);
    assert!(
        noise_a.iter().any(|p| p[0] > 0.05) && noise_a.iter().any(|p| (p[0] - noise_a[0][0]).abs() > 0.01),
        "noise varies"
    );
}

#[test]
fn layers_in_2_5d_project_and_sort_by_depth() {
    let d = doc(
        r##"width="64" height="64" background="#000000""##,
        "",
        r#"<layer id="flat" asset="white" x="32" y="32" anchorX="2" anchorY="2" scaleX="8" scaleY="8" threeD="true" rotationY="60"/>"#,
    );
    let Some(r) = render(&d) else { return };
    let row: Vec<bool> = (0..64).map(|x| r.at(x, 32)[0] > 0.5).collect();
    let width = row.iter().filter(|&&b| b).count();
    // 32 px wide at 60° → about 16 px, perspective keeps it near that
    assert!((13..=19).contains(&width), "projected width {width}");
    // depth sort: the nearer (negative z, still in front of the eye at z ≈ −88.8) red layer covers the farther white one regardless of order
    let d = doc(
        r##"width="64" height="64" background="#000000""##,
        "",
        r#"<layer id="near" asset="red" x="16" y="16" scaleX="8" scaleY="8" threeD="true" zDepth="-50"/>
           <layer id="far" asset="white" x="16" y="16" scaleX="8" scaleY="8" threeD="true" zDepth="100"/>"#,
    );
    let r = render(&d).unwrap();
    assert_px(&r, 30, 30, [1.0, 0.0, 0.0, 1.0], 1e-3);
}

#[test]
fn positive_2_5d_rotations_turn_the_right_and_top_edges_away() {
    // conventions 5.14: the edge that turns away is farther from the eye, so it projects shorter
    let flat = |rot: &str| {
        let d = doc(
            r##"width="64" height="64" background="#000000""##,
            "",
            &format!(
                r#"<layer id="flat" asset="white" x="32" y="32" anchorX="2" anchorY="2" scaleX="8" scaleY="8" threeD="true" {rot}="40"/>"#
            ),
        );
        render(&d)
    };
    let Some(r) = flat("rotationY") else { return };
    let cols: Vec<u32> = (0..64).filter(|x| r.at(*x, 32)[0] > 0.5).collect();
    let height = |x: u32| (0..64).filter(|y| r.at(x, *y)[0] > 0.5).count();
    let (left, right) = (height(cols[0] + 1), height(cols[cols.len() - 1] - 1));
    assert!(right + 2 < left, "rotationY: right edge {right} px vs left {left} px");
    let r = flat("rotationX").unwrap();
    let rows: Vec<u32> = (0..64).filter(|y| r.at(32, *y)[0] > 0.5).collect();
    let width = |y: u32| (0..64).filter(|x| r.at(*x, y)[0] > 0.5).count();
    let (top, bottom) = (width(rows[0] + 1), width(rows[rows.len() - 1] - 1));
    assert!(top + 2 < bottom, "rotationX: top edge {top} px vs bottom {bottom} px");
}

#[test]
fn fit_cover_contain_and_blur_fill() {
    let d = doc(
        r##"background="#000000""##,
        "",
        r#"<layer id="cover" asset="wide" fit="cover" boxWidth="16" boxHeight="16"/>
           <layer id="contain" asset="wide" x="20" fit="contain" boxWidth="16" boxHeight="16"/>
           <layer id="blur" asset="wide" x="40" fit="contain-blur" boxWidth="16" boxHeight="16"/>
           <layer id="flip" asset="wide" x="0" y="20" flipX="true"/>"#,
    );
    let Some(r) = render(&d) else { return };
    // cover shows the middle half of the 2:1 image: red on the left, blue on the right
    assert_px(&r, 2, 8, [1.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 13, 8, [0.0, 0.0, 1.0, 1.0], 1e-3);
    // contain letterboxes: 16×8 content centred vertically
    assert_px(&r, 22, 1, [0.0, 0.0, 0.0, 1.0], 1e-3);
    assert_px(&r, 22, 8, [1.0, 0.0, 0.0, 1.0], 1e-3);
    // contain-blur fills the bars with a blurred cover of the same image
    let bar = r.at(48, 1);
    assert!(bar[0] > 0.05 || bar[2] > 0.05, "blur fill {bar:?}");
    // flipX puts blue on the left
    assert_px(&r, 1, 21, [0.0, 0.0, 1.0, 1.0], 1e-3);
}

#[test]
fn flex_layout_positions_rendered_layers() {
    let d = doc(
        r##"background="#000000""##,
        "",
        r#"<group id="row" width="64" height="32" layout="row" gap="4" justify="center" alignItems="center">
             <layer id="a" asset="red"/><layer id="b" asset="red"/><layer id="c" asset="red"/>
           </group>"#,
    );
    let Some(r) = render(&d) else { return };
    // 3×4 + 2×4 = 20 wide, centred: starts at 22; vertically at 14
    for x in [22, 30, 38] {
        assert_px(&r, x + 1, 15, [1.0, 0.0, 0.0, 1.0], 1e-3);
        assert_px(&r, x + 5, 15, [0.0, 0.0, 0.0, 1.0], 1e-3);
    }
}

#[test]
fn image_sequences_follow_source_time() {
    let d = doc(r##"background="#000000""##, "", r#"<layer id="s" asset="seq" scaleX="4" scaleY="4"/>"#);
    let Some(a) = render_times(&d, &[0.0]) else { return };
    let b = render_times(&d, &[0.15]).unwrap();
    assert!((a.at(1, 1)[0] - lin8(80)).abs() < 3e-3);
    assert!((b.at(1, 1)[0] - lin8(160)).abs() < 3e-3);
}

#[test]
fn a_clip_that_does_not_loop_ends_with_its_media() {
    // the three-frame sequence at 10 fps runs out at 0.3 s
    let d = doc(
        r##"background="#000000""##,
        "",
        r#"<layer id="s" asset="seq" scaleX="4" scaleY="4"/><layer id="l" asset="seq" x="16" scaleX="4" scaleY="4" loop="1"/>"#,
    );
    let Some(r) = render_times(&d, &[0.45]) else { return };
    assert_px(&r, 1, 1, [0.0, 0.0, 0.0, 1.0], 1e-3);
    assert!((r.at(17, 1)[0] - lin8(160)).abs() < 3e-3, "the looping copy plays on: {:?}", r.at(17, 1));
}

#[test]
fn working_spaces_and_non_linear_compositing() {
    // ACEScg working space: sRGB red decodes into AP1 and displays back as red
    let d = doc(
        r##"workingColorSpace="acescg" background="#000000""##,
        "",
        r#"<layer id="a" asset="red" scaleX="4" scaleY="4"/>"#,
    );
    let Some(r) = render(&d) else { return };
    let p = r.at(2, 2);
    assert!((p[0] - 0.6131).abs() < 3e-3 && (p[1] - 0.0702).abs() < 3e-3, "AP1 red {p:?}");
    let rgba = r.renderer.to_srgb8(&[p]);
    assert!(rgba[0] >= 254 && rgba[1] <= 1 && rgba[2] <= 1, "{rgba:?}");
    // linearLight=false blends encoded values: 50 % red over blue is (0.5, 0, 0.5) encoded
    let d = doc(r##"linearLight="false" background="#0000FF""##, "", r#"<layer id="a" asset="red" opacity="0.5"/>"#);
    let r = render(&d).unwrap();
    assert_px(&r, 1, 1, [0.5, 0.0, 0.5, 1.0], 3e-3);
    assert_eq!(r.renderer.to_srgb8(&[r.at(1, 1)])[..3], [128, 0, 128]);
}

#[test]
fn every_node_kind_is_supported() {
    // particle emitters draw and report nothing as unsupported
    let d = doc(r##"background="#000000""##, "", r##"<particleEmitter id="sparks" preset="sparks" x="32" y="16"/>"##);
    let Some(r) = render_times(&d, &[1.0]) else { return };
    assert!(r.stats.unsupported.is_empty(), "{:?}", r.stats.unsupported);
    assert!(r.px.iter().any(|p| p[0] > 0.1), "sparks drawn");
}

#[test]
fn animated_z_restacks_with_warm_caches() {
    // red rises from under green to over it at t = 1; the group repeats it inside an isolated
    // offscreen. Frames alternate so every cache sees both orders.
    let pair = |p: &str| {
        format!(
            r##"<shape id="{p}r" shape="rect" x="0" y="0" width="16" height="16" fill="#FF0000">
                  <animate property="z"><key time="0" value="0" interpolation="hold"/><key time="1" value="2"/></animate>
                </shape>
                <shape id="{p}g" shape="rect" x="0" y="0" width="16" height="16" fill="#00FF00" z="1"/>"##
        )
    };
    let body = format!(r#"{}<group id="iso" x="32" isolate="true">{}</group>"#, pair("a"), pair("b"));
    let d = doc("", "", &body);
    let (green, red) = ([0.0, 1.0, 0.0, 1.0], [1.0, 0.0, 0.0, 1.0]);
    for (ts, want) in [
        (&[0.5][..], green),
        (&[1.5][..], red),
        (&[0.5, 1.5][..], red),
        (&[1.5, 0.5][..], green),
        (&[0.5, 1.5, 0.5, 1.5][..], red),
    ] {
        let Some(r) = render_times(&d, ts) else { return };
        assert_px(&r, 8, 8, want, 1e-3);
        assert_px(&r, 40, 8, want, 1e-3);
    }
}

#[test]
fn isolated_content_fading_in_from_zero_is_not_cached_empty() {
    // an instance (always isolated) and an isolated group fade in from opacity 0; the frame at 0
    // must not leave an empty offscreen in the cache for the later frames
    let d = doc(
        "",
        r##"<symbols><symbol id="card" width="16" height="16"><shape id="bg" shape="rect" width="16" height="16" fill="#00FF00"/></symbol></symbols>"##,
        r##"<instance id="i" symbol="card" x="0" y="0" opacity="0">
              <animate property="opacity"><key time="0.5" value="0"/><key time="1" value="1"/></animate>
            </instance>
            <group id="g" x="32" y="0" isolate="true" opacity="0">
              <shape id="s" shape="rect" width="16" height="16" fill="#00FF00"/>
              <animate property="opacity"><key time="0.5" value="0"/><key time="1" value="1"/></animate>
            </group>"##,
    );
    let green = [0.0, 1.0, 0.0, 1.0];
    for ts in [&[1.5][..], &[0.0, 1.5][..], &[0.0, 0.7, 1.5][..]] {
        let Some(r) = render_times(&d, ts) else { return };
        assert_px(&r, 8, 8, green, 1e-3);
        assert_px(&r, 40, 8, green, 1e-3);
    }
}

#[test]
fn specialized_ellipse_masks_match_general_masks_exactly() {
    let mut body = String::new();
    for (b, blend) in ["normal", "dissolve", "multiply", "add"].iter().enumerate() {
        for (f, feather) in [0.001, 0.5, 7.0].iter().enumerate() {
            for (e, expansion) in [-0.75, 0.0, 1.25].iter().enumerate() {
                let x = 6 + b * 42 + e * 9;
                let y = 6 + f * 38 + e * 3;
                body.push_str(&format!(
                    r#"<layer id="l{b}_{f}_{e}" asset="quad" x="{x}" y="{y}" scaleX="9" scaleY="7" rotation="13" opacity="0.7" blend="{blend}">
                    <mask type="ellipse" x="0.1" y="0.2" width="1.6" height="1.2" feather="{feather}" expansion="{expansion}" invert="{}" opacity="0.8"/>
                    </layer>"#,
                    e == 1,
                ));
            }
        }
    }
    let project = r##"width="200" height="140" background="#33445580""##;
    let Some(specialized) = render(&doc(project, "", &body)) else { return };
    // A disabled second mask preserves coverage but forces the general shader.
    let general = render(&doc(
        project,
        "",
        &body.replace("</layer>", r#"<mask type="rect" width="2" height="2" mode="none"/></layer>"#),
    ))
    .unwrap();
    assert_eq!(specialized.px, general.px);
}

fn emitter_box(attrs: &str) -> Option<[u32; 4]> {
    // a dust cloud over a 200 x 100 rect on a 480 x 270 frame, speed 0 so the cloud keeps its shape
    let xml = format!(
        r##"<scene version="1.1"><project width="480" height="270" fps="24" duration="2" background="#000000" motionBlur="false" seed="1"/>
<composition><particleEmitter id="e" preset="dust" x="240" y="135" emitterShape="rect" emitterWidth="200" emitterHeight="100" rate="120" lifetime="2" speed="0" size="6" color="#FFFFFF" seed="5" preroll="2" {attrs}/></composition></scene>"##
    );
    let d = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
    let r = render(&d)?;
    let lit: Vec<(u32, u32)> =
        (0..270u32).flat_map(|y| (0..480u32).map(move |x| (x, y))).filter(|&(x, y)| r.at(x, y)[0] > 0.2).collect();
    assert!(lit.len() > 50, "the cloud is drawn: {} lit pixels", lit.len());
    let (x0, x1) = (lit.iter().map(|p| p.0).min()?, lit.iter().map(|p| p.0).max()?);
    let (y0, y1) = (lit.iter().map(|p| p.1).min()?, lit.iter().map(|p| p.1).max()?);
    Some([x0, y0, x1, y1])
}

#[test]
fn particle_emitters_place_themselves_in_2_5d() {
    // nodeAttributes promise threeD placement for every node: an emitter at depth shrinks, and turns, like a layer
    let Some(flat) = emitter_box("") else { return };
    let w = |b: [u32; 4]| (b[2] - b[0]) as f64;
    let h = |b: [u32; 4]| (b[3] - b[1]) as f64;
    // zDepth 0 changes nothing
    let zero = emitter_box(r#"threeD="true" zDepth="0""#).unwrap();
    assert!((w(zero) - w(flat)).abs() <= 2.0 && (h(zero) - h(flat)).abs() <= 2.0, "{zero:?} vs {flat:?}");
    // zDepth 600 under a 60 degree horizontal field: scale f / (f + 600) with f = 240 / tan 30 degrees = 415.7
    let far = emitter_box(r#"threeD="true" zDepth="600""#).unwrap();
    let want = 415.7 / (415.7 + 600.0);
    for (name, got, base) in [("width", w(far), w(flat)), ("height", h(far), h(flat))] {
        let ratio = got / base;
        assert!((ratio - want).abs() < 0.07, "{name} ratio {ratio:.2}, wanted about {want:.2}: {far:?} vs {flat:?}");
    }
    // rotationY turns the sheet edge-on: narrower, same height
    let turned = emitter_box(r#"threeD="true" rotationY="60""#).unwrap();
    assert!(w(turned) < w(flat) * 0.8, "rotationY did not narrow the cloud: {turned:?} vs {flat:?}");
    assert!((h(turned) - h(flat)).abs() < h(flat) * 0.25, "{turned:?} vs {flat:?}");
}

#[test]
fn an_animated_path_is_drawn_as_animated() {
    // string keys hold between keys: the shape follows its animated @path, not only the static one
    let d = doc(
        r##"width="64" height="64" background="#000000""##,
        "",
        r##"<shape id="s" shape="path" path="M0 0 H10 V10 H0 Z" width="10" height="10" fill="#FFFFFF">
             <animate property="path"><key time="0" value="M0 0 H10 V10 H0 Z"/><key time="1" value="M30 30 H60 V60 H30 Z"/></animate></shape>"##,
    );
    let Some(first) = render_times(&d, &[0.0]) else { return };
    assert_px(&first, 5, 5, [1.0, 1.0, 1.0, 1.0], 1e-3);
    assert_px(&first, 45, 45, [0.0, 0.0, 0.0, 1.0], 1e-3);
    let later = render_times(&d, &[1.5]).unwrap();
    assert_px(&later, 45, 45, [1.0, 1.0, 1.0, 1.0], 1e-3);
    assert_px(&later, 5, 5, [0.0, 0.0, 0.0, 1.0], 1e-3);
}

#[test]
fn a_grid_generator_takes_its_line_width_from_line_width_not_from_scale() {
    // lines are about 4 % of `scale` (4 px at scale 100) unless `lineWidth` (pixels) says otherwise; the pitch is
    // `scale` either way
    let width_at = |attrs: &str, scale: u32| {
        let xml = format!(
            r##"<scene version="1.1"><project width="128" height="64" fps="10" duration="1" background="#000000"/>
<assets><generator id="gr" kind="grid" width="128" height="64" scale="{scale}" paint="#FFFFFF" paint2="#000000" {attrs}/></assets>
<composition><layer id="l" asset="gr"/></composition></scene>"##
        );
        let d = sr_model::load_str(&xml, &sr_model::LoadOptions::default()).unwrap_or_else(|e| panic!("{e:?}"));
        let r = render(&d)?;
        // a row away from the horizontal lines: the vertical line through the centre (x = 64)
        Some((0..128u32).filter(|&x| r.at(x, 10)[0] > 0.5).count())
    };
    let Some(legacy) = width_at("", 100) else { return };
    assert_eq!(legacy, 4, "the default stays about 4 % of the scale");
    assert_eq!(width_at(r#"lineWidth="1""#, 100).unwrap(), 1);
    assert_eq!(width_at(r#"lineWidth="3""#, 100).unwrap(), 3);
    // a 1 px line at a 25 px pitch: five vertical lines across 128 px, each one pixel wide
    assert_eq!(width_at(r#"lineWidth="1""#, 25).unwrap(), 5);
    assert_eq!(width_at(r#"lineWidth="0""#, 100).unwrap(), 0, "no lines");
}
