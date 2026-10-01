//! Still-image formats: every fixture decodes to the pattern it was written
//! from (`tools/fixtures/make_still_formats.py`), upright, with its colour tag.

use std::path::PathBuf;

use sr_media::still::{self, Colour};

const W: u32 = 12;
const H: u32 = 8;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/still").join(name)
}

fn pattern(x: u32, y: u32, alpha: bool) -> [u8; 4] {
    let c = |v: i32| v.clamp(0, 255) as u8;
    let (x, y) = (x as i32, y as i32);
    [c(x * 21), c(y * 33), c(200 - x * 15), if alpha { c(255 - (x + y) * 12) } else { 255 }]
}

fn expected() -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(fixture("expected.json")).unwrap()).unwrap()
}

fn ffmpeg_available() -> bool {
    std::process::Command::new(sr_media::ffmpeg()).arg("-version").output().is_ok_and(|o| o.status.success())
}

/// Whether the HEIF fixture `name` decodes here (through libheif, when `libheif`): that takes libheif's
/// `heif-convert` with its HEVC plugin, or an FFmpeg that reads HEIF items. Without, its checks are skipped
/// with a notice; `SR_REQUIRE_HEIC=1` (set in CI) runs them regardless, so a missing decoder fails there.
fn heif_decodable(name: &str, libheif: bool) -> bool {
    if std::env::var_os("SR_REQUIRE_HEIC").is_some_and(|v| v == "1") {
        return true;
    }
    let why = match still::open(&fixture(name)) {
        Ok(s) if !libheif || s.decoder == "heif-convert" => return true,
        Ok(s) => format!("decoded by {}, not libheif", s.decoder),
        Err(e) => e,
    };
    eprintln!("skipping the {name} checks (SR_REQUIRE_HEIC=1 requires them): {why}");
    false
}

#[test]
fn lossless_formats_decode_exactly() {
    let mut names = vec![
        ("pattern.png", true),
        ("pattern.webp", true),
        ("pattern.tiff", true),
        ("pattern.tga", true),
        ("pattern.qoi", true),
        ("pattern.bmp", false),
        ("pattern.ppm", false),
        ("pattern.gif", false),
        ("pattern.dds", true),
        ("pattern.ff", true),
        ("pattern.jxl", false),
    ];
    if ffmpeg_available() {
        // Formats only FFmpeg reads.
        names.extend([("pattern.psd", false), ("pattern.jp2", false)]);
        // FFmpeg alone misreads the identity-matrix AVIF: exact through libheif.
        if heif_decodable("pattern.avif", true) {
            names.push(("pattern.avif", false));
        }
    }
    for (name, alpha) in names {
        let s = still::open(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let img = s.image.to_rgba8();
        assert_eq!(img.dimensions(), (W, H), "{name}");
        for (x, y, p) in img.enumerate_pixels() {
            assert_eq!(p.0, pattern(x, y, alpha), "{name} ({}) at {x},{y}", s.decoder);
        }
        assert!(!s.linear, "{name}");
    }
}

#[test]
fn external_tools_read_what_the_native_decoders_cannot() {
    if !ffmpeg_available() {
        return;
    }
    for (name, codec) in [("pattern.psd", "psd"), ("pattern.jp2", "jpeg2000"), ("pattern.avif", "av1")] {
        let s = still::open(&fixture(name)).unwrap();
        assert!(
            (s.decoder.starts_with("ffmpeg:") && s.decoder.contains(codec))
                || (codec == "av1" && s.decoder == "heif-convert"),
            "{name}: {}",
            s.decoder
        );
    }
}

#[test]
fn linear_light_formats_keep_values_above_one() {
    for name in ["linear.hdr", "linear.exr"] {
        let s = still::open(&fixture(name)).unwrap();
        assert!(s.linear, "{name}");
        let img = s.image.to_rgba32f();
        for (x, _, p) in img.enumerate_pixels() {
            let g = 4.0 * x as f32 / (W - 1) as f32;
            // RGBE keeps 8 mantissa bits of the largest channel.
            let tol = if name.ends_with("hdr") { 0.02 * g.max(1.0) } else { 1e-6 };
            assert!((p[1] - g).abs() <= tol && (p[0] - 0.25).abs() <= tol.max(0.004), "{name} {x}: {:?}", p.0);
        }
    }
}

#[test]
fn exif_orientation_turns_photos_upright() {
    let s = still::open(&fixture("rotated.jpg")).unwrap();
    let img = s.image.to_rgb8();
    assert_eq!(img.dimensions(), (H, W));
    let e = expected();
    let mut worst = 0i32;
    for (x, y, p) in img.enumerate_pixels() {
        for c in 0..3 {
            let r = e["rotated.jpg"][y as usize][x as usize][c].as_i64().unwrap() as i32;
            worst = worst.max((p[c] as i32 - r).abs());
        }
    }
    // Different IDCTs (zune-jpeg against libjpeg) round differently.
    assert!(worst <= 3, "worst difference {worst}");
}

#[test]
fn embedded_colour_descriptions_are_reported() {
    let p3 = match still::open(&fixture("p3.png")).unwrap().colour {
        Colour::Icc(p) => p,
        other => panic!("{other:?}"),
    };
    // Display P3 red colorant (D50-adapted) and the sRGB tone curve.
    assert!((p3.to_xyz_d50[0][0] - 0.5151).abs() < 2e-3, "{:?}", p3.to_xyz_d50);
    assert!((p3.curves[0].eval(0.5) - 0.2140).abs() < 1e-3);
    assert_eq!(p3.description, "Display P3 (test)");
    match still::open(&fixture("gray.png")).unwrap().colour {
        Colour::Icc(p) => assert!(p.gray),
        other => panic!("{other:?}"),
    }
    assert_eq!(still::open(&fixture("pattern.bmp")).unwrap().colour, Colour::Unknown);
    match still::open(&fixture("cmyk.jpg")).unwrap().colour {
        Colour::Unsupported(why) => assert!(why.contains("CMYK"), "{why}"),
        other => panic!("{other:?}"),
    }
    if heif_decodable("p3-icc.heic", false) {
        match still::open(&fixture("p3-icc.heic")).unwrap().colour {
            Colour::Icc(p) => assert_eq!(p.description, "Display P3 (test)"),
            other => panic!("{other:?}"),
        }
        assert_eq!(still::open(&fixture("p3-nclx.heic")).unwrap().colour, Colour::Cicp(12, 13));
    }
}

#[test]
fn heic_pixels_match_libheif() {
    if !heif_decodable("p3-nclx.heic", false) {
        return;
    }
    let e = expected();
    for name in ["p3-icc.heic", "p3-nclx.heic"] {
        let img = still::open(&fixture(name)).unwrap().image.to_rgb8();
        assert_eq!(img.dimensions(), (W, H), "{name}");
        let mut worst = 0i32;
        for (x, y, p) in img.enumerate_pixels() {
            for c in 0..3 {
                let r = e[name][y as usize][x as usize][c].as_i64().unwrap() as i32;
                worst = worst.max((p[c] as i32 - r).abs());
            }
        }
        // Both decode the same HEVC stream; YUV → RGB rounding differs by a step or two.
        assert!(worst <= 3, "{name}: worst difference {worst}");
    }
}
