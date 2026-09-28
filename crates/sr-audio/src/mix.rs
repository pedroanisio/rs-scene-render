//! The mixing graph: tracks and video-layer audio feed buses, buses feed
//! buses or the master; fades, automation, ducking, effects, loudness
//! normalisation and the true-peak limiter.

use std::collections::HashMap;
use std::sync::Arc;

use crate::dsp::{coeff, db_to_lin, hermite, lin_to_db};
use crate::effects::{self, Effect};
use crate::layout::{route, Layout};
use crate::loudness::{self, Planar};

/// Decoded source audio at the mix rate.
#[derive(Debug, Clone)]
pub struct Source {
    /// Channel layout of the source.
    pub layout: Layout,
    /// One buffer per source channel.
    pub planar: Planar,
}

impl Source {
    /// From interleaved samples.
    pub fn from_interleaved(samples: &[f32], channels: usize, layout: Layout) -> Source {
        let channels = channels.max(1);
        let frames = samples.len() / channels;
        let planar = (0..channels).map(|c| (0..frames).map(|i| samples[i * channels + c]).collect()).collect();
        Source { layout, planar }
    }

    /// Length in samples.
    pub fn len(&self) -> usize {
        self.planar.first().map(Vec::len).unwrap_or(0)
    }

    /// No samples.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A value over time: constant, or one value per control frame (linear in between).
#[derive(Debug, Clone, PartialEq)]
pub enum Curve {
    /// Constant.
    Const(f64),
    /// Values at `k / control_fps`.
    Frames(Vec<f64>),
}

impl Curve {
    fn at(&self, t: f64, fps: f64) -> f64 {
        match self {
            Curve::Const(v) => *v,
            Curve::Frames(v) if v.is_empty() => 0.0,
            Curve::Frames(v) => {
                let x = (t * fps).max(0.0);
                let i = (x.floor() as usize).min(v.len() - 1);
                let j = (i + 1).min(v.len() - 1);
                v[i] + (v[j] - v[i]) * (x - i as f64).min(1.0)
            }
        }
    }
}

/// Fade shapes of `fadeCurve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeCurve {
    /// g = x.
    Linear,
    /// g = sin(x·π/2), constant power for crossfades.
    EqualPower,
    /// g = log10(1 + 9x): fast rise, slow finish.
    Logarithmic,
    /// g = (10^x − 1)/9: slow rise, fast finish.
    Exponential,
    /// g = 3x² − 2x³.
    SCurve,
}

impl FadeCurve {
    /// Parses the schema name.
    pub fn parse(s: &str) -> FadeCurve {
        match s {
            "equal-power" => FadeCurve::EqualPower,
            "logarithmic" => FadeCurve::Logarithmic,
            "exponential" => FadeCurve::Exponential,
            "s-curve" => FadeCurve::SCurve,
            _ => FadeCurve::Linear,
        }
    }

    /// Gain at fade position `x` in [0, 1] (0 silent, 1 full).
    pub fn gain(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        match self {
            FadeCurve::Linear => x,
            FadeCurve::EqualPower => (x * std::f64::consts::FRAC_PI_2).sin(),
            FadeCurve::Logarithmic => (1.0 + 9.0 * x).log10(),
            FadeCurve::Exponential => (10f64.powf(x) - 1.0) / 9.0,
            FadeCurve::SCurve => x * x * (3.0 - 2.0 * x),
        }
    }
}

/// Where a track's source plays.
#[derive(Debug, Clone, PartialEq)]
pub enum Placement {
    /// An audio track on the timeline.
    Timeline {
        /// Start on the timeline, seconds.
        start: f64,
        /// Source in point.
        clip_in: f64,
        /// Source out point (end of source when `None`).
        clip_out: Option<f64>,
        /// Additional repetitions.
        loops: u32,
        /// Playback speed.
        speed: f64,
        /// Play backwards.
        reverse: bool,
        /// Keep pitch when `speed` ≠ 1 (time-stretch instead of resample).
        preserve_pitch: bool,
        /// Loop or trim at bar boundaries (source bpm) to end at this time.
        fit: Option<(f64, f64)>,
    },
    /// Source time per control frame, from a video layer's media clock
    /// (`None` while the layer is inactive).
    Mapped(Vec<Option<f64>>),
}

