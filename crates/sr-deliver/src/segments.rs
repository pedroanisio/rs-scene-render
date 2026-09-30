//! The time map of an output with `<segment>` children (SREP 13).
//!
//! Output time t runs over the segments in document order. Segment i starts at
//! Tᵢ = d₁ + … + dᵢ₋₁ and maps its local time u = t − Tᵢ to composition time:
//! a + s·u for a span (a = `from` or its marker, s = `speed`), or its
//! `timeRemap` curve. Outside a segment (at a transition) a span continues
//! linearly and a remap continues with the slope of its end keys; every
//! composition time is then clamped to the project. Frame k is at output time
//! k/fps and belongs to the segment whose half-open interval holds it.

use sr_eval::channel::{Channel, ChannelSpec, Lookup};
use sr_eval::value::PropKind;
use sr_eval::Program;
use sr_model::model as m;

/// How one segment maps its local time to composition time.
#[derive(Debug, Clone)]
enum Map {
    /// a + s·u.
    Span { a: f64, s: f64 },
    /// A `timeRemap` curve of local time, extended linearly past its end keys.
    Remap(Box<Channel>),
}

/// An animated or fixed coordinate of the reframing focus.
#[derive(Debug, Clone)]
enum Focus {
    Fixed(f64),
    Animated(Box<Channel>),
}

/// One segment, placed on the output's timeline.
#[derive(Debug, Clone)]
pub struct Segment {
    /// Output time where the segment starts.
    pub start: f64,
    /// Output duration.
    pub duration: f64,
    map: Map,
    focus: [Option<Focus>; 2],
    /// The segment's element.
    pub elem: m::Segment,
}

/// The segments of an output.
#[derive(Debug, Clone)]
pub struct TimeMap {
    /// Segments in playing order.
    pub segments: Vec<Segment>,
    /// Output duration: the sum of the segments' durations.
    pub duration: f64,
    /// The project's duration: composition times are clamped to [0, project].
    project: f64,
}

/// A frame of the output: its output time, segment, time in the segment and composition time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameTime {
    pub output: f64,
    pub segment: usize,
    pub local: f64,
    pub composition: f64,
}

struct Markers<'a>(&'a Program);

impl Lookup for Markers<'_> {
    fn token(&self, _: &str) -> Option<[f64; 4]> {
        None
    }
    fn marker(&self, id: &str) -> Option<f64> {
        sr_eval::eval::marker(self.0, id)
    }
}

fn channel(keys: &[m::Key], default: m::Curve, p: &Program) -> Result<Channel, String> {
    let spec = ChannelSpec {
        keys,
        default,
        before: m::Extrapolation::Linear,
        after: m::Extrapolation::Linear,
        additive: false,
        time_base: m::TimeBase::Local,
        kind: PropKind::Number(Default::default()),
    };
    Channel::compile(&spec, &Markers(p))
}

impl TimeMap {
    /// The output's time map, or `None` when it has no segments.
    pub fn of(p: &Program, output: &m::Output) -> Result<Option<TimeMap>, String> {
        let mut segments = Vec::new();
        let mut start = 0.0;
        let marker = |id: &str| sr_eval::eval::marker(p, id).ok_or_else(|| format!("no marker {id:?}"));
        for c in &output.children {
            let m::OutputChild::Segment(e) = c else { continue };
            let label = e.id.clone().unwrap_or_else(|| format!("segment {}", segments.len() + 1));
            let remap = e.children.iter().find_map(|c| match c {
                m::SegmentChild::TimeRemap(r) => Some(r),
                _ => None,
            });
            let (map, duration) = match remap {
                Some(r) => {
                    let ch = channel(&r.keys, r.default_interpolation, p).map_err(|er| format!("{label}: {er}"))?;
                    let end = r.keys.iter().map(|k| k.time).fold(0.0f64, f64::max);
                    (Map::Remap(Box::new(ch)), end)
                }
                None => {
                    let a = match (&e.from, &e.from_marker) {
                        (Some(v), _) => *v,
                        (None, Some(mk)) => marker(mk)?,
                        _ => return Err(format!("{label}: no start")),
                    };
                    let b = match (&e.to, &e.to_marker) {
                        (Some(v), _) => *v,
                        (None, Some(mk)) => marker(mk)?,
                        _ => return Err(format!("{label}: no end")),
                    };
                    if b <= a {
                        return Err(format!("{label}: ends at {b} s, before it starts at {a} s"));
                    }
                    let s = e.speed.get();
                    (Map::Span { a, s }, (b - a) / s)
                }
            };
            if duration <= 0.0 {
                return Err(format!("{label}: plays for no time"));
            }
            let mut focus = [e.focus_x.map(|v| Focus::Fixed(v.get())), e.focus_y.map(|v| Focus::Fixed(v.get()))];
            for c in &e.children {
                let m::SegmentChild::Animate(an) = c else { continue };
                let k = match an.property.as_str() {
                    "focusX" => 0,
                    "focusY" => 1,
                    other => return Err(format!("{label}: animate property {other:?}: only focusX and focusY")),
                };
                let ch = channel(&an.keys, an.default_interpolation, p).map_err(|er| format!("{label}: {er}"))?;
                focus[k] = Some(Focus::Animated(Box::new(ch)));
            }
            segments.push(Segment { start, duration, map, focus, elem: e.clone() });
            start += duration;
        }
        if segments.is_empty() {
            return Ok(None);
        }
        Ok(Some(TimeMap { segments, duration: start, project: p.duration }))
    }

