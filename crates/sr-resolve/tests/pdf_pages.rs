//! SREP 17 resolve step: pdf page images and text-anchored regions, on PDFs generated here with known glyph
//! positions (Helvetica, every advance 500/1000 em, so a 10 pt character is 5 pt wide). Sizes are exact; boxes
//! are to 0.5 px.

#![cfg(feature = "pdf")]

use sr_resolve::pdf::{self, PdfError};

/// A PDF with one page per (page dictionary entries, content stream). Font /F1 is Helvetica with the standard
/// encoding and every width 500.
fn make_pdf(pages: &[(&str, &str)]) -> Vec<u8> {
    let mut objs: Vec<String> = Vec::new();
    let n = pages.len();
    // 1 catalog, 2 pages, 3 font, then (page, content) pairs
    objs.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")));
    let widths = vec!["500"; 224].join(" ");
    objs.push(format!(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 32 /LastChar 255 /Widths [{widths}] >>"
    ));
    for (i, (dict, content)) in pages.iter().enumerate() {
        objs.push(format!(
            "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R {dict} >>",
            5 + 2 * i
        ));
        objs.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
    }
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in offsets {
        out.extend(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

/// Text at (x, y) in points, 10 pt Helvetica.
fn text_at(x: f64, y: f64, s: &str) -> String {
    format!("BT /F1 10 Tf 1 0 0 1 {x} {y} Tm ({s}) Tj ET")
}

const PAGE: &str = "/MediaBox [0 0 200 100]";

#[track_caller]
fn near(got: [f64; 4], want: [f64; 4]) {
    for k in 0..4 {
        assert!((got[k] - want[k]).abs() <= 0.5, "box {got:?}, want {want:?} (±0.5 px)");
    }
}

#[test]
fn size_is_the_ceiling_of_points_times_dpi() {
    let doc = make_pdf(&[(PAGE, ""), ("/MediaBox [0 0 100 50]", "")]);
    let a = pdf::render_page(&doc, 1, 150.0, [255, 255, 255, 255], false).unwrap();
    // 200 pt · 150 / 72 = 416.67, 100 pt -> 208.33
    assert_eq!((a.width, a.height), (417, 209));
    // an A4 page at 150 dpi is 1241 x 1754, as pdftoppm makes it
    let a4 = make_pdf(&[("/MediaBox [0 0 595.276 841.89]", "")]);
    let p = pdf::render_page(&a4, 1, 150.0, [255, 255, 255, 255], false).unwrap();
    assert_eq!((p.width, p.height), (1241, 1754));
    // page selection
    let b = pdf::render_page(&doc, 2, 72.0, [255, 255, 255, 255], false).unwrap();
    assert_eq!((b.width, b.height), (100, 50));
    assert_eq!(pdf::render_page(&doc, 3, 72.0, [0; 4], false).unwrap_err(), PdfError::NoPage { page: 3, count: 2 });
    assert!(matches!(pdf::render_page(&doc, 0, 72.0, [0; 4], false), Err(PdfError::NoPage { .. })));
}

#[test]
fn rotate_and_crop_box() {
    let rotated = make_pdf(&[(&format!("{PAGE} /Rotate 90"), "")]);
    let r = pdf::render_page(&rotated, 1, 72.0, [255; 4], false).unwrap();
    assert_eq!((r.width, r.height), (100, 200));
    let cropped = make_pdf(&[(&format!("{PAGE} /CropBox [10 10 110 60]"), "")]);
    let c = pdf::render_page(&cropped, 1, 72.0, [255; 4], false).unwrap();
    assert_eq!((c.width, c.height), (100, 50));
    // a crop box larger than the media box is clipped to it
    let wide = make_pdf(&[(&format!("{PAGE} /CropBox [-50 -50 400 400]"), "")]);
    let w = pdf::render_page(&wide, 1, 72.0, [255; 4], false).unwrap();
    assert_eq!((w.width, w.height), (200, 100));
}

#[test]
fn background_and_content() {
    // an empty page is the background everywhere
    let empty = make_pdf(&[(PAGE, "")]);
    let e = pdf::render_page(&empty, 1, 72.0, [10, 200, 30, 255], false).unwrap();
    assert!(e.rgba.as_chunks::<4>().0.iter().all(|p| *p == [10, 200, 30, 255]));
    // a filled rectangle at (20, 20)-(60, 40) in PDF space (y up) is at x 20..60, y 60..80 in pixels
    let filled = make_pdf(&[(PAGE, "0 0 1 rg 20 20 40 20 re f")]);
    let f = pdf::render_page(&filled, 1, 72.0, [255, 255, 255, 255], false).unwrap();
    let px = |x: u32, y: u32| &f.rgba[((y * f.width + x) * 4) as usize..][..4];
    assert_eq!(px(40, 70), [0, 0, 255, 255]);
    assert_eq!(px(40, 50), [255, 255, 255, 255]);
    assert_eq!(px(10, 70), [255, 255, 255, 255]);
    // text draws dark pixels where it is set
    let text = make_pdf(&[(PAGE, &text_at(20.0, 50.0, "HELLO"))]);
    let t = pdf::render_page(&text, 1, 144.0, [255, 255, 255, 255], false).unwrap();
    let dark = t.rgba.as_chunks::<4>().0.iter().filter(|p| p[0] < 128).count();
    assert!(dark > 50, "{dark} dark pixels");
    // a transparent background keeps the page's alpha
    let clear = pdf::render_page(&filled, 1, 72.0, [0, 0, 0, 0], false).unwrap();
    assert_eq!(clear.rgba[((10 * clear.width + 10) * 4 + 3) as usize], 0);
}

#[test]
fn character_boxes_follow_the_glyph_positions() {
    // "Hello world" from (20, 50): character k from x = 20 + 5k; the em box from 2 pt below to 8 pt above
    // the baseline, which is y = 100 - 50 = 50 in pixels at 72 dpi
    let doc = make_pdf(&[(PAGE, &text_at(20.0, 50.0, "Hello world"))]);
    let chars = pdf::page_chars(&doc, 1, 72.0).unwrap();
    let text: String = chars.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(text, "Hello world");
    near(chars[6].rect, [50.0, 42.0, 55.0, 52.0]);
    near(pdf::find(&chars, "world", 1).unwrap(), [50.0, 42.0, 25.0, 10.0]);
    // the same at 150 dpi scales by 150/72
    let chars = pdf::page_chars(&doc, 1, 150.0).unwrap();
    let s = 150.0 / 72.0;
    near(pdf::find(&chars, "world", 1).unwrap(), [50.0 * s, 42.0 * s, 25.0 * s, 10.0 * s]);
}

#[test]
fn occurrences_and_misses() {
    let doc = make_pdf(&[(PAGE, &text_at(10.0, 50.0, "ab ab ab"))]);
    let chars = pdf::page_chars(&doc, 1, 72.0).unwrap();
    near(pdf::find(&chars, "ab", 1).unwrap(), [10.0, 42.0, 10.0, 10.0]);
    near(pdf::find(&chars, "ab", 2).unwrap(), [25.0, 42.0, 10.0, 10.0]);
    near(pdf::find(&chars, "ab", 3).unwrap(), [40.0, 42.0, 10.0, 10.0]);
    assert_eq!(
        pdf::find(&chars, "ab", 4).unwrap_err(),
        PdfError::NoMatch { text: "ab".into(), occurrence: 4, found: 3 }
    );
    assert!(matches!(pdf::find(&chars, "Ab", 1), Err(PdfError::NoMatch { .. })), "case-sensitive");
    assert!(matches!(pdf::find(&chars, "zz", 1), Err(PdfError::NoMatch { .. })));
    // matches do not overlap: "aa" in "aaa" occurs once
    let doc = make_pdf(&[(PAGE, &text_at(10.0, 50.0, "aaa"))]);
    let chars = pdf::page_chars(&doc, 1, 72.0).unwrap();
    assert!(pdf::find(&chars, "aa", 2).is_err());
}

#[test]
fn line_end_hyphens_ligatures_and_white_space() {
    // "exam-" on one line and "ple text" on the next: "example text" spans both lines
    let content = format!("{} {}", text_at(20.0, 60.0, "exam-"), text_at(20.0, 45.0, "ple text"));
    let doc = make_pdf(&[(PAGE, &content)]);
    let chars = pdf::page_chars(&doc, 1, 72.0).unwrap();
    // from x = 20 to the end of "text" (8 characters on line 2: 20 + 40 = 60); from line 1's top (100-60-8 = 32)
    // to line 2's bottom (100-45+2 = 57)
    near(pdf::find(&chars, "example text", 1).unwrap(), [20.0, 32.0, 40.0, 25.0]);
    // the phrase is normalised too: runs of white space and a line break in it
    near(pdf::find(&chars, "example\n   text", 1).unwrap(), [20.0, 32.0, 40.0, 25.0]);
    // the standard-encoding "fi" ligature (code 0xAE) reads as "fi" after NFKC
    let doc = make_pdf(&[(PAGE, &text_at(20.0, 50.0, "\\256ne print"))]);
    let chars = pdf::page_chars(&doc, 1, 72.0).unwrap();
    near(pdf::find(&chars, "fine", 1).unwrap(), [20.0, 42.0, 15.0, 10.0]);
}

#[test]
fn unreadable_input() {
    assert!(matches!(pdf::render_page(b"not a pdf", 1, 72.0, [0; 4], false), Err(PdfError::Unreadable(_))));
}

// ------------------------------------------------------------------ resolve, end to end

mod end_to_end {
    use super::*;
    use sha2::{Digest, Sha256};
    use sr_resolve::{resolve, Options, Status};
    use std::path::{Path, PathBuf};

    const ZERO: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    fn hex(b: &[u8]) -> String {
        Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
    }

    fn project(name: &str, pdf_bytes: &[u8], pinned: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sr-resolve-pdf-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("paper.pdf"), pdf_bytes).unwrap();
        let xml = format!(
            r#"<scene version="1.2">
  <project width="320" height="180" fps="10" duration="1"/>
  <assets>
    <pdf id="paper" src="paper.pdf" sha256="{pinned}" page="1" dpi="72" cache="pages/paper-1.png" cacheSha256="{ZERO}" width="1" height="1">
      <region id="w" text="world" x="0" y="0" width="0" height="0"/>
      <region id="hand" x="3" y="4" width="5" height="6"/>
    </pdf>
  </assets>
  <composition><layer id="page" asset="paper"/></composition>
</scene>
"#
        );
        std::fs::write(d.join("scene.scene.xml"), xml).unwrap();
        d
    }

    fn opts() -> Options {
        Options { store: Some(PathBuf::new()), ..Default::default() }
    }

    fn load(dir: &Path) -> sr_model::Document {
        sr_model::load_file(dir.join("scene.scene.xml"), &sr_model::LoadOptions::default())
            .unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn resolve_renders_pins_and_finds_regions() {
        let bytes = make_pdf(&[(PAGE, &text_at(20.0, 50.0, "Hello world"))]);
        let dir = project("ok", &bytes, &hex(&bytes));
        let doc_path = dir.join("scene.scene.xml");
        let rows = resolve(&doc_path, &opts()).unwrap();
        let row = rows.iter().find(|r| r.id == "paper").unwrap();
        assert_eq!(row.status, Status::Made, "{row:?}");
        // the document now validates with its assets: the cache exists and matches cacheSha256
        let doc = load(&dir);
        let Some(sr_model::model::AssetsChild::Pdf(p)) = doc.asset("paper") else { panic!("no pdf asset") };
        assert_eq!((p.width, p.height), (200, 100));
        let cache = std::fs::read(dir.join("pages/paper-1.png")).unwrap();
        assert_eq!(p.cache_sha256.to_string(), hex(&cache));
        let w = p.regions.iter().find(|g| g.id == "w").unwrap();
        near([w.x, w.y, w.width.get(), w.height.get()], [50.0, 42.0, 25.0, 10.0]);
        // regions without text are left as written
        let hand = p.regions.iter().find(|g| g.id == "hand").unwrap();
        assert_eq!([hand.x, hand.y, hand.width.get(), hand.height.get()], [3.0, 4.0, 5.0, 6.0]);
        // a second run finds everything up to date and changes nothing
        let before = std::fs::read_to_string(&doc_path).unwrap();
        let rows = resolve(&doc_path, &opts()).unwrap();
        assert_eq!(rows.iter().find(|r| r.id == "paper").unwrap().status, Status::UpToDate);
        assert_eq!(std::fs::read_to_string(&doc_path).unwrap(), before);
        // check mode reports a request that changed, without writing
        std::fs::write(&doc_path, before.replace(r#"dpi="72""#, r#"dpi="144""#)).unwrap();
        let rows = resolve(&doc_path, &Options { check: true, ..opts() }).unwrap();
        assert_eq!(rows.iter().find(|r| r.id == "paper").unwrap().status, Status::Stale);
        let rows = resolve(&doc_path, &opts()).unwrap();
        assert_eq!(rows.iter().find(|r| r.id == "paper").unwrap().status, Status::Made);
        let doc = load(&dir);
        let Some(sr_model::model::AssetsChild::Pdf(p)) = doc.asset("paper") else { panic!() };
        assert_eq!((p.width, p.height), (400, 200));
    }

    #[test]
    fn a_source_hash_mismatch_fails() {
        let bytes = make_pdf(&[(PAGE, &text_at(20.0, 50.0, "Hello world"))]);
        let dir = project("mismatch", &bytes, &hex(b"something else"));
        let rows = resolve(&dir.join("scene.scene.xml"), &opts()).unwrap();
        let row = rows.iter().find(|r| r.id == "paper").unwrap();
        assert_eq!(row.status, Status::Error);
        assert!(row.message.contains("SHA-256"), "{}", row.message);
        assert!(!dir.join("pages/paper-1.png").exists());
    }

    #[test]
    fn a_missing_phrase_fails() {
        let bytes = make_pdf(&[(PAGE, &text_at(20.0, 50.0, "Hello there"))]);
        let dir = project("missing", &bytes, &hex(&bytes));
        let rows = resolve(&dir.join("scene.scene.xml"), &opts()).unwrap();
        let row = rows.iter().find(|r| r.id == "paper").unwrap();
        assert_eq!(row.status, Status::Error);
        assert!(row.message.contains("world"), "{}", row.message);
    }
}
