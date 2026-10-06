//! SREP 21: under `fontPolicy="pinned"` faces come only from the document's font assets, by the family, weight and
//! style the assets declare; a character falls back only through the style's own families, and one no permitted face
//! has is drawn as the primary face's `.notdef`. Under either policy the layout says which characters no face has
//! (`FONT-GLYPH`) and which faces were drawn instead of what a style asked for (`FONT-SUB`).

#[path = "support/test_font.rs"]
mod test_font;
use test_font::Metrics;

use sr_text::layout::{self, Opts, Para, Run};
use sr_text::{FontLib, Style};

/// The test font, written once for every test of this file (tests run at once, and a file rewritten while another
/// test reads it would be read half-written).
fn test_font() -> std::path::PathBuf {
    static PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("sr-text-srep21-{}", std::process::id()));
        test_font::write(&dir, "srep21.ttf", "SREP Test", Metrics::default())
    })
    .clone()
}

/// A host face that has Latin-1 letters, pinned here as a document's second font asset would be.
fn latin_font() -> std::path::PathBuf {
    let candidates = ["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/TTF/DejaVuSans.ttf"];
    candidates
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())
        .expect("DejaVu Sans is installed (fonts-dejavu), as the engine's text tests require")
}

fn style(families: &[&str]) -> Style {
    Style { families: families.iter().map(|f| f.to_string()).collect(), size: 100.0, ..Default::default() }
}

fn lay(lib: &mut FontLib, text: &str, st: Style) -> sr_text::Layout {
    let para =
        Para { runs: vec![Run { text: text.into(), style: 0, role: None }], styles: vec![st], opts: Opts::default() };
    layout::layout(lib, &para)
}

#[test]
fn a_pinned_library_knows_only_the_pinned_faces() {
    let mut lib = FontLib::pinned();
    assert!(lib.is_pinned());
    assert!(lib.families().is_empty(), "no host fonts: {:?}", lib.families());
    let face = lib.pin(&test_font(), 0, "Brand", 400, false).expect("the test font pins");
    // the family is the asset's, whatever the file's name table says
    assert_eq!(lib.select(&style(&["Brand"])), Some(face));
    assert_eq!(lib.select(&style(&["brand"])), Some(face), "families match without case");
    // a family no asset declares, or a generic one, is the document's first pinned face, never a host face
    assert_eq!(lib.select(&style(&["DejaVu Sans"])), Some(face));
    assert_eq!(lib.select(&style(&["sans-serif"])), Some(face));
    assert_eq!(lib.loaded(), 1);
}

#[test]
fn the_nearest_weight_and_slant_of_the_family() {
    let mut lib = FontLib::pinned();
    let regular = lib.pin(&test_font(), 0, "Brand", 400, false).unwrap();
    let bold = lib.pin(&latin_font(), 0, "Brand", 700, false).unwrap();
    assert_eq!(lib.select(&Style { weight: 400, ..style(&["Brand"]) }), Some(regular));
    assert_eq!(lib.select(&Style { weight: 650, ..style(&["Brand"]) }), Some(bold));
}

#[test]
fn a_missing_character_is_the_primary_notdef_unless_a_fallback_family_has_it() {
    let mut lib = FontLib::pinned();
    let primary = lib.pin(&test_font(), 0, "Brand", 400, false).unwrap();
    let latin = lib.pin(&latin_font(), 0, "Latin", 400, false).unwrap();
    // é (U+00E9) is outside the test font: without a fallback family it is .notdef of the primary face
    let l = lay(&mut lib, "Hé", style(&["Brand"]));
    assert_eq!(l.missing, ['é']);
    let e = l.glyphs.iter().find(|g| l.chars[g.ch] == 'é').unwrap();
    assert_eq!((e.face, e.gid), (primary, 0), "drawn as the primary face's .notdef");
    // the pinned Latin face is not consulted unless the style names it as a fallback
    let l = lay(&mut lib, "Hé", style(&["Brand", "Latin"]));
    assert!(l.missing.is_empty());
    let e = l.glyphs.iter().find(|g| l.chars[g.ch] == 'é').unwrap();
    assert_eq!(e.face, latin);
    assert!(e.gid != 0);
}

#[test]
fn the_host_draws_what_a_system_document_lacks() {
    // under the default policy a character the document's face lacks falls back to a host face that has it
    let mut lib = FontLib::new(true);
    let path = test_font();
    lib.file(&path, 0).unwrap();
    let st = Style { file: Some((path, 0)), ..style(&["SREP Test"]) };
    let l = lay(&mut lib, "Hé", st);
    assert!(l.missing.is_empty(), "{:?}", l.missing);
    // a character no installed face has is reported
    let l = lay(&mut lib, "H\u{10FFFD}", style(&["DejaVu Sans"]));
    assert_eq!(l.missing, ['\u{10FFFD}']);
    // white space and default-ignorable characters need no glyph
    assert!(lay(&mut lib, "a b\u{200D}c\u{FE0F}", style(&["DejaVu Sans"])).missing.is_empty());
}

#[test]
fn a_face_other_than_the_one_asked_for_is_a_substitution() {
    let mut lib = FontLib::new(true);
    assert!(lay(&mut lib, "Hi", style(&["DejaVu Sans"])).substituted.is_empty());
    assert!(lay(&mut lib, "Hi", style(&["sans-serif"])).substituted.is_empty(), "a generic family names no face");
    let l = lay(&mut lib, "Hi", style(&["No Such Family 21"]));
    assert_eq!(l.substituted.len(), 1, "{:?}", l.substituted);
    assert!(l.substituted[0].requested.starts_with("No Such Family 21 400"), "{:?}", l.substituted);
    // a face from the file the style names is what it asked for
    let path = test_font();
    lib.file(&path, 0).unwrap();
    let st = Style { file: Some((path, 0)), weight: 700, ..style(&["Anything"]) };
    assert!(lay(&mut lib, "Hi", st).substituted.is_empty());
}
