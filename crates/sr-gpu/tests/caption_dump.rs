//! The caption dump (SREP 52 conformance): each track's pages after paging, the words of each cue with their times,
//! and the word the presets treat as current, as the burn-in layout computes them. No GPU is needed.

use sr_gpu::caption_dump::{dump, CaptionDump};

fn doc(captions: &str) -> sr_model::Document {
    let xml = format!(
        r##"<scene version="1.2"><project width="640" height="360" fps="24" duration="8" background="#000000FF"/><composition/><captions>{captions}</captions></scene>"##
    );
    sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"))
}

fn dumped(captions: &str, at: &[f64]) -> CaptionDump {
    let d = doc(captions);
    let ev = sr_eval::Evaluator::new(&d, &sr_eval::EvalOptions::default()).unwrap_or_else(|r| panic!("{r:?}"));
    dump(ev.program(), at)
}

fn texts(d: &CaptionDump, track: usize) -> Vec<String> {
    d.tracks[track].pages.iter().map(|p| p.text.clone()).collect()
}

const THREE_LINES: &str = "aa bb&#10;cc dd ee ff&#10;gg hh";

fn track(attrs: &str, text: &str) -> String {
    format!(
        r#"<captionTrack id="cap" language="en" preset="classic" {attrs}><cue start="0" end="8" text="{text}"/></captionTrack>"#
    )
}

