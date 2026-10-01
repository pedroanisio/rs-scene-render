//! Wide alpha morphology (stroke, outline, matte-choke beyond a few texels): the texels that
//! decide the max and min of alpha over a small disc are found once and the disc doubled pass by
//! pass, so the cost grows with the logarithm of the radius and the result keeps every level of
//! alpha, partial coverage included.

mod common;

use common::*;

fn scene(size: u32, body: &str, effects: &str) -> sr_model::Document {
    let effects = if effects.is_empty() { String::new() } else { format!("<effects>{effects}</effects>") };
    let xml = format!(
        r##"<scene version="1.1"><project width="{size}" height="{size}" fps="10" duration="1" background="#00000000"/><composition>{body}</composition>{effects}</scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

/// Max (dilate) or min (erode) of alpha over a disc of radius `r` with a one-texel soft rim,
/// texels beyond the frame being transparent: what `fs_morph` defines, by visiting every texel.
fn reference(alpha: &[f32], size: i32, r: f32, erode: bool) -> Vec<f32> {
    let reach = r.ceil() as i32 + 1;
    let mut out = Vec::with_capacity(alpha.len());
    for y in 0..size {
        for x in 0..size {
            let mut best = alpha[(y * size + x) as usize];
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let w = (r + 1.0 - ((dx * dx + dy * dy) as f32).sqrt()).clamp(0.0, 1.0);
                    let (qx, qy) = (x + dx, y + dy);
                    let a = if qx < 0 || qy < 0 || qx >= size || qy >= size {
                        0.0
                    } else {
                        alpha[(qy * size + qx) as usize]
                    };
                    best = if erode { best.min(a + 1.0 - w) } else { best.max(a * w) };
                }
            }
            out.push(best);
        }
    }
    out
}

/// Mean absolute difference, and how many pixels differ by more than 0.2.
fn diff(a: &[f32], b: &[f32]) -> (f32, usize) {
    let d = a.iter().zip(b).map(|(x, y)| (x - y).abs());
    (d.clone().sum::<f32>() / a.len() as f32, d.filter(|e| *e > 0.2).count())
}

#[test]
fn wide_strokes_cost_a_pass_per_doubling_of_their_width() {
    let passes = |size: u32| {
        let d = scene(
            320,
            r##"<shape id="dot" shape="ellipse" x="150" y="150" width="20" height="20" fill="#FFFFFF" effects="ring"/>"##,
            &format!(r##"<effect id="ring" type="stroke" size="{size}" color="#FF0000"/>"##),
        );
        let r = render_sub(&d, 0.0)?;
        assert!(r.stats.unsupported.is_empty(), "{:?}", r.stats.unsupported);
        // the stroke reaches `size` px beyond the dot of radius 10 and no further
        let edge = 160 + 10 + size;
        assert!(r.at(edge - 3, 160)[3] > 0.95, "size {size}: {:?}", r.at(edge - 3, 160));
        assert!(r.at(edge + 3, 160)[3] < 0.05, "size {size}: {:?}", r.at(edge + 3, 160));
        Some(r.stats.fx_passes)
    };
    let (Some(narrow), Some(wide)) = (passes(8), passes(128)) else { return };
    // every texel of a 4 px disc, then a pass per doubling, the last one drawing the stroke.
    // Reading every texel of the whole disc would be one pass of 50 000 texel reads per pixel.
    assert_eq!(narrow, 2, "8 px: a 4 px disc, doubled by the stroke's pass");
    assert_eq!(wide, narrow + 4, "128 px is four doublings of 8 px");
}

#[test]
fn wide_morphology_follows_every_level_of_alpha() {
    // a disc (anti-aliased), opaque or not quite, on a quarter-opaque plate: three levels of
    // alpha and edges of partial coverage
    for top in ["FF", "CC"] {
        let body = |fx: &str| {
            format!(
                r##"<group id="g"{fx}><shape id="plate" shape="rect" x="34" y="40" width="60" height="50" fill="#FFFFFF40"/>
                <shape id="disc" shape="ellipse" x="52.3" y="50.6" width="22" height="26" fill="#FFFFFF{top}"/></group>"##
            )
        };
        let Some(src) = render_sub(&scene(128, &body(""), ""), 0.0) else { return };
        let alpha: Vec<f32> = src.px.iter().map(|p| p[3]).collect();
        for radius in [6.0f32, 13.0] {
            for (effect, erode) in [
                (format!(r##"<effect id="fx" type="stroke" size="{radius}" color="#FF0000"/>"##), false),
                (
                    format!(r##"<effect id="fx" type="stroke" size="{radius}" position="inside" color="#FF0000"/>"##),
                    true,
                ),
                (format!(r##"<effect id="fx" type="matte-choke" amount="{radius}" softness="0"/>"##), true),
                (format!(r##"<effect id="fx" type="matte-choke" amount="-{radius}" softness="0"/>"##), false),
            ] {
                let r = render_sub(&scene(128, &body(r#" effects="fx""#), &effect), 0.0).unwrap();
                assert!(r.stats.unsupported.is_empty(), "{:?}", r.stats.unsupported);
                let m = reference(&alpha, 128, radius, erode);
                let (got, want): (Vec<f32>, Vec<f32>) = if effect.contains("stroke") {
                    // the red ring: max - alpha outside, alpha - min inside; composited over or
                    // under white, its coverage is what the frame lacks in green
                    let ring = |i: usize| if erode { alpha[i] - m[i] } else { m[i] - alpha[i] };
                    let cover = |i: usize| if erode { ring(i) } else { ring(i) * (1.0 - alpha[i]) };
                    (
                        r.px.iter().map(|p| p[0] - p[1]).collect(),
                        (0..alpha.len()).map(|i| cover(i).clamp(0.0, 1.0)).collect(),
                    )
                } else {
                    (r.px.iter().map(|p| p[3]).collect(), m)
                };
                // the rim of the disc is one texel soft: a few of its texels, where two levels of
                // alpha meet, may take the other one's coverage
                let (mean, off) = diff(&got, &want);
                eprintln!("{top} {effect}: mean {mean:.5}, {off} px off");
                assert!(mean < 3e-3 && off <= 32, "{top} {effect}: mean {mean}, {off} px off the exact result");
            }
        }
    }
}

#[test]
fn wide_chokes_keep_soft_edges_soft() {
    // a feathered edge moves in by the choke; it does not collapse to a hard one
    let body = |fx: &str| {
        format!(r##"<shape id="sq" shape="rect" x="44" y="44" width="40" height="40" fill="#FFFFFF" effects="{fx}"/>"##)
    };
    let soft = r#"<effect id="soft" type="blur" radius="5"/>"#;
    let Some(src) = render_sub(&scene(128, &body("soft"), soft), 0.0) else { return };
    let alpha: Vec<f32> = src.px.iter().map(|p| p[3]).collect();
    for (amount, erode) in [(7.0f32, true), (-7.0, false)] {
        let fx = format!(r#"{soft}<effect id="choke" type="matte-choke" amount="{amount}" softness="0"/>"#);
        let r = render_sub(&scene(128, &body("soft choke"), &fx), 0.0).unwrap();
        let got: Vec<f32> = r.px.iter().map(|p| p[3]).collect();
        let (mean, off) = diff(&got, &reference(&alpha, 128, amount.abs(), erode));
        eprintln!("choke {amount}: mean {mean:.5}, {off} px off");
        assert!(mean < 4e-3 && off == 0, "choke {amount}: mean {mean}, {off} px off the exact result");
    }
}
