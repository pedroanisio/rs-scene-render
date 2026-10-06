//! Encoder command lines for every codec and container of the schema, with
//! hardware encoder selection, HDR metadata, two-pass and still images.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Mutex, OnceLock};

use crate::MediaError;

/// Output codecs (`output/@codec`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum Codec {
    H264,
    H265,
    Ffv1,
    Av1,
    Vp9,
    Prores,
    Dnxhr,
    Gif,
    Apng,
    Webp,
    PngSequence,
    JpegSequence,
    ExrSequence,
    TiffSequence,
    AudioOnly,
}

impl Codec {
    /// Parses the schema name.
    pub fn parse(s: &str) -> Option<Codec> {
        Some(match s {
            "h264" => Codec::H264,
            "h265" => Codec::H265,
            "ffv1" => Codec::Ffv1,
            "av1" => Codec::Av1,
            "vp9" => Codec::Vp9,
            "prores" => Codec::Prores,
            "dnxhr" => Codec::Dnxhr,
            "gif" => Codec::Gif,
            "apng" => Codec::Apng,
            "webp" => Codec::Webp,
            "png-sequence" => Codec::PngSequence,
            "jpeg-sequence" => Codec::JpegSequence,
            "exr-sequence" => Codec::ExrSequence,
            "tiff-sequence" => Codec::TiffSequence,
            "audio-only" => Codec::AudioOnly,
            _ => return None,
        })
    }

    /// Writes numbered image files.
    pub fn is_sequence(&self) -> bool {
        matches!(self, Codec::PngSequence | Codec::JpegSequence | Codec::ExrSequence | Codec::TiffSequence)
    }

    /// Carries no video.
    pub fn is_audio_only(&self) -> bool {
        *self == Codec::AudioOnly
    }

    /// Can carry an audio stream.
    pub fn takes_audio(&self) -> bool {
        !self.is_sequence() && !matches!(self, Codec::Gif | Codec::Apng | Codec::Webp)
    }
}

/// Containers (`output/@container`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum Container {
    Mp4,
    Mov,
    Mkv,
    Webm,
    Mxf,
    Wav,
    M4a,
    Mp3,
}

impl Container {
    /// Parses the schema name.
    pub fn parse(s: &str) -> Option<Container> {
        Some(match s {
            "mp4" => Container::Mp4,
            "mov" => Container::Mov,
            "mkv" => Container::Mkv,
            "webm" => Container::Webm,
            "mxf" => Container::Mxf,
            "wav" => Container::Wav,
            "m4a" => Container::M4a,
            "mp3" => Container::Mp3,
            _ => return None,
        })
    }

    /// From a file extension.
    pub fn from_path(p: &Path) -> Option<Container> {
        Container::parse(&p.extension()?.to_str()?.to_ascii_lowercase())
    }

    fn muxer(&self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mov => "mov",
            Container::Mkv => "matroska",
            Container::Webm => "webm",
            Container::Mxf => "mxf",
            Container::Wav => "wav",
            Container::M4a => "ipod",
            Container::Mp3 => "mp3",
        }
    }

    /// The default container of a codec.
    pub fn default_for(c: Codec) -> Option<Container> {
        match c {
            Codec::H264 | Codec::H265 | Codec::Av1 => Some(Container::Mp4),
            Codec::Vp9 => Some(Container::Webm),
            Codec::Prores => Some(Container::Mov),
            Codec::Dnxhr => Some(Container::Mxf),
            Codec::Ffv1 => Some(Container::Mkv),
            Codec::AudioOnly => Some(Container::Wav),
            _ => None,
        }
    }

    fn accepts(&self, c: Codec) -> bool {
        use Codec::*;
        match self {
            Container::Mp4 => matches!(c, H264 | H265 | Av1 | Vp9),
            Container::Mov => matches!(c, H264 | H265 | Prores | Dnxhr | Av1),
            Container::Mkv => matches!(c, H264 | H265 | Ffv1 | Av1 | Vp9 | Prores | Dnxhr),
            Container::Webm => matches!(c, Vp9 | Av1),
            Container::Mxf => matches!(c, Dnxhr | Prores | H264),
            Container::Wav | Container::M4a | Container::Mp3 => c == AudioOnly,
        }
    }
}

/// Which encoder family to use for H.264, H.265 and AV1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
pub enum Hardware {
    /// First working hardware encoder, else software.
    Auto,
    /// libx264, libx265, SVT-AV1.
    Software,
    /// NVIDIA NVENC.
    Nvenc,
    /// Apple VideoToolbox.
    VideoToolbox,
    /// Linux VA-API.
    Vaapi,
    /// Intel Quick Sync.
    Qsv,
    /// AMD AMF.
    Amf,
}

impl Hardware {
    /// Parses a CLI name.
    pub fn parse(s: &str) -> Option<Hardware> {
        Some(match s {
            "auto" => Hardware::Auto,
            "software" => Hardware::Software,
            "nvenc" => Hardware::Nvenc,
            "videotoolbox" => Hardware::VideoToolbox,
            "vaapi" => Hardware::Vaapi,
            "qsv" => Hardware::Qsv,
            "amf" => Hardware::Amf,
            _ => return None,
        })
    }

