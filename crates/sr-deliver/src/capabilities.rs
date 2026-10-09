//! The engine's capability manifest (SREP 22): `capabilities.json` at the root of the repository and in the
//! distribution (`scene-render capabilities` prints it), format `scene-render-capabilities/1`. It lists every
//! construct of the scene format this engine does not draw exactly, with its status; a construct it does not list is
//! claimed exact. With a render report, each `approximate` or `reported` construct a document uses is reported once,
//! as `SUP-APPROX` or `SUP-REPORTED` (warning), naming the construct.

use serde::{Deserialize, Serialize};
use sr_model::Severity;

use crate::render_report::{code, Finding};

/// The manifest this engine publishes.
pub const MANIFEST: &str = include_str!("../../../capabilities.json");

/// The value of the manifest's `format` field.
pub const FORMAT: &str = "scene-render-capabilities/1";

/// A capability manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// [`FORMAT`].
    pub format: String,
    /// The engine.
    pub engine: Engine,
    /// The version of the scene schema the manifest covers.
    pub schema: String,
    /// The constructs not drawn exactly.
    pub entries: Vec<Entry>,
}

/// The engine section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Engine {
    /// The engine's name.
    pub name: String,
    /// The engine's version.
    pub version: String,
}

/// How a construct is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Passes the kit's cases for it.
    Exact,
    /// Drawn, measurably different.
    Approximate,
    /// Recognised, not drawn, reported as `SUP-REPORTED`.
    Reported,
    /// Not recognised.
    Unsupported,
}

/// One construct and its status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// `element`, `element/@attribute` or `element/@attribute=value`.
    pub construct: String,
    /// Its status.
    pub status: Status,
    /// The specification definition the status refers to, when there is one.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub definition: Option<String>,
    /// What is different, or when.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub note: Option<String>,
}

/// A construct, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Construct<'a> {
    /// The element name.
    pub element: &'a str,
    /// The attribute, when the construct names one.
    pub attribute: Option<&'a str>,
    /// The enumeration value, when the construct names one.
    pub value: Option<&'a str>,
}

impl<'a> Construct<'a> {
    /// Parses `element`, `element/@attribute` or `element/@attribute=value`; `None` for any other form.
    pub fn parse(s: &'a str) -> Option<Construct<'a>> {
        let (element, rest) = match s.split_once("/@") {
            Some((e, r)) => (e, Some(r)),
            None => (s, None),
        };
        let name_ok = |n: &str| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !name_ok(element) {
            return None;
        }
        let (attribute, value) = match rest {
            None => (None, None),
            Some(r) => match r.split_once('=') {
                Some((a, v)) if name_ok(a) && !v.is_empty() => (Some(a), Some(v)),
                Some(_) => return None,
                None if name_ok(r) => (Some(r), None),
                None => return None,
            },
        };
        Some(Construct { element, attribute, value })
    }

    /// The first element of `doc` that uses the construct: an element of that name, carrying the attribute (with
    /// that value) when the construct names one.
    pub fn first_use<'d, 'i>(&self, doc: &'d roxmltree::Document<'i>) -> Option<roxmltree::Node<'d, 'i>> {
        doc.root_element().descendants().find(|n| {
            n.is_element()
                && n.tag_name().name() == self.element
                && match (self.attribute, self.value) {
                    (None, _) => true,
                    (Some(a), None) => n.has_attribute(a),
                    (Some(a), Some(v)) => n.attribute(a).is_some_and(|x| x.trim() == v),
                }
        })
    }
}

/// The engine's manifest, parsed.
pub fn manifest() -> Manifest {
    serde_json::from_str(MANIFEST).expect("capabilities.json is checked by the test suite")
}

/// `SUP-APPROX` and `SUP-REPORTED` for every `approximate` or `reported` construct of `manifest` that the document
/// `xml` uses, once each, at its first use.
pub fn findings(manifest: &Manifest, xml: &str) -> Vec<Finding> {
    let Ok(doc) = roxmltree::Document::parse(xml) else { return Vec::new() };
    let mut out = Vec::new();
    for e in &manifest.entries {
        let code = match e.status {
            Status::Approximate => code::SUP_APPROX,
            Status::Reported => code::SUP_REPORTED,
            Status::Exact | Status::Unsupported => continue,
        };
        let Some(c) = Construct::parse(&e.construct) else { continue };
        let Some(n) = c.first_use(&doc) else { continue };
        let how = match e.status {
            Status::Approximate => "is drawn approximately",
            _ => "is not drawn",
        };
        let note = e.note.as_deref().map(|n| format!(": {n}")).unwrap_or_default();
        let mut f = Finding::scene(
            code,
            Severity::Warning,
            format!("{} {how} by {} {}{note}", e.construct, manifest.engine.name, manifest.engine.version),
        );
        f.path = sr_model::diag::element_path(n);
        f.at = Some(crate::render_report::At { offset: Some(n.range().start as u32), id: None });
        out.push(f);
    }
    out
}
