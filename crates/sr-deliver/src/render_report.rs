//! Render reports (SREP 18): one JSON object per output, `scene-render-report/1`, saying what happened to the
//! picture: the document's validation findings, what the render measured (accessibility, safe area, text fit) and
//! why it stopped, when it did.
//!
//! The report is deterministic: no timestamps, host names or paths outside the document's folder; findings are
//! sorted by time (findings without one first), then code, then path. Findings never change pixels.

use std::path::Path;

use serde::Serialize;
use sr_model::{Diagnostic, Severity};

use crate::DeliverError;

/// The value of the report's `format` field.
pub const FORMAT: &str = "scene-render-report/1";

/// This engine's name in reports, and the namespace of its own finding codes (`X-rs-scene-render-…`).
pub const ENGINE: &str = "rs-scene-render";

/// Finding codes of the registry (SREP 18, Specification 4, with the codes other SREPs add as they land).
pub mod code {
    /// The document fails the XSD (or is not well-formed XML).
    pub const XSD: &str = "XSD";
    /// Prefix of a failed Schematron assert: `SCH-C3`.
    pub const SCH_PREFIX: &str = "SCH-";
    /// An asset file is missing or fails its `sha256`/`cacheSha256`.
    pub const ASSET_MISSING: &str = "ASSET-MISSING";
    /// Text does not fit its box at the size drawn.
    pub const TXT_FIT: &str = "TXT-FIT";
    /// Characters were dropped by `maxLines` or `overflow`.
    pub const TXT_CUT: &str = "TXT-CUT";
    /// The photosensitivity check (`flashCheck`).
    pub const ACC_FLASH: &str = "ACC-FLASH";
    /// `requireCaptions="true"` and an output without captions.
    pub const ACC_CAPTIONS: &str = "ACC-CAPTIONS";
    /// The text contrast check (`contrastCheck`).
    pub const LEG_CONTRAST: &str = "LEG-CONTRAST";
    /// Reading speed above the limit (SREP 19).
    pub const LEG_SPEED: &str = "LEG-SPEED";
    /// Shown for less than `minDisplayTime` (SREP 19).
    pub const LEG_SHORT: &str = "LEG-SHORT";
    /// Drawn smaller than `minTextSize` (SREP 19).
    pub const LEG_SIZE: &str = "LEG-SIZE";
    /// The safe-area check (`safeArea/@enforce`).
    pub const SAFE_AREA: &str = "SAFE-AREA";
    /// Prefix of an inert-attribute finding: `INERT-I2`.
    pub const INERT_PREFIX: &str = "INERT-";

    /// An engine-specific code: `X-rs-scene-render-<code>`.
    pub fn engine(code: &str) -> String {
        format!("X-{}-{code}", super::ENGINE)
    }
}

/// One finding of a report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    /// A registry code, or `X-rs-scene-render-<code>`.
    pub code: String,
    /// `error`, `warning` or `info`.
    pub severity: Severity,
    /// XPath of the element concerned, `/scene` when nothing narrower applies.
    pub path: String,
    /// The element's `id`, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// `[start, end]` in composition seconds, when the finding concerns a time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<[f64; 2]>,
    /// The quantity found, when a check measured one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured: Option<f64>,
    /// The threshold it was compared with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
    /// Their unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// For people; its wording is not part of the contract.
    pub message: String,
    /// Where the finding was raised, to place it when `path` is not yet an XPath: the source offset of the element
    /// and an effective node id (resolved by [`ReportWriter`]).
    #[serde(skip)]
    pub at: Option<At>,
}

/// Where a finding was raised, before it is placed in the document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct At {
    /// Byte offset of the element in the source, when known.
    pub offset: Option<u32>,
    /// The node's effective id (`instance/child` inside instances), when known.
    pub id: Option<String>,
}