    fn suffix(&self) -> &'static str {
        match self {
            Hardware::Nvenc => "nvenc",
            Hardware::VideoToolbox => "videotoolbox",
            Hardware::Vaapi => "vaapi",
            Hardware::Qsv => "qsv",
            Hardware::Amf => "amf",
            _ => "",
        }
    }
}

/// Raw frames written to the encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum InputFormat {
    /// 8-bit 4:2:0, Y plane then interleaved CbCr.
    Nv12,
    /// 10-bit 4:2:0 in 16-bit words (MSB aligned).
    P010,
    /// 8-bit RGBA, straight alpha, display-encoded.
    Rgba8,
    /// 16-bit RGBA little endian, straight alpha, display-encoded.
    Rgba16,
    /// 32-bit float planar G, B, R, A, scene-linear.
    Gbrapf32,
}

impl InputFormat {
    fn ffmpeg(&self) -> &'static str {
        match self {
            InputFormat::Nv12 => "nv12",
            InputFormat::P010 => "p010le",
            InputFormat::Rgba8 => "rgba",
            InputFormat::Rgba16 => "rgba64le",
            InputFormat::Gbrapf32 => "gbrapf32le",
        }
    }

    /// Bytes per frame.
    pub fn frame_bytes(&self, w: u32, h: u32) -> usize {
        let (w, h) = (w as usize, h as usize);
        match self {
            InputFormat::Nv12 => w * h + 2 * w.div_ceil(2) * h.div_ceil(2),
            InputFormat::P010 => 2 * (w * h + 2 * w.div_ceil(2) * h.div_ceil(2)),
            InputFormat::Rgba8 => 4 * w * h,
            InputFormat::Rgba16 => 8 * w * h,
            InputFormat::Gbrapf32 => 16 * w * h,
        }
    }

    fn is_yuv(&self) -> bool {
        matches!(self, InputFormat::Nv12 | InputFormat::P010)
    }
}

/// Colour tags of the encoded stream (FFmpeg names).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ColorTags {
    /// `color_primaries`.
    pub primaries: String,
    /// `color_trc`.
    pub transfer: String,
    /// `colorspace` (matrix).
    pub matrix: String,
    /// Full range.
    pub full_range: bool,
}

/// HDR static metadata.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct Hdr {
    /// Maximum content light level, cd/m².
    pub max_cll: Option<u32>,
    /// Maximum frame-average light level, cd/m².
    pub max_fall: Option<u32>,
    /// SMPTE ST 2086 mastering display, x265 syntax `G(x,y)B(x,y)R(x,y)WP(x,y)L(max,min)`.
    pub mastering_display: Option<String>,
}

/// Everything an encode needs.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct EncodeSpec {
    pub path: PathBuf,
    pub codec: Codec,
    pub container: Option<Container>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub input: InputFormat,
    pub pixel_format: String,
    pub preset: String,
    pub profile: Option<String>,
    pub level: Option<String>,
    pub prores_profile: Option<String>,
    pub crf: u32,
    pub bitrate: Option<u64>,
    pub max_bitrate: Option<u64>,
    pub buffer_size: Option<u64>,
    /// `Some((pass, log file))` for one pass of a two-pass encode.
    pub pass: Option<(u8, PathBuf)>,
    pub keyframe_interval: f64,
    pub b_frames: Option<u32>,
    pub faststart: bool,
    pub alpha: bool,
    /// Audio file to mux, with codec and bitrate.
    pub audio: Option<(PathBuf, String, u64)>,
    pub loop_count: u32,
    pub start_number: u64,
    pub color: ColorTags,
    pub hdr: Hdr,
    pub metadata: Vec<(String, String)>,
    /// Chapters to embed: an FFmetadata file (containers without chapters ignore it).
    pub chapters: Option<PathBuf>,
    pub hardware: Hardware,
    /// Audio sample format bits for PCM outputs.
    pub audio_bits: u16,
}

/// Encoders the local FFmpeg can actually open, probed once per process.
pub fn working_encoder(name: &str) -> bool {
    static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(v) = cache.lock().unwrap().get(name) {
        return *v;
    }
    let mut cmd = Command::new(crate::ffmpeg());
    cmd.args(["-v", "error", "-nostdin"]);
    if name.ends_with("_vaapi") {
        cmd.args(["-vaapi_device", "/dev/dri/renderD128"]);
    }
    cmd.args(["-f", "lavfi", "-i", "color=c=gray:s=256x256:r=25:d=0.2"]);
    if name.ends_with("_vaapi") {
        cmd.args(["-vf", "format=nv12,hwupload"]);
    }
    cmd.args(["-frames:v", "2", "-c:v", name, "-f", "null", "-"]);
    let ok = cmd.stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    cache.lock().unwrap().insert(name.to_string(), ok);
    ok
}

