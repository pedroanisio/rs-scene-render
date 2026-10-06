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

use crate::access::{Judge, Observe, Seen, Streaming, OBSERVATION_FRAMES};
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
    /// Quality tier overriding the document's `project@quality`.
    pub quality: Option<m::ProjectQuality>,
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
            quality: None,
        }
    }
}

/// Device chosen for scene rendering, independently of the video encoder.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RenderAdapter {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    /// True for a software rasteriser such as llvmpipe.
    pub software: bool,
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
    /// Rendering device, independently selected from the video encoder. Absent for audio-only output.
    pub render_adapter: Option<RenderAdapter>,
    /// Quality tier the frames were rendered at (`draft`, `preview` or `final`).
    pub quality: String,
    /// Wall-clock seconds for the whole output.
    pub seconds: f64,
    /// Frames per second achieved for the video pass.
    pub render_fps: f64,
    /// Seconds spent evaluating (waiting for the next graph, when a helper thread evaluates
    /// ahead), rendering (CPU planning and submission), waiting for converted frames, and
    /// blocked on the encoder. Waiting for converted frames is residual blocking after
    /// overlapping work, not total GPU execution time; render may include internal waits.
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
    /// The decoded-AAC check of the master's true-peak ceiling: what it measured and any gain it applied.
    pub audio_ceiling: Option<crate::ceiling::Held>,
    /// Content not rendered by this batch, per node.
    pub unsupported: Vec<String>,
    /// Accessibility findings (flash analysis, text contrast, required captions).
    pub accessibility: Vec<String>,
    /// The finding that fails delivery (a check set to `error`).
    #[serde(skip)]
    pub accessibility_error: Option<String>,
    /// Delivery warnings: content that renders but is likely a mistake (a segment starting on an
    /// empty frame, a caption shown too briefly to read).
    pub warnings: Vec<String>,
    /// Warnings raised by composition or overlay templating; strict delivery rejects these.
    pub evaluation_warnings: Vec<sr_model::Diagnostic>,
    /// Remote locations the files were delivered to.
    pub uploads: Vec<String>,
    /// Encoded passes (2 for two-pass, more when fitting a file size).
    pub passes: u32,
    /// Time segments rendered and encoded at once (1 for a serial render).
    pub segments: u32,
    /// Why the render ran with one worker although more were asked for or chosen (GPU debug layers on).
    pub serial_because: Option<String>,
}

