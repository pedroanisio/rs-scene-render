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
];

fn same(value: &AttrValue, default: &str) -> bool {
    match (value, default.parse::<f64>()) {
        (AttrValue::Num(v), Ok(d)) => (*v - d).abs() < 1e-12,
        _ => value.to_string() == default,
    }
}

/// Reports every use of a pending SREP's feature on `e` (one element; the caller walks the tree).
pub fn check(e: &dyn Element, warnings: &mut Vec<Diagnostic>) {
    check_against(PENDING, e, warnings)
}

/// [`check`] against the rows of `table`.
fn check_against(table: &[Pending], e: &dyn Element, warnings: &mut Vec<Diagnostic>) {
    // an element reached through a typed field (an output, the project, an accessibility block) is named by its
    // complex type ("outputType"); one reached through a child enum is named by its element ("shape")
    let raw = e.element_name();
    let name = raw.strip_suffix("Type").unwrap_or(raw);
    for p in table {
        let element_use = p.elements.contains(&name);
        let attribute_use = p.attributes.iter().find(|(el, attr, default)| {
            *el == name && e.get_attr(attr).is_some_and(|v| default.is_none_or(|d| !same(&v, d)))
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

#[cfg(test)]
mod tests {
    //! The guard's detection of attributes on elements reached through typed fields (output, project), which no row
    //! of [`PENDING`] exercises once SREPs 18, 19 and 21 are implemented: synthetic rows keep that path covered.

    use super::*;

    /// Synthetic rows on typed-field elements, with and without a default.
    const SYNTHETIC: &[Pending] = &[
        Pending {
            srep: 9001,
            what: "a synthetic output attribute",
            elements: &[],
            attributes: &[("output", "report", None)],
        },
        Pending {
            srep: 9002,
            what: "a synthetic project attribute",
            elements: &[],
            attributes: &[("project", "fontPolicy", Some("system"))],
        },
    ];

    /// The E22 findings of the synthetic rows over every element of `xml`, walked as the evaluator walks it.
    fn findings(xml: &str) -> Vec<Diagnostic> {
        let doc = sr_model::load_str(xml, &sr_model::LoadOptions::without_assets()).unwrap_or_else(|e| panic!("{e:?}"));
        let mut w = Vec::new();
        sr_model::element::walk(&doc.scene, &mut |e| check_against(SYNTHETIC, e, &mut w));
        w
    }

    #[test]
    fn an_attribute_on_a_typed_field_element_is_found() {
        let w = findings(
            r#"<scene version="1.2"><project width="64" height="64" fps="10" duration="1" fontPolicy="pinned"/>
            <output id="o" path="out/o.mp4" codec="h264" report="out/r.json"/><composition/></scene>"#,
        );
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w.iter().all(|d| d.code == "E22" && d.severity == sr_model::Severity::Warning), "{w:?}");
        assert!(
            w.iter().any(|d| d.message.contains("@report of <output>") && d.message.contains("SREP 9001")),
            "{w:?}"
        );
        assert!(
            w.iter().any(|d| d.message.contains("@fontPolicy of <project>") && d.message.contains("SREP 9002")),
            "{w:?}"
        );
    }

    #[test]
    fn absent_or_default_is_not_a_use() {
        let w = findings(
            r#"<scene version="1.2"><project width="64" height="64" fps="10" duration="1" fontPolicy="system"/>
            <output id="o" path="out/o.mp4" codec="h264"/><composition/></scene>"#,
        );
        assert!(w.is_empty(), "{w:?}");
    }
}