impl Finding {
    /// A finding about the node with effective id `id`, placed later by [`ReportWriter`].
    pub fn node(code: impl Into<String>, severity: Severity, id: &str, message: impl Into<String>) -> Finding {
        Finding {
            code: code.into(),
            severity,
            path: String::new(),
            node: None,
            time: None,
            measured: None,
            limit: None,
            unit: None,
            message: message.into(),
            at: Some(At { offset: None, id: Some(id.to_string()) }),
        }
    }

    /// A finding about the whole document (`/scene`).
    pub fn scene(code: impl Into<String>, severity: Severity, message: impl Into<String>) -> Finding {
        Finding {
            code: code.into(),
            severity,
            path: "/scene".into(),
            node: None,
            time: None,
            measured: None,
            limit: None,
            unit: None,
            message: message.into(),
            at: None,
        }
    }

    /// The finding at `[start, end]` composition seconds.
    pub fn at_time(mut self, start: f64, end: f64) -> Finding {
        self.time = Some([start, end]);
        self
    }

    /// The finding with a measured quantity, the limit it was compared with, and their unit.
    pub fn measuring(mut self, measured: f64, limit: Option<f64>, unit: &str) -> Finding {
        self.measured = Some(measured);
        self.limit = limit;
        self.unit = Some(unit.to_string());
        self
    }

    /// The finding of a diagnostic: its code mapped to the registry ([`report_code`]), its severity, and its place.
    pub fn of_diagnostic(d: &Diagnostic) -> Finding {
        let xpath = d.path.starts_with('/');
        Finding {
            code: report_code(d),
            severity: d.severity,
            path: if xpath { d.path.clone() } else { String::new() },
            node: None,
            time: None,
            measured: None,
            limit: None,
            unit: None,
            message: d.message.clone(),
            at: Some(At {
                offset: (d.loc.line > 0 || d.loc.offset > 0).then_some(d.loc.offset),
                id: (!xpath && !d.path.is_empty()).then(|| d.path.clone()),
            }),
        }
    }
}

/// The registry code of a diagnostic: structure and well-formedness errors are `XSD`, Schematron asserts `SCH-<id>`,
/// missing or mismatched files `ASSET-MISSING`, safe-area findings `SAFE-AREA`, inert attributes keep their
/// `INERT-<rule>`; every other code is this engine's own, `X-rs-scene-render-<code>`.
pub fn report_code(d: &Diagnostic) -> String {
    let c = d.code.as_str();
    let structural = c == "XML" || (c.len() == 3 && c.starts_with('S') && c[1..].bytes().all(|b| b.is_ascii_digit()));
    if structural {
        code::XSD.into()
    } else if c.starts_with(code::INERT_PREFIX) {
        c.into()
    } else if c == "A01" || c == "A02" || (c == "A04" && d.is_error()) {
        code::ASSET_MISSING.into()
    } else if c == "SA01" {
        code::SAFE_AREA.into()
    } else if sr_model::codes::SCH_ASSERTS.iter().any(|a| a.0 == c) {
        format!("{}{c}", code::SCH_PREFIX)
    } else {
        code::engine(c)
    }
}

/// The findings of a delivery error: the diagnostics of an invalid document or program, a missing caption track,
/// or the engine's own account of what stopped the render. An accessibility check set to `error`, and a safe area
/// enforced as `error` (`SA01`), are already among the delivery's findings, with their nodes and times, and add
/// nothing.
pub fn error_findings(e: &DeliverError) -> Vec<Finding> {
    match e {
        DeliverError::Document(r) => {
            r.diagnostics.iter().filter(|d| d.code != "SA01").map(Finding::of_diagnostic).collect()
        }
        DeliverError::Accessibility(msg) if msg.starts_with("requireCaptions") => {
            vec![Finding::scene(code::ACC_CAPTIONS, Severity::Error, msg.clone())]
        }
        DeliverError::Accessibility(_) => Vec::new(),
        DeliverError::Render { time, message } => {
            vec![Finding::scene(code::engine("RENDER"), Severity::Error, message.clone()).at_time(*time, *time)]
        }
        other => vec![Finding::scene(code::engine("DELIVERY"), Severity::Error, other.to_string())],
    }
}

