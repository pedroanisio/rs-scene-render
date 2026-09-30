//! Catalogue of every diagnostic code, used by `scene-render explain`.

include!(concat!(env!("OUT_DIR"), "/sch_asserts.rs"));

/// One diagnostic code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Code {
    /// The code as it appears in diagnostics.
    pub code: &'static str,
    /// Stage that emits it: `xml`, `structure`, `rules`, `assets` or `model`.
    pub stage: &'static str,
    /// One-line description.
    pub summary: &'static str,
    /// For Schematron asserts: the rule context and the XPath test.
    pub rule: Option<(&'static str, &'static str)>,
}

const FIXED: &[(&str, &str, &str)] = &[
    ("XML", "xml", "The file is not well-formed XML."),
    ("S01", "structure", "The root element is not <scene>."),
    ("S02", "structure", "An element is not allowed where it appears: unknown, out of order, or repeated too often."),
    ("S03", "structure", "A required child element is missing."),
    ("S04", "structure", "An attribute is not declared for its element."),
    ("S05", "structure", "A required attribute is missing."),
    ("S06", "structure", "An attribute value violates its type: lexical form, enumeration, bounds, pattern or length."),
    ("S07", "structure", "Text appears inside an element that only takes child elements."),
    ("S08", "structure", "Text content violates its type."),
    ("S09", "structure", "Two elements share the same xs:ID value; ids are unique across the document."),
    ("S10", "structure", "An xs:IDREF value names no element id."),
    ("S11", "structure", "An element or attribute is in a foreign namespace; scene-render uses no namespace."),
    ("S12", "structure", "Elements are nested deeper than the supported limit of 256 levels."),
    (
        "W01",
        "structure",
        "A numeric attribute holds INF, -INF or NaN; the schema accepts them but the renderer needs finite values.",
    ),
    ("A01", "assets", "A referenced input file does not exist."),
    ("A02", "assets", "A file's SHA-256 digest differs from the declared sha256 or cacheSha256."),
    ("A03", "assets", "A remote input (http, https, s3, …) could not be verified offline."),
    ("A04", "assets", "Frames of an image sequence are missing; an error when missingFrame=\"error\"."),
    ("A05", "assets", "A referenced file exists but could not be read."),
    ("A06", "assets", "A physics cache is declared but absent; the simulation will be recomputed."),
    ("A07", "assets", "An image's declared width and height differ from its local source file. Layout uses the declared dimensions, so a different aspect ratio distorts the image. Set the asset dimensions to the file's size and use the layer's boxWidth, boxHeight and fit for layout. Proxy representations are not compared."),
    ("P01", "rules", "A soft body is too stiff for its mass: stable integration needs more than 4096 substeps per physics@fixedStep."),
    ("M01", "model", "Internal error: the validated document could not be converted to the typed model."),
];

/// Every code: fixed codes first, then the Schematron asserts in schema order.
pub fn all() -> Vec<Code> {
    let mut v: Vec<Code> =
        FIXED.iter().map(|(code, stage, summary)| Code { code, stage, summary, rule: None }).collect();
    v.extend(SCH_ASSERTS.iter().map(|(id, _p, ctx, test, msg)| Code {
        code: id,
        stage: "rules",
        summary: msg,
        rule: Some((ctx, test)),
    }));
    v
}

/// Looks up a code, case-insensitively.
pub fn lookup(code: &str) -> Option<Code> {
    all().into_iter().find(|c| c.code.eq_ignore_ascii_case(code))
}
