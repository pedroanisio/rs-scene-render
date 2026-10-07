//! The audio of an output with segments (SREP 13).
//!
//! The scene mix renders every source in composition time with its own gain, fades, effects and
//! ducking: its post-fader output is the source's stem. Each selected stem is mapped through the
//! segments into output time: `stretch` keeps pitch (WSOLA), `resample` plays faster or slower at a
//! pitch scaled by the rate, `mute` is silence; a freeze is silence and a reversal plays backwards.
//! Segment i occupies samples round(Tᵢ·r) to round(Tᵢ₊₁·r), and consecutive segments meet in an
//! equal-power crossfade over `joinFade` seconds centred on the join, or over the transition's
//! window. The output's own audio tracks join the mapped stems in output time, and the composition's
//! buses and master (effects, normalisation measured on this programme, limiter) run once on the
//! result in output time.

use std::collections::HashSet;
use std::sync::Arc;

use rayon::prelude::*;
use sr_audio::{Curve, Mix, Node, NodeKind, Placement, Planar, Source};
use sr_eval::Program;
use sr_model::model as m;

use crate::audio::{self, SceneAudio, Sources};
use crate::segments::TimeMap;
use crate::DeliverError;

/// A bus for stems that other nodes read (ducking, sidechains) but the output does not play: it is
/// muted, and no document id can name it (ids are NCNames, without colons).
const UNPLAYED: &str = "sr:unplayed";

/// The output's programme.
pub struct OutputAudio {
    /// Master in the mix layout, the output's duration long.
    pub master: Planar,
    /// Post-fader tracks and buses in output time, for audio-reactive output shaders.
    pub nodes: std::collections::HashMap<String, Planar>,
    /// Integrated loudness, LUFS.
    pub loudness: f64,
    /// True peak, dBTP.
    pub true_peak: f64,
}

/// Whether a source (its id, role and bus) is selected by `o`: it matches any of `audioTracks`,
/// `audioRoles` and `audioBuses`, or the output gives none of them.
fn selected(o: &m::Output, id: &str, role: &str, bus: Option<&str>) -> bool {
    let (t, r, b) = (&o.audio_tracks, &o.audio_roles, &o.audio_buses);
    if t.is_none() && r.is_none() && b.is_none() {
        return true;
    }
    t.as_ref().is_some_and(|v| v.iter().any(|x| x == id))
        || r.as_ref().is_some_and(|v| v.iter().any(|x| x == role))
        || b.as_ref().zip(bus).is_some_and(|(v, bus)| v.iter().any(|x| x == bus))
}

/// Whether the output selects the composition's audio track `id`.
pub(crate) fn track_selected(p: &Program, o: &m::Output, id: &str) -> bool {
    p.scene.audio_mix.iter().flat_map(|am| &am.children).any(|c| match c {
        m::AudioMixChild::AudioTrack(t) if t.id == id => selected(o, id, t.role.as_str(), t.bus.as_deref()),
        _ => false,
    })
}

/// The nodes that `n` reads besides its inputs: ducking keys and effect sidechains.
fn reads(n: &Node) -> impl Iterator<Item = &String> {
    n.duck.iter().flat_map(|d| d.under.iter()).chain(n.effects.iter().filter_map(|e| e.sidechain.as_ref()))
}

fn key_bus(id: &str) -> String {
    format!("sr:key:{id}")
}

/// Bus keys read the complete signal, including sources excluded from audible playback.
fn redirect_keys(n: &mut Node, buses: &HashSet<&str>) {
    for id in
        n.duck.iter_mut().flat_map(|d| &mut d.under).chain(n.effects.iter_mut().filter_map(|e| e.sidechain.as_mut()))
    {
        if buses.contains(id.as_str()) {
            *id = key_bus(id);
        }
    }
}

/// Automation keyed in composition time, sampled at the output's control frames through the map.
fn map_curve(c: &Curve, tm: &TimeMap, fps: f64) -> Curve {
    match c {
        Curve::Const(_) => c.clone(),
        Curve::Frames(_) => {
            let n = (tm.duration * fps).ceil() as usize + 1;
            Curve::Frames((0..n).map(|k| c.at(tm.at(k as f64 / fps).composition, fps)).collect())
        }
    }
}