/// The encoder a codec uses under a hardware preference.
pub fn choose_encoder(codec: Codec, hw: Hardware) -> Result<String, MediaError> {
    let (hw_base, software): (&str, &[&str]) = match codec {
        Codec::H264 => ("h264", &["libx264"]),
        Codec::H265 => ("hevc", &["libx265"]),
        Codec::Av1 => ("av1", &["libsvtav1", "libaom-av1", "librav1e"]),
        Codec::Vp9 => return Ok("libvpx-vp9".into()),
        Codec::Prores => return Ok("prores_ks".into()),
        Codec::Dnxhr => return Ok("dnxhd".into()),
        Codec::Ffv1 => return Ok("ffv1".into()),
        Codec::Gif => return Ok("gif".into()),
        Codec::Apng => return Ok("apng".into()),
        Codec::Webp => return Ok("libwebp_anim".into()),
        Codec::PngSequence => return Ok("png".into()),
        Codec::JpegSequence => return Ok("mjpeg".into()),
        Codec::ExrSequence => return Ok("exr".into()),
        Codec::TiffSequence => return Ok("tiff".into()),
        Codec::AudioOnly => return Ok(String::new()),
    };
    let hw_list: Vec<Hardware> = match hw {
        Hardware::Software => Vec::new(),
        Hardware::Auto => vec![Hardware::Nvenc, Hardware::VideoToolbox, Hardware::Qsv, Hardware::Amf, Hardware::Vaapi],
        h => vec![h],
    };
    for h in &hw_list {
        let name = format!("{hw_base}_{}", h.suffix());
        if working_encoder(&name) {
            return Ok(name);
        }
    }
    if hw != Hardware::Auto && hw != Hardware::Software {
        return Err(MediaError::Invalid(format!(
            "the {hw_base}_{} encoder is not available in this FFmpeg build or on this machine",
            hw.suffix()
        )));
    }
    software
        .iter()
        .find(|s| working_encoder(s))
        .map(|s| s.to_string())
        .ok_or_else(|| MediaError::Invalid(format!("no working encoder for {codec:?} (tried {})", software.join(", "))))
}

fn preset_rank(p: &str) -> usize {
    ["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow", "placebo"]
        .iter()
        .position(|x| *x == p)
        .unwrap_or(5)
}

/// The arguments that follow `-c:a CODEC`: the bitrate of a compressed stream and, for FFmpeg's own AAC
/// encoder, neither perceptual noise substitution nor temporal noise shaping. With them the encoder
/// codes single frames far from the signal, heard as clicks: a mix mastered to a true peak of −2 dB came
/// out 5 dB over it in one channel for a frame. Measured with FFmpeg 8.1 at 192 kb/s on the mixes of
/// eight films, in 20 ms windows where the decoded stream departs from the mix by more than 0.15 of full
/// scale: 4 to 13 windows a film with both tools on (departures up to 1.4), at most 2 with noise
/// substitution off (up to 0.22), none with both off (up to 0.14). The overall error is no higher
/// without them. Noise substitution does the same damage at 256 and 320 kb/s.
fn compressed_audio_args(codec: &str, bitrate: u64) -> Vec<String> {
    if codec.starts_with("pcm") {
        return Vec::new();
    }
    let mut a = vec!["-b:a".to_string(), bitrate.to_string()];
    if codec == "aac" {
        a.extend(["-aac_pns", "0", "-aac_tns", "0"].map(String::from));
    }
    a
}

impl EncodeSpec {
    /// The container in use (explicit, from the extension, or the codec default).
    pub fn container(&self) -> Option<Container> {
        self.container.or_else(|| Container::from_path(&self.path)).or_else(|| Container::default_for(self.codec))
    }

    /// Checks codec, container and alpha combinations.
    pub fn validate(&self) -> Result<(), MediaError> {
        if let Some(c) = self.container() {
            if !self.codec.is_sequence()
                && !matches!(self.codec, Codec::Gif | Codec::Apng | Codec::Webp)
                && !c.accepts(self.codec)
            {
                return Err(MediaError::Invalid(format!("container {c:?} cannot hold {:?}", self.codec)));
            }
        }
        if self.alpha
            && matches!(self.codec, Codec::H264 | Codec::H265 | Codec::Av1 | Codec::Dnxhr | Codec::JpegSequence)
        {
            return Err(MediaError::Invalid(format!(
                "{:?} cannot carry alpha; use prores (4444), vp9, ffv1, png, apng, webp, exr or tiff",
                self.codec
            )));
        }
        if self.codec == Codec::Dnxhr && (self.width < 256 || self.height < 120) {
            return Err(MediaError::Invalid(format!(
                "DNxHR needs frames of at least 256×120, not {}×{}",
                self.width, self.height
            )));
        }
        if self.codec.is_sequence() && !self.path.file_name().is_some_and(|n| n.to_string_lossy().contains('%')) {
            return Err(MediaError::Invalid(format!(
                "sequence output {} needs a printf pattern such as frame_%05d.png",
                self.path.display()
            )));
        }
        Ok(())
    }

    /// The encoder pixel format.
    fn pix_fmt(&self) -> String {
        let deep =
            self.pixel_format.contains("10") || self.pixel_format.contains("12") || self.pixel_format.contains("16");
        match self.codec {
            Codec::Prores => {
                let p = self.prores_profile.as_deref().unwrap_or("hq");
                if self.alpha || p.starts_with("4444") {
                    if self.alpha { "yuva444p10le" } else { "yuv444p10le" }.into()
                } else {
                    "yuv422p10le".into()
                }
            }
            Codec::Dnxhr => if deep { "yuv422p10le" } else { "yuv422p" }.into(),
            Codec::Vp9 if self.alpha => "yuva420p".into(),
            Codec::Ffv1 if self.alpha => "yuva444p10le".into(),
            Codec::Gif => "pal8".into(),
            Codec::Apng | Codec::PngSequence => match (self.alpha, deep) {
                (true, true) => "rgba64be",
                (true, false) => "rgba",
                (false, true) => "rgb48be",
                (false, false) => "rgb24",
            }
            .into(),
            Codec::Webp => if self.alpha { "yuva420p" } else { "yuv420p" }.into(),
            Codec::JpegSequence => "yuvj444p".into(),
            Codec::ExrSequence => if self.alpha { "gbrapf32le" } else { "gbrpf32le" }.into(),
            Codec::TiffSequence => if self.alpha { "rgba64le" } else { "rgb48le" }.into(),
            _ => self.pixel_format.clone(),
        }
    }

