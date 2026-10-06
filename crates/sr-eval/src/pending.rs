//! Accepted SREPs that the Rust reference does not implement yet.
//!
//! The schema (sr-core 1.3.0) accepts every accepted SREP's elements and attributes, so a document may use a feature
//! this engine ignores. It must not render silently wrong: each use is reported as a warning (E22), which `--strict`
//! counts, and the document still validates. A SREP leaves the table below, in the commit that implements it and
//! passes its conformance case.
//!
//! To implement a pending SREP: delete its row from [`PENDING`].

use sr_model::diag::Diagnostic;
use sr_model::element::{AttrValue, Element};

/// One accepted SREP the engine does not implement yet.
pub struct Pending {
    /// The SREP's number.
    pub srep: u32,
    /// What it adds, for the message.
    pub what: &'static str,
    /// Elements that exist only through it.
    pub elements: &'static [&'static str],
    /// `(element, attribute, default)`: the attribute is a use unless it is absent or equals its default.
    pub attributes: &'static [(&'static str, &'static str, Option<&'static str>)],
}

/// The pending SREPs. One row each; the gap branches delete their own.
pub const PENDING: &[Pending] = &[
    Pending { srep: 16, what: "connectors that follow the nodes they join", elements: &["connector"], attributes: &[] },
    Pending {
        srep: 17,
        what: "PDF page assets and text-anchored regions",
        elements: &["pdf", "region"],
        attributes: &[("shape", "region", None), ("shape", "regionLayer", None), ("shape", "regionPadding", Some("0"))],
    },
    Pending { srep: 18, what: "render reports", elements: &[], attributes: &[("output", "report", None)] },
    Pending {
        srep: 19,
        what: "legibility checks for video text and captions",
        elements: &[],
        attributes: &[
            ("accessibility", "legibilityCheck", Some("off")),
            ("accessibility", "readingSpeed", None),
            ("accessibility", "minDisplayTime", Some("0.8333")),
            ("accessibility", "minTextSize", None),
            ("captionTrack", "readingSpeed", None),
        ],
    },
    Pending { srep: 21, what: "the pinned-font policy", elements: &[], attributes: &[("project", "fontPolicy", Some("system"))] },
    Pending { srep: 26, what: "repeat copies placed on generated points", elements: &["points"], attributes: &[] },
];

fn same(value: &AttrValue, default: &str) -> bool {
    match (value, default.parse::<f64>()) {
        (AttrValue::Num(v), Ok(d)) => (*v - d).abs() < 1e-12,
        _ => value.to_string() == default,
    }
}

/// Reports every use of a pending SREP's feature on `e` (one element; the caller walks the tree).
pub fn check(e: &dyn Element, warnings: &mut Vec<Diagnostic>) {
    // an element reached through a typed field (an output, the project, an accessibility block) is named by its
    // complex type ("outputType"); one reached through a child enum is named by its element ("shape")
    let raw = e.element_name();
    let name = raw.strip_suffix("Type").unwrap_or(raw);
    for p in PENDING {
        let element_use = p.elements.contains(&name);
        let attribute_use = p.attributes.iter().find(|(el, attr, default)| {
            *el == name
                && e.get_attr(attr).is_some_and(|v| default.is_none_or(|d| !same(&v, d)))
        });
        let used = if element_use {
            Some(format!("<{name}>"))
        } else {
            attribute_use.map(|(el, attr, _)| format!("@{attr} of <{el}>"))
        };
        if let Some(used) = used {
            warnings.push(Diagnostic::warning(
                "E22",
                format!(
                    "{used} belongs to SREP {} ({}), which is accepted but not implemented by this engine yet; it is ignored",
                    p.srep, p.what
                ),
                e.loc(),
                e.element_id().unwrap_or(""),
            ));
        }
    }
}
