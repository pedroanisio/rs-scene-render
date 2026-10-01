//! Value types whose lexical forms carry structure: lengths with units,
//! colours, paints, rational frame rates, timecodes, points, aspect ratios,
//! SHA-256 digests and language tags.

use std::fmt;

use crate::parse::{ParseValue, ValueError};
use crate::xsd::parse_xsd_double;

macro_rules! serialize_display {
    ($($t:ty),*) => {$(
        impl serde::Serialize for $t {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }
    )*};
}

// ------------------------------------------------------------------ Length

/// Unit of a [`Length`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LengthUnit {
    /// Pixels (a bare number).
    Px,
    /// Percent of the parent box along the same axis.
    Percent,
    /// Percent of the output frame width.
    Vw,
    /// Percent of the output frame height.
    Vh,
    /// Percent of the smaller output frame dimension.
    Vmin,
    /// Percent of the larger output frame dimension.
    Vmax,
}

impl LengthUnit {
    fn suffix(self) -> &'static str {
        match self {
            LengthUnit::Px => "",
            LengthUnit::Percent => "%",
            LengthUnit::Vw => "vw",
            LengthUnit::Vh => "vh",
            LengthUnit::Vmin => "vmin",
            LengthUnit::Vmax => "vmax",
        }
    }
}

/// `lengthType`: pixels, or a percentage relative to the parent box or the frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Length {
    /// Magnitude in the unit.
    pub value: f64,
    /// Unit.
    pub unit: LengthUnit,
}

impl Length {
    /// A length in pixels.
    pub const fn px(value: f64) -> Self {
        Length { value, unit: LengthUnit::Px }
    }

    /// Resolves to pixels. `parent` is the parent box extent along this axis;
    /// `frame_w` and `frame_h` are the output frame size.
    pub fn resolve(&self, parent: f64, frame_w: f64, frame_h: f64) -> f64 {
        match self.unit {
            LengthUnit::Px => self.value,
            LengthUnit::Percent => self.value / 100.0 * parent,
            LengthUnit::Vw => self.value / 100.0 * frame_w,
            LengthUnit::Vh => self.value / 100.0 * frame_h,
            LengthUnit::Vmin => self.value / 100.0 * frame_w.min(frame_h),
            LengthUnit::Vmax => self.value / 100.0 * frame_w.max(frame_h),
        }
    }
}

impl ParseValue for Length {
    fn parse_value(s: &str) -> Result<Self, ValueError> {
        let s = s.trim();
        for (suffix, unit) in [
            ("vmin", LengthUnit::Vmin),
            ("vmax", LengthUnit::Vmax),
            ("vw", LengthUnit::Vw),
            ("vh", LengthUnit::Vh),
            ("%", LengthUnit::Percent),
        ] {
            if let Some(num) = s.strip_suffix(suffix) {
                let value = num.parse::<f64>().map_err(|_| ValueError::new(format!("{s:?} is not a length")))?;
                return Ok(Length { value, unit });
            }
        }
        parse_xsd_double(s).map(Length::px).ok_or_else(|| ValueError::new(format!("{s:?} is not a length")))
    }
}

impl fmt::Display for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.value, self.unit.suffix())
    }
}

// ------------------------------------------------------------------ Colour and paint

/// A colour with straight (non-premultiplied) alpha in the working colour space.
/// Channels lie in `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    /// Red.
    pub r: f32,
    /// Green.
    pub g: f32,
    /// Blue.
    pub b: f32,
    /// Alpha.
    pub a: f32,
}

impl Rgba {
    /// Opaque black, the project background default.
    pub const BLACK: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

    fn parse_literal(s: &str) -> Option<Rgba> {
        if let Some(hex) = s.strip_prefix('#') {
            if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let ch = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok().map(|v| v as f32 / 255.0);
            return Some(Rgba { r: ch(0)?, g: ch(2)?, b: ch(4)?, a: if hex.len() == 8 { ch(6)? } else { 1.0 } });
        }
        let parts: Vec<f32> = s.split(',').map(|p| p.trim().parse::<f32>()).collect::<Result<_, _>>().ok()?;
        if !(3..=4).contains(&parts.len()) || parts.iter().any(|v| !(0.0..=1.0).contains(v)) {
            return None;
        }
        Some(Rgba { r: parts[0], g: parts[1], b: parts[2], a: parts.get(3).copied().unwrap_or(1.0) })
    }
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let exact = |v: f32| {
            let s = v * 255.0;
            (s - s.round()).abs() < 1e-3
        };
        if [self.r, self.g, self.b, self.a].into_iter().all(exact) {
            let b = |v: f32| (v * 255.0).round() as u8;
            write!(f, "#{:02x}{:02x}{:02x}{:02x}", b(self.r), b(self.g), b(self.b), b(self.a))
        } else {
            write!(f, "{},{},{},{}", self.r, self.g, self.b, self.a)
        }
    }
}