    /// Composition time of segment `i` at local time `u`, extended past the segment's ends and clamped.
    pub fn composition(&self, i: usize, u: f64) -> f64 {
        let c = match &self.segments[i].map {
            Map::Span { a, s } => a + s * u,
            Map::Remap(ch) => ch.eval(u).as_num().unwrap_or(0.0),
        };
        c.clamp(0.0, self.project)
    }

    /// The segment holding output time `t` (intervals are half-open; the end belongs to the last).
    /// A time within a nanosecond of a segment's end is the next segment's start: durations such as
    /// 17.1 − 17.0 are not exact in binary, and a frame on a join must show the incoming segment.
    pub fn segment_at(&self, t: f64) -> usize {
        let n = self.segments.len();
        self.segments.iter().position(|s| t < s.start + s.duration - 1e-9).unwrap_or(n - 1)
    }

    /// Output time `t` as a frame time.
    pub fn at(&self, t: f64) -> FrameTime {
        let i = self.segment_at(t);
        let local = (t - self.segments[i].start).max(0.0);
        FrameTime { output: t, segment: i, local, composition: self.composition(i, local) }
    }

    /// The output's frames at `fps`: ⌈T·fps⌉ of them, frame k at k/fps.
    pub fn frames(&self, fps: f64) -> Vec<FrameTime> {
        let n = (self.duration * fps - 1e-9).ceil().max(1.0) as usize;
        (0..n).map(|k| self.at(k as f64 / fps)).collect()
    }

    /// Composition time for a sub-frame sample `dt` output seconds away from frame time `f`, in the
    /// frame's own segment (motion blur covers the composition time its shutter spans at that speed).
    pub fn sample(&self, f: &FrameTime, dt: f64) -> f64 {
        self.composition(f.segment, f.local + dt)
    }

    /// The reframing focus of a frame: each coordinate from its segment when given, else `default`.
    pub fn focus(&self, f: &FrameTime, default: [f64; 2]) -> [f64; 2] {
        let s = &self.segments[f.segment];
        let pick = |k: usize| match &s.focus[k] {
            Some(Focus::Fixed(v)) => *v,
            Some(Focus::Animated(ch)) => ch.eval(f.local).as_num().unwrap_or(default[k]).clamp(0.0, 1.0),
            None => default[k],
        };
        [pick(0), pick(1)]
    }

    /// Whether any segment has a transition child.
    pub fn has_transitions(&self) -> bool {
        self.segments.iter().any(|s| s.elem.children.iter().any(|c| matches!(c, m::SegmentChild::Transition(_))))
    }

