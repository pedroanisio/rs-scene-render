//! An output's captions in output time (SREP 13).
//!
//! With segments, the composition's caption tracks (inline, from a file, or transcribed) are mapped
//! through them: each cue appears in every output interval that shows its composition span,
//! including repeated visits and holds, with word times mapped and clipped to each interval.
//! A cue left without words is dropped. A track that transcribes an audio track contributes only
//! while that track is selected and the segment is not muted. The output's own caption tracks are in
//! output time already. Every track comes out with inline cues, for burning and for sidecar files.

use sr_eval::Program;
use sr_model::model as m;
use sr_text::captions::{Cue, Word};

use crate::segments::TimeMap;
use crate::DeliverError;

/// A mapped cue shown for less than this is reported.
const SHORTEST: f64 = 0.7;

/// The caption tracks of an output, in output time.
pub struct OutputCaptions {
    /// Mapped composition tracks (with segments) and the output's own tracks, with inline cues.
    pub tracks: Vec<m::CaptionTrack>,
    /// Mapped cues too short to read.
    pub warnings: Vec<String>,
}

impl OutputCaptions {
    /// Whether anything would be burned in: the track `burnCaptions` names, or else any track that
    /// is not sidecar-only.
    pub fn burns(&self, output: &m::Output) -> bool {
        match &output.burn_captions {
            Some(id) => self.tracks.iter().any(|t| t.id == *id),
            None => self.tracks.iter().any(|t| t.mode.as_str() != "sidecar" && !t.cues.is_empty()),
        }
    }
}

/// The output's caption tracks in output time: with a time map, the composition's mapped through it,
/// then the output's own.
pub fn output_captions(p: &Program, output: &m::Output, tm: Option<&TimeMap>) -> Result<OutputCaptions, DeliverError> {
    let base = p.base_dirs.first().cloned().unwrap_or_default();
    let mut tracks = Vec::new();
    let mut warnings = Vec::new();
    if let (Some(tm), Some(caps)) = (tm, &p.scene.captions) {
        for tr in &caps.caption_tracks {
            let cues = sr_gpu::text::track_cues(tr, &base).map_err(DeliverError::Invalid)?;
            // a transcription follows its audio: nothing while that track is not played
            let hears = tr.transcribe.as_deref().is_none_or(|id| crate::segment_audio::track_selected(p, output, id));
            let mut mapped = Vec::new();
            if hears {
                for (i, seg) in tm.segments.iter().enumerate() {
                    if tr.transcribe.is_some() && seg.elem.audio == m::SegmentAudio::Mute {
                        continue;
                    }
                    let map = CueMap::new(tm, i);
                    mapped.extend(cues.iter().flat_map(|c| map.cues(c, tr.line_breaks.as_str() == "source")));
                }
            }
            mapped.sort_by(|a, b| a.start.total_cmp(&b.start));
            for c in mapped.iter().filter(|c| c.end - c.start < SHORTEST) {
                warnings.push(format!(
                    "caption track {}: the cue at {:.2} s shows for {:.2} s, under {SHORTEST} s",
                    tr.id,
                    c.start,
                    c.end - c.start
                ));
            }
            tracks.push(inline(tr, &mapped));
        }
    }
    for c in &output.children {
        let m::OutputChild::CaptionTrack(tr) = c else { continue };
        let cues = sr_gpu::text::track_cues(tr, &base).map_err(DeliverError::Invalid)?;
        tracks.push(inline(tr, &cues));
    }
    Ok(OutputCaptions { tracks, warnings })
}

/// `tr` with `cues` inline, and no file or transcription to read.
fn inline(tr: &m::CaptionTrack, cues: &[Cue]) -> m::CaptionTrack {
    let mut t = tr.clone();
    t.src = None;
    t.transcribe = None;
    t.cache = None;
    t.cache_sha256 = None;
    t.cues = cues
        .iter()
        .map(|c| m::Cue {
            loc: Default::default(),
            start: c.start,
            end: c.end,
            text: Some(c.text.clone()),
            speaker: c.speaker.clone(),
            style: c.style.clone(),
            position: c.position.clone(),
            words: c
                .words
                .iter()
                .map(|w| m::Word {
                    loc: Default::default(),
                    start: w.start,
                    end: w.end,
                    text: w.text.clone(),
                    emphasis: w.emphasis,
                })
                .collect(),
        })
        .collect();
    t
}