/// `colorType`: a literal colour or a style token reference `var(--name)`.
#[derive(Debug, Clone, PartialEq)]
pub enum Color {
    /// A literal colour.
    Rgba(Rgba),
    /// `var(--name)`: the value of the style token `name`.
    Token(String),
}

fn token_ref(s: &str) -> Option<&str> {
    s.strip_prefix("var(--")?.strip_suffix(')')
}

impl ParseValue for Color {
    fn parse_value(s: &str) -> Result<Self, ValueError> {
        if let Some(name) = token_ref(s) {
            return Ok(Color::Token(name.to_string()));
        }
        Rgba::parse_literal(s).map(Color::Rgba).ok_or_else(|| ValueError::new(format!("{s:?} is not a colour")))
    }
}

impl ParseValue for Vec<Color> {
    fn parse_value(s: &str) -> Result<Self, ValueError> {
        s.split_whitespace().map(Color::parse_value).collect()
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Color::Rgba(c) => c.fmt(f),
            Color::Token(t) => write!(f, "var(--{t})"),
        }
    }
}

/// `paintRefType`: `url(#id)` naming an element of `<paints>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PaintRef(pub String);

impl ParseValue for PaintRef {
    fn parse_value(s: &str) -> Result<Self, ValueError> {
        s.strip_prefix("url(#")
            .and_then(|r| r.strip_suffix(')'))
            .filter(|id| !id.is_empty())
            .map(|id| PaintRef(id.to_string()))
            .ok_or_else(|| ValueError::new(format!("{s:?} is not a paint reference url(#id)")))
    }
}

impl fmt::Display for PaintRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "url(#{})", self.0)
    }
}

/// `paintType`: a colour, or a gradient, pattern or noise from `<paints>`.
#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    /// A colour or token reference.
    Color(Color),
    /// `url(#id)`.
    Ref(PaintRef),
}

impl ParseValue for Paint {
    fn parse_value(s: &str) -> Result<Self, ValueError> {
        if s.starts_with("url(") {
            PaintRef::parse_value(s).map(Paint::Ref)
        } else {
            Color::parse_value(s).map(Paint::Color)
        }
    }
}

impl fmt::Display for Paint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Paint::Color(c) => c.fmt(f),
            Paint::Ref(r) => r.fmt(f),
        }
    }
}

// ------------------------------------------------------------------ Time

/// `fpsType`: a rational frame rate such as `30`, `24000/1001` or `30000/1001`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fps {
    /// Numerator (frames).
    pub num: u32,
    /// Denominator (seconds).
    pub den: u32,
}

impl Fps {
    /// A rate from a numerator and denominator; both must be non-zero.
    pub fn new(num: u32, den: u32) -> Option<Self> {
        (num > 0 && den > 0).then_some(Fps { num, den })
    }

    /// Frames per second as a float.
    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Duration of one frame in seconds.
    pub fn frame_duration(self) -> f64 {
        self.den as f64 / self.num as f64
    }

    /// Start time of frame `i` in seconds, computed exactly from the ratio.
    pub fn frame_time(self, i: u64) -> f64 {
        (i as u128 * self.den as u128) as f64 / self.num as f64
    }

    /// Index of the frame displayed at time `t` (seconds). Times within one
    /// nanosecond below a frame boundary belong to the next frame, which
    /// absorbs rounding in times written as decimals.
    pub fn frame_at(self, t: f64) -> u64 {
        let f = t * self.num as f64 / self.den as f64;
        (f + 1e-9 * self.as_f64()).floor().max(0.0) as u64
    }

    /// Number of frames needed to cover `duration` seconds: the count is
    /// rounded to the nearest frame when within one nanosecond of it and
    /// rounded up otherwise, so the last partial frame is rendered.
    pub fn frame_count(self, duration: f64) -> u64 {
        let f = duration * self.num as f64 / self.den as f64;
        let r = f.round();
        if (f - r).abs() <= 1e-9 * self.as_f64() {
            r as u64
        } else {
            f.ceil() as u64
        }
    }

    /// Nominal integer rate used by timecode (e.g. 30 for 30000/1001).
    pub fn nominal(self) -> u32 {
        self.as_f64().round() as u32
    }
}