/// An `<output>` element built from command-line settings.
pub fn adhoc_output(path: &str, codec: &str) -> Result<m::Output, DeliverError> {
    // a command-line path is a file path, not a URI: the rules judge its file name (extension, frame
    // pattern), and a `%` in a directory is no escape
    let name = Path::new(path).file_name().map_or(path.into(), |n| n.to_string_lossy());
    let xml = format!(r#"<output path="{}" codec="{}"/>"#, xml_escape(&name), xml_escape(codec));
    let doc = format!(
        r##"<scene version="1.1"><project width="16" height="16" fps="1" duration="1"/>{xml}<composition/></scene>"##
    );
    let d = sr_model::load_str(&doc, &sr_model::LoadOptions::without_assets()).map_err(|e| match e {
        sr_model::LoadError::Invalid(r) => DeliverError::Document(r),
        e => DeliverError::Invalid(e.to_string()),
    })?;
    Ok(m::Output { path: path.to_string(), ..d.scene.outputs[0].clone() })
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
    /// Without segments, the composition time of output time 0 (`output/@start`).
    origin: f64,
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

    /// The picture of a frame and how to place it: the render `frame` of segment time `f`, or inside a
    /// join both sides, each through its own map and placed with its own focus, combined in the
    /// output's frame.
    fn picture(
        &mut self,
        frame: &sr_gpu::Frame,
        f: Option<&crate::segments::FrameTime>,
        unsupported: &mut std::collections::BTreeSet<String>,
    ) -> Result<(std::sync::Arc<sr_gpu::resources::Tex>, sr_gpu::output::Placement), DeliverError> {
        let tm = self.segments.as_ref().map(|(tm, _)| *tm);
        let Some(j) = tm.zip(f).and_then(|(tm, f)| tm.join_at(f.output)) else {
            return Ok((frame.texture.clone(), self.placement(f)));
        };
        let (ev, working) = (self.ev, self.renderer.working());
        let tex = self.join_tex.get_or_insert_with(|| [(); 3].map(|_| self.renderer.texture(self.size))).clone();
        let own = if f.is_some_and(|f| f.segment == j.from.segment) { 0 } else { 1 };
        let sides = [j.from, j.to];
        self.stage.place(&frame.texture, &working, &tex[own], self.placement(Some(&sides[own])));
        let other = sides[1 - own];
        let go = ev.evaluate(other.composition);
        let f2 = self.render_side(&go, other.composition, Some(&other))?;
        unsupported.extend(f2.stats.unsupported.iter().cloned());
        self.stage.place(&f2.texture, &working, &tex[1 - own], self.placement(Some(&other)));
        let problems = self.renderer.join(
            ev.program(),
            &j.join.elem,
            &tex[0],
            &tex[1],
            j.progress,
            j.velocity,
            j.from.output,
            &tex[2],
        );
        unsupported.extend(problems);
        Ok((tex[2].clone(), sr_gpu::output::Placement::default()))
    }
}

impl Video<'_> {
    /// Renders `times` and hands each converted frame to `sink` and what the accessibility checks
    /// take from it to `seen`; adds stage timings to `report`. The checks' verdicts come from
    /// [`Video::judge`], once every frame of the output has been observed.
    fn run(
        &mut self,
        times: &[f64],
        report: &mut Report,
        seen: &mut dyn Observe,
        mut sink: impl FnMut(Vec<u8>) -> Result<f64, DeliverError>,
    ) -> Result<(), DeliverError> {
        let p = self.ev.program();
        let working = self.renderer.working();
        let mut pending: Option<(f64, sr_gpu::output::Pending)> = None;
        // A worker reuses its report for several chunks; retain earlier findings.
        let mut unsupported = report.unsupported.iter().cloned().collect::<std::collections::BTreeSet<_>>();
        let checks = Checks::of(p);
        let flash_picture = checks.flash_on().then(|| self.renderer.texture(self.size));
        self.renderer.contrast_probe = checks.contrast_on();
        let opacity_of = |g: &sr_eval::FrameGraph, id: &sr_gpu::ContrastTarget| match id {
            sr_gpu::ContrastTarget::Node(id) => {
                g.nodes.iter().find(|n| &*n.id == id).map(|n| n.world_opacity).unwrap_or(1.0)
            }
            sr_gpu::ContrastTarget::Captions => 1.0,
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
                let out_t = ft.map_or(t - self.origin, |f| f.output);
                let frame = self.render_side(&g, t, ft.as_ref())?;
                unsupported.extend(frame.stats.unsupported.iter().cloned());
                for id in &frame.stats.contrast_unprobed {
                    seen.unprobed(id, k, opacity_of(&g, id));
                }
                for (id, ratio) in &frame.stats.contrast {
                    seen.contrast(id, opacity_of(&g, id), *ratio, t);
                }
                report.decode_wait_seconds += frame.stats.decode_wait;
                report.vector_seconds += frame.stats.vector_seconds;
                let (picture, placement) = self.picture(&frame, ft.as_ref(), &mut unsupported)?;
                if checks.contrast_on() {
                    if let Some(o) = self.overlay.as_mut() {
                        for (id, opacity, ratio) in o.contrast(&mut self.stage, out_t, &picture, placement)? {
                            let id = sr_gpu::ContrastTarget::Node(format!("output layer: {id}"));
                            seen.contrast(&id, opacity, ratio, out_t);
                        }
                    }
                }
                let over = match self.overlay.as_mut() {
                    Some(o) => {
                        let (tex, problems) = o.draw(&mut self.stage, out_t)?;
                        unsupported.extend(problems);
                        Some(tex)
                    }
                    None => None,
                };
                if let Some(tex) = flash_picture.as_ref() {
                    self.stage.place_with_overlay(&picture, &working, tex, placement, over);
                    seen.flash(out_t, self.renderer.flash_grid(tex));
                }
                seen.finish_frame().map_err(DeliverError::Invalid)?;
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
                meter::add(&mut report.stage_seconds, 0, (t1 - t0).as_secs_f64());
                meter::add(&mut report.stage_seconds, 1, (t2 - t1).as_secs_f64());
                if let Some((_, prev)) = pending.replace((t, next)) {
                    let w = Instant::now();
                    let bytes = self.stage.wait(prev);
                    meter::add(&mut report.stage_seconds, 2, w.elapsed().as_secs_f64());
                    meter::add(&mut report.stage_seconds, 3, sink(bytes)?);
                }
                if k + 1 == times.len() {
                    if let Some((_, last)) = pending.take() {
                        let w = Instant::now();
                        let bytes = self.stage.wait(last);
                        meter::add(&mut report.stage_seconds, 2, w.elapsed().as_secs_f64());
                        meter::add(&mut report.stage_seconds, 3, sink(bytes)?);
                    }
                }
            }
            Ok(())
        })?;
        report.unsupported = unsupported.into_iter().collect();
        Ok(())
    }

    /// Completes the accessibility checks over an output and records their findings in `report`.
    /// `judge` has observed every frame of `times`, in order; text the inline probe could not reach is
    /// measured here, then the verdicts are given.
    fn judge(&mut self, mut judge: Judge, times: &[f64], report: &mut Report) {
        let p = self.ev.program();
        let checks = Checks::of(p);
        // Probe over the whole span of maximum visibility. A middle-frame sample can miss low
        // contrast at either end, or a change in the text's backdrop.
        for (id, seen) in std::mem::take(&mut judge.unprobed) {
            for (k, opacity) in probe_frames(&seen) {
                let t = times[k];
                let g = self.ev.evaluate(t);
                let ev = &self.ev;
                let mut sub = |st: f64| ev.evaluate(st);
                if let Some(ratio) = self.renderer.contrast_with_without(&g, p, &id, &mut sub) {
                    judge.contrast(&id, opacity, ratio, t);
                }
            }
        }
        // accessibility verdicts
        let fail = |mode: &str, msg: String, report: &mut Report| {
            if mode == "error" && report.accessibility_error.is_none() {
                report.accessibility_error = Some(msg.clone());
            }
            report.accessibility.push(msg);
        };
        if let Some(msg) = judge.flash.as_ref().and_then(|d| d.verdict()) {
            fail(&checks.flash, msg, report);
        }
        let min_contrast = checks.min_contrast;
        for (id, (_, ratio, at)) in &judge.lowest {
            if *ratio < min_contrast {
                fail(&checks.contrast, format!("contrastCheck: {id} reaches only {ratio:.2}:1 against its background at {at:.3} s (minimum {min_contrast}:1)"), report);
            }
        }
    }
}

/// The accessibility checks a document asks for on its rendered frames.
struct Checks {
    /// `flashCheck`: `off`, `warn` or `error`.
    flash: String,
    /// `contrastCheck`: `off`, `warn` or `error`.
    contrast: String,
    /// `minContrast`.
    min_contrast: f64,
}

impl Checks {
    fn of(p: &sr_eval::Program) -> Checks {
        let acc = p.scene.metadata.as_ref().and_then(|m| {
            m.children.iter().find_map(|c| match c {
                m::MetadataChild::Accessibility(a) => Some(a),
                _ => None,
            })
        });
        // Without an <accessibility> element nothing is checked: the XSD defaults (flashCheck
        // "warn", contrastCheck "off") apply to the element's attributes, and the checks
        // are skipped when it is absent. Declare <accessibility/> to opt in.
        Checks {
            flash: acc.map(|a| a.flash_check.to_string()).unwrap_or_else(|| "off".into()),
            contrast: acc.map(|a| a.contrast_check.to_string()).unwrap_or_else(|| "off".into()),
            min_contrast: acc.map(|a| a.min_contrast.get()).unwrap_or(4.5),
        }
    }

