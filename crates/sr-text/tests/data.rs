use sr_text::animate::{self, Animator, OnPath, Preset, Props, Selector, Shape, Unit};
use sr_text::captions::{self, Preset as CapPreset};
use sr_text::glyph::{self, Decor};
use sr_text::layout::{self, Opts, Para, Run};
use sr_text::{chart, code, formula, FontLib, Style};
use sr_vector::Paint;
use std::sync::{Mutex, OnceLock};

fn lib() -> std::sync::MutexGuard<'static, FontLib> {
    static L: OnceLock<Mutex<FontLib>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(FontLib::new(true))).lock().unwrap_or_else(|e| e.into_inner())
}

fn st(size: f64) -> Style {
    Style { families: vec!["DejaVu Sans".into()], size, ..Default::default() }
}

fn lay(lib: &mut FontLib, runs: &[(&str, Option<&str>)]) -> sr_text::Layout {
    let para = Para {
        runs: runs.iter().map(|(t, r)| Run { text: t.to_string(), style: 0, role: r.map(str::to_string) }).collect(),
        styles: vec![st(40.0)],
        opts: Opts::default(),
    };
    layout::layout(lib, &para)
}

fn coverage(d: &sr_text::Drawing, size: [u32; 2]) -> f32 {
    let e = sr_vector::tile::encode(&d.scene, size);
    sr_vector::tile::render_cpu(&e, &|_, _, _| [1.0; 4]).iter().map(|c| c[3]).sum()
}

#[test]
fn presets_and_selectors() {
    let mut lib = lib();
    let l = lay(&mut lib, &[("Hello world", None)]);
    let roles = vec![None];
    // typewriter half way: the first half shows, the rest is hidden
    let a = Animator { preset: Some((Preset::Typewriter, 0.0, 1.0)), ..Default::default() };
    let (fx, _) = animate::apply(&lib, &l, &roles, &[a], 0.5);
    let shown: Vec<bool> = fx.iter().map(|f| f.opacity > 0.5).collect();
    assert!(shown[0] && !shown[shown.len() - 1]);
    // before the preset starts everything is hidden, after it ends everything shows
    let a = Animator { preset: Some((Preset::FadeIn, 1.0, 1.0)), ..Default::default() };
    assert!(animate::apply(&lib, &l, &roles, std::slice::from_ref(&a), 0.5).0.iter().all(|f| f.opacity < 1e-9));
    assert!(animate::apply(&lib, &l, &roles, &[a], 2.5).0.iter().all(|f| (f.opacity - 1.0).abs() < 1e-9));
    // every preset parses and runs
    for name in [
        "typewriter",
        "fade-in",
        "fade-out",
        "word-by-word",
        "letter-by-letter",
        "line-by-line",
        "slide-up",
        "slide-down",
        "slide-left",
        "slide-right",
        "pop",
        "scale-in",
        "blur-in",
        "wave",
        "bounce",
        "spin",
        "ascend",
        "shift",
        "scramble",
        "counter",
        "karaoke",
        "highlight",
        "tracking-in",
        "mask-reveal",
    ] {
        let p = Preset::parse(name).unwrap_or_else(|| panic!("{name}"));
        let a = Animator { preset: Some((p, 0.0, 1.0)), ..Default::default() };
        let (fx, _) = animate::apply(&lib, &l, &roles, &[a], 0.4);
        assert_eq!(fx.len(), l.glyphs.len());
    }
    // range selector: a square range over the first word moves only its characters
    let a = Animator {
        unit: Unit::Word,
        selector: Selector::Range {
            percent: true,
            start: 0.0,
            end: 50.0,
            offset: 0.0,
            amount: 100.0,
            shape: Shape::Square,
            smoothness: 1.0,
            ease_high: 0.0,
            ease_low: 0.0,
            order: animate::Order::Forward,
            seed: 0,
        },
        props: Props { y: Some(-20.0), ..Default::default() },
        ..Default::default()
    };
    let (fx, _) = animate::apply(&lib, &l, &roles, &[a], 0.0);
    for (g, f) in l.glyphs.iter().zip(&fx) {
        let moved = (f.xf.0[5] + 20.0).abs() < 1e-9;
        assert_eq!(moved, g.word == 0 && !l.chars[g.ch].is_whitespace(), "{:?}", l.chars[g.ch]);
    }
    // span roles restrict selection
    let l2 = lay(&mut lib, &[("Price ", None), ("$99", Some("price"))]);
    let roles2 = vec![None, Some("price".to_string())];
    let a = Animator {
        role: Some("price".into()),
        props: Props { opacity: Some(0.0), ..Default::default() },
        ..Default::default()
    };
    let (fx, _) = animate::apply(&lib, &l2, &roles2, &[a], 0.0);
    for (g, f) in l2.glyphs.iter().zip(&fx) {
        assert_eq!(f.opacity < 0.5, g.span == 1);
    }
    // counter: numbers count up from 0 before layout, keeping decimals and thousands commas
    assert_eq!(animate::counter_text("Count 1,234.50 and 7", 0.5), "Count 617.25 and 4");
    assert_eq!(animate::counter_text("12,000,000", 0.5), "6,000,000");
    let k = animate::counter_progress(0.0, 1.0, 100.0, 0.5).unwrap();
    assert!((k - 0.875).abs() < 1e-12, "cubic-out");
    assert_eq!(animate::counter_text("1234", k), "1080");
    assert_eq!(animate::counter_progress(0.0, 1.0, 100.0, 1.0), None, "at the end the text shows its value");
}