impl ParseValue for Fps {
    fn parse_value(s: &str) -> Result<Self, ValueError> {
        let s = s.trim();
        let (n, d) = s.split_once('/').unwrap_or((s, "1"));
        let n: u32 =
            n.parse().map_err(|_| ValueError::new(format!("{s:?}: frame-rate numerator must be 1..=4294967295")))?;
        let d: u32 =
            d.parse().map_err(|_| ValueError::new(format!("{s:?}: frame-rate denominator must be 1..=4294967295")))?;
        Fps::new(n, d).ok_or_else(|| ValueError::new(format!("{s:?} is not a frame rate")))
    }
}

impl fmt::Display for Fps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

/// `timecodeType`: `HH:MM:SS:FF`, or `HH:MM:SS;FF` for drop-frame timecode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Timecode {
    /// Hours.
    pub hours: u32,
    /// Minutes.
    pub minutes: u32,
    /// Seconds.
    pub seconds: u32,
    /// Frames.
    pub frames: u32,
    /// Drop-frame counting (`;` separator).
    pub drop_frame: bool,
}

impl Timecode {
    /// Frame number of this timecode at `fps`. Drop-frame timecode skips
    /// frame labels 0..n at the start of every minute except every tenth,
    /// where n is 2 at 29.97 fps and scales with the nominal rate.
    pub fn to_frame(self, fps: Fps) -> Result<u64, ValueError> {
        let nominal = fps.nominal() as u64;
        if nominal == 0 || self.frames as u64 >= nominal {
            return Err(ValueError::new(format!(
                "frame field {} is not below the nominal rate {nominal}",
                self.frames
            )));
        }
        let (h, m, s, f) = (self.hours as u64, self.minutes as u64, self.seconds as u64, self.frames as u64);
        let base = ((h * 60 + m) * 60 + s) * nominal + f;
        if !self.drop_frame {
            return Ok(base);
        }
        if !nominal.is_multiple_of(30) {
            return Err(ValueError::new("drop-frame timecode requires a 29.97 or 59.94 fps family rate"));
        }
        let drop = nominal / 15;
        if s == 0 && m % 10 != 0 && f < drop {
            return Err(ValueError::new(format!(
                "frame label {f} does not exist in drop-frame timecode at minute {m}"
            )));
        }
        let total_minutes = h * 60 + m;
        Ok(base - drop * (total_minutes - total_minutes / 10))
    }

    /// Start time in seconds at `fps`.
    pub fn to_seconds(self, fps: Fps) -> Result<f64, ValueError> {
        self.to_frame(fps).map(|fr| fps.frame_time(fr))
    }
}

impl ParseValue for Timecode {
    fn parse_value(s: &str) -> Result<Self, ValueError> {
        let s = s.trim();
        let err = || ValueError::new(format!("{s:?} is not a timecode HH:MM:SS:FF"));
        let b = s.as_bytes();
        if b.len() != 11 || b[2] != b':' || b[5] != b':' || !(b[8] == b':' || b[8] == b';') {
            return Err(err());
        }
        let num = |r: std::ops::Range<usize>| s.get(r).and_then(|x| x.parse::<u32>().ok()).ok_or_else(err);
        let tc = Timecode {
            hours: num(0..2)?,
            minutes: num(3..5)?,
            seconds: num(6..8)?,
            frames: num(9..11)?,
            drop_frame: b[8] == b';',
        };
        if tc.minutes > 59 || tc.seconds > 59 {
            return Err(err());
        }
        Ok(tc)
    }
}

impl fmt::Display for Timecode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sep = if self.drop_frame { ';' } else { ':' };
        write!(f, "{:02}:{:02}:{:02}{sep}{:02}", self.hours, self.minutes, self.seconds, self.frames)
    }
}

// ------------------------------------------------------------------ Geometry and misc

/// `pointType`: an `x,y` pair.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point2 {
    /// X.
    pub x: f64,
    /// Y.
    pub y: f64,
}

impl ParseValue for Point2 {
    fn parse_value(s: &str) -> Result<Self, ValueError> {
        let (x, y) = s.trim().split_once(',').ok_or_else(|| ValueError::new(format!("{s:?} is not a point x,y")))?;
        let p = |v: &str| v.parse::<f64>().map_err(|_| ValueError::new(format!("{s:?} is not a point x,y")));
        Ok(Point2 { x: p(x)?, y: p(y)? })
    }
}

impl fmt::Display for Point2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{}", self.x, self.y)
    }
}

/// `aspectType`: `W:H`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Aspect {
    /// Width term.
    pub w: u32,
    /// Height term.
    pub h: u32,
}