    fn flash_on(&self) -> bool {
        self.flash != "off"
    }

    fn contrast_on(&self) -> bool {
        self.contrast != "off"
    }
}

/// Writes frames through an encoder on its own thread; returns blocked seconds per frame.
struct Feeder {
    tx: Option<std::sync::mpsc::SyncSender<Vec<u8>>>,
    handle: Option<std::thread::JoinHandle<Result<String, sr_media::MediaError>>>,
    /// Set once every frame was sent: an encode whose frames stop short is abandoned, not finished.
    complete: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Feeder {
    fn start(spec: &EncodeSpec) -> Result<Feeder, DeliverError> {
        use std::sync::atomic::Ordering::Relaxed;
        let mut enc = Encoder::start(spec)?;
        let (tx, rx) = sync_channel::<Vec<u8>>(3);
        let complete = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let all_sent = complete.clone();
        let handle = std::thread::Builder::new().name("sr-encode".into()).spawn(move || {
            for f in rx {
                enc.write(&f)?;
            }
            if !all_sent.load(Relaxed) {
                // the render failed: dropping the encoder stops FFmpeg and removes the partial file
                return Err(sr_media::MediaError::Invalid("the encode was abandoned".into()));
            }
            let name = enc.encoder.clone();
            enc.finish()?;
            Ok(name)
        })?;
        Ok(Feeder { tx: Some(tx), handle: Some(handle), complete })
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
        self.complete.store(true, std::sync::atomic::Ordering::Relaxed);
        drop(self.tx.take());
        match self.handle.take() {
            Some(h) => Ok(h.join().map_err(|_| DeliverError::Invalid("encoder thread panicked".into()))??),
            None => Ok(String::new()),
        }
    }
}

impl Drop for Feeder {
    fn drop(&mut self) {
        // not finished: wait for the encoder to be stopped, so nothing of it outlives the delivery
        drop(self.tx.take());
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// The delivery's temporary directory, removed when the delivery ends, however it ends.
struct Scratch(PathBuf);

impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The evaluation options an output is rendered with.
fn eval_options(output: &m::Output, opts: &Options) -> EvalOptions {
    EvalOptions {
        variant: output.variant.clone(),
        layout: output.layout.clone(),
        params: opts.params.clone(),
        row: opts.row.clone(),
        ..Default::default()
    }
}

/// Whether rendering `output` draws anything in 3D, so a caller that makes the GPU can ask for one
/// that can run the 3D pass (`deliver` makes its own for a caller that passes none).
pub fn output_uses_3d(doc: &sr_model::Document, output: &m::Output, opts: &Options) -> Result<bool, DeliverError> {
    let ev = Evaluator::new(doc, &eval_options(output, opts)).map_err(DeliverError::Document)?;
    Ok(ev.program().uses_3d())
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
    let mut eo = eval_options(output, opts);
    let ev0 = Evaluator::new(doc, &eo).map_err(DeliverError::Document)?;
    let p0 = ev0.program();
    let frame_rate = output.fps.unwrap_or(p0.fps);
    let fps = frame_rate.as_f64();
    // with segments, the output has its own timeline: start and end (from the command line) are output times
    let segments = crate::segments::TimeMap::of(p0, output).map_err(DeliverError::Invalid)?;
    let timeline = segments.as_ref().map(|tm| tm.duration).unwrap_or(p0.duration);
    let start = opts.start.unwrap_or(output.start).max(0.0);
    let end = opts.end.or(output.end).unwrap_or(timeline).min(timeline);
    if end <= start {
        return Err(DeliverError::Invalid(format!("empty range {start}..{end}")));
    }
    // CLI overrides define the effective output, including the origin/duration of its own
    // audio, overlay and captions. Explicit segments retain their independent output clock.
    let effective_output = m::Output { start, end: Some(end), ..output.clone() };
    let output = &effective_output;
    let mut report = Report {
        path: resolve(&base, &output.path),
        fps,
        quality: opts.quality.unwrap_or(doc.scene.project.quality).as_str().to_string(),
        ..Default::default()
    };
    report.warnings.extend(doc.warnings().iter().map(|d| format!("{}: {}", d.code, d.message)));
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
    report.evaluation_warnings.extend_from_slice(ev.warnings());
    if let Some(parent) = report.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // captions in output time: with segments the composition's are mapped; the output's own tracks
    let captions = crate::captions::output_captions(p, output, segments.as_ref())?;
    report.warnings.extend(captions.warnings.iter().cloned());
    // Check once for the delivery, before workers start. Composition tracks and
    // output-owned tracks both satisfy the document's caption requirement.
    let requires_captions = p.scene.metadata.as_ref().is_some_and(|md| {
        md.children.iter().any(|c| matches!(c, m::MetadataChild::Accessibility(a) if a.require_captions))
    });
    if requires_captions
        && captions.tracks.is_empty()
        && p.scene.captions.as_ref().is_none_or(|c| c.caption_tracks.is_empty())
    {
        return Err(DeliverError::Accessibility("requireCaptions: the output has no caption track".into()));
    }
    let tmp = Scratch(std::env::temp_dir().join(format!(
        "scene-render-{}-{}",
        std::process::id(),
        started.elapsed().as_nanos()
    )));
    std::fs::create_dir_all(&tmp.0)?;
    let mut chapters = None;
    if let Some(tm) = &segments {
        report.warnings.extend(empty_ends(&ev, tm));
        chapters = write_chapters(p, tm, start, end, &tmp)?;
    }
    // with segments the programme is the output's own mix in output time (its own tracks may be its
    // only audio)
    let takes_audio = output.audio && (codec.takes_audio() || codec.is_audio_only());
    let own_tracks = output.children.iter().any(|c| matches!(c, m::OutputChild::AudioTrack(_)));
    if takes_audio && scene_audio.is_none() && own_tracks {
        scene_audio = Some(audio::silent(p, fps));
    }
    // Without segments, selection attributes have no effect. Only output-owned
    // tracks need a programme of their own, starting at output/@start.
    let plain = match (&segments, own_tracks) {
        (None, true) => Some(crate::segment_audio::output_map(p, output)?),
        _ => None,
    };
    let programme = match (segments.as_ref().or(plain.as_ref()), &scene_audio) {
        (Some(tm), Some(sa)) if takes_audio => {
            let t = Instant::now();
            let a =
                crate::segment_audio::render(p, sa, output, tm, fps, representation.as_deref(), segments.is_some())?;
            report.audio_seconds += t.elapsed().as_secs_f64();
            Some(a.master)
        }
        _ => None,
    };
    let wants_audio = takes_audio && scene_audio.is_some() && (segments.is_none() || programme.is_some());
    let mut audio_file = None;
    if let (true, Some(sa)) = (wants_audio, &scene_audio) {
        // Plain programmes start at their mapped composition time; the scene and
        // explicitly segmented programmes start at zero on their respective clocks.
        let origin = plain.as_ref().map_or(0.0, |tm| tm.composition(0, 0.0));
        let master = programme.as_ref().unwrap_or(&sa.mixed.master);
        let mut part = audio::slice(master, sa.mix.rate, start - origin, end - origin);
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
        // a lossy encode can leave the master's ceiling: hold it for the stream that is delivered
        let master_settings = &sa.mix.master;
        let limited = master_settings.limiter || master_settings.normalize != sr_audio::mix::Normalize::None;
        if limited && crate::ceiling::applies(&output.audio_codec, container) {
            let held = crate::ceiling::hold(
                &mut part,
                sa.mix.rate,
                sa.mix.layout,
                master_settings.true_peak,
                output.audio_bitrate,
                &tmp,
            )?;
            if !held.passes.is_empty() {
                report.true_peak = Some(sr_audio::loudness::true_peak(&part));
                let l = sr_audio::loudness::integrated(&part, sa.mix.rate as f64, &weights);
                report.loudness = (l > -150.0).then_some(l);
            }
            report.audio_ceiling = Some(held);
        }
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
            let mut spec = base_spec(
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
            spec.chapters = chapters.clone();
            let enc = Encoder::start(&spec)?;
            report.encoder = enc.encoder.clone();
            enc.finish()?;
            report.files.push(report.path.clone());
            report.passes = 1;
        }
    } else {
        // a document with 3D objects needs an adapter that can run the 3D pass; the one asked for or handed in
        // that cannot is an error, not frames without the objects
        let gpu = match gpu {
            Some(g) => {
                if p.uses_3d() {
                    sr_gpu::gpu::require_3d(&g.info)?;
                }
                g.clone()
            }
            None => Gpu::new_for(p.uses_3d())?,
        };
        report.render_adapter = Some(RenderAdapter {
            name: gpu.info.name.clone(),
            backend: format!("{:?}", gpu.info.backend),
            device_type: format!("{:?}", gpu.info.device_type),
            software: gpu.is_software(),
        });
        // 360 video renders at the scene360 size unless the output asks for another
        // a layout that crops or fits delivers at its own frame size
        let frame = p
            .scene
            .scene360
            .as_ref()
            .map(|s| [s.width as f64, s.height as f64])
            .or(p.reframe.map(|r| r.size))
            .unwrap_or(p.size);
        let size = sr_gpu::output::frame_size(
            [output.width.map(|w| w as f64).unwrap_or(frame[0]), output.height.map(|h| h as f64).unwrap_or(frame[1])],
            &gpu.device.limits(),
        )
        .map_err(DeliverError::Invalid)?;
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
        renderer.quality = opts.quality;
        renderer.representation = representation.clone();
        renderer.burn_captions = output.burn_captions.clone();
        // with segments, captions burn in output time over the placed picture
        renderer.captions_off = segments.is_some();
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
                let n = frame_rate.frame_count(end - start).max(1);
                (0..n).map(|k| start + frame_rate.frame_time(k)).collect()
            }
        };
        // content a safe area holds to its region: findings are warnings, or fail before anything is rendered
        let (from, to) = (times.first().copied().unwrap_or(0.0), times.last().map(|t| t + 1.0 / fps).unwrap_or(0.0));
        let safe = sr_gpu::safe_audit::check(&ev, from, to);
        let diagnostics = safe.diagnostics(p);
        if diagnostics.iter().any(|d| d.is_error()) {
            return Err(DeliverError::Document(sr_model::Report { diagnostics }));
        }
        report.warnings.extend(diagnostics.iter().map(|d| format!("{}: {}", d.code, d.message)));
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
            overlay: crate::overlay::Overlay::new(
                doc,
                output,
                timeline,
                size,
                &captions,
                &gpu,
                &eo,
                representation.as_deref(),
            )?,
            origin: if segments.is_some() { 0.0 } else { output.start },
            join_tex: None,
        };
        if let Some(overlay) = &video.overlay {
            for warning in overlay.warnings() {
                if !report.evaluation_warnings.contains(warning) {
                    report.evaluation_warnings.push(warning.clone());
                }
            }
        }
        let audio_in = audio_file.clone().map(|f| (f, output.audio_codec.clone(), output.audio_bitrate));
        let mut spec = base_spec(output, &report.path, codec, size, fps, format, &color, audio_in, p, opts);
        spec.start_number = (start * fps).round() as u64;
        spec.chapters = chapters.clone();
        let fit = output.max_file_size;
        let checks = Checks::of(p);
        let t_video = Instant::now();
        report.segments = 1;
        // llvmpipe already spreads one device over every core, so automatic parallelism would only add
        // devices competing for them; an explicit worker count is still honoured
        let wanted_workers = if gpu.is_software() && matches!(opts.parallel, Parallel::Auto) {
            1
        } else {
            segment_count(output, codec, opts, &ev, end - start).min(n as usize)
        };
        // the debug layers name every object through the Vulkan loader, which is not safe from several devices at once
        let workers = if sr_gpu::gpu::debug_layers() && wanted_workers > 1 {
            report.serial_because = Some(format!(
                "GPU debug layers are on (SR_GPU_DEBUG / --debug-gpu): rendering with 1 worker instead of {wanted_workers}, as their object naming is not safe from several devices at once"
            ));
            1
        } else {
            wanted_workers
        };
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
                chapters: None,
                bitrate: None,
                ..spec.clone()
            };
            let mut feeder = Feeder::start(&ispec)?;
            let mut done = 0;
            let mut judge = Judge::new(checks.flash_on());
            video.stage.seek(spec.start_number as u32);
            video.run(&times, &mut report, &mut judge, |b| {
                done += 1;
                progress(done, n);
                feeder.send(b)
            })?;
            video.judge(judge, &times, &mut report);
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
                    report.encoder = replay(&inter, &s, format, size, n)?;
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
        } else if workers > 1 {
            // Each worker renders contiguous chunks with its own encoder. Accessibility-enabled
            // chunks fit one bounded observation batch, so ordering does not serialize the workers.
            // Every chunk starts on a keyframe; stream copy joins them without re-encoding.
            use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
            let chunks = (workers * 3).min(((end - start) / 10.0).floor() as usize).max(workers).min(n as usize);
            let observe = checks.flash_on() || checks.contrast_on();
            let chunks = if observe { chunks.max((n as usize).div_ceil(OBSERVATION_FRAMES)) } else { chunks };
            let bounds: Vec<usize> = (0..=chunks).map(|i| (i as u64 * n / chunks as u64) as usize).collect();
            let ext = report.path.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_string();
            let parts: Vec<PathBuf> = (0..chunks).map(|i| tmp.join(format!("segment{i:04}.{ext}"))).collect();
            let part_spec = |i: usize| EncodeSpec {
                path: parts[i].clone(),
                audio: None,
                faststart: false,
                metadata: Vec::new(),
                chapters: None,
                ..spec.clone()
            };
            let (audio, keep_alpha) = (video.renderer.audio.clone(), video.keep_alpha);
            // with output segments, the frame time of each entry of `times`
            let all_frames = video.segments.as_ref().map(|(_, f)| f.clone());
            let map = segments.as_ref();
            let cancelled = AtomicBool::new(false);
            let done = AtomicU64::new(0);
            // Create worker devices sequentially and keep them alive until every
            // worker has joined. Concurrent device creation crashed the Vulkan loader;
            // independent devices avoid allocation contention between renderers.
            let worker_gpus: Vec<_> = (0..workers).map(|_| gpu.open_like()).collect::<Result<_, _>>()?;
            // Fixed round-robin chunk ownership lets the coordinator drain each worker in time
            // order. Two queued batches per worker plus one being rendered: at most 384 flash
            // grids (about 11.4 MiB) per worker, independent of output duration.
            let mut judge = Judge::new(checks.flash_on());
            let (senders, receivers): (Vec<_>, Vec<_>) = (0..workers).map(|_| sync_channel::<Seen>(2)).unzip();
            let results: Vec<Result<(Report, String), DeliverError>> = std::thread::scope(|sc| {
                let handles: Vec<_> = worker_gpus
                    .iter()
                    .zip(senders)
                    .enumerate()
                    .map(|(worker_index, (gpu, tx))| {
                        let (
                            cancelled,
                            done,
                            bounds,
                            times,
                            part_spec,
                            audio,
                            all_frames,
                            representation,
                            ev,
                            eo,
                            captions,
                        ) = (
                            &cancelled,
                            &done,
                            &bounds,
                            &times,
                            &part_spec,
                            &audio,
                            &all_frames,
                            &representation,
                            &ev,
                            &eo,
                            &captions,
                        );
                        sc.spawn(move || -> Result<(Report, String), DeliverError> {
                            let gpu = gpu.clone();
                            let mut renderer = Renderer::new(gpu.clone(), p);
                            renderer.quality = opts.quality;
                            renderer.representation = representation.clone();
                            renderer.burn_captions = output.burn_captions.clone();
                            renderer.audio = audio.clone();
                            renderer.captions_off = map.is_some();
                            let stage = OutputStage::new(gpu.device.clone(), gpu.queue.clone());
                            let overlay = crate::overlay::Overlay::new(
                                doc,
                                output,
                                timeline,
                                size,
                                captions,
                                &gpu,
                                eo,
                                representation.as_deref(),
                            )?;
                            let mut worker = Video {
                                ev,
                                renderer,
                                stage,
                                color,
                                format,
                                size,
                                keep_alpha,
                                fps,
                                reframe: p.reframe,
                                segments: None,
                                overlay,
                                origin: if map.is_some() { 0.0 } else { output.start },
                                join_tex: None,
                            };
                            let (mut part, mut encoder) = (Report::default(), String::new());
                            let mut seen = Streaming::new(tx, observe);
                            for i in (worker_index..chunks).step_by(workers) {
                                if cancelled.load(Relaxed) {
                                    return Ok((part, encoder));
                                }
                                let chunk = (|| {
                                    let mut feeder = Feeder::start(&part_spec(i))?;
                                    worker.stage.seek((spec.start_number + bounds[i] as u64) as u32);
                                    worker.segments = map
                                        .zip(all_frames.as_ref())
                                        .map(|(tm, f)| (tm, f[bounds[i]..bounds[i + 1]].to_vec()));
                                    worker.run(&times[bounds[i]..bounds[i + 1]], &mut part, &mut seen, |b| {
                                        done.fetch_add(1, Relaxed);
                                        feeder.send(b)
                                    })?;
                                    seen.flush().map_err(DeliverError::Invalid)?;
                                    feeder.finish()
                                })();
                                match chunk {
                                    Ok(e) => encoder = e,
                                    Err(e) => {
                                        // stop the other workers at their next chunk
                                        cancelled.store(true, Relaxed);
                                        return Err(e);
                                    }
                                }
                            }
                            Ok((part, encoder))
                        })
                    })
                    .collect();
                'chunks: for i in 0..chunks {
                    for _ in 0..if observe { (bounds[i + 1] - bounds[i]).div_ceil(OBSERVATION_FRAMES) } else { 1 } {
                        loop {
                            match receivers[i % workers].recv_timeout(std::time::Duration::from_millis(200)) {
                                Ok(seen) => {
                                    judge.replay(seen, bounds[i]);
                                    break;
                                }
                                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => progress(done.load(Relaxed), n),
                                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                    cancelled.store(true, Relaxed);
                                    break 'chunks;
                                }
                            }
                        }
                    }
                }
                // On an error or panic, unblock every producer before joining the scoped threads.
                drop(receivers);
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
            // every chunk was rendered, so every chunk has been judged; text the workers could not
            // measure in place is measured by this thread's renderer
            video.judge(judge, &times, &mut report);
            progress(n, n);
            spec.join(&parts, &tmp.join("segments.txt"))?;
            report.frames = n;
            report.passes = 1;
            report.segments = workers as u32;
        } else {
            let mut feeder = Feeder::start(&spec)?;
            let mut done = 0;
            let mut judge = Judge::new(checks.flash_on());
            video.stage.seek(spec.start_number as u32);
            video.run(&times, &mut report, &mut judge, |b| {
                done += 1;
                progress(done, n);
                feeder.send(b)
            })?;
            video.judge(judge, &times, &mut report);
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
            let srgb = OutputColor::new(m::ColorSpace::Srgb, m::Transfer::Srgb, true);
            // A plain still's time is composition time; a segmented still's time is
            // output time. Both use the delivered picture's placement and overlay clock.
            let (t, f, output_time) = match &segments {
                Some(tm) => {
                    let t = match &st.marker {
                        Some(mk) => {
                            let c = sr_eval::eval::marker(p, mk)
                                .ok_or_else(|| DeliverError::Invalid(format!("{}: no marker {mk:?}", st.path)))?;
                            tm.output_of(c).ok_or_else(|| {
                                DeliverError::Invalid(format!("{}: marker {mk:?} is in no segment", st.path))
                            })?
                        }
                        None => st.time.clamp(0.0, tm.duration),
                    };
                    let f = tm.at(t);
                    (f.composition, Some(f), t)
                }
                None => {
                    let t = match &st.marker {
                        Some(mk) => sr_eval::eval::marker(p, mk).unwrap_or(st.time),
                        None => st.time,
                    }
                    .clamp(0.0, p.duration);
                    (t, None, t - video.origin)
                }
            };
            let g = ev.evaluate(t);
            let frame = video.render_side(&g, t, f.as_ref())?;
            let mut unsupported = report.unsupported.iter().cloned().collect::<std::collections::BTreeSet<_>>();
            unsupported.extend(frame.stats.unsupported.iter().cloned());
            let (picture, placement) = video.picture(&frame, f.as_ref(), &mut unsupported)?;
            let over = match video.overlay.as_mut() {
                Some(o) => {
                    let (tex, problems) = o.draw(&mut video.stage, output_time)?;
                    unsupported.extend(problems);
                    Some(tex)
                }
                None => None,
            };
            let working = video.renderer.working();
            let pending =
                video.stage.submit_placed(&picture, &working, &srgb, InputFormat::Rgba8, size, true, placement, over);
            let (rgba, psize) = (video.stage.wait(pending), size);
            report.unsupported = unsupported.into_iter().collect();
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
    drop(tmp);
    // caption files: the output-time tracks (with segments, the composition's mapped), and without
    // segments the composition's own, cut to the rendered range
    let base_dir = p.base_dirs.first().cloned().unwrap_or_default();
    if segments.is_none() {
        let tracks = p.scene.captions.as_ref().map(|c| c.caption_tracks.as_slice()).unwrap_or_default();
        report.files.extend(write_sidecars(tracks, &base_dir, output, &report.path, start, end)?);
    }
    let origin = if segments.is_some() { 0.0 } else { output.start };
    report.files.extend(write_sidecars(
        &captions.tracks,
        &base_dir,
        output,
        &report.path,
        start - origin,
        end - origin,
    )?);
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
/// render the same from a cold start (no simulation). The accessibility checks compare frames across
/// segments and do not limit the count: the segments' observations are judged together, in frame order.
fn segment_count(output: &m::Output, codec: Codec, opts: &Options, ev: &Evaluator, duration: f64) -> usize {
    if output.two_pass
        || output.max_file_size.is_some()
        || codec.is_sequence()
        || codec.is_audio_only()
        || matches!(codec, Codec::Gif | Codec::Apng | Codec::Webp)
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
        chapters: None,
        hardware: opts.hardware,
        audio_bits: p.scene.audio_mix.as_ref().map(|a| a.bit_depth.as_str().parse().unwrap_or(24)).unwrap_or(24),
    }
}

