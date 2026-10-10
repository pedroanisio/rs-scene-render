//! Build-mode programs (SREP 66): each `<program>` in build mode runs once when the document is loaded, and its
//! output takes its place.
//!
//! * In the composition (or a symbol, group, …), the program becomes a `<group>` that carries its node
//!   attributes and behaviour children, with the emitted nodes as its last children.
//! * In `<parameters>`, the program becomes `<data format="json">` holding the emitted rows.
//!
//! The expanded text is then validated and built like any document. Its errors are reported as PRG14 at the
//! program that emitted the offending output.
//!
//! Outputs are cached on disk by the key of SREP 66, Determinism 6, under `$SR_PROGRAM_CACHE`, else
//! `$XDG_CACHE_HOME/scene-render/program`, else `~/.cache/scene-render/program`. `SR_PROGRAM_CACHE=off` turns the
//! cache off. A cache file holds the output's SHA-256 before the output. A file whose digest does not match is
//! ignored and recomputed.

use std::fmt::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};

use roxmltree::Node;
use sha2::{Digest, Sha256};

use crate::assets::{resolve, Resolved};
use crate::diag::{element_path, Diagnostic, Loc};

/// Elements a program's output may hold at most.
pub const MAX_ELEMENTS: usize = 1_000_000;

/// Attributes of `<program>` that are not node attributes of the group it becomes.
const PROGRAM_ONLY: [&str; 10] =
    ["src", "sha256", "seed", "fuel", "memoryLimit", "outputSha256", "mode", "width", "height", "stepsPerFrame"];

/// A document whose build-mode programs were replaced by their output.
pub(crate) struct Expanded {
    /// The new document text.
    pub xml: String,
    /// Where each program's output lies in `xml`, with the program's id, location and path in the original.
    pub outputs: Vec<(Range<usize>, String, Loc, String)>,
}

/// Whether `n` is a program that runs when the document is built.
pub fn is_build_program(n: Node<'_, '_>) -> bool {
    n.is_element()
        && n.tag_name().name() == "program"
        && (n.parent_element().is_some_and(|p| p.tag_name().name() == "parameters")
            || n.attribute("mode").is_none_or(|m| m == "build"))
}

/// The inputs and limits of a program element, as SREP 66 reads them.
pub fn program_call(n: Node<'_, '_>) -> (sr_wasm::Inputs, sr_wasm::Limits) {
    let num = |k: &str| n.attribute(k).and_then(|v| v.trim().parse::<u64>().ok());
    let project_seed = n
        .document()
        .root_element()
        .children()
        .find(|c| c.tag_name().name() == "project")
        .and_then(|p| p.attribute("seed"))
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let params = n
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "param")
        .map(|c| (c.attribute("name").unwrap_or("").to_string(), c.attribute("value").unwrap_or("").to_string()))
        .collect();
    let limits = sr_wasm::Limits {
        fuel: num("fuel").unwrap_or(1_000_000_000),
        memory_mib: num("memoryLimit").map(|m| m as u32).unwrap_or(64),
    };
    (sr_wasm::Inputs { project_seed, seed: num("seed").unwrap_or(0), params }, limits)
}

/// Reads a program's module and checks it against `@sha256` (PRG10).
pub fn read_module(n: Node<'_, '_>, base: &Path) -> Result<(Vec<u8>, String), String> {
    let src = n.attribute("src").unwrap_or("");
    let sha = n.attribute("sha256").unwrap_or("");
    read_module_file(src, sha, base).map(|bytes| (bytes, sha.trim().to_ascii_lowercase()))
}

