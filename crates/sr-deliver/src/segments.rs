//! The time map of an output with `<segment>` children (SREP 13).
//!
//! Output time t runs over the segments in document order. Segment i starts at
//! Tᵢ = d₁ + … + dᵢ₋₁ and maps its local time u = t − Tᵢ to composition time:
//! a + s·u for a span (a = `from` or its marker, s = `speed`), or its
//! `timeRemap` curve. Outside a segment (at a transition) a span continues
//! linearly and a remap continues with the slope of its end keys; every
//! composition time is then clamped to the project. Frame k is at output time
//! k/fps and belongs to the segment whose half-open interval holds it. A
//! segment's `transition` child joins it to the next over a window around the
//! join, in which both sides are drawn, each through its own map.

use sr_eval::channel::{Channel, ChannelSpec, Lookup};
use sr_eval::value::PropKind;
use sr_eval::Program;
use sr_model::model as m;

/// How one segment maps its local time to composition time.
#[derive(Debug, Clone)]
enum Map {
    /// a + s·u.
    Span { a: f64, s: f64 },
    /// A `timeRemap` curve of local time, extended linearly past its end keys, and its key times.
    Remap(Box<Channel>, Vec<f64>),
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

/// A transition from segment `from` to the next, over output times [window.0, window.1).
#[derive(Debug, Clone)]
pub struct Join {
    pub from: usize,
    pub window: (f64, f64),
    ease: sr_eval::curve::Ease,
    /// The transition element.
    pub elem: m::Transition,
}

/// Both sides of a frame inside a join's window, and the transition's eased progress.
#[derive(Debug, Clone, Copy)]
pub struct JoinFrame<'a> {
    pub join: &'a Join,
    pub from: FrameTime,
    pub to: FrameTime,
    /// Eased progress in [0, 1].
    pub progress: f64,
    /// d(progress)/dt in 1/s of output time.
    pub velocity: f64,
}

