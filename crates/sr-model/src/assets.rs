//! Verification of the files a document references.
//!
//! Every attribute of type `xs:anyURI` that names an input (all of them
//! except `output/@path`, `still/@path` and `destination/@uri`) must resolve
//! to an existing file. Where the document declares a digest — `@sha256` for
//! `@src`, `@cacheSha256` for `@cache` — the file's SHA-256 must match it,
//! which is what makes renders of generated media and transcriptions
//! reproducible. Image sequences are expanded and every frame is checked.
//!
//! Codes: `A01` missing file, `A02` digest mismatch, `A03` remote input not
//! verified offline, `A04` missing sequence frames, `A05` unreadable file,
//! `A06` physics cache absent (recomputed at render time).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rayon::prelude::*;
use roxmltree::{Document, Node};
use sha2::{Digest, Sha256};

use crate::diag::{element_path, Diagnostic, Loc, Severity};
use crate::xsd::{root_builtin, Builtin, COMPLEX_TYPES};

/// Frames beyond this count are not checked individually.
const MAX_SEQUENCE_FRAMES: i64 = 1_000_000;

/// Names of input-file attributes, derived from the schema: every `xs:anyURI`
/// attribute except the output-side `path` and `uri`.
pub fn input_uri_attributes() -> &'static BTreeSet<&'static str> {
    static S: OnceLock<BTreeSet<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        COMPLEX_TYPES
            .iter()
            .flat_map(|c| c.attrs.iter())
            .filter(|a| root_builtin(a.ty) == Some(Builtin::AnyUri))
            .map(|a| a.name)
            .filter(|n| !matches!(*n, "path" | "uri"))
            .collect()
    })
}

/// Where a URI points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// A local file.
    Local(PathBuf),
    /// A remote resource with the given scheme.
    Remote(String),
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (
                b.get(i + 1).and_then(|c| (*c as char).to_digit(16)),
                b.get(i + 2).and_then(|c| (*c as char).to_digit(16)),
            ) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolves a URI reference against the document directory.
pub fn resolve(uri: &str, base_dir: &Path) -> Resolved {
    let uri = uri.trim();
    if let Some(rest) = uri.strip_prefix("file://") {
        let p = rest.strip_prefix("localhost").unwrap_or(rest);
        return Resolved::Local(PathBuf::from(percent_decode(p)));
    }
    if let Some((scheme, _)) = uri.split_once(':') {
        let is_scheme = scheme.len() >= 2
            && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        if is_scheme {
            if scheme.eq_ignore_ascii_case("file") {
                return Resolved::Local(PathBuf::from(percent_decode(&uri[5..])));
            }
            return Resolved::Remote(scheme.to_ascii_lowercase());
        }
    }
    let path = PathBuf::from(percent_decode(uri));
    if path.is_absolute() {
        Resolved::Local(path)
    } else {
        Resolved::Local(base_dir.join(path))
    }
}

/// SHA-256 of a file, streamed.
pub fn sha256_file(path: &Path) -> std::io::Result<[u8; 32]> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().into())
}