/// Automatic ducking.
#[derive(Debug, Clone, PartialEq)]
pub struct Duck {
    /// Nodes whose level triggers ducking.
    pub under: Vec<String>,
    /// Gain while triggered, dB.
    pub amount: f64,
    /// Trigger level, dBFS.
    pub threshold: f64,
    /// Attack, seconds.
    pub attack: f64,
    /// Release, seconds.
    pub release: f64,
}

/// What a node is.
#[derive(Debug, Clone)]
pub enum NodeKind {
    /// Plays a source.
    Track {
        /// Source audio.
        source: Arc<Source>,
        /// Placement.
        placement: Placement,
        /// Fade-in length, seconds.
        fade_in: f64,
        /// Fade-out length, seconds.
        fade_out: f64,
        /// Fade shape.
        fade_curve: FadeCurve,
    },
    /// Sums the nodes routed to it.
    Bus,
}

/// A track or bus.
#[derive(Debug, Clone)]
pub struct Node {
    /// Id (track, layer or bus).
    pub id: String,
    /// Track or bus.
    pub kind: NodeKind,
    /// `volume` automation (0–1).
    pub volume: Curve,
    /// `gain` automation, dB.
    pub gain: Curve,
    /// `pan` automation, −1 to 1.
    pub pan: Curve,
    /// Silenced.
    pub mute: bool,
    /// Destination bus (master when `None`).
    pub output: Option<String>,
    /// Ducking.
    pub duck: Option<Duck>,
    /// Effects in order.
    pub effects: Vec<Effect>,
}

/// Loudness normalisation of the master.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Normalize {
    /// None.
    None,
    /// Static gain to the integrated target.
    Integrated,
    /// Slow gain riding on 3-second short-term loudness.
    Dynamic,
}

/// The master bus.
#[derive(Debug, Clone)]
pub struct Master {
    /// `volume` automation.
    pub volume: Curve,
    /// Normalisation mode.
    pub normalize: Normalize,
    /// Target loudness, LUFS.
    pub loudness: f64,
    /// True-peak ceiling, dBTP.
    pub true_peak: f64,
    /// Run the true-peak limiter even without normalisation.
    pub limiter: bool,
    /// Effects before the fader.
    pub effects: Vec<Effect>,
}

impl Default for Master {
    fn default() -> Master {
        Master {
            volume: Curve::Const(1.0),
            normalize: Normalize::None,
            loudness: -14.0,
            true_peak: -1.0,
            limiter: false,
            effects: Vec::new(),
        }
    }
}

/// A complete mix description.
#[derive(Debug, Clone)]
pub struct Mix {
    /// Sample rate.
    pub rate: u32,
    /// Output layout.
    pub layout: Layout,
    /// Length, seconds.
    pub duration: f64,
    /// Rate of automation curves and mapped placements.
    pub control_fps: f64,
    /// Tracks and buses.
    pub nodes: Vec<Node>,
    /// Master bus.
    pub master: Master,
}

/// Mixing errors.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MixError {
    /// A bus, ducking source or sidechain names an unknown node.
    #[error("{0} refers to unknown audio node {1:?}")]
    Unknown(String, String),
    /// Routing, ducking and sidechains form a cycle.
    #[error("audio routing cycle: {0}")]
    Cycle(String),
}

/// The rendered mix.
#[derive(Debug, Clone)]
pub struct Mixed {
    /// Master output in the mix layout.
    pub master: Planar,
    /// Post-fader output of every node.
    pub nodes: HashMap<String, Planar>,
    /// Integrated loudness of the master before normalisation, LUFS.
    pub loudness_before: f64,
    /// Integrated loudness of the output, LUFS.
    pub loudness: f64,
    /// True peak of the output, dBTP.
    pub true_peak: f64,
}