/// Samples shared by all cues/words of a segment. Keys split the grid so short
/// holds and jumps are represented too; crossings are refined against the actual curve.
struct CueMap<'a> {
    tm: &'a TimeMap,
    segment: usize,
    samples: Vec<(f64, f64)>,
}

impl<'a> CueMap<'a> {
    fn new(tm: &'a TimeMap, segment: usize) -> Self {
        let mut samples = Vec::new();
        if tm.span(segment).is_none() {
            let duration = tm.segments[segment].duration;
            samples.push((0.0, tm.composition(segment, 0.0)));
            let mut start = 0.0;
            for end in tm.keys(segment).iter().copied().filter(|&t| t > 0.0 && t < duration).chain([duration]) {
                let n = ((end - start) * 1000.0).ceil().max(64.0) as usize;
                for k in 1..=n {
                    let t = start + (end - start) * k as f64 / n as f64;
                    samples.push((t, tm.composition(segment, t)));
                }
                start = end;
            }
        }
        Self { tm, segment, samples }
    }

    /// All local output intervals whose composition times fall in [lo, hi).
    fn intervals(&self, lo: f64, hi: f64) -> Vec<(f64, f64)> {
        if hi <= lo {
            return Vec::new();
        }
        let i = self.segment;
        let duration = self.tm.segments[i].duration;
        if let Some((a, speed)) = self.tm.span(i) {
            let start = if lo <= self.tm.composition(i, 0.0) { 0.0 } else { (lo - a) / speed };
            let end = if hi > self.tm.composition(i, duration) { duration } else { (hi - a) / speed };
            let (start, end) = (start.clamp(0.0, duration), end.clamp(0.0, duration));
            return if start < end { vec![(start, end)] } else { Vec::new() };
        }
        let mut cuts = vec![0.0, duration];
        for pair in self.samples.windows(2) {
            let [(t0, v0), (t1, v1)] = [pair[0], pair[1]];
            for bound in [lo, hi] {
                if (v0 < bound) == (v1 < bound) {
                    continue;
                }
                let (mut left, mut right) = (t0, t1);
                for _ in 0..48 {
                    let mid = (left + right) * 0.5;
                    if (self.tm.composition(i, mid) < bound) == (v0 < bound) {
                        left = mid;
                    } else {
                        right = mid;
                    }
                }
                // A jump across both bounds gives the same cut twice. Midpoint
                // classification below excludes cues skipped by that jump.
                cuts.push(right);
            }
        }
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-10);
        let mut intervals: Vec<(f64, f64)> = Vec::new();
        for pair in cuts.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let value = self.tm.composition(i, (a + b) * 0.5);
            if value < lo || value >= hi {
                continue;
            }
            if let Some(last) = intervals.last_mut().filter(|last| (last.1 - a).abs() < 1e-10) {
                last.1 = b;
            } else {
                intervals.push((a, b));
            }
        }
        intervals
    }

    fn cues(&self, c: &Cue, source_lines: bool) -> Vec<Cue> {
        let breaks = if source_lines { sr_text::captions::source_words(c).1 } else { Vec::new() };
        let origin = self.tm.segments[self.segment].start;
        let word_times: Vec<_> =
            c.words.iter().map(|w| self.intervals(w.start.max(c.start), w.end.min(c.end))).collect();
        self.intervals(c.start, c.end)
            .into_iter()
            .filter_map(|(start, end)| {
                let mut words = Vec::new();
                let mut retained = 0;
                for (wi, (w, times)) in c.words.iter().zip(&word_times).enumerate() {
                    let before = words.len();
                    for &(a, b) in times {
                        let (a, b) = (a.max(start), b.min(end));
                        if a < b {
                            words.push((Word { start: origin + a, end: origin + b, ..w.clone() }, wi));
                        }
                    }
                    retained += usize::from(words.len() > before);
                }
                if !c.words.is_empty() && words.is_empty() {
                    return None;
                }
                words.sort_by(|a, b| a.0.start.total_cmp(&b.0.start));
                let text = if source_lines {
                    // Carry source-line membership with words through trims and reversing remaps.
                    let mut text = String::new();
                    let mut previous = None;
                    for (word, wi) in &words {
                        let line = breaks.partition_point(|start| start <= wi);
                        if let Some(last) = previous {
                            text.push(if last == line { ' ' } else { '\n' });
                        }
                        text.push_str(&word.text);
                        previous = Some(line);
                    }
                    text
                } else if retained < c.words.len() {
                    words.iter().map(|(w, _)| w.text.as_str()).collect::<Vec<_>>().join(" ")
                } else {
                    c.text.clone()
                };
                Some(Cue {
                    start: origin + start,
                    end: origin + end,
                    text,
                    words: words.into_iter().map(|(w, _)| w).collect(),
                    ..c.clone()
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(outputs: &str, captions: &str) -> (sr_eval::Evaluator, m::Output) {
        let xml = format!(
            r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="10" background="#000000"/>
              {outputs}<composition/>{captions}</scene>"##
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let out = ev.program().scene.outputs[0].clone();
        (ev, out)
    }

    const WORDS: &str = r#"<captions><captionTrack id="cc" language="en">
        <cue start="1" end="3"><word start="1" end="1.5" text="one"/><word start="1.6" end="2.4" text="two"/><word start="2.5" end="3" text="three"/></cue>
        <cue start="5" end="6" text="later"/>
      </captionTrack></captions>"#;

    #[test]
    fn reversing_remap_repeats_cues_and_word_timings() {
        let (ev, o) = eval(
            r#"<output path="a.mp4" codec="h264"><segment><timeRemap>
              <key time="0" value="0"/><key time="2" value="4"/><key time="4" value="0"/>
              </timeRemap></segment></output>"#,
            r#"<captions><captionTrack id="cc" language="en"><cue start="1" end="2">
              <word start="1" end="1.5" text="one"/><word start="1.5" end="2" text="two"/>
              </cue></captionTrack></captions>"#,
        );
        let tm = TimeMap::of(ev.program(), &o).unwrap().unwrap();
        let caps = output_captions(ev.program(), &o, Some(&tm)).unwrap();
        let cues = &caps.tracks[0].cues;
        assert_eq!(cues.len(), 2, "{cues:?}");
        for (cue, (a, b)) in cues.iter().zip([(0.5, 1.0), (3.0, 3.5)]) {
            assert!((cue.start - a).abs() < 1e-8 && (cue.end - b).abs() < 1e-8);
            assert_eq!(cue.words.len(), 2);
        }
        assert_eq!(cues[1].words[0].text, "two");
        assert!((cues[1].words[0].start - 3.0).abs() < 1e-8);
        assert!((cues[1].words[0].end - 3.25).abs() < 1e-8);
        assert!((cues[1].words[1].end - 3.5).abs() < 1e-8);
    }

    #[test]
    fn held_remap_omits_skipped_cues_and_keeps_each_visible_hold() {
        let (ev, o) = eval(
            r#"<output path="a.mp4" codec="h264"><segment><timeRemap>
              <key time="0" value="1" interpolation="hold"/><key time="1" value="3" interpolation="hold"/>
              <key time="2" value="1" interpolation="hold"/><key time="3" value="1"/>
              </timeRemap></segment></output>"#,
            r#"<captions><captionTrack id="cc" language="en"><cue start="0.5" end="1.5" text="held"/>
              <cue start="1.6" end="2.5" text="skipped"/></captionTrack></captions>"#,
        );
        let tm = TimeMap::of(ev.program(), &o).unwrap().unwrap();
        let caps = output_captions(ev.program(), &o, Some(&tm)).unwrap();
        let cues = &caps.tracks[0].cues;
        assert_eq!(cues.len(), 2, "{cues:?}");
        for (cue, (a, b)) in cues.iter().zip([(0.0, 1.0), (2.0, 3.0)]) {
            assert_eq!(cue.text.as_deref(), Some("held"));
            assert!((cue.start - a).abs() < 1e-8 && (cue.end - b).abs() < 1e-8, "{cue:?}");
        }
    }

    #[test]
    fn eased_revisits_and_short_holds_follow_the_actual_remap() {
        for keys in [
            r#"<key time="0" value="0" interpolation="ease-in-out"/><key time="2" value="4" interpolation="ease-in-out"/><key time="4" value="0"/>"#,
            r#"<key time="0" value="0" interpolation="hold"/><key time="0.0001" value="1.5" interpolation="hold"/><key time="0.0002" value="3" interpolation="hold"/><key time="4" value="3"/>"#,
        ] {
            let (ev, o) = eval(
                &format!(
                    r#"<output path="a.mp4" codec="h264"><segment><timeRemap>{keys}</timeRemap></segment></output>"#
                ),
                r#"<captions><captionTrack id="cc" language="en"><cue start="1" end="2" text="visible"/></captionTrack></captions>"#,
            );
            let tm = TimeMap::of(ev.program(), &o).unwrap().unwrap();
            let caps = output_captions(ev.program(), &o, Some(&tm)).unwrap();
            for t in (0..800).map(|k| (k as f64 + 0.25) / 200.0).chain([0.00005, 0.00015, 0.00025]) {
                let source = tm.composition(0, t);
                let visible = caps.tracks[0].cues.iter().any(|c| c.start <= t && t < c.end);
                assert_eq!(visible, (1.0..2.0).contains(&source), "output {t}, composition {source}");
            }
        }
    }

    #[test]
    fn freeze_keeps_active_cues_and_words_for_the_segment() {
        let (ev, o) = eval(
            r#"<output path="a.mp4" codec="h264"><segment from="8" to="9"/>
              <segment><timeRemap><key time="0" value="2"/><key time="2" value="2"/></timeRemap></segment>
              </output>"#,
            r#"<captions><captionTrack id="cc" language="en">
              <cue start="1" end="3" text="held"/>
              <cue start="1" end="3"><word start="1" end="2" text="before"/>
                <word start="2" end="2.5" text="during"/><word start="2.5" end="3" text="after"/></cue>
              <cue start="0" end="2" text="ended"/><cue start="3" end="4" text="later"/>
              </captionTrack></captions>"#,
        );
        let tm = TimeMap::of(ev.program(), &o).unwrap().unwrap();
        let caps = output_captions(ev.program(), &o, Some(&tm)).unwrap();
        let cues = &caps.tracks[0].cues;
        assert_eq!(cues.len(), 2);
        assert_eq!((cues[0].start, cues[0].end, cues[0].text.as_deref()), (1.0, 3.0, Some("held")));
        assert_eq!((cues[1].start, cues[1].end, cues[1].text.as_deref()), (1.0, 3.0, Some("during")));
        assert_eq!(cues[1].words.len(), 1);
        assert_eq!((cues[1].words[0].start, cues[1].words[0].end), (1.0, 3.0));
        assert!(caps.warnings.is_empty());
    }

    #[test]
    fn cues_keep_the_words_inside_the_span_at_output_times() {
        // the span 2..6 at speed 2 starts at output 0: "two" is cut to 2..2.4, "three" at 0.25..0.5
        let (ev, o) = eval(r#"<output path="a.mp4" codec="h264"><segment from="2" to="6" speed="2"/></output>"#, WORDS);
        let tm = TimeMap::of(ev.program(), &o).unwrap().unwrap();
        let caps = output_captions(ev.program(), &o, Some(&tm)).unwrap();
        let cues = &caps.tracks[0].cues;
        assert_eq!(cues.len(), 2);
        assert_eq!((cues[0].start, cues[0].end), (0.0, 0.5));
        assert_eq!(cues[0].text.as_deref(), Some("two three"));
        let w: Vec<(f64, f64)> = cues[0].words.iter().map(|w| (w.start, w.end)).collect();
        let near = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9;
        assert!(w.len() == 2 && near(w[0], (0.0, 0.2)) && near(w[1], (0.25, 0.5)), "{w:?}");
        assert_eq!((cues[1].start, cues[1].end), (1.5, 2.0));
        // both are shorter than 0.7 s
        assert_eq!(caps.warnings.len(), 2);
    }

    #[test]
    fn a_cue_appears_once_per_segment_and_backwards_maps_swap() {
        let (ev, o) = eval(
            r#"<output path="a.mp4" codec="h264">
                 <segment from="5" to="6"/>
                 <segment><timeRemap><key time="0" value="6" interpolation="linear"/><key time="1" value="5"/></timeRemap></segment>
               </output>"#,
            WORDS,
        );
        let tm = TimeMap::of(ev.program(), &o).unwrap().unwrap();
        let cues = output_captions(ev.program(), &o, Some(&tm)).unwrap().tracks.remove(0).cues;
        assert_eq!(cues.len(), 2);
        assert_eq!((cues[0].start, cues[0].end), (0.0, 1.0));
        assert!((cues[1].start - 1.0).abs() < 1e-9 && (cues[1].end - 2.0).abs() < 1e-9);
    }

    #[test]
    fn transcriptions_follow_the_selected_unmuted_audio() {
        use sha2::Digest;
        let json = r#"{"segments":[{"start":0,"end":1,"text":"x"}]}"#;
        let sha: String = sha2::Sha256::digest(json.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        let caps = format!(
            r#"<captions><captionTrack id="vo-cc" language="en" transcribe="vo" cache="vo.json" cacheSha256="{sha}"/></captions>"#
        );
        let mix = r#"<audioMix><audioTrack id="vo" asset="a" role="voiceover"/></audioMix>"#;
        let assets = r#"<assets><audio id="a" src="a.wav"/></assets>"#;
        for (attrs, audio, want) in
            [("", "stretch", 1), (r#" audioRoles="music""#, "stretch", 0), (r#" audioRoles="voiceover""#, "mute", 0)]
        {
            let xml = format!(
                r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="10" background="#000000"/>
                  <output path="a.mp4" codec="h264"{attrs}><segment from="0" to="2" audio="{audio}"/></output>
                  {assets}<composition/>{mix}{caps}</scene>"##
            );
            let dir = std::env::temp_dir().join(format!("sr-captions-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("vo.json"), json).unwrap();
            let path = dir.join("scene.xml");
            std::fs::write(&path, xml).unwrap();
            let doc = sr_model::load_file(&path, &sr_model::LoadOptions::without_assets())
                .unwrap_or_else(|e| panic!("{e:?}"));
            let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
            let o = ev.program().scene.outputs[0].clone();
            let tm = TimeMap::of(ev.program(), &o).unwrap().unwrap();
            let caps = output_captions(ev.program(), &o, Some(&tm)).unwrap();
            assert_eq!(caps.tracks[0].cues.len(), want, "{attrs} {audio}");
        }
    }
    #[test]
    fn source_line_breaks_follow_trimmed_and_reversed_words() {
        let track = r#"<captions><captionTrack id="cc" language="en" lineBreaks="source">
          <cue start="0" end="4" text="aa bb&#10;cc dd"/></captionTrack></captions>"#;
        for (segment, expected) in [
            (r#"<segment from="1" to="4"/>"#, "bb\ncc dd"),
            (
                r#"<segment><timeRemap><key time="0" value="4"/><key time="4" value="0"/></timeRemap></segment>"#,
                "dd cc\nbb aa",
            ),
        ] {
            let (ev, out) = eval(&format!(r#"<output path="a.mp4" codec="h264">{segment}</output>"#), track);
            let tm = TimeMap::of(ev.program(), &out).unwrap().unwrap();
            let caps = output_captions(ev.program(), &out, Some(&tm)).unwrap();
            assert_eq!(caps.tracks[0].line_breaks.as_str(), "source");
            assert_eq!(caps.tracks[0].cues[0].text.as_deref(), Some(expected));
            let cues = sr_gpu::text::track_cues(&caps.tracks[0], std::path::Path::new("")).unwrap();
            let pages = sr_text::captions::paginate_with_line_breaks(
                &cues,
                None,
                80,
                2,
                false,
                sr_text::captions::LineBreaks::Source,
            );
            assert_eq!(pages[0].text(), expected);
            assert_eq!(pages[0].words().len(), expected.split_whitespace().count());
        }
    }
}
