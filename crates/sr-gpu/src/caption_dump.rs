//! The caption pages of a document as data: for each caption track, the words of each cue with their times, the
//! pages after paging (each page's lines, start and end), and the word the presets treat as current over time.
//!
//! The pages exist only inside the burn-in layout; this module reads them from the same place
//! ([`crate::text::tracks`]) and finds the page on screen and its current word with the same functions
//! ([`Track::page_at`], [`captions::active_word`]), so what it reports is what a render draws. It exists for
//! conformance checks of caption paging (SREP 52: `lineBreaks`, the limits, and the word indices the presets
//! address), through `scene-render captions`.
//!
//! Word indices are indices in the cue: the words of a cue, in order, over all of its pages (SREP 52, Semantics 5).
//! The output is deterministic: the same document and times give the same values, in the same order.

use serde::Serialize;
use sr_eval::Program;
use sr_text::captions::{self, Page};

use crate::text::{tracks, TextCache, Track};
use crate::vector::Attrs;

/// Every caption track of a document, in document order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CaptionDump {
    pub tracks: Vec<TrackDump>,
}

impl CaptionDump {
    /// Whether every track could be read (no track carries an error).
    pub fn is_complete(&self) -> bool {
        self.tracks.iter().all(|t| t.error.is_none())
    }
}

/// One caption track after paging.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackDump {
    pub id: String,
    /// Whether the track is burned into the picture (`mode` other than `sidecar`).
    pub burn: bool,
    /// The preset the layout applies (an unknown name is `classic`).
    pub preset: &'static str,
    /// `greedy` or `source`.
    pub line_breaks: &'static str,
    pub max_chars_per_line: usize,
    pub max_words_per_line: Option<usize>,
    pub max_lines: usize,
    /// Why the track's cues could not be read (a missing subtitle file or transcription cache); its cues, pages and
    /// samples are then empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The number of words over all cues.
    pub word_count: usize,
    pub cues: Vec<CueDump>,
    pub pages: Vec<PageDump>,
    /// The page and current word at each requested time, in the order asked.
    pub at: Vec<Sample>,
}

/// A cue and the words the pages show for it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CueDump {
    pub start: f64,
    pub end: f64,
    pub words: Vec<WordDump>,
}

/// A word of a cue: its index in the cue, its text and its time.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WordDump {
    pub index: usize,
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// A page: on screen from `start` until (not including) `end`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageDump {
    /// Index of the page's cue in the track.
    pub cue: usize,
    pub start: f64,
    pub end: f64,
    /// The index in the cue of the page's first word.
    pub first_word: usize,
    /// The page's lines, each its words joined by one space.
    pub lines: Vec<String>,
    /// The lines joined by line feeds: the text the layout sets.
    pub text: String,
    /// The current word over the page's time, as spans that cover it in order.
    pub active: Vec<SpanDump>,
}

/// A span of a page's time with one current word (an index in the cue), or none before the first word starts.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpanDump {
    pub start: f64,
    pub end: f64,
    pub word: Option<usize>,
}

/// What is on screen at a time: the page (its index in the track), its cue, and the current word (an index in the
/// cue). All three are absent when no page of the track is on screen.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sample {
    pub time: f64,
    pub page: Option<usize>,
    pub cue: Option<usize>,
    pub word: Option<usize>,
}

/// The caption tracks of a compiled document after paging, with the page and current word at each time of `at`.
pub fn dump(p: &Program, at: &[f64]) -> CaptionDump {
    let model = p.scene.captions.as_ref().map(|c| c.caption_tracks.as_slice()).unwrap_or(&[]);
    let loaded = tracks(&mut TextCache::default(), p);
    let dumped = model
        .iter()
        .zip(loaded.iter())
        .map(|(m, tr)| {
            let line_breaks = if m.line_breaks.as_str() == "source" { "source" } else { "greedy" };
            let mut out = TrackDump {
                id: m.id.clone(),
                burn: true,
                preset: captions::Preset::Classic.name(),
                line_breaks,
                max_chars_per_line: m.max_chars_per_line as usize,
                max_words_per_line: m.max_words_per_line.map(|x| x as usize),
                max_lines: m.max_lines as usize,
                error: None,
                word_count: 0,
                cues: Vec::new(),
                pages: Vec::new(),
                at: Vec::new(),
            };
            match tr {
                Ok(tr) => fill(&mut out, tr, at),
                Err(e) => {
                    // as the layout reads them (`text::load_track`)
                    let a = Attrs { e: m, props: None };
                    out.burn = a.str("mode").unwrap_or_default() != "sidecar";
                    out.preset = captions::Preset::parse(&a.str("preset").unwrap_or_default()).name();
                    out.error = Some(e.clone());
                    out.at = at.iter().map(|&time| Sample { time, page: None, cue: None, word: None }).collect();
                }
            }
            out
        })
        .collect();
    CaptionDump { tracks: dumped }
}

fn fill(out: &mut TrackDump, tr: &Track, at: &[f64]) {
    out.burn = tr.burn;
    out.preset = tr.preset.name();
    // the index in its cue of each page's first word: a cue's words run on over its pages
    let mut next = vec![0usize; tr.cues.len()];
    let first: Vec<usize> = tr
        .pages
        .iter()
        .map(|pg| {
            let i = next[pg.cue];
            next[pg.cue] += pg.words().len();
            i
        })
        .collect();
    out.cues = tr.cues.iter().map(|c| CueDump { start: c.start, end: c.end, words: Vec::new() }).collect();
    for (pg, &f) in tr.pages.iter().zip(&first) {
        let words = &mut out.cues[pg.cue].words;
        words.extend(pg.words().into_iter().enumerate().map(|(k, w)| WordDump {
            index: f + k,
            text: w.text.clone(),
            start: w.start,
            end: w.end,
        }));
    }
    out.word_count = out.cues.iter().map(|c| c.words.len()).sum();
    out.pages = tr.pages.iter().zip(&first).map(|(pg, &f)| page(tr.preset, pg, f)).collect();
    out.at = at
        .iter()
        .map(|&time| match tr.page_at(time) {
            Some((i, pg)) => Sample {
                time,
                page: Some(i),
                cue: Some(pg.cue),
                word: captions::active_word(tr.preset, pg, time).map(|w| first[i] + w),
            },
            None => Sample { time, page: None, cue: None, word: None },
        })
        .collect();
}

fn page(preset: captions::Preset, pg: &Page, first: usize) -> PageDump {
    let lines: Vec<String> =
        pg.lines.iter().map(|l| l.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")).collect();
    PageDump {
        cue: pg.cue,
        start: pg.start,
        end: pg.end,
        first_word: first,
        text: pg.text(),
        lines,
        active: captions::active_spans(preset, pg)
            .into_iter()
            .map(|(start, end, w)| SpanDump { start, end, word: w.map(|w| first + w) })
            .collect(),
    }
}
