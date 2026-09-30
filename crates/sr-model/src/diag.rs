//! Diagnostics, source positions and validation reports.

use std::cell::RefCell;
use std::fmt;

/// Severity of a [`Diagnostic`]. Errors make a document unusable; warnings do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The document violates the contract and cannot be rendered.
    Error,
    /// The document is valid but something deserves attention.
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        })
    }
}

/// A position in the source document: 1-based line and character column,
/// plus the 0-based byte offset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub struct Loc {
    /// 1-based line.
    pub line: u32,
    /// 1-based column, counted in characters.
    pub column: u32,
    /// 0-based byte offset.
    pub offset: u32,
}

/// Maps byte offsets to line and column in one source text.
#[derive(Debug, Clone)]
pub struct LineIndex {
    starts: Vec<usize>,
    text: std::sync::Arc<str>,
}

impl LineIndex {
    /// Indexes the line starts of `text`.
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i + 1));
        LineIndex { starts, text: text.into() }
    }

    /// Position of a byte offset.
    pub fn loc(&self, offset: usize) -> Loc {
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let start = self.starts[line];
        let end = offset.min(self.text.len());
        let column = self.text.get(start..end).map(|s| s.chars().count()).unwrap_or(end - start) + 1;
        Loc { line: line as u32 + 1, column: column as u32, offset: offset as u32 }
    }

    /// Text of a 1-based line without its terminator.
    pub fn line_text(&self, line: u32) -> &str {
        let i = line.saturating_sub(1) as usize;
        let Some(&start) = self.starts.get(i) else { return "" };
        let end = self.starts.get(i + 1).copied().unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }
}

thread_local! {
    static ACTIVE: RefCell<Option<LineIndex>> = const { RefCell::new(None) };
}

/// Makes `index` the line index used by [`Loc::of`] on this thread while `f` runs.
pub(crate) fn with_line_index<R>(index: &LineIndex, f: impl FnOnce() -> R) -> R {
    struct Reset(Option<LineIndex>);
    impl Drop for Reset {
        fn drop(&mut self) {
            ACTIVE.with(|a| *a.borrow_mut() = self.0.take());
        }
    }
    let prev = ACTIVE.with(|a| a.borrow_mut().replace(index.clone()));
    let _reset = Reset(prev);
    f()
}

impl Loc {
    /// Position of an XML node, using the active line index of this thread.
    pub fn of(n: roxmltree::Node<'_, '_>) -> Loc {
        Self::at(n.range().start)
    }

    /// Position of a byte offset, using the active line index of this thread.
    pub fn at(offset: usize) -> Loc {
        ACTIVE.with(|a| match a.borrow().as_ref() {
            Some(ix) => ix.loc(offset),
            None => Loc { line: 0, column: 0, offset: offset as u32 },
        })
    }
}

impl fmt::Display for Loc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// One problem found in a document.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Diagnostic {
    /// Error or warning.
    pub severity: Severity,
    /// Stable code: `XML`, `S01`–`S12` (structure), `V1`–`V4`, `C1`–`C44`,
    /// `R1`–`R25-*` (Schematron assert ids), `A01`–`A07` (assets), `W01`.
    /// `scene-render explain <code>` describes each one.
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Where the problem is.
    pub loc: Loc,
    /// XPath-like location of the element, e.g. `/scene/composition/layer[2]`.
    pub path: String,
    /// Optional suggestion for fixing the problem.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

impl Diagnostic {
    /// An error diagnostic.
    pub fn error(code: impl Into<String>, message: impl Into<String>, loc: Loc, path: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Error,
            code: code.into(),
            message: message.into(),
            loc,
            path: path.into(),
            help: None,
        }
    }

    /// A warning diagnostic.
    pub fn warning(code: impl Into<String>, message: impl Into<String>, loc: Loc, path: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Warning,
            code: code.into(),
            message: message.into(),
            loc,
            path: path.into(),
            help: None,
        }
    }

    /// Attaches a suggestion.
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// True for errors.
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}[{}] {}: {}", self.severity, self.code, self.loc, self.message)
    }
}

/// XPath-like path of an element: `/scene/composition/layer[2]`. The index is
/// the position among same-named siblings and is omitted when the element is
/// the only one of its name.
pub fn element_path(n: roxmltree::Node<'_, '_>) -> String {
    let mut parts = Vec::new();
    let mut cur = Some(n);
    while let Some(e) = cur {
        if !e.is_element() {
            break;
        }
        let name = e.tag_name().name();
        let parent = e.parent_element();
        let same: Vec<_> = match parent {
            Some(p) => p.children().filter(|c| c.is_element() && c.tag_name().name() == name).collect(),
            None => vec![e],
        };
        if same.len() > 1 {
            let idx = same.iter().position(|c| *c == e).unwrap_or(0) + 1;
            parts.push(format!("{name}[{idx}]"));
        } else {
            parts.push(name.to_string());
        }
        cur = parent;
    }
    parts.reverse();
    format!("/{}", parts.join("/"))
}

/// The outcome of validating one document.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Report {
    /// Diagnostics in document order.
    pub diagnostics: Vec<Diagnostic>,
}

impl Report {
    /// True when at least one diagnostic is an error.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    /// Number of errors.
    pub fn error_count(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.is_error()).count()
    }

    /// Number of warnings.
    pub fn warning_count(&self) -> usize {
        self.diagnostics.len() - self.error_count()
    }

    /// Distinct diagnostic codes, sorted.
    pub fn codes(&self) -> Vec<&str> {
        let mut c: Vec<&str> = self.diagnostics.iter().map(|d| d.code.as_str()).collect();
        c.sort_unstable();
        c.dedup();
        c
    }

    pub(crate) fn sort(&mut self) {
        self.diagnostics.sort_by(|a, b| {
            (a.loc.offset, a.severity, &a.code, &a.message).cmp(&(b.loc.offset, b.severity, &b.code, &b.message))
        });
        self.diagnostics.dedup();
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for d in &self.diagnostics {
            writeln!(f, "{d}")?;
        }
        write!(f, "{} error(s), {} warning(s)", self.error_count(), self.warning_count())
    }
}