/// File name of frame `k` of a printf-patterned sequence path.
pub fn sequence_name(pattern: &Path, k: u64) -> String {
    // the pattern is in the file name: a `%` in a directory name is just that
    let s = pattern.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    if let Some(i) = s.find('%') {
        let rest = &s[i + 1..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if rest[digits.len()..].starts_with('d') {
            let width: usize = digits.trim_start_matches('0').parse().unwrap_or(0);
            let name = format!("{}{k:0width$}{}", &s[..i], &rest[digits.len() + 1..]);
            return pattern.with_file_name(name).to_string_lossy().into_owned();
        }
    }
    pattern.to_string_lossy().into_owned()
}

/// Decodes the intermediate and feeds it to an encoder for one pass.
fn replay(
    inter: &Path,
    spec: &EncodeSpec,
    format: InputFormat,
    size: [u32; 2],
    frames: u64,
) -> Result<String, DeliverError> {
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
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| sr_media::MediaError::Spawn { tool: sr_media::ffmpeg(), source: e })?;
    let mut out = dec.stdout.take().expect("piped");
    let mut err = dec.stderr.take().expect("piped");
    let said = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = err.read_to_end(&mut v);
        v
    });
    let mut buf = vec![0u8; format.frame_bytes(size[0], size[1])];
    let fed = Encoder::start(spec).map_err(DeliverError::from).and_then(|mut enc| {
        let mut n = 0u64;
        while out.read_exact(&mut buf).is_ok() {
            enc.write(&buf)?;
            n += 1;
        }
        Ok((enc, n))
    });
    // the decoder is reaped whatever became of the encoder
    if fed.is_err() {
        let _ = dec.kill();
    }
    let status = dec.wait()?;
    let said = sr_media::reason(&said.join().unwrap_or_default());
    let (enc, n) = fed?;
    if !status.success() || n != frames {
        // the encoder is dropped unfinished: a pass fed part of the frames is no output
        return Err(sr_media::MediaError::Failed {
            tool: "ffmpeg".into(),
            path: inter.display().to_string(),
            message: format!(
                "the intermediate decoded to {n} of {frames} frames{}{said}",
                if said.is_empty() { "" } else { ": " }
            ),
        }
        .into());
    }
    let name = enc.encoder.clone();
    enc.finish()?;
    Ok(name)
}