fn hex(d: &[u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Expands an image-sequence pattern (`name_%04d.png`, `name_%d.png` or
/// `name_####.png`) for one frame number.
pub fn sequence_frame(pattern: &str, frame: i64) -> Option<String> {
    if let Some(start) = pattern.find('%') {
        let rest = &pattern[start + 1..];
        let spec_len = rest.find('d')?;
        let spec = &rest[..spec_len];
        if !spec.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let width: usize = if spec.is_empty() { 0 } else { spec.trim_start_matches('0').parse().unwrap_or(0) };
        let num = format_frame(frame, width);
        return Some(format!("{}{}{}", &pattern[..start], num, &rest[spec_len + 1..]));
    }
    let start = pattern.find('#')?;
    let width = pattern[start..].bytes().take_while(|b| *b == b'#').count();
    Some(format!("{}{}{}", &pattern[..start], format_frame(frame, width), &pattern[start + width..]))
}

fn format_frame(frame: i64, width: usize) -> String {
    if frame < 0 {
        format!("-{:0width$}", frame.unsigned_abs(), width = width.saturating_sub(1))
    } else {
        format!("{frame:0width$}")
    }
}

struct Check<'a, 'i> {
    node: Node<'a, 'i>,
    attr: &'static str,
    value: &'a str,
    path: PathBuf,
    expected: Option<[u8; 32]>,
    hash_attr: &'static str,
    missing: Severity,
    missing_code: &'static str,
}

fn attr_loc(n: Node, attr: &str) -> Loc {
    n.attributes()
        .find(|a| a.name() == attr && a.namespace().is_none())
        .map(|a| Loc::at(a.range().start))
        .unwrap_or_else(|| Loc::of(n))
}

fn parse_digest(s: &str) -> Option<[u8; 32]> {
    <crate::values::Sha256 as crate::parse::ParseValue>::parse_value(s).ok().map(|d| d.0)
}

/// Verifies every input file of `doc`, resolving relative references
/// against `base_dir`, and appends diagnostics.
pub fn verify(doc: &Document<'_>, base_dir: &Path, out: &mut Vec<Diagnostic>) {
    let uri_attrs = input_uri_attributes();
    let mut checks: Vec<Check> = Vec::new();
    let mut remote: Vec<(Node, &'static str, String)> = Vec::new();
    for n in doc.root_element().descendants().filter(|n| n.is_element()) {
        let name = n.tag_name().name();
        if name == "imageSequence" {
            sequence(n, base_dir, out);
        }
        for &attr in uri_attrs {
            if name == "imageSequence" && attr == "src" {
                continue;
            }
            let Some(value) = n.attribute(attr) else { continue };
            let (expected, hash_attr) = match attr {
                "src" => (n.attribute("sha256").and_then(parse_digest), "sha256"),
                "cache" => (n.attribute("cacheSha256").and_then(parse_digest), "cacheSha256"),
                _ => (None, ""),
            };
            let (missing, missing_code) = match (name, attr) {
                ("physics", "cache") => (Severity::Warning, "A06"),
                (_, "proxy") => (Severity::Warning, "A01"),
                _ => (Severity::Error, "A01"),
            };
            match resolve(value, base_dir) {
                Resolved::Local(path) => {
                    checks.push(Check { node: n, attr, value, path, expected, hash_attr, missing, missing_code })
                }
                Resolved::Remote(scheme) => remote.push((n, attr, scheme)),
            }
        }
    }

    for (n, attr, scheme) in remote {
        out.push(Diagnostic::warning(
            "A03",
            format!("@{attr} of <{}> is a {scheme} URI and was not verified offline", n.tag_name().name()),
            attr_loc(n, attr),
            element_path(n),
        ));
    }

    // Hash each distinct file once, in parallel.
    let to_hash: BTreeSet<&PathBuf> = checks.iter().filter(|c| c.expected.is_some()).map(|c| &c.path).collect();
    let digests: HashMap<&PathBuf, std::io::Result<[u8; 32]>> =
        to_hash.into_par_iter().map(|p| (p, sha256_file(p))).collect::<Vec<_>>().into_iter().collect();

    for c in &checks {
        let ename = c.node.tag_name().name();
        let loc = attr_loc(c.node, c.attr);
        let path = element_path(c.node);
        let shown = format!("{:?}", c.value);
        let meta = std::fs::metadata(&c.path);
        match meta {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let msg = if c.missing_code == "A06" {
                    format!("physics cache {shown} does not exist; the simulation will be recomputed")
                } else {
                    format!("@{} of <{ename}>: file {shown} does not exist (resolved to {})", c.attr, c.path.display())
                };
                let resolvable = matches!(ename, "generated" | "captionTrack") && c.attr == "cache";
                out.push(Diagnostic {
                    severity: c.missing,
                    code: c.missing_code.into(),
                    message: msg,
                    loc,
                    path,
                    help: resolvable.then(|| "run `scene-render resolve` to make the cache and pin its digest".into()),
                });
                continue;
            }
            Err(e) => {
                out.push(Diagnostic::error(
                    "A05",
                    format!("@{} of <{ename}>: cannot read {shown}: {e}", c.attr),
                    loc,
                    path,
                ));
                continue;
            }
            Ok(m) if m.is_dir() => {
                out.push(Diagnostic::error(
                    "A05",
                    format!("@{} of <{ename}>: {shown} is a directory, not a file", c.attr),
                    loc,
                    path,
                ));
                continue;
            }
            Ok(_) => {}
        }
        let Some(expected) = c.expected else { continue };
        match &digests[&c.path] {
            Ok(actual) if *actual == expected => {}
            Ok(actual) => out.push(
                Diagnostic::error(
                    "A02",
                    format!(
                        "{shown} does not match @{} of <{ename}>: expected {}, found {}",
                        c.hash_attr,
                        hex(&expected),
                        hex(actual)
                    ),
                    attr_loc(c.node, c.hash_attr),
                    path,
                )
                .with_help(if matches!(ename, "generated" | "captionTrack") {
                    "run `scene-render resolve` to make the cache and pin its digest"
                } else {
                    "the file changed since the digest was recorded; regenerate it or update the digest"
                }),
            ),
            Err(e) => out.push(Diagnostic::error("A05", format!("cannot read {shown}: {e}"), loc, path)),
        }
    }
}

fn sequence(n: Node, base_dir: &Path, out: &mut Vec<Diagnostic>) {
    let Some(src) = n.attribute("src") else { return };
    let int = |k: &str| {
        n.attribute(k).and_then(|v| crate::xsd::parse_xsd_integer(v.trim())).and_then(|v| i64::try_from(v).ok())
    };
    let (Some(first), Some(last)) = (int("first"), int("last")) else { return };
    let step = int("step").unwrap_or(1).max(1);
    let loc = attr_loc(n, "src");
    let path = element_path(n);
    if last < first {
        out.push(Diagnostic::error(
            "A04",
            format!("image sequence runs from frame {first} to {last}; last precedes first"),
            loc,
            path,
        ));
        return;
    }
    if sequence_frame(src, first).is_none() {
        out.push(Diagnostic::error(
            "A04",
            format!("image sequence src {src:?} has no frame placeholder (%0Nd, %d or ####)"),
            loc,
            path,
        ));
        return;
    }
    let count = (last - first) / step + 1;
    if count > MAX_SEQUENCE_FRAMES {
        out.push(Diagnostic::warning(
            "A04",
            format!("image sequence has {count} frames; frames were not checked individually"),
            loc,
            path,
        ));
        return;
    }
    let frames: Vec<i64> = (0..count).map(|i| first + i * step).collect();
    let missing: Vec<i64> = frames
        .par_iter()
        .filter(|f| match resolve(&sequence_frame(src, **f).unwrap_or_default(), base_dir) {
            Resolved::Local(p) => !p.is_file(),
            Resolved::Remote(_) => false,
        })
        .copied()
        .collect();
    if let Some(Resolved::Remote(scheme)) = sequence_frame(src, first).map(|f| resolve(&f, base_dir)) {
        out.push(Diagnostic::warning(
            "A03",
            format!("image sequence is a {scheme} URI and was not verified offline"),
            loc,
            path,
        ));
        return;
    }
    if missing.is_empty() {
        return;
    }
    let mut runs: BTreeMap<i64, i64> = BTreeMap::new();
    let mut prev: Option<i64> = None;
    let mut start = 0;
    for &f in &missing {
        match prev {
            Some(p) if f == p + step => {}
            _ => start = f,
        }
        runs.insert(start, f);
        prev = Some(f);
    }
    let shown: Vec<String> =
        runs.iter().take(5).map(|(a, b)| if a == b { a.to_string() } else { format!("{a}–{b}") }).collect();
    let more = if runs.len() > 5 { ", …" } else { "" };
    let policy = n.attribute("missingFrame").unwrap_or("error");
    let msg = format!("image sequence is missing {} of {count} frames ({}{more})", missing.len(), shown.join(", "));
    if policy == "error" {
        out.push(
            Diagnostic::error("A04", msg, loc, path)
                .with_help("add the frames or set missingFrame to hold, black or transparent"),
        );
    } else {
        out.push(Diagnostic::warning("A04", format!("{msg}; missingFrame=\"{policy}\" applies"), loc, path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_patterns() {
        assert_eq!(sequence_frame("f_%04d.png", 7).unwrap(), "f_0007.png");
        assert_eq!(sequence_frame("f_%d.png", 12).unwrap(), "f_12.png");
        assert_eq!(sequence_frame("f_####.exr", 42).unwrap(), "f_0042.exr");
        assert_eq!(sequence_frame("f_%04d.png", -3).unwrap(), "f_-003.png");
        assert!(sequence_frame("plain.png", 1).is_none());
    }

    #[test]
    fn uri_resolution() {
        let base = Path::new("/proj");
        assert_eq!(resolve("media/a%20b.png", base), Resolved::Local(PathBuf::from("/proj/media/a b.png")));
        assert_eq!(resolve("/abs/x.png", base), Resolved::Local(PathBuf::from("/abs/x.png")));
        assert_eq!(resolve("file:///abs/x.png", base), Resolved::Local(PathBuf::from("/abs/x.png")));
        assert_eq!(resolve("https://cdn/x.png", base), Resolved::Remote("https".into()));
        assert!(input_uri_attributes().contains("src"));
        assert!(input_uri_attributes().contains("cache"));
        assert!(!input_uri_attributes().contains("path"));
    }
}
