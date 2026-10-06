//! The document's font policy (SREP 21): `project/@fontPolicy="pinned"` makes text layout a function of the
//! document. Every face comes from a font asset pinned by `sha256`, and no host font is consulted.

use sr_model::element::Element as _;
use sr_model::model::{AssetsChild, Scene};

/// Whether `scene` pins its fonts (`project/@fontPolicy="pinned"`; the default, `system`, does not).
pub fn pinned(scene: &Scene) -> bool {
    scene.project.get_attr("fontPolicy").is_some_and(|v| v.to_string() == "pinned")
}

/// A font asset as a face to pin: id, `src`, collection index, family, weight, italic.
#[derive(Debug, Clone, PartialEq)]
pub struct FontAsset {
    /// `font/@id`.
    pub id: String,
    /// `font/@src`.
    pub src: String,
    /// `font/@collectionIndex`.
    pub index: u32,
    /// `font/@family`.
    pub family: String,
    /// `font/@weight`.
    pub weight: u16,
    /// `font/@fontStyle` is italic or oblique.
    pub italic: bool,
}

/// The font assets of `scene`, in document order.
pub fn font_assets(scene: &Scene) -> Vec<FontAsset> {
    scene
        .assets
        .iter()
        .flat_map(|a| a.children.iter())
        .filter_map(|c| match c {
            AssetsChild::Font(f) => Some(FontAsset {
                id: f.id.clone(),
                src: f.src.clone(),
                index: f.collection_index as u32,
                family: f.family.clone(),
                weight: match f.get_attr("weight") {
                    Some(sr_model::element::AttrValue::Num(w)) => w.clamp(1.0, 1000.0) as u16,
                    _ => 400,
                },
                italic: f.get_attr("fontStyle").is_some_and(|s| s.to_string() != "normal"),
            }),
            _ => None,
        })
        .collect()
}

impl crate::Program {
    /// Whether the document pins its fonts (SREP 21).
    pub fn fonts_pinned(&self) -> bool {
        pinned(&self.scene)
    }
}