#[test]
fn pages_follow_the_line_breaks_mode_and_the_limits() {
    let pages = |attrs: &str| texts(&dumped(&track(attrs, THREE_LINES), &[]), 0);
    // SREP 52 conformance: source, default, greedy, limits
    assert_eq!(pages(r#"lineBreaks="source" maxCharsPerLine="80" maxLines="2""#), ["aa bb\ncc dd ee ff", "gg hh"]);
    assert_eq!(pages(r#"maxCharsPerLine="80" maxLines="2""#), ["aa bb cc dd ee ff gg hh"]);
    assert_eq!(pages(r#"lineBreaks="greedy" maxCharsPerLine="80" maxLines="2""#), ["aa bb cc dd ee ff gg hh"]);
    assert_eq!(pages(r#"lineBreaks="source" maxCharsPerLine="5" maxLines="2""#), ["aa bb\ncc dd", "ee ff\ngg hh"]);
    assert_eq!(
        pages(r#"lineBreaks="source" maxWordsPerLine="2" maxCharsPerLine="80" maxLines="2""#),
        ["aa bb\ncc dd", "ee ff\ngg hh"]
    );
    assert_eq!(pages(r#"lineBreaks="source" maxCharsPerLine="80" maxLines="1""#), ["aa bb", "cc dd ee ff", "gg hh"]);
}

#[test]
fn a_page_lists_its_lines_cue_times_and_first_word() {
    // eight words of two letters over 8 s: each word lasts exactly 1 s
    let d = dumped(
        &track(r#"lineBreaks="source" maxWordsPerLine="2" maxCharsPerLine="80" maxLines="2""#, THREE_LINES),
        &[],
    );
    let t = &d.tracks[0];
    assert_eq!((t.id.as_str(), t.preset, t.line_breaks, t.burn), ("cap", "classic", "source", true));
    assert_eq!((t.max_chars_per_line, t.max_words_per_line, t.max_lines), (80, Some(2), 2));
    assert_eq!(t.error, None);
    assert_eq!(t.word_count, 8);
    assert_eq!(t.pages.len(), 2);
    let (p0, p1) = (&t.pages[0], &t.pages[1]);
    assert_eq!(p0.lines, ["aa bb", "cc dd"]);
    assert_eq!(p1.lines, ["ee ff", "gg hh"]);
    assert_eq!((p0.cue, p0.start, p0.end, p0.first_word), (0, 0.0, 4.0, 0));
    assert_eq!((p1.cue, p1.start, p1.end, p1.first_word), (0, 4.0, 8.0, 4));
    // the cue lists every word once, in order, with its time
    assert_eq!(t.cues.len(), 1);
    let words: Vec<_> = t.cues[0].words.iter().map(|w| (w.index, w.text.as_str(), w.start, w.end)).collect();
    assert_eq!(words[0], (0, "aa", 0.0, 1.0));
    assert_eq!(words[5], (5, "ff", 5.0, 6.0));
    assert_eq!(words.len(), 8);
    // the current word of the second page is addressed by its index in the cue
    assert_eq!(
        p1.active.iter().map(|s| (s.start, s.end, s.word)).collect::<Vec<_>>(),
        [(4.0, 5.0, Some(4)), (5.0, 6.0, Some(5)), (6.0, 7.0, Some(6)), (7.0, 8.0, Some(7))]
    );
}

#[test]
fn presets_address_the_same_word_indices_in_both_modes() {
    for preset in ["karaoke", "highlight", "boxed-word"] {
        let doc = format!(
            r#"<captionTrack id="src" language="en" preset="{preset}" lineBreaks="source" maxCharsPerLine="80" maxLines="2"><cue start="0" end="4" text="aa bb&#10;cc dd"/></captionTrack><captionTrack id="grd" language="en" preset="{preset}" lineBreaks="greedy" maxCharsPerLine="80" maxLines="2"><cue start="0" end="4" text="aa bb&#10;cc dd"/></captionTrack>"#
        );
        let d = dumped(&doc, &[0.5, 1.5, 2.5, 3.5, 4.0, 9.0]);
        assert_eq!(texts(&d, 0), ["aa bb\ncc dd"]);
        assert_eq!(texts(&d, 1), ["aa bb cc dd"]);
        for t in &d.tracks {
            assert_eq!(t.preset, preset);
            let at: Vec<_> = t.at.iter().map(|s| (s.time, s.page, s.cue, s.word)).collect();
            assert_eq!(
                at,
                [
                    (0.5, Some(0), Some(0), Some(0)),
                    (1.5, Some(0), Some(0), Some(1)),
                    (2.5, Some(0), Some(0), Some(2)),
                    (3.5, Some(0), Some(0), Some(3)),
                    // a page ends at its end time: nothing is on screen after the cue
                    (4.0, None, None, None),
                    (9.0, None, None, None)
                ]
            );
        }
    }
}

#[test]
fn blank_lines_and_crlf_change_neither_words_nor_times() {
    let text = "&#10; aa bb&#13;&#10;&#10; cc dd&#10;";
    let src = dumped(&track(r#"lineBreaks="source" maxCharsPerLine="80" maxLines="2""#, text), &[]);
    let grd = dumped(&track(r#"lineBreaks="greedy" maxCharsPerLine="80" maxLines="2""#, text), &[]);
    assert_eq!(texts(&src, 0), ["aa bb\ncc dd"]);
    assert_eq!(texts(&grd, 0), ["aa bb cc dd"]);
    assert_eq!((src.tracks[0].word_count, grd.tracks[0].word_count), (4, 4));
    assert_eq!(src.tracks[0].cues, grd.tracks[0].cues);
}

#[test]
fn one_word_pages_each_word_and_sidecar_tracks_are_listed_unburned() {
    let d = dumped(
        r#"<captionTrack id="ow" language="en" preset="one-word" lineBreaks="source"><cue start="0" end="4" text="aa bb&#10;cc dd"/></captionTrack><captionTrack id="sc" language="en" mode="sidecar"><cue start="0" end="2" text="hello"/></captionTrack>"#,
        &[2.5],
    );
    assert_eq!(texts(&d, 0), ["aa", "bb", "cc", "dd"]);
    assert_eq!(d.tracks[0].pages.iter().map(|p| p.first_word).collect::<Vec<_>>(), [0, 1, 2, 3]);
    assert_eq!((d.tracks[0].at[0].page, d.tracks[0].at[0].word), (Some(2), Some(2)));
    assert!(!d.tracks[1].burn);
    assert_eq!(texts(&d, 1), ["hello"]);
}

#[test]
fn a_track_that_cannot_be_read_is_listed_with_its_error() {
    let d = dumped(
        r#"<captionTrack id="gone" language="en" src="no-such-file.srt"/><captionTrack id="ok" language="en"><cue start="0" end="1" text="hi"/></captionTrack>"#,
        &[0.5],
    );
    let gone = &d.tracks[0];
    assert!(gone.error.as_deref().is_some_and(|e| e.contains("no-such-file.srt")), "{:?}", gone.error);
    assert!(gone.pages.is_empty() && gone.cues.is_empty());
    assert_eq!(gone.at[0].page, None);
    assert_eq!(texts(&d, 1), ["hi"]);
    assert!(!d.is_complete());
    assert!(dumped(r#"<captionTrack id="ok" language="en"><cue start="0" end="1" text="hi"/></captionTrack>"#, &[])
        .is_complete());
}

#[test]
fn the_json_form_is_stable_and_camel_cased() {
    let caps = track(r#"lineBreaks="source" maxCharsPerLine="80" maxLines="2""#, THREE_LINES);
    let a = serde_json::to_string(&dumped(&caps, &[0.5])).unwrap();
    let b = serde_json::to_string(&dumped(&caps, &[0.5])).unwrap();
    assert_eq!(a, b, "the dump is deterministic");
    let v: serde_json::Value = serde_json::from_str(&a).unwrap();
    let t = &v["tracks"][0];
    assert_eq!(t["lineBreaks"], "source");
    assert_eq!(t["maxCharsPerLine"], 80);
    assert_eq!(t["maxWordsPerLine"], serde_json::Value::Null);
    assert_eq!(t["wordCount"], 8);
    assert_eq!(t["pages"][0]["text"], "aa bb\ncc dd ee ff");
    assert_eq!(t["pages"][0]["lines"], serde_json::json!(["aa bb", "cc dd ee ff"]));
    assert_eq!(t["pages"][1]["firstWord"], 6);
    assert_eq!(t["at"][0], serde_json::json!({"time": 0.5, "page": 0, "cue": 0, "word": 0}));
    assert!(t.get("error").is_none(), "no error key when the track loaded");
}

#[test]
fn a_document_without_captions_dumps_no_tracks() {
    let d = sr_model::load_str(
        r#"<scene version="1.2"><project width="64" height="64" fps="24" duration="1"/><composition/></scene>"#,
        &sr_model::LoadOptions::without_assets(),
    )
    .unwrap();
    let ev = sr_eval::Evaluator::new(&d, &sr_eval::EvalOptions::default()).unwrap();
    let out = dump(ev.program(), &[0.0]);
    assert!(out.tracks.is_empty() && out.is_complete());
}
