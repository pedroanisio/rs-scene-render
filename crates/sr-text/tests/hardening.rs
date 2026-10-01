//! Malformed input must give an error or a best-effort result, never a panic.

use sr_text::captions;

#[test]
fn ass_unclosed_override_block_does_not_panic() {
    let head = "[Events]\nFormat: Layer, Start, End, Style, Name, Text\n";
    for tail in ["Hello {", "Hi {é", "{", "{\\k50", "a{\\k20}b{\\k30é"] {
        let src = format!("{head}Dialogue: 0,0:00:01.00,0:00:02.00,Default,,{tail}\n");
        let cues = captions::parse(&src, "ass").unwrap();
        assert_eq!(cues.len(), 1, "{tail}");
    }
    let src = format!("{head}Dialogue: 0,0:00:01.00,0:00:02.00,Default,,Hello {{\n");
    assert_eq!(captions::parse(&src, "ass").unwrap()[0].text, "Hello");
}

#[test]
fn vtt_unclosed_inline_tag_does_not_panic() {
    for body in ["<00:00:01.000>hi <", "<00:00:01.000>hi <é", "<0", "<00:00:01.000>a <00:00:01.500", "<1é"] {
        let src = format!("WEBVTT\n\n00:00:00.000 --> 00:00:02.000\n{body}\n");
        let cues = captions::parse(&src, "vtt").unwrap();
        assert_eq!(cues.len(), 1, "{body}");
    }
    let src = "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\n<00:00:01.000>hi <\n";
    let c = &captions::parse(src, "vtt").unwrap()[0];
    assert_eq!(c.words.len(), 1);
    assert_eq!((c.words[0].text.as_str(), c.words[0].start), ("hi", 1.0));
}

fn formula(tex: &str) -> Result<sr_text::Drawing, String> {
    let paint = sr_vector::Paint::Solid { rgba: [0.0, 0.0, 0.0, 1.0], srgb: true };
    sr_text::formula::draw(&mut sr_text::FontLib::new(false), tex, 20.0, [100.0, 100.0], paint, 0.1)
}

#[test]
fn formula_nesting_is_limited() {
    // a big stack, so a parser without the limit answers instead of overflowing
    let errors = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(|| ["{", "\\frac", "\\sqrt[", "x^{", "\\left(", "\\sqrt"].map(|t| formula(&t.repeat(3000)).unwrap_err()))
        .unwrap()
        .join()
        .unwrap();
    for e in errors {
        assert!(e.contains("nested"), "{e}");
    }
    assert!(formula(&"{".repeat(100_000)).unwrap_err().contains("nested"));
    // ordinary nesting parses (no math font is loaded here)
    let deep = format!("{}x{}", "{".repeat(40), "}".repeat(40));
    for tex in [deep.as_str(), "\\frac{a^{b_{c}}}{\\sqrt[3]{\\left(x\\right)}}"] {
        assert!(formula(tex).unwrap_err().contains("MATH"), "{tex}");
    }
}