    /// FFmpeg arguments and the encoder name.
    pub fn args(&self) -> Result<(Vec<String>, String), MediaError> {
        self.validate()?;
        let mut a: Vec<String> =
            vec!["-hide_banner".into(), "-nostdin".into(), "-y".into(), "-v".into(), "error".into()];
        let s = |a: &mut Vec<String>, xs: &[&str]| a.extend(xs.iter().map(|x| x.to_string()));
        let mut encoder = choose_encoder(self.codec, self.hardware)?;
        // SVT-AV1 needs frames of at least 64×64
        if encoder == "libsvtav1" && (self.width < 64 || self.height < 64) && working_encoder("libaom-av1") {
            encoder = "libaom-av1".into();
        }
        let vaapi = encoder.ends_with("_vaapi");
        if vaapi {
            s(&mut a, &["-vaapi_device", "/dev/dri/renderD128"]);
        }
        if !self.codec.is_audio_only() {
            s(
                &mut a,
                &[
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    self.input.ffmpeg(),
                    "-s",
                    &format!("{}x{}", self.width, self.height),
                    "-framerate",
                    &format!("{}", self.fps),
                    "-i",
                    "pipe:0",
                ],
            );
        }
        let audio =
            self.audio.as_ref().filter(|_| self.codec.takes_audio() && self.pass.as_ref().is_none_or(|p| p.0 == 2));
        if let Some((p, _, _)) = audio {
            a.push("-i".into());
            a.push(p.display().to_string());
        }
        let container = self.container();
        let index = usize::from(!self.codec.is_audio_only()) + usize::from(audio.is_some());
        let map_chapters = self.chapter_input(&mut a, container, index);
        if self.codec.is_audio_only() {
            let (codec, bits) = match container {
                Some(Container::Mp3) => ("libmp3lame".to_string(), 0),
                Some(Container::M4a) => (self.audio.as_ref().map(|x| x.1.clone()).unwrap_or_else(|| "aac".into()), 0),
                _ => (
                    match self.audio_bits {
                        16 => "pcm_s16le",
                        32 => "pcm_f32le",
                        _ => "pcm_s24le",
                    }
                    .to_string(),
                    self.audio_bits,
                ),
            };
            let codec = if container == Some(Container::M4a) && codec == "aac" {
                codec
            } else if bits == 0 && container != Some(Container::M4a) {
                "libmp3lame".into()
            } else {
                codec
            };
            s(&mut a, &["-map", "0:a:0", "-c:a", &codec]);
            a.extend(compressed_audio_args(&codec, self.audio.as_ref().map(|x| x.2).unwrap_or(192000)));
            a.extend(map_chapters);
            self.tail(&mut a, container);
            return Ok((a, codec));
        }
        s(&mut a, &["-map", "0:v:0"]);
        // pixel conversion and colour tags
        let pix = self.pix_fmt();
        let mut filters: Vec<String> = Vec::new();
        let matrix = if self.color.matrix.is_empty() { "bt709" } else { self.color.matrix.as_str() };
        let range = if self.color.full_range { "pc" } else { "tv" };
        let rgb_out = pix.starts_with("rgb") || pix.starts_with("gbr") || pix == "pal8";
        if !self.input.is_yuv() && !rgb_out {
            let m = match matrix {
                "bt2020nc" => "bt2020",
                "smpte170m" => "bt601",
                _ => "bt709",
            };
            filters.push(format!(
                "scale=in_range=pc:out_color_matrix={m}:out_range={}",
                if pix.starts_with("yuvj") { "pc" } else { range }
            ));
        }
        if !rgb_out {
            // tag the frames too: FFmpeg 8.1 drops -color_primaries and -color_trc (below) when the
            // frames arrive untagged
            let mut tags = Vec::new();
            if !self.color.primaries.is_empty() {
                tags.push(format!("color_primaries={}", self.color.primaries));
            }
            if !self.color.transfer.is_empty() {
                tags.push(format!("color_trc={}", self.color.transfer));
            }
            tags.push(format!("colorspace={matrix}:range={range}"));
            filters.push(format!("setparams={}", tags.join(":")));
        }
        if vaapi {
            filters.push("format=nv12,hwupload".into());
        }
        if self.codec == Codec::Gif {
            let chain = if filters.is_empty() { String::new() } else { format!("{},", filters.join(",")) };
            s(&mut a, &["-filter_complex", &format!("[0:v]{chain}split[a][b];[a]palettegen=reserve_transparent=1:stats_mode=full[p];[b][p]paletteuse=dither=sierra2_4a[v]"), "-map", "[v]"]);
            // the -map above supersedes 0:v:0
            let pos = a.iter().position(|x| x == "0:v:0").unwrap();
            a.drain(pos - 1..=pos);
        } else if !filters.is_empty() {
            s(&mut a, &["-vf", &filters.join(",")]);
        }
        s(&mut a, &["-c:v", &encoder]);
        // VA-API frames are uploaded by the filter chain in their own format
        if !vaapi {
            s(&mut a, &["-pix_fmt", &pix]);
        }
        let crf = self.crf.to_string();
        let gop = ((self.keyframe_interval * self.fps).round() as u64).max(1).to_string();
        let rate_control = |a: &mut Vec<String>, quality_flag: &str, quality: String| {
            if let Some(b) = self.bitrate {
                s(a, &["-b:v", &b.to_string()]);
            } else {
                s(a, &[quality_flag, &quality]);
            }
            if let Some(m) = self.max_bitrate {
                s(a, &["-maxrate", &m.to_string()]);
            }
            if let Some(b) = self.buffer_size {
                s(a, &["-bufsize", &b.to_string()]);
            }
        };
        let hdr_x265 = || {
            let mut p = Vec::new();
            if let Some(m) = &self.hdr.mastering_display {
                p.push(format!("master-display={m}"));
            }
            if self.hdr.max_cll.is_some() || self.hdr.max_fall.is_some() {
                p.push(format!("max-cll={},{}", self.hdr.max_cll.unwrap_or(0), self.hdr.max_fall.unwrap_or(0)));
            }
            if !p.is_empty() {
                p.push("hdr10=1:repeat-headers=1".into());
            }
            p
        };
        match encoder.as_str() {
            "libx264" => {
                s(&mut a, &["-preset", &self.preset]);
                rate_control(&mut a, "-crf", crf);
                s(&mut a, &["-g", &gop]);
                if let Some((p, log)) = &self.pass {
                    s(&mut a, &["-pass", &p.to_string(), "-passlogfile", &log.display().to_string()]);
                }
            }
            "libx265" => {
                s(&mut a, &["-preset", &self.preset]);
                rate_control(&mut a, "-crf", crf);
                let mut params = hdr_x265();
                params.push(format!("keyint={gop}"));
                if let Some((p, log)) = &self.pass {
                    params.push(format!("pass={p}:stats={}", log.display()));
                }
                s(&mut a, &["-x265-params", &params.join(":")]);
            }
            "libsvtav1" => {
                let svt = 12usize.saturating_sub(preset_rank(&self.preset) * 10 / 9).max(2);
                s(&mut a, &["-preset", &svt.to_string()]);
                rate_control(&mut a, "-crf", crf);
                let mut params = vec![format!("keyint={gop}")];
                if let Some((p, log)) = &self.pass {
                    s(&mut a, &["-pass", &p.to_string(), "-passlogfile", &log.display().to_string()]);
                }
                if let Some(m) = &self.hdr.mastering_display {
                    params.push(format!("mastering-display={m}"));
                }
                if self.hdr.max_cll.is_some() {
                    params.push(format!(
                        "content-light={},{}",
                        self.hdr.max_cll.unwrap_or(0),
                        self.hdr.max_fall.unwrap_or(0)
                    ));
                }
                s(&mut a, &["-svtav1-params", &params.join(":")]);
            }
            "libaom-av1" => {
                s(
                    &mut a,
                    &["-cpu-used", &(8 - preset_rank(&self.preset).min(8)).to_string(), "-row-mt", "1", "-g", &gop],
                );
                rate_control(&mut a, "-crf", crf);
                if self.bitrate.is_none() {
                    s(&mut a, &["-b:v", "0"]);
                }
                if let Some((p, log)) = &self.pass {
                    s(&mut a, &["-pass", &p.to_string(), "-passlogfile", &log.display().to_string()]);
                }
            }
            "librav1e" => {
                s(
                    &mut a,
                    &[
                        "-speed",
                        &(10 - preset_rank(&self.preset).min(10)).to_string(),
                        "-qp",
                        &(self.crf * 4).min(255).to_string(),
                        "-g",
                        &gop,
                    ],
                );
            }
            "libvpx-vp9" => {
                rate_control(&mut a, "-crf", crf);
                if self.bitrate.is_none() {
                    s(&mut a, &["-b:v", "0"]);
                }
                s(
                    &mut a,
                    &[
                        "-deadline",
                        "good",
                        "-cpu-used",
                        &(5 - preset_rank(&self.preset).min(5)).to_string(),
                        "-row-mt",
                        "1",
                        "-g",
                        &gop,
                    ],
                );
                if self.alpha {
                    s(&mut a, &["-auto-alt-ref", "0"]);
                }
                if let Some((p, log)) = &self.pass {
                    s(&mut a, &["-pass", &p.to_string(), "-passlogfile", &log.display().to_string()]);
                }
            }
            "prores_ks" => {
                let p = match self.prores_profile.as_deref().unwrap_or(if self.alpha { "4444" } else { "hq" }) {
                    "proxy" => 0,
                    "lt" => 1,
                    "422" => 2,
                    "hq" => 3,
                    "4444" => 4,
                    _ => 5,
                };
                s(&mut a, &["-profile:v", &p.to_string(), "-vendor", "apl0"]);
                if self.alpha {
                    s(&mut a, &["-alpha_bits", "16"]);
                }
            }
            "dnxhd" => {
                let deep = pix.contains("10");
                s(
                    &mut a,
                    &["-profile:v", self.profile.as_deref().unwrap_or(if deep { "dnxhr_hqx" } else { "dnxhr_hq" })],
                );
            }
            "ffv1" => s(&mut a, &["-level", "3", "-g", "1", "-slicecrc", "1"]),
            "gif" => s(&mut a, &["-loop", &self.loop_count.to_string()]),
            "apng" => s(&mut a, &["-plays", &self.loop_count.to_string()]),
            "libwebp_anim" => s(
                &mut a,
                &[
                    "-loop",
                    &self.loop_count.to_string(),
                    "-quality",
                    &(100u32.saturating_sub(self.crf * 2)).to_string(),
                    "-lossless",
                    "0",
                ],
            ),
            "png" | "tiff" | "exr" => {}
            "mjpeg" => s(&mut a, &["-q:v", &(2 + self.crf.min(51) * 29 / 51).to_string()]),
            e if e.ends_with("_nvenc") => {
                s(&mut a, &["-preset", &format!("p{}", (preset_rank(&self.preset) * 7 / 9 + 1).min(7)), "-rc", "vbr"]);
                rate_control(&mut a, "-cq", crf);
                s(&mut a, &["-g", &gop]);
            }
            e if e.ends_with("_videotoolbox") => {
                if let Some(b) = self.bitrate {
                    s(&mut a, &["-b:v", &b.to_string()]);
                } else {
                    s(&mut a, &["-q:v", &(100u32.saturating_sub(self.crf * 2)).to_string()]);
                }
                s(&mut a, &["-g", &gop]);
            }
            e if e.ends_with("_vaapi") => {
                rate_control(&mut a, "-qp", crf);
                s(&mut a, &["-g", &gop]);
            }
            e if e.ends_with("_qsv") => {
                s(&mut a, &["-preset", &self.preset]);
                rate_control(&mut a, "-global_quality", crf);
                s(&mut a, &["-g", &gop]);
            }
            e if e.ends_with("_amf") => {
                s(&mut a, &["-rc", "cqp", "-qp_i", &crf, "-qp_p", &crf, "-g", &gop]);
            }
            _ => {}
        }
        if let Some(p) = &self.profile {
            if !matches!(self.codec, Codec::Dnxhr | Codec::Prores) {
                s(&mut a, &["-profile:v", p]);
            }
        }
        if let Some(l) = &self.level {
            s(&mut a, &["-level", l]);
        }
        if let Some(b) = self.b_frames {
            if matches!(self.codec, Codec::H264 | Codec::H265) {
                s(&mut a, &["-bf", &b.to_string()]);
            }
        }
        if matches!(self.codec, Codec::H265) && matches!(container, Some(Container::Mp4 | Container::Mov)) {
            s(&mut a, &["-tag:v", "hvc1"]);
        }
        if !rgb_out {
            s(
                &mut a,
                &[
                    "-color_primaries",
                    &self.color.primaries,
                    "-color_trc",
                    &self.color.transfer,
                    "-colorspace",
                    matrix,
                    "-color_range",
                    range,
                ],
            );
        }
        if let Some((_, codec, bitrate)) = audio {
            let codec = if container == Some(Container::Webm) && codec == "aac" { "libopus" } else { codec.as_str() };
            s(&mut a, &["-map", "1:a:0", "-c:a", codec]);
            a.extend(compressed_audio_args(codec, *bitrate));
        }
        if let Some((1, _)) = &self.pass {
            s(&mut a, &["-an", "-f", "null", if cfg!(windows) { "NUL" } else { "/dev/null" }]);
            return Ok((a, encoder));
        }
        a.extend(map_chapters);
        self.tail(&mut a, container);
        Ok((a, encoder))
    }

