//! One `<output>` from document to delivered files.
//!
//! Stages overlap: while the encoder thread writes frame N, the GPU renders
//! and converts frame N+1 and the readback of frame N maps; a helper thread
//! evaluates frame N+2 when the document has no simulation; decoders run in
//! their own FFmpeg processes ahead of the render position. Audio mixes
//! first, because loudness normalisation needs the whole programme and the
//! evaluator reads the analysis table built from it.

use std::path::{Path, PathBuf};
use std::sync::mpsc::sync_channel;
use std::time::Instant;

use sr_eval::{EvalOptions, Evaluator};
use sr_gpu::output::{OutputColor, OutputStage};
use sr_gpu::{Gpu, Renderer};
use sr_media::encode::{Codec, ColorTags, Container, EncodeSpec, Encoder, Hardware, Hdr, InputFormat, StillFormat};
use sr_model::model as m;

use crate::audio::{self, SceneAudio};
use crate::DeliverError;

/// Delivery options that are not part of the document.
#[derive(Debug, Clone)]
pub struct Options {
    /// Directory relative output paths resolve against (default: the document's).
    pub out_dir: Option<PathBuf>,
    /// Preferred asset representation, over the output's own.
    pub representation: Option<String>,
    /// Hardware encoder preference.
    pub hardware: Hardware,
    /// Override the output's start time.
    pub start: Option<f64>,
    /// Override the output's end time.
    pub end: Option<f64>,
    /// Upload to destinations after rendering.
    pub upload: bool,
    /// Parameters, as on the command line.
    pub params: Vec<(String, String)>,
    /// Data row.
    pub row: Option<(Option<String>, usize)>,
    /// Segments rendered at once for a single-pass video output.
    pub parallel: Parallel,
}

/// How many time segments of a video output render and encode at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Parallel {
    /// 3 with a hardware encoder, otherwise half the cores (at most 4); fewer for short ranges.
    #[default]
    Auto,
    /// Exactly this many (1 renders serially).
    Count(u32),
}

impl Parallel {
    /// `auto` or a positive count.
    pub fn parse(s: &str) -> Option<Parallel> {
        match s {
            "auto" => Some(Parallel::Auto),
            n => n.parse().ok().filter(|&n| n > 0).map(Parallel::Count),
        }
    }
}

impl Default for Options {
    fn default() -> Options {
        Options {
            out_dir: None,
            representation: None,
            hardware: Hardware::Auto,
            start: None,
            end: None,
            upload: true,
            params: Vec::new(),
            row: None,
            parallel: Parallel::default(),
        }
    }
}

/// What a delivery produced and how long each stage took.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Report {
    /// Main output path (pattern for sequences).
    pub path: PathBuf,
    /// Every file written.
    pub files: Vec<PathBuf>,
    /// Video frames encoded.
    pub frames: u64,
    /// Output size.
    pub size: [u32; 2],
    /// Output frame rate.
    pub fps: f64,
    /// Encoder in use.
    pub encoder: String,
    /// Wall-clock seconds for the whole output.
    pub seconds: f64,
    /// Frames per second achieved for the video pass.
    pub render_fps: f64,
    /// Seconds spent evaluating (waiting for the next graph, when a helper thread evaluates
    /// ahead), rendering (CPU planning and submission), waiting for converted frames, and
    /// blocked on the encoder.
    pub stage_seconds: [f64; 4],
    /// Seconds of the render stage spent waiting for video decoders.
    pub decode_wait_seconds: f64,
    /// Seconds of the render stage spent building and tile-encoding vector geometry.
    pub vector_seconds: f64,
    /// Seconds spent mixing audio.
    pub audio_seconds: f64,
    /// Integrated loudness of the delivered mix, LUFS.
    pub loudness: Option<f64>,
    /// True peak of the delivered mix, dBTP.
    pub true_peak: Option<f64>,
    /// Content not rendered by this batch, per node.
    pub unsupported: Vec<String>,
    /// Accessibility findings (flash analysis, text contrast, required captions).
    pub accessibility: Vec<String>,
    /// The finding that fails delivery (a check set to `error`).
    #[serde(skip)]
    pub accessibility_error: Option<String>,
    /// Remote locations the files were delivered to.
    pub uploads: Vec<String>,
    /// Encoded passes (2 for two-pass, more when fitting a file size).
    pub passes: u32,
    /// Time segments rendered and encoded at once (1 for a serial render).
    pub segments: u32,
}