/// CONVENTIONS 5.11: the preset table (unit, mode, ease, overlap) of the Python renderer.
#[test]
fn preset_table_timing() {
    let mut lib = lib();
    let l = lay(&mut lib, &[("ab cd ef", None)]);
    let roles = vec![None];
    let at = |p: Preset, t: f64| {
        let a = Animator { preset: Some((p, 0.0, 1.0)), ..Default::default() };
        animate::apply(&lib, &l, &roles, &[a], t).0
    };
    // word-by-word: three word slots of 1/3 s each, no fade
    let fx = at(Preset::WordByWord, 0.5);
    let op: Vec<f64> = fx.iter().map(|f| f.opacity).collect();
    assert_eq!(op, [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0]);
    // fade-in: 8 character slots, overlap 0.6, d = 1 / (1 + 7 · 0.4), quad-out
    let d = 1.0 / (1.0 + 7.0 * 0.4);
    let fx = at(Preset::FadeIn, 0.3);
    for (i, f) in fx.iter().enumerate() {
        let q = ((0.3 - i as f64 * d * 0.4) / d).clamp(0.0, 1.0);
        let e = 1.0 - (1.0 - q) * (1.0 - q);
        assert!((f.opacity - e).abs() < 1e-9, "{i}: {} != {e}", f.opacity);
    }
    // karaoke: word units, linear, overlap 0: the second word is half lit at t = 0.5
    let fx = at(Preset::Karaoke, 0.5);
    assert!((fx[0].fill_mix - 1.0).abs() < 1e-9 && (fx[3].fill_mix - 0.5).abs() < 1e-9 && fx[6].fill.is_none());
    // highlight: a box per word wiping in, no fill change
    let fx = at(Preset::Highlight, 0.5);
    assert!(fx[0].fill.is_none() && fx[3].highlight.as_ref().is_some_and(|h| (h.fraction - 0.5).abs() < 1e-9));
    // scramble: hidden before its start, then the unrevealed characters are random letters
    let a = Animator { preset: Some((Preset::Scramble, 1.0, 1.0)), ..Default::default() };
    assert!(animate::apply(&lib, &l, &roles, &[a.clone()], 0.5).0.iter().all(|f| f.opacity == 0.0));
    let chars: Vec<char> = "ab cd ef".chars().collect();
    let s: String = animate::scramble_text(&chars, &|_| true, &a, 1.0, 1.0, 1.5).into_iter().collect();
    assert_eq!(&s[..4], "ab c", "the first four slots are revealed");
    assert!(s[4..].chars().all(|c| c == ' ' || c.is_ascii_lowercase()) && &s[4..] != "d ef");
    // wave: y = −0.25 em · sin(2π(1.5 t − p/8)) · envelope
    let fx = at(Preset::Wave, 0.5);
    let em = l.styles[0].size;
    for (i, f) in fx.iter().enumerate().filter(|(i, _)| chars[*i] != ' ') {
        let y = -0.25 * em * (std::f64::consts::TAU * (0.75 - i as f64 / 8.0)).sin();
        assert!((f.xf.0[5] - y).abs() < 1e-6, "{i}");
    }
}

