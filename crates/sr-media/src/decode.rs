//! Frame-accurate video decode with look-ahead, and audio decode.

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::Arc;

use crate::probe::{probe, VideoInfo};
use crate::MediaError;

/// How samples of a decoded frame are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum PixelLayout {
    /// Planar Y, Cb, Cr at 8 bits with chroma shifts (x, y).
    Yuv8(u32, u32),
    /// Planar Y, Cb, Cr at 16 bits (little endian, full 16-bit scale) with chroma shifts.
    Yuv16(u32, u32),
    /// Packed RGBA, 8 bits, straight alpha.
    Rgba8,
    /// Packed RGBA, 16 bits little endian, straight alpha.
    Rgba16,
}

impl PixelLayout {
    /// The layout that carries a stream without loss of depth or alpha.
    pub fn for_stream(v: &VideoInfo) -> PixelLayout {
        match (v.rgb || v.alpha, v.depth > 8) {
            (true, false) => PixelLayout::Rgba8,
            (true, true) => PixelLayout::Rgba16,
            (false, deep) => {
                let s = match v.chroma_shift {
                    (1, 1) | (2, _) => (1, 1),
                    (1, 0) => (1, 0),
                    _ => (0, 0),
                };
                if deep {
                    PixelLayout::Yuv16(s.0, s.1)
                } else {
                    PixelLayout::Yuv8(s.0, s.1)
                }
            }
        }
    }

    /// FFmpeg pixel format name.
    pub fn ffmpeg_name(&self) -> &'static str {
        match self {
            PixelLayout::Yuv8(1, 1) => "yuv420p",
            PixelLayout::Yuv8(1, 0) => "yuv422p",
            PixelLayout::Yuv8(..) => "yuv444p",
            PixelLayout::Yuv16(1, 1) => "yuv420p16le",
            PixelLayout::Yuv16(1, 0) => "yuv422p16le",
            PixelLayout::Yuv16(..) => "yuv444p16le",
            PixelLayout::Rgba8 => "rgba",
            PixelLayout::Rgba16 => "rgba64le",
        }
    }

    /// Plane sizes (width, height, bytes per texel) for a frame of `w`×`h`.
    pub fn planes(&self, w: u32, h: u32) -> Vec<(u32, u32, u32)> {
        let chroma = |sx: u32, sy: u32| ((w + (1 << sx) - 1) >> sx, (h + (1 << sy) - 1) >> sy);
        match *self {
            PixelLayout::Yuv8(sx, sy) => {
                let (cw, ch) = chroma(sx, sy);
                vec![(w, h, 1), (cw, ch, 1), (cw, ch, 1)]
            }
            PixelLayout::Yuv16(sx, sy) => {
                let (cw, ch) = chroma(sx, sy);
                vec![(w, h, 2), (cw, ch, 2), (cw, ch, 2)]
            }
            PixelLayout::Rgba8 => vec![(w, h, 4)],
            PixelLayout::Rgba16 => vec![(w, h, 8)],
        }
    }

    /// Bytes per frame.
    pub fn frame_bytes(&self, w: u32, h: u32) -> usize {
        self.planes(w, h).iter().map(|(a, b, c)| (a * b * c) as usize).sum()
    }
}

/// One plane of a frame.
#[derive(Debug, Clone)]
pub struct Plane {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Tightly packed rows.
    pub data: Vec<u8>,
}

/// A decoded frame.
#[derive(Debug, Clone)]
pub struct VideoFrame {
    /// Frame index at the decoder's frame rate.
    pub index: i64,
    /// Coded width.
    pub width: u32,
    /// Coded height.
    pub height: u32,
    /// Sample layout.
    pub layout: PixelLayout,
    /// Planes in layout order.
    pub planes: Vec<Plane>,
}

struct Reader {
    child: Child,
    rx: Receiver<Arc<VideoFrame>>,
    errors: Option<std::thread::JoinHandle<Vec<u8>>>,
    /// The frame decoding started at.
    first: i64,
    next: i64,
}