/// The segments of an output.
#[derive(Debug, Clone)]
pub struct TimeMap {
    /// Segments in playing order.
    pub segments: Vec<Segment>,
    /// Transitions between consecutive segments, in order.
    pub joins: Vec<Join>,
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
                    let mut keys: Vec<f64> = r.keys.iter().map(|k| k.time).collect();
                    keys.sort_by(f64::total_cmp);
                    (Map::Remap(Box::new(ch), keys), end)
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
        // a transition joins its segment to the next over δ = min(duration, dᵢ, dᵢ₊₁), placed on the
        // join by its alignment; the last segment's transition has nothing to join
        let mut joins = Vec::new();
        for (i, w) in segments.windows(2).enumerate() {
            let Some(tr) = w[0].elem.children.iter().find_map(|c| match c {
                m::SegmentChild::Transition(t) => Some(t),
                _ => None,
            }) else {
                continue;
            };
            let cut = w[1].start;
            let d = tr.duration.get().min(w[0].duration).min(w[1].duration);
            let window = match tr.alignment {
                m::TransitionAlignment::Center => (cut - d / 2.0, cut + d / 2.0),
                m::TransitionAlignment::End => (cut - d, cut),
                m::TransitionAlignment::Start => (cut, cut + d),
            };
            let ease = sr_eval::curve::resolve(tr.curve, &Default::default());
            joins.push(Join { from: i, window, ease, elem: tr.clone() });
        }
        Ok(Some(TimeMap { segments, joins, duration: start, project: p.duration }))
    }

    /// One segment playing the composition from `a` to `b` as it is: an output without segments.
    pub fn span_of(p: &Program, a: f64, b: f64) -> Result<TimeMap, String> {
        if b <= a {
            return Err(format!("empty range {a}..{b}"));
        }
        let elem = m::Segment {
            loc: Default::default(),
            id: None,
            from: Some(a),
            to: Some(b),
            from_marker: None,
            to_marker: None,
            speed: m::PositiveDecimal::new(1.0).map_err(|e| e.to_string())?,
            audio: m::SegmentAudio::Stretch,
            focus_x: None,
            focus_y: None,
            children: Vec::new(),
        };
        let seg = Segment { start: 0.0, duration: b - a, map: Map::Span { a, s: 1.0 }, focus: [None, None], elem };
        Ok(TimeMap { segments: vec![seg], joins: Vec::new(), duration: b - a, project: p.duration })
    }

    /// Composition time of segment `i` at local time `u`, extended past the segment's ends and clamped.
    pub fn composition(&self, i: usize, u: f64) -> f64 {
        self.unclamped(i, u).clamp(0.0, self.project)
    }

    /// Composition time of segment `i` at local time `u`, extended past the segment's ends but not
    /// clamped: audio past the project's ends is silence, where the picture holds its end frame.
    pub fn unclamped(&self, i: usize, u: f64) -> f64 {
        match &self.segments[i].map {
            Map::Span { a, s } => a + s * u,
            Map::Remap(ch, _) => ch.eval(u).as_num().unwrap_or(0.0),
        }
    }

    /// The start and speed of segment `i` when it plays a span (`None` for a remap).
    pub fn span(&self, i: usize) -> Option<(f64, f64)> {
        match self.segments[i].map {
            Map::Span { a, s } => Some((a, s)),
            Map::Remap(..) => None,
        }
    }

    /// The key times of segment `i`'s remap, in local time (none for a span).
    pub fn keys(&self, i: usize) -> &[f64] {
        match &self.segments[i].map {
            Map::Remap(_, k) => k,
            Map::Span { .. } => &[],
        }
    }

    /// The first output time showing composition time `c`, including reversed or frozen remaps.
    pub fn output_of(&self, c: f64) -> Option<f64> {
        if !c.is_finite() || !(0.0..=self.project).contains(&c) {
            return None;
        }
        self.segments.iter().enumerate().find_map(|(i, seg)| {
            let u = match self.span(i) {
                Some((a, s)) => (c - a) / s,
                None => self.remap_time(i, c)?,
            };
            (u >= -1e-9 && u < seg.duration - 1e-9).then(|| seg.start + u.max(0.0))
        })
    }

    /// Find crossings in chronological order on a 1 ms grid, splitting at every key and
    /// sampling even short key intervals. Refine against the actual curve rather than a
    /// linear approximation, and reject sign changes caused by a jump over the target.
    fn remap_time(&self, i: usize, c: f64) -> Option<f64> {
        let duration = self.segments[i].duration;
        let value = |u| self.composition(i, u) - c;
        let mut left = 0.0;
        let mut a = value(left);
        if a.abs() < 1e-9 {
            return Some(left);
        }
        for end in self.keys(i).iter().copied().filter(|&t| t > 0.0 && t < duration).chain([duration]) {
            let start = left;
            let n = ((end - start) * 1000.0).ceil().max(64.0) as usize;
            for k in 1..=n {
                let right = start + (end - start) * k as f64 / n as f64;
                let b = value(right);
                if b == 0.0 || a.is_sign_positive() != b.is_sign_positive() {
                    let (mut lo, mut hi) = (left, right);
                    for _ in 0..50 {
                        let mid = (lo + hi) * 0.5;
                        let v = value(mid);
                        if v == 0.0 || v.is_sign_positive() != a.is_sign_positive() {
                            hi = mid;
                        } else {
                            lo = mid;
                        }
                    }
                    if hi < duration - 1e-9 && value(hi).abs() < 1e-9 {
                        return Some(hi);
                    }
                }
                left = right;
                a = b;
            }
        }
        None
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

    /// Whether any two segments are joined by a transition.
    pub fn has_transitions(&self) -> bool {
        !self.joins.is_empty()
    }

    /// Segment `i` at output time `t`, which may lie outside it (extended and clamped).
    fn side(&self, i: usize, t: f64) -> FrameTime {
        let local = t - self.segments[i].start;
        FrameTime { output: t, segment: i, local, composition: self.composition(i, local) }
    }

    /// Both sides of output time `t` when it is inside a join's window.
    pub fn join_at(&self, t: f64) -> Option<JoinFrame<'_>> {
        let join = self.joins.iter().find(|j| t >= j.window.0 - 1e-9 && t < j.window.1 - 1e-9)?;
        let (w0, w1) = join.window;
        let u = ((t - w0) / (w1 - w0)).clamp(0.0, 1.0);
        let (lo, hi) = ((u - 1e-3).max(0.0), (u + 1e-3).min(1.0));
        let velocity = (join.ease.apply(hi) - join.ease.apply(lo)) / (hi - lo) / (w1 - w0);
        Some(JoinFrame {
            join,
            from: self.side(join.from, t),
            to: self.side(join.from + 1, t),
            progress: join.ease.apply(u).clamp(0.0, 1.0),
            velocity,
        })
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
    fn markers_find_the_first_time_in_remapped_segments() {
        for (keys, composition, expected) in [
            (r#"<key time="0" value="0"/><key time="2" value="3"/>"#, 1.5, Some(1.0)),
            (r#"<key time="0" value="3"/><key time="2" value="0"/>"#, 1.5, Some(1.0)),
            (r#"<key time="0" value="0" interpolation="quad-in"/><key time="2" value="3"/>"#, 0.75, Some(1.0)),
            (r#"<key time="0" value="1"/><key time="2" value="1"/>"#, 1.0, Some(0.0)),
            (r#"<key time="0" value="0"/><key time="1" value="2"/><key time="2" value="0"/>"#, 1.0, Some(0.5)),
            (
                r#"<key time="0" value="0" interpolation="hold"/><key time="1" value="2"/><key time="2" value="2"/>"#,
                1.0,
                None,
            ),
            (
                r#"<key time="0" value="0" interpolation="hold"/><key time="1" value="2"/><key time="2" value="2"/>"#,
                2.0,
                Some(1.0),
            ),
            (r#"<key time="0" value="0"/><key time="2" value="3"/>"#, 3.0, None),
        ] {
            let (ev, o) = program(&format!(
                r#"<output path="a.mp4" codec="h264"><segment><timeRemap>{keys}</timeRemap></segment></output>"#
            ));
            let tm = TimeMap::of(ev.program(), &o).unwrap().unwrap();
            let got = tm.output_of(composition);
            match (got, expected) {
                (Some(a), Some(b)) => assert!((a - b).abs() < 1e-7, "{keys}: {a} != {b}"),
                (None, None) => {}
                _ => panic!("{keys}: {got:?} != {expected:?}"),
            }
        }
        let (ev, o) = program(
            r#"<output path="a.mp4" codec="h264"><segment from="2" to="3"/><segment><timeRemap><key time="0" value="0"/><key time="1" value="2"/></timeRemap></segment><segment from="0" to="2"/></output>"#,
        );
        let tm = TimeMap::of(ev.program(), &o).unwrap().unwrap();
        assert!((tm.output_of(1.0).unwrap() - 1.5).abs() < 1e-7);
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

    #[test]
    fn joins_take_their_window_from_the_alignment_and_the_shorter_side() {
        let (p, o) = program(
            r#"<output path="a.mp4" codec="h264">
                 <segment from="2" to="3"><transition type="crossfade" duration="0.5" curve="linear"/></segment>
                 <segment from="0" to="1"><transition type="wipe" duration="1" alignment="end"/></segment>
                 <segment from="1" to="1.2"><transition type="crossfade"/></segment>
               </output>"#,
        );
        let tm = TimeMap::of(p.program(), &o).unwrap().unwrap();
        // centred on the join at 1: 0.75‥1.25; the second is capped by the next segment's 0.2 s and ends
        // on its join at 2; the last segment's transition has nothing to join
        assert_eq!(tm.joins.len(), 2);
        assert_eq!(tm.joins[0].window, (0.75, 1.25));
        let w = tm.joins[1].window;
        assert!((w.0 - 1.8).abs() < 1e-12 && (w.1 - 2.0).abs() < 1e-12, "{w:?}");
        // before the join both sides are drawn: the incoming one extended before its start, clamped at 0
        let j = tm.join_at(0.8).unwrap();
        assert_eq!((j.from.segment, j.to.segment), (0, 1));
        assert!((j.from.composition - 2.8).abs() < 1e-12);
        assert_eq!(j.to.composition, 0.0);
        assert!((j.progress - 0.1).abs() < 1e-9 && (j.velocity - 2.0).abs() < 1e-6);
        // after it, the outgoing one runs on past its end
        assert!((tm.join_at(1.1).unwrap().from.composition - 3.0).abs() < 1e-12);
        assert!(tm.join_at(0.7).is_none() && tm.join_at(1.25).is_none());
    }
}
