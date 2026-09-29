//! Still images in every common format, with orientation and colour.
//!
//! * PNG, JPEG, WebP, GIF, TIFF, BMP, TGA, QOI, PNM, ICO, DDS, farbfeld,
//!   Radiance HDR and OpenEXR decode in-process (the `image` crate); JPEG XL
//!   decodes in-process through `jxl-oxide`.
//! * HEIC, HEIF and AVIF, and anything else the in-process decoders cannot read
//!   (PSD, JPEG 2000, SGI, PCX, …), decode through FFmpeg, one frame. The
//!   colour of HEIF files is read from their `colr` property, since FFmpeg does
//!   not pass it on.
//! * EXIF orientation is applied, so phone photos come out upright.
//! * The embedded colour description is returned for the caller to honour:
//!   an ICC profile (PNG `iCCP`, JPEG `APP2`, WebP `ICCP`, TIFF, JPEG XL, HEIF
//!   `prof`) or H.273 code points (HEIF `nclx`, ICC `cicp`).
//!
//! Animated GIF, APNG and WebP load their first frame here. As `<video>`
//! sources GIF and APNG play through FFmpeg like any clip; FFmpeg does not
//! decode animated WebP.

use std::io::Read;
use std::path::Path;
use std::process::Command;

use image::{DynamicImage, ImageDecoder};

use crate::heif;
use crate::icc;

/// The colour description a file carries.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Colour {
    /// None: the document's declared colour space applies.
    #[default]
    Unknown,
    /// A matrix/TRC ICC profile.
    Icc(Box<icc::Profile>),
    /// ITU-T H.273 colour primaries and transfer characteristics.
    Cicp(u16, u16),
    /// A profile that could not be used, and why (CMYK, lookup-table only, damaged).
    Unsupported(String),
}

/// A decoded still.
pub struct Still {
    /// Pixels, upright.
    pub image: DynamicImage,
    /// Embedded colour description.
    pub colour: Colour,
    /// Whether the format stores linear scene light (Radiance HDR, OpenEXR, floating-point TIFF).
    pub linear: bool,
    /// The decoder that read it (`png`, `jpeg`, `jxl`, `ffmpeg:hevc`, …).
    pub decoder: String,
}

fn icc_colour(bytes: Option<Vec<u8>>) -> Colour {
    match bytes.filter(|b| !b.is_empty()) {
        None => Colour::Unknown,
        Some(b) => match icc::parse(&b) {
            Ok(p) => match p.cicp {
                // A profile stating H.273 code points means exactly those (ICC 4.4, clause 10.3).
                Some((pr, tc)) if pr != 2 && tc != 2 => Colour::Cicp(pr as u16, tc as u16),
                _ => Colour::Icc(Box::new(p)),
            },
            Err(e) => Colour::Unsupported(e),
        },
    }
}

/// Decodes a still image file.
pub fn open(path: &Path) -> Result<Still, String> {
    let mut head = [0u8; 64];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let head = &head[..n];
    if heif::is_heif(head) {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let colour = heif::colour(&bytes)
            .into_iter()
            .map(|c| match c {
                heif::Colr::Icc(b) => icc_colour(Some(b)),
                heif::Colr::Nclx(p, t, _, _) => Colour::Cicp(p, t),
            })
            .find(|c| !matches!(c, Colour::Unsupported(_)))
            .unwrap_or_default();
        let mut s = ffmpeg_still(path)?;
        s.colour = colour;
        return Ok(s);
    }
    if head.starts_with(&[0xff, 0x0a]) || head.get(4..8) == Some(b"JXL ") {
        return jxl(path);
    }
    match native(path) {
        Ok(s) => Ok(s),
        Err(native_err) => ffmpeg_still(path).map_err(|e| format!("{}: {native_err}; FFmpeg: {e}", path.display())),
    }
}

fn native(path: &Path) -> Result<Still, String> {
    let reader = image::ImageReader::open(path).and_then(|r| r.with_guessed_format()).map_err(|e| e.to_string())?;
    let format = reader.format().ok_or("unknown format")?;
    let mut dec = reader.into_decoder().map_err(|e| e.to_string())?;
    let colour = icc_colour(dec.icc_profile().ok().flatten());
    let orientation = dec.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(dec).map_err(|e| e.to_string())?;
    image.apply_orientation(orientation);
    let linear = matches!(format, image::ImageFormat::Hdr | image::ImageFormat::OpenExr)
        || matches!(image, DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_));
    Ok(Still { image, colour, linear, decoder: format!("{format:?}").to_lowercase() })
}

fn jxl(path: &Path) -> Result<Still, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut dec = jxl_oxide::integration::JxlDecoder::new(std::io::BufReader::new(f)).map_err(|e| e.to_string())?;
    let colour = icc_colour(dec.icc_profile().ok().flatten());
    // jxl-oxide renders the codestream's orientation itself.
    let image = DynamicImage::from_decoder(dec).map_err(|e| e.to_string())?;
    let linear = matches!(image, DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_))
        && matches!(colour, Colour::Icc(ref p) if p.curves.iter().all(|c| *c == icc::Curve::Identity));
    Ok(Still { image, colour, linear, decoder: "jxl".into() })
}

/// One frame through FFmpeg as RGBA (PAM), upright (FFmpeg applies display rotation). Sources
/// deeper than 8 bits come out as 16-bit; 8-bit ones stay 8-bit, since widening them in FFmpeg
/// rounds some values down a step.
fn ffmpeg_still(path: &Path) -> Result<Still, String> {
    let info = crate::probe(path).ok().and_then(|i| i.video);
    let deep = info.as_ref().is_some_and(|v| v.depth > 8);
    let out = Command::new(crate::ffmpeg())
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-f",
            "image2pipe",
            "-c:v",
            "pam",
            "-pix_fmt",
            if deep { "rgba64be" } else { "rgba" },
            "-",
        ])
        .output()
        .map_err(|e| format!("cannot run FFmpeg: {e}"))?;
    if !out.status.success() || out.stdout.is_empty() {
        let msg = String::from_utf8_lossy(&out.stderr);
        return Err(msg.lines().last().unwrap_or("no frame").to_string());
    }
    let image = pam(&out.stdout).ok_or("unreadable FFmpeg output")?;
    let codec = info.map(|v| v.codec).unwrap_or_default();
    Ok(Still { image, colour: Colour::Unknown, linear: false, decoder: format!("ffmpeg:{codec}") })
}

/// A 4-channel PAM image, 8 or 16 bits.
fn pam(b: &[u8]) -> Option<DynamicImage> {
    let end = b.windows(7).position(|w| w == b"ENDHDR\n")? + 7;
    let head = std::str::from_utf8(&b[..end]).ok()?;
    let field = |k: &str| head.lines().find_map(|l| l.strip_prefix(k)).and_then(|v| v.trim().parse::<u32>().ok());
    let (w, h) = (field("WIDTH ")?, field("HEIGHT ")?);
    (field("DEPTH ")? == 4).then_some(())?;
    let n = (w * h * 4) as usize;
    match field("MAXVAL ")? {
        255 => image::RgbaImage::from_raw(w, h, b.get(end..end + n)?.to_vec()).map(DynamicImage::ImageRgba8),
        65535 => {
            let px = b.get(end..end + 2 * n)?.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            image::ImageBuffer::from_raw(w, h, px).map(DynamicImage::ImageRgba16)
        }
        _ => None,
    }
}