impl Reader {
    /// Once the stream has ended: what went wrong, when the decoder failed or complained.
    fn failure(&mut self) -> Option<String> {
        let status = self.child.wait().ok();
        let said = crate::tail(&self.errors.take().and_then(|h| h.join().ok()).unwrap_or_default(), 3);
        match status {
            Some(s) if !s.success() && said.is_empty() => Some(format!("ffmpeg ended with {s}")),
            _ => Some(said).filter(|s| !s.is_empty()),
        }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Decodes one video file by frame index at a fixed rate.
///
/// Frame `n` is the source image displayed at time `n / fps`. A background
/// thread keeps up to `lookahead` frames ahead of the last request; a
/// request behind the ring or far ahead of it restarts decoding with an
/// accurate seek.
pub struct VideoDecoder {
    path: PathBuf,
    /// Stream properties.
    pub info: VideoInfo,
    /// Frame rate frames are indexed at.
    pub fps: f64,
    /// Sample layout of delivered frames.
    pub layout: PixelLayout,
    lookahead: usize,
    reader: Option<Reader>,
    ring: VecDeque<Arc<VideoFrame>>,
    end: Option<i64>,
    /// Container duration in seconds (0 when unknown).
    duration: f64,
    /// Decoder restarts (seeks) so far.
    pub seeks: usize,
    /// Decoding that stopped or complained before the end of the stream (damaged or truncated media): later
    /// frames hold the last good one. Each is also printed once on standard error.
    pub warnings: Vec<String>,
}

impl VideoDecoder {
    /// Opens `path`, indexing frames at `fps` (the stream's own rate when `None`).
    pub fn open(path: &Path, fps: Option<f64>, lookahead: usize) -> Result<VideoDecoder, MediaError> {
        let info = probe(path)?;
        let duration = info.duration;
        let v = info.video.ok_or_else(|| MediaError::NoStream(path.display().to_string(), "video"))?;
        let fps = fps.filter(|f| *f > 0.0).unwrap_or(if v.fps > 0.0 { v.fps } else { 25.0 });
        Ok(VideoDecoder {
            path: path.to_path_buf(),
            layout: PixelLayout::for_stream(&v),
            info: v,
            fps,
            lookahead: lookahead.max(1),
            reader: None,
            ring: VecDeque::new(),
            end: None,
            duration,
            seeks: 0,
            warnings: Vec::new(),
        })
    }

    fn start(&mut self, index: i64) -> Result<(), MediaError> {
        self.reader = None;
        self.seeks += 1;
        let (w, h) = (self.info.width, self.info.height);
        let mut cmd = Command::new(crate::ffmpeg());
        cmd.args(["-nostdin", "-v", "error"]);
        if index > 0 {
            cmd.args(["-ss", &format!("{:.6}", index as f64 / self.fps)]);
        }
        cmd.arg("-noautorotate").arg("-i").arg(&self.path);
        cmd.args(["-map", "0:v:0", "-an", "-sn", "-dn", "-fps_mode", "cfr", "-r", &format!("{}", self.fps)]);
        cmd.args(["-f", "rawvideo", "-pix_fmt", self.layout.ffmpeg_name(), "pipe:1"]);
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| MediaError::Spawn { tool: crate::ffmpeg(), source: e })?;
        let mut out = child.stdout.take().expect("piped");
        let mut err = child.stderr.take().expect("piped");
        // drained so the decoder never blocks on it; its last lines say why decoding stopped
        let errors = std::thread::Builder::new()
            .name("sr-video-errors".into())
            .spawn(move || {
                let (mut kept, mut buf) = (Vec::new(), [0u8; 4096]);
                while let Ok(n @ 1..) = err.read(&mut buf) {
                    kept.extend_from_slice(&buf[..n]);
                    if kept.len() > 16384 {
                        kept.drain(..kept.len() - 8192);
                    }
                }
                kept
            })
            .map_err(MediaError::Io)?;
        let (tx, rx) = sync_channel(self.lookahead);
        let layout = self.layout;
        let sizes = layout.planes(w, h);
        std::thread::Builder::new()
            .name("sr-video-decode".into())
            .spawn(move || {
                let mut k = index;
                loop {
                    let mut planes = Vec::with_capacity(sizes.len());
                    for &(pw, ph, bpp) in &sizes {
                        let mut data = vec![0u8; (pw * ph * bpp) as usize];
                        if out.read_exact(&mut data).is_err() {
                            return;
                        }
                        planes.push(Plane { width: pw, height: ph, data });
                    }
                    if tx.send(Arc::new(VideoFrame { index: k, width: w, height: h, layout, planes })).is_err() {
                        return;
                    }
                    k += 1;
                }
            })
            .map_err(MediaError::Io)?;
        self.reader = Some(Reader { child, rx, errors: Some(errors), first: index, next: index });
        Ok(())
    }

    /// Whether a decoder started at `index` delivers a frame.
    fn has_frame(&mut self, index: i64) -> Result<bool, MediaError> {
        self.ring.clear();
        self.start(index)?;
        let got = self.reader.as_ref().expect("started").rx.recv().is_ok();
        self.reader = None;
        Ok(got)
    }

    /// The last frame, after a seek to `past` found none: the end is bracketed with a few seeks (around the
    /// container's duration, then by halves) and read up to, rather than stepped back to a frame at a time.
    fn last_frame(&mut self, past: i64) -> Result<Arc<VideoFrame>, MediaError> {
        let (mut lo, mut hi) = (0, past);
        let guess = (self.duration * self.fps).ceil() as i64;
        let mut tries = vec![guess - 12, guess + 4];
        while hi - lo > 16 {
            let mid = tries.pop().filter(|m| lo < *m && *m < hi).unwrap_or(lo + (hi - lo) / 2);
            if self.has_frame(mid)? {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        self.frame(lo)?;
        let f = self.frame(hi - 1)?;
        self.end.get_or_insert(hi);
        Ok(f)
    }

    /// Frame `index`, or the last frame when `index` is past the end.
    pub fn frame(&mut self, index: i64) -> Result<Arc<VideoFrame>, MediaError> {
        let index = index.max(0);
        let index = match self.end {
            Some(e) if index >= e => e - 1,
            _ => index,
        };
        if let Some(f) = self.ring.iter().find(|f| f.index == index) {
            return Ok(f.clone());
        }
        let restart = match &self.reader {
            None => true,
            Some(r) => index < r.next || index > r.next + (self.lookahead as i64).max(8) * 2,
        };
        if restart {
            self.ring.clear();
            self.start(index)?;
        }
        loop {
            let r = self.reader.as_mut().expect("started");
            match r.rx.recv() {
                Ok(f) => {
                    r.next = f.index + 1;
                    self.ring.push_back(f.clone());
                    while self.ring.len() > 4 {
                        self.ring.pop_front();
                    }
                    if f.index == index {
                        return Ok(f);
                    }
                }
                Err(_) => {
                    // end of stream: remember it and hold the last frame
                    let (first, next, why) = (r.first, r.next, r.failure());
                    self.reader = None;
                    if next == first {
                        if first > 0 {
                            return self.last_frame(first);
                        }
                        return Err(MediaError::Failed {
                            tool: "ffmpeg".into(),
                            path: self.path.display().to_string(),
                            message: why.unwrap_or_else(|| "no frames decoded".into()),
                        });
                    }
                    if let Some(why) = why {
                        let w = format!("{}: decoding stopped after frame {}: {why}", self.path.display(), next - 1);
                        if !self.warnings.contains(&w) {
                            eprintln!("warning: {w}");
                            self.warnings.push(w);
                        }
                    }
                    self.end = Some(next);
                    return Ok(self.ring.back().expect("frames were read").clone());
                }
            }
        }
    }
}

/// Decoded audio.
#[derive(Debug, Clone, Default)]
pub struct AudioData {
    /// Sample rate.
    pub sample_rate: u32,
    /// Channels.
    pub channels: u16,
    /// Channel layout name from the source.
    pub layout: String,
    /// Interleaved samples.
    pub samples: Vec<f32>,
}

impl AudioData {
    /// Frames (samples per channel).
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }
}

/// Decodes audio stream `stream` of `path` to interleaved `f32` at `rate`.
pub fn decode_audio(path: &Path, stream: usize, rate: u32) -> Result<AudioData, MediaError> {
    let info = probe(path)?;
    let a = info.audio.get(stream).cloned().ok_or_else(|| MediaError::NoStream(path.display().to_string(), "audio"))?;
    let out = Command::new(crate::ffmpeg())
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-map", &format!("0:a:{stream}"), "-vn", "-f", "f32le", "-ar", &rate.to_string(), "pipe:1"])
        .output()
        .map_err(|e| MediaError::Spawn { tool: crate::ffmpeg(), source: e })?;
    if !out.status.success() {
        return Err(MediaError::Failed {
            tool: "ffmpeg".into(),
            path: path.display().to_string(),
            message: crate::tail(&out.stderr, 3),
        });
    }
    let samples = out.stdout.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    Ok(AudioData { sample_rate: rate, channels: a.channels.max(1), layout: a.layout, samples })
}
