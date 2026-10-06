//! Legibility checks for video text and captions (SREP 19): reading speed, display time and drawn text size,
//! measured on what the output draws and reported through the render report (SREP 18) as `LEG-SPEED`,
//! `LEG-SHORT` and `LEG-SIZE`, at the severity `accessibility/@legibilityCheck` sets. The check is off by default.
//!
//! - **Visible time** V: a caption cue's [start, end) within the output's range; a text layer's frames of the output
//!   in which it is drawn with nonzero opacity (its window, its ancestors' windows and clocks, conditions), counted
//!   at the output's frame rate.
//! - **Characters** N: Unicode code points of the text after NFC, line breaks not counted.
//! - **Drawn size**: the text's size after `autoFit`, times the layer's world scale (√|det| of its 2D world matrix),
//!   times the scale from the rendered frame to the output, in output pixels.
//!
//! Burned caption tracks are measured as laid out, page by page; sidecar tracks from their cues. Text revealed
//! progressively by an animator is measured over its whole visible time.

use std::collections::BTreeMap;

use sr_eval::{Evaluator, Program};
use sr_model::element::Element;
use sr_model::model as m;
use sr_model::values::{Length, LengthUnit};
use sr_model::Severity;
use unicode_normalization::UnicodeNormalization;

use crate::render_report::{code, Finding};

/// The default `minDisplayTime`, seconds (5/6 s, rounded as the schema declares it).
pub const DEFAULT_MIN_DISPLAY_TIME: f64 = 0.8333;

/// What `accessibility` asks the legibility check for.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// `warn` gives warnings, `error` errors.
    pub severity: Severity,
    /// `readingSpeed`, characters per second.
    pub reading_speed: Option<f64>,
    /// `minDisplayTime`, seconds.
    pub min_display_time: f64,
    /// `minTextSize`.
    pub min_text_size: Option<Length>,
}

impl Settings {
    /// The settings of the document's `accessibility` element; `None` when the check is off (the default) or the
    /// element is absent.
    pub fn of(p: &Program) -> Option<Settings> {
        let a = p.scene.metadata.as_ref()?.children.iter().find_map(|c| match c {
            m::MetadataChild::Accessibility(a) => Some(a),
            _ => None,
        })?;
        let text = |name: &str| a.get_attr(name).map(|v| v.to_string());
        let num = |name: &str| match a.get_attr(name) {
            Some(sr_model::element::AttrValue::Num(v)) => Some(v),
            _ => None,
        };
        let severity = match text("legibilityCheck").as_deref() {
            Some("warn") => Severity::Warning,
            Some("error") => Severity::Error,
            _ => return None,
        };
        let min_text_size = match a.get_attr("minTextSize") {
            Some(sr_model::element::AttrValue::Length(l)) => Some(l),
            Some(sr_model::element::AttrValue::Num(v)) => Some(Length { value: v, unit: LengthUnit::Px }),
            _ => None,
        };
        Some(Settings {
            severity,
            reading_speed: num("readingSpeed"),
            min_display_time: num("minDisplayTime").unwrap_or(DEFAULT_MIN_DISPLAY_TIME),
            min_text_size,
        })
    }
}

/// N: Unicode code points of `text` after NFC, line breaks not counted.
pub fn characters(text: &str) -> usize {
    text.nfc().filter(|c| !matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}' | '\u{85}' | '\u{0B}' | '\u{0C}')).count()
}

/// `len` resolved against an output frame of `size` pixels: `vw`, `vh`, `vmin`, `vmax`, pixels; a percentage is of
/// the frame's height, as `vh`.
pub fn resolve(len: &Length, size: [f64; 2]) -> f64 {
    let [w, h] = size;
    match len.unit {
        LengthUnit::Px => len.value,
        LengthUnit::Vw => len.value / 100.0 * w,
        LengthUnit::Vh | LengthUnit::Percent => len.value / 100.0 * h,
        LengthUnit::Vmin => len.value / 100.0 * w.min(h),
        LengthUnit::Vmax => len.value / 100.0 * w.max(h),
    }
}

/// One caption shown on screen: a burned page or a sidecar cue, in the time of the output's captions.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    /// From, seconds.
    pub start: f64,
    /// Until, seconds (exclusive).
    pub end: f64,
    /// What it says.
    pub text: String,
}

/// A caption track as the output shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ShownTrack {
    /// The track's id.
    pub id: String,
    /// `captionTrack/@readingSpeed`, which overrides `accessibility/@readingSpeed`.
    pub reading_speed: Option<f64>,
    /// Pages (burned) or cues (sidecar), in order.
    pub shown: Vec<Shown>,
}

impl ShownTrack {
    /// A track's `readingSpeed`, read through the element (the attribute exists from schema 1.2).
    pub fn reading_speed_of(tr: &m::CaptionTrack) -> Option<f64> {
        match tr.get_attr("readingSpeed") {
            Some(sr_model::element::AttrValue::Num(v)) => Some(v),
            _ => None,
        }
    }
}