/// Renders the output's audio. `select` applies the output's `audioTracks`, `audioRoles` and
/// `audioBuses`; without segments (an output whose own tracks join the composition's audio) every
/// source plays, since those attributes act only on outputs with segments.
pub fn render(
    p: &Program,
    sa: &SceneAudio,
    output: &m::Output,
    tm: &TimeMap,
    fps: f64,
    representation: Option<&str>,
    select: bool,
) -> Result<OutputAudio, DeliverError> {
    let rate = sa.mix.rate;
    let roles: std::collections::HashMap<&str, &str> = p
        .scene
        .audio_mix
        .iter()
        .flat_map(|am| &am.children)
        .filter_map(|c| match c {
            m::AudioMixChild::AudioTrack(t) => Some((t.id.as_str(), t.role.as_str())),
            _ => None,
        })
        .collect();
    // the output's own tracks, in output time
    let mut own = own_nodes(p, output, tm, rate, fps, representation)?;
    let tracks: Vec<&Node> = sa.mix.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Track { .. })).collect();
    let buses: Vec<&Node> = sa.mix.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Bus)).collect();
    let play: HashSet<&str> = tracks
        .iter()
        .filter(|n| {
            !select
                || selected(output, &n.id, roles.get(n.id.as_str()).copied().unwrap_or("other"), n.output.as_deref())
        })
        .map(|n| n.id.as_str())
        .collect();
    let mut needed: HashSet<&str> = play.clone();
    for n in buses.iter().copied().chain(own.iter()) {
        needed.extend(reads(n).map(String::as_str));
    }
    needed.extend(sa.mix.master.effects.iter().filter_map(|e| e.sidechain.as_deref()));
    // When selection removes sources, a bus used as a key needs a separate, inaudible signal.
    // Include its entire input tree. With every source playing, the existing buses suffice.
    let mut key_buses: HashSet<&str> = buses
        .iter()
        .filter(|b| play.len() < tracks.len() && needed.contains(b.id.as_str()))
        .map(|b| b.id.as_str())
        .collect();
    loop {
        let before = key_buses.len();
        for node in &sa.mix.nodes {
            if node.output.as_deref().is_some_and(|id| key_buses.contains(id)) {
                needed.insert(node.id.as_str());
                if matches!(node.kind, NodeKind::Bus) {
                    key_buses.insert(node.id.as_str());
                }
            }
        }
        if key_buses.len() == before {
            break;
        }
    }
    let windows = windows(tm, output.join_fade.get());
    let n = (tm.duration * rate as f64).round() as usize;
    let stems: Vec<Node> = tracks
        .par_iter()
        .filter(|t| needed.contains(t.id.as_str()))
        .map(|t| {
            let stem = sa.mixed.nodes.get(&t.id).cloned().unwrap_or_default();
            let mapped = map_stem(&stem, rate as f64, tm, &windows, n);
            Node {
                id: t.id.clone(),
                kind: NodeKind::Track {
                    source: Arc::new(Source { layout: sa.mix.layout, planar: mapped }),
                    placement: Placement::Timeline {
                        start: 0.0,
                        clip_in: 0.0,
                        clip_out: None,
                        loops: 0,
                        speed: 1.0,
                        reverse: false,
                        preserve_pitch: false,
                        fit: None,
                    },
                    fade_in: 0.0,
                    fade_out: 0.0,
                    fade_curve: sr_audio::FadeCurve::Linear,
                },
                volume: Curve::Const(1.0),
                gain: Curve::Const(0.0),
                pan: Curve::Const(0.0),
                mute: false,
                output: if play.contains(t.id.as_str()) { t.output.clone() } else { Some(UNPLAYED.into()) },
                duck: None,
                effects: Vec::new(),
            }
        })
        .collect();
    let mut nodes = stems;
    // Taps share post-fader audio with the audible graph, so selected and output-owned tracks
    // feed keys without having their effects or ducking applied a second time.
    for source in tracks.iter().copied().chain(own.iter()) {
        if let Some(bus) = source.output.as_deref().filter(|id| key_buses.contains(id)) {
            nodes.push(Node {
                id: format!("sr:tap:{}", source.id),
                kind: NodeKind::Tap { source: source.id.clone() },
                volume: Curve::Const(1.0),
                gain: Curve::Const(0.0),
                pan: Curve::Const(0.0),
                mute: false,
                output: Some(key_bus(bus)),
                duck: None,
                effects: Vec::new(),
            });
        }
    }
    if !key_buses.is_empty() || nodes.iter().any(|s| s.output.as_deref() == Some(UNPLAYED)) {
        nodes.push(Node {
            id: UNPLAYED.into(),
            kind: NodeKind::Bus,
            volume: Curve::Const(1.0),
            gain: Curve::Const(0.0),
            pan: Curve::Const(0.0),
            mute: true,
            output: None,
            duck: None,
            effects: Vec::new(),
        });
    }
    for b in buses {
        let mut b = b.clone();
        b.volume = map_curve(&b.volume, tm, fps);
        b.gain = map_curve(&b.gain, tm, fps);
        b.pan = map_curve(&b.pan, tm, fps);
        redirect_keys(&mut b, &key_buses);
        if key_buses.contains(b.id.as_str()) {
            let mut key = b.clone();
            key.id = key_bus(&b.id);
            key.output = Some(
                b.output.as_deref().filter(|id| key_buses.contains(id)).map(key_bus).unwrap_or_else(|| UNPLAYED.into()),
            );
            nodes.push(key);
        }
        nodes.push(b);
    }
    for node in &mut own {
        redirect_keys(node, &key_buses);
    }
    nodes.extend(own);
    let mut master = sa.mix.master.clone();
    for key in master.effects.iter_mut().filter_map(|e| e.sidechain.as_mut()) {
        if key_buses.contains(key.as_str()) {
            *key = key_bus(key);
        }
    }
    master.volume = map_curve(&master.volume, tm, fps);
    let mix = Mix { rate, layout: sa.mix.layout, duration: tm.duration, control_fps: fps, nodes, master };
    let mixed = mix.render()?;
    Ok(OutputAudio { master: mixed.master, nodes: mixed.nodes, loudness: mixed.loudness, true_peak: mixed.true_peak })
}