/// An `<output>` element built from command-line settings.
pub fn adhoc_output(path: &str, codec: &str) -> Result<m::Output, DeliverError> {
    let xml = format!(r#"<output path="{}" codec="{}"/>"#, xml_escape(path), xml_escape(codec));
    let doc = format!(
        r##"<scene version="1.1"><project width="16" height="16" fps="1" duration="1"/>{xml}<composition/></scene>"##
    );
    let d = sr_model::load_str(&doc, &sr_model::LoadOptions::without_assets()).map_err(|e| match e {
        sr_model::LoadError::Invalid(r) => DeliverError::Document(r),
        e => DeliverError::Invalid(e.to_string()),
    })?;
    Ok(d.scene.outputs[0].clone())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

fn resolve(dir: &Path, p: &str) -> PathBuf {
    let p = p.strip_prefix("file://").unwrap_or(p);
    let pb = PathBuf::from(p);
    if pb.is_absolute() {
        pb
    } else {
        dir.join(pb)
    }
}

/// The raw frame format the GPU writes for an output.
fn input_format(o: &m::Output, codec: Codec) -> InputFormat {
    let deep = o.pixel_format.contains("10") || o.pixel_format.contains("12") || o.pixel_format.contains("16");
    match codec {
        Codec::ExrSequence => InputFormat::Gbrapf32,
        Codec::TiffSequence | Codec::Prores | Codec::Dnxhr => InputFormat::Rgba16,
        Codec::PngSequence | Codec::Apng => {
            if deep {
                InputFormat::Rgba16
            } else {
                InputFormat::Rgba8
            }
        }
        Codec::Gif | Codec::Webp | Codec::JpegSequence => InputFormat::Rgba8,
        _ if o.alpha => InputFormat::Rgba16,
        _ => match o.pixel_format.as_str() {
            "yuv420p" | "nv12" => InputFormat::Nv12,
            "yuv420p10le" | "p010le" | "p010" => InputFormat::P010,
            _ => InputFormat::Rgba16,
        },
    }
}

fn metadata(scene: &m::Scene) -> Vec<(String, String)> {
    let mut v = vec![("encoder".to_string(), format!("scene-render {}", env!("CARGO_PKG_VERSION")))];
    if let Some(md) = &scene.metadata {
        for (k, val) in [
            ("title", &md.title),
            ("artist", &md.author),
            ("comment", &md.description),
            ("copyright", &md.copyright),
            ("keywords", &md.keywords),
        ] {
            if let Some(val) = val {
                v.push((k.to_string(), val.clone()));
            }
        }
    }
    v
}

struct Video<'a> {
    ev: &'a Evaluator,
    renderer: Renderer,
    stage: OutputStage,
    color: OutputColor,
    format: InputFormat,
    size: [u32; 2],
    keep_alpha: bool,
    /// Output frame rate.
    fps: f64,
    /// How frames are placed in the output (a layout's reframing).
    reframe: Option<sr_eval::program::Reframe>,
    /// The output's segments, and the frame time of each entry of `times`.
    segments: Option<(&'a crate::segments::TimeMap, Vec<crate::segments::FrameTime>)>,
    /// The symbol drawn over every frame in output time.
    overlay: Option<crate::overlay::Overlay>,
    /// Output-sized working textures: the two placed sides of a join, and their combination.
    join_tex: Option<[std::sync::Arc<sr_gpu::resources::Tex>; 3]>,
}

impl Video<'_> {
    /// The placement in the output of a frame of segment time `f` (the output's own frame without segments).
    fn placement(&self, f: Option<&crate::segments::FrameTime>) -> sr_gpu::output::Placement {
        use sr_gpu::output::Placement;
        let Some(r) = self.reframe else { return Placement::default() };
        let focus = match (&self.segments, f) {
            (Some((tm, _)), Some(f)) => tm.focus(f, r.focus),
            _ => r.focus,
        };
        match r.mode {
            m::Reframe::Crop => Placement::Crop { focus },
            m::Reframe::FitBlur => Placement::FitBlur { focus },
            // the focus only places cropped or blurred-fit frames; a plain fit is centred
            _ => Placement::Fit { focus: [0.5, 0.5] },
        }
    }
}

impl Video<'_> {
    /// Renders the graph `g` of composition time `t`, the frame of segment time `f` when the output
    /// has segments. A sub-frame sample dt seconds from the frame (motion blur, video lookups) is
    /// taken dt output seconds away and mapped through the frame's segment, so blur covers the
    /// composition time the shutter spans at that speed.
    fn render_side(
        &mut self,
        g: &sr_eval::FrameGraph,
        t: f64,
        f: Option<&crate::segments::FrameTime>,
    ) -> Result<sr_gpu::Frame, DeliverError> {
        let (ev, p, fps) = (self.ev, self.ev.program(), self.fps);
        let seg = self.segments.as_ref().zip(f).map(|((tm, _), f)| (*tm, *f));
        let mut sub = |st: f64| match seg {
            Some((tm, f)) if (st - t).abs() < 1.0 / fps => ev.evaluate(tm.sample(&f, st - t)),
            _ => ev.evaluate(st),
        };
        let frame = self.renderer.render_with(g, p, Some(&mut sub));
        if let Some(e) = frame.stats.errors.first() {
            return Err(DeliverError::Render { time: t, message: e.clone() });
        }
        Ok(frame)
    }
}