#[test]
fn text_on_path_follows_the_curve() {
    let mut lib = lib();
    let l = lay(&mut lib, &[("ABC", None)]);
    let mut path = sr_vector::Path::default();
    path.move_to(sr_vector::geom::p(0.0, 100.0));
    path.line_to(sr_vector::geom::p(0.0, 400.0));
    let xf = animate::on_path(
        &l,
        &OnPath {
            path,
            start_offset: 0.0,
            first_margin: 0.0,
            last_margin: 0.0,
            reverse: false,
            perpendicular: true,
            force_alignment: false,
        },
    );
    // along a vertical path the glyph centres move down x = 0
    for (g, x) in l.glyphs.iter().zip(&xf) {
        let c = x.apply(sr_vector::geom::p(g.x + g.advance * 0.5, l.lines[0].baseline));
        assert!(c.x.abs() < 1e-6 && c.y > 100.0);
    }
}

#[test]
fn caption_parsers() {
    let srt = "1\n00:00:01,000 --> 00:00:02,500\nHello <i>there</i>\n\n2\n00:00:03,000 --> 00:00:04,000\nBye\n";
    let c = captions::parse(srt, "srt").unwrap();
    assert_eq!(c.len(), 2);
    assert_eq!((c[0].start, c[0].end, c[0].text.as_str()), (1.0, 2.5, "Hello there"));
    let vtt = "WEBVTT\n\n00:01.000 --> 00:03.000 line:90%\n<v Ann>Hello <00:01.500>big <00:02.000>world\n";
    let c = captions::parse(vtt, "vtt").unwrap();
    assert_eq!(c[0].speaker.as_deref(), Some("Ann"));
    assert_eq!(
        c[0].words.iter().map(|w| (w.text.as_str(), w.start)).collect::<Vec<_>>(),
        vec![("Hello", 1.0), ("big", 1.5), ("world", 2.0)]
    );
    let ass = "[Script Info]\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,{\\k50}Sing {\\k100}along\\Nnow\n";
    let c = captions::parse(ass, "ass").unwrap();
    assert_eq!(c[0].text, "Sing along\nnow");
    assert_eq!(c[0].words[0].text, "Sing");
    assert!((c[0].words[1].start - 1.5).abs() < 1e-9 && (c[0].words[1].end - 2.5).abs() < 1e-9);
    let ttml = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter" ttp:frameRate="25"><body><div><p begin="00:00:01:05" end="2.5s">One<br/>two</p><p begin="30f" dur="1s">three</p></div></body></tt>"#;
    let c = captions::parse(ttml, "ttml").unwrap();
    assert_eq!(c.len(), 2);
    assert!((c[0].start - 1.2).abs() < 1e-9 && (c[0].end - 2.5).abs() < 1e-9);
    assert_eq!(c[0].text, "One\ntwo");
    assert!((c[1].start - 1.2).abs() < 1e-9 && (c[1].end - 2.2).abs() < 1e-9);
    // SCC pop-on: RCL, PAC, "HI", EOC shows it; EDM clears it
    let scc = "Scenarist_SCC V1.0\n\n00:00:01:00\t9420 9420 9452 9452 c849 942f 942f\n\n00:00:03:00\t942c 942c\n";
    let c = captions::parse(scc, "scc").unwrap();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].text, "HI");
    assert!(c[0].start > 1.0 && c[0].start < 1.3 && c[0].end > 3.0 && c[0].end < 3.2, "{:?}", c[0]);
    assert!(captions::parse("nope", "vtt").is_err());
    // transcripts
    let t = captions::from_transcript(r#"{"segments":[{"start":0,"end":2,"text":"a b","words":[{"start":0,"end":1,"word":"a"},{"start":1,"end":2,"word":"b"}]}]}"#).unwrap();
    assert_eq!(t[0].words.len(), 2);
    // paging and sidecars
    let cue = captions::Cue {
        start: 0.0,
        end: 4.0,
        text: "one two three four five six seven eight".into(),
        ..Default::default()
    };
    let pages = captions::paginate(std::slice::from_ref(&cue), Some(2), 32, 2, false);
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].text(), "one two\nthree four");
    assert!(captions::to_vtt(std::slice::from_ref(&cue)).starts_with("WEBVTT\n\n00:00:00.000 --> 00:00:04.000\n"));
    assert!(captions::to_srt(&[cue]).starts_with("1\n00:00:00,000 --> 00:00:04,000\n"));
    assert_eq!(captions::profanity("oh shit, no"), "oh s***, no");
}