/// Caption sidecars: tracks listed in the output's `captions`, or with mode
/// sidecar or both, written next to the output as `<stem>.<track>.<language>.vtt`
/// (`.srt` when the track's source is SRT), with times relative to the range start.
/// Warnings for segments whose span begins or ends where nothing is drawn above the background.
fn empty_ends(ev: &Evaluator, tm: &crate::segments::TimeMap) -> Vec<String> {
    let fps = ev.program().fps.as_f64();
    let drawn = |c: f64| {
        ev.evaluate(c).nodes.iter().any(|n| {
            n.draw
                && !n.is_matte
                && n.world_opacity > 1e-3
                && !matches!(n.kind, "group" | "camera" | "instance" | "repeat" | "transition" | "null")
        })
    };
    let mut out = Vec::new();
    for (i, seg) in tm.segments.iter().enumerate() {
        let name = seg.elem.id.clone().unwrap_or_else(|| format!("segment {}", i + 1));
        let last = (seg.duration - 1.0 / fps).max(0.0);
        for (edge, u) in [("starts", 0.0), ("ends", last)] {
            let c = tm.composition(i, u);
            if !drawn(c) {
                out.push(format!("{name} {edge} at {c:.3} s, where nothing is drawn above the background"));
            }
        }
    }
    out
}

