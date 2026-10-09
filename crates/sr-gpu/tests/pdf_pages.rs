//! SREP 17 conformance cases on the rendered frame: a pdf asset is drawn from its pinned 200 × 100 blue cache
//! image, a cache whose digest differs is refused, and a red mark sits on its region. Boxes within 2 px.

mod common;
use common::*;

use sha2::Digest;

/// A folder with the 200 × 100 blue page image and its SHA-256.
fn page_dir() -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("sr-gpu-pdf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let img = image::RgbaImage::from_pixel(200, 100, image::Rgba([0, 0, 255, 255]));
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    std::fs::write(dir.join("page.png"), &png).unwrap();
    let sha = sha2::Sha256::digest(&png).iter().map(|b| format!("{b:02x}")).collect();
    (dir, sha)
}

fn scene(cache_sha: &str, body: &str) -> Result<sr_model::Document, sr_model::LoadError> {
    let (dir, _) = page_dir();
    let zero = "0".repeat(64);
    let xml = format!(
        r##"<scene version="1.2"><project width="640" height="360" fps="24" duration="1" background="#000000FF"/><assets><pdf id="paper" src="absent.pdf" sha256="{zero}" cache="page.png" cacheSha256="{cache_sha}" width="200" height="100"><region id="g" x="20" y="10" width="60" height="20"/></pdf></assets><composition>{body}</composition></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions { verify_assets: true, base_dir: Some(dir) })
}

/// Bounding box of the pixels where `pick` holds.
fn box_of(r: &Rendered, pick: impl Fn([f32; 4]) -> bool) -> Option<[u32; 4]> {
    let mut b = [u32::MAX, u32::MAX, 0, 0];
    for y in 0..r.size[1] {
        for x in 0..r.size[0] {
            if pick(r.at(x, y)) {
                b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
            }
        }
    }
    (b[0] != u32::MAX).then_some(b)
}

#[track_caller]
fn near(got: Option<[u32; 4]>, want: [f64; 4]) {
    let g = got.expect("drawn");
    for k in 0..4 {
        assert!((g[k] as f64 - want[k]).abs() <= 2.0, "box {g:?}, want {want:?} (±2)");
    }
}

fn blue(p: [f32; 4]) -> bool {
    p[2] > 0.5 && p[0] < 0.5 && p[1] < 0.5
}

fn red(p: [f32; 4]) -> bool {
    p[0] > 0.5 && p[1] < 0.5 && p[2] < 0.5
}

#[test]
fn pdf_page() {
    let (_, sha) = page_dir();
    let d = scene(&sha, r#"<layer id="L" asset="paper" x="100" y="50" scaleX="2" scaleY="2"/>"#).unwrap();
    let Some(r) = render(&d) else { return };
    // centre (300, 150), 400 x 200
    near(box_of(&r, blue), [100.0, 50.0, 500.0, 250.0]);
}

#[test]
fn pdf_bad_cache() {
    let d = scene(&"0".repeat(64), r#"<layer id="L" asset="paper"/>"#);
    match d {
        Err(sr_model::LoadError::Invalid(r)) => assert!(r.diagnostics.iter().any(|d| d.code == "A02"), "{r}"),
        Err(e) => panic!("{e:?}"),
        Ok(_) => panic!("a cache whose SHA-256 differs from cacheSha256 must be refused"),
    }
}

#[test]
fn pdf_region() {
    let (_, sha) = page_dir();
    let d = scene(
        &sha,
        r##"<layer id="L" asset="paper" x="100" y="50" scaleX="2" scaleY="2"/><shape id="s" shape="rect" region="g" regionLayer="L" regionPadding="5" fill="#FF0000FF"/>"##,
    )
    .unwrap();
    let Some(r) = render(&d) else { return };
    // centre (200, 90), 140 x 60
    near(box_of(&r, red), [130.0, 60.0, 270.0, 120.0]);
}

#[test]
fn pdf_region_fit() {
    let (_, sha) = page_dir();
    let d = scene(
        &sha,
        r##"<layer id="L" asset="paper" boxWidth="100" boxHeight="100" fit="contain"/><shape id="s" shape="rect" region="g" regionLayer="L" fill="#FF0000FF"/>"##,
    )
    .unwrap();
    let Some(r) = render(&d) else { return };
    // centre (25, 35), 30 x 10
    near(box_of(&r, red), [10.0, 30.0, 40.0, 40.0]);
}

#[test]
fn pdf_region_rotate() {
    let (_, sha) = page_dir();
    let d = scene(
        &sha,
        r##"<layer id="L" asset="paper" x="300" y="50" rotation="90"/><shape id="s" shape="rect" region="g" regionLayer="L" fill="#FF0000FF"/>"##,
    )
    .unwrap();
    let Some(r) = render(&d) else { return };
    // centre (280, 100), 20 x 60
    near(box_of(&r, red), [270.0, 70.0, 290.0, 130.0]);
}