#[test]
fn caption_effects_track_the_active_word() {
    let mut lib = lib();
    let cue = captions::Cue { start: 0.0, end: 3.0, text: "aa bb cc".into(), ..Default::default() };
    let pages = captions::paginate(std::slice::from_ref(&cue), None, 32, 2, false);
    let page = pages[0].clone();
    assert_eq!((page.start, page.end), (0.0, 3.0), "one page spans its cue");
    let l = lay(&mut lib, &[(&page.text(), None)]);
    let active = Paint::Solid { rgba: [1.0, 0.8, 0.0, 1.0], srgb: true };
    // one-word: a page per word, from the previous word's end to the next word's start
    let ones = captions::paginate(std::slice::from_ref(&cue), None, 32, 2, true);
    assert_eq!(ones.iter().map(|p| p.text()).collect::<Vec<_>>(), ["aa", "bb", "cc"]);
    assert_eq!((ones[0].start, ones[2].end), (0.0, 3.0));
    assert!((ones[1].start - ones[0].end).abs() < 1e-12);
    // karaoke lights whole words: done words are lit, the current word cross-fades, later ones wait
    let words = page.words();
    let (w1, t) = (words[1], 1.5);
    let q = (t - w1.start) / (w1.end - w1.start);
    let fx = captions::effects(CapPreset::Karaoke, &l, &page, t, &active);
    let mix = |w: usize| -> Vec<f64> {
        fx.iter()
            .zip(&l.glyphs)
            .filter(|(_, g)| g.word == w && !l.chars[g.ch].is_whitespace())
            .map(|(f, _)| f.fill_mix)
            .collect()
    };
    assert!(mix(0).iter().all(|m| *m == 1.0));
    assert!(mix(1).iter().all(|m| (m - q).abs() < 1e-9), "{:?} vs {q}", mix(1));
    assert!(mix(2).iter().all(|m| *m == 0.0));
    // fade: pages fade in and out over 0.15 s
    let fx = captions::effects(CapPreset::Fade, &l, &page, 0.075, &active);
    assert!(fx.iter().all(|f| (f.opacity - 0.5).abs() < 1e-9));
    for p in [
        "classic",
        "boxed-line",
        "boxed-word",
        "one-word",
        "karaoke",
        "highlight",
        "pop",
        "fade",
        "bounce",
        "slide",
        "typewriter",
        "enlarge",
        "none",
    ] {
        assert_eq!(captions::effects(CapPreset::parse(p), &l, &page, 0.7, &active).len(), l.glyphs.len());
    }
}

