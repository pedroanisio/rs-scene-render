//! Frame fingerprints, so `render --changed-only` and `watch` render only the frames an edit
//! changed.
//!
//! A frame's fingerprint hashes what decides its pixels: the evaluated frame (every node's
//! animated values, transforms and timing), the source text of every element it draws (its
//! static attributes), the document outside the composition (assets, paints, effects,
//! symbols, project, colour management), the files those name, and the render settings. When
//! the scene uses motion blur or time effects a frame also shows other times, so the whole
//! composition joins the shared part and any edit renders every frame again.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sr_eval::FrameGraph;
use sr_model::element::Element;

/// The source text of the element starting at byte `offset` of `text` (at its `<`): up to the
/// end of its start tag when it closes itself, else to the end of its matching end tag.
pub fn element_span(text: &str, offset: usize) -> Option<&str> {
    let b = text.as_bytes();
    if b.get(offset) != Some(&b'<') {
        return None;
    }
    let mut depth = 0usize;
    let mut i = offset;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->")? + 3;
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            i += rest.find("]]>")? + 3;
            continue;
        }
        if rest.starts_with("<?") {
            i += rest.find("?>")? + 2;
            continue;
        }
        let closing = rest.starts_with("</");
        // the end of this tag, skipping quoted attribute values
        let mut j = i + 1;
        let mut quote = None;
        while j < b.len() {
            match (quote, b[j]) {
                (None, b'"') | (None, b'\'') => quote = Some(b[j]),
                (Some(q), c) if c == q => quote = None,
                (None, b'>') => break,
                _ => {}
            }
            j += 1;
        }
        if j >= b.len() {
            return None;
        }
        let self_closing = b[j - 1] == b'/';
        if closing {
            depth = depth.checked_sub(1)?;
        } else if !self_closing {
            depth += 1;
        }
        i = j + 1;
        if depth == 0 {
            return Some(&text[offset..i]);
        }
    }
    None
}

/// Byte range of the composition in `text`, when there is one.
fn composition_range(text: &str) -> Option<(usize, usize)> {
    let start = text.find("<composition")?;
    let span = element_span(text, start)?;
    Some((start, start + span.len()))
}

