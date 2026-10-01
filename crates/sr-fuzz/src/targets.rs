//! Byte-level targets: the parsers that read files a document points at.
//!
//! Each [`Target`] takes arbitrary bytes and must never panic: captions
//! (SRT, WebVTT, ASS, TTML, SCC, transcripts), Lottie JSON and dotLottie
//! archives, SVG and path data, formulas, text layout, PMTiles, vector
//! tiles, GeoJSON/TopoJSON/KML/GPX, PLY and splats, USD, glTF, and the
//! document pipeline with asset verification on. Inputs are built-in seeds
//! and the repository's fixtures, mutated by bit flips, boundary bytes and
//! integers, span deletion and duplication, splices between seeds, inserted
//! number tokens and truncation; text formats also get the document
//! mutations of [`crate::mutate_doc`].

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use sr_text::layout::{self, Align, AutoFit, Dir, Opts, Overflow, Para, Run, Wrap, Writing};
use sr_vector::Paint;

use crate::Rng;

/// Inputs are cut to this length, so a campaign's speed does not drift with duplicated spans.
pub const MAX_INPUT: usize = 64 << 10;

/// A parser under test.
pub struct Target {
    pub name: &'static str,
    /// Text formats also take the document mutations.
    pub text: bool,
    pub seeds: fn() -> Vec<Vec<u8>>,
    pub run: fn(&[u8]),
}

/// Every byte-level target.
pub const TARGETS: &[Target] = &[
    Target { name: "captions", text: true, seeds: seeds_captions, run: run_captions },
    Target { name: "lottie", text: true, seeds: seeds_lottie, run: run_lottie },
    Target { name: "dotlottie", text: false, seeds: seeds_dotlottie, run: run_lottie },
    Target { name: "svg", text: true, seeds: seeds_svg, run: run_svg },
    Target { name: "path", text: true, seeds: seeds_path, run: run_path },
    Target { name: "formula", text: true, seeds: seeds_formula, run: run_formula },
    Target { name: "text", text: true, seeds: seeds_text, run: run_text },
    Target { name: "pmtiles", text: false, seeds: seeds_pmtiles, run: run_pmtiles },
    Target { name: "mvt", text: false, seeds: seeds_mvt, run: run_mvt },
    Target { name: "geodata", text: true, seeds: seeds_geodata, run: run_geodata },
    Target { name: "ply", text: false, seeds: seeds_ply, run: run_ply },
    Target { name: "usd", text: false, seeds: seeds_usd, run: run_usd },
    Target { name: "gltf", text: false, seeds: seeds_gltf, run: run_gltf },
    Target { name: "document-assets", text: true, seeds: seeds_documents, run: run_doc_assets },
];

/// The target called `name`.
pub fn target(name: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|t| t.name == name)
}

fn repo(rel: &str) -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join(rel)
}

/// Built-in seeds plus the repository files that exist and fit [`MAX_INPUT`].
fn with_files(built_in: &[&[u8]], files: &[&str]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = built_in.iter().map(|s| s.to_vec()).collect();
    out.extend(files.iter().filter_map(|f| std::fs::read(repo(f)).ok()).filter(|b| b.len() <= MAX_INPUT));
    out
}

// ------------------------------------------------------------------ captions

const SRT: &str = "1\n00:00:01,000 --> 00:00:02,500\n<i>Hello</i> &amp; world\nsecond line\n\n2\n00:00:03,000 --> 00:00:04,000 X1:10\nBye\n";
const VTT: &str = "WEBVTT\n\nintro\n00:00.000 --> 00:02.000 line:90% align:center\n<v Ana>Hello <00:00:00.500>there <00:00:01.000><b>you</b>\n\n00:00:02.000 --> 00:00:03.000\nplain &lt;text&gt;\n";
const ASS: &str = "[Script Info]\nTitle: t\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:03.00,Default,Ana,0,0,0,,{\\k50}Hel{\\kf30}lo {\\ko20\\b1}wor{\\i1}ld\\Nnext, line\nDialogue: 0,0:00:03.00,0:00:04.00,Top,,0,0,0,,{\\an8}plain\n";
const TTML: &str = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter" ttp:frameRate="24" ttp:frameRateMultiplier="1000 1001" ttp:tickRate="10000000"><body><div><p begin="00:00:01:12" end="2s" style="s1" agent="ana">Hello<br/><span begin="0.5s" end="1s">timed</span> text</p><p begin="3000ms" dur="48f">Later</p><p begin="50000000t">Ticks</p></div></body></tt>"#;
const SCC: &str = "Scenarist_SCC V1.0\n\n00:00:01:00\t9420 9420 94ae 94ae 9452 9452 c8e5 ecec ef20 f7ef f2ec 6480 942c 942c 942f 942f\n\n00:00:03;15\t9425 9425 94ad 94ad c1b0 1130 9137 942c\n";
const TRANSCRIPT: &str = r#"{"segments":[{"start":0,"end":1.5,"text":" Hello there","speaker":"a","words":[{"start":0,"end":0.5,"word":" Hello"},{"start":0.5,"end":1.5,"text":"there","emphasis":true}]}],"words":[{"start":0,"end":1,"text":"x"}]}"#;