/// The mix nodes of the output's own audio tracks, in output time.
fn own_nodes(
    p: &Program,
    output: &m::Output,
    tm: &TimeMap,
    rate: u32,
    fps: f64,
    representation: Option<&str>,
) -> Result<Vec<Node>, DeliverError> {
    let mut sources = Sources::new(rate);
    let tracks: Vec<_> = output
        .children
        .iter()
        .filter_map(|c| match c {
            m::OutputChild::AudioTrack(t) => Some(t),
            _ => None,
        })
        .collect();
    let auto = audio::Automation::tracks(p, &tracks, tm.duration, fps);
    let curve = |key: &str, prop: &'static str, v: f64| auto.curve(key, prop, v);
    let mut own = Vec::new();
    for t in tracks {
        let at = match &t.start_marker {
            // a marker is composition time: the output time that first shows it
            Some(mk) => {
                let c = sr_eval::eval::marker(p, mk)
                    .ok_or_else(|| DeliverError::Invalid(format!("audio track {}: no marker {mk:?}", t.id)))?;
                tm.output_of(c).ok_or_else(|| {
                    DeliverError::Invalid(format!("audio track {}: marker {mk:?} is in no segment", t.id))
                })?
            }
            None => 0.0,
        };
        let start = t.start.get() + at;
        own.push(audio::track_node(p, t, start, tm.duration, representation, &curve, &mut sources)?);
    }
    Ok(own)
}

/// The output's time map: its segments, or without them the composition from `output/@start` to its
/// end played as it is.
pub fn output_map(p: &Program, output: &m::Output) -> Result<TimeMap, DeliverError> {
    match TimeMap::of(p, output).map_err(DeliverError::Invalid)? {
        Some(tm) => Ok(tm),
        None => {
            let end = output.end.unwrap_or(p.duration).min(p.duration);
            TimeMap::span_of(p, output.start, end).map_err(DeliverError::Invalid)
        }
    }
}