/// The engine section.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Engine {
    /// `rs-scene-render`.
    pub name: String,
    /// The engine's version.
    pub version: String,
}

/// The scene section.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SceneInfo {
    /// SHA-256 of the document's bytes as read, lowercase hex.
    pub sha256: String,
    /// The document's `scene/@version`, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// The output section: the output as rendered.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutputInfo {
    /// `output/@id`, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// Start of the rendered range, seconds.
    pub start: f64,
    /// End of the rendered range, seconds.
    pub end: f64,
}

/// A render report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RenderReport {
    /// [`FORMAT`].
    pub format: &'static str,
    /// The engine.
    pub engine: Engine,
    /// The document.
    pub scene: SceneInfo,
    /// The output.
    pub output: OutputInfo,
    /// The findings, sorted.
    pub findings: Vec<Finding>,
}

/// Builds and writes reports for one document: places findings in the document (XPath, node id), removes paths
/// outside its folder from messages, sorts and deduplicates.
pub struct ReportWriter<'a> {
    bytes: &'a [u8],
    folder: std::path::PathBuf,
    xml: Option<roxmltree::Document<'a>>,
}

impl<'a> ReportWriter<'a> {
    /// A writer for the document `bytes`, read from a file in `folder`.
    pub fn new(bytes: &'a [u8], folder: &Path) -> ReportWriter<'a> {
        let folder = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
        let xml = std::str::from_utf8(bytes).ok().and_then(|t| roxmltree::Document::parse(t).ok());
        ReportWriter { bytes, folder, xml }
    }

    /// The scene section: the SHA-256 of the bytes and the declared version.
    pub fn scene(&self) -> SceneInfo {
        use sha2::Digest;
        let sha256 = sha2::Sha256::digest(self.bytes).iter().map(|b| format!("{b:02x}")).collect();
        let version = self.xml.as_ref().and_then(|x| x.root_element().attribute("version")).map(str::to_string);
        SceneInfo { sha256, version }
    }

    /// The `output` elements of the document as written, for a document that could not be loaded: each one's id
    /// and its `report` attribute, in document order.
    pub fn raw_outputs(&self) -> Vec<RawOutput> {
        let Some(x) = &self.xml else { return Vec::new() };
        let root = x.root_element();
        let project = root.children().find(|c| c.has_tag_name("project"));
        let num = |n: Option<roxmltree::Node>, a: &str| {
            n.and_then(|n| n.attribute(a)).and_then(|v| v.trim().parse::<f64>().ok())
        };
        root.children()
            .filter(|c| c.has_tag_name("output"))
            .map(|o| {
                let width = num(Some(o), "width").or(num(project, "width")).unwrap_or(0.0);
                let height = num(Some(o), "height").or(num(project, "height")).unwrap_or(0.0);
                RawOutput {
                    report: o.attribute("report").map(str::to_string),
                    info: OutputInfo {
                        id: o.attribute("id").map(str::to_string),
                        width: width.max(0.0) as u32,
                        height: height.max(0.0) as u32,
                        start: num(Some(o), "start").unwrap_or(0.0),
                        end: num(Some(o), "end").or(num(project, "duration")).unwrap_or(0.0),
                    },
                }
            })
            .collect()
    }

    /// The report of `output` with `findings`: placed, cleaned, sorted and deduplicated.
    pub fn report(&self, output: OutputInfo, findings: Vec<Finding>) -> RenderReport {
        let mut findings: Vec<Finding> = findings.into_iter().map(|f| self.place(f)).collect();
        findings.sort_by(order);
        findings.dedup();
        RenderReport {
            format: FORMAT,
            engine: Engine { name: ENGINE.into(), version: env!("CARGO_PKG_VERSION").into() },
            scene: self.scene(),
            output,
            findings,
        }
    }

    /// Writes `report` to `path` as one UTF-8 JSON object (and a final newline), creating its directory.
    pub fn write(report: &RenderReport, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let mut text = serde_json::to_string_pretty(report).map_err(std::io::Error::other)?;
        text.push('\n');
        std::fs::write(path, text)
    }

    fn place(&self, mut f: Finding) -> Finding {
        f.message = self.clean(&f.message);
        let at = f.at.take().unwrap_or_default();
        let Some(x) = &self.xml else {
            if f.path.is_empty() {
                f.path = "/scene".into();
            }
            return f;
        };
        let elements = || x.root_element().descendants().filter(roxmltree::Node::is_element);
        // the innermost element whose source contains the offset: an element's own start, or one of its attributes
        let by_offset = at.offset.and_then(|o| elements().rfind(|n| n.range().contains(&(o as usize))));
        let element = if f.path.starts_with('/') {
            by_offset.filter(|n| sr_model::diag::element_path(*n) == f.path)
        } else {
            // the element where the finding was raised (a part such as a mask has no id of its own, and names its
            // node), else the element an effective id names directly or (inside an instance) ends with
            let id = at.id.as_deref();
            let own = id.map(|i| i.rsplit('/').next().unwrap_or(i));
            by_offset
                .filter(|n| n.attribute("id").is_none_or(|i| Some(i) == own))
                .or_else(|| id.and_then(|i| elements().find(|n| n.attribute("id") == Some(i))))
                .or_else(|| own.and_then(|o| elements().find(|n| n.attribute("id") == Some(o))))
        };
        match element {
            Some(n) => {
                if !f.path.starts_with('/') {
                    f.path = sr_model::diag::element_path(n);
                }
                f.node = n.attribute("id").map(str::to_string);
            }
            None if !f.path.starts_with('/') => f.path = "/scene".into(),
            None => {}
        }
        f
    }

    /// `msg` without paths outside the document's folder: paths inside it become relative, temporary files are
    /// named as such, and other absolute paths keep only their file name.
    fn clean(&self, msg: &str) -> String {
        let folder = format!("{}/", self.folder.display());
        let msg = msg.replace(&folder, "");
        let temp = std::env::temp_dir();
        msg.split_inclusive(|c: char| c.is_whitespace())
            .map(|word| {
                let trimmed = word.trim_end().trim_matches(|c: char| "\"'`(),;:[]{}".contains(c));
                if trimmed.is_empty() || !trimmed.starts_with('/') {
                    return word.to_string();
                }
                let p = Path::new(trimmed);
                let on_disk = p.exists() || p.parent().is_some_and(|d| d.components().count() > 1 && d.exists());
                if !on_disk {
                    return word.to_string();
                }
                let replacement = if p.starts_with(&temp) {
                    "<temporary file>".to_string()
                } else {
                    format!("…/{}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
                };
                word.replacen(trimmed, &replacement, 1)
            })
            .collect()
    }
}

/// An `output` element of a document that could not be loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct RawOutput {
    /// `output/@report`.
    pub report: Option<String>,
    /// What can be read of the output.
    pub info: OutputInfo,
}

/// Findings without a time first, then by start time, code and path (and the rest, so that equal keys still
/// sort the same way every time).
fn order(a: &Finding, b: &Finding) -> std::cmp::Ordering {
    let t = |f: &Finding| f.time.map(|t| t[0]);
    let key = |f: &Finding| (f.code.clone(), f.path.clone(), f.node.clone(), f.message.clone());
    match (t(a), t(b)) {
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(x), Some(y)) => x.total_cmp(&y),
        (None, None) => std::cmp::Ordering::Equal,
    }
    .then_with(|| key(a).cmp(&key(b)))
    .then_with(|| a.severity.cmp(&b.severity))
    .then_with(|| {
        let n = |f: &Finding| (f.time.map(|t| t[1]), f.measured, f.limit);
        n(a).partial_cmp(&n(b)).unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// Where the report of an output goes: `output/@report`, resolved against the document's folder.
pub fn report_path(folder: &Path, output: &sr_model::model::Output) -> Option<std::path::PathBuf> {
    use sr_model::element::Element as _;
    let r = output.get_attr("report")?.to_string();
    Some(crate::pipeline_resolve(folder, &r))
}
