//! The XSD 1.0 contract of scene-render 1.1.
//!
//! [`SIMPLE_TYPES`] and [`COMPLEX_TYPES`] are generated from
//! `schema/scene-render-1.1.xsd` by `build.rs`; [`structure::validate`]
//! interprets them against a parsed document and [`simple::check`] validates
//! one lexical value against one simple type.

pub mod simple;
pub mod structure;

/// `maxOccurs="unbounded"`.
pub const UNBOUNDED: u32 = u32::MAX;

/// XSD built-in datatypes used by the schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum Builtin {
    String,
    Double,
    /// `xs:decimal`: kept as its exact text (SREP 75).
    Decimal,
    Boolean,
    Integer,
    Int,
    NonNegativeInteger,
    PositiveInteger,
    UnsignedLong,
    Id,
    IdRef,
    IdRefs,
    NcName,
    NmToken,
    NmTokens,
    AnyUri,
    DateTime,
}

/// Facets of an `xs:restriction`.
#[derive(Debug, Clone, Copy)]
pub struct Restriction {
    /// Index of the base simple type.
    pub base: usize,
    /// Allowed literals; empty when the restriction has no enumeration.
    pub enums: &'static [&'static str],
    /// `xs:minInclusive`.
    pub min_inclusive: Option<f64>,
    /// `xs:maxInclusive`.
    pub max_inclusive: Option<f64>,
    /// `xs:minExclusive`.
    pub min_exclusive: Option<f64>,
    /// `xs:maxExclusive`.
    pub max_exclusive: Option<f64>,
    /// `xs:pattern` (XSD regular expression, implicitly anchored).
    pub pattern: Option<&'static str>,
    /// `xs:maxLength` in characters.
    pub max_length: Option<usize>,
}

/// Variety of a simple type.
#[derive(Debug, Clone, Copy)]
pub enum SimpleKind {
    /// A built-in datatype.
    Builtin(Builtin),
    /// A restriction of another simple type.
    Restriction(Restriction),
    /// A union of member types (indices).
    Union(&'static [usize]),
    /// A whitespace-separated list of the item type (index).
    List(usize),
}

/// A simple type definition.
#[derive(Debug, Clone, Copy)]
pub struct SimpleType {
    /// XSD name, `xs:<builtin>`, or `owner@attribute` for anonymous types.
    pub name: &'static str,
    /// Definition.
    pub kind: SimpleKind,
}

/// How an attribute takes part in `xs:ID` / `xs:IDREF` identity constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind {
    /// Not an identity attribute.
    None,
    /// `xs:ID`: unique across the document.
    Id,
    /// `xs:IDREF`: must equal some `xs:ID` value.
    IdRef,
    /// `xs:IDREFS`: every token must equal some `xs:ID` value.
    IdRefs,
}

/// An attribute declaration.
#[derive(Debug, Clone, Copy)]
pub struct AttrDecl {
    /// Attribute name.
    pub name: &'static str,
    /// Index of its simple type.
    pub ty: usize,
    /// `use="required"`.
    pub required: bool,
    /// Default value, if declared.
    pub default: Option<&'static str>,
    /// Identity role.
    pub id: IdKind,
}

/// A content-model particle. Model groups are expanded in place.
#[derive(Debug, Clone, Copy)]
pub enum Particle {
    /// A local element declaration.
    Element {
        /// Element name.
        name: &'static str,
        /// Index of its complex type.
        ty: usize,
        /// `minOccurs`.
        min: u32,
        /// `maxOccurs`.
        max: u32,
    },
    /// `xs:sequence`.
    Seq {
        /// Items in order.
        items: &'static [Particle],
        /// `minOccurs`.
        min: u32,
        /// `maxOccurs`.
        max: u32,
    },
    /// `xs:choice`.
    Choice {
        /// Alternatives.
        items: &'static [Particle],
        /// `minOccurs`.
        min: u32,
        /// `maxOccurs`.
        max: u32,
    },
}

/// Content of a complex type.
#[derive(Debug, Clone, Copy)]
pub enum Content {
    /// No children and no text.
    Empty,
    /// Text of the given simple type.
    Simple(usize),
    /// Element-only content.
    Elements(&'static Particle),
}

/// A complex type definition with extension already applied.
#[derive(Debug, Clone, Copy)]
pub struct ComplexType {
    /// XSD name (`scene` for the anonymous root type).
    pub name: &'static str,
    /// Attribute declarations, attribute groups expanded.
    pub attrs: &'static [AttrDecl],
    /// Content model.
    pub content: Content,
}

impl ComplexType {
    /// Declaration of attribute `name`.
    pub fn attr(&self, name: &str) -> Option<&'static AttrDecl> {
        self.attrs.iter().find(|a| a.name == name)
    }
}

include!(concat!(env!("OUT_DIR"), "/schema_tables.rs"));

/// Root element name.
pub const ROOT_ELEMENT: &str = "scene";

/// Complex type by XSD name.
pub fn complex_type(name: &str) -> Option<&'static ComplexType> {
    COMPLEX_TYPES.iter().find(|c| c.name == name)
}

/// Built-in datatype at the root of a simple type's restriction chain, or
/// `None` for unions and lists.
pub fn root_builtin(mut ty: usize) -> Option<Builtin> {
    loop {
        match SIMPLE_TYPES[ty].kind {
            SimpleKind::Builtin(b) => return Some(b),
            SimpleKind::Restriction(r) => ty = r.base,
            SimpleKind::Union(_) | SimpleKind::List(_) => return None,
        }
    }
}

/// Parses an XSD integer literal (`[+-]?[0-9]+`).
pub fn parse_xsd_integer(s: &str) -> Option<i128> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || digits.len() > 38 {
        return None;
    }
    s.strip_prefix('+').unwrap_or(s).parse().ok()
}

/// Parses an XSD 1.0 `xs:double` literal, including `INF`, `-INF` and `NaN`.
pub fn parse_xsd_double(s: &str) -> Option<f64> {
    match s {
        "INF" => return Some(f64::INFINITY),
        "-INF" => return Some(f64::NEG_INFINITY),
        "NaN" => return Some(f64::NAN),
        _ => {}
    }
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let fs = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - fs;
    }
    if int_digits + frac_digits == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let es = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == es {
            return None;
        }
    }
    if i != b.len() {
        return None;
    }
    s.parse().ok()
}