/// Each of the output's own audio tracks alone, post-fader, in output time (what a transcriber hears),
/// with the sample rate: for the resolve step.
pub fn own_tracks(
    doc: &sr_model::Document,
    output: &m::Output,
) -> Result<(u32, std::collections::HashMap<String, Planar>), DeliverError> {
    let ev = sr_eval::Evaluator::new(
        doc,
        &sr_eval::EvalOptions { variant: output.variant.clone(), layout: output.layout.clone(), ..Default::default() },
    )
    .map_err(DeliverError::Document)?;
    let p = ev.program();
    let tm = output_map(p, output)?;
    let am = p.scene.audio_mix.as_ref();
    let rate = am.map(|a| a.sample_rate as u32).unwrap_or(48000);
    let layout = am
        .map(|a| sr_audio::Layout::parse(a.channel_layout.as_str(), a.channels as u32))
        .unwrap_or(sr_audio::Layout::Stereo);
    let fps = output.fps.map(|f| f.as_f64()).unwrap_or(p.fps.as_f64());
    let mut nodes = own_nodes(p, output, &tm, rate, fps, None)?;
    // alone: no ducking under, or keying from, anything else
    for n in &mut nodes {
        n.duck = None;
        n.output = None;
        n.effects.retain(|e| e.sidechain.is_none());
    }
    let mix = Mix { rate, layout, duration: tm.duration, control_fps: fps, nodes, master: Default::default() };
    Ok((rate, mix.render()?.nodes))
}

/// The crossfade window of each join, in output seconds: the transition's, or `join_fade` centred
/// on the join, never longer than either segment (an empty window is a cut).
fn windows(tm: &TimeMap, join_fade: f64) -> Vec<(f64, f64)> {
    (0..tm.segments.len().saturating_sub(1))
        .map(|i| match tm.joins.iter().find(|j| j.from == i) {
            Some(j) => j.window,
            None => {
                let cut = tm.segments[i + 1].start;
                let d = join_fade.min(tm.segments[i].duration).min(tm.segments[i + 1].duration);
                (cut - d / 2.0, cut + d / 2.0)
            }
        })
        .collect()
}

/// A stem in composition time mapped into an output of `n` samples.
fn map_stem(stem: &Planar, rate: f64, tm: &TimeMap, windows: &[(f64, f64)], n: usize) -> Planar {
    let chans = stem.len().max(1);
    let mut out = vec![vec![0f32; n]; chans];
    if stem.is_empty() {
        return out;
    }
    let at = |t: f64| ((t * rate).round().max(0.0) as usize).min(n);
    let last = tm.segments.len() - 1;
    for i in 0..=last {
        // the samples segment i sounds in: from the start of the window it fades in over to the end of
        // the one it fades out over
        let (w_in, w_out) = (i.checked_sub(1).map(|j| windows[j]), (i < last).then(|| windows[i]));
        let a = w_in.map_or(0, |w| at(w.0));
        let b = w_out.map_or(n, |w| at(w.1));
        if b <= a {
            continue;
        }
        let body = segment(stem, rate, tm, i, a, b);
        for (o, c) in out.iter_mut().zip(&body) {
            for (k, v) in c.iter().enumerate() {
                let t = (a + k) as f64 / rate;
                let mut g = 1.0;
                if let Some((w0, w1)) = w_in.filter(|w| w.1 > w.0) {
                    if t < w1 {
                        g *= ((t - w0) / (w1 - w0)).clamp(0.0, 1.0) * std::f64::consts::FRAC_PI_2;
                        g = g.sin();
                    }
                }
                if let Some((w0, w1)) = w_out.filter(|w| w.1 > w.0) {
                    if t >= w0 {
                        g *= (((t - w0) / (w1 - w0)).clamp(0.0, 1.0) * std::f64::consts::FRAC_PI_2).cos();
                    }
                }
                o[a + k] += (*v as f64 * g) as f32;
            }
        }
    }
    out
}

