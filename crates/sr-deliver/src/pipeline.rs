//! One `<output>` from document to delivered files.
//!
//! Stages overlap: while the encoder thread writes frame N, the GPU renders
//! and converts frame N+1 and the readback of frame N maps; decoders run in
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
    /// Seconds spent evaluating, rendering (CPU planning and submission), waiting for converted frames, and blocked on the encoder.
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
        let flash_mode = acc.as_ref().map(|a| a.flash_check.to_string()).unwrap_or_else(|| "warn".into());
        let contrast_mode = acc.as_ref().map(|a| a.contrast_check.to_string()).unwrap_or_else(|| "off".into());
        let min_contrast = acc.as_ref().map(|a| a.min_contrast.get()).unwrap_or(4.5);
        let mut flash = (flash_mode != "off").then(crate::access::FlashDetector::default);
        self.renderer.contrast_probe = contrast_mode != "off";
        let mut lowest: std::collections::BTreeMap<String, (f64, f64)> = Default::default();
        for (k, &t) in times.iter().enumerate() {
            let t0 = Instant::now();
            let g = self.ev.evaluate(t);
            let t1 = Instant::now();
            let ev = &self.ev;
            let mut sub = |st: f64| ev.evaluate(st);
            let frame = self.renderer.render_with(&g, p, Some(&mut sub));
            if let Some(e) = frame.stats.errors.first() {
                return Err(DeliverError::Render { time: t, message: e.clone() });
            }
            unsupported.extend(frame.stats.unsupported.iter().cloned());
            if let Some(det) = flash.as_mut() {
                let cells = self.renderer.flash_grid(&frame.texture);
                det.push(t, &cells);
            }
            for (id, ratio) in &frame.stats.contrast {
                let e = lowest.entry(id.clone()).or_insert((f64::MAX, t));
                if *ratio < e.0 {
                    *e = (*ratio, t);
                }
            }
            report.decode_wait_seconds += frame.stats.decode_wait;
            report.vector_seconds += frame.stats.vector_seconds;
            let next =
                self.stage.submit(&frame.texture, &working, &self.color, self.format, self.size, self.keep_alpha);
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
        report.unsupported = unsupported.into_iter().collect();
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
        for (id, (ratio, at)) in &lowest {
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
    let start = opts.start.unwrap_or(output.start).max(0.0);
    let end = opts.end.or(output.end).unwrap_or(p0.duration).min(p0.duration);
    if end <= start {
        return Err(DeliverError::Invalid(format!("empty range {start}..{end}")));
    }
    let mut report = Report { path: resolve(&base, &output.path), fps, ..Default::default() };
    // audio first: the mix feeds the analysis table
    let t_audio = Instant::now();
    let scene_audio: Option<SceneAudio> = audio::mix_scene(&ev0, fps, representation.as_deref())?;
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
    let wants_audio = output.audio && scene_audio.is_some() && (codec.takes_audio() || codec.is_audio_only());
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
        let frame = p
            .scene
            .scene360
            .as_ref()
            .map(|s| [s.width as u32, s.height as u32])
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
        let mut video = Video {
            ev: &ev,
            renderer,
            stage: OutputStage::new(gpu.device.clone(), gpu.queue.clone()),
            color,
            format,
            size,
            keep_alpha: output.alpha || rgb_image,
        };
        let n = ((end - start) * fps).round().max(1.0) as u64;
        let times: Vec<f64> = (0..n).map(|k| start + k as f64 / fps).collect();
        let audio_in = audio_file.clone().map(|f| (f, output.audio_codec.clone(), output.audio_bitrate));
        let mut spec = base_spec(output, &report.path, codec, size, fps, format, &color, audio_in, p, opts);
        spec.start_number = (start * fps).round() as u64;
        let fit = output.max_file_size;
        let t_video = Instant::now();
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
        } else {
            let mut feeder = Feeder::start(&spec)?;
            let mut done = 0;
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
            let t = st.marker.as_deref().and_then(|mk| sr_eval::eval::marker(p, mk)).unwrap_or(st.time);
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
    report.files.extend(write_sidecars(p, output, &report.path, start, end)?);
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
