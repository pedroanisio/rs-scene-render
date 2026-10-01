//! Still images honour their embedded colour: ICC profiles are checked against
//! LittleCMS (`expected.json`, written by `tools/fixtures/make_still_formats.py`).

mod common;
use common::*;

use std::path::PathBuf;

use sr_gpu::color::Working;
use sr_gpu::resources::{self, Coding};
use sr_model::model::{AlphaMode, ColorSpace, Transfer};

fn still(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sr-media/tests/fixtures/still").join(name)
}

fn expected() -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(still("expected.json")).unwrap()).unwrap()
}

const LINEAR: Working = Working { space: ColorSpace::LinearSrgb, linear: true };

/// Whether the HEIC fixtures decode here (libheif's `heif-convert` with its HEVC plugin, or an FFmpeg that
/// reads HEIF items). Without, their checks are skipped with a notice; `SR_REQUIRE_HEIC=1` (set in CI) runs
/// them regardless, so a missing decoder fails there.
fn heic_decodable() -> bool {
    if std::env::var_os("SR_REQUIRE_HEIC").is_some_and(|v| v == "1") {
        return true;
    }
    match sr_media::still::open(&still("p3-nclx.heic")) {
        Ok(_) => true,
        Err(why) => {
            eprintln!("skipping the HEIC checks (SR_REQUIRE_HEIC=1 requires them): {why}");
            false
        }
    }
}

/// The decoded pixels as 8-bit display sRGB.
fn display(name: &str, embedded: bool) -> (u32, Vec<[f64; 3]>) {
    let d = resources::decode_image(
        &still(name),
        ColorSpace::Srgb,
        Transfer::Auto,
        AlphaMode::Auto,
        embedded,
        &LINEAR,
        4096,
    )
    .unwrap();
    let (w, _, px) = &d.levels[0];
    (
        *w,
        px.iter().map(|p| LINEAR.to_display_srgb([p[0] as f64, p[1] as f64, p[2] as f64]).map(|v| v * 255.0)).collect(),
    )
}

/// Largest difference to LittleCMS over the pixels it did not clip to the sRGB gamut.
fn worst_against_lcms(name: &str, key: &str) -> f64 {
    let e = expected();
    let (w, px) = display(name, true);
    let mut worst = 0f64;
    for (i, p) in px.iter().enumerate() {
        let want = &e[key][i / w as usize][i % w as usize];
        let want: Vec<f64> = (0..3).map(|c| want.get(c).or(Some(want)).and_then(|v| v.as_f64()).unwrap()).collect();
        if want.iter().any(|v| *v <= 0.0 || *v >= 255.0) {
            continue;
        }
        for c in 0..3 {
            worst = worst.max((p[c] - want[c]).abs());
        }
    }
    worst
}

#[test]
fn icc_profiles_match_littlecms() {
    for name in ["p3.png", "adobe.png", "table.png", "srgb-icc.png", "gray.png"] {
        let worst = worst_against_lcms(name, name);
        // LittleCMS rounds to 8 bits; the profiles' fixed-point colorants add a little more.
        assert!(worst <= 1.0, "{name}: worst difference {worst:.3} of 255");
    }
}

#[test]
fn declared_colour_space_overrides_the_profile() {
    let (w, declared) = display("p3.png", false);
    let (_, embedded) = display("p3.png", true);
    // Declared sRGB: the file values come back unchanged.
    for (i, p) in declared.iter().enumerate() {
        let (x, y) = ((i % w as usize) as f64, (i / w as usize) as f64);
        let want = [60.0 + x * 12.0, 80.0 + y * 15.0, 150.0 - x * 5.0];
        assert!((0..3).all(|c| (p[c] - want[c]).abs() < 0.51), "{i}: {p:?} {want:?}");
    }
    // Display P3 red is more saturated than sRGB red of the same code value.
    assert!(embedded[w as usize - 1][0] > declared[w as usize - 1][0] + 5.0);
}

#[test]
fn profiles_that_name_a_space_take_its_exact_path() {
    let open = |n: &str| sr_media::still::open(&still(n)).unwrap();
    let c = |n: &str| resources::coding(&open(n), ColorSpace::Srgb, Transfer::Auto, true);
    assert_eq!(c("srgb-icc.png"), Coding::Named(ColorSpace::Srgb, Transfer::Srgb));
    assert_eq!(c("p3.png"), Coding::Named(ColorSpace::DisplayP3, Transfer::Srgb));
    assert!(matches!(c("adobe.png"), Coding::Profile(..)));
    assert_eq!(c("pattern.bmp"), Coding::Named(ColorSpace::Srgb, Transfer::Auto));
    assert_eq!(c("linear.exr"), Coding::Named(ColorSpace::Srgb, Transfer::Linear));
    assert_eq!(
        resources::coding(&open("p3.png"), ColorSpace::Srgb, Transfer::Auto, false),
        Coding::Named(ColorSpace::Srgb, Transfer::Auto)
    );
    if heic_decodable() {
        assert_eq!(c("p3-nclx.heic"), Coding::Named(ColorSpace::DisplayP3, Transfer::Srgb));
        assert_eq!(c("p3-icc.heic"), Coding::Named(ColorSpace::DisplayP3, Transfer::Srgb));
        // HEVC is lossy: allow a few steps on top of LittleCMS.
        for name in ["p3-nclx.heic", "p3-icc.heic"] {
            let worst = worst_against_lcms(name, "p3-srgb-lcms");
            assert!(worst <= 4.0, "{name}: worst difference {worst:.3} of 255");
        }
    }
}

#[test]
fn image_layers_honour_the_profile_in_render() {
    let e = expected();
    for mode in ["embedded", "declared"] {
        let xml = format!(
            r#"<scene version="1.1"><project width="16" height="8" fps="10" duration="1"/>
               <assets><image id="p3" src="p3.png" width="12" height="8" colorProfile="{mode}"/></assets>
               <composition><layer id="a" asset="p3"/></composition></scene>"#
        );
        let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(still("")) };
        let d = sr_model::load_str(&xml, &opts).unwrap();
        let Some(r) = render(&d) else { return };
        let got = r.at(11, 0);
        let want: [f32; 3] = if mode == "embedded" {
            [0, 1, 2].map(|c| lin8(e["p3.png"][0][11][c].as_u64().unwrap() as u8))
        } else {
            [lin8(192), lin8(80), lin8(95)]
        };
        assert!((0..3).all(|c| (got[c] - want[c]).abs() < 0.006), "{mode}: {got:?} want {want:?}");
    }
}

#[test]
fn unusable_profiles_are_reported_and_animated_gifs_play() {
    let xml = r#"<scene version="1.1"><project width="16" height="8" fps="10" duration="1"/>
        <assets><image id="c" src="cmyk.jpg" width="12" height="8"/>
          <video id="g" src="anim.gif" width="12" height="8" fps="10" duration="0.3"/></assets>
        <composition><layer id="cmyk" asset="c"/><layer id="gif" asset="g" x="0" y="0"/></composition></scene>"#;
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(still("")) };
    let d = sr_model::load_str(xml, &opts).unwrap();
    let Some(r) = render_times(&d, &[0.15]) else { return };
    assert!(
        r.stats.unsupported.iter().any(|u| u.starts_with("cmyk:") && u.contains("CMYK")),
        "{:?}",
        r.stats.unsupported
    );
    // The GIF's second frame (100–200 ms) is green.
    assert_px(&r, 5, 4, [0.0, 1.0, 0.0, 1.0], 2e-3);
}