    /// Joins video `segments`, each encoded with this spec (without audio) and each starting on a
    /// keyframe, into `self.path` in order: the video is stream-copied and `self.audio` is muxed
    /// as a single encode would mux it. `list` is where the concat list is written.
    /// Adds the chapters' FFmetadata input to `a` as input `index`, when there are chapters, the pass
    /// writes the file and the container holds chapters; returns the options that map them.
    fn chapter_input(&self, a: &mut Vec<String>, container: Option<Container>, index: usize) -> Vec<String> {
        let holds = matches!(
            container,
            Some(Container::Mp4 | Container::Mov | Container::Mkv | Container::Webm | Container::M4a)
        );
        match &self.chapters {
            Some(c) if holds && self.pass.as_ref().is_none_or(|p| p.0 == 2) => {
                a.extend(["-f".into(), "ffmetadata".into(), "-i".into(), c.display().to_string()]);
                vec!["-map_chapters".into(), index.to_string()]
            }
            _ => Vec::new(),
        }
    }

    pub fn join(&self, segments: &[PathBuf], list: &Path) -> Result<(), MediaError> {
        let quote = |p: &Path| format!("file '{}'\n", p.display().to_string().replace('\'', r"'\''"));
        std::fs::write(list, segments.iter().map(|p| quote(p)).collect::<String>())?;
        let mut a: Vec<String> = ["-hide_banner", "-nostdin", "-y", "-v", "error", "-f", "concat", "-safe", "0", "-i"]
            .iter()
            .map(|x| x.to_string())
            .collect();
        a.push(list.display().to_string());
        let audio =
            self.audio.as_ref().filter(|_| self.codec.takes_audio() && self.pass.as_ref().is_none_or(|p| p.0 == 2));
        if let Some((p, _, _)) = audio {
            a.push("-i".into());
            a.push(p.display().to_string());
        }
        let container = self.container();
        let map_chapters = self.chapter_input(&mut a, container, 1 + usize::from(audio.is_some()));
        a.extend(["-map", "0:v:0", "-c:v", "copy"].iter().map(|x| x.to_string()));
        if matches!(self.codec, Codec::H265) && matches!(container, Some(Container::Mp4 | Container::Mov)) {
            a.extend(["-tag:v".into(), "hvc1".into()]);
        }
        if let Some((_, codec, bitrate)) = audio {
            let codec = if container == Some(Container::Webm) && codec == "aac" { "libopus" } else { codec.as_str() };
            a.extend(["-map".into(), "1:a:0".into(), "-c:a".into(), codec.into()]);
            a.extend(compressed_audio_args(codec, *bitrate));
        }
        a.extend(map_chapters);
        self.tail(&mut a, container);
        if let Some(dir) = self.path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let output = staged_output(&self.path)?;
        *a.last_mut().expect("output argument") = output.display().to_string();
        let out = Command::new(crate::ffmpeg())
            .args(&a)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| MediaError::Spawn { tool: crate::ffmpeg(), source: e })?;
        if !out.status.success() {
            return Err(MediaError::Failed {
                tool: "ffmpeg".into(),
                path: self.path.display().to_string(),
                message: format!(
                    "joining {} segments: {}",
                    segments.len(),
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
            });
        }
        publish_output(output, &self.path)?;
        Ok(())
    }