impl Video<'_> {
    /// Renders `times` and hands each converted frame to `sink`; returns stage timings.
    fn run(
        &mut self,
        times: &[f64],
        report: &mut Report,
        mut sink: impl FnMut(Vec<u8>) -> Result<f64, DeliverError>,
    ) -> Result<(), DeliverError> {
        let p = self.ev.program();
        let working = self.renderer.working();
        let mut pending: Option<(f64, sr_gpu::output::Pending)> = None;
        let mut unsupported = std::collections::BTreeSet::new();
        let acc = p.scene.metadata.as_ref().and_then(|m| {
            m.children.iter().find_map(|c| match c {
                m::MetadataChild::Accessibility(a) => Some(a.clone()),
                _ => None,
            })
        });
        // Without an <accessibility> element nothing is checked: the XSD defaults (flashCheck
        // "warn", contrastCheck "off") apply to the element's attributes, and the checks
        // are skipped when it is absent. Declare <accessibility/> to opt in.
        let flash_mode = acc.as_ref().map(|a| a.flash_check.to_string()).unwrap_or_else(|| "off".into());
        let contrast_mode = acc.as_ref().map(|a| a.contrast_check.to_string()).unwrap_or_else(|| "off".into());
        let min_contrast = acc.as_ref().map(|a| a.min_contrast.get()).unwrap_or(4.5);
        let mut flash = (flash_mode != "off").then(crate::access::FlashDetector::default);
        self.renderer.contrast_probe = contrast_mode != "off";
        // Each text is judged at its most visible: the lowest ratio among the frames where its accumulated
        // opacity is at its maximum over the encode. A label fading in passes through every ratio down to
        // 1:1 on its way to rest, which is not what a reader faces; text held dim is judged dim.
        let mut lowest: std::collections::BTreeMap<String, (f64, f64, f64)> = Default::default(); // (opacity, ratio, t)
        // text inside isolated groups: (frame index, opacity) where each appears (measured after the pass)
        let mut unprobed: std::collections::BTreeMap<String, Vec<(usize, f64)>> = Default::default();
        let opacity_of = |g: &sr_eval::FrameGraph, id: &str| g.nodes.iter().find(|n| &*n.id == id).map(|n| n.world_opacity).unwrap_or(1.0);
        let keep = |lowest: &mut std::collections::BTreeMap<String, (f64, f64, f64)>, id: &str, op: f64, ratio: f64, t: f64| {
            let e = lowest.entry(id.to_string()).or_insert((f64::MIN, f64::MAX, t));
            if op > e.0 + 1e-3 {
                *e = (op, ratio, t);
            } else if (op - e.0).abs() <= 1e-3 && ratio < e.1 {
                *e = (e.0, ratio, t);
            }
        };
        let ev = self.ev;
        // Without a simulation, evaluation is a pure function of time, so a helper thread
        // evaluates the next frame while this one renders: the channel holds one graph and
        // the helper works on the one after it, two frames ahead at most. Simulated state
        // is stepped in time order under a lock, so those documents evaluate here, serially.
        let ahead = !ev.has_simulation();
        std::thread::scope(|s| -> Result<(), DeliverError> {
            let graphs = if ahead {
                let (tx, rx) = sync_channel::<sr_eval::FrameGraph>(1);
                std::thread::Builder::new().name("sr-eval".into()).spawn_scoped(s, move || {
                    for &t in times {
                        // the render loop stopped early: nobody wants the rest
                        if tx.send(ev.evaluate(t)).is_err() {
                            break;
                        }
                    }
                })?;
                Some(rx)
            } else {
                None
            };
            for (k, &t) in times.iter().enumerate() {
                let t0 = Instant::now();
                let g = match &graphs {
                    Some(rx) => rx.recv().map_err(|_| DeliverError::Invalid("evaluator thread stopped".into()))?,
                    None => ev.evaluate(t),
                };
                let t1 = Instant::now();
                let ft = self.segments.as_ref().map(|(_, frames)| frames[k]);
                let tm = self.segments.as_ref().map(|(tm, _)| *tm);
                let join = tm.zip(ft).and_then(|(tm, f)| tm.join_at(f.output));
                let frame = self.render_side(&g, t, ft.as_ref())?;
                unsupported.extend(frame.stats.unsupported.iter().cloned());
                if let Some(det) = flash.as_mut() {
                    let cells = self.renderer.flash_grid(&frame.texture);
                    det.push(t, &cells);
                }
                for id in &frame.stats.contrast_unprobed {
                    unprobed.entry(id.clone()).or_default().push((k, opacity_of(&g, id)));
                }
                for (id, ratio) in &frame.stats.contrast {
                    keep(&mut lowest, id, opacity_of(&g, id), *ratio, t);
                }
                report.decode_wait_seconds += frame.stats.decode_wait;
                report.vector_seconds += frame.stats.vector_seconds;
                let mut picture = frame.texture.clone();
                let mut placement = self.placement(ft.as_ref());
                if let Some(j) = join {
                    // both sides of a join, each through its own map and placed with its own focus,
                    // combined in the output's frame
                    let tex =
                        self.join_tex.get_or_insert_with(|| [(); 3].map(|_| self.renderer.texture(self.size))).clone();
                    let own = if ft.is_some_and(|f| f.segment == j.from.segment) { 0 } else { 1 };
                    let sides = [j.from, j.to];
                    self.stage.place(&frame.texture, &working, &tex[own], self.placement(Some(&sides[own])));
                    let other = sides[1 - own];
                    let go = ev.evaluate(other.composition);
                    let f2 = self.render_side(&go, other.composition, Some(&other))?;
                    unsupported.extend(f2.stats.unsupported.iter().cloned());
                    self.stage.place(&f2.texture, &working, &tex[1 - own], self.placement(Some(&other)));
                    let problems = self.renderer.join(
                        p,
                        &j.join.elem,
                        &tex[0],
                        &tex[1],
                        j.progress,
                        j.velocity,
                        j.from.output,
                        &tex[2],
                    );
                    unsupported.extend(problems);
                    picture = tex[2].clone();
                    placement = sr_gpu::output::Placement::default();
                }
                let out_t = ft.map_or(t, |f| f.output);
                let over = match self.overlay.as_mut() {
                    Some(o) => {
                        let (tex, problems) = o.draw(&mut self.stage, out_t)?;
                        unsupported.extend(problems);
                        Some(tex)
                    }
                    None => None,
                };
                let next = self.stage.submit_placed(
                    &picture,
                    &working,
                    &self.color,
                    self.format,
                    self.size,
                    self.keep_alpha,
                    placement,
                    over,
                );
                let t2 = Instant::now();
                report.stage_seconds[0] += (t1 - t0).as_secs_f64();
                report.stage_seconds[1] += (t2 - t1).as_secs_f64();
                if let Some((_, prev)) = pending.replace((t, next)) {
                    let w = Instant::now();
                    let bytes = self.stage.wait(prev);
                    report.stage_seconds[2] += w.elapsed().as_secs_f64();
                    report.stage_seconds[3] += sink(bytes)?;
                }
                if k + 1 == times.len() {
                    if let Some((_, last)) = pending.take() {
                        let w = Instant::now();
                        let bytes = self.stage.wait(last);
                        report.stage_seconds[2] += w.elapsed().as_secs_f64();
                        report.stage_seconds[3] += sink(bytes)?;
                    }
                }
            }
            Ok(())
        })?;
        report.unsupported = unsupported.into_iter().collect();
        // text the inline probe could not reach: measure once, at the middle of the longest run of
        // frames in which it is drawn, by rendering that frame with and without it
        for (id, seen) in &unprobed {
            // among the frames where the text is at its most visible, the middle of the longest run
            let top = seen.iter().map(|s| s.1).fold(f64::MIN, f64::max);
            let frames: Vec<usize> = seen.iter().filter(|s| s.1 >= top - 1e-3).map(|s| s.0).collect();
            let Some(t) = middle_of_longest_run(&frames).map(|k| times[k]) else { continue };
            let g = self.ev.evaluate(t);
            let ev = &self.ev;
            let mut sub = |st: f64| ev.evaluate(st);
            if let Some(ratio) = self.renderer.contrast_with_without(&g, p, id, &mut sub) {
                keep(&mut lowest, id, top, ratio, t);
            }
        }
        // accessibility verdicts
        report.accessibility.clear();
        report.accessibility_error = None;
        let fail = |mode: &str, msg: String, report: &mut Report| {
            if mode == "error" && report.accessibility_error.is_none() {
                report.accessibility_error = Some(msg.clone());
            }
            report.accessibility.push(msg);
        };
        if let Some(msg) = flash.as_ref().and_then(|d| d.verdict()) {
            fail(&flash_mode, msg, report);
        }
        for (id, (_, ratio, at)) in &lowest {
            if *ratio < min_contrast {
                fail(&contrast_mode, format!("contrastCheck: {id} reaches only {ratio:.2}:1 against its background at {at:.3} s (minimum {min_contrast}:1)"), report);
            }
        }
        if acc.as_ref().is_some_and(|a| a.require_captions)
            && p.scene.captions.as_ref().map(|c| c.caption_tracks.is_empty()).unwrap_or(true)
        {
            fail("error", "requireCaptions: the document has no caption track".into(), report);
        }
        Ok(())
    }
}

