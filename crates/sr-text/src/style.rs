//! Resolved character styles and paragraph options.

use sr_vector::Paint;

/// Text case transforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transform {
    #[default]
    None,
    Upper,
    Lower,
    Capitalize,
    SmallCaps,
}

/// Decoration lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Decoration {
    #[default]
    None,
    Underline,
    LineThrough,
    Overline,
}

/// Where a stroke sits relative to the outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StrokePos {
    #[default]
    Center,
    Inside,
    Outside,
}

/// A fully resolved character style.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    /// Families in preference order (`font`, then `fallback`); generic names allowed.
    pub families: Vec<String>,
    /// A font file to use first (from `fontFile` or `fontAsset`) and its collection index.
    pub file: Option<(std::path::PathBuf, u32)>,
    pub weight: u16,
    pub italic: bool,
    /// Width as a fraction of normal (1 = normal).
    pub stretch: f64,
    /// Variation axis settings.
    pub variations: Vec<([u8; 4], f32)>,
    /// OpenType features (`liga=0`, `smcp`, …).
    pub features: Vec<String>,
    pub size: f64,
    pub color: Option<Paint>,
    pub stroke: Option<(Paint, f64, StrokePos)>,
    /// Shadow: straight RGBA (document literal), offset x, y, blur.
    pub shadow: Option<([f64; 4], f64, f64, f64)>,
    pub baseline_shift: f64,
    /// Tracking in thousandths of an em.
    pub tracking: f64,
    pub transform: Transform,
    pub decoration: Decoration,
    pub highlight: Option<Paint>,
    /// Line height multiple, when the style sets one.
    pub line_height: Option<f64>,
}

impl Default for Style {
    fn default() -> Style {
        Style {
            families: vec!["sans-serif".into()],
            file: None,
            weight: 400,
            italic: false,
            stretch: 1.0,
            variations: Vec::new(),
            features: Vec::new(),
            size: 32.0,
            color: Some(Paint::Solid { rgba: [1.0; 4], srgb: false }),
            stroke: None,
            shadow: None,
            baseline_shift: 0.0,
            tracking: 0.0,
            transform: Transform::None,
            decoration: Decoration::None,
            highlight: None,
            line_height: None,
        }
    }
}

/// Parses `wght=700, wdth 80` style variation lists.
pub fn parse_variations(s: &str) -> Vec<([u8; 4], f32)> {
    s.split(',')
        .filter_map(|part| {
            let part = part.trim().trim_matches('"').trim_matches('\'');
            let (tag, val) = part.split_once(|c: char| c == '=' || c.is_whitespace())?;
            let tag = tag.trim().trim_matches('"').trim_matches('\'');
            let b = tag.as_bytes();
            (b.len() == 4)
                .then(|| ([b[0], b[1], b[2], b[3]], val.trim().parse().ok()))?
                .1
                .map(|v| ([b[0], b[1], b[2], b[3]], v))
        })
        .collect()
}