    fn tail(&self, a: &mut Vec<String>, container: Option<Container>) {
        for (k, v) in &self.metadata {
            a.push("-metadata".into());
            a.push(format!("{k}={v}"));
        }
        if self.faststart && matches!(container, Some(Container::Mp4 | Container::Mov | Container::M4a)) {
            a.push("-movflags".into());
            a.push("+faststart".into());
        }
        if self.codec.is_sequence() {
            a.extend(["-f".into(), "image2".into(), "-start_number".into(), self.start_number.to_string()]);
        } else if let Some(m) = match self.codec {
            Codec::Gif => Some("gif"),
            Codec::Apng => Some("apng"),
            Codec::Webp => Some("webp"),
            _ => None,
        } {
            a.extend(["-f".into(), m.into()]);
        } else if let (Some(c), false) = (container, matches!(self.codec, Codec::Gif | Codec::Apng | Codec::Webp)) {
            a.push("-f".into());
            a.push(c.muxer().into());
        }
        // only a sequence's file name is a frame pattern: a `%` in its directory is escaped
        match (self.path.parent(), self.path.file_name()) {
            (Some(dir), Some(name)) if self.codec.is_sequence() => {
                a.push(Path::new(&dir.to_string_lossy().replace('%', "%%")).join(name).display().to_string())
            }
            _ => a.push(self.path.display().to_string()),
        }
    }
}