/// Writes frames through an encoder on its own thread; returns blocked seconds per frame.
struct Feeder {
    tx: Option<std::sync::mpsc::SyncSender<Vec<u8>>>,
    handle: Option<std::thread::JoinHandle<Result<String, sr_media::MediaError>>>,
}

impl Feeder {
    fn start(spec: &EncodeSpec) -> Result<Feeder, DeliverError> {
        let mut enc = Encoder::start(spec)?;
        let (tx, rx) = sync_channel::<Vec<u8>>(3);
        let handle = std::thread::Builder::new().name("sr-encode".into()).spawn(move || {
            for f in rx {
                enc.write(&f)?;
            }
            let name = enc.encoder.clone();
            enc.finish()?;
            Ok(name)
        })?;
        Ok(Feeder { tx: Some(tx), handle: Some(handle) })
    }

    fn send(&mut self, frame: Vec<u8>) -> Result<f64, DeliverError> {
        let t = Instant::now();
        if self.tx.as_ref().expect("open").send(frame).is_err() {
            // the encoder thread stopped: surface its error
            return Err(self.finish().err().unwrap_or_else(|| DeliverError::Invalid("encoder stopped".into())));
        }
        Ok(t.elapsed().as_secs_f64())
    }

    fn finish(&mut self) -> Result<String, DeliverError> {
        drop(self.tx.take());
        match self.handle.take() {
            Some(h) => Ok(h.join().map_err(|_| DeliverError::Invalid("encoder thread panicked".into()))??),
            None => Ok(String::new()),
        }
    }
}

