//! Stream properties from `ffprobe`.

use std::path::Path;
use std::process::Command;

use crate::MediaError;

/// Properties of a media file's first video and selected audio stream.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct MediaInfo {
    /// Container duration in seconds.
    pub duration: f64,
    /// Video stream, if any.
    pub video: Option<VideoInfo>,
    /// Audio streams in file order.
    pub audio: Vec<AudioInfo>,
}

/// Video stream properties.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct VideoInfo {
    /// Coded width.
    pub width: u32,
    /// Coded height.
    pub height: u32,
    /// Average frame rate.
    pub fps: f64,
    /// Codec name.
    pub codec: String,
    /// FFmpeg pixel format name.
    pub pix_fmt: String,
    /// Bits per component.
    pub depth: u32,
    /// Horizontal and vertical chroma subsampling shifts.
    pub chroma_shift: (u32, u32),
    /// Has an alpha channel.
    pub alpha: bool,
    /// RGB (not YUV) samples.
    pub rgb: bool,
    /// `color_space` tag (matrix coefficients).
    pub matrix: String,
    /// `color_transfer` tag.
    pub transfer: String,
    /// `color_primaries` tag.
    pub primaries: String,
    /// Full-range samples.
    pub full_range: bool,
    /// Display rotation in degrees clockwise from the display matrix.
    pub rotation: i32,
}

/// Audio stream properties.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct AudioInfo {
    /// Sample rate.
    pub sample_rate: u32,
    /// Channels.
    pub channels: u16,
    /// Channel layout name (`stereo`, `5.1(side)`, …).
    pub layout: String,
    /// Codec name.
    pub codec: String,
}

fn rate(s: &str) -> f64 {
    match s.split_once('/') {
        Some((a, b)) => {
            let (a, b): (f64, f64) = (a.parse().unwrap_or(0.0), b.parse().unwrap_or(1.0));
            if b == 0.0 {
                0.0
            } else {
                a / b
            }
        }
        None => s.parse().unwrap_or(0.0),
    }
}

/// Pixel format facts: (depth, (chroma shift x, y), alpha, rgb).
pub fn pix_fmt_facts(p: &str) -> (u32, (u32, u32), bool, bool) {
    let depth = ["16", "14", "12", "10", "9"].iter().find(|d| p.contains(*d)).map(|d| d.parse().unwrap()).unwrap_or(8);
    let depth = if p.contains("f32") || p.contains("f16") { 16 } else { depth };
    let rgb = p.starts_with("rgb")
        || p.starts_with("bgr")
        || p.starts_with("gbr")
        || p.starts_with("argb")
        || p.starts_with("abgr")
        || p.starts_with("pal")
        || p.starts_with("gray")
        || p.starts_with("ya")
        || p == "0rgb"
        || p == "rgb0";
    let alpha = p.starts_with("yuva")
        || p.contains("rgba")
        || p.contains("bgra")
        || p.contains("argb")
        || p.contains("abgr")
        || p.starts_with("gbrap")
        || p.starts_with("ya")
        || p.starts_with("pal");
    let shift = if p.contains("420")
        || p.starts_with("nv12")
        || p.starts_with("nv21")
        || p.starts_with("p010")
        || p.starts_with("p016")
    {
        (1, 1)
    } else if p.contains("422") || p.starts_with("nv16") || p.starts_with("p210") || p.starts_with("y210") {
        (1, 0)
    } else if p.contains("411") {
        (2, 0)
    } else {
        (0, 0)
    };
    (depth, shift, alpha, rgb)
}

/// Probes a media file.
pub fn probe(path: &Path) -> Result<MediaInfo, MediaError> {
    let out = Command::new(crate::ffprobe())
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .map_err(|e| MediaError::Spawn { tool: crate::ffprobe(), source: e })?;
    if !out.status.success() {
        return Err(MediaError::Failed {
            tool: "ffprobe".into(),
            path: path.display().to_string(),
            message: crate::tail(&out.stderr, 3),
        });
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| MediaError::Invalid(format!("ffprobe output: {e}")))?;
    let s = |x: &serde_json::Value, k: &str| x.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let n = |x: &serde_json::Value, k: &str| {
        x.get(k).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0)
    };
    let mut info = MediaInfo { duration: s(&v["format"], "duration").parse().unwrap_or(0.0), ..Default::default() };
    for st in v["streams"].as_array().into_iter().flatten() {
        match st["codec_type"].as_str() {
            Some("video") if info.video.is_none() && st["disposition"]["attached_pic"].as_u64() != Some(1) => {
                let pix = s(st, "pix_fmt");
                let (depth, chroma_shift, alpha, rgb) = pix_fmt_facts(&pix);
                let mut rotation = 0;
                for sd in st["side_data_list"].as_array().into_iter().flatten() {
                    if let Some(r) = sd.get("rotation").and_then(|r| r.as_f64()) {
                        // ffprobe reports counter-clockwise degrees
                        rotation = ((-r).round() as i32).rem_euclid(360);
                    }
                }
                if let Some(r) = st["tags"]["rotate"].as_str().and_then(|r| r.parse::<i32>().ok()) {
                    rotation = r.rem_euclid(360);
                }
                let fps = match rate(&s(st, "avg_frame_rate")) {
                    f if f > 0.0 => f,
                    _ => rate(&s(st, "r_frame_rate")),
                };
                info.video = Some(VideoInfo {
                    width: n(st, "width") as u32,
                    height: n(st, "height") as u32,
                    fps,
                    codec: s(st, "codec_name"),
                    pix_fmt: pix,
                    depth,
                    chroma_shift,
                    alpha,
                    rgb,
                    matrix: s(st, "color_space"),
                    transfer: s(st, "color_transfer"),
                    primaries: s(st, "color_primaries"),
                    full_range: s(st, "color_range") == "pc",
                    rotation,
                });
            }
            Some("audio") => info.audio.push(AudioInfo {
                sample_rate: s(st, "sample_rate").parse().unwrap_or(0),
                channels: n(st, "channels") as u16,
                layout: s(st, "channel_layout"),
                codec: s(st, "codec_name"),
            }),
            _ => {}
        }
    }
    if info.duration == 0.0 {
        info.duration = v["streams"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|st| s(st, "duration").parse::<f64>().ok())
            .fold(0.0, f64::max);
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_formats() {
        assert_eq!(pix_fmt_facts("yuv420p"), (8, (1, 1), false, false));
        assert_eq!(pix_fmt_facts("yuv420p10le"), (10, (1, 1), false, false));
        assert_eq!(pix_fmt_facts("yuva444p12le"), (12, (0, 0), true, false));
        assert_eq!(pix_fmt_facts("yuv422p10le"), (10, (1, 0), false, false));
        assert_eq!(pix_fmt_facts("rgba"), (8, (0, 0), true, true));
        assert_eq!(pix_fmt_facts("gbrp16le"), (16, (0, 0), false, true));
        assert_eq!(rate("30000/1001"), 30000.0 / 1001.0);
    }
}