/// WSOLA time-stretch: output length = input length × `factor`, pitch kept.
pub fn time_stretch(src: &Planar, factor: f64) -> Planar {
    const N: usize = 1024;
    const HS: usize = N / 2;
    const TOL: i64 = 256;
    let n = src.first().map(Vec::len).unwrap_or(0);
    if n < N || (factor - 1.0).abs() < 1e-9 {
        return src.clone();
    }
    let out_len = (n as f64 * factor) as usize;
    let win = crate::dsp::hann(N);
    let mono: Vec<f32> = (0..n).map(|i| src.iter().map(|c| c[i]).sum::<f32>()).collect();
    let mut out = vec![vec![0f64; out_len + N]; src.len()];
    let mut norm = vec![0f64; out_len + N];
    let ha = HS as f64 / factor;
    let mut prev: i64 = 0;
    let mut k = 0usize;
    while k * HS < out_len {
        let nominal = (k as f64 * ha) as i64;
        let pos = if k == 0 {
            0
        } else {
            // best match to the natural continuation of the previous segment
            let target = prev + HS as i64;
            let mut best = (f64::MIN, nominal);
            let mut d = -TOL;
            while d <= TOL {
                let c = nominal + d;
                if c >= 0 && c as usize + N <= n && target >= 0 && target as usize + N <= n {
                    let mut s = 0.0;
                    for j in (0..N).step_by(4) {
                        s += mono[c as usize + j] as f64 * mono[target as usize + j] as f64;
                    }
                    if s > best.0 {
                        best = (s, c);
                    }
                }
                d += 4;
            }
            best.1
        };
        let pos = pos.clamp(0, (n - N) as i64);
        for (ch, o) in src.iter().zip(out.iter_mut()) {
            for j in 0..N {
                o[k * HS + j] += ch[pos as usize + j] as f64 * win[j];
            }
        }
        for j in 0..N {
            norm[k * HS + j] += win[j];
        }
        prev = pos;
        k += 1;
    }
    out.into_iter()
        .map(|o| (0..out_len).map(|i| if norm[i] > 1e-6 { (o[i] / norm[i]) as f32 } else { 0.0 }).collect())
        .collect()
}

impl Mix {
    fn frames(&self) -> usize {
        (self.duration * self.rate as f64).round() as usize
    }

    /// Renders a track's source onto the timeline (source channels, before routing).
    fn place(&self, source: &Source, placement: &Placement, fade_in: f64, fade_out: f64, curve: FadeCurve) -> Planar {
        let rate = self.rate as f64;
        let total = self.frames();
        let chans = source.planar.len().max(1);
        let mut out = vec![vec![0f32; total]; chans];
        match placement {
            Placement::Timeline { start, clip_in, clip_out, loops, speed, reverse, preserve_pitch, fit } => {
                let a = ((clip_in * rate).round().max(0.0) as usize).min(source.len());
                let b =
                    clip_out.map(|c| ((c * rate).round() as usize).min(source.len())).unwrap_or(source.len()).max(a);
                let mut seg: Planar = source.planar.iter().map(|c| c[a..b].to_vec()).collect();
                if *reverse {
                    for c in seg.iter_mut() {
                        c.reverse();
                    }
                }
                let speed = speed.abs().max(1e-3);
                if (speed - 1.0).abs() > 1e-9 {
                    seg = if *preserve_pitch {
                        time_stretch(&seg, 1.0 / speed)
                    } else {
                        let len = (seg[0].len() as f64 / speed) as usize;
                        seg.iter().map(|c| (0..len).map(|i| hermite(c, i as f64 * speed)).collect()).collect()
                    };
                }
                let seg_len = seg[0].len();
                let start_s = (start * rate).round() as i64;
                let mut body: Planar = seg
                    .iter()
                    .map(|c| {
                        let mut v = Vec::with_capacity(seg_len * (*loops as usize + 1));
                        for _ in 0..=*loops {
                            v.extend_from_slice(c);
                        }
                        v
                    })
                    .collect();
                if let Some((bpm, end)) = fit {
                    // whole bars of the segment, repeated to reach the end, cut at the end
                    let bar = (4.0 * 60.0 / bpm * rate).round() as usize;
                    let bars = (seg_len / bar.max(1)).max(1);
                    let unit = (bars * bar).min(seg_len);
                    let want = ((end - start) * rate).round().max(0.0) as usize;
                    body = seg.iter().map(|c| (0..want).map(|i| c[i % unit]).collect()).collect();
                    let ramp = ((0.01 * rate) as usize).min(want);
                    for c in body.iter_mut() {
                        for k in 0..ramp {
                            c[want - 1 - k] *= k as f32 / ramp as f32;
                        }
                    }
                }
                let len = body[0].len();
                let (fi, fo) = ((fade_in * rate) as usize, (fade_out * rate) as usize);
                for (c, ch) in body.iter().enumerate() {
                    for (i, &v) in ch.iter().enumerate() {
                        let t = start_s + i as i64;
                        if t < 0 || t as usize >= total {
                            continue;
                        }
                        let mut g = 1.0;
                        if i < fi {
                            g *= curve.gain(i as f64 / fi as f64);
                        }
                        if len - i <= fo {
                            g *= curve.gain((len - i) as f64 / fo as f64);
                        }
                        out[c][t as usize] = (v as f64 * g) as f32;
                    }
                }
            }
            Placement::Mapped(times) => {
                let fps = self.control_fps;
                for i in 0..total {
                    let x = i as f64 / rate * fps;
                    let k = x.floor() as usize;
                    let (Some(Some(s0)), s1) = (times.get(k), times.get(k + 1)) else { continue };
                    let st = match s1 {
                        Some(Some(s1)) if (s1 - s0).abs() < 1.0 => s0 + (s1 - s0) * (x - k as f64),
                        _ => *s0 + (x - k as f64) / fps,
                    };
                    let pos = st * rate;
                    if pos < 0.0 || pos >= source.len() as f64 {
                        continue;
                    }
                    for c in 0..chans {
                        out[c][i] = hermite(&source.planar[c], pos);
                    }
                }
            }
        }
        out
    }