/// The stem's samples at `at` (fractional sample index), zero outside it.
fn read(c: &[f32], at: f64) -> f32 {
    sr_audio::dsp::hermite(c, at)
}

/// A slice of `len` samples of every channel from sample `from` (zero outside the stem), reversed
/// when `backwards`.
fn slice(stem: &Planar, from: i64, len: usize, backwards: bool) -> Planar {
    stem.iter()
        .map(|c| {
            let mut v: Vec<f32> = (0..len as i64)
                .map(|k| from + k)
                .map(|j| if j < 0 { 0.0 } else { *c.get(j as usize).unwrap_or(&0.0) })
                .collect();
            if backwards {
                v.reverse();
            }
            v
        })
        .collect()
}

/// `src` time-stretched to exactly `len` samples, keeping pitch.
fn stretch(src: Planar, len: usize) -> Planar {
    let have = src.first().map(Vec::len).unwrap_or(0);
    let mut out =
        if have == len || have == 0 { src } else { sr_audio::mix::time_stretch(&src, len as f64 / have as f64) };
    for c in out.iter_mut() {
        c.resize(len, 0.0);
    }
    out
}

/// Output samples `a..b` of segment `i` (its map extended past its ends where a crossfade needs it).
fn segment(stem: &Planar, rate: f64, tm: &TimeMap, i: usize, a: usize, b: usize) -> Planar {
    let len = b - a;
    let chans = stem.len();
    let seg = &tm.segments[i];
    // local time counts from the segment's first sample, round(Tᵢ·r), so that it starts on its first
    // composition sample whatever the rounding
    let first = (seg.start * rate).round();
    let local = |s: usize| (s as f64 - first) / rate;
    match seg.elem.audio {
        m::SegmentAudio::Mute => vec![vec![0.0; len]; chans],
        m::SegmentAudio::Resample => {
            // read the stem at the mapped time of every output sample: pitch follows the rate; where the
            // map stands still (a freeze) there is no sound
            let h = 0.5 / rate;
            (0..chans)
                .map(|ch| {
                    (a..b)
                        .map(|s| {
                            let u = local(s);
                            let slope = (tm.unclamped(i, u + h) - tm.unclamped(i, u - h)) / (2.0 * h);
                            if slope.abs() < 1e-6 {
                                0.0
                            } else {
                                read(&stem[ch], tm.unclamped(i, u) * rate)
                            }
                        })
                        .collect()
                })
                .collect()
        }
        m::SegmentAudio::Stretch => match tm.span(i) {
            Some((from, 1.0)) => {
                // at speed 1 the samples are copied: output sample s is stem sample s + offset
                let offset = (from * rate).round() as i64 - first as i64;
                slice(stem, a as i64 + offset, len, false)
            }
            Some((from, speed)) => {
                let src = from + speed * local(a);
                let have = ((len as f64) * speed).round() as usize;
                stretch(slice(stem, (src * rate).round() as i64, have, false), len)
            }
            None => {
                // a remap: between consecutive keys the audio runs at that interval's mean rate, stretched to
                // keep pitch, backwards where the map runs backwards, silent where it stands still
                let (ua, ub) = (local(a), local(b));
                let mut cuts = vec![ua];
                cuts.extend(tm.keys(i).iter().copied().filter(|k| *k > ua && *k < ub));
                cuts.push(ub);
                let mut out = vec![Vec::with_capacity(len); chans];
                for w in cuts.windows(2) {
                    let s0 = (first + w[0] * rate).round().max(a as f64) as usize;
                    let s1 = ((first + w[1] * rate).round().max(0.0) as usize).clamp(s0, b);
                    let piece_len = s1 - s0;
                    let (c0, c1) = (tm.unclamped(i, w[0]), tm.unclamped(i, w[1]));
                    let have = ((c1 - c0).abs() * rate).round() as usize;
                    let piece = if have == 0 {
                        vec![vec![0.0; piece_len]; chans]
                    } else {
                        let backwards = c1 < c0;
                        let first = (c0.min(c1) * rate).round() as i64;
                        stretch(slice(stem, first, have, backwards), piece_len)
                    };
                    for (o, p) in out.iter_mut().zip(piece) {
                        o.extend(p);
                    }
                }
                for c in out.iter_mut() {
                    c.resize(len, 0.0);
                }
                out
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(outputs: &str) -> (TimeMap, m::Output) {
        let xml = format!(
            r##"<scene version="1.2"><project width="64" height="36" fps="10" duration="4" background="#000000"/>
              {outputs}<composition/></scene>"##
        );
        let doc = sr_model::load_str(&xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e}"));
        let ev = sr_eval::Evaluator::new(&doc, &Default::default()).unwrap();
        let out = ev.program().scene.outputs[0].clone();
        (TimeMap::of(ev.program(), &out).unwrap().unwrap(), out)
    }

    /// A stem whose sample k holds k + 1, so a mapped sample says where it came from.
    fn ramp(n: usize) -> Planar {
        vec![(0..n).map(|k| (k + 1) as f32).collect()]
    }

    #[test]
    fn segments_take_their_samples_on_cumulative_rounding() {
        // at 1000 Hz: 0.3333 s then 0.5 s; the first ends at round(333.33) = 333
        let (tm, o) = map(
            r#"<output path="a.mp4" codec="h264" joinFade="0"><segment from="1" to="2" speed="3"/><segment from="2" to="2.5" audio="resample"/></output>"#,
        );
        let n = (tm.duration * 1000.0).round() as usize;
        let out = map_stem(&ramp(4000), 1000.0, &tm, &windows(&tm, o.join_fade.get()), n);
        assert_eq!(n, 833);
        // the second segment starts at sample 333 with composition 2 s = stem sample 2000
        assert_eq!(out[0][333], 2001.0);
        assert_eq!(out[0][832], 2500.0);
    }

    #[test]
    fn speed_one_copies_and_mute_and_freeze_are_silent() {
        let (tm, o) = map(r#"<output path="a.mp4" codec="h264" joinFade="0">
                 <segment from="2" to="2.1"/>
                 <segment from="0" to="0.1" audio="mute"/>
                 <segment audio="resample"><timeRemap><key time="0" value="1"/><key time="0.1" value="1"/></timeRemap></segment>
               </output>"#);
        let n = (tm.duration * 1000.0).round() as usize;
        let out = map_stem(&ramp(4000), 1000.0, &tm, &windows(&tm, o.join_fade.get()), n);
        assert_eq!(&out[0][..3], &[2001.0, 2002.0, 2003.0]);
        assert!(out[0][100..300].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn a_backwards_remap_plays_the_samples_reversed() {
        let (tm, o) = map(
            r#"<output path="a.mp4" codec="h264"><segment audio="resample"><timeRemap><key time="0" value="2" interpolation="linear"/><key time="1" value="1"/></timeRemap></segment></output>"#,
        );
        let out = map_stem(&ramp(4000), 1000.0, &tm, &windows(&tm, o.join_fade.get()), 1000);
        assert!((out[0][0] - 2001.0).abs() < 1e-3 && (out[0][500] - 1501.0).abs() < 1e-3);
    }

    #[test]
    fn joins_crossfade_at_equal_power() {
        // a constant stem: through the 0.1 s crossfade the gains are cos and sin of the same angle
        let (tm, o) = map(
            r#"<output path="a.mp4" codec="h264" joinFade="0.1"><segment from="0" to="1"/><segment from="2" to="3"/></output>"#,
        );
        let flat = vec![vec![1.0f32; 4000]];
        let out = map_stem(&flat, 1000.0, &tm, &windows(&tm, o.join_fade.get()), 2000);
        // the power sums to 1 across the join; the midpoint is at −3 dB from each side
        let mid = out[0][1000] as f64;
        assert!((mid - 2f64.sqrt()).abs() < 1e-3, "{mid}");
        assert!((out[0][900] - 1.0).abs() < 1e-3 && (out[0][1100] - 1.0).abs() < 1e-3);
    }
}