#[test]
fn charts_codes_formulas_audiograms() {
    let mut lib = lib();
    let base = chart::Chart {
        kind: chart::Kind::Column,
        size: [400.0, 300.0],
        labels: vec!["a".into(), "b".into(), "c".into()],
        series: vec![chart::Series { name: "s".into(), values: vec![1.0, 3.0, 2.0], color: None }],
        progress: 1.0,
        show_axes: true,
        show_values: true,
        format: None,
        text: st(14.0),
    };
    let full = coverage(&chart::draw(&mut lib, &base, 0.25), [400, 300]);
    let half = coverage(&chart::draw(&mut lib, &chart::Chart { progress: 0.5, ..base.clone() }, 0.25), [400, 300]);
    assert!(half < full && half > 0.0);
    for k in ["bar", "line", "area", "pie", "donut", "scatter", "counter", "progress"] {
        let c = chart::Chart { kind: chart::Kind::parse(k).unwrap(), ..base.clone() };
        assert!(coverage(&chart::draw(&mut lib, &c, 0.25), [400, 300]) > 100.0, "{k}");
    }
    assert_eq!(chart::format_number(1234.5, Some("$#,##0.00")), "$1,234.50");
    assert_eq!(chart::format_number(0.25, Some("0%")), "25%");
    assert_eq!(chart::format_number(1.234, Some("%.2f")), "1.23");
    // QR: version 1 is 21 modules; with a 4-module quiet zone at 29 px each module is 1 px
    let black = Paint::Solid { rgba: [0.0, 0.0, 0.0, 1.0], srgb: true };
    let white = Paint::Solid { rgba: [1.0; 4], srgb: true };
    let q = code::draw("qr", "HELLO", [29.0, 29.0], black.clone(), white.clone(), "L", 4, 0.1).unwrap();
    let e = sr_vector::tile::encode(&q.scene, [29, 29]);
    let px = sr_vector::tile::render_cpu(&e, &|pi, _, _| if pi == 1 { [0.0, 0.0, 0.0, 1.0] } else { [1.0; 4] });
    // finder pattern: the top-left module after the quiet zone is dark, the quiet zone is light
    assert!(px[4 * 29 + 4][0] < 0.1 && px[0][0] > 0.9);
    for k in ["datamatrix", "pdf417", "code128"] {
        assert!(code::draw(k, "HELLO 123", [200.0, 100.0], black.clone(), white.clone(), "M", 2, 0.25).is_ok(), "{k}");
    }
    assert!(code::draw("ean13", "590123412345", [200.0, 100.0], black.clone(), white.clone(), "M", 2, 0.25).is_ok());
    assert!(code::draw("upc-a", "01234567890", [200.0, 100.0], black.clone(), white.clone(), "M", 2, 0.25).is_ok());
    assert!(code::draw("ean13", "12", [200.0, 100.0], black, white, "M", 2, 0.25).is_err());
    // formulas
    let f = formula::draw(
        &mut lib,
        r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
        48.0,
        [600.0, 200.0],
        Paint::Solid { rgba: [1.0; 4], srgb: false },
        0.25,
    )
    .unwrap();
    assert!(coverage(&f, [600, 200]) > 2000.0);
    let s = formula::draw(
        &mut lib,
        r"\sum_{i=1}^{n} i^2 + \left( \int_0^1 x\,dx \right)",
        48.0,
        [600.0, 200.0],
        Paint::Solid { rgba: [1.0; 4], srgb: false },
        0.25,
    )
    .unwrap();
    assert!(coverage(&s, [600, 200]) > 1000.0);
    assert!(formula::draw(
        &mut lib,
        r"\frac{1}{",
        48.0,
        [100.0, 100.0],
        Paint::Solid { rgba: [1.0; 4], srgb: false },
        0.25
    )
    .is_err());
    // audiograms
    let hist: Vec<f64> = (0..64).map(|i| (i as f64 * 0.3).sin().abs()).collect();
    for k in ["bars", "line", "wave", "circle", "spectrum"] {
        let d = sr_text::audiogram::draw(
            sr_text::audiogram::Kind::parse(k),
            32,
            [300.0, 120.0],
            Paint::Solid { rgba: [1.0; 4], srgb: false },
            &hist,
            [0.8, 0.5, 0.2],
            0.3,
            0.25,
        );
        assert!(coverage(&d, [300, 120]) > 50.0, "{k}");
    }
    let _ = glyph::GlyphFx::default();
    let _ = Decor::default();
}