/// Reads the module at `src` (resolved against `base`) and checks it against the hex digest `sha256` (PRG10).
pub fn read_module_file(src: &str, sha256: &str, base: &Path) -> Result<Vec<u8>, String> {
    let path = match resolve(src, base) {
        Resolved::Local(p) => p,
        Resolved::Remote(scheme) => {
            return Err(format!("{src:?} is a {scheme} URI; a program's module is a local file"))
        }
    };
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {src:?} ({}): {e}", path.display()))?;
    let actual = hex(&Sha256::digest(&bytes));
    let expected = sha256.trim().to_ascii_lowercase();
    if actual != expected {
        return Err(format!("{src:?} does not match @sha256: expected {expected}, found {actual}"));
    }
    Ok(bytes)
}

fn hex(b: &[u8]) -> String {
    b.iter().fold(String::with_capacity(b.len() * 2), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

fn cache_dir() -> Option<PathBuf> {
    match std::env::var_os("SR_PROGRAM_CACHE") {
        Some(v) if v == "off" => None,
        Some(v) if !v.is_empty() => Some(PathBuf::from(v)),
        _ => std::env::var_os("XDG_CACHE_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .map(|d| d.join("scene-render").join("program")),
    }
}

fn cached(key: &str) -> Option<Vec<u8>> {
    let data = std::fs::read(cache_dir()?.join(key)).ok()?;
    let (digest, output) = (data.get(..64)?, data.get(65..)?);
    (data.get(64) == Some(&b'\n') && digest == hex(&Sha256::digest(output)).as_bytes()).then(|| output.to_vec())
}

fn store(key: &str, output: &[u8]) {
    let Some(dir) = cache_dir() else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let tmp = dir.join(format!(".{key}.{}", std::process::id()));
    let mut data = hex(&Sha256::digest(output)).into_bytes();
    data.push(b'\n');
    data.extend_from_slice(output);
    if std::fs::write(&tmp, &data).is_ok() && std::fs::rename(&tmp, dir.join(key)).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Runs a build-mode program, or takes its output from the cache, and checks `@outputSha256` (PRG15).
fn output_of(n: Node<'_, '_>, base: &Path) -> Result<Vec<u8>, (&'static str, String)> {
    let (module, sha) = read_module(n, base).map_err(|m| ("PRG10", m))?;
    let (inputs, limits) = program_call(n);
    let key = sr_wasm::cache_key(&sha, &inputs, limits);
    let output = match cached(&key) {
        Some(o) => o,
        None => {
            let o = sr_wasm::generate(&module, &inputs, limits).map_err(|e| (e.code.as_str(), e.message))?;
            store(&key, &o);
            o
        }
    };
    if let Some(want) = n.attribute("outputSha256") {
        let got = hex(&Sha256::digest(&output));
        if got != want.trim().to_ascii_lowercase() {
            return Err(("PRG15", format!("the output's SHA-256 is {got}, not the outputSha256 {want}")));
        }
    }
    Ok(output)
}

fn escape(s: &str, attr: bool) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' if attr => o.push_str("&quot;"),
            '\n' if attr => o.push_str("&#10;"),
            '\t' if attr => o.push_str("&#9;"),
            '\r' => o.push_str("&#13;"),
            c => o.push(c),
        }
    }
    o
}

/// Checks a node program's output as SREP 66, Semantics 5 asks, before it goes in place.
fn check_fragment(text: &str) -> Result<(), String> {
    if text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
        return Err("the output has a DOCTYPE or an entity declaration".into());
    }
    let wrapped = format!("<sr-fragment>{text}</sr-fragment>");
    let opts = roxmltree::ParsingOptions { allow_dtd: false, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(&wrapped, opts)
        .map_err(|e| format!("the output is not a well-formed XML fragment: {e}"))?;
    let root = doc.root_element();
    for c in root.children() {
        if c.is_pi() {
            return Err("the output has a processing instruction".into());
        }
        if c.is_text() && !c.text().unwrap_or("").trim().is_empty() {
            return Err("the output has text outside its elements".into());
        }
    }
    let mut count = 0usize;
    for d in root.descendants().filter(|d| d.is_element() && *d != root) {
        count += 1;
        if count > MAX_ELEMENTS {
            return Err(format!("the output has more than {MAX_ELEMENTS} elements"));
        }
        if d.tag_name().name() == "program" {
            return Err("the output contains a program; a program's output is final".into());
        }
        if d.tag_name().namespace().is_some() {
            return Err("the output has an element in a namespace".into());
        }
    }
    if root.descendants().any(|d| d.is_pi()) {
        return Err("the output has a processing instruction".into());
    }
    Ok(())
}

/// Checks a data program's output: JSON that a data source accepts (an array of rows, or an object with a `rows`
/// array).
fn check_rows(text: &str) -> Result<(), String> {
    let j: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("the output is not JSON: {e}"))?;
    match &j {
        serde_json::Value::Array(_) => Ok(()),
        serde_json::Value::Object(o) if matches!(o.get("rows"), Some(serde_json::Value::Array(_))) => Ok(()),
        _ => Err("the output is not an array of rows or an object with a \"rows\" array".into()),
    }
}