fn fnv(bytes: &[u8], mut h: u64) -> u64 {
    for &c in bytes {
        h ^= c as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

const SEED: u64 = 0xcbf2_9ce4_8422_2325;

/// Effects that make a frame show the scene at other times.
const TIME_EFFECTS: &[&str] = &["posterize-time", "echo", "pixel-motion-blur"];

/// The files document `doc` at `path` names (sources, caches, environments).
pub fn files(path: &Path, doc: &sr_model::Document) -> Vec<PathBuf> {
    let base = path.parent().unwrap_or(Path::new("."));
    let mut files = Vec::new();
    sr_model::element::walk(&doc.scene, &mut |e| {
        for attr in ["src", "cache", "environment"] {
            if let Some(sr_model::element::AttrValue::Str(s)) = e.get_attr(attr) {
                files.push(base.join(s));
            }
        }
    });
    files.sort();
    files.dedup();
    files
}

/// A file's size and modification time.
pub type Stamp = (u64, Option<std::time::SystemTime>);

/// A file's size and modification time, when it exists.
pub fn stamp(f: &Path) -> Option<Stamp> {
    std::fs::metadata(f).ok().map(|m| (m.len(), m.modified().ok()))
}

/// Hash of what a renderer is built from (the project, colour management, styles and media dependencies): while it
/// stays the same, a renderer made for an earlier version of the document can render this one.
pub fn setup_key(text: &str, path: &Path, doc: &sr_model::Document) -> u64 {
    let mut h = SEED;
    for tag in ["<project", "<colorManagement", "<styles"] {
        if let Some(at) = text.find(tag) {
            h = fnv(element_span(text, at).unwrap_or("").as_bytes(), h);
        }
    }
    for f in files(path, doc) {
        h = fnv(format!("{}:{:?}", f.display(), stamp(&f)).as_bytes(), h);
    }
    h
}

/// The fingerprints of frames rendered with `--changed-only`, by output file, kept beside them.
pub struct Sidecar {
    path: PathBuf,
    pub frames: BTreeMap<String, String>,
}

impl Sidecar {
    /// The sidecar of outputs written into `dir`.
    pub fn load(dir: &Path) -> Sidecar {
        let path = dir.join(".scene-render-frames.json");
        let frames = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| serde_json::from_value(v["frames"].clone()).ok())
            .unwrap_or_default();
        Sidecar { path, frames }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let doc = serde_json::json!({ "version": 1, "frames": self.frames });
        std::fs::write(&self.path, serde_json::to_string_pretty(&doc).expect("serialisable"))
    }
}

/// Fingerprints of frames of one document.
pub struct Fingerprints {
    /// What every frame depends on.
    shared: u64,
    /// Why frames are not told apart (any edit to the composition changes them all).
    pub whole: Option<String>,
}

impl Fingerprints {
    /// Fingerprints for `doc`, whose source is `text` at `path`, rendered with `settings`
    /// (quality, bit depth, variant and anything else that changes pixels).
    pub fn new(text: &str, path: &Path, doc: &sr_model::Document, settings: &str) -> Fingerprints {
        let mut shared = fnv(settings.as_bytes(), SEED);
        shared = fnv(env!("CARGO_PKG_VERSION").as_bytes(), shared);
        // Invalidate fingerprints produced before transitions/diagnostics were tracked.
        shared = fnv(b"frame-fingerprint-v2", shared);
        // the document outside the composition
        let (a, b) = composition_range(text).unwrap_or((text.len(), text.len()));
        shared = fnv(&text.as_bytes()[..a], shared);
        shared = fnv(&text.as_bytes()[b..], shared);
        // files the document names: size and modification time
        for f in files(path, doc) {
            shared = fnv(format!("{}:{:?}", f.display(), stamp(&f)).as_bytes(), shared);
        }
        let whole = if doc.scene.project.motion_blur {
            Some("the scene uses motion blur, so frames show other times".to_string())
        } else if doc
            .scene
            .effects
            .as_ref()
            .is_some_and(|fx| fx.effects.iter().any(|e| TIME_EFFECTS.contains(&e.r#type.as_str())))
        {
            Some("the scene uses time effects, so frames show other times".to_string())
        } else if text.contains("<include") {
            Some("the scene includes other documents".to_string())
        } else {
            None
        };
        if whole.is_some() {
            shared = fnv(&text.as_bytes()[a..b], shared);
        }
        Fingerprints { shared, whole }
    }

    /// The fingerprint of an evaluated frame.
    pub fn frame(&self, text: &str, g: &FrameGraph) -> u64 {
        let mut h = self.shared;
        for n in &g.nodes {
            // what the evaluation decided, and the element's own attributes
            h = fnv(serde_json::to_string(n).unwrap_or_default().as_bytes(), h);
            let off = n.elem.loc().offset as usize;
            h = fnv(element_span(text, off).unwrap_or("").as_bytes(), h);
        }
        for tr in &g.transitions {
            h = fnv(serde_json::to_string(tr).unwrap_or_default().as_bytes(), h);
            if let Some(elem) = &tr.elem {
                h = fnv(element_span(text, elem.loc().offset as usize).unwrap_or("").as_bytes(), h);
            }
        }
        fnv(serde_json::to_string(&g.elements).unwrap_or_default().as_bytes(), h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_end_at_the_matching_end_tag() {
        let t = r#"<a x="1"><b/><!-- <c> --><a y=">">t</a></a><d/>"#;
        assert_eq!(element_span(t, 0), Some(r#"<a x="1"><b/><!-- <c> --><a y=">">t</a></a>"#));
        assert_eq!(element_span(t, 9), Some("<b/>"));
        let inner = t.find(r#"<a y"#).unwrap();
        assert_eq!(element_span(t, inner), Some(r#"<a y=">">t</a>"#));
        assert_eq!(element_span(t, t.find("<d").unwrap()), Some("<d/>"));
        assert_eq!(element_span(t, 1), None, "not at a tag");
    }

    #[test]
    fn element_offsets_point_at_their_tags() {
        let xml = r##"<scene version="1.1"><project width="8" height="8" fps="1" duration="1"/>
<composition><shape id="s" shape="rect" width="2" height="2" fill="#FF0000"><animate property="x"><key time="0" value="0"/><key time="1" value="4"/></animate></shape></composition></scene>"##;
        let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap();
        let ev = sr_eval::Evaluator::new(&doc, &sr_eval::EvalOptions::default()).unwrap();
        let g = ev.evaluate(0.0);
        let n = g.nodes.iter().find(|n| &*n.id == "s").unwrap();
        let span = element_span(xml, n.elem.loc().offset as usize).unwrap();
        assert!(span.starts_with("<shape id=\"s\"") && span.ends_with("</shape>"), "{span}");
    }
}