/// Renders one output of `doc` and delivers it.
pub fn deliver(
    doc: &sr_model::Document,
    output: &m::Output,
    gpu: Option<&Gpu>,
    opts: &Options,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Report, DeliverError> {
    let started = Instant::now();
    let codec = Codec::parse(output.codec.as_str())
        .ok_or_else(|| DeliverError::Invalid(format!("unknown codec {}", output.codec.as_str())))?;
    let base =
        opts.out_dir.clone().or_else(|| Some(doc.base_dir().to_path_buf())).unwrap_or_else(|| PathBuf::from("."));
    let representation = opts.representation.clone().or_else(|| output.representation.clone());
    let mut eo = EvalOptions {
        variant: output.variant.clone(),
        layout: output.layout.clone(),
        params: opts.params.clone(),
        row: opts.row.clone(),
        ..Default::default()
    };
    let ev0 = Evaluator::new(doc, &eo).map_err(DeliverError::Document)?;
    let p0 = ev0.program();
    let fps = output.fps.map(|f| f.as_f64()).unwrap_or(p0.fps.as_f64());
    // with segments, the output has its own timeline: start and end (from the command line) are output times
    let segments = crate::segments::TimeMap::of(p0, output).map_err(DeliverError::Invalid)?;
    let timeline = segments.as_ref().map(|tm| tm.duration).unwrap_or(p0.duration);
    let start = opts.start.unwrap_or(output.start).max(0.0);
    let end = opts.end.or(output.end).unwrap_or(timeline).min(timeline);
    if end <= start {
        return Err(DeliverError::Invalid(format!("empty range {start}..{end}")));
    }
    let mut report = Report { path: resolve(&base, &output.path), fps, ..Default::default() };
    // audio first: the mix feeds the analysis table
    let t_audio = Instant::now();
    let mut scene_audio: Option<SceneAudio> = audio::mix_scene(&ev0, fps, representation.as_deref())?;
    report.audio_seconds = t_audio.elapsed().as_secs_f64();
    let ev = match &scene_audio {
        Some(sa) => {
            eo.analysis = sa.analysis.clone();
            Evaluator::new(doc, &eo).map_err(DeliverError::Document)?
        }
        None => ev0,
    };
    let p = ev.program();
    if let Some(parent) = report.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp =
        std::env::temp_dir().join(format!("scene-render-{}-{}", std::process::id(), started.elapsed().as_nanos()));
    std::fs::create_dir_all(&tmp)?;
    let wants_audio =
        output.audio && scene_audio.is_some() && (codec.takes_audio() || codec.is_audio_only()) && segments.is_none();
    if segments.is_some() && output.audio && scene_audio.is_some() {
        report.unsupported.push("audio of an output with segments is not rendered yet".into());
    }
    let mut audio_file = None;
    if let (true, Some(sa)) = (wants_audio, &scene_audio) {
        let part = audio::slice(&sa.mixed.master, sa.mix.rate, start, end);
        let weights = sa.mix.layout.loudness_weights();
        // integrated loudness needs at least one 400 ms gating block
        let l = sr_audio::loudness::integrated(&part, sa.mix.rate as f64, &weights);
        report.loudness = (l > -150.0).then_some(l);
        report.true_peak = Some(sr_audio::loudness::true_peak(&part));
        let container = output
            .container
            .map(|c| c.as_str().to_string())
            .and_then(|c| Container::parse(&c))
            .or_else(|| Container::from_path(&report.path));
        if codec.is_audio_only() && container.is_none_or(|c| c == Container::Wav) {
            // final PCM with TPDF dither at the mix bit depth
            sr_audio::wav::write(&report.path, &part, sa.mix.rate, sa.bits, sa.mix.layout, sa.dither)?;
            report.files.push(report.path.clone());
            report.passes = 1;
            report.encoder = format!("pcm_s{}le", sa.bits);
        } else {
            let f = tmp.join("mix.wav");
            sr_audio::wav::write(&f, &part, sa.mix.rate, 32, sa.mix.layout, false)?;
            audio_file = Some(f);
        }
    }
    if codec.is_audio_only() {
        if report.files.is_empty() {
            let Some(f) = audio_file.clone() else {
                return Err(DeliverError::Invalid("audio-only output, but the scene has no audio".into()));
            };
            let spec = base_spec(
                output,
                &report.path,
                codec,
                [16, 16],
                fps,
                InputFormat::Nv12,
                &OutputColor::new(m::ColorSpace::Srgb, m::Transfer::Auto, false),
                Some((f, output.audio_codec.clone(), output.audio_bitrate)),
                p,
                opts,
            );
            let enc = Encoder::start(&spec)?;
            report.encoder = enc.encoder.clone();
            enc.finish()?;
            report.files.push(report.path.clone());
            report.passes = 1;
        }
    } else {
        let gpu = match gpu {
            Some(g) => g.clone(),
            None => Gpu::new()?,
        };
        // 360 video renders at the scene360 size unless the output asks for another
        // a layout that crops or fits delivers at its own frame size
        let frame = p
            .scene
            .scene360
            .as_ref()
            .map(|s| [s.width as u32, s.height as u32])
            .or(p.reframe.map(|r| [r.size[0].round() as u32, r.size[1].round() as u32]))
            .unwrap_or([p.size[0].round() as u32, p.size[1].round() as u32]);
        let size =
            [output.width.map(|w| w as u32).unwrap_or(frame[0]), output.height.map(|h| h as u32).unwrap_or(frame[1])];
        report.size = size;
        let format = input_format(output, codec);
        let exr = codec == Codec::ExrSequence;
        let rgb_image = codec.is_sequence() || matches!(codec, Codec::Gif | Codec::Apng | Codec::Webp);
        let color = if exr {
            OutputColor::new(
                if output.color_space == m::ColorSpace::Srgb { m::ColorSpace::LinearSrgb } else { output.color_space },
                m::Transfer::Linear,
                true,
            )
        } else {
            OutputColor::new(output.color_space, output.transfer, output.color_range.as_str() == "full" || rgb_image)
        };
        let mut renderer = Renderer::new(gpu.clone(), p);
        renderer.representation = representation.clone();
        renderer.burn_captions = output.burn_captions.clone();
        if let Some(sa) = scene_audio.as_mut() {
            // the mix is done with these buffers (the master was sliced to disk above), so
            // the shaders take them over rather than doubling a programme's worth of audio
            let mut channels: std::collections::HashMap<String, std::sync::Arc<Vec<Vec<f32>>>> =
                std::mem::take(&mut sa.mixed.nodes).into_iter().map(|(k, v)| (k, std::sync::Arc::new(v))).collect();
            channels.insert("master".into(), std::sync::Arc::new(std::mem::take(&mut sa.mixed.master)));
            renderer.audio =
                Some(std::sync::Arc::new(sr_gpu::shader::AudioSignals { rate: sa.mix.rate as f64, channels }));
        }
        let frame_times: Option<Vec<crate::segments::FrameTime>> = segments.as_ref().map(|tm| {
            tm.frames(fps).into_iter().filter(|f| f.output >= start - 1e-9 && f.output < end - 1e-9).collect()
        });
        let times: Vec<f64> = match &frame_times {
            Some(f) => f.iter().map(|f| f.composition).collect(),
            None => {
                let n = ((end - start) * fps).round().max(1.0) as u64;
                (0..n).map(|k| start + k as f64 / fps).collect()
            }
        };
        let n = times.len() as u64;
        let mut video = Video {
            ev: &ev,
            renderer,
            stage: OutputStage::new(gpu.device.clone(), gpu.queue.clone()),
            color,
            format,
            size,
            keep_alpha: output.alpha || rgb_image,
            fps,
            reframe: p.reframe,
            segments: segments.as_ref().zip(frame_times),
            overlay: crate::overlay::Overlay::new(doc, output, timeline, size, &gpu)?,
            join_tex: None,
        };
        let audio_in = audio_file.clone().map(|f| (f, output.audio_codec.clone(), output.audio_bitrate));
        let mut spec = base_spec(output, &report.path, codec, size, fps, format, &color, audio_in, p, opts);
        spec.start_number = (start * fps).round() as u64;
        let fit = output.max_file_size;
        let t_video = Instant::now();
        report.segments = 1;
        let segments = segment_count(output, codec, opts, &ev, end - start).min(n as usize);
        if output.two_pass || fit.is_some() {
            // render once into a lossless intermediate, then encode it as often as needed
            let inter = tmp.join("intermediate.mkv");
            let ipix = match format {
                InputFormat::Nv12 => "yuv420p",
                InputFormat::P010 => "yuv420p10le",
                InputFormat::Rgba8 => "bgra",
                _ => "gbrap16le",
            };
            let ispec = EncodeSpec {
                path: inter.clone(),
                codec: Codec::Ffv1,
                container: Some(Container::Mkv),
                pixel_format: ipix.into(),
                audio: None,
                pass: None,
                alpha: false,
                hdr: Hdr::default(),
                metadata: Vec::new(),
                bitrate: None,
                ..spec.clone()
            };
            let mut feeder = Feeder::start(&ispec)?;
            let mut done = 0;
            video.stage.seek(spec.start_number as u32);
            video.run(&times, &mut report, |b| {
                done += 1;
                progress(done, n);
                feeder.send(b)
            })?;
            feeder.finish()?;
            report.frames = n;
            let duration = end - start;
            let audio_bps = if spec.audio.is_some() { output.audio_bitrate } else { 0 };
            let mut bitrate = fit.map(|max| sr_media::encode::fit_bitrate(max, duration, audio_bps)).or(output.bitrate);
            for attempt in 0..4 {
                let log = tmp.join(format!("pass{attempt}"));
                let passes: &[u8] = if output.two_pass || fit.is_some() { &[1, 2] } else { &[0] };
                for &pass in passes {
                    let s = EncodeSpec { bitrate, pass: (pass > 0).then(|| (pass, log.clone())), ..spec.clone() };
                    report.encoder = replay(&inter, &s, format, size)?;
                    report.passes += 1;
                }
                match fit {
                    Some(max) => {
                        let got = std::fs::metadata(&report.path)?.len();
                        if got <= max {
                            break;
                        }
                        if attempt == 3 {
                            return Err(DeliverError::Invalid(format!(
                                "{} is {got} bytes after four attempts; maxFileSize is {max}",
                                report.path.display()
                            )));
                        }
                        bitrate = bitrate.map(|b| ((b as f64) * (max as f64 / got as f64) * 0.95) as u64);
                    }
                    None => break,
                }
            }
        } else if segments > 1 {
            // `segments` workers, each with its own renderer and encoder, take contiguous chunks of
            // the range in order (about three per worker, so a heavy stretch does not hold up the
            // rest); every chunk starts on a keyframe and the chunks are joined by stream copy
            use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
            let chunks = (segments * 3).min(((end - start) / 10.0).floor() as usize).max(segments).min(n as usize);
            let bounds: Vec<usize> = (0..=chunks).map(|i| (i as u64 * n / chunks as u64) as usize).collect();
            let ext = report.path.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_string();
            let parts: Vec<PathBuf> = (0..chunks).map(|i| tmp.join(format!("segment{i:04}.{ext}"))).collect();
            let part_spec = |i: usize| EncodeSpec {
                path: parts[i].clone(),
                audio: None,
                faststart: false,
                metadata: Vec::new(),
                ..spec.clone()
            };
            let (audio, keep_alpha) = (video.renderer.audio.clone(), video.keep_alpha);
            let next = AtomicUsize::new(0);
            let done = AtomicU64::new(0);
            let results: Vec<Result<(Report, String), DeliverError>> = std::thread::scope(|sc| {
                let handles: Vec<_> = (0..segments)
                    .map(|_| {
                        sc.spawn(|| -> Result<(Report, String), DeliverError> {
                            // a device of its own: renderers sharing one device slowed each other
                            // down to a crawl over a long programme (buffer creation dominated)
                            let gpu = Gpu::new()?;
                            let mut renderer = Renderer::new(gpu.clone(), p);
                            renderer.representation = representation.clone();
                            renderer.burn_captions = output.burn_captions.clone();
                            renderer.audio = audio.clone();
                            let stage = OutputStage::new(gpu.device.clone(), gpu.queue.clone());
                            let mut worker = Video { ev: &ev, renderer, stage, color, format, size, keep_alpha };
                            let (mut part, mut encoder) = (Report::default(), String::new());
                            loop {
                                let i = next.fetch_add(1, Relaxed);
                                if i >= chunks {
                                    return Ok((part, encoder));
                                }
                                let chunk = (|| {
                                    let mut feeder = Feeder::start(&part_spec(i))?;
                                    worker.stage.seek((spec.start_number + bounds[i] as u64) as u32);
                                    worker.run(&times[bounds[i]..bounds[i + 1]], &mut part, |b| {
                                        done.fetch_add(1, Relaxed);
                                        feeder.send(b)
                                    })?;
                                    feeder.finish()
                                })();
                                match chunk {
                                    Ok(e) => encoder = e,
                                    Err(e) => {
                                        // stop the other workers at their next chunk
                                        next.store(chunks, Relaxed);
                                        return Err(e);
                                    }
                                }
                            }
                        })
                    })
                    .collect();
                while !handles.iter().all(|h| h.is_finished()) {
                    progress(done.load(Relaxed), n);
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                handles
                    .into_iter()
                    .map(|h| {
                        h.join().unwrap_or_else(|_| Err(DeliverError::Invalid("a segment render panicked".into())))
                    })
                    .collect()
            });
            let mut unsupported = std::collections::BTreeSet::new();
            for r in results {
                let (part, encoder) = r?;
                for (a, b) in report.stage_seconds.iter_mut().zip(part.stage_seconds) {
                    *a += b;
                }
                report.decode_wait_seconds += part.decode_wait_seconds;
                report.vector_seconds += part.vector_seconds;
                unsupported.extend(part.unsupported);
                if report.encoder.is_empty() {
                    report.encoder = encoder;
                }
            }
            report.unsupported = unsupported.into_iter().collect();
            progress(n, n);
            spec.join(&parts, &tmp.join("segments.txt"))?;
            report.frames = n;
            report.passes = 1;
            report.segments = segments as u32;
        } else {
            let mut feeder = Feeder::start(&spec)?;
            let mut done = 0;
            video.stage.seek(spec.start_number as u32);
            video.run(&times, &mut report, |b| {
                done += 1;
                progress(done, n);
                feeder.send(b)
            })?;
            report.encoder = feeder.finish()?;
            report.frames = n;
            report.passes = 1;
        }
        report.render_fps = n as f64 / t_video.elapsed().as_secs_f64();
        if let Some(msg) = report.accessibility_error.clone() {
            return Err(DeliverError::Accessibility(msg));
        }
        if let Some(s360) = p.scene.scene360.as_ref().filter(|_| output.spherical_metadata && !codec.is_sequence()) {
            use sr_media::spherical::{inject, Projection, Stereo};
            let projection = match s360.layout {
                m::Scene360Layout::Equirectangular => Some(Projection::Equirectangular),
                m::Scene360Layout::Cubemap => Some(Projection::Cubemap),
                _ => None,
            };
            let stereo = match s360.stereo {
                m::Stereo::Mono => Stereo::Mono,
                m::Stereo::TopBottom => Stereo::TopBottom,
                m::Stereo::LeftRight => Stereo::LeftRight,
            };
            let ext = report.path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            match projection {
                Some(pr) if matches!(ext.as_str(), "mp4" | "mov" | "m4v") => {
                    if let Err(e) = inject(&report.path, pr, stereo) {
                        report.unsupported.push(format!("spherical metadata: {e}"));
                    }
                }
                Some(_) => {
                    report.unsupported.push(format!("spherical metadata is written for MP4 and MOV only, not .{ext}"))
                }
                None => report.unsupported.push(format!(
                    "spherical metadata for {} needs a mesh projection, which is not written",
                    s360.layout
                )),
            }
        }
        if codec.is_sequence() {
            for k in 0..n {
                report.files.push(PathBuf::from(sequence_name(&report.path, spec.start_number + k)));
            }
        } else {
            report.files.push(report.path.clone());
        }
        // posters and thumbnails
        for c in &output.children {
            let (m::OutputChild::Poster(st) | m::OutputChild::Thumbnail(st)) = c else { continue };
            let t = match (&st.marker, &segments) {
                (Some(mk), _) => sr_eval::eval::marker(p, mk).unwrap_or(st.time),
                // a still's time is output time
                (None, Some(tm)) => tm.at(st.time.clamp(0.0, tm.duration)).composition,
                (None, None) => st.time,
            };
            let g = ev.evaluate(t.clamp(0.0, p.duration));
            let mut sub = |st: f64| ev.evaluate(st);
            let frame = video.renderer.render_with(&g, p, Some(&mut sub));
            let psize = [p.size[0].round() as u32, p.size[1].round() as u32];
            let srgb = OutputColor::new(m::ColorSpace::Srgb, m::Transfer::Srgb, true);
            let pending =
                video.stage.submit(&frame.texture, &video.renderer.working(), &srgb, InputFormat::Rgba8, psize, true);
            let rgba = video.stage.wait(pending);
            let path = resolve(&base, &st.path);
            let fmt = StillFormat::parse(st.format.as_str()).unwrap_or(StillFormat::Jpeg);
            sr_media::encode::write_still(
                &path,
                &rgba,
                psize[0],
                psize[1],
                fmt,
                st.width.map(|w| w as u32),
                st.quality.get(),
            )?;
            report.files.push(path);
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    if segments.is_some() {
        if p.scene.captions.is_some() {
            report.unsupported.push("caption files of an output with segments are not written yet".into());
        }
    } else {
        report.files.extend(write_sidecars(p, output, &report.path, start, end)?);
    }
    if opts.upload {
        let dests: Vec<&m::Destination> = output
            .children
            .iter()
            .filter_map(|c| if let m::OutputChild::Destination(d) = c { Some(d) } else { None })
            .collect();
        report.uploads = crate::destinations::deliver_all(&dests, &report.files, output, &base)?;
    }
    report.seconds = started.elapsed().as_secs_f64();
    Ok(report)
}

/// Segments to render `output` in at once: 1 unless it is a single-pass video file whose frames
/// render the same from a cold start (no simulation, no accessibility analysis across frames).
fn segment_count(output: &m::Output, codec: Codec, opts: &Options, ev: &Evaluator, duration: f64) -> usize {
    let p = ev.program();
    let accessibility = p.scene.metadata.as_ref().is_some_and(|m| {
        m.children.iter().any(|c| match c {
            m::MetadataChild::Accessibility(a) => {
                a.flash_check.to_string() != "off" || a.contrast_check.to_string() != "off"
            }
            _ => false,
        })
    });
    if output.two_pass
        || output.max_file_size.is_some()
        || codec.is_sequence()
        || codec.is_audio_only()
        || matches!(codec, Codec::Gif | Codec::Apng | Codec::Webp)
        || accessibility
        || ev.has_simulation()
    {
        return 1;
    }
    let wanted = match opts.parallel {
        Parallel::Count(n) => n as usize,
        Parallel::Auto => {
            // measured on a 229 s programme: NVENC levels off at 3, libx264 (CPU-bound) at 4 on 8 cores
            let hardware = sr_media::encode::choose_encoder(codec, opts.hardware)
                .is_ok_and(|e| ["_nvenc", "_videotoolbox", "_qsv", "_amf", "_vaapi"].iter().any(|s| e.ends_with(s)));
            let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
            let n = if hardware { 3 } else { (cores / 2).clamp(1, 4) };
            // every segment pays the renderer's start-up: keep them at least 20 s long
            n.min((duration / 20.0).floor() as usize).max(1)
        }
    };
    wanted.max(1)
}

#[allow(clippy::too_many_arguments)]
fn base_spec(
    o: &m::Output,
    path: &Path,
    codec: Codec,
    size: [u32; 2],
    fps: f64,
    input: InputFormat,
    color: &OutputColor,
    audio: Option<(PathBuf, String, u64)>,
    p: &sr_eval::Program,
    opts: &Options,
) -> EncodeSpec {
    let (primaries, transfer, matrix) = color.ffmpeg_tags();
    EncodeSpec {
        path: path.to_path_buf(),
        codec,
        container: o.container.and_then(|c| Container::parse(c.as_str())),
        width: size[0],
        height: size[1],
        fps,
        input,
        pixel_format: o.pixel_format.clone(),
        preset: o.preset.clone(),
        profile: o.profile.clone(),
        level: o.level.clone(),
        prores_profile: o.prores_profile.map(|p| p.as_str().to_string()),
        crf: o.crf as u32,
        bitrate: o.bitrate,
        max_bitrate: o.max_bitrate,
        buffer_size: o.buffer_size,
        pass: None,
        keyframe_interval: o.keyframe_interval.get(),
        b_frames: o.b_frames.map(|b| b as u32),
        faststart: o.faststart,
        alpha: o.alpha,
        audio,
        loop_count: o.loop_count as u32,
        start_number: 0,
        color: ColorTags { primaries, transfer, matrix, full_range: color.full_range },
        hdr: Hdr {
            max_cll: o.max_cll.map(|v| v as u32),
            max_fall: o.max_fall.map(|v| v as u32),
            mastering_display: o.mastering_display.clone(),
        },
        metadata: if o.embed_metadata { metadata(&p.scene) } else { Vec::new() },
        hardware: opts.hardware,
        audio_bits: p.scene.audio_mix.as_ref().map(|a| a.bit_depth.as_str().parse().unwrap_or(24)).unwrap_or(24),
    }
}

/// File name of frame `k` of a printf-patterned sequence path.
pub fn sequence_name(pattern: &Path, k: u64) -> String {
    let s = pattern.to_string_lossy();
    if let Some(i) = s.find('%') {
        let rest = &s[i + 1..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if rest[digits.len()..].starts_with('d') {
            let width: usize = digits.trim_start_matches('0').parse().unwrap_or(0);
            return format!("{}{k:0width$}{}", &s[..i], &rest[digits.len() + 1..]);
        }
    }
    s.into_owned()
}

/// Decodes the intermediate and feeds it to an encoder for one pass.
fn replay(inter: &Path, spec: &EncodeSpec, format: InputFormat, size: [u32; 2]) -> Result<String, DeliverError> {
    use std::io::Read;
    let pix = match format {
        InputFormat::Nv12 => "nv12",
        InputFormat::P010 => "p010le",
        InputFormat::Rgba8 => "rgba",
        InputFormat::Rgba16 => "rgba64le",
        InputFormat::Gbrapf32 => "gbrapf32le",
    };
    let mut dec = std::process::Command::new(sr_media::ffmpeg())
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(inter)
        .args(["-f", "rawvideo", "-pix_fmt", pix, "pipe:1"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| sr_media::MediaError::Spawn { tool: sr_media::ffmpeg(), source: e })?;
    let mut out = dec.stdout.take().expect("piped");
    let mut enc = Encoder::start(spec)?;
    let mut buf = vec![0u8; format.frame_bytes(size[0], size[1])];
    while out.read_exact(&mut buf).is_ok() {
        enc.write(&buf)?;
    }
    let _ = dec.wait();
    let name = enc.encoder.clone();
    enc.finish()?;
    Ok(name)
}

/// Caption sidecars: tracks listed in the output's `captions`, or with mode
/// sidecar or both, written next to the output as `<stem>.<track>.<language>.vtt`
/// (`.srt` when the track's source is SRT), with times relative to the range start.
fn write_sidecars(
    p: &sr_eval::Program,
    output: &m::Output,
    out: &std::path::Path,
    start: f64,
    end: f64,
) -> Result<Vec<PathBuf>, DeliverError> {
    let Some(caps) = p.scene.captions.as_ref() else { return Ok(Vec::new()) };
    let base = p.base_dirs.first().cloned().unwrap_or_default();
    let mut files = Vec::new();
    for tr in &caps.caption_tracks {
        let listed = output.captions.as_ref().is_some_and(|ids| ids.contains(&tr.id));
        let mode = tr.mode.to_string();
        if !listed && mode == "burn" {
            continue;
        }
        let cues = sr_gpu::text::track_cues(tr, &base).map_err(DeliverError::Invalid)?;
        let cues: Vec<sr_text::captions::Cue> = cues
            .into_iter()
            .filter(|c| c.end > start && c.start < end)
            .map(|mut c| {
                c.start = (c.start - start).max(0.0);
                c.end = (c.end.min(end) - start).max(c.start);
                for w in &mut c.words {
                    w.start = (w.start - start).max(0.0);
                    w.end = (w.end - start).max(w.start);
                }
                c
            })
            .collect();
        let srt = tr.format.as_ref().map(|f| f.to_string()) == Some("srt".into());
        let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
        let name = format!("{stem}.{}.{}.{}", tr.id, tr.language, if srt { "srt" } else { "vtt" });
        let path = out.with_file_name(name);
        let text = if srt { sr_text::captions::to_srt(&cues) } else { sr_text::captions::to_vtt(&cues) };
        std::fs::write(&path, text)?;
        files.push(path);
    }
    Ok(files)
}

/// Index (into `frames`' values) of the middle frame of the longest run of consecutive indices.
fn middle_of_longest_run(frames: &[usize]) -> Option<usize> {
    let (mut best, mut start) = ((0usize, 0usize), 0usize);
    for i in 1..=frames.len() {
        if i == frames.len() || frames[i] != frames[i - 1] + 1 {
            if i - start > best.1 - best.0 {
                best = (start, i);
            }
            start = i;
        }
    }
    (best.1 > best.0).then(|| frames[(best.0 + best.1 - 1) / 2])
}

#[cfg(test)]
mod run_tests {
    use super::middle_of_longest_run;

    #[test]
    fn middle_of_longest_run_picks_the_longest_contiguous_span() {
        assert_eq!(middle_of_longest_run(&[]), None);
        assert_eq!(middle_of_longest_run(&[7]), Some(7));
        assert_eq!(middle_of_longest_run(&[0, 1, 5, 6, 7, 8, 9, 20]), Some(7));
        assert_eq!(middle_of_longest_run(&[3, 4, 10, 11]), Some(3));
    }
}