fn staged_output(path: &Path) -> std::io::Result<tempfile::TempPath> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut builder = tempfile::Builder::new();
    builder.prefix(".scene-render-");
    // Keep encoded bytes private until publication, including for new outputs.
    // tempfile uses owner-only permissions; never relax them while FFmpeg writes.
    Ok(builder.tempfile_in(parent)?.into_temp_path())
}

fn publish_output(output: tempfile::TempPath, path: &Path) -> std::io::Result<()> {
    if let Ok(metadata) = std::fs::metadata(path) {
        std::fs::set_permissions(&output, metadata.permissions())?;
    }
    output.persist(path).map_err(|e| e.error)
}

/// A running encoder fed through its standard input. Dropped before [`Encoder::finish`] succeeds, it stops
/// FFmpeg and removes the unfinished file.
pub struct Encoder {
    child: Child,
    stdin: Option<ChildStdin>,
    errors: Option<std::thread::JoinHandle<Vec<u8>>>,
    path: PathBuf,
    /// The one file this run writes (none for a first pass or a sequence).
    output: Option<tempfile::TempPath>,
    finished: bool,
    /// Encoder name in use.
    pub encoder: String,
    /// Full command line, for diagnostics.
    pub command: Vec<String>,
}

impl Encoder {
    /// Starts FFmpeg for `spec`.
    pub fn start(spec: &EncodeSpec) -> Result<Encoder, MediaError> {
        let (mut args, encoder) = spec.args()?;
        if let Some(dir) = spec.path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let first_pass = matches!(spec.pass, Some((1, _)));
        let output = if !first_pass && !spec.codec.is_sequence() {
            let temp = staged_output(&spec.path)?;
            // args already selected the muxer from the original destination.
            *args.last_mut().expect("output argument") = temp.display().to_string();
            Some(temp)
        } else {
            None
        };
        let mut child = Command::new(crate::ffmpeg())
            .args(&args)
            .stdin(if spec.codec.is_audio_only() { Stdio::null() } else { Stdio::piped() })
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| MediaError::Spawn { tool: crate::ffmpeg(), source: e })?;
        let mut err = child.stderr.take().expect("piped");
        let errors = std::thread::spawn(move || {
            let mut v = Vec::new();
            let _ = err.read_to_end(&mut v);
            v
        });
        Ok(Encoder {
            stdin: child.stdin.take(),
            child,
            errors: Some(errors),
            path: spec.path.clone(),
            output,
            finished: false,
            encoder,
            command: args,
        })
    }

    fn said(&mut self) -> Vec<u8> {
        self.errors.take().and_then(|h| h.join().ok()).unwrap_or_default()
    }

    /// Writes one frame.
    pub fn write(&mut self, frame: &[u8]) -> Result<(), MediaError> {
        let Some(s) = self.stdin.as_mut() else { return Ok(()) };
        let Err(e) = s.write_all(frame) else { return Ok(()) };
        // FFmpeg stopped reading (it rejected the stream, or has no such encoder): it says why as it ends
        drop(self.stdin.take());
        let _ = self.child.wait();
        let said = crate::reason(&self.said());
        Err(MediaError::Failed {
            tool: "ffmpeg".into(),
            path: self.path.display().to_string(),
            message: if said.is_empty() { format!("encoder stopped accepting frames: {e}") } else { said },
        })
    }

    /// Closes the input and waits for the file to be finished.
    pub fn finish(mut self) -> Result<(), MediaError> {
        drop(self.stdin.take());
        let status = self.child.wait()?;
        let err = self.said();
        if !status.success() {
            return Err(MediaError::Failed {
                tool: "ffmpeg".into(),
                path: self.path.display().to_string(),
                message: crate::reason(&err),
            });
        }
        if let Some(output) = self.output.take() {
            publish_output(output, &self.path)?;
        }
        self.finished = true;
        Ok(())
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // abandoned or failed: stop FFmpeg before it finalises, and leave no truncated file behind
        let _ = self.child.kill();
        let _ = self.child.wait();
        // TempPath removes only the unfinished file owned by this invocation.
    }
}

