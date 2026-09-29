//! The scene's audio: tracks, buses and master from `<audioMix>`, the
//! audio of video layers, automation sampled from the evaluator, and the
//! analysis table that `audioAmplitude()`, `beat()` and audio links read.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use rayon::prelude::*;
use sr_audio::effects::{Band, Effect, Kind};
use sr_audio::{
    Curve, Duck, Extent, FadeCurve, Layout, Master, Mix, Mixed, Node, NodeKind, Normalize, Placement, Source,
};
use sr_eval::{Analysis, Evaluator, Program, Value};
#[allow(unused_imports)]
use sr_model::element::Element;
use sr_model::model::{self as m, AssetsChild};

use crate::DeliverError;

/// A rendered scene mix with its delivery format.
pub struct SceneAudio {
    /// The mix description.
    pub mix: Mix,
    /// The rendered mix.
    pub mixed: Mixed,
    /// `audioMix/@bitDepth`.
    pub bits: u16,
    /// `master/@dither`.
    pub dither: bool,
    /// The analysis table built from it.
    pub analysis: Analysis,
}

const MASTER_KEY: &str = "audioMixType/master[0]";

fn source_path(p: &Program, asset: &AssetsChild, doc: usize, representation: Option<&str>) -> Option<PathBuf> {
    let none = Vec::new();
    let (src, reps) = match asset {
        AssetsChild::Audio(a) => (a.src.as_str(), &a.representations),
        AssetsChild::Video(v) => (v.src.as_str(), &v.representations),
        // generated media plays from its verified cache file
        AssetsChild::Generated(g) => (g.cache.as_str(), &none),
        _ => return None,
    };
    let src = representation.and_then(|r| reps.iter().find(|x| x.name == r)).map(|x| x.src.as_str()).unwrap_or(src);
    match sr_model::assets::resolve(src, p.base_dirs.get(doc)?) {
        sr_model::assets::Resolved::Local(path) => Some(path),
        sr_model::assets::Resolved::Remote(_) => None,
    }
}

fn find_asset<'p>(p: &'p Program, key: &str) -> Option<(&'p AssetsChild, usize)> {
    let Some((doc, id)) = p.assets.get(key) else {
        // assets used only by the audio mix live in the main document
        return p.scene.assets.as_ref()?.children.iter().find(|c| c.id() == Some(key)).map(|a| (a, 0));
    };
    let scene = if *doc == 0 { &p.scene } else { &p.includes.get(*doc as usize - 1)?.1 };
    let a = scene.assets.as_ref()?.children.iter().find(|c| c.id() == Some(id.as_str()))?;
    Some((a, *doc as usize))
}