    fn order(&self) -> Result<Vec<usize>, MixError> {
        let index: HashMap<&str, usize> = self.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); self.nodes.len()];
        for (i, n) in self.nodes.iter().enumerate() {
            if let Some(o) = &n.output {
                let &b = index.get(o.as_str()).ok_or_else(|| MixError::Unknown(n.id.clone(), o.clone()))?;
                deps[b].push(i);
            }
            let keys =
                n.duck.iter().flat_map(|d| d.under.iter()).chain(n.effects.iter().filter_map(|e| e.sidechain.as_ref()));
            for k in keys {
                let &j = index.get(k.as_str()).ok_or_else(|| MixError::Unknown(n.id.clone(), k.clone()))?;
                deps[i].push(j);
            }
        }
        let mut state = vec![0u8; self.nodes.len()];
        let mut order = Vec::new();
        fn visit(
            i: usize,
            deps: &[Vec<usize>],
            state: &mut [u8],
            order: &mut Vec<usize>,
            nodes: &[Node],
        ) -> Result<(), MixError> {
            match state[i] {
                2 => return Ok(()),
                1 => return Err(MixError::Cycle(nodes[i].id.clone())),
                _ => {}
            }
            state[i] = 1;
            for &d in &deps[i] {
                visit(d, deps, state, order, nodes)?;
            }
            state[i] = 2;
            order.push(i);
            Ok(())
        }
        for i in 0..self.nodes.len() {
            visit(i, &deps, &mut state, &mut order, &self.nodes)?;
        }
        Ok(order)
    }

    /// Renders the mix.
    pub fn render(&self) -> Result<Mixed, MixError> {
        let order = self.order()?;
        let rate = self.rate as f64;
        let total = self.frames();
        let chans = self.layout.channels();
        let silent = || vec![vec![0f32; total]; chans];
        let mut outputs: HashMap<String, Planar> = HashMap::new();
        for &i in &order {
            let n = &self.nodes[i];
            // input in the mix layout
            let mut buf = match &n.kind {
                NodeKind::Track { source, placement, fade_in, fade_out, fade_curve } => {
                    let placed = self.place(source, placement, *fade_in, *fade_out, *fade_curve);
                    self.route_block(&placed, source.layout, &n.pan)
                }
                NodeKind::Bus => {
                    let mut sum = silent();
                    for m in self.nodes.iter().filter(|m| m.output.as_deref() == Some(n.id.as_str())) {
                        if let Some(o) = outputs.get(&m.id) {
                            for (s, c) in sum.iter_mut().zip(o) {
                                for (a, b) in s.iter_mut().zip(c) {
                                    *a += b;
                                }
                            }
                        }
                    }
                    self.route_block(&sum, self.layout, &n.pan)
                }
            };
            for e in &n.effects {
                let key = e.sidechain.as_ref().and_then(|k| outputs.get(k));
                effects::process(e, &mut buf, rate, key);
            }
            // fader: volume × gain automation
            for s in 0..total {
                let t = s as f64 / rate;
                let g = n.volume.at(t, self.control_fps) * db_to_lin(n.gain.at(t, self.control_fps));
                for c in buf.iter_mut() {
                    c[s] = (c[s] as f64 * g) as f32;
                }
            }
            if let Some(d) = &n.duck {
                let key: Vec<f64> = (0..total)
                    .map(|s| {
                        d.under
                            .iter()
                            .filter_map(|u| outputs.get(u))
                            .flat_map(|o| o.iter().map(move |c| (c[s] as f64).abs()))
                            .fold(0.0, f64::max)
                    })
                    .collect();
                let env_c = coeff(0.01, rate);
                let (ca, cr) = (coeff(d.attack, rate), coeff(d.release, rate));
                let (mut env, mut g) = (0.0, 0.0);
                for s in 0..total {
                    env = key[s].max(env * env_c);
                    let target = if lin_to_db(env) > d.threshold { d.amount.min(0.0) } else { 0.0 };
                    g = if target < g { target + (g - target) * ca } else { target + (g - target) * cr };
                    let lin = db_to_lin(g);
                    for c in buf.iter_mut() {
                        c[s] = (c[s] as f64 * lin) as f32;
                    }
                }
            }
            if n.mute {
                buf = silent();
            }
            outputs.insert(n.id.clone(), buf);
        }
        let mut master = silent();
        for n in self.nodes.iter().filter(|n| n.output.is_none()) {
            if let Some(o) = outputs.get(&n.id) {
                for (s, c) in master.iter_mut().zip(o) {
                    for (a, b) in s.iter_mut().zip(c) {
                        *a += b;
                    }
                }
            }
        }
        for e in &self.master.effects {
            let key = e.sidechain.as_ref().and_then(|k| outputs.get(k));
            effects::process(e, &mut master, rate, key);
        }
        for s in 0..total {
            let g = self.master.volume.at(s as f64 / rate, self.control_fps);
            for c in master.iter_mut() {
                c[s] = (c[s] as f64 * g) as f32;
            }
        }
        let weights = self.layout.loudness_weights();
        let before = loudness::integrated(&master, rate, &weights);
        match self.master.normalize {
            Normalize::None => {}
            Normalize::Integrated => {
                if before > -70.0 {
                    let g = db_to_lin(self.master.loudness - before);
                    for c in master.iter_mut() {
                        for v in c.iter_mut() {
                            *v = (*v as f64 * g) as f32;
                        }
                    }
                }
            }
            Normalize::Dynamic => {
                let hop = 0.1;
                let st = loudness::short_term(&master, rate, &weights, hop);
                let gains: Vec<f64> = st
                    .iter()
                    .map(|l| if *l > -50.0 { (self.master.loudness - l).clamp(-20.0, 20.0) } else { 0.0 })
                    .collect();
                // smooth with a 3 s time constant in both directions
                let k = (-hop / 3.0f64).exp();
                let mut sm = Vec::with_capacity(gains.len());
                let mut g = gains.first().copied().unwrap_or(0.0);
                for v in &gains {
                    g = v + (g - v) * k;
                    sm.push(g);
                }
                // short-term block k covers [k·hop, k·hop + 3 s]: apply at its centre
                for s in 0..total {
                    let x = ((s as f64 / rate - 1.5) / hop).max(0.0);
                    let i = (x as usize).min(sm.len().saturating_sub(1));
                    let gd = sm.get(i).copied().unwrap_or(0.0);
                    let lin = db_to_lin(gd);
                    for c in master.iter_mut() {
                        c[s] = (c[s] as f64 * lin) as f32;
                    }
                }
            }
        }
        if self.master.limiter || self.master.normalize != Normalize::None {
            loudness::limit(&mut master, rate, self.master.true_peak, 0.05);
        }
        let loudness = loudness::integrated(&master, rate, &weights);
        let true_peak = loudness::true_peak(&master);
        Ok(Mixed { master, nodes: outputs, loudness_before: before, loudness, true_peak })
    }

    /// Routes `src` (in `layout`) into the mix layout with pan automation, per 256-sample block.
    fn route_block(&self, src: &Planar, layout: Layout, pan: &Curve) -> Planar {
        let total = src.first().map(Vec::len).unwrap_or(0);
        let chans = self.layout.channels();
        let mut out = vec![vec![0f32; total]; chans];
        let rate = self.rate as f64;
        let mut s = 0;
        while s < total {
            let e = (s + 256).min(total);
            let m = route(layout, self.layout, pan.at((s + e) as f64 * 0.5 / rate, self.control_fps));
            for (o, row) in m.iter().enumerate() {
                for (i, &g) in row.iter().enumerate() {
                    if g == 0.0 || i >= src.len() {
                        continue;
                    }
                    let g = g as f32;
                    for k in s..e {
                        out[o][k] += src[i][k] * g;
                    }
                }
            }
            s = e;
        }
        out
    }
}