/// Still formats of posters and thumbnails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StillFormat {
    Jpeg,
    Png,
    Webp,
    Avif,
}

impl StillFormat {
    /// Parses the schema name.
    pub fn parse(s: &str) -> Option<StillFormat> {
        Some(match s {
            "jpeg" => StillFormat::Jpeg,
            "png" => StillFormat::Png,
            "webp" => StillFormat::Webp,
            "avif" => StillFormat::Avif,
            _ => return None,
        })
    }
}

/// Encodes one straight-alpha 8-bit sRGB RGBA image, scaled to `width` (aspect kept).
pub fn write_still(
    path: &Path,
    rgba: &[u8],
    w: u32,
    h: u32,
    format: StillFormat,
    width: Option<u32>,
    quality: f64,
) -> Result<(), MediaError> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let mut args: Vec<String> =
        ["-hide_banner", "-nostdin", "-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    args.push(format!("{w}x{h}"));
    args.extend(["-i".into(), "pipe:0".into(), "-frames:v".into(), "1".into()]);
    if let Some(tw) = width {
        args.extend(["-vf".into(), format!("scale={tw}:-2:flags=lanczos")]);
    }
    let q = quality.clamp(0.0, 1.0);
    match format {
        StillFormat::Jpeg => args.extend([
            "-c:v".into(),
            "mjpeg".into(),
            "-pix_fmt".into(),
            "yuvj444p".into(),
            "-q:v".into(),
            format!("{}", (2.0 + (1.0 - q) * 29.0).round()),
        ]),
        StillFormat::Png => args.extend(["-c:v".into(), "png".into()]),
        StillFormat::Webp => {
            args.extend(["-c:v".into(), "libwebp".into(), "-quality".into(), format!("{}", (q * 100.0).round())])
        }
        StillFormat::Avif => {
            let enc = if working_encoder("libaom-av1") { "libaom-av1" } else { "libsvtav1" };
            args.extend([
                "-c:v".into(),
                enc.into(),
                "-still-picture".into(),
                "1".into(),
                "-crf".into(),
                format!("{}", ((1.0 - q) * 63.0).round()),
                "-pix_fmt".into(),
                "yuv444p".into(),
            ]);
            args.extend(["-f".into(), "avif".into()]);
        }
    }
    if matches!(format, StillFormat::Jpeg | StillFormat::Png | StillFormat::Webp) {
        args.extend(["-f".into(), "image2".into(), "-update".into(), "1".into()]);
    }
    args.push(path.display().to_string());
    let mut child = Command::new(crate::ffmpeg())
        .args(&args)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|e| MediaError::Spawn { tool: crate::ffmpeg(), source: e })?;
    child.stdin.take().expect("piped").write_all(rgba)?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(MediaError::Failed {
            tool: "ffmpeg".into(),
            path: path.display().to_string(),
            message: crate::tail(&out.stderr, 3),
        });
    }
    Ok(())
}

/// Video bitrate that fits `max_bytes` over `duration` seconds after audio
/// and a 3 % container allowance.
pub fn fit_bitrate(max_bytes: u64, duration: f64, audio_bps: u64) -> u64 {
    let total = max_bytes as f64 * 8.0 * 0.97 / duration.max(0.001);
    (total - audio_bps as f64).max(10_000.0) as u64
}