/// Replaces every build-mode program of `doc` by its output. `None` when the document has none.
pub(crate) fn expand(
    doc: &roxmltree::Document<'_>,
    xml: &str,
    base: &Path,
) -> Option<Result<Expanded, Vec<Diagnostic>>> {
    let programs: Vec<Node> = doc.root_element().descendants().filter(|n| is_build_program(*n)).collect();
    if programs.is_empty() {
        return None;
    }
    let mut errors = Vec::new();
    let mut replacements: Vec<(Range<usize>, String, Range<usize>, Node)> = Vec::new();
    for n in programs {
        let id = n.attribute("id").unwrap_or("").to_string();
        let fail = |code: &str, message: String| {
            Diagnostic::error(code, format!("program {id:?}: {message}"), Loc::of(n), element_path(n))
        };
        let output = match output_of(n, base) {
            Ok(o) => o,
            Err((code, message)) => {
                errors.push(fail(code, message));
                continue;
            }
        };
        let text = match String::from_utf8(output) {
            Ok(t) => t,
            Err(_) => {
                errors.push(fail("PRG14", "the output is not UTF-8".into()));
                continue;
            }
        };
        let data = n.parent_element().is_some_and(|p| p.tag_name().name() == "parameters");
        let checked = if data { check_rows(&text) } else { check_fragment(&text) };
        if let Err(m) = checked {
            errors.push(fail("PRG14", m));
            continue;
        }
        let (mut head, body, tail);
        if data {
            head = format!("<data id=\"{}\" format=\"json\">", escape(&id, true));
            body = escape(&text, false);
            tail = "</data>".to_string();
        } else {
            head = String::from("<group");
            for a in n.attributes().filter(|a| !PROGRAM_ONLY.contains(&a.name()) && a.name() != "prewarm") {
                let _ = write!(head, " {}=\"{}\"", a.name(), escape(a.value(), true));
            }
            head.push('>');
            for c in n.children() {
                if c.is_element() && c.tag_name().name() == "param" {
                    continue;
                }
                head.push_str(&xml[c.range()]);
            }
            body = text;
            tail = "</group>".to_string();
        }
        let start_of_body = head.len();
        let end_of_body = start_of_body + body.len();
        replacements.push((n.range(), head + &body + &tail, start_of_body..end_of_body, n));
    }
    if !errors.is_empty() {
        return Some(Err(errors));
    }
    replacements.sort_by_key(|r| r.0.start);
    let mut out = String::with_capacity(xml.len());
    let mut outputs = Vec::new();
    let mut at = 0;
    for (range, text, body, n) in replacements {
        out.push_str(&xml[at..range.start]);
        let base_offset = out.len();
        out.push_str(&text);
        outputs.push((
            base_offset + body.start..base_offset + body.end,
            n.attribute("id").unwrap_or("").to_string(),
            Loc::of(n),
            element_path(n),
        ));
        at = range.end;
    }
    out.push_str(&xml[at..]);
    Some(Ok(Expanded { xml: out, outputs }))
}