/// The output's chapters (markers of kind `chapter` at the first output time each maps to; those in
/// skipped spans are dropped), cut to `start`..`end`, as an FFmetadata file in `dir`.
fn write_chapters(
    p: &sr_eval::Program,
    tm: &crate::segments::TimeMap,
    start: f64,
    end: f64,
    dir: &Path,
) -> Result<Option<PathBuf>, DeliverError> {
    let mut marks: Vec<(f64, String)> = p
        .scene
        .markers
        .iter()
        .flat_map(|mk| &mk.children)
        .filter_map(|c| match c {
            m::MarkersChild::Marker(mk) if mk.kind.as_str() == "chapter" => {
                let t = tm.output_of(mk.time)?;
                Some((t, mk.label.clone().or_else(|| mk.id.clone()).unwrap_or_default()))
            }
            _ => None,
        })
        .filter(|(t, _)| *t >= start - 1e-9 && *t < end)
        .collect();
    if marks.is_empty() {
        return Ok(None);
    }
    marks.sort_by(|a, b| a.0.total_cmp(&b.0));
    let ms = |t: f64| ((t - start) * 1000.0).round().max(0.0) as u64;
    let esc = |s: &str| {
        s.chars().fold(String::new(), |mut o, c| {
            if matches!(c, '=' | ';' | '#' | '\\' | '\n') {
                o.push('\\');
            }
            o.push(c);
            o
        })
    };
    let mut text = String::from(";FFMETADATA1\n");
    for (k, (t, title)) in marks.iter().enumerate() {
        let next = marks.get(k + 1).map_or(end, |m| m.0);
        text.push_str(&format!(
            "[CHAPTER]\nTIMEBASE=1/1000\nSTART={}\nEND={}\ntitle={}\n",
            ms(*t),
            ms(next),
            esc(title)
        ));
    }
    let path = dir.join("chapters.txt");
    std::fs::write(&path, text)?;
    Ok(Some(path))
}