    /// Whether any segment moves the reframing focus.
    pub fn has_focus(&self) -> bool {
        self.segments.iter().any(|s| s.focus.iter().any(Option::is_some))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(outputs: &str) -> (sr_eval::Evaluator, m::Output) {
        let xml = format!(
            r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="3" background="#000000"/>
              {outputs}<markers><marker id="m1" time="1"/></markers><composition/></scene>"##
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let out = ev.program().scene.outputs.first().cloned().unwrap();
        (ev, out)
    }

    #[test]
    fn spans_play_in_order_at_their_speeds() {
        let (p, o) = program(
            r#"<output path="a.mp4" codec="h264"><segment from="2" to="3"/><segment from="0" to="1" speed="2"/></output>"#,
        );
        let tm = TimeMap::of(p.program(), &o).unwrap().unwrap();
        assert_eq!(tm.duration, 1.5);
        // the SREP's worked values: output 0.5 → 2.5, 1.25 → 0.5
        assert_eq!(tm.at(0.5).composition, 2.5);
        assert_eq!(tm.at(1.25).composition, 0.5);
        // a frame exactly at the join belongs to the incoming segment
        let j = tm.at(1.0);
        assert_eq!((j.segment, j.composition), (1, 0.0));
        // ⌈1.5 × 10⌉ frames
        assert_eq!(tm.frames(10.0).len(), 15);
    }

    #[test]
    fn markers_remaps_and_clamping() {
        let (p, o) = program(
            r#"<output path="a.mp4" codec="h264">
                 <segment fromMarker="m1" to="2"/>
                 <segment><timeRemap><key time="0" value="3"/><key time="1" value="1"/></timeRemap></segment>
               </output>"#,
        );
        let tm = TimeMap::of(p.program(), &o).unwrap().unwrap();
        assert_eq!(tm.segments[0].duration, 1.0);
        assert_eq!(tm.at(0.0).composition, 1.0);
        // backwards: segment time 0.25 → 2.5
        assert!((tm.at(1.25).composition - 2.5).abs() < 1e-12);
        // extended past its start (slope −2) and clamped to the project's 3 s
        assert_eq!(tm.composition(1, -0.5), 3.0);
        // a span extended before its start is clamped at 0
        assert_eq!(tm.composition(0, -5.0), 0.0);
    }

    #[test]
    fn frame_counts_round_up_without_drift() {
        let (p, o) = program(
            r#"<output path="a.mp4" codec="h264"><segment from="0" to="1" speed="3"/><segment from="1" to="2" speed="3"/></output>"#,
        );
        let tm = TimeMap::of(p.program(), &o).unwrap().unwrap();
        // 2/3 s at 10 fps: ⌈6.67⌉ = 7 frames, frame 4 at 0.4 s is in the second segment (starts at 1/3)
        let f = tm.frames(10.0);
        assert_eq!(f.len(), 7);
        assert_eq!(f[3].segment, 0);
        assert_eq!(f[4].segment, 1);
        assert!((f[4].composition - (1.0 + 3.0 * (0.4 - 1.0 / 3.0))).abs() < 1e-12);
    }

    #[test]
    fn a_frame_on_an_inexact_join_shows_the_incoming_segment() {
        // 17.1 − 17.0 = 0.10000000000000142 in binary; frame 3 at 30 fps (0.1 s) is on the join
        let (p, o) = program(
            r#"<output path="a.mp4" codec="h264"><segment from="1.0" to="1.1"/><segment from="2.0" to="2.1"/></output>"#,
        );
        let tm = TimeMap::of(p.program(), &o).unwrap().unwrap();
        let f = tm.frames(30.0);
        assert_eq!((f[3].segment, f[3].composition), (1, 2.0));
        let (p, o) = program(
            r#"<output path="a.mp4" codec="h264"><segment from="1.7" to="1.8"/><segment from="2.0" to="2.1"/></output>"#,
        );
        let tm = TimeMap::of(p.program(), &o).unwrap().unwrap();
        assert_eq!(tm.frames(30.0)[3].segment, 1);
    }

    #[test]
    fn focus_is_fixed_animated_or_the_layouts() {
        let (p, o) = program(
            r#"<output path="a.mp4" codec="h264">
                 <segment from="0" to="1" focusX="0.2"/>
                 <segment from="1" to="2"><animate property="focusY"><key time="0" value="0"/><key time="1" value="1"/></animate></segment>
               </output>"#,
        );
        let tm = TimeMap::of(p.program(), &o).unwrap().unwrap();
        assert_eq!(tm.focus(&tm.at(0.5), [0.5, 0.5]), [0.2, 0.5]);
        assert_eq!(tm.focus(&tm.at(1.25), [0.5, 0.5]), [0.5, 0.25]);
        assert!(tm.has_focus() && !tm.has_transitions());
    }
}