fn seeds_captions() -> Vec<Vec<u8>> {
    with_files(
        &[SRT.as_bytes(), VTT.as_bytes(), ASS.as_bytes(), TTML.as_bytes(), SCC.as_bytes(), TRANSCRIPT.as_bytes()],
        &["tests/corpus/media/transcript.vtt"],
    )
}

/// Every caption parser on `data`, then the cue consumers.
pub fn run_captions(data: &[u8]) {
    use sr_text::captions;
    let text = String::from_utf8_lossy(data);
    let mut all = Vec::new();
    for format in ["srt", "vtt", "ass", "ttml", "itt", "scc"] {
        if let Ok(cues) = captions::parse(&text, format) {
            all.extend(cues);
        }
    }
    if let Ok(cues) = captions::from_transcript(&text) {
        all.extend(cues);
    }
    all.truncate(64);
    for c in &all {
        let _ = captions::timed_words(c);
        let _ = captions::profanity(&c.text);
    }
    let _ = captions::paginate(&all, Some(3), 16, 2, false);
    let _ = captions::paginate(&all, None, 1, 1, true);
    let _ = captions::to_srt(&all);
    let _ = captions::to_vtt(&all);
}

// ------------------------------------------------------------------ Lottie

const LOTTIE: &str = r##"{"v":"5.7.0","fr":30,"ip":0,"op":60,"w":100,"h":100,
"markers":[{"cm":"intro","tm":10,"dr":20}],
"slots":{"c":{"p":{"a":0,"k":[0,0,1,1]}}},
"assets":[{"id":"pre","layers":[{"ty":1,"ind":1,"ip":0,"op":60,"sw":20,"sh":20,"sc":"#ff8800","ks":{}},{"ty":0,"refId":"pre2","ip":0,"op":60,"w":20,"h":20,"ks":{}}]},{"id":"pre2","layers":[{"ty":0,"refId":"pre","ip":0,"op":60,"ks":{}}]}],
"layers":[
{"ty":4,"ind":2,"parent":1,"ip":0,"op":60,"st":0,"ks":{"p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}},
 "masksProperties":[{"mode":"a","inv":false,"o":{"a":0,"k":100},"pt":{"a":0,"k":{"c":true,"v":[[0,0],[50,0],[50,50]],"i":[[0,0],[0,0],[0,0]],"o":[[0,0],[0,0],[0,0]]}}}],
 "shapes":[{"ty":"gr","it":[
  {"ty":"rc","p":{"a":0,"k":[10,10]},"s":{"a":0,"k":[20,20]},"r":{"a":0,"k":2}},
  {"ty":"el","p":{"a":0,"k":[40,40]},"s":{"a":0,"k":[20,10]}},
  {"ty":"sr","sy":1,"p":{"a":0,"k":[60,60]},"pt":{"a":0,"k":5},"r":{"a":0,"k":0},"or":{"a":0,"k":10},"ir":{"a":0,"k":4},"os":{"a":0,"k":0},"is":{"a":0,"k":0}},
  {"ty":"sh","ks":{"a":1,"k":[{"t":0,"s":[{"c":false,"v":[[0,0],[30,0]],"i":[[0,0],[0,0]],"o":[[0,0],[0,0]]}],"o":{"x":0.3,"y":0},"i":{"x":0.7,"y":1}},{"t":30,"s":[{"c":false,"v":[[0,0],[30,30]],"i":[[0,0],[0,0]],"o":[[0,0],[0,0]]}]}]}},
  {"ty":"tm","s":{"a":0,"k":10},"e":{"a":0,"k":90},"o":{"a":0,"k":45},"m":2},
  {"ty":"rd","r":{"a":0,"k":3}},
  {"ty":"zz","s":{"a":0,"k":2},"r":{"a":0,"k":3},"pt":{"a":0,"k":2}},
  {"ty":"pb","a":{"a":0,"k":10}},
  {"ty":"tw","a":{"a":0,"k":30}},
  {"ty":"op","a":{"a":0,"k":2},"lj":2,"ml":{"a":0,"k":4}},
  {"ty":"rp","c":{"a":0,"k":3},"o":{"a":0,"k":0},"m":1,"tr":{"p":{"a":0,"k":[5,0]},"s":{"a":0,"k":[90,90]},"r":{"a":0,"k":10},"so":{"a":0,"k":100},"eo":{"a":0,"k":50}}},
  {"ty":"mm","mm":3},
  {"ty":"fl","c":{"sid":"c","a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100},"r":2},
  {"ty":"st","c":{"a":0,"k":[0,0,0,1]},"o":{"a":0,"k":100},"w":{"a":0,"k":2},"lc":2,"lj":2,"ml":4,"d":[{"n":"d","v":{"a":0,"k":4}},{"n":"g","v":{"a":0,"k":2}},{"n":"o","v":{"a":0,"k":1}}]},
  {"ty":"gf","o":{"a":0,"k":100},"s":{"a":0,"k":[0,0]},"e":{"a":0,"k":[100,0]},"t":2,"h":{"a":0,"k":50},"a":{"a":0,"k":0},"g":{"p":2,"k":{"a":0,"k":[0,1,0,0,1,0,0,1,0,1,1,0.5]}}},
  {"ty":"gs","o":{"a":0,"k":100},"w":{"a":0,"k":1},"s":{"a":0,"k":[0,0]},"e":{"a":0,"k":[100,0]},"t":1,"g":{"p":2,"k":{"a":0,"k":[0,1,0,0,1,0,0,1]}}},
  {"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100},"sk":{"a":0,"k":5},"sa":{"a":0,"k":10}}]}]},
{"ty":3,"ind":1,"ip":0,"op":60,"ks":{"p":{"s":true,"x":{"a":0,"k":5},"y":{"a":1,"k":[{"t":0,"s":[0],"h":1},{"t":30,"s":[60],"to":[1,1],"ti":[2,2]},{"t":50}]}}}},
{"ty":0,"ind":4,"refId":"pre","ip":0,"op":60,"st":5,"sr":2,"w":50,"h":50,"tt":1,"tp":2,"tm":{"a":0,"k":0.5},"ks":{"o":{"a":0,"k":50}}},
{"ty":1,"ind":5,"td":1,"ip":0,"op":60,"sw":100,"sh":100,"sc":"#00ff00","ks":{}}]}"##;

fn seeds_lottie() -> Vec<Vec<u8>> {
    with_files(&[LOTTIE.as_bytes()], &["tests/corpus/media/intro.json"])
}

/// A zip of stored entries.
fn zip_stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data) in files {
        let off = out.len() as u32;
        let sizes = |v: &mut Vec<u8>| {
            v.extend([0u8; 4]);
            v.extend((data.len() as u32).to_le_bytes());
            v.extend((data.len() as u32).to_le_bytes());
            v.extend((name.len() as u16).to_le_bytes());
        };
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend([20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        sizes(&mut out);
        out.extend([0, 0]);
        out.extend(name.as_bytes());
        out.extend(*data);
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend([20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        sizes(&mut central);
        central.extend([0u8; 12]);
        central.extend(off.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let cd = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0, 0, 0, 0]);
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(cd.to_le_bytes());
    out.extend([0, 0]);
    out
}

fn seeds_dotlottie() -> Vec<Vec<u8>> {
    let small = br##"{"fr":30,"ip":0,"op":10,"w":10,"h":10,"layers":[{"ty":1,"ip":0,"op":10,"sw":5,"sh":5,"sc":"#102030","ks":{}}]}"##;
    vec![
        zip_stored(&[("manifest.json", b"{}"), ("animations/a.json", LOTTIE.as_bytes())]),
        zip_stored(&[("a/one.json", small), ("a/two.json", small), ("images/x.png", b"\x89PNG")]),
    ]
}

/// Parses Lottie JSON or a dotLottie archive and renders three frames.
pub fn run_lottie(data: &[u8]) {
    use sr_vector::lottie::Lottie;
    let _ = Lottie::parse(data, Some("a"), &[]);
    let overrides = [("c".to_string(), "#ff000080".to_string()), ("d".to_string(), "1, 2".to_string())];
    if let Ok(l) = Lottie::parse(data, None, &overrides) {
        let _ = l.segment("intro");
        let _ = l.segment("1,2");
        for f in [l.ip, l.frame_at(0.5, None), l.op - 1.0] {
            let _ = l.render(f, 0.5);
        }
    }
}

// ------------------------------------------------------------------ SVG and paths

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="50" viewBox="0 0 200 100">
<defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="200" y2="0"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f" stop-opacity="0.5"/></linearGradient>
<radialGradient id="r" cx="50%" cy="50%" r="50%" fx="30%"><stop offset="0" stop-color="white"/><stop offset="1" stop-color="black"/></radialGradient>
<clipPath id="c"><rect x="0" y="0" width="100" height="100" rx="5"/></clipPath><mask id="m"><circle cx="50" cy="50" r="40" fill="#fff"/></mask>
<symbol id="s" viewBox="0 0 10 10"><path d="M0 0h10v10z"/></symbol></defs>
<rect x="0" y="0" width="200" height="100" fill="url(#r)"/>
<g clip-path="url(#c)" opacity="0.5" transform="translate(10 5) rotate(15) scale(1.2, 0.8) skewX(5)"><ellipse cx="50" cy="50" rx="30" ry="20" fill="#ff0000" mask="url(#m)"/></g>
<path d="M10 10 L190 10 C150 40 100 40 60 90 Q30 60 10 90 A20 30 45 1 0 80 80 Z" fill="none" stroke="url(#g)" stroke-width="4" stroke-dasharray="10 5" stroke-dashoffset="3" stroke-linecap="round" stroke-linejoin="bevel"/>
<polygon points="10,10 20,30 30,10" fill-rule="evenodd"/><polyline points="1 2 3 4 5 6"/><line x1="0" y1="0" x2="5" y2="5" stroke="blue"/>
<use xlink:href="#s" x="20" y="20" width="30" height="30"/><text x="10" y="20" font-size="12">Hi</text></svg>"##;

fn seeds_svg() -> Vec<Vec<u8>> {
    vec![SVG.as_bytes().to_vec(), br#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0L1 1"/></svg>"#.to_vec()]
}

/// Loads an SVG document.
pub fn run_svg(data: &[u8]) {
    let _ = sr_vector::svg::load(data, 0.5);
}

fn seeds_path() -> Vec<Vec<u8>> {
    [
        "M10 10 L190 10 C150 40 100 40 60 90 S20 20 5 5 Q30 60 10 90 T50 50 A20 30 45 1 0 80 80 Z",
        "m1,2 l3-4 h5 v-6 c1 2 3 4 5 6 s1 2 3 4 q1 2 3 4 t5 6 a1 2 3 0 1 4 5 z m0 0",
        "M.5.5 1e2-3E-1 L 1e308 -1e308 A0 0 0 0 0 1 1 a1,1,0,11,2,2",
        "M0 0 H10 V10 H0 Z M2 2 L8 2 8 8 2 8 z",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect()
}

/// Parses path data and runs the path operations on it.
pub fn run_path(data: &[u8]) {
    use sr_vector::geom::Xf;
    if let Ok(p) = sr_vector::Path::parse(&String::from_utf8_lossy(data)) {
        let _ = p.bounds();
        let cs = p.contours();
        let _ = sr_vector::Path::from_contours(&cs);
        let polys = p.transform(&Xf::rotate(30.0)).flatten(0.5);
        let _ = sr_vector::measure::dash(&polys, &[3.0, 1.0], 0.5);
        let _ = sr_vector::measure::trim(&polys, 0.1, 0.7, 0.3, sr_vector::measure::TrimMode::Sequential);
    }
}

// ------------------------------------------------------------------ text

fn fonts() -> MutexGuard<'static, sr_text::FontLib> {
    static LIB: OnceLock<Mutex<sr_text::FontLib>> = OnceLock::new();
    LIB.get_or_init(|| Mutex::new(sr_text::FontLib::new(true))).lock().unwrap_or_else(|e| e.into_inner())
}

fn seeds_formula() -> Vec<Vec<u8>> {
    [
        r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
        r"\sum_{i=1}^{n} i^2 = \left( \frac{n(n+1)(2n+1)}{6} \right) \quad \int_0^\infty e^{-x}\,dx",
        r"\sqrt[3]{x_1^2 + \alpha'} \leq \lim_{n \to \infty} \text{rate} \cdot \mathrm{d}t \left\{ a \right.",
        r"a^{b^{c_d}} \{ x \} \langle y \rangle \; \! \sin\theta",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect()
}

/// Typesets a formula (parsing only when no MATH font is installed).
pub fn run_formula(data: &[u8]) {
    let tex = String::from_utf8_lossy(data);
    let paint = Paint::Solid { rgba: [0.0, 0.0, 0.0, 1.0], srgb: true };
    let _ = sr_text::formula::draw(&mut fonts(), &tex, 24.0, [200.0, 100.0], paint, 0.5);
}

fn seeds_text() -> Vec<Vec<u8>> {
    [
        "\u{1}\u{2}\u{3}\u{4}Hello world, extraordinarily hyphenatable incomprehensibilities again and again",
        "\u{13}\u{21}\u{32}\u{40}مرحبا بالعالم שלום עולם mixed ‏RTL‎ and LTR 123",
        "\u{25}\u{16}\u{07}\u{38}日本語のテキスト、縦書き。😀👨‍👩‍👧 e\u{301} İstanbul\u{ad}soft\u{200b}zero\ttab\nline\r\nbreak",
        "\u{ff}\u{80}\u{7f}\u{0}ﬁ ﬂ office AVATAR To. 1/2 --- x²",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect()
}

/// Lays out and draws text; the first four bytes pick the box and flow options.
pub fn run_text(data: &[u8]) {
    let (head, body) = data.split_at(data.len().min(4));
    let b = |k: usize| head.get(k).copied().unwrap_or(0) as usize;
    let text: String = String::from_utf8_lossy(body).chars().take(400).collect();
    let opts = Opts {
        width: [f64::INFINITY, 200.0, 40.0, 1.0, 0.0, 1e6][b(0) % 6],
        height: [f64::INFINITY, 100.0, 10.0, 0.0][b(0) / 6 % 4],
        align: [Align::Start, Align::Center, Align::End, Align::Justify][b(1) % 4],
        direction: [Dir::Auto, Dir::Ltr, Dir::Rtl][b(1) / 4 % 3],
        writing: [Writing::Horizontal, Writing::VerticalRl, Writing::VerticalLr][b(1) / 12 % 3],
        wrap: [Wrap::Word, Wrap::Character, Wrap::None, Wrap::Balance][b(2) % 4],
        hyphenate: b(2) / 4 % 2 == 1,
        auto_fit: [AutoFit::None, AutoFit::Shrink, AutoFit::Grow, AutoFit::Fit][b(2) / 8 % 4],
        max_lines: [None, Some(1), Some(3)][b(2) / 32 % 3],
        overflow: [Overflow::Visible, Overflow::Ellipsis, Overflow::Clip][b(3) % 3],
        letter_spacing: [0.0, 2.0, -1.0][b(3) / 3 % 3],
        line_height: [1.2, 0.5, 3.0][b(3) / 9 % 3],
        ..Default::default()
    };
    let mid = text.char_indices().nth(text.chars().count() / 2).map_or(0, |c| c.0);
    let style = |size: f64, italic: bool| sr_text::Style { size, italic, ..Default::default() };
    let para = Para {
        runs: vec![
            Run { text: text[..mid].to_string(), style: 0, role: None },
            Run { text: text[mid..].to_string(), style: 1, role: Some("em".into()) },
        ],
        styles: vec![style(24.0, false), style([8.0, 64.0, 0.5][b(3) / 27 % 3], true)],
        opts,
    };
    let mut lib = fonts();
    let lay = layout::layout(&mut lib, &para);
    let _ = sr_text::glyph::draw(&lib, &lay, None, &Default::default(), 0.5);
}

// ------------------------------------------------------------------ maps

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn pb_bytes(out: &mut Vec<u8>, field: u64, b: &[u8]) {
    varint(out, field << 3 | 2);
    varint(out, b.len() as u64);
    out.extend(b);
}

fn pb_varint(out: &mut Vec<u8>, field: u64, v: u64) {
    varint(out, field << 3);
    varint(out, v);
}

/// A vector tile: one layer with a line and a polygon, two keys and typed values.
fn mvt_tile() -> Vec<u8> {
    let feature = |id: u64, kind: u64, tags: &[u8], geom: &[u8]| {
        let mut f = Vec::new();
        pb_varint(&mut f, 1, id);
        pb_bytes(&mut f, 2, tags);
        pb_varint(&mut f, 3, kind);
        pb_bytes(&mut f, 4, geom);
        f
    };
    let mut layer = Vec::new();
    pb_varint(&mut layer, 15, 2);
    pb_bytes(&mut layer, 1, b"roads");
    pb_bytes(&mut layer, 2, &feature(1, 2, &[0, 0, 1, 1], &[9, 4, 4, 18, 10, 0, 0, 10]));
    pb_bytes(&mut layer, 2, &feature(2, 3, &[0, 2], &[9, 0, 0, 26, 20, 0, 0, 20, 19, 0, 15]));
    pb_bytes(&mut layer, 2, &feature(3, 1, &[1, 3], &[17, 2, 2, 6, 6]));
    pb_bytes(&mut layer, 3, b"kind");
    pb_bytes(&mut layer, 3, b"lanes");
    pb_bytes(&mut layer, 4, &[10, 4, b'r', b'o', b'a', b'd']);
    pb_bytes(&mut layer, 4, &[40, 2]);
    pb_bytes(&mut layer, 4, &[25, 0, 0, 0, 0, 0, 0, 0xf0, 0x3f]);
    pb_bytes(&mut layer, 4, &[56, 1]);
    pb_varint(&mut layer, 5, 4096);
    let mut tile = Vec::new();
    pb_bytes(&mut tile, 3, &layer);
    tile
}

fn seeds_mvt() -> Vec<Vec<u8>> {
    vec![mvt_tile()]
}

/// Decodes a vector tile and groups its polygons.
pub fn run_mvt(data: &[u8]) {
    if let Ok(layers) = sr_geo::mvt::decode(data) {
        for f in layers.iter().flat_map(|l| &l.features) {
            let _ = f.polygons();
        }
    }
}

fn seeds_pmtiles() -> Vec<Vec<u8>> {
    use sr_geo::pmtiles::{write, TileType};
    let tile = mvt_tile();
    let tiles = [((0, 0, 0), tile.clone()), ((1, 0, 0), tile.clone()), ((1, 1, 0), tile.clone()), ((1, 1, 1), vec![1])];
    let mut out = vec![write(&tiles, TileType::Mvt, 1, &serde_json::json!({"vector_layers": [{"id": "roads"}]}))];
    out.extend(with_files(&[], &["tests/corpus/media/streets.pmtiles", "crates/sr-geo/tests/fixtures/raster.pmtiles"]));
    out
}

/// A file holding one input, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    /// Writes `data` under `$SR_FUZZ_SCRATCH`, or the workspace's `target/sr-fuzz`.
    fn new(name: &str, data: &[u8]) -> Option<Scratch> {
        let dir = std::env::var_os("SR_FUZZ_SCRATCH").map(PathBuf::from).unwrap_or_else(|| repo("target/sr-fuzz"));
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join(format!("{}-{name}", std::process::id()));
        std::fs::write(&path, data).ok()?;
        Some(Scratch(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Reads a PMTiles header and directory from memory, then opens the bytes as an archive.
pub fn run_pmtiles(data: &[u8]) {
    use sr_geo::pmtiles;
    let _ = pmtiles::parse_directory(data);
    let _ = pmtiles::decompress(2, data.to_vec());
    let Ok(h) = pmtiles::Header::parse(data) else { return };
    let Some(file) = Scratch::new("in.pmtiles", data) else { return };
    if let Ok(a) = pmtiles::Archive::open(&file.0) {
        for z in [0, h.min_zoom.min(20), h.max_zoom.min(20)] {
            let n = 1u32 << z;
            for (x, y) in [(0, 0), (n / 2, n / 2), (n - 1, n - 1)] {
                if let Ok(Some(t)) = a.tile(z, x, y) {
                    run_mvt(&t);
                }
            }
        }
    }
}

const GEOJSON: &str = r#"{"type":"FeatureCollection","features":[
{"type":"Feature","id":7,"properties":{"name":"a","n":1.5,"b":true,"z":null},"geometry":{"type":"Polygon","coordinates":[[[0,0],[10,0],[10,10],[0,10],[0,0]],[[2,2],[2,4],[4,4],[2,2]]]}},
{"type":"Feature","properties":{},"geometry":{"type":"MultiLineString","coordinates":[[[170,0],[-170,5]],[[0,89],[0,-89]]]}},
{"type":"Feature","properties":null,"geometry":{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[1,2,3]},{"type":"MultiPoint","coordinates":[[1,2],[3,4]]},{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]]]},{"type":"LineString","coordinates":[[0,0],[1,1]]}]}},
{"type":"Feature","geometry":null}]}"#;
const TOPOJSON: &str = r#"{"type":"Topology","transform":{"scale":[0.01,0.01],"translate":[-10,-10]},
"objects":{"land":{"type":"GeometryCollection","geometries":[{"type":"Polygon","id":"p","properties":{"name":"x"},"arcs":[[0,-2]]},{"type":"MultiPolygon","arcs":[[[0],[1]]]},{"type":"LineString","arcs":[0,1]},{"type":"MultiLineString","arcs":[[-1]]},{"type":"Point","coordinates":[100,200]},{"type":"MultiPoint","coordinates":[[1,2]]}]},"one":{"type":"Polygon","arcs":[[1]]}},
"arcs":[[[0,0],[1000,0],[0,1000],[-1000,0],[0,-1000]],[[200,200],[100,0],[0,100],[-100,-100]]]}"#;
const KML: &str = r#"<kml xmlns="http://www.opengis.net/kml/2.2"><Document><Folder><Placemark id="a"><name>Spot</name><description>d</description><ExtendedData><Data name="k"><value>v</value></Data></ExtendedData><Point><coordinates>-9.1,38.7,0</coordinates></Point></Placemark>
<Placemark><name>Area</name><MultiGeometry><Polygon><outerBoundaryIs><LinearRing><coordinates>0,0 10,0 10,10 0,0</coordinates></LinearRing></outerBoundaryIs><innerBoundaryIs><LinearRing><coordinates>2,1 3,1 3,2 2,1</coordinates></LinearRing></innerBoundaryIs></Polygon><LineString><coordinates>0,0,5
1,1,5</coordinates></LineString></MultiGeometry></Placemark></Folder></Document></kml>"#;
const GPX: &str = r#"<gpx version="1.1" xmlns="http://www.topografix.com/GPX/1/1"><wpt lat="38.7" lon="-9.1"><ele>10</ele><name>W</name></wpt><rte><name>R</name><rtept lat="1" lon="2"/><rtept lat="3" lon="4"/></rte><trk><name>T</name><trkseg><trkpt lat="38.7" lon="-9.1"><ele>5</ele><time>2020-01-01T00:00:00Z</time></trkpt><trkpt lat="38.8" lon="-9.2"/></trkseg><trkseg><trkpt lat="0" lon="179.9"/><trkpt lat="0" lon="-179.9"/></trkseg></trk></gpx>"#;

fn seeds_geodata() -> Vec<Vec<u8>> {
    with_files(
        &[GEOJSON.as_bytes(), TOPOJSON.as_bytes(), KML.as_bytes(), GPX.as_bytes()],
        &["tests/corpus/media/places.geojson"],
    )
}

/// Reads `data` as each of GeoJSON, TopoJSON, KML and GPX.
pub fn run_geodata(data: &[u8]) {
    use sr_geo::data::{parse, Format};
    let text = String::from_utf8_lossy(data);
    for format in [Format::GeoJson, Format::TopoJson, Format::Kml, Format::Gpx] {
        let _ = parse(&text, format, None);
        let _ = parse(&text, format, Some("land"));
    }
}

// ------------------------------------------------------------------ 3D

fn seeds_ply() -> Vec<Vec<u8>> {
    let ascii = b"ply\nformat ascii 1.0\ncomment made by hand\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0 255 0 0\n1 0 0 0 255 0\n0 1 0 0 0 255\n3 0 1 2\n".to_vec();
    let floats = |v: &[f32]| v.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>();
    let mut binary = b"ply\nformat binary_little_endian 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty float nx\nproperty float ny\nproperty float nz\nproperty float s\nproperty float t\nelement face 1\nproperty list uchar uint vertex_indices\nend_header\n".to_vec();
    binary.extend(floats(&[0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
    binary.extend(floats(&[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0]));
    binary.extend(floats(&[0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0]));
    binary.push(3);
    binary.extend([0u32, 1, 2].iter().flat_map(|i| i.to_le_bytes()));
    let mut big = b"ply\nformat binary_big_endian 1.0\nelement vertex 1\nproperty double x\nproperty double y\nproperty double z\nproperty short q\nend_header\n".to_vec();
    big.extend([1.0f64, 2.0, 3.0].iter().flat_map(|f| f.to_be_bytes()));
    big.extend([0, 7]);
    let names = [
        "x", "y", "z", "f_dc_0", "f_dc_1", "f_dc_2", "opacity", "scale_0", "scale_1", "scale_2", "rot_0", "rot_1",
        "rot_2", "rot_3",
    ];
    let mut gauss = b"ply\nformat binary_little_endian 1.0\nelement vertex 2\n".to_vec();
    for n in names {
        gauss.extend(format!("property float {n}\n").bytes());
    }
    gauss.extend(b"end_header\n");
    for k in 0..2 {
        gauss.extend(floats(&[k as f32, 0.5, -0.5, 0.1, 0.2, 0.3, 2.0, -1.0, -1.0, -1.0, 1.0, 0.0, 0.0, 0.0]));
    }
    let mut splat = Vec::new();
    for k in 0..3u8 {
        splat.extend(floats(&[k as f32, 1.0, 2.0, 0.1, 0.1, 0.1]));
        splat.extend([255, 128, k, 200, 255, 128, 128, 128]);
    }
    vec![ascii, binary, big, gauss, splat]
}

/// Imports PLY meshes and Gaussian splats, and `.splat` records.
pub fn run_ply(data: &[u8]) {
    let _ = sr_3d::import::ply(data);
    let _ = sr_3d::import::splat(data);
}

fn seeds_usd() -> Vec<Vec<u8>> {
    let usda = b"#usda 1.0\n(\n    upAxis = \"Z\"\n    metersPerUnit = 0.01\n)\ndef Xform \"root\"\n{\n    double3 xformOp:translate = (1, 2, 3)\n    uniform token[] xformOpOrder = [\"xformOp:translate\"]\n    def Mesh \"tri\"\n    {\n        int[] faceVertexCounts = [3]\n        int[] faceVertexIndices = [0, 1, 2]\n        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]\n        normal3f[] normals = [(0, 0, 1), (0, 0, 1), (0, 0, 1)]\n        texCoord2f[] primvars:st = [(0, 0), (1, 0), (0, 1)]\n        rel material:binding = </root/mat>\n    }\n    def Material \"mat\"\n    {\n        def Shader \"s\"\n        {\n            uniform token info:id = \"UsdPreviewSurface\"\n            color3f inputs:diffuseColor = (1, 0.5, 0)\n            float inputs:roughness = 0.4\n        }\n    }\n}\n";
    with_files(
        &[usda],
        &[
            "crates/sr-3d/tests/fixtures/values.usdc",
            "crates/sr-3d/tests/fixtures/stage.usdc",
            "crates/sr-3d/tests/fixtures/stage.usdz",
            "crates/sr-3d/tests/fixtures/stage.usda",
        ],
    )
}

/// Reads a USDC crate from memory, then imports the bytes as a USD file (USDA, USDC or USDZ).
pub fn run_usd(data: &[u8]) {
    let _ = sr_3d::usdc::read(data);
    if let Some(file) = Scratch::new("in.usd", data) {
        let _ = sr_3d::import::usd(&file.0);
    }
}

const GLTF: &str = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
"nodes":[{"name":"tri","mesh":0,"translation":[1,2,3],"rotation":[0,0,0,1],"scale":[1,1,1],"children":[1]},{"matrix":[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]}],
"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1,"material":0,"mode":4}]}],
"materials":[{"pbrMetallicRoughness":{"baseColorFactor":[1,0.5,0,1],"metallicFactor":0,"roughnessFactor":0.5},"alphaMode":"BLEND","doubleSided":true,"extensions":{"KHR_materials_emissive_strength":{"emissiveStrength":2}}}],
"animations":[{"channels":[{"sampler":0,"target":{"node":0,"path":"translation"}}],"samplers":[{"input":2,"output":0,"interpolation":"LINEAR"}]}],
"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"},{"bufferView":2,"componentType":5126,"count":3,"type":"SCALAR","min":[0],"max":[1]}],
"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":6},{"buffer":0,"byteOffset":44,"byteLength":12}],
"buffers":[{"byteLength":56,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAAABAAIAAAAAAAAAAAAAPwAAgD8="}]}"#;

fn seeds_gltf() -> Vec<Vec<u8>> {
    with_files(&[GLTF.as_bytes()], &["tests/corpus/media/robot.glb"])
}

/// Imports the bytes as a `.gltf` or `.glb` file.
pub fn run_gltf(data: &[u8]) {
    if let Some(file) = Scratch::new("in.gltf", data) {
        let _ = sr_3d::import::gltf(&file.0);
    }
}

// ------------------------------------------------------------------ documents

fn seeds_documents() -> Vec<Vec<u8>> {
    crate::corpus(&repo("tests/corpus")).into_iter().map(String::into_bytes).filter(|b| b.len() <= MAX_INPUT).collect()
}

/// [`crate::run_doc`] with asset verification on: referenced files resolve against the
/// corpus (`tests/corpus/valid`), are read and hashed.
pub fn run_doc_assets(data: &[u8]) {
    let input = String::from_utf8_lossy(data);
    let opts = sr_model::LoadOptions { verify_assets: true, base_dir: Some(repo("tests/corpus/valid")) };
    let _ = sr_model::validate_str(&input, &opts);
    if let Ok(doc) = sr_model::load_str(&input, &opts) {
        if let Ok(ev) = sr_eval::Evaluator::new(&doc, &Default::default()) {
            let d = ev.program().duration;
            for t in [0.0, d * 0.5, d] {
                let _ = ev.evaluate(t);
            }
        }
    }
}

// ------------------------------------------------------------------ campaign

const NUMBERS: &[&str] = &[
    "0",
    "-1",
    "1e308",
    "-1e308",
    "1e-320",
    "NaN",
    "Infinity",
    "99999999999999999999",
    "4294967296",
    "65536",
    "-0.5",
    "18446744073709551615",
    "0x7fffffff",
    "1e12",
    "100000000000",
];

/// One mutation of an input.
pub fn mutate_bytes(r: &mut Rng, seeds: &[Vec<u8>], src: &[u8]) -> Vec<u8> {
    let mut s = src.to_vec();
    for _ in 0..1 + r.below(4) {
        let a = r.below(s.len().max(1)).min(s.len());
        let b = (a + 1 + r.below(64)).min(s.len());
        match r.below(9) {
            0 if a < s.len() => s[a] ^= 1 << r.below(8),
            1 if a < s.len() => s[a] = *r.pick(&[0x00, 0xff, 0x7f, 0x80, 0x01, b' ', b'\n']),
            2 => {
                s.drain(a..b);
            }
            3 => {
                let span = s[a..b].to_vec();
                s.splice(a..a, span);
            }
            4 => {
                // a boundary integer over the bytes here, little- or big-endian
                let v: u64 =
                    *r.pick(&[0, 1, 0x7f, 0x80, 0xff, 0x7fff, 0xffff, 0x7fff_ffff, 0xffff_ffff, u64::MAX, 1 << 62]);
                let w = *r.pick(&[1usize, 2, 4, 8]);
                let mut bytes = v.to_le_bytes()[..w].to_vec();
                if r.below(2) == 0 {
                    bytes.reverse();
                }
                let end = (a + w).min(s.len());
                s.splice(a..end, bytes);
            }
            5 => {
                let o = r.pick(seeds);
                let oa = r.below(o.len().max(1)).min(o.len());
                let ob = (oa + r.below(200)).min(o.len());
                s.splice(a..a, o[oa..ob].iter().copied());
            }
            6 => {
                // replace a run of digits with a boundary number
                let from = s[a..].iter().position(u8::is_ascii_digit).map(|k| a + k).unwrap_or(a);
                let to = s[from..].iter().position(|c| !c.is_ascii_digit() && *c != b'.').map_or(s.len(), |k| from + k);
                s.splice(from..to, r.pick(NUMBERS).bytes());
            }
            7 => s.truncate(a),
            _ => {
                let c = b' ' + r.below(95) as u8;
                s.insert(a, c);
            }
        }
    }
    s.truncate(MAX_INPUT);
    s
}

/// A panic found by the fuzzer.
#[derive(Debug, Clone)]
pub struct Crash {
    pub target: &'static str,
    pub input: Vec<u8>,
    pub message: String,
}

/// Runs `f(input)`, turning a panic into a [`Crash`].
pub fn guard(target: &'static str, input: &[u8], f: fn(&[u8])) -> Option<Crash> {
    catch_unwind(AssertUnwindSafe(|| f(input))).err().map(|e| Crash {
        target,
        input: input.to_vec(),
        message: e
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default(),
    })
}

/// Statistics of a campaign on one target.
#[derive(Debug, Default, Clone)]
pub struct Campaign {
    pub inputs: u64,
    pub crashes: Vec<Crash>,
}

/// Fuzzes `t` until `stop()` returns true: every seed as it is, then mutations.
pub fn fuzz(seed: u64, t: &Target, mut stop: impl FnMut(&Campaign) -> bool) -> Campaign {
    let mut r = Rng(seed | 1);
    let mut c = Campaign::default();
    let seeds = (t.seeds)();
    let texts: Vec<String> = seeds.iter().map(|s| String::from_utf8_lossy(s).into_owned()).collect();
    let mut next = 0;
    while !stop(&c) && !seeds.is_empty() {
        let input = if next < seeds.len() {
            next += 1;
            seeds[next - 1].clone()
        } else if t.text && r.below(2) == 0 {
            let base = r.pick(&texts).clone();
            let mut s = crate::mutate_doc(&mut r, &texts, &base).into_bytes();
            s.truncate(MAX_INPUT);
            s
        } else {
            let base = r.pick(&seeds).clone();
            mutate_bytes(&mut r, &seeds, &base)
        };
        if let Some(k) = guard(t.name, &input, t.run) {
            c.crashes.push(k);
        }
        c.inputs += 1;
    }
    c
}