/// Writes the caption files of `tracks` next to `out`: tracks the output lists in `captions`, and
/// every track that is not burn-only, cut to `start`..`end` and starting at `start`.
fn write_sidecars(
    tracks: &[m::CaptionTrack],
    base: &Path,
    output: &m::Output,
    out: &std::path::Path,
    start: f64,
    end: f64,
) -> Result<Vec<PathBuf>, DeliverError> {
    let mut files = Vec::new();
    for tr in tracks {
        let listed = output.captions.as_ref().is_some_and(|ids| ids.contains(&tr.id));
        let mode = tr.mode.to_string();
        if !listed && mode == "burn" {
            continue;
        }
        let cues = sr_gpu::text::track_cues(tr, base).map_err(DeliverError::Invalid)?;
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

/// Most with/without renders spent measuring one text's contrast.
const CONTRAST_PROBES: usize = 32;

/// The frames to measure a text's contrast at, of the (frame, opacity) it was seen in: those where it is at
/// its most visible, all of them up to [`CONTRAST_PROBES`], else that many spread evenly from the first to
/// the last, so a title held for minutes does not render every one of its frames twice more.
fn probe_frames(seen: &[(usize, f64)]) -> Vec<(usize, f64)> {
    let top = seen.iter().map(|s| s.1).fold(f64::MIN, f64::max);
    let at_top: Vec<(usize, f64)> = seen.iter().filter(|s| s.1 >= top - 1e-3).copied().collect();
    if at_top.len() <= CONTRAST_PROBES {
        return at_top;
    }
    (0..CONTRAST_PROBES).map(|i| at_top[i * (at_top.len() - 1) / (CONTRAST_PROBES - 1)]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_title_is_probed_a_bounded_number_of_times() {
        // a title fading in over a second, then held for five minutes at 30 fps
        let seen: Vec<(usize, f64)> = (0..9030).map(|k| (k, (k as f64 / 30.0).min(1.0))).collect();
        let frames = probe_frames(&seen);
        assert!(frames.len() <= CONTRAST_PROBES, "{} probe renders", frames.len());
        // both ends of the hold, and spread over it
        assert_eq!((frames[0], frames[frames.len() - 1]), ((30, 1.0), (9029, 1.0)));
        assert!(frames.windows(2).all(|w| w[1].0 > w[0].0 && w[1].0 - w[0].0 <= 9000 / (CONTRAST_PROBES - 1) + 1));
        // a short one is probed at every frame
        assert_eq!(probe_frames(&seen[..40]), seen[30..40].to_vec());
        assert!(probe_frames(&[]).is_empty());
    }

    #[test]
    fn only_the_file_name_of_a_sequence_is_a_pattern() {
        assert_eq!(sequence_name(Path::new("out/f_%04d.png"), 7), "out/f_0007.png");
        assert_eq!(sequence_name(Path::new("renders/50%/f_%04d.png"), 7), "renders/50%/f_0007.png");
        assert_eq!(sequence_name(Path::new("renders/%d/f_%d.png"), 12), "renders/%d/f_12.png");
        assert_eq!(sequence_name(Path::new("out/plain.png"), 7), "out/plain.png");
    }

    /// A 5-frame FFV1 intermediate, as the two-pass path writes it.
    fn intermediate(dir: &Path) -> Option<(EncodeSpec, PathBuf)> {
        std::process::Command::new(sr_media::ffmpeg()).arg("-version").output().ok()?;
        std::fs::create_dir_all(dir).ok()?;
        let adhoc = adhoc_output("x.mkv", "ffv1").ok()?;
        let doc = sr_model::load_str(
            r#"<scene version="1.1"><project width="64" height="36" fps="25" duration="1"/><composition/></scene>"#,
            &sr_model::LoadOptions::without_assets(),
        )
        .ok()?;
        let ev = Evaluator::new(&doc, &EvalOptions::default()).ok()?;
        let color = OutputColor::new(m::ColorSpace::Srgb, m::Transfer::Auto, false);
        let opts = Options { hardware: Hardware::Software, ..Default::default() };
        let out = dir.join("out.mkv");
        let spec =
            base_spec(&adhoc, &out, Codec::Ffv1, [64, 36], 25.0, InputFormat::Rgba8, &color, None, ev.program(), &opts);
        let inter = dir.join("intermediate.mkv");
        let mut enc =
            Encoder::start(&EncodeSpec { path: inter.clone(), pixel_format: "bgra".into(), ..spec.clone() }).ok()?;
        for k in 0..5u8 {
            enc.write(&vec![k * 40; InputFormat::Rgba8.frame_bytes(64, 36)]).ok()?;
        }
        enc.finish().ok()?;
        Some((spec, inter))
    }

    #[test]
    fn a_partial_or_failed_intermediate_decode_fails_the_pass() {
        let dir = std::env::temp_dir().join(format!("sr-deliver-replay-{}", std::process::id()));
        let Some((spec, inter)) = intermediate(&dir) else { return };
        assert!(replay(&inter, &spec, InputFormat::Rgba8, [64, 36], 5).is_ok());
        let successful_output = std::fs::read(&spec.path).unwrap();
        // cut in the middle of its frames
        let bytes = std::fs::read(&inter).unwrap();
        std::fs::write(&inter, &bytes[..bytes.len() * 6 / 10]).unwrap();
        let cut = replay(&inter, &spec, InputFormat::Rgba8, [64, 36], 5);
        assert!(cut.is_err(), "a truncated intermediate was encoded as if whole");
        assert_eq!(std::fs::read(&spec.path).unwrap(), successful_output, "a failed pass replaced the previous output");
        std::fs::write(&inter, b"not a video").unwrap();
        let e = replay(&inter, &spec, InputFormat::Rgba8, [64, 36], 5).unwrap_err().to_string();
        assert!(e.contains("intermediate.mkv") && e.contains("Invalid data"), "{e}");
        assert_eq!(std::fs::read(&spec.path).unwrap(), successful_output);
    }
}

/// Live stage times of the frames rendered so far in this process, readable while an encode runs, so a progress
/// line can say where the time goes (a slow encode must not be a mystery until it ends).
pub mod meter {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    static STAGE_NS: [AtomicU64; 4] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

    /// Adds `secs` to stage `i` of `report` and of the live meter (0 evaluate, 1 render, 2 GPU and readback wait,
    /// 3 encoder blocked).
    pub(crate) fn add(report: &mut [f64; 4], i: usize, secs: f64) {
        report[i] += secs;
        STAGE_NS[i].fetch_add((secs.max(0.0) * 1e9) as u64, Relaxed);
    }

    /// Seconds spent in each stage so far, summed over every render worker of this process.
    pub fn snapshot() -> [f64; 4] {
        std::array::from_fn(|i| STAGE_NS[i].load(Relaxed) as f64 * 1e-9)
    }
}
