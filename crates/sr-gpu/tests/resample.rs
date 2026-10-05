//! `layer/@resample`: the filter used where a layer is drawn larger than its source.

mod common;
use common::*;

/// Texel values of the 4 × 1 test image (the red channel; grey on the other two).
const TEXELS: [u8; 4] = [0, 255, 0, 255];

fn project(asset: &str, attrs: &str, scale: f64) -> sr_model::Document {
    let img = image::RgbaImage::from_fn(4, 1, |x, _| {
        let v = TEXELS[x as usize];
        image::Rgba([v, v, v, 255])
    });
    img.save(fixtures().join("ramp4.png")).unwrap();
    let assets = ASSETS.replace("</assets>", r#"<image id="ramp4" src="ramp4.png" width="4" height="1"/></assets>"#);
    let xml = format!(
        r##"<scene version="1.1"><project width="64" height="16" fps="10" duration="2" background="#000000"/>{assets}<composition><layer id="l" asset="{asset}" x="0" y="0" scaleX="{scale}" scaleY="{scale}" {attrs}/></composition></scene>"##
    );
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(fixtures()) };
    sr_model::load_str(&xml, &opts).unwrap_or_else(|e| panic!("{e:?}\n{xml}"))
}

fn document(attrs: &str, scale: f64) -> sr_model::Document {
    project("ramp4", attrs, scale)
}

fn row(r: &Rendered, y: u32) -> Vec<f32> {
    (0..r.size[0]).map(|x| r.at(x, y)[0]).collect()
}

/// The weights of the four taps around fraction `t` for a cubic with Mitchell–Netravali parameters (b, c).
fn weights(t: f64, b: f64, c: f64) -> [f64; 4] {
    let k = |s: f64| {
        let s = s.abs();
        (if s < 1.0 {
            (12.0 - 9.0 * b - 6.0 * c) * s.powi(3) + (-18.0 + 12.0 * b + 6.0 * c) * s * s + (6.0 - 2.0 * b)
        } else if s < 2.0 {
            (-b - 6.0 * c) * s.powi(3)
                + (6.0 * b + 30.0 * c) * s * s
                + (-12.0 * b - 48.0 * c) * s
                + (8.0 * b + 24.0 * c)
        } else {
            0.0
        }) / 6.0
    };
    [k(t + 1.0), k(t), k(1.0 - t), k(2.0 - t)]
}

/// The reference sample of the ramp at pixel column `x` of a 64-pixel row drawn 16 times larger, in linear light,
/// with the four-texel neighbourhood clamped to the edge and the result clamped to the nearest two texels' range.
fn reference(x: u32, b: f64, c: f64) -> f32 {
    let pos = (x as f64 + 0.5) / 16.0; // texel space; texel centres at i + 0.5
    let i = (pos - 0.5).floor();
    let t = pos - 0.5 - i;
    let w = weights(t, b, c);
    let texel = |j: i64| lin8(TEXELS[j.clamp(0, 3) as usize]) as f64;
    let v: f64 = (0..4).map(|k| w[k] * texel(i as i64 + k as i64 - 1)).sum();
    let (lo, hi) = (texel(i as i64).min(texel(i as i64 + 1)), texel(i as i64).max(texel(i as i64 + 1)));
    v.clamp(lo, hi) as f32
}

#[test]
fn resample_defaults_to_linear() {
    let Some(a) = render(&document("", 16.0)) else { return };
    let Some(b) = render(&document(r#"resample="linear""#, 16.0)) else { return };
    assert_eq!(row(&a, 8), row(&b, 8));
}

#[test]
fn bicubic_is_catmull_rom_with_the_neighbourhood_clamp() {
    let Some(lin) = render(&document("", 16.0)) else { return };
    let Some(bic) = render(&document(r#"resample="bicubic""#, 16.0)) else { return };
    let (l, b) = (row(&lin, 8), row(&bic, 8));
    assert_ne!(l, b, "bicubic must differ from bilinear on a magnified ramp");
    for x in 0..64u32 {
        let want = reference(x, 0.0, 0.5);
        assert!((b[x as usize] - want).abs() < 0.01, "x {x}: got {} want {want}", b[x as usize]);
    }
    // a texel centre keeps its texel (the filter interpolates): texel 1 is at x = 24
    assert!((b[23] - lin8(255)).abs() < 0.02 && (b[24] - lin8(255)).abs() < 0.02);
}

#[test]
fn mitchell_is_the_b_c_one_third_cubic() {
    let Some(m) = render(&document(r#"resample="mitchell""#, 16.0)) else { return };
    let m = row(&m, 8);
    for x in 0..64u32 {
        let want = reference(x, 1.0 / 3.0, 1.0 / 3.0);
        assert!((m[x as usize] - want).abs() < 0.01, "x {x}: got {} want {want}", m[x as usize]);
    }
    // Mitchell does not interpolate: at the centre of texel 1 the value is (1·0 + 16·1 + 1·0)/18 of full scale
    assert!((m[24] - 16.0 / 18.0).abs() < 0.03, "{}", m[24]);
}

#[test]
fn a_minified_layer_keeps_trilinear_whatever_resample_says() {
    // the 16 x 16 checkerboard drawn at half size: no magnification, so every pixel is as without the attribute
    let Some(a) = render(&project("checker", "", 0.5)) else { return };
    let Some(b) = render(&project("checker", r#"resample="bicubic""#, 0.5)) else { return };
    let Some(c) = render(&project("checker", r#"resample="mitchell""#, 0.5)) else { return };
    assert_eq!(a.px, b.px);
    assert_eq!(a.px, c.px);
}
