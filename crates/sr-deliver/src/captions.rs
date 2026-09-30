//! An output's captions in output time (SREP 13).
//!
//! With segments, the composition's caption tracks (inline, from a file, or transcribed) are mapped
//! through them: a cue meeting a segment's span appears once for that segment, over the output interval
//! its clipped ends map to (swapped when the map runs backwards), keeping only the words inside the
//! span; a cue left without words is dropped. A track that transcribes an audio track contributes only
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
                    mapped.extend(cues.iter().filter_map(|c| map_cue(c, tm, i)));
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

/// The composition interval segment `i` plays, and output time as a function of composition time in it.
fn span(tm: &TimeMap, i: usize) -> (f64, f64) {
    let d = tm.segments[i].duration;
    match tm.span(i) {
        Some((a, s)) => (a, a + s * d),
        None => {
            let n = ((d * 1000.0).ceil() as usize).max(1);
            (0..=n)
                .map(|k| tm.unclamped(i, d * k as f64 / n as f64))
                .fold((f64::MAX, f64::MIN), |(lo, hi), c| (lo.min(c), hi.max(c)))
        }
    }
}

/// The first output time at which segment `i` shows composition time `c` (inside its span).
fn output_time(tm: &TimeMap, i: usize, c: f64) -> f64 {
    let seg = &tm.segments[i];
    match tm.span(i) {
        Some((a, s)) => seg.start + (c - a) / s,
        None => tm.shows(i, c).unwrap_or(seg.start + seg.duration),
    }
}

/// Cue `c` as segment `i` shows it, if the segment's span meets it.
fn map_cue(c: &Cue, tm: &TimeMap, i: usize) -> Option<Cue> {
    let (lo, hi) = span(tm, i);
    let (p, q) = (c.start.max(lo), c.end.min(hi));
    if q <= p {
        return None;
    }
    let at = |x: f64| output_time(tm, i, x);
    let ends = |a: f64, b: f64| {
        let (x, y) = (at(a), at(b));
        if y < x {
            (y, x)
        } else {
            (x, y)
        }
    };
    let (start, end) = ends(p, q);
    let mut words: Vec<Word> = c
        .words
        .iter()
        .filter(|w| w.end > p && w.start < q)
        .map(|w| {
            let (start, end) = ends(w.start.max(p), w.end.min(q));
            Word { start, end, ..w.clone() }
        })
        .collect();
    if !c.words.is_empty() && words.is_empty() {
        return None;
    }
    words.sort_by(|a, b| a.start.total_cmp(&b.start));
    let text = if words.len() < c.words.len() {
        words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    } else {
        c.text.clone()
    };
    Some(Cue { start, end, text, words, ..c.clone() })
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
}