/// `LEG-SPEED` and `LEG-SHORT` for every cue or page of `tracks` within `range` (output seconds).
pub fn caption_findings(s: &Settings, tracks: &[ShownTrack], range: [f64; 2]) -> Vec<Finding> {
    let mut out = Vec::new();
    for tr in tracks {
        let limit = tr.reading_speed.or(s.reading_speed);
        for c in &tr.shown {
            let (a, b) = (c.start.max(range[0]), c.end.min(range[1]));
            if b <= a {
                continue;
            }
            let v = b - a;
            let n = characters(&c.text);
            if let Some(limit) = limit {
                let speed = n as f64 / v;
                if speed > limit {
                    out.push(
                        Finding::node(
                            code::LEG_SPEED,
                            s.severity,
                            &tr.id,
                            format!(
                                "caption {:?} of track {}: {n} characters in {v:.3} s, {speed:.1} characters per second (limit {limit})",
                                c.text, tr.id
                            ),
                        )
                        .at_time(a, b)
                        .measuring(speed, Some(limit), "cps"),
                    );
                }
            }
            if v < s.min_display_time {
                out.push(
                    Finding::node(
                        code::LEG_SHORT,
                        s.severity,
                        &tr.id,
                        format!(
                            "caption {:?} of track {}: shown for {v:.3} s (minimum {} s)",
                            c.text, tr.id, s.min_display_time
                        ),
                    )
                    .at_time(a, b)
                    .measuring(v, Some(s.min_display_time), "s"),
                );
            }
        }
    }
    out
}

/// What the check saw of one text layer over the output's frames.
#[derive(Debug, Clone, Default)]
struct Seen {
    loc: Option<sr_model::Loc>,
    frames: usize,
    first: f64,
    last: f64,
    chars: usize,
    /// Smallest drawn size and the frames below the minimum: (smallest, first, last).
    small: Option<(f64, f64, f64)>,
}

/// `LEG-SPEED`, `LEG-SHORT` and `LEG-SIZE` for the text layers drawn at `times` (composition seconds of the output's
/// frames, `fps` frames per second), rendered at `frame` pixels and delivered at `output` pixels.
pub fn text_findings(
    ev: &Evaluator,
    s: &Settings,
    times: &[f64],
    fps: f64,
    frame: [f64; 2],
    output: [f64; 2],
) -> Vec<Finding> {
    let p = ev.program();
    let to_output = ((output[0] / frame[0].max(1e-9)) * (output[1] / frame[1].max(1e-9))).sqrt();
    let min_size = s.min_text_size.as_ref().map(|l| resolve(l, output));
    let mut tc = sr_gpu::text::TextCache::default();
    let mut seen: BTreeMap<String, Seen> = BTreeMap::new();
    for &t in times {
        let g = ev.evaluate_layout(t);
        for n in &g.nodes {
            if !n.draw || n.world_opacity <= 0.0 {
                continue;
            }
            let is_text = n
                .asset
                .as_deref()
                .and_then(|k| sr_gpu::text::asset_of(p, k))
                .is_some_and(|(a, _)| matches!(a, m::AssetsChild::Text(_)));
            if !is_text {
                continue;
            }
            let entry = seen.entry(n.id.to_string()).or_default();
            if entry.frames == 0 {
                entry.first = t;
            }
            entry.frames += 1;
            entry.last = t;
            entry.loc = Some(n.elem.loc());
            let need_layout = entry.frames == 1 || min_size.is_some();
            if !need_layout {
                continue;
            }
            let Some(lay) = sr_gpu::text_audit::layout_of(&mut tc, p, &g, n) else { continue };
            if entry.frames == 1 {
                entry.chars = characters(&lay.chars.iter().collect::<String>());
            }
            if let Some(min) = min_size {
                let [a, b, c, d, _, _] = n.world.0;
                let world = (a * d - b * c).abs().sqrt();
                let Some(base) = lay.styles.first() else { continue };
                let size = base.size * world * to_output;
                if size < min {
                    entry.small = Some(match entry.small {
                        Some((s0, f, _)) => (s0.min(size), f, t),
                        None => (size, t, t),
                    });
                }
            }
        }
    }
    let mut out = Vec::new();
    for (id, v) in seen {
        let at = |mut f: Finding| {
            f.at = Some(crate::render_report::At { offset: v.loc.map(|l| l.offset), id: Some(id.clone()) });
            f
        };
        let shown = v.frames as f64 / fps;
        let span = [v.first, v.last + 1.0 / fps];
        if let Some(limit) = s.reading_speed {
            let speed = v.chars as f64 / shown;
            if speed > limit {
                out.push(at(Finding::node(
                    code::LEG_SPEED,
                    s.severity,
                    &id,
                    format!("text {id:?}: {} characters drawn for {shown:.3} s, {speed:.1} characters per second (limit {limit})", v.chars),
                )
                .at_time(span[0], span[1])
                .measuring(speed, Some(limit), "cps")));
            }
        }
        if shown < s.min_display_time {
            out.push(at(Finding::node(
                code::LEG_SHORT,
                s.severity,
                &id,
                format!("text {id:?}: drawn for {shown:.3} s (minimum {} s)", s.min_display_time),
            )
            .at_time(span[0], span[1])
            .measuring(shown, Some(s.min_display_time), "s")));
        }
        if let (Some((size, from, to)), Some(min)) = (v.small, min_size) {
            out.push(at(Finding::node(
                code::LEG_SIZE,
                s.severity,
                &id,
                format!("text {id:?}: drawn at {size:.1} px, under the minimum of {min:.1} px"),
            )
            .at_time(from, to + 1.0 / fps)
            .measuring(size, Some(min), "px")));
        }
    }
    out
}