fn effect_of(e: &m::AudioEffect) -> Option<Effect> {
    let kind = Kind::parse(e.r#type.as_str())?;
    let bands = e
        .children
        .iter()
        .filter_map(|c| match c {
            m::AudioEffectChild::Band(b) => Some(Band {
                shape: match b.kind.as_str() {
                    "low-shelf" => sr_audio::dsp::Shape::LowShelf,
                    "high-shelf" => sr_audio::dsp::Shape::HighShelf,
                    "highpass" => sr_audio::dsp::Shape::HighPass,
                    "lowpass" => sr_audio::dsp::Shape::LowPass,
                    "notch" => sr_audio::dsp::Shape::Notch,
                    _ => sr_audio::dsp::Shape::Peak,
                },
                frequency: b.frequency.get(),
                gain: b.gain.get(),
                q: b.q.get(),
            }),
            _ => None,
        })
        .collect();
    Some(Effect {
        kind,
        enabled: e.enabled,
        mix: e.mix.get(),
        frequency: e.frequency.map(|f| f.get()),
        gain: e.gain.get(),
        threshold: e.threshold.get(),
        ratio: e.ratio.get(),
        attack: e.attack.get(),
        release: e.release.get(),
        knee: e.knee.get(),
        time: e.time.map(|t| t.get()),
        feedback: e.feedback.get(),
        room_size: e.room_size.get(),
        width: e.width.get(),
        semitones: e.semitones,
        amount: e.amount.get(),
        bands,
        sidechain: e.sidechain.clone(),
    })
}

fn duck(under: &Option<Vec<String>>, amount: f64, threshold: f64, attack: f64, release: f64) -> Option<Duck> {
    under.as_ref().filter(|u| !u.is_empty()).map(|u| Duck { under: u.clone(), amount, threshold, attack, release })
}

/// Samples of animated properties per control frame.
struct Automation {
    props: HashMap<String, HashMap<&'static str, Vec<f64>>>,
    layer_times: HashMap<String, Vec<Option<f64>>>,
    layer_volume: HashMap<String, Vec<f64>>,
}

impl Automation {
    fn curve(&self, key: &str, prop: &'static str, fallback: f64) -> Curve {
        match self.props.get(key).and_then(|p| p.get(prop)) {
            Some(v) if v.iter().any(|x| (x - v[0]).abs() > 1e-12) => Curve::Frames(v.clone()),
            Some(v) if !v.is_empty() => Curve::Const(v[0]),
            _ => Curve::Const(fallback),
        }
    }
}

/// Builds and renders the scene mix, or `None` when the scene has no audio.
pub fn mix_scene(ev: &Evaluator, fps: f64, representation: Option<&str>) -> Result<Option<SceneAudio>, DeliverError> {
    let p = ev.program();
    let scene = &p.scene;
    let am = scene.audio_mix.as_ref();
    // layers whose video asset carries audio
    let audio_layers: Vec<(u32, &m::Layer, &AssetsChild, usize, u64)> = p
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(i, n)| {
            let l = match &*n.elem {
                m::Node::Layer(l) => l,
                _ => return None,
            };
            let key = n.asset.as_deref()?;
            let (a, doc) = find_asset(p, key)?;
            match a {
                AssetsChild::Video(v) if v.has_audio => Some((i as u32, l, a, doc, v.audio_stream)),
                _ => None,
            }
        })
        .collect();
    if am.is_none() && audio_layers.is_empty() {
        return Ok(None);
    }
    let rate = am.map(|a| a.sample_rate as u32).unwrap_or(48000);
    let layout = am.map(|a| Layout::parse(a.channel_layout.as_str(), a.channels as u32)).unwrap_or(Layout::Stereo);
    // automation and media clocks, one sample per frame
    let frames = (p.duration * fps).ceil() as usize + 1;
    let mut auto = Automation { props: HashMap::new(), layer_times: HashMap::new(), layer_volume: HashMap::new() };
    let layer_ids: HashMap<&str, ()> = audio_layers.iter().map(|(i, ..)| (&*p.nodes[*i as usize].id, ())).collect();
    let animated_audio = p.elements.iter().any(|e| {
        let k = &*e.key;
        k == MASTER_KEY
            || am.is_some_and(|a| {
                a.children.iter().any(|c| {
                    matches!(c, m::AudioMixChild::AudioTrack(t) if t.id == k)
                        || matches!(c, m::AudioMixChild::Bus(b) if b.id == k)
                })
            })
    });
    if animated_audio || !layer_ids.is_empty() {
        for k in 0..frames {
            let g = ev.evaluate(k as f64 / fps);
            for e in &g.elements {
                for name in ["volume", "gain", "pan"] {
                    if let Some(v) = e.props.get(name).and_then(Value::as_num) {
                        let slot = auto
                            .props
                            .entry(e.key.to_string())
                            .or_default()
                            .entry(name)
                            .or_insert_with(|| Vec::with_capacity(frames));
                        slot.resize(k, v);
                        slot.push(v);
                    }
                }
            }
            let mut seen: HashMap<&str, ()> = HashMap::new();
            for n in &g.nodes {
                if layer_ids.contains_key(&*n.id) {
                    seen.insert(&n.id, ());
                    let t = auto.layer_times.entry(n.id.to_string()).or_insert_with(|| vec![None; frames]);
                    t[k] = n.source_time;
                    let vol = n.props.get("volume").and_then(Value::as_num);
                    let v = auto.layer_volume.entry(n.id.to_string()).or_insert_with(|| vec![f64::NAN; frames]);
                    v[k] = vol.unwrap_or(f64::NAN);
                }
            }
        }
    }
    // decoded sources
    let mut sources: HashMap<(PathBuf, u64), Arc<Source>> = HashMap::new();
    let mut load = |path: PathBuf, stream: u64| -> Result<Arc<Source>, DeliverError> {
        if let Some(s) = sources.get(&(path.clone(), stream)) {
            return Ok(s.clone());
        }
        let a = sr_media::decode_audio(&path, stream as usize, rate)?;
        let s = Arc::new(Source::from_interleaved(
            &a.samples,
            a.channels as usize,
            Layout::for_channels(a.channels as u32, &a.layout),
        ));
        sources.insert((path, stream), s.clone());
        Ok(s)
    };
    let mut nodes = Vec::new();
    let mut master = Master::default();
    if let Some(am) = am {
        for c in &am.children {
            match c {
                m::AudioMixChild::AudioTrack(t) => {
                    let (asset, doc) = find_asset(p, &t.asset).ok_or_else(|| {
                        DeliverError::Invalid(format!("audio track {}: unknown asset {}", t.id, t.asset))
                    })?;
                    let path = source_path(p, asset, doc, representation).ok_or_else(|| {
                        DeliverError::Invalid(format!(
                            "audio track {}: asset {} is not a local audio or video file",
                            t.id, t.asset
                        ))
                    })?;
                    let stream = match asset {
                        AssetsChild::Video(v) => v.audio_stream,
                        _ => 0,
                    };
                    let bpm = match asset {
                        AssetsChild::Audio(a) => a.bpm.map(|b| b.get()),
                        _ => None,
                    };
                    let start = t.start.get()
                        + t.start_marker.as_deref().and_then(|mk| sr_eval::eval::marker(p, mk)).unwrap_or(0.0);
                    let fit = if t.fit_to_duration {
                        Some((
                            bpm.ok_or_else(|| {
                                DeliverError::Invalid(format!(
                                    "audio track {}: fitToDuration needs @bpm on asset {}",
                                    t.id, t.asset
                                ))
                            })?,
                            p.duration,
                        ))
                    } else {
                        None
                    };
                    nodes.push(Node {
                        id: t.id.clone(),
                        kind: NodeKind::Track {
                            source: load(path, stream)?,
                            placement: Placement::Timeline {
                                start,
                                clip_in: t.clip_in.get(),
                                clip_out: t.clip_out.map(|c| c.get()),
                                loops: t.r#loop as u32,
                                speed: t.speed,
                                reverse: t.reverse,
                                preserve_pitch: t.preserve_pitch,
                                fit,
                            },
                            fade_in: t.fade_in.get(),
                            fade_out: t.fade_out.get(),
                            fade_curve: FadeCurve::parse(t.fade_curve.as_str()),
                        },
                        volume: auto.curve(&t.id, "volume", t.volume.get()),
                        gain: auto.curve(&t.id, "gain", t.gain.get()),
                        pan: auto.curve(&t.id, "pan", t.pan),
                        mute: t.mute,
                        output: t.bus.clone(),
                        duck: duck(
                            &t.duck_under,
                            t.duck_amount.get(),
                            t.duck_threshold.get(),
                            t.duck_attack.get(),
                            t.duck_release.get(),
                        ),
                        effects: t
                            .children
                            .iter()
                            .filter_map(
                                |c| if let m::AudioTrackChild::AudioEffect(e) = c { effect_of(e) } else { None },
                            )
                            .collect(),
                    });
                }
                m::AudioMixChild::Bus(b) => nodes.push(Node {
                    id: b.id.clone(),
                    kind: NodeKind::Bus,
                    volume: auto.curve(&b.id, "volume", b.volume.get()),
                    gain: auto.curve(&b.id, "gain", b.gain.get()),
                    pan: auto.curve(&b.id, "pan", b.pan),
                    mute: b.mute,
                    output: b.output.clone(),
                    duck: duck(
                        &b.duck_under,
                        b.duck_amount.get(),
                        b.duck_threshold.get(),
                        b.duck_attack.get(),
                        b.duck_release.get(),
                    ),
                    effects: b
                        .children
                        .iter()
                        .filter_map(|c| if let m::BusChild::AudioEffect(e) = c { effect_of(e) } else { None })
                        .collect(),
                }),
                m::AudioMixChild::Master(ms) => {
                    master = Master {
                        volume: auto.curve(MASTER_KEY, "volume", ms.volume.get()),
                        normalize: match ms.normalize.as_str() {
                            "integrated" => Normalize::Integrated,
                            "dynamic" => Normalize::Dynamic,
                            _ => Normalize::None,
                        },
                        loudness: ms.loudness.get(),
                        true_peak: ms.true_peak.get(),
                        limiter: ms.limiter,
                        effects: ms
                            .children
                            .iter()
                            .filter_map(|c| if let m::MasterChild::AudioEffect(e) = c { effect_of(e) } else { None })
                            .collect(),
                    };
                }
            }
        }
    }
    for (i, l, asset, doc, stream) in &audio_layers {
        let id = p.nodes[*i as usize].id.to_string();
        let Some(path) = source_path(p, asset, *doc, representation) else { continue };
        let times = auto.layer_times.get(&id).cloned().unwrap_or_default();
        let vols = auto.layer_volume.get(&id).cloned().unwrap_or_default();
        let volume = if vols.iter().any(|v| v.is_finite() && (v - l.volume.get()).abs() > 1e-12) {
            Curve::Frames(vols.iter().map(|v| if v.is_finite() { *v } else { l.volume.get() }).collect())
        } else {
            Curve::Const(l.volume.get())
        };
        nodes.push(Node {
            id,
            kind: NodeKind::Track {
                source: load(path, *stream)?,
                placement: Placement::Mapped(times),
                fade_in: 0.0,
                fade_out: 0.0,
                fade_curve: FadeCurve::Linear,
            },
            volume,
            gain: Curve::Const(0.0),
            pan: Curve::Const(0.0),
            mute: l.mute,
            output: l.audio_bus.clone(),
            duck: None,
            effects: Vec::new(),
        });
    }
    let mix = Mix { rate, layout, duration: p.duration, control_fps: fps, nodes, master };
    let mixed = mix.render()?;
    // analysis: envelopes of every track and audio layer (independent, so in parallel, over
    // the extent the mix found for each), beats from a beat grid's source
    let frames = (p.duration * fps).ceil() as usize;
    let extent = |id: &str, out: &sr_audio::Planar| mixed.extents.get(id).copied().unwrap_or_else(|| Extent::of(out));
    let tracks: Vec<(String, [Vec<f32>; 4])> = mix
        .nodes
        .par_iter()
        .filter_map(|n| {
            let (NodeKind::Track { .. }, Some(out)) = (&n.kind, mixed.nodes.get(&n.id)) else { return None };
            let env = sr_audio::analysis::envelopes_in(out, extent(&n.id, out), rate as f64, fps, frames);
            Some((n.id.clone(), env))
        })
        .collect();
    let mut analysis = Analysis { fps, tracks: tracks.into_iter().collect(), beats: None };
    if let Some(grid) = scene.markers.as_ref().and_then(|mk| {
        mk.children.iter().find_map(|c| if let m::MarkersChild::BeatGrid(b) = c { Some(b) } else { None })
    }) {
        if let Some((id, src)) = grid.source.as_ref().and_then(|s| mixed.nodes.get_key_value(s)) {
            let ext = extent(id, src);
            let (_, beats) = sr_audio::analysis::beats_in(src, ext, rate as f64, Some(grid.bpm.get()), p.duration);
            analysis.beats = Some(beats);
        }
    }
    let bits = am.map(|a| a.bit_depth.as_str().parse().unwrap_or(24)).unwrap_or(24);
    let dither = am
        .and_then(|a| {
            a.children.iter().find_map(|c| if let m::AudioMixChild::Master(ms) = c { Some(ms.dither) } else { None })
        })
        .unwrap_or(true);
    Ok(Some(SceneAudio { mix, mixed, bits, dither, analysis }))
}

/// The master between `start` and `end` seconds.
pub fn slice(buf: &sr_audio::Planar, rate: u32, start: f64, end: f64) -> sr_audio::Planar {
    let a = (start * rate as f64).round().max(0.0) as usize;
    let b = (end * rate as f64).round().max(0.0) as usize;
    buf.iter().map(|c| (a..b).map(|i| c.get(i).copied().unwrap_or(0.0)).collect()).collect()
}