impl Aspect {
    /// Width divided by height.
    pub fn ratio(self) -> f64 {
        self.w as f64 / self.h as f64
    }
}

impl ParseValue for Aspect {
    fn parse_value(s: &str) -> Result<Self, ValueError> {
        let err = || ValueError::new(format!("{s:?} is not an aspect ratio W:H"));
        let (w, h) = s.trim().split_once(':').ok_or_else(err)?;
        let (w, h) = (w.parse::<u32>().map_err(|_| err())?, h.parse::<u32>().map_err(|_| err())?);
        if w == 0 || h == 0 {
            return Err(err());
        }
        Ok(Aspect { w, h })
    }
}

impl fmt::Display for Aspect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.w, self.h)
    }
}

/// `sha256Type`: a SHA-256 digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha256(pub [u8; 32]);

impl Sha256 {
    /// Lowercase hexadecimal form.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl ParseValue for Sha256 {
    fn parse_value(s: &str) -> Result<Self, ValueError> {
        let s = s.trim();
        if s.len() != 64 || !s.is_ascii() {
            return Err(ValueError::new(format!("{s:?} is not a 64-digit SHA-256")));
        }
        let mut out = [0u8; 32];
        for (i, o) in out.iter_mut().enumerate() {
            *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16)
                .map_err(|_| ValueError::new(format!("{s:?} is not a hexadecimal SHA-256")))?;
        }
        Ok(Sha256(out))
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// `languageTagType`: a BCP 47 language tag such as `en-US`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LanguageTag(pub String);

impl LanguageTag {
    /// Primary language subtag, lowercased.
    pub fn primary(&self) -> String {
        self.0.split('-').next().unwrap_or("").to_ascii_lowercase()
    }
}

impl ParseValue for LanguageTag {
    fn parse_value(s: &str) -> Result<Self, ValueError> {
        Ok(LanguageTag(s.trim().to_string()))
    }
}

impl fmt::Display for LanguageTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

serialize_display!(Length, Rgba, Color, PaintRef, Paint, Fps, Timecode, Point2, Aspect, Sha256, LanguageTag);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        assert_eq!(Length::parse_value("50%").unwrap().resolve(200.0, 1920.0, 1080.0), 100.0);
        assert_eq!(Length::parse_value("10vmin").unwrap().resolve(0.0, 1920.0, 1080.0), 108.0);
        assert_eq!(Length::parse_value(" -12.5 ").unwrap(), Length::px(-12.5));
        assert_eq!(Length::parse_value("1e2").unwrap(), Length::px(100.0));
    }

    #[test]
    fn colours() {
        assert_eq!(
            Color::parse_value("#ff000080").unwrap(),
            Color::Rgba(Rgba { r: 1.0, g: 0.0, b: 0.0, a: 128.0 / 255.0 })
        );
        assert_eq!(Color::parse_value("var(--brand)").unwrap(), Color::Token("brand".into()));
        assert_eq!(Color::parse_value("1,0.5,0").unwrap().to_string(), "1,0.5,0,1");
        assert_eq!(Paint::parse_value("url(#g1)").unwrap(), Paint::Ref(PaintRef("g1".into())));
        assert_eq!(Rgba::BLACK.to_string(), "#000000ff");
    }

    #[test]
    fn frame_rates() {
        let ntsc = Fps::parse_value("30000/1001").unwrap();
        assert_eq!(ntsc.nominal(), 30);
        assert_eq!(ntsc.frame_count(10.01), 300);
        let f30 = Fps::parse_value("30").unwrap();
        assert_eq!(f30.frame_count(5.0), 150);
        assert_eq!(f30.frame_count(5.01), 151);
        assert_eq!(f30.frame_at(0.1), 3);
        assert_eq!(f30.frame_time(45), 1.5);
        assert!(Fps::parse_value("99999999999").is_err());
    }

    #[test]
    fn timecodes() {
        let ntsc = Fps::new(30000, 1001).unwrap();
        let tc = Timecode::parse_value("00:01:00;02").unwrap();
        assert_eq!(tc.to_frame(ntsc).unwrap(), 1800);
        assert!(Timecode::parse_value("00:01:00;00").unwrap().to_frame(ntsc).is_err());
        assert_eq!(Timecode::parse_value("00:10:00;00").unwrap().to_frame(ntsc).unwrap(), 17982);
        assert_eq!(Timecode::parse_value("01:00:00:00").unwrap().to_frame(Fps::new(25, 1).unwrap()).unwrap(), 90000);
    }
}
